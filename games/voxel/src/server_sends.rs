//! Ce que le serveur a déjà envoyé à chaque joueur, et ce qu'il lui envoie à chaque pas : les chunks
//! (`ChunkSender`), les entités (`EntityTracker`) et les vues du joueur (`PlayerViews`). C'est l'état
//! du serveur : la fenêtre n'en lit rien, elle reçoit les messages (`ServerMsg`).

use std::collections::HashMap;

use engine_voxel::{ChunkSender, ChunkUpdate, World};

use crate::entities::{EntityTracker, NetIds, Viewer, collect};
use crate::player_view::PlayerViews;
use crate::players::PlayerId;
use crate::protocol::ServerMsg;
use crate::server_side::VoxelServer;

/// Les envois du serveur, joueur par joueur.
#[derive(Default)]
pub struct ServerSends {
    /// Ce que chaque joueur a reçu du monde.
    pub senders: HashMap<PlayerId, ChunkSender>,
    /// Ce que chaque joueur a reçu des entités.
    pub trackers: HashMap<PlayerId, EntityTracker>,
    /// Ce que le serveur a envoyé en dernier des jauges, de l'inventaire et des écrans de chaque joueur.
    pub player_views: PlayerViews,
    /// Le pas fixe du serveur auquel en sont les entités.
    pub tick: u64,
    /// Ce qu'a la fenêtre du ciel (voir `sky_view.rs`).
    pub sky: crate::sky_view::SkyEcho,
}

impl ServerSends {
    /// Un joueur est parti : on oublie ce qu'on lui avait envoyé.
    pub fn forget(&mut self, id: PlayerId) {
        self.senders.remove(&id);
        self.trackers.remove(&id);
        self.player_views.forget(id);
    }

    /// Plus aucun chunk en attente d'envoi.
    pub fn idle(&self) -> bool {
        self.senders.values().all(|s| s.pending() == 0)
    }
}

impl VoxelServer {
    /// Fin d'image : chaque joueur reçoit les chunks de sa portée et ceux qui ont changé.
    pub(crate) fn send_world(&mut self) {
        let changes = self.world.borrow_mut().take_changes();
        let radius = self.world.borrow().config().view_radius;
        let mut updates = Vec::new();
        for player in self.players.iter() {
            let eye = crate::player::eye(&self.ecs, player.entity, 1.0);
            let sender = self
                .sends
                .senders
                .entry(player.id)
                .or_insert_with(|| ChunkSender::new(radius));
            sender.step(
                &self.world.borrow(),
                World::chunk_containing(eye),
                &changes,
                &mut updates,
            );
            self.replies.extend(
                updates
                    .drain(..)
                    .map(|update| (player.id, update_msg(update))),
            );
        }
    }

    /// Un pas fixe : un numéro pour les entités neuves, puis, pour chaque joueur, ce qui entre dans sa
    /// portée, en sort, bouge ou change d'apparence.
    pub(crate) fn send_entities(&mut self) {
        self.sends.tick += 1;
        NetIds::assign(&mut self.ecs);
        self.keep_ghosts_alive();
        let players = &self.players;
        let all = collect(&self.ecs, &self.mods.subgrids.borrow(), &|entity| {
            players.by_entity(entity).map(|p| p.name.clone())
        });
        let interval = crate::client_world::entity_interval();
        let tick = self.sends.tick;
        let mut out = Vec::new();
        for player in self.players.iter() {
            // Le joueur n'a rien reçu du monde avant le premier envoi de chunks.
            let Some(sender) = self.sends.senders.get(&player.id) else {
                continue;
            };
            let knows = |column| sender.has_column(column);
            let position = self
                .ecs
                .get::<crate::components::Transform>(player.entity)
                .map_or(glam::Vec3::ZERO, |t| t.position);
            let viewer = Viewer {
                entity: player.entity,
                position,
                knows: &knows,
            };
            self.sends
                .trackers
                .entry(player.id)
                .or_insert_with(|| EntityTracker::new(interval))
                .step(tick, &viewer, &all, &mut out);
            self.replies
                .extend(out.drain(..).map(|msg| (player.id, msg)));
        }
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
