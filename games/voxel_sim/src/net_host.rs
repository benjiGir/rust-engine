//! L'hôte d'une partie en réseau local : il écoute, accueille les joueurs et leur envoie ce que le
//! serveur leur dit.
//!
//! Le serveur ne change pas : un joueur venu du réseau est un [`PlayerState`] comme celui de la
//! fenêtre, son entité est prédite par sa propre fenêtre (`Predicted`), et ses messages passent
//! par [`VoxelServer::handle`]. Seul le trajet change : ses `ClientMsg` arrivent dans des trames
//! (`postcard`), et les réponses que le serveur range pour lui (`replies`) en repartent
//! ([`VoxelServer::route_replies`]).
//!
//! La poignée de main : le client envoie [`Hello`] (version du protocole, nom, identifiant,
//! empreinte du contenu) ; l'hôte répond [`Welcome`], accepté (où apparaître) ou refusé (pourquoi).
//! Un joueur retrouve son inventaire et sa place par son identifiant (`players/<id>.ron`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_net::{Connection, Event, Frame, Listener, SendError};
use engine_voxel::Chunk;
use glam::{IVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::attributes::BaseAttributes;
use crate::gear::Gear;
use crate::inventory::{self, Inventory};
use crate::player::{self, MoveMode};
use crate::players::{PlayerId, PlayerIdentity, PlayerState, uuid_name};
use crate::protocol::{ClientMsg, ServerMsg};
use crate::server_side::VoxelServer;

/// La version du protocole : un client d'une autre version est refusé. 2 : la portée de l'hôte dans
/// l'accueil, l'objet tenu et les gestes des joueurs dans leur apparence.
pub const PROTOCOL: u32 = 2;
/// Le port par défaut (le suivant s'il est pris).
pub const DEFAULT_PORT: u16 = 25600;
/// Au-delà de ces octets en attente vers un joueur, ses chunks neufs attendent.
pub const CHUNK_BACKLOG: usize = 2 << 20;
/// Le temps laissé à un client pour se présenter.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Ce que le client envoie en premier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub name: String,
    pub uuid: u128,
    /// L'empreinte des blocs et des objets (voir [`content_fingerprint`]).
    pub content: u64,
}

/// La réponse de l'hôte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Welcome {
    Accepted {
        player: PlayerId,
        /// Les pieds, le regard et le mode du joueur à son arrivée.
        feet: Vec3,
        yaw: f32,
        pitch: f32,
        fly: bool,
        /// La graine du monde (la météo de la fenêtre en tire ses nuages).
        seed: u32,
        /// La portée de l'hôte, en chunks : il n'envoie pas de chunks au-delà, et le terrain lointain
        /// de l'invité doit commencer là.
        view_radius: i32,
    },
    Refused(String),
}

/// L'empreinte du contenu : les blocs et les objets, dans l'ordre de leurs numéros. Deux jeux qui
/// ont la même empreinte donnent les mêmes numéros aux mêmes choses.
pub fn content_fingerprint(
    registry: &engine_voxel::BlockRegistry,
    items: &engine_voxel::ItemRegistry,
) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    for (id, def) in registry.iter() {
        (id.0, def.key.as_str()).hash(&mut hasher);
    }
    for (_, def) in items.iter() {
        def.key.hash(&mut hasher);
    }
    hasher.finish()
}

/// Une connexion qui ne s'est pas encore présentée.
struct Pending {
    connection: Connection,
    since: Instant,
}

/// L'écoute et les joueurs connectés.
pub struct NetHost {
    listener: Listener,
    pending: Vec<Pending>,
    peers: HashMap<PlayerId, Connection>,
    /// Les joueurs d'un lien en mémoire (la fenêtre d'un serveur intégré, voir `local_link.rs`).
    locals: HashMap<PlayerId, crate::local_link::LocalPeer>,
    /// Les trames des chunks envoyés dans cette image : un chunk encodé une fois pour tous les joueurs.
    /// L'`Arc` est gardé avec la trame : son adresse ne peut pas resservir tant qu'il est là.
    chunk_frames: HashMap<(IVec3, usize), (Arc<Chunk>, Frame)>,
    /// L'annonce de la partie sur le réseau local (`None` si l'UDP n'est pas disponible).
    announcer: Option<engine_net::Announcer>,
    /// Le nombre d'invités au plus (`None` : pas de limite).
    pub max_players: Option<usize>,
    /// Les noms admis (vide : tout le monde).
    pub whitelist: Vec<String>,
}

