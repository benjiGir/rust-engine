//! Un bot : un vrai client TCP, sans fenêtre, pour tester et mesurer le multijoueur.
//!
//! Il se présente comme une fenêtre (`net_client::join_with`), range les chunks reçus dans son
//! propre [`World`], suit sa position (les `Correct` du serveur), son inventaire et le conteneur
//! ouvert (les vues), et envoie des `ClientMsg` : se déplacer en suivant le sol, viser un bloc,
//! l'utiliser (poser, ouvrir), parler. Il n'a pas de physique : il se pose sur le premier bloc solide
//! sous ses pieds, ce que le serveur accepte (le contrôle des mouvements n'est pas actif par défaut).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use engine_net::Event;
use engine_voxel::{BlockId, BlockRegistry, PLAYER_EYE_HEIGHT, World};
use glam::{IVec3, Vec3};

use crate::container_view::ContainerView;
use crate::net_join::{RemoteLink, join_with};
use crate::player::{MoveMode, WALK_SPEED};
use crate::player_view::InventoryView;
use crate::players::PlayerId;
use crate::protocol::{Aim, ClientMsg, ServerMsg};

/// Ce qu'un bot fait de lui-même à chaque pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Il reste où il est.
    Idle,
    /// Il marche tout droit.
    Walk,
    /// Il marche et change de direction de temps en temps.
    Explore,
}

impl Behaviour {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "idle" => Some(Self::Idle),
            "walk" => Some(Self::Walk),
            "explore" => Some(Self::Explore),
            _ => None,
        }
    }
}

/// Un client sans fenêtre.
pub struct Bot {
    pub name: String,
    link: RemoteLink,
    pub player: PlayerId,
    /// Les chunks reçus.
    pub world: World,
    /// Les pieds.
    pub feet: Vec3,
    pub inventory: InventoryView,
    pub container: Option<ContainerView>,
    pub chat: Vec<String>,
    /// Les octets reçus (les trames décompressées).
    pub received_bytes: usize,
    pub chunks_received: usize,
    step: u64,
    yaw: f32,
}

impl Bot {
    /// Rejoint `address` sous ce nom et cet identifiant ; `content` est l'empreinte du contenu
    /// (`net_host::content_fingerprint`), `registry` celui des blocs (pour relire le monde).
    pub fn join(
        address: &str,
        name: &str,
        uuid: u128,
        content: u64,
        registry: Arc<BlockRegistry>,
    ) -> Result<Self> {
        let joined = join_with(address, name, uuid, content)?;
        Ok(Self {
            name: name.to_string(),
            player: joined.link.player,
            link: joined.link,
            world: World::with_registry(registry),
            feet: joined.feet,
            inventory: InventoryView::default(),
            container: None,
            chat: Vec::new(),
            received_bytes: 0,
            chunks_received: 0,
            step: 0,
            yaw: (uuid % 628) as f32 / 100.0,
        })
    }

    /// Les octets arrivés sur le fil (compressés), depuis l'arrivée.
    pub fn wire_bytes(&self) -> usize {
        self.link.connection.received_bytes()
    }

    pub fn send(&self, msg: &ClientMsg) {
        let bytes = postcard::to_stdvec(msg).expect("un message se sérialise");
        // Une file pleine ou une connexion fermée se voit au prochain `poll`.
        let _ = self.link.connection.send(engine_net::encode(&bytes));
    }

    /// Applique ce que le serveur a envoyé. Une connexion fermée est une erreur.
    pub fn poll(&mut self) -> Result<()> {
        while let Some(event) = self.link.connection.try_recv() {
            let bytes = match event {
                Event::Frame(bytes) => bytes,
                Event::Closed(reason) => bail!("{} : connexion fermée ({reason})", self.name),
            };
            self.received_bytes += bytes.len();
            match postcard::from_bytes::<ServerMsg>(&bytes)? {
                ServerMsg::Chunk { pos, chunk } | ServerMsg::ChunkChanged { pos, chunk, .. } => {
                    self.chunks_received += 1;
                    self.world.insert(pos, chunk);
                }
                ServerMsg::Unload(pos) => {
                    self.world.remove(pos);
                }
                ServerMsg::Correct { position, .. } => self.feet = position,
                ServerMsg::Inventory(inventory) => self.inventory = inventory,
                ServerMsg::Container(container) => self.container = container,
                ServerMsg::Chat(line) => self.chat.push(line),
                _ => {}
            }
        }
        Ok(())
    }

    /// Attend, en appelant `pump` (l'hôte, dans un test) entre deux lectures, que `done` soit vrai.
    pub fn wait_until(
        &mut self,
        timeout: Duration,
        mut pump: impl FnMut(),
        mut done: impl FnMut(&Self) -> bool,
    ) -> Result<()> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            pump();
            self.poll()?;
            if done(self) {
                return Ok(());
            }
            if std::time::Instant::now() > deadline {
                bail!("{} : délai dépassé", self.name);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.feet + Vec3::Y * PLAYER_EYE_HEIGHT
    }

    pub fn block(&self, pos: IVec3) -> BlockId {
        self.world.block(pos)
    }

