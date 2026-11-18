//! Ce que les scripts savent du joueur : un instantané que le jeu rafraîchit à chaque pas de
//! simulation (et juste avant de lui confier un clic). Le script le lit avec `player_info`, et
//! lance des rayons dans le monde avec `player_target` sans repasser par le jeu.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use glam::Vec3;

/// L'état du joueur, vu des scripts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSnapshot {
    /// Ses yeux.
    pub eye: Vec3,
    /// Son regard, de longueur 1.
    pub look: Vec3,
    pub health: f32,
    pub max_health: f32,
    /// Son niveau d'expérience.
    pub level: f32,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            eye: Vec3::ZERO,
            look: Vec3::Z,
            health: 20.0,
            max_health: 20.0,
            level: 0.0,
        }
    }
}

impl PlayerSnapshot {
    /// L'ordre des 9 décimaux de `player_info`.
    pub fn to_floats(self) -> [f32; 9] {
        [
            self.eye.x,
            self.eye.y,
            self.eye.z,
            self.look.x,
            self.look.y,
            self.look.z,
            self.health,
            self.max_health,
            self.level,
        ]
    }
}

/// Le même instantané, partagé entre le jeu (qui l'écrit) et les scripts (qui le lisent).
pub type SharedPlayer = Rc<Cell<PlayerSnapshot>>;

/// Les identifiants des quêtes terminées du monde, partagés entre le jeu (qui les écrit) et les
/// scripts (`quest_done`).
pub type SharedQuests = Rc<RefCell<HashSet<String>>>;

/// Les noms d'effets que `player_effect` accepte.
pub const EFFECTS: [&str; 5] = [
    "speed",
    "strength",
    "regeneration",
    "fire_resistance",
    "healing",
];
