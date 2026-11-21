//! Rejoindre la partie d'un hôte (`VOXEL_JOIN=adresse:port`).
//!
//! La fenêtre d'un invité garde un serveur, mais une **coquille** : construit pour le contenu (blocs,
//! objets, mods, générateur du terrain lointain, avec la graine de l'hôte), il n'avance jamais. Ses
//! messages partent sur la connexion au lieu de [`VoxelServer::handle`], ses réponses en arrivent au
//! lieu de `route_replies`, et la boucle de la fenêtre saute tout ce qui fait avancer le monde
//! (`hosting()` est faux). Les outils de l'hôte (`ctl`, captures) liraient la coquille : ils ne sont
//! pas faits pour l'invité.

use crate::VoxelGame;
#[allow(unused_imports)]
pub(crate) use crate::net_join::*;
use crate::protocol::{ClientMsg, ServerMsg};
use engine_net::Event;

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
