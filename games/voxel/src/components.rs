//! Les composants du jeu : les données attachées aux entités de l'ECS.
//!
//! Chacun dérive `Inspect` : l'inspecteur (F3) peut les afficher et les modifier en
//! direct, sans code d'interface écrit à la main.

use engine_inspect::Inspect;
use glam::Vec3;

/// Où est l'entité : la position de ses pieds et son orientation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Transform {
    pub position: Vec3,
    /// En radians, même convention que la caméra (0 = regarde vers +x).
    pub yaw: f32,
}

/// La position au pas de physique précédent : le rendu interpole entre les deux.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct PreviousPosition(pub Vec3);

#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Velocity(pub Vec3);

/// La boîte de collision et ce qu'elle touche.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Collider {
    pub width: f32,
    pub height: f32,
    pub on_ground: bool,
    pub in_liquid: bool,
    /// Le terrain autour n'est pas encore chargé : l'entité attend sans bouger.
    pub frozen: bool,
}

impl Collider {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            ..Self::default()
        }
    }
}

/// Ce que l'entité veut faire : marcher dans une direction, sauter. Le joueur le
/// remplit avec le clavier, les créatures avec leur IA ; la physique fait le reste.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct MoveIntent {
    /// Vitesse horizontale voulue, en blocs par seconde.
    pub walk: Vec3,
    pub jump: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Inspect)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn full(max: f32) -> Self {
        Self { current: max, max }
    }
}

/// Marque le joueur. `Flying` : en vol libre, la physique l'ignore.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Player;

#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Flying;

/// Une créature : son type (indice dans le registre des créatures).
#[derive(Debug, Clone, Copy, PartialEq, Inspect)]
pub struct Mob {
    pub kind: u32,
}

/// L'IA d'errance : une direction gardée quelques secondes, puis une autre.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Wander {
    pub direction: Vec3,
    pub timer: f32,
    /// Temps avant de pouvoir blesser à nouveau (créatures hostiles).
    pub attack_cooldown: f32,
    /// Après un coup, un animal fuit pendant ce temps.
    pub panic: f32,
}

/// Une poussée horizontale en cours (le recul d'un coup), gardée d'un pas à l'autre.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Impulse(pub Vec3);

/// Clignote en rouge quelques instants après un coup.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct Hurt {
    pub timer: f32,
}

/// En rut : l'animal cherche un partenaire du même type, pendant `timer` secondes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Love {
    pub timer: f32,
}

/// Après une naissance, le parent ne se reproduit plus pendant `timer` secondes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BreedCooldown {
    pub timer: f32,
}

/// Un petit : plus petit, il grandit en `age_left` secondes et ne se reproduit pas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Baby {
    pub age_left: f32,
}

/// Une créature qui reste près de chez elle (les habitants d'un village).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Home(pub Vec3);

/// Un marchand : combien de fois chacun de ses échanges est encore possible.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trader {
    pub uses: Vec<u32>,
}

/// L'entité se tient sur une structure mobile : sa physique se fait dans le repère de cette
/// structure (voir `ships.rs`), pour qu'elle marche sur un pont qui bouge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnGrid(pub engine_voxel::GridId);
