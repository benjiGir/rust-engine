//! `engine_core` : la fenêtre, la boucle de jeu et les entrées.
//!
//! Un jeu implémente le trait [`Game`] puis appelle [`run`].
//! C'est l'équivalent de ce que Unity fait avant d'appeler tes `MonoBehaviour`.

mod app;
mod input;
mod time;

pub use app::{AppConfig, Context, Game, run};
pub use input::Input;
pub use time::FixedTimestep;

// On réexporte winit pour que les jeux utilisent exactement la même version.
pub use winit;
