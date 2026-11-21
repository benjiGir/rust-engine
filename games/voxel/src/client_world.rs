//! Le monde que voit la fenêtre : une **réplique** des chunks que le serveur envoie.
//!
//! Le serveur (le `ChunkStreamer` partagé avec l'ECS et les mods) charge, génère, éclaire et
//! sauvegarde les chunks, applique les actions des joueurs, mais ne maille rien. Un
//! [`engine_voxel::ChunkSender`] par joueur lit le monde du serveur à chaque image et en fait des
//! messages ([`ServerMsg::Chunk`], [`ServerMsg::ChunkChanged`], [`ServerMsg::Unload`]) : les chunks
//! sont des `Arc`, ils passent sans copie. Le client range ces messages dans sa réplique, un second
//! `ChunkStreamer` qui ne génère, n'éclaire ni ne sauvegarde rien, mais maille. Tout ce que la fenêtre
//! dessine ou vise (rendu, rayon de visée, culling des grottes, pas, couvercles de coffres, eau, vent,
//! lumière du bras) lit cette réplique, jamais le monde du serveur.
//!
//! Une édition du joueur est appliquée par le serveur dans l'image, envoyée à la fin de cette image, et
//! remaillée par la réplique avant l'envoi au GPU : elle se voit tout de suite.
//!
//! Le serveur et le client sont encore dans le même thread, et le lien est la liste `replies` du jeu,
//! vidée à chaque image. `VoxelGame::streamer()` désigne le monde du serveur, `VoxelGame::view_world()`
//! celui que voit la fenêtre.

use std::collections::HashMap;

use crate::VoxelGame;
use crate::client_entities::ClientEntities;
use crate::entities::{EntityTracker, NetId, NetIds, Viewer, collect};
use crate::player_view::{ClientPlayer, PlayerViews};
use crate::players::PlayerId;
use crate::protocol::ServerMsg;
use engine_voxel::{ChunkSender, ChunkStreamer, ChunkUpdate, StreamStats, View, World};

/// La réplique du client et les expéditeurs du serveur.
pub struct ClientWorld {
    /// Le monde que voit la fenêtre : les chunks reçus, maillés ici.
    replica: ChunkStreamer,
    /// Ce que chaque joueur a reçu du monde (voir `engine_voxel::ChunkSender`).
    senders: HashMap<PlayerId, ChunkSender>,
    /// Les messages de chunks reçus, appliqués ensemble (voir [`ClientWorld::apply_received`]).
    incoming: Vec<ChunkUpdate>,
    /// Ce que chaque joueur a reçu des entités (voir `entities.rs`).
    trackers: HashMap<PlayerId, EntityTracker>,
    /// Les entités que voit la fenêtre : créatures, objets, wagons, projectiles, structures, autres
    /// joueurs (voir `client_entities.rs`).
    pub entities: ClientEntities,
    /// Le pas fixe du serveur auquel en sont les entités.
    tick: u64,
    /// Ce que le serveur a envoyé en dernier des jauges et de l'inventaire de chaque joueur.
    pub player_views: PlayerViews,
    /// Le joueur de la fenêtre, tel que le serveur le décrit (voir `player_view.rs`).
    pub me: ClientPlayer,
}