impl NetHost {
    pub fn port(&self) -> u16 {
        self.listener.port()
    }

    /// Change le nom annoncé sur le réseau local.
    pub fn set_announced_name(&mut self, name: &str) {
        let port = self.port();
        self.announcer = engine_net::Announcer::new(port, name, engine_net::ANNOUNCE_PORT).ok();
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

fn encode<T: Serialize>(value: &T) -> Frame {
    engine_net::encode(&postcard::to_stdvec(value).expect("un message se sérialise toujours"))
}

impl VoxelServer {
    /// Ouvre la partie au réseau local sur `port` (ou le suivant libre). Renvoie le port.
    pub fn open_lan(&mut self, port: u16) -> std::io::Result<u16> {
        if let Some(net) = &self.net {
            return Ok(net.port());
        }
        let listener = Listener::bind(port, 10)?;
        let port = listener.port();
        let name = format!("Partie de {}", self.players.local().name);
        let announcer = engine_net::Announcer::new(port, &name, engine_net::ANNOUNCE_PORT)
            .map_err(|err| log::warn!("annonce sur le réseau local impossible : {err}"))
            .ok();
        log::info!("partie ouverte au réseau local sur le port {port}");
        self.listen(listener, announcer);
        Ok(port)
    }

    /// Écoute sur la boucle locale seulement, sans annonce (le serveur intégré dans un thread, voir
    /// `dedicated::spawn_integrated`). Renvoie le port.
    pub fn open_local(&mut self) -> std::io::Result<u16> {
        let listener = Listener::bind_loopback()?;
        let port = listener.port();
        self.listen(listener, None);
        Ok(port)
    }

    fn listen(&mut self, listener: Listener, announcer: Option<engine_net::Announcer>) {
        self.net = Some(NetHost {
            listener,
            pending: Vec::new(),
            peers: HashMap::new(),
            locals: HashMap::new(),
            chunk_frames: HashMap::new(),
            announcer,
            max_players: None,
            whitelist: Vec::new(),
        });
    }

    /// Les connexions et les messages arrivés : nouveaux venus, poignées de main, `ClientMsg` des
    /// joueurs, départs.
    pub fn poll_net(&mut self) {
        let Some(net) = &mut self.net else {
            return;
        };
        if let Some(announcer) = &mut net.announcer {
            announcer.tick();
        }
        loop {
            match net.listener.accept() {
                Ok(Some(connection)) => {
                    log::info!("connexion de {}", connection.peer());
                    crate::net_join::simulate_lag_from_env(&connection);
                    net.pending.push(Pending {
                        connection,
                        since: Instant::now(),
                    });
                }
                Ok(None) => break,
                Err(err) => {
                    log::warn!("écoute : {err}");
                    break;
                }
            }
        }
        // Les présentations.
        let mut hellos = Vec::new();
        let pending = std::mem::take(&mut net.pending);
        for p in pending {
            match p.connection.try_recv() {
                Some(Event::Frame(bytes)) => match postcard::from_bytes::<Hello>(&bytes) {
                    Ok(hello) => hellos.push((p.connection, hello)),
                    Err(err) => {
                        log::warn!("{} : présentation illisible ({err})", p.connection.peer())
                    }
                },
                Some(Event::Closed(reason)) => {
                    log::info!("{} parti : {reason}", p.connection.peer())
                }
                None if p.since.elapsed() > HELLO_TIMEOUT => {
                    log::warn!("{} ne s'est pas présenté", p.connection.peer());
                }
                None => net.pending.push(p),
            }
        }
        for (connection, hello) in hellos {
            self.welcome(connection, hello);
        }
        // Les messages des joueurs.
        let Some(net) = &mut self.net else {
            return;
        };
        let mut inbox = Vec::new();
        let mut gone = Vec::new();
        for (&id, connection) in &net.peers {
            while let Some(event) = connection.try_recv() {
                match event {
                    Event::Frame(bytes) => match postcard::from_bytes::<ClientMsg>(&bytes) {
                        Ok(msg) => inbox.push((id, msg)),
                        Err(err) => {
                            log::warn!("message illisible de {id:?} : {err}");
                            gone.push((id, "message illisible".to_string()));
                            break;
                        }
                    },
                    Event::Closed(reason) => {
                        gone.push((id, reason));
                        break;
                    }
                }
            }
        }
        for (&id, peer) in &net.locals {
            loop {
                match peer.from_client.try_recv() {
                    Ok(msg) => inbox.push((id, msg)),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        gone.push((id, "fenêtre fermée".to_string()));
                        break;
                    }
                }
            }
        }
        for (id, msg) in inbox {
            if !gone.iter().any(|(g, _)| *g == id) {
                self.handle(id, msg);
            }
        }
        for (id, reason) in gone {
            self.leave(id, &reason);
        }
    }

    /// Accepte ou refuse un nouveau venu.
    fn welcome(&mut self, connection: Connection, hello: Hello) {
        let refuse = |connection: &Connection, why: String| {
            log::info!("{} refusé : {why}", connection.peer());
            let _ = connection.send(encode(&Welcome::Refused(why)));
        };
        if hello.protocol != PROTOCOL {
            refuse(
                &connection,
                format!(
                    "version du jeu différente (protocole {} ici, {} chez vous)",
                    PROTOCOL, hello.protocol
                ),
            );
            return;
        }
        let content = content_fingerprint(&self.registry, &self.content.items);
        if hello.content != content {
            refuse(
                &connection,
                "contenu différent (mods ou version)".to_string(),
            );
            return;
        }
        if self.players.by_uuid(hello.uuid).is_some() {
            refuse(&connection, "ce joueur est déjà dans la partie".to_string());
            return;
        }
        let name = hello.name.trim();
        let name = if name.is_empty() { "Invité" } else { name };
        if let Some(net) = &self.net {
            if net.max_players.is_some_and(|max| net.peers.len() >= max) {
                refuse(&connection, "la partie est pleine".to_string());
                return;
            }
            if !net.whitelist.is_empty() && !net.whitelist.iter().any(|w| w == name) {
                refuse(
                    &connection,
                    "vous n'êtes pas sur la liste blanche".to_string(),
                );
                return;
            }
        }
        let id = self.add_remote_player(name, hello.uuid);
        let state = self.players.get(id).expect("le joueur vient d'arriver");
        let feet = player::body(&self.ecs, state.entity).position;
        let fly = player::mode(&self.ecs, state.entity) == MoveMode::Fly;
        let (yaw, pitch) = (state.yaw, state.pitch);
        let seed = self.dims.base_seed;
        let view_radius = self.world.borrow().config().view_radius;
        let accepted = Welcome::Accepted {
            player: id,
            feet,
            yaw,
            pitch,
            fly,
            seed,
            view_radius,
        };
        if connection.send(encode(&accepted)).is_err() {
            self.leave(id, "parti pendant l'accueil");
            return;
        }
        log::info!("{name} a rejoint la partie ({})", connection.peer());
        if let Some(net) = &mut self.net {
            net.peers.insert(id, connection);
        }
        self.tell_everyone(&format!("{name} a rejoint la partie"));
        self.send_player_list();
    }

    /// Fait entrer un joueur par un lien en mémoire (la fenêtre d'un serveur intégré) : il n'y a pas de
    /// poignée de main, c'est la même application. Rend ce que la fenêtre en tire pour démarrer.
    pub fn join_local(&mut self, name: &str, uuid: u128) -> crate::net_join::Joined {
        let id = self.add_remote_player(name, uuid);
        let (link, peer) = crate::local_link::pair();
        if let Some(net) = &mut self.net {
            net.locals.insert(id, peer);
        }
        let state = self.players.get(id).expect("le joueur vient d'arriver");
        let feet = player::body(&self.ecs, state.entity).position;
        let fly = player::mode(&self.ecs, state.entity) == MoveMode::Fly;
        let (yaw, pitch) = (state.yaw, state.pitch);
        log::info!("{name} a rejoint la partie (lien en mémoire)");
        self.tell_everyone(&format!("{name} a rejoint la partie"));
        self.send_player_list();
        crate::net_join::Joined {
            link: crate::net_join::RemoteLink {
                transport: crate::net_join::Transport::Local(link),
                player: id,
                address: "serveur intégré".into(),
            },
            feet,
            yaw,
            pitch,
            fly,
            seed: self.dims.base_seed,
            view_radius: self.world.borrow().config().view_radius,
        }
    }

    /// Où faire apparaître un nouveau venu : le point de départ s'il est seul, sinon une place libre
    /// autour (à 2, puis 3 et 4 blocs), pour que deux joueurs n'arrivent pas l'un dans l'autre.
    fn arrival_spot(&self) -> Vec3 {
        let spawn = self.spawn;
        let center = spawn.as_vec3() + Vec3::new(0.5, 0.0, 0.5);
        let others = self.players.iter().count();
        if others == 0 {
            return center;
        }
        let streamer = self.world.borrow();
        let world = streamer.world();
        let directions = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
        ];
        for distance in 2..=4 {
            for k in 0..directions.len() {
                let (dx, dz) = directions[(others + k) % directions.len()];
                let column = spawn + IVec3::new(dx * distance, 0, dz * distance);
                // Le sol le plus haut, de quelques blocs au-dessus à quelques blocs en dessous.
                if let Some(floor) = (-4..=4)
                    .rev()
                    .map(|dy| column + IVec3::new(0, dy - 1, 0))
                    .find(|&floor| {
                        world.is_solid(floor)
                            && !world.is_solid(floor + IVec3::Y)
                            && !world.is_solid(floor + IVec3::Y * 2)
                            && world.block(floor + IVec3::Y).is_air()
                    })
                {
                    return (floor + IVec3::Y).as_vec3() + Vec3::new(0.5, 0.0, 0.5);
                }
            }
        }
        center
    }

