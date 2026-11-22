//! Se présenter à un hôte (la poignée de main du client) : la fenêtre d'un invité et les bots
//! (`bot.rs`) s'en servent. Voir `net_client.rs` pour l'invité.

use crate::protocol::{ClientMsg, ServerMsg};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use engine_net::{Connection, Event};
use glam::Vec3;

use crate::net_host::{Hello, PROTOCOL, Welcome, content_fingerprint};
use crate::players::PlayerId;
use crate::server_side::VoxelServer;

/// Le temps laissé à l'hôte pour répondre.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Par où passent les messages : le réseau, ou un lien en mémoire avec un serveur intégré.
pub enum Transport {
    Net(Connection),
    Local(crate::local_link::LocalLink),
}

/// Le lien d'un invité avec son hôte.
pub struct RemoteLink {
    pub transport: Transport,
    /// Le numéro que l'hôte nous a donné.
    pub player: PlayerId,
    pub address: String,
}

impl RemoteLink {
    /// La connexion réseau (un bot, un invité), s'il en a une.
    pub fn connection(&self) -> Option<&Connection> {
        match &self.transport {
            Transport::Net(connection) => Some(connection),
            Transport::Local(_) => None,
        }
    }

    /// Envoie un message ; faux si le lien ne suit plus.
    pub fn send(&self, msg: &ClientMsg) -> bool {
        match &self.transport {
            Transport::Net(connection) => {
                let bytes = postcard::to_stdvec(msg).expect("un message se sérialise");
                connection.send(engine_net::encode(&bytes)).is_ok()
            }
            Transport::Local(link) => link.to_server.send(msg.clone()).is_ok(),
        }
    }

    /// Ce que l'hôte a envoyé depuis la dernière fois ; une erreur si le lien est perdu (avec ce qui
    /// était arrivé avant, perdu aussi).
    pub fn receive(&self) -> Result<Vec<ServerMsg>, String> {
        let mut received = Vec::new();
        match &self.transport {
            Transport::Net(connection) => {
                while let Some(event) = connection.try_recv() {
                    match event {
                        engine_net::Event::Frame(bytes) => {
                            let msg = postcard::from_bytes::<ServerMsg>(&bytes)
                                .map_err(|err| format!("message illisible ({err})"))?;
                            received.push(msg);
                        }
                        engine_net::Event::Closed(reason) => return Err(reason),
                    }
                }
            }
            Transport::Local(link) => loop {
                match link.from_server.try_recv() {
                    Ok(msg) => received.push(msg),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return Err("le serveur intégré s'est arrêté".into());
                    }
                }
            },
        }
        Ok(received)
    }

    pub fn close(&self) {
        if let Transport::Net(connection) = &self.transport {
            connection.close();
        }
    }
}

/// Ce que l'hôte a répondu à notre arrivée.
pub struct Joined {
    pub link: RemoteLink,
    pub feet: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fly: bool,
    pub seed: u32,
    /// La portée de l'hôte, en chunks (voir `Welcome::Accepted`).
    pub view_radius: i32,
}

/// Le nom et l'identifiant sous lesquels se présenter : ceux des réglages, ou `VOXEL_NAME` et
/// `VOXEL_UUID` (un second joueur sur la même machine, pour les essais).
pub fn identity(settings: &crate::settings::Settings) -> (String, u128) {
    let name = std::env::var("VOXEL_NAME").unwrap_or_else(|_| settings.player_name.clone());
    let uuid = std::env::var("VOXEL_UUID")
        .ok()
        .and_then(|u| crate::players::parse_uuid(&u))
        .or_else(|| crate::players::parse_uuid(&settings.player_uuid))
        .unwrap_or_default();
    (name, uuid)
}

/// Se présente à l'hôte et attend sa réponse.
pub fn join(address: &str, server: &VoxelServer, name: &str, uuid: u128) -> Result<Joined> {
    let content = content_fingerprint(&server.registry, &server.content.items);
    join_with(address, name, uuid, content)
}

/// Se présente avec une empreinte de contenu déjà calculée (les bots, voir `bot.rs`).
pub fn join_with(address: &str, name: &str, uuid: u128, content: u64) -> Result<Joined> {
    let connection = Connection::connect(address, JOIN_TIMEOUT)
        .with_context(|| format!("impossible de joindre {address}"))?;
    simulate_lag_from_env(&connection);
    let hello = Hello {
        protocol: PROTOCOL,
        name: name.to_string(),
        uuid,
        content,
    };
    let frame = engine_net::encode(&postcard::to_stdvec(&hello)?);
    if connection.send(frame).is_err() {
        bail!("{address} a fermé la connexion");
    }
    match connection.recv_timeout(JOIN_TIMEOUT) {
        Some(Event::Frame(bytes)) => match postcard::from_bytes::<Welcome>(&bytes)? {
            Welcome::Accepted {
                player,
                feet,
                yaw,
                pitch,
                fly,
                seed,
                view_radius,
            } => Ok(Joined {
                link: RemoteLink {
                    transport: Transport::Net(connection),
                    player,
                    address: address.to_string(),
                },
                feet,
                yaw,
                pitch,
                fly,
                seed,
                view_radius,
            }),
            Welcome::Refused(why) => bail!("{address} refuse : {why}"),
        },
        Some(Event::Closed(reason)) => bail!("{address} a fermé la connexion : {reason}"),
        None => bail!("{address} ne répond pas"),
    }
}

/// `VOXEL_NET_LAG=ms` et `VOXEL_NET_JITTER=ms` : un lien lent simulé (voir
/// `engine_net::Connection::simulate_lag`), pour éprouver la prédiction et l'interpolation.
pub fn simulate_lag_from_env(connection: &Connection) {
    let ms = |name: &str| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(Duration::ZERO, Duration::from_millis)
    };
    let (lag, jitter) = (ms("VOXEL_NET_LAG"), ms("VOXEL_NET_JITTER"));
    if !lag.is_zero() || !jitter.is_zero() {
        log::info!("lien lent simulé : {lag:?} + jusqu'à {jitter:?}");
        connection.simulate_lag(lag, jitter);
    }
}