    /// Le haut du premier bloc solide sous `at` (cherché de 4 blocs au-dessus à 24 en dessous), si
    /// la colonne est arrivée.
    pub fn ground_below(&self, at: Vec3) -> Option<f32> {
        let x = at.x.floor() as i32;
        let z = at.z.floor() as i32;
        let top = at.y.floor() as i32 + 4;
        let column = World::chunk_containing(Vec3::new(at.x, at.y, at.z));
        self.world.chunk(column)?;
        (top - 28..=top)
            .rev()
            .find(|&y| self.world.is_solid(IVec3::new(x, y, z)))
            .map(|y| y as f32 + 1.0)
    }

    /// Va à `feet` (le serveur range le mouvement comme celui d'une fenêtre).
    pub fn move_to(&mut self, feet: Vec3) {
        self.step += 1;
        self.feet = feet;
        let msg = ClientMsg::Move {
            position: feet,
            velocity: Vec3::ZERO,
            yaw: self.yaw,
            pitch: 0.0,
            on_ground: true,
            in_liquid: false,
            walk: Vec3::ZERO,
            jump: false,
            frozen: false,
            mode: MoveMode::Walk,
            step: self.step,
        };
        self.send(&msg);
    }

    /// Un pas de `dt` secondes de son comportement.
    pub fn act(&mut self, behaviour: Behaviour, dt: f32) {
        if behaviour == Behaviour::Idle {
            return;
        }
        if behaviour == Behaviour::Explore && self.step % 180 == 179 {
            self.yaw += 1.3;
        }
        let direction = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
        let next = self.feet + direction * WALK_SPEED * dt;
        match self.ground_below(next) {
            // Une marche d'un bloc au plus : au-delà, il tourne.
            Some(ground) if (ground - self.feet.y).abs() <= 1.0 => {
                self.move_to(Vec3::new(next.x, ground, next.z));
            }
            Some(_) => self.yaw += 2.0,
            None => {}
        }
    }

    /// La visée vers le centre d'un bloc.
    pub fn aim_at(&self, block: IVec3) -> Aim {
        let eye = self.eye();
        Aim {
            eye,
            forward: (block.as_vec3() + Vec3::splat(0.5) - eye).normalize_or(Vec3::NEG_Y),
        }
    }

    /// Le clic droit sur un bloc : poser ce qu'il tient contre la face visée, ouvrir, utiliser.
    pub fn use_on(&self, block: IVec3) {
        self.send(&ClientMsg::Use {
            aim: self.aim_at(block),
            sneaking: false,
        });
    }

    pub fn say(&self, text: &str) {
        self.send(&ClientMsg::Chat(text.to_string()));
    }

    /// Se déconnecte.
    pub fn leave(self) {
        self.link.connection.close();
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::net_host::content_fingerprint;
    use crate::server_side::VoxelServer;

    pub const WAIT: Duration = Duration::from_secs(30);

    /// Une image de l'hôte sans fenêtre.
    pub fn pump(server: &mut VoxelServer) {
        server.poll_net();
        server.frame(1.0 / 60.0);
        server.stream_world();
        server.send_world();
        server.fixed_step(1.0 / 60.0);
        server.replicate_player_views();
        server.route_replies();
        server.end_net_frame();
    }

    pub fn bot(server: &mut VoxelServer, port: u16, name: &str, uuid: u128) -> Bot {
        let content = content_fingerprint(&server.registry, &server.content.items);
        let registry = server.registry.clone();
        let address = format!("127.0.0.1:{port}");
        // La poignée de main attend l'hôte : il tourne dans un autre thread le temps de l'accueil.
        let handle = std::thread::spawn({
            let name = name.to_string();
            move || Bot::join(&address, &name, uuid, content, registry)
        });
        while !handle.is_finished() {
            pump(server);
            std::thread::sleep(Duration::from_millis(1));
        }
        handle.join().unwrap().expect("le bot est accueilli")
    }

    #[test]
    fn a_bot_receives_the_world_and_walks_on_it() {
        let mut server = crate::server_boot::tests::headless();
        let port = server.open_lan(0).unwrap();
        let mut bot = bot(&mut server, port, "Robot", 9);
        let start = bot.feet;
        bot.wait_until(
            WAIT,
            || pump(&mut server),
            |b| b.ground_below(b.feet).is_some(),
        )
        .unwrap();
        assert!(bot.chunks_received > 0 && bot.received_bytes > 0);
        for _ in 0..60 {
            bot.act(Behaviour::Walk, 1.0 / 60.0);
            pump(&mut server);
            bot.poll().unwrap();
        }
        // Le dernier pas, encore en route.
        let entity = server.players.get(bot.player).unwrap().entity;
        let last = bot.feet;
        let deadline = std::time::Instant::now() + WAIT;
        while crate::player::body(&server.ecs, entity)
            .position
            .distance(last)
            > 0.01
        {
            pump(&mut server);
            assert!(
                std::time::Instant::now() < deadline,
                "le dernier pas n'arrive pas"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let entity = server.players.get(bot.player).unwrap().entity;
        let seen = crate::player::body(&server.ecs, entity).position;
        assert!(seen.distance(start) > 1.0, "le serveur l'a vu marcher");
        assert!(seen.distance(bot.feet) < 0.01);
    }
}