/// Le nombre de pas fixes entre deux envois de positions : 1 sur le lien en mémoire (60 Hz, comme
/// l'ECS), 3 (20 Hz) sera celui du réseau. `VOXEL_ENTITY_INTERVAL` le change pour les essais.
fn entity_interval() -> u32 {
    std::env::var("VOXEL_ENTITY_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(1, |n: u32| n.clamp(1, 12))
}

impl ClientWorld {
    /// La réplique d'un serveur : même registre, mêmes réglages de maillage, aucun chunk.
    pub fn new(server: &ChunkStreamer) -> Self {
        Self {
            replica: ChunkStreamer::replica_of(server),
            senders: HashMap::new(),
            incoming: Vec::new(),
            trackers: HashMap::new(),
            entities: ClientEntities::new(entity_interval()),
            tick: 0,
            player_views: PlayerViews::default(),
            me: ClientPlayer::default(),
        }
    }

    pub fn replica(&self) -> &ChunkStreamer {
        &self.replica
    }

    /// Un joueur est parti : on oublie ce qu'on lui avait envoyé.
    pub fn forget_player(&mut self, id: PlayerId) {
        self.senders.remove(&id);
        self.trackers.remove(&id);
        self.player_views.forget(id);
    }

    pub fn replica_mut(&mut self) -> &mut ChunkStreamer {
        &mut self.replica
    }

    /// Les blocs que voit la fenêtre.
    pub fn world(&self) -> &World {
        self.replica.world()
    }

    /// Range un message de chunk du serveur (les autres messages ne sont pas pour la réplique).
    pub fn receive(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Chunk { pos, chunk } => {
                self.incoming.push(ChunkUpdate::Chunk { pos, chunk })
            }
            ServerMsg::ChunkChanged {
                pos,
                chunk,
                edits,
                remesh,
            } => self.incoming.push(ChunkUpdate::Changed {
                pos,
                chunk,
                edits,
                remesh,
            }),
            ServerMsg::Unload(pos) => self.incoming.push(ChunkUpdate::Unload(pos)),
            ServerMsg::Vitals(vitals) => self.me.receive_vitals(vitals),
            ServerMsg::Inventory(inventory) => self.me.inventory = inventory,
            ServerMsg::Container(container) => self.me.container = container,
            ServerMsg::ScreenView(screen) => self.me.screen = screen,
            ServerMsg::Quests(quests) => self.me.quests = quests,
            // Les retours (son, message, bras) et `Correct` sont appliqués par le jeu.
            ServerMsg::Correct { .. }
            | ServerMsg::Feedback(_)
            | ServerMsg::Died { .. }
            | ServerMsg::Respawned
            | ServerMsg::OpenScreen(_)
            | ServerMsg::CloseScreen
            | ServerMsg::Chat(_)
            | ServerMsg::ModOptions { .. }
            | ServerMsg::ModHud { .. } => {}
            // Les entités : Tick, Spawn, Despawn, Moves, EntityState, GridBlocks.
            entity_msg => self.entities.receive(&entity_msg),
        }
    }

    /// La réplique applique les chunks reçus : ils sont remplacés d'abord, les blocs édités se remaillent
    /// à sa prochaine mise à jour.
    pub fn apply_received(&mut self) {
        if !self.incoming.is_empty() {
            self.replica
                .apply_updates(std::mem::take(&mut self.incoming));
        }
    }

    /// Plus rien n'est en route : tout ce que le serveur voulait envoyer est reçu et maillé.
    pub fn idle(&self) -> bool {
        self.incoming.is_empty()
            && self.replica.is_idle()
            && self.senders.values().all(|s| s.pending() == 0)
    }
}

/// Les compteurs du monde pour le titre de la fenêtre et les logs : le chargement est celui du
/// serveur, le maillage et les envois au GPU sont ceux de la réplique.
pub fn combined_stats(server: StreamStats, replica: StreamStats) -> StreamStats {
    StreamStats {
        in_flight: server.in_flight + replica.in_flight,
        meshes_waiting: replica.meshes_waiting,
        bytes_waiting: replica.bytes_waiting,
        unloads_waiting: replica.unloads_waiting,
        candidates: server.candidates + replica.candidates,
        jobs_done: server.jobs_done + replica.jobs_done,
        jobs_discarded: server.jobs_discarded + replica.jobs_discarded,
        ..server
    }
}

impl VoxelGame {
    /// Le monde que voit la fenêtre (la réplique), pour tout ce qui se dessine ou se vise.
    pub(crate) fn view_world(&self) -> &World {
        self.client.view.world()
    }

    /// Le streamer de la réplique, pour ce qui lit des blocs actifs (`BlockAccess`) à l'écran.
    pub(crate) fn view_streamer(&self) -> &ChunkStreamer {
        self.client.view.replica()
    }

    /// Les compteurs du chargement (serveur) et du maillage (réplique), ensemble.
    pub(crate) fn stream_stats(&self) -> StreamStats {
        combined_stats(self.streamer().stats(), self.client.view.replica().stats())
    }

    /// Le serveur et la réplique n'ont plus rien à faire, et tout ce qui est en route est arrivé.
    pub(crate) fn world_idle(&self) -> bool {
        self.streamer().is_idle() && self.client.view.idle()
    }

    /// Ce que les commandes de `ctl` ont changé après la fin de l'image : envoyé, remaillé et déjà au GPU.
    pub(crate) fn flush_world_view(&mut self) {
        self.sync_world_view();
        self.apply_stream_events();
    }

    /// Ce que le regard de la fenêtre touche parmi les entités qu'elle voit : la créature la plus
    /// proche, sinon le wagon, avant le bloc visé (ou la portée). `None` : rien à frapper.
    pub(crate) fn window_entity(&self) -> Option<NetId> {
        let reach = self.server.players.local().gear.reach();
        let distance = self.client.target.map_or(reach, |hit| hit.distance);
        self.client.view.entities.hit(
            self.client.camera.position,
            self.client.camera.forward(),
            distance,
        )
    }

