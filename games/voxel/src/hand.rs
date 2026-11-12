//! Animation du bras et raccordement au joueur.
pub use crate::hand_model::{ArmColors, Hand, HandInput};
use crate::{
    components::{Collider, Velocity},
    player::{self, MoveMode},
};
use engine_core::{Context, winit::keyboard::KeyCode};
use glam::{Vec2, Vec3};

impl crate::VoxelGame {
    /// Les animations du bras : marche, minage, changement d'objet, inertie.
    pub(crate) fn animate_hand(&mut self, ctx: &Context, dt: f32) {
        let velocity = self
            .ecs
            .get::<Velocity>(self.player)
            .map_or(Vec3::ZERO, |v| v.0);
        let on_ground = self
            .ecs
            .get::<Collider>(self.player)
            .is_some_and(|c| c.on_ground)
            || player::mode(&self.ecs, self.player) == MoveMode::Fly;
        self.hand.update(HandInput {
            dt,
            speed: Vec2::new(velocity.x, velocity.z).length(),
            sprinting: ctx.input.key_held(KeyCode::ControlLeft),
            on_ground,
            mining: self.mining.is_some() || self.air_swing,
            selected: (self.inventory.selected, self.inventory.selected_item()),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
        });
    }
}
