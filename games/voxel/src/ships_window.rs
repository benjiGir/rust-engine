//! La part fenêtre de `ships.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `ships.rs`).

use super::*;
use crate::key_codes::KeyCodes;
#[allow(unused_imports)]
use crate::ships::*;

impl VoxelGame {
    /// La barre : gauche et droite, envoyées au serveur quand elles changent (voir `update_helm`).
    pub(crate) fn update_ships(&mut self, ctx: &Context) {
        let keys = &self.settings.keys;
        let steer = f32::from(ctx.input.key_held(keys.key(Action::Right)) as u8)
            - f32::from(ctx.input.key_held(keys.key(Action::Left)) as u8);
        if steer != self.client.steer_sent {
            self.client.steer_sent = steer;
            self.send_now(crate::protocol::ClientMsg::Steer(steer));
        }
    }
}