    /// Un pas fixe du serveur : il donne un numéro aux entités neuves, puis dit à chaque joueur ce qui
    /// entre dans sa portée, en sort, bouge ou change d'apparence. Le client applique les messages
    /// aussitôt (voir `apply_replies`) : le lien est la liste `replies`, vidée à chaque pas.
    pub(crate) fn replicate_entities(&mut self) {
        self.client.view.tick += 1;
        NetIds::assign(&mut self.server.ecs);
        self.keep_ghosts_alive();
        let players = &self.server.players;
        let all = collect(
            &self.server.ecs,
            &self.server.mods.subgrids.borrow(),
            &|entity| players.by_entity(entity).map(|p| p.name.clone()),
        );
        let view = &mut self.client.view;
        let interval = view.entities.interval();
        let mut out = Vec::new();
        for player in self.server.players.iter() {
            // Le joueur n'a rien reçu du monde avant le premier envoi de chunks.
            let Some(sender) = view.senders.get(&player.id) else {
                continue;
            };
            let knows = |column| sender.has_column(column);
            let position = self
                .server
                .ecs
                .get::<crate::components::Transform>(player.entity)
                .map_or(glam::Vec3::ZERO, |t| t.position);
            let viewer = Viewer {
                entity: player.entity,
                position,
                knows: &knows,
            };
            view.trackers
                .entry(player.id)
                .or_insert_with(|| EntityTracker::new(interval))
                .step(view.tick, &viewer, &all, &mut out);
            self.server
                .replies
                .extend(out.drain(..).map(|msg| (player.id, msg)));
        }
        self.apply_replies();
    }

    /// Fin d'image : le serveur envoie ce qui a changé à chaque joueur, le client applique les messages
    /// reçus et sa réplique remaille (les éditions dans cette image, le reste en tâche de fond).
    pub(crate) fn sync_world_view(&mut self) {
        // Ce que le serveur a laissé pour la fenêtre : les durées des éditions (mesures), et un
        // changement de dimension à faire (recharger le monde).
        for time in std::mem::take(&mut self.server.edit_times) {
            self.client.frame_stats.record_edit(time);
        }
        if let Some(request) = self.server.travel.take() {
            self.client.request = Some(request);
        }
        let changes = self.streamer_mut().take_changes();
        let radius = self.client.view.replica().config().view_radius;
        let mut updates = Vec::new();
        for player in self.server.players.iter() {
            let eye = crate::player::eye(&self.server.ecs, player.entity, 1.0);
            let sender = self
                .client
                .view
                .senders
                .entry(player.id)
                .or_insert_with(|| ChunkSender::new(radius));
            sender.step(
                &self.server.world.borrow(),
                World::chunk_containing(eye),
                &changes,
                &mut updates,
            );
            // Les messages des autres joueurs partiront sur le réseau (lot 5).
            self.server.replies.extend(
                updates
                    .drain(..)
                    .map(|update| (player.id, update_msg(update))),
            );
        }
        self.replicate_player_views();
        self.apply_replies();
        let eye = crate::player::eye(&self.server.ecs, self.server.players.entity(), 1.0);
        let view = View {
            position: eye,
            forward: self.client.camera.forward(),
            mesh: true,
        };
        self.client.view.replica_mut().update_views(&[view]);
    }
}

/// Un message de chunk du serveur, tel que le client le reçoit.
fn update_msg(update: ChunkUpdate) -> ServerMsg {
    match update {
        ChunkUpdate::Chunk { pos, chunk } => ServerMsg::Chunk { pos, chunk },
        ChunkUpdate::Changed {
            pos,
            chunk,
            edits,
            remesh,
        } => ServerMsg::ChunkChanged {
            pos,
            chunk,
            edits,
            remesh,
        },
        ChunkUpdate::Unload(pos) => ServerMsg::Unload(pos),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_stats_take_the_loading_from_the_server_and_the_meshes_from_the_replica() {
        let server = StreamStats {
            loaded: 100,
            lit_columns: 9,
            queued: 4,
            in_flight: 2,
            candidates: 3,
            jobs_done: 10,
            ..StreamStats::default()
        };
        let replica = StreamStats {
            loaded: 60,
            in_flight: 1,
            meshes_waiting: 7,
            bytes_waiting: 4096,
            unloads_waiting: 5,
            candidates: 1,
            jobs_done: 20,
            jobs_discarded: 2,
            ..StreamStats::default()
        };
        let both = combined_stats(server, replica);
        assert_eq!((both.loaded, both.lit_columns, both.queued), (100, 9, 4));
        assert_eq!((both.meshes_waiting, both.bytes_waiting), (7, 4096));
        assert_eq!(both.unloads_waiting, 5);
        assert_eq!((both.in_flight, both.candidates), (3, 4));
        assert_eq!((both.jobs_done, both.jobs_discarded), (30, 2));
        assert_eq!(both.backlog(), 4 + 3 + 7 + 4);
    }
}
