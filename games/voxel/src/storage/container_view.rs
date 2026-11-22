//! La part fenêtre de `storage/container_view.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du
//! plan multijoueur ; la simulation reste dans `storage/container_view.rs`).

use crate::VoxelGame;
use voxel_sim::player::inventory::ItemStack;
#[allow(unused_imports)]
use voxel_sim::storage::container_view::*;

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
