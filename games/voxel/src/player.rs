//! Le joueur, une entité de l'ECS comme les créatures : `Transform`, `Velocity`,
//! `Collider`, `MoveIntent`, `Health`, plus le marqueur `Player` (et `Flying` en vol libre).
//!
//! En marche, la physique tourne dans le système `physics`, à 60 Hz pile : le résultat
//! ne dépend pas des FPS (un saut monte toujours à la même hauteur). Mais l'écran, lui,
//! s'affiche à 120 Hz : entre deux pas de physique, on place la caméra entre l'ancienne
//! et la nouvelle position (interpolation par `alpha`), sinon l'image saccaderait.

use engine_ecs::{Entity, World};
use engine_voxel::{PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
use glam::Vec3;

use crate::components::{
    Collider, Flying, Health, MoveIntent, Player, PreviousPosition, Transform, Velocity,
};

pub const WALK_SPEED: f32 = 4.3;
pub const SPRINT_SPEED: f32 = 5.6;
pub const FLY_SPEED: f32 = 12.0;
pub const FLY_SPRINT_FACTOR: f32 = 3.0;
pub const PLAYER_HEALTH: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// Crée l'entité du joueur, les yeux en `eye`.
pub fn spawn(world: &mut World, eye: Vec3, mode: MoveMode) -> Entity {
    let feet = eye - Vec3::Y * PLAYER_EYE_HEIGHT;
    let player = world.spawn((
        Transform {
            position: feet,
            yaw: 0.0,
        },
        PreviousPosition(feet),
        Velocity::default(),
        Collider::new(PLAYER_WIDTH, PLAYER_HEIGHT),
        MoveIntent::default(),
        Health::full(PLAYER_HEALTH),
        Player,
    ));
    if mode == MoveMode::Fly {
        world.insert(player, Flying);
    }
    player
}

pub fn mode(world: &World, player: Entity) -> MoveMode {
    if world.has::<Flying>(player) {
        MoveMode::Fly
    } else {
        MoveMode::Walk
    }
}

fn feet(world: &World, player: Entity) -> Vec3 {
    world
        .get::<Transform>(player)
        .map(|t| t.position)
        .unwrap_or_default()
}

/// Position des yeux à afficher. `alpha` (entre 0 et 1) : où l'on en est entre le pas
/// de physique précédent et le dernier. En vol, pas d'interpolation : on bouge à chaque image.
pub fn eye(world: &World, player: Entity, alpha: f32) -> Vec3 {
    let current = feet(world, player);
    let feet = match (mode(world, player), world.get::<PreviousPosition>(player)) {
        (MoveMode::Walk, Some(previous)) => previous.0.lerp(current, alpha),
        _ => current,
    };
    feet + Vec3::Y * PLAYER_EYE_HEIGHT
}

/// Le corps physique à la position actuelle : pour interdire de poser un bloc dedans.
pub fn body(world: &World, player: Entity) -> PlayerBody {
    PlayerBody::new(feet(world, player))
}

/// Vrai si le joueur attend que le terrain soit chargé.
pub fn frozen(world: &World, player: Entity) -> bool {
    mode(world, player) == MoveMode::Walk && world.get::<Collider>(player).is_some_and(|c| c.frozen)
}

pub fn toggle_mode(world: &mut World, player: Entity) {
    match mode(world, player) {
        MoveMode::Walk => {
            world.insert(player, Flying);
        }
        MoveMode::Fly => {
            world.remove::<Flying>(player);
            // On repart d'une vitesse nulle, et sans interpoler depuis l'ancienne position.
            let position = feet(world, player);
            if let Some(mut previous) = world.get_mut::<PreviousPosition>(player) {
                previous.0 = position;
            }
            if let Some(mut velocity) = world.get_mut::<Velocity>(player) {
                velocity.0 = Vec3::ZERO;
            }
        }
    }
}

/// Place les yeux (vol automatique, visite guidée).
pub fn set_eye(world: &World, player: Entity, eye: Vec3) {
    if let Some(mut transform) = world.get_mut::<Transform>(player) {
        transform.position = eye - Vec3::Y * PLAYER_EYE_HEIGHT;
    }
    if let Some(mut previous) = world.get_mut::<PreviousPosition>(player) {
        previous.0 = eye - Vec3::Y * PLAYER_EYE_HEIGHT;
    }
}

/// Les entrées de la frame : vitesse voulue et saut.
pub fn set_intent(world: &World, player: Entity, walk: Vec3, jump: bool) {
    if let Some(mut intent) = world.get_mut::<MoveIntent>(player) {
        intent.walk = walk;
        intent.jump = jump;
    }
}

pub fn health(world: &World, player: Entity) -> Health {
    world
        .get::<Health>(player)
        .map(|h| *h)
        .unwrap_or(Health::full(PLAYER_HEALTH))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_goes_from_previous_to_current() {
        let mut world = World::new();
        let player = spawn(&mut world, Vec3::new(0.0, 10.0, 0.0), MoveMode::Walk);
        world.get_mut::<PreviousPosition>(player).unwrap().0 = Vec3::new(0.0, 5.0, 0.0);
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(1.0, 5.0, 0.0);
        let mid = eye(&world, player, 0.5);
        assert!((mid - Vec3::new(0.5, 5.0 + PLAYER_EYE_HEIGHT, 0.0)).length() < 1e-6);
    }

    #[test]
    fn toggling_mode_keeps_the_eye_in_place() {
        let mut world = World::new();
        let player = spawn(&mut world, Vec3::new(3.0, 50.0, 4.0), MoveMode::Walk);
        let before = eye(&world, player, 1.0);
        toggle_mode(&mut world, player);
        assert_eq!(mode(&world, player), MoveMode::Fly);
        assert!((eye(&world, player, 1.0) - before).length() < 1e-5);
        toggle_mode(&mut world, player);
        assert_eq!(mode(&world, player), MoveMode::Walk);
        assert!((eye(&world, player, 0.0) - before).length() < 1e-5);
    }
}
