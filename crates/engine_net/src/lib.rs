//! Le réseau du moteur, sans rien savoir du jeu : des trames d'octets compressées
//! ([`frame`]), des connexions TCP qui lisent et écrivent sur leurs propres threads
//! ([`connection`]), et l'annonce des parties sur le réseau local ([`lan`]). Le jeu choisit
//! ce qu'il met dans les trames (`postcard` pour le voxel).

pub mod connection;
pub mod frame;
pub mod lan;

pub use connection::{Connection, Event, Listener, SendError};
pub use frame::{Frame, FrameError, FrameReader, encode};
pub use lan::{ANNOUNCE_PORT, Announcer, Discovery, LanGame};
