//! Le réseau du moteur, sans rien savoir du jeu : des trames d'octets compressées
//! ([`frame`]), des connexions TCP qui lisent et écrivent sur leurs propres threads
//! ([`connection`]). Le jeu choisit ce qu'il met dans les trames (`postcard` pour le voxel).

pub mod connection;
pub mod frame;

pub use connection::{Connection, Event, Listener, SendError};
pub use frame::{Frame, FrameError, FrameReader, encode};
