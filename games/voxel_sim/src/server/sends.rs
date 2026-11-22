//! Ce que le serveur a déjà envoyé à chaque joueur, et ce qu'il lui envoie à chaque pas : les chunks
//! (`ChunkSender`), les entités (`EntityTracker`) et les vues du joueur (`PlayerViews`). C'est l'état
//! du serveur : la fenêtre n'en lit rien, elle reçoit les messages (`ServerMsg`).

use std::collections::HashMap;

use engine_voxel::{ChunkSender, ChunkUpdate, World};

use crate::net::entities::{EntityTracker, NetIds, Viewer, collect};
use crate::net::player_view::PlayerViews;
use crate::net::protocol::ServerMsg;
use crate::player::players::PlayerId;
use crate::server::VoxelServer;

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
    pub sky: crate::net::sky_view::SkyEcho,
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
    pub fn send_world(&mut self) {
        let changes = self.world.borrow_mut().take_changes();
        let radius = self.world.borrow().config().view_radius;
        let mut updates = Vec::new();
        for player in self.players.iter() {
            let eye = crate::player::eye(&self.ecs, player.entity, 1.0);
            // Un joueur du réseau dont le lien a du retard ne reçoit que les changements, le temps
            // de le rattraper (voir `net_host.rs`).
            let budget = if self.net_backlog(player.id) > crate::net::host::CHUNK_BACKLOG {
                0
            } else {
                engine_voxel::DEFAULT_SEND_BUDGET
            };
            let sender = self
                .sends
                .senders
                .entry(player.id)
                .or_insert_with(|| ChunkSender::new(radius));
            sender.set_budget(budget);
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
    pub fn send_entities(&mut self) {
        self.sends.tick += 1;
        NetIds::assign(&mut self.ecs);
        self.keep_ghosts_alive();
        let players = &self.players;
        let all = collect(&self.ecs, &self.mods.subgrids.borrow(), &|entity| {
            players.by_entity(entity).map(|p| {
                let look = crate::net::entities::Look {
                    held: p
                        .inventory
                        .selected_stack()
                        .map(|stack| crate::player::inventory::ItemStack { count: 1, ..stack }),
                    swings: p.swings,
                    digging: p.dig.is_some(),
                    ..crate::net::entities::Look::default()
                };
                (p.name.clone(), look)
            })
        });
        let interval = crate::server::sends::entity_interval();
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
        ChunkUpdate::Empty(pos) => ServerMsg::ChunkEmpty(pos),
        ChunkUpdate::Unload(pos) => ServerMsg::Unload(pos),
    }
}

/// Le nombre de pas fixes entre deux envois de positions : 1 sur le lien en mémoire (60 Hz, comme
/// l'ECS), 3 (20 Hz) sera celui du réseau. `VOXEL_ENTITY_INTERVAL` le change pour les essais.
pub fn entity_interval() -> u32 {
    std::env::var("VOXEL_ENTITY_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(1, |n: u32| n.clamp(1, 12))
}

#[cfg(test)]
mod empty_chunk_tests {
    use engine_voxel::{ChunkStreamer, ChunkUpdate, World};

    use crate::net::protocol::ServerMsg;
    use crate::server::boot::tests::headless;

    #[test]
    fn an_air_chunk_the_server_never_stores_is_ready_on_the_client() {
        let mut server = headless();
        let mut replica = ChunkStreamer::replica_of(&server.world.borrow());
        let feet = crate::player::body(&server.ecs, server.players.entity()).position;
        let column = World::chunk_containing(feet);
        // Le monde se charge et s'envoie jusqu'à ce que la colonne du joueur soit arrivée.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            server.stream_world();
            server.send_world();
            let updates: Vec<ChunkUpdate> = server
                .route_replies()
                .into_iter()
                .filter_map(|msg| match msg {
                    ServerMsg::Chunk { pos, chunk } => Some(ChunkUpdate::Chunk { pos, chunk }),
                    ServerMsg::ChunkEmpty(pos) => Some(ChunkUpdate::Empty(pos)),
                    _ => None,
                })
                .collect();
            replica.apply_updates(updates);
            if replica.is_generated(column) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "la colonne n'arrive pas"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Une couche de cette colonne générée vide par le serveur (de l'air, jamais stockée).
        let shared = server.world.clone();
        let streamer = shared.borrow();
        let air = streamer
            .world()
            .vertical()
            .map(|y| glam::IVec3::new(column.x, y, column.z))
            .find(|&pos| streamer.is_generated(pos) && streamer.world().chunk(pos).is_none())
            .expect("une couche d'air dans la colonne du joueur");
        assert!(
            replica.is_generated(air),
            "le client sait la couche d'air prête"
        );
        assert!(
            replica.world().chunk(air).is_none(),
            "et ne stocke rien pour elle"
        );
    }
}
