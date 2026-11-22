//! Le contrôle des mouvements : le client prédit son déplacement avec la physique locale, le
//! simulateur vérifie seulement que c'est possible.
//!
//! [`movement_check`] est une fonction pure : elle compare deux positions successives du joueur et
//! refuse ce qu'aucun mode de déplacement ne permet (trop vite, à travers un bloc plein). Un refus
//! devient un `ServerMsg::Correct` que le client applique (voir `server/intents.rs`). Ce n'est pas
//! une anti-triche (pas de rejeu de la prédiction) : seulement le « moved too quickly » d'un
//! serveur de réseau local.
//!
//! En solo le contrôle ne tourne que si `VOXEL_CHECK_MOVES=1` est mis : par défaut il ne coûte rien
//! et ne change rien.

use glam::{IVec3, Vec3};

use crate::player::{FLY_SPEED, FLY_SPRINT_FACTOR, MoveMode, SPRINT_SPEED};

/// La durée d'un pas fixe, en secondes (60 Hz).
pub const STEP_SECONDS: f32 = 1.0 / 60.0;
/// Ce qu'on tolère en plus de la distance permise : un joueur repoussé hors d'un bloc, un
/// arrondi, le premier pas après un chargement.
const SLACK: f32 = 0.6;
/// L'écart entre deux échantillons du chemin pour chercher un mur.
const WALL_SAMPLE: f32 = 0.25;
/// La hauteur, au-dessus des pieds, où l'on cherche un mur : le milieu du corps ne peut jamais
/// être dans un cube plein si la physique a fait son travail.
const BODY_MIDDLE: f32 = 0.9;
/// La vitesse de chute maximale de la physique (60) et une marge.
const MAX_DOWN: f32 = 70.0;
/// Un saut (9 blocs par seconde), une marge pour un élan hors de l'eau.
const MAX_UP: f32 = 14.0;

/// Où était le joueur à un pas donné.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveSample {
    /// Les pieds.
    pub position: Vec3,
    pub mode: MoveMode,
    pub on_ground: bool,
    /// Le numéro du pas fixe où cette position a été prise.
    pub step: u64,
}

/// Les vitesses permises, en blocs par seconde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveLimits {
    pub horizontal: f32,
    pub up: f32,
    pub down: f32,
}

impl MoveLimits {
    /// Les limites d'un mode, avec la vitesse de marche du joueur (`speed` : 1 sans bonus, plus
    /// avec une potion ou une pièce d'équipement). Larges : un coup qui repousse, un courant ou
    /// une glissade ne doivent pas être pris pour de la triche.
    pub fn new(mode: MoveMode, speed: f32) -> Self {
        match mode {
            MoveMode::Walk => Self {
                horizontal: (SPRINT_SPEED * speed * 2.0).max(12.0),
                up: MAX_UP,
                down: MAX_DOWN,
            },
            MoveMode::Fly => {
                let fly = FLY_SPEED * FLY_SPRINT_FACTOR * 2.0;
                Self {
                    horizontal: fly,
                    up: fly,
                    down: fly,
                }
            }
        }
    }

    fn widest(self, other: Self) -> Self {
        Self {
            horizontal: self.horizontal.max(other.horizontal),
            up: self.up.max(other.up),
            down: self.down.max(other.down),
        }
    }
}

/// Pourquoi un mouvement est refusé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveRefusal {
    /// Une position qui n'est pas un nombre.
    Invalid,
    /// Plus loin, à l'horizontale, que la vitesse permise ne le laisse faire.
    TooFast { distance: f32, allowed: f32 },
    /// Plus haut ou plus bas que la vitesse verticale permise.
    TooSteep { distance: f32, allowed: f32 },
    /// Le chemin traverse un bloc plein.
    ThroughWall(IVec3),
}

