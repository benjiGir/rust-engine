//! Le joueur : marche avec gravité et collisions, ou vol libre (touche F).
//!
//! En marche, la physique tourne dans `fixed_update`, à 60 Hz pile : le résultat ne
//! dépend pas des FPS (un saut monte toujours à la même hauteur). Mais l'écran,
//! lui, s'affiche à 120 Hz : entre deux pas de physique, on place la caméra entre
//! l'ancienne et la nouvelle position (interpolation par `alpha`), sinon l'image
//! saccaderait en répétant deux fois la même position.

use engine_voxel::{ChunkStreamer, PLAYER_EYE_HEIGHT, PlayerBody, World};
use glam::{IVec3, Vec3};

pub const WALK_SPEED: f32 = 4.3;
pub const SPRINT_SPEED: f32 = 5.6;
pub const FLY_SPEED: f32 = 12.0;
pub const FLY_SPRINT_FACTOR: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    Walk,
    Fly,
}

impl MoveMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Walk => "marche",
            Self::Fly => "vol",
        }
    }
}

pub struct Player {
    pub mode: MoveMode,
    pub body: PlayerBody,
    /// Position des pieds au pas de physique précédent, pour l'interpolation.
    previous_feet: Vec3,
    /// Position des yeux en vol libre.
    fly_eye: Vec3,
    /// Vrai si le joueur attend que le terrain autour de lui soit chargé.
    pub frozen: bool,
}

impl Player {
    pub fn new(eye: Vec3, mode: MoveMode) -> Self {
        let feet = eye - Vec3::Y * PLAYER_EYE_HEIGHT;
        Self {
            mode,
            body: PlayerBody::new(feet),
            previous_feet: feet,
            fly_eye: eye,
            frozen: true,
        }
    }

    /// Position des yeux à afficher. `alpha` (entre 0 et 1) : où l'on en est
    /// entre le pas de physique précédent et le dernier.
    pub fn eye(&self, alpha: f32) -> Vec3 {
        match self.mode {
            MoveMode::Walk => {
                self.previous_feet.lerp(self.body.position, alpha) + Vec3::Y * PLAYER_EYE_HEIGHT
            }
            MoveMode::Fly => self.fly_eye,
        }
    }

    /// Le corps à la position actuelle, quel que soit le mode : pour interdire
    /// de poser un bloc dans le joueur, même en vol.
    pub fn current_body(&self) -> PlayerBody {
        match self.mode {
            MoveMode::Walk => self.body,
            MoveMode::Fly => PlayerBody::new(self.fly_eye - Vec3::Y * PLAYER_EYE_HEIGHT),
        }
    }

    pub fn toggle_mode(&mut self) {
        match self.mode {
            MoveMode::Walk => {
                self.fly_eye = self.body.eye();
                self.mode = MoveMode::Fly;
            }
            MoveMode::Fly => {
                self.body = PlayerBody::new(self.fly_eye - Vec3::Y * PLAYER_EYE_HEIGHT);
                self.previous_feet = self.body.position;
                self.mode = MoveMode::Walk;
            }
        }
    }

    /// Un pas de physique (mode marche seulement). `walk` : vitesse horizontale voulue.
    pub fn fixed_update(&mut self, streamer: &ChunkStreamer, walk: Vec3, jump: bool, dt: f32) {
        if self.mode != MoveMode::Walk {
            return;
        }
        self.previous_feet = self.body.position;
        // Tant que le terrain autour du joueur n'est pas généré, le monde n'y contient
        // que de l'air : sans ce gel, le joueur tomberait à travers le sol.
        self.frozen = !surroundings_ready(streamer, &self.body);
        if self.frozen {
            self.body.velocity = Vec3::ZERO;
            return;
        }
        self.body.step(streamer.world(), walk, jump, dt);
    }

    /// Vol libre : déplacement direct, sans collision, à chaque image.
    pub fn fly(&mut self, direction: Vec3, speed: f32, dt: f32) {
        self.fly_eye += direction.normalize_or_zero() * speed * dt;
    }

    /// Place les yeux (vol automatique).
    pub fn set_fly_eye(&mut self, eye: Vec3) {
        self.fly_eye = eye;
    }
}

/// Les chunks que la boîte du joueur touche, plus un bloc de marge, sont-ils générés ?
fn surroundings_ready(streamer: &ChunkStreamer, body: &PlayerBody) -> bool {
    let (min, max) = body.bounds();
    let (min, max) = (min - Vec3::ONE, max + Vec3::ONE);
    let first = World::chunk_containing(min);
    let last = World::chunk_containing(max);
    (first.x..=last.x).all(|x| {
        (first.y..=last.y)
            .all(|y| (first.z..=last.z).all(|z| streamer.is_generated(IVec3::new(x, y, z))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::{StreamingConfig, TerrainGenerator};

    #[test]
    fn interpolation_goes_from_previous_to_current() {
        let mut player = Player::new(Vec3::new(0.0, 10.0, 0.0), MoveMode::Walk);
        player.previous_feet = Vec3::new(0.0, 5.0, 0.0);
        player.body.position = Vec3::new(1.0, 5.0, 0.0);
        let mid = player.eye(0.5);
        assert!((mid - Vec3::new(0.5, 5.0 + PLAYER_EYE_HEIGHT, 0.0)).length() < 1e-6);
    }

    #[test]
    fn toggling_mode_keeps_the_eye_in_place() {
        let mut player = Player::new(Vec3::new(3.0, 50.0, 4.0), MoveMode::Walk);
        let eye = player.eye(1.0);
        player.toggle_mode();
        assert_eq!(player.mode, MoveMode::Fly);
        assert!((player.eye(1.0) - eye).length() < 1e-5);
        player.toggle_mode();
        assert!((player.eye(1.0) - eye).length() < 1e-5);
    }

    #[test]
    fn player_is_frozen_until_terrain_is_loaded() {
        // Un streamer qui n'a encore rien chargé.
        let streamer = ChunkStreamer::new(TerrainGenerator::new(1), StreamingConfig::default());
        let mut player = Player::new(Vec3::new(0.5, 60.0, 0.5), MoveMode::Walk);
        for _ in 0..60 {
            player.fixed_update(&streamer, Vec3::ZERO, false, 1.0 / 60.0);
        }
        assert!(player.frozen);
        assert_eq!(player.eye(1.0).y, 60.0, "n'est pas tombé");
    }
}
