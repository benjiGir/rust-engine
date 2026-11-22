//! La part fenêtre de `screen_actions.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `screen_actions.rs`).

use crate::VoxelGame;
#[allow(unused_imports)]
use voxel_sim::server::screen_actions::*;

impl VoxelGame {
    /// La vue d'écran que la fenêtre a reçue.
    pub(crate) fn shown_screen(&self) -> Option<&ScreenView> {
        self.client.view.me.screen.as_ref()
    }
}
