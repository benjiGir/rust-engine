//! Se présenter à un hôte (la poignée de main du client) : la fenêtre d'un invité et les bots
//! (`bot.rs`) s'en servent. Voir `net_client.rs` pour l'invité.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use engine_net::{Connection, Event};
use glam::Vec3;

use crate::net_host::{Hello, PROTOCOL, Welcome, content_fingerprint};
use crate::players::PlayerId;
use crate::server_side::VoxelServer;

/// Le temps laissé à l'hôte pour répondre.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Le lien d'un invité avec son hôte.
pub struct RemoteLink {
    pub(crate) connection: Connection,
    /// Le numéro que l'hôte nous a donné.
    pub player: PlayerId,
    pub address: String,
}

/// Ce que l'hôte a répondu à notre arrivée.
pub struct Joined {
    pub link: RemoteLink,
    pub feet: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fly: bool,
    pub seed: u32,
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
            } => Ok(Joined {
                link: RemoteLink {
                    connection,
                    player,
                    address: address.to_string(),
                },
                feet,
                yaw,
                pitch,
                fly,
                seed,
            }),
            Welcome::Refused(why) => bail!("{address} refuse : {why}"),
        },
        Some(Event::Closed(reason)) => bail!("{address} a fermé la connexion : {reason}"),
        None => bail!("{address} ne répond pas"),
    }
}
