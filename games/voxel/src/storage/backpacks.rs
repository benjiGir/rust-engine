//! La part fenêtre de `storage/backpacks.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `storage/backpacks.rs`).

use crate::VoxelGame;
#[allow(unused_imports)]
use voxel_sim::storage::backpacks::*;

impl VoxelGame {
    /// Ouvre le premier sac de l'inventaire (barre d'abord).
    pub(crate) fn open_first_bag(&mut self) -> bool {
        let slot = (0..self.server.players.me_mut().inventory.slots.len()).find(|&i| {
            self.server.players.me_mut().inventory.slots[i]
                .is_some_and(|s| is_backpack(&self.client.content.items.get(s.item).key))
        });
        slot.is_some_and(|slot| self.server.open_bag_at(slot))
    }
}
