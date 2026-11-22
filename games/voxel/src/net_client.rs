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
pub(crate) use voxel_sim::net::join::*;
use voxel_sim::net::protocol::{ClientMsg, ServerMsg};

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
        if !link.send(msg) {
            log::warn!("le lien avec l'hôte ne suit plus");
        }
    }

    /// Ce que l'hôte a envoyé depuis la dernière fois. Une connexion perdue ramène au menu.
    pub(crate) fn receive_remote(&mut self) -> Vec<ServerMsg> {
        let Some(link) = &self.client.remote else {
            return Vec::new();
        };
        match link.receive() {
            Ok(received) => received,
            Err(reason) => {
                log::warn!("connexion à {} perdue : {reason}", link.address);
                link.close();
                self.client.request = Some(voxel_sim::launch::Request::Title);
                Vec::new()
            }
        }
    }
}