/// Compare deux positions successives. `solid` dit si une cellule est un cube plein (qui arrête le
/// corps de part en part).
pub fn movement_check(
    previous: &MoveSample,
    next: &MoveSample,
    limits: MoveLimits,
    solid: impl Fn(IVec3) -> bool,
) -> Result<(), MoveRefusal> {
    if !next.position.is_finite() {
        return Err(MoveRefusal::Invalid);
    }
    let steps = next.step.saturating_sub(previous.step).max(1);
    let seconds = steps as f32 * STEP_SECONDS;
    // Un changement de mode (la touche de vol) laisse la plus large des deux limites.
    let limits = if previous.mode == next.mode {
        limits
    } else {
        limits.widest(MoveLimits::new(previous.mode, 1.0))
    };
    let delta = next.position - previous.position;
    let horizontal = Vec3::new(delta.x, 0.0, delta.z).length();
    let allowed = limits.horizontal * seconds + SLACK;
    if horizontal > allowed {
        return Err(MoveRefusal::TooFast {
            distance: horizontal,
            allowed,
        });
    }
    let allowed = if delta.y >= 0.0 {
        limits.up
    } else {
        limits.down
    } * seconds
        + SLACK;
    if delta.y.abs() > allowed {
        return Err(MoveRefusal::TooSteep {
            distance: delta.y.abs(),
            allowed,
        });
    }
    // Le vol libre traverse tout, comme la physique du jeu.
    if next.mode == MoveMode::Walk && previous.mode == MoveMode::Walk {
        let from = previous.position + Vec3::Y * BODY_MIDDLE;
        let length = delta.length();
        let samples = (length / WALL_SAMPLE).ceil().max(1.0) as u32;
        for i in 1..=samples {
            let point = from + delta * (i as f32 / samples as f32);
            let cell = point.floor().as_ivec3();
            if solid(cell) {
                return Err(MoveRefusal::ThroughWall(cell));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(position: Vec3, mode: MoveMode, step: u64) -> MoveSample {
        MoveSample {
            position,
            mode,
            on_ground: true,
            step,
        }
    }

    fn open_air(_: IVec3) -> bool {
        false
    }

    /// Un mur : la colonne de cellules `x = 10`, sur quatre blocs de haut.
    fn wall_at_ten(cell: IVec3) -> bool {
        cell.x == 10 && (0..4).contains(&cell.y)
    }

    #[test]
    fn walking_and_sprinting_are_accepted() {
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        let mut at = sample(Vec3::new(0.0, 1.0, 0.0), MoveMode::Walk, 0);
        for step in 1..=120 {
            let next = sample(
                at.position + Vec3::new(SPRINT_SPEED * STEP_SECONDS, 0.0, 0.0),
                MoveMode::Walk,
                step,
            );
            assert_eq!(movement_check(&at, &next, limits, open_air), Ok(()));
            at = next;
        }
    }

    #[test]
    fn flying_fast_is_accepted_in_fly_mode_only() {
        let from = sample(Vec3::ZERO, MoveMode::Fly, 0);
        let fast = Vec3::new(FLY_SPEED * FLY_SPRINT_FACTOR * STEP_SECONDS, 0.0, 0.0);
        let to = sample(fast, MoveMode::Fly, 1);
        let fly = MoveLimits::new(MoveMode::Fly, 1.0);
        assert_eq!(movement_check(&from, &to, fly, open_air), Ok(()));
        // La même distance à pied, sur un seul pas, est trop vite.
        let from = sample(Vec3::ZERO, MoveMode::Walk, 0);
        let to = sample(Vec3::new(2.0, 0.0, 0.0), MoveMode::Walk, 1);
        let walk = MoveLimits::new(MoveMode::Walk, 1.0);
        assert!(matches!(
            movement_check(&from, &to, walk, open_air),
            Err(MoveRefusal::TooFast { .. })
        ));
    }

    #[test]
    fn teleporting_is_refused() {
        let from = sample(Vec3::new(0.0, 64.0, 0.0), MoveMode::Walk, 10);
        let to = sample(Vec3::new(500.0, 64.0, 0.0), MoveMode::Walk, 11);
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        assert!(matches!(
            movement_check(&from, &to, limits, open_air),
            Err(MoveRefusal::TooFast { .. })
        ));
        // Même en laissant du temps, 500 blocs en une seconde restent impossibles à pied.
        let later = sample(Vec3::new(500.0, 64.0, 0.0), MoveMode::Walk, 70);
        assert!(movement_check(&from, &later, limits, open_air).is_err());
    }

    #[test]
    fn crossing_a_wall_is_refused_even_at_a_legal_speed() {
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        let from = sample(Vec3::new(9.8, 1.0, 0.5), MoveMode::Walk, 0);
        // 1 bloc en 6 pas : une vitesse permise, mais le mur est au milieu.
        let to = sample(Vec3::new(10.9, 1.0, 0.5), MoveMode::Walk, 6);
        assert!(matches!(
            movement_check(&from, &to, limits, wall_at_ten),
            Err(MoveRefusal::ThroughWall(_))
        ));
        // En vol, on traverse.
        let from = sample(from.position, MoveMode::Fly, 0);
        let to = sample(to.position, MoveMode::Fly, 6);
        let fly = MoveLimits::new(MoveMode::Fly, 1.0);
        assert_eq!(movement_check(&from, &to, fly, wall_at_ten), Ok(()));
    }

    #[test]
    fn falling_is_accepted_and_rising_too_fast_is_not() {
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        let from = sample(Vec3::new(0.0, 100.0, 0.0), MoveMode::Walk, 0);
        // La chute maximale de la physique : 60 blocs par seconde.
        let fall = sample(
            Vec3::new(0.0, 100.0 - 60.0 * STEP_SECONDS, 0.0),
            MoveMode::Walk,
            1,
        );
        assert_eq!(movement_check(&from, &fall, limits, open_air), Ok(()));
        let rise = sample(Vec3::new(0.0, 103.0, 0.0), MoveMode::Walk, 1);
        assert!(matches!(
            movement_check(&from, &rise, limits, open_air),
            Err(MoveRefusal::TooSteep { .. })
        ));
    }

    #[test]
    fn a_jump_is_accepted() {
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        let from = sample(Vec3::new(0.0, 5.0, 0.0), MoveMode::Walk, 0);
        let up = sample(
            Vec3::new(0.0, 5.0 + 9.0 * STEP_SECONDS, 0.0),
            MoveMode::Walk,
            1,
        );
        assert_eq!(movement_check(&from, &up, limits, open_air), Ok(()));
    }

    #[test]
    fn not_a_number_is_refused() {
        let limits = MoveLimits::new(MoveMode::Walk, 1.0);
        let from = sample(Vec3::ZERO, MoveMode::Walk, 0);
        let to = sample(Vec3::new(f32::NAN, 0.0, 0.0), MoveMode::Walk, 1);
        assert_eq!(
            movement_check(&from, &to, limits, open_air),
            Err(MoveRefusal::Invalid)
        );
    }

    #[test]
    fn a_speed_bonus_widens_the_limit() {
        let from = sample(Vec3::ZERO, MoveMode::Walk, 0);
        let to = sample(Vec3::new(2.0, 0.0, 0.0), MoveMode::Walk, 10);
        // 2 blocs en 10 pas : 12 blocs par seconde, la limite de la marche normale...
        let slow = MoveLimits::new(MoveMode::Walk, 1.0);
        assert!(movement_check(&from, &to, slow, open_air).is_ok());
        let far = sample(Vec3::new(6.0, 0.0, 0.0), MoveMode::Walk, 10);
        assert!(movement_check(&from, &far, slow, open_air).is_err());
        // ... mais 6 blocs sont permis avec une vitesse triple.
        let fast = MoveLimits::new(MoveMode::Walk, 3.0);
        assert!(movement_check(&from, &far, fast, open_air).is_ok());
    }
}
