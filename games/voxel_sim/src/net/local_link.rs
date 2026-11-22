//! Le lien en mémoire entre un serveur intégré et la fenêtre de la même application (voir
//! `dedicated::spawn_integrated`) : deux canaux, un par sens, qui passent les messages tels quels. Rien
//! n'est encodé : les chunks restent des `Arc` partagés, comme quand le serveur tourne dans le thread
//! de la fenêtre.

use std::sync::mpsc::{self, Receiver, Sender};

use crate::net::protocol::{ClientMsg, ServerMsg};

/// Le bout de la fenêtre.
pub struct LocalLink {
    pub to_server: Sender<ClientMsg>,
    pub from_server: Receiver<ServerMsg>,
}

/// Le bout du serveur.
pub struct LocalPeer {
    pub from_client: Receiver<ClientMsg>,
    pub to_client: Sender<ServerMsg>,
}

/// Un lien neuf : un bout pour chacun.
pub fn pair() -> (LocalLink, LocalPeer) {
    let (to_server, from_client) = mpsc::channel();
    let (to_client, from_server) = mpsc::channel();
    (
        LocalLink {
            to_server,
            from_server,
        },
        LocalPeer {
            from_client,
            to_client,
        },
    )
}