    /// Un joueur venu du réseau : son fichier de `players/` s'il est déjà venu, sinon le point de
    /// départ et le nécessaire de départ.
    pub fn add_remote_player(&mut self, name: &str, uuid: u128) -> PlayerId {
        let store = self.dims.meta_store.clone().or_else(|| self.store.clone());
        let file = store.and_then(|s| match s.load_player(&uuid_name(uuid)) {
            Ok(file) => file,
            Err(err) => {
                log::error!("fichier du joueur {name} : {err:#}");
                None
            }
        });
        let spawn = self.arrival_spot();
        let (eye, mode) = file.as_ref().map_or(
            (
                spawn + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
                MoveMode::Walk,
            ),
            |f| {
                let mode = if f.player.mode == "fly" {
                    MoveMode::Fly
                } else {
                    MoveMode::Walk
                };
                (Vec3::from(f.player.position), mode)
            },
        );
        let entity = player::spawn(&mut self.ecs, eye, mode);
        self.ecs.insert(entity, crate::components::Predicted);
        let items = &self.content.items;
        let inventory = file.as_ref().and_then(|f| f.items.as_ref()).map_or_else(
            || inventory::starter_inventory(items),
            |saved| Inventory::from_save(saved, items),
        );
        let id = self.players.next_id();
        let mut state = PlayerState::new(
            PlayerIdentity {
                id,
                name: name.to_string(),
                uuid,
            },
            entity,
            inventory,
            Gear::new(self.base_attributes()),
            &self.survival,
            &self.hunger_rules,
        );
        if let Some(file) = &file {
            state.apply_save(&file.player);
            state.yaw = file.player.yaw;
            state.pitch = file.player.pitch;
            state.adventure.progress = crate::tiers::Progress::from_save(
                &file.adventure,
                &crate::adventure_data::current().tiers,
            );
        }
        state.synced = Some(eye - Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT);
        self.players.add(state);
        self.players.sync_ecs(&mut self.ecs);
        id
    }

