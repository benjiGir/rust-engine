//! Rejoindre la partie d'un hôte (`VOXEL_JOIN=adresse:port`).
//!
//! La fenêtre d'un invité garde un serveur, mais une **coquille** : construit pour le contenu (blocs,
//! objets, mods, générateur du terrain lointain, avec la graine de l'hôte), il n'avance jamais. Ses
//! messages partent sur la connexion au lieu de [`VoxelServer::handle`], ses réponses en arrivent au
//! lieu de `route_replies`, et la boucle de la fenêtre saute tout ce qui fait avancer le monde
//! (`hosting()` est faux). Les outils de l'hôte (`ctl`, captures) liraient la coquille : ils ne sont
//! pas faits pour l'invité.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use engine_net::{Connection, Event};
use glam::Vec3;

use crate::VoxelGame;
use crate::net_host::{Hello, PROTOCOL, Welcome, content_fingerprint};
use crate::players::PlayerId;
use crate::protocol::{ClientMsg, ServerMsg};
use crate::server_side::VoxelServer;

/// Le temps laissé à l'hôte pour répondre.
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

/// Le lien d'un invité avec son hôte.
pub struct RemoteLink {
    connection: Connection,
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

/// Se présente à l'hôte et attend sa réponse.
pub fn join(address: &str, server: &VoxelServer, name: &str, uuid: u128) -> Result<Joined> {
    let connection = Connection::connect(address, JOIN_TIMEOUT)
        .with_context(|| format!("impossible de joindre {address}"))?;
    let hello = Hello {
        protocol: PROTOCOL,
        name: name.to_string(),
        uuid,
        content: content_fingerprint(&server.registry, &server.content.items),
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

impl VoxelGame {
    /// Cette fenêtre fait tourner le monde (seule, ou en hôte) ; faux pour un invité.
    pub(crate) fn hosting(&self) -> bool {
        self.client.remote.is_none()
    }

    /// Un invité envoie ses messages à l'hôte.
    pub(crate) fn send_remote(&mut self, msg: &ClientMsg) {
        let Some(link) = &self.client.remote else {
            return;
        };
        let frame = engine_net::encode(&postcard::to_stdvec(msg).expect("un message se sérialise"));
        if link.connection.send(frame).is_err() {
            log::warn!("le lien avec l'hôte ne suit plus");
        }
    }

    /// Ce que l'hôte a envoyé depuis la dernière fois. Une connexion perdue ramène au menu.
    pub(crate) fn receive_remote(&mut self) -> Vec<ServerMsg> {
        let Some(link) = &self.client.remote else {
            return Vec::new();
        };
        let mut received = Vec::new();
        let mut lost = None;
        while let Some(event) = link.connection.try_recv() {
            match event {
                Event::Frame(bytes) => match postcard::from_bytes::<ServerMsg>(&bytes) {
                    Ok(msg) => received.push(msg),
                    Err(err) => {
                        lost = Some(format!("message illisible ({err})"));
                        break;
                    }
                },
                Event::Closed(reason) => {
                    lost = Some(reason);
                    break;
                }
            }
        }
        if let Some(reason) = lost {
            log::warn!("connexion à {} perdue : {reason}", link.address);
            link.connection.close();
            self.client.request = Some(crate::launch::Request::Title);
        }
        received
    }
}
