//! La part fenêtre de `container_view.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `container_view.rs`).

use crate::VoxelGame;
#[allow(unused_imports)]
use crate::container_view::*;
use crate::inventory::ItemStack;

impl VoxelGame {
    /// La vue du conteneur ouvert que la fenêtre a reçue.
    pub(crate) fn shown_container(&self) -> Option<&ContainerView> {
        self.client.view.me.container.as_ref()
    }

    /// La pile d'une case du conteneur ouvert, telle que la fenêtre la voit.
    pub(crate) fn shown_stack(&self, index: usize) -> Option<ItemStack> {
        self.shown_container()?
            .stack(&self.client.content.items, index)
    }
}