    /// Les valeurs de base des attributs d'un nouveau joueur (celles du joueur de la fenêtre).
    fn base_attributes(&self) -> BaseAttributes {
        self.players.local().gear.base()
    }

    /// Un joueur s'en va : il est sauvegardé, son entité disparaît, les autres sont prévenus.
    pub fn leave(&mut self, id: PlayerId, reason: &str) {
        if let Some(net) = &mut self.net {
            if let Some(connection) = net.peers.remove(&id) {
                connection.close();
            }
            net.locals.remove(&id);
        }
        let Some(state) = self.players.get(id) else {
            return;
        };
        let name = state.name.clone();
        let store = self.dims.meta_store.clone().or_else(|| self.store.clone());
        if let Some(store) = store
            && let Err(err) = store.save_player(&uuid_name(state.uuid), &self.player_file(state))
        {
            log::error!("sauvegarde du joueur {name} : {err:#}");
        }
        if let Some(state) = self.players.remove(id) {
            self.ecs.despawn(state.entity);
            self.players.sync_ecs(&mut self.ecs);
            self.sends.forget(id);
        }
        log::info!("{name} est parti ({reason})");
        self.tell_everyone(&format!("{name} a quitté la partie"));
        self.send_player_list();
    }

    /// La liste des joueurs, à tous (les fantômes n'y sont pas).
    pub fn send_player_list(&mut self) {
        let names: Vec<String> = self
            .players
            .iter()
            .filter(|p| !p.ghost)
            .map(|p| p.name.clone())
            .collect();
        let ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            self.replies
                .push((id, ServerMsg::PlayerList(names.clone())));
        }
    }

    /// Une ligne de chat pour tous les joueurs.
    pub fn tell_everyone(&mut self, line: &str) {
        let ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            self.replies.push((id, ServerMsg::Chat(line.to_string())));
        }
    }

    /// Les réponses du serveur : celles du joueur de la fenêtre sont rendues, celles des joueurs du
    /// réseau partent sur leur connexion (celles d'un fantôme sont oubliées).
    pub fn route_replies(&mut self) -> Vec<ServerMsg> {
        let local = self.players.local_id();
        let mut mine = Vec::new();
        let mut full = Vec::new();
        for (player, reply) in std::mem::take(&mut self.replies) {
            if player == local {
                mine.push(reply);
                continue;
            }
            let Some(net) = &mut self.net else {
                continue;
            };
            if let Some(peer) = net.locals.get(&player) {
                // En mémoire : le message passe tel quel, sans encodage.
                if peer.to_client.send(reply).is_err() {
                    full.push((player, "fenêtre fermée"));
                }
                continue;
            }
            let Some(connection) = net.peers.get(&player) else {
                continue;
            };
            let frame = match &reply {
                ServerMsg::Chunk { pos, chunk } => {
                    let key = (*pos, Arc::as_ptr(chunk) as usize);
                    net.chunk_frames
                        .entry(key)
                        .or_insert_with(|| (chunk.clone(), encode(&reply)))
                        .1
                        .clone()
                }
                _ => encode(&reply),
            };
            match connection.send(frame) {
                Ok(()) => {}
                Err(SendError::Full) => full.push((player, "le lien ne suit plus")),
                Err(SendError::Closed) => full.push((player, "connexion fermée")),
            }
        }
        for (player, reason) in full {
            self.leave(player, reason);
        }
        mine
    }

    /// Fin d'image : les trames de chunks gardées pour cette image sont oubliées.
    pub fn end_net_frame(&mut self) {
        if let Some(net) = &mut self.net {
            net.chunk_frames.clear();
        }
    }

    /// Les octets qui attendent de partir vers ce joueur (0 s'il n'est pas du réseau).
    pub fn net_backlog(&self, player: PlayerId) -> usize {
        self.net
            .as_ref()
            .and_then(|n| n.peers.get(&player))
            .map_or(0, Connection::queued_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(20);

    /// Une image de l'hôte sans fenêtre : le réseau, le monde, un pas, les réponses.
    fn pump(server: &mut VoxelServer) {
        server.poll_net();
        server.frame(1.0 / 60.0);
        server.stream_world();
        server.send_world();
        server.fixed_step(1.0 / 60.0);
        server.replicate_player_views();
        server.route_replies();
        server.end_net_frame();
    }

    /// Fait tourner l'hôte jusqu'à ce que le client reçoive un message qui convienne.
    fn wait_for<T>(
        server: &mut VoxelServer,
        client: &Connection,
        mut want: impl FnMut(&[u8]) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            pump(server);
            while let Some(event) = client.try_recv() {
                match event {
                    Event::Frame(bytes) => {
                        if let Some(found) = want(&bytes) {
                            return found;
                        }
                    }
                    Event::Closed(reason) => panic!("connexion fermée : {reason}"),
                }
            }
            assert!(Instant::now() < deadline, "rien reçu à temps");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn hello(server: &VoxelServer, name: &str, uuid: u128) -> Frame {
        encode(&Hello {
            protocol: PROTOCOL,
            name: name.to_string(),
            uuid,
            content: content_fingerprint(&server.registry, &server.content.items),
        })
    }

    #[test]
    fn a_player_joins_over_the_network_plays_and_leaves() {
        let mut server = crate::server_boot::tests::headless();
        let port = server.open_lan(0).unwrap();
        let address = format!("127.0.0.1:{port}");
        let client = Connection::connect(&address, WAIT).unwrap();
        client.send(hello(&server, "Alice", 42)).unwrap();
        let (id, feet) =
            wait_for(
                &mut server,
                &client,
                |bytes| match postcard::from_bytes::<Welcome>(bytes).ok()? {
                    Welcome::Accepted { player, feet, .. } => Some((player, feet)),
                    Welcome::Refused(why) => panic!("refusé : {why}"),
                },
            );
        assert_eq!(server.net.as_ref().unwrap().peer_count(), 1);
        let entity = server.players.get(id).unwrap().entity;
        assert!(server.ecs.has::<crate::components::Predicted>(entity));
        // La liste des joueurs arrive à chacun.
        let names = wait_for(
            &mut server,
            &client,
            |bytes| match postcard::from_bytes::<ServerMsg>(bytes).ok()? {
                ServerMsg::PlayerList(names) => Some(names),
                _ => None,
            },
        );
        assert!(names.contains(&"Alice".to_string()) && names.len() == 2);
        // Le monde arrive : des chunks, encodés pour le réseau.
        wait_for(&mut server, &client, |bytes| {
            matches!(
                postcard::from_bytes::<ServerMsg>(bytes),
                Ok(ServerMsg::Chunk { .. })
            )
            .then_some(())
        });
        assert!(server.net_backlog(id) < 64 << 20);
        // Ses mouvements arrivent au serveur comme ceux de la fenêtre.
        let moved = feet + Vec3::X * 0.1;
        let step = ClientMsg::Move {
            position: moved,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            in_liquid: false,
            walk: Vec3::ZERO,
            jump: false,
            frozen: false,
            mode: MoveMode::Walk,
            step: 1,
        };
        client.send(encode(&step)).unwrap();
        let deadline = Instant::now() + WAIT;
        while player::body(&server.ecs, entity).position != moved {
            pump(&mut server);
            assert!(Instant::now() < deadline, "le mouvement n'est pas arrivé");
            std::thread::sleep(Duration::from_millis(1));
        }
        // Un second client, d'un autre contenu, est refusé ; le même joueur aussi.
        let other = Connection::connect(&address, WAIT).unwrap();
        let mut wrong = Hello {
            protocol: PROTOCOL,
            name: "Bob".into(),
            uuid: 7,
            content: 0,
        };
        wrong.content ^= content_fingerprint(&server.registry, &server.content.items) ^ 1;
        other.send(encode(&wrong)).unwrap();
        let why = wait_for(&mut server, &other, |bytes| {
            match postcard::from_bytes::<Welcome>(bytes).ok()? {
                Welcome::Refused(why) => Some(why),
                Welcome::Accepted { .. } => panic!("un autre contenu est accepté"),
            }
        });
        assert!(why.contains("contenu"));
        let twin = Connection::connect(&address, WAIT).unwrap();
        twin.send(hello(&server, "Alice", 42)).unwrap();
        let why = wait_for(&mut server, &twin, |bytes| {
            match postcard::from_bytes::<Welcome>(bytes).ok()? {
                Welcome::Refused(why) => Some(why),
                Welcome::Accepted { .. } => panic!("le même joueur entre deux fois"),
            }
        });
        assert!(why.contains("déjà"));
        // Il s'en va : son entité disparaît.
        drop(client);
        let deadline = Instant::now() + WAIT;
        while server.players.get(id).is_some() {
            pump(&mut server);
            assert!(Instant::now() < deadline, "le départ n'est pas vu");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!server.ecs.is_alive(entity));
    }
}
