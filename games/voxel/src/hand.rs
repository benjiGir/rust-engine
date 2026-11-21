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
            .get::<Velocity>(self.players.entity())
            .map_or(Vec3::ZERO, |v| v.0);
        let on_ground = self
            .ecs
            .get::<Collider>(self.players.entity())
            .is_some_and(|c| c.on_ground)
            || player::mode(&self.ecs, self.players.entity()) == MoveMode::Fly;
        self.client.hand.update(HandInput {
            dt,
            speed: Vec2::new(velocity.x, velocity.z).length(),
            sprinting: ctx.input.key_held(KeyCode::ControlLeft),
            on_ground,
            mining: self.players.me_mut().mining.is_some() || self.players.me_mut().air_swing,
            // L'objet tenu : l'inventaire tel que le serveur l'a envoyé.
            selected: (
                usize::from(self.client.view.me.inventory.selected),
                self.client
                    .view
                    .me
                    .inventory
                    .selected_stack()
                    .map(|s| s.item),
            ),
            yaw: self.client.camera.yaw,
            pitch: self.client.camera.pitch,
        });
    }
}
