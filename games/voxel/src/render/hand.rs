//! Animation du bras et raccordement au joueur.
pub use crate::render::hand_model::{ArmColors, Hand, HandInput};
use engine_core::{Context, winit::keyboard::KeyCode};
use glam::Vec2;
use voxel_sim::player::MoveMode;

impl crate::VoxelGame {
    /// Les animations du bras : marche, minage, changement d'objet, inertie.
    pub(crate) fn animate_hand(&mut self, ctx: &Context, dt: f32) {
        let velocity = self.client.body.body.velocity;
        let on_ground = self.client.body.body.on_ground || self.client.body.mode == MoveMode::Fly;
        self.client.hand.update(HandInput {
            dt,
            speed: Vec2::new(velocity.x, velocity.z).length(),
            sprinting: ctx.input.key_held(KeyCode::ControlLeft),
            on_ground,
            mining: self.client.mining.is_some() || self.client.air_swing,
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
