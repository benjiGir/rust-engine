//! La part fenêtre de `dimensions.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `dimensions.rs`).

use crate::VoxelGame;
#[allow(unused_imports)]
use voxel_sim::world::dimensions::*;

impl VoxelGame {
    /// Les noms des dimensions du monde, pour `ctl` et la console.
    pub(crate) fn dimension_names(&self) -> Vec<String> {
        self.server
            .dims
            .registry
            .iter()
            .map(|d| d.key.clone())
            .collect()
    }
}
