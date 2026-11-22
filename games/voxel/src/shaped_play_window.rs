//! La part fenêtre de `shaped_play.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `shaped_play.rs`).

use crate::VoxelGame;
use crate::ui_info::Card;
use engine_voxel::RayHit;
use voxel_sim::shaping;
#[allow(unused_imports)]
use voxel_sim::shaping::play::*;

impl VoxelGame {
    /// La fiche de visée d'une case de formes : ses matériaux, et la pièce visée.
    pub(crate) fn describe_shaped(&self, card: &mut Card, hit: &RayHit) {
        let Some(shaped) = self.server.shaped_at(hit.block_pos) else {
            return;
        };
        let names: Vec<String> = shaped
            .materials
            .iter()
            .map(|m| shaping::materials::material_name(&self.client.registry, m).to_lowercase())
            .collect();
        card.name = format!("Case de formes : {}", names.join(", "));
        if let Some(index) = self.server.aimed_part(hit.block_pos, &shaped) {
            let part = shaped.parts[index];
            let material =
                shaping::materials::material_name(&self.client.registry, shaped.material_of(&part));
            let (what, size) = describe_piece(&part.piece);
            card.lines.insert(
                0,
                (
                    format!(
                        "Pièce visée : {what} de {} ({size})",
                        material.to_lowercase()
                    ),
                    crate::skin::TEXT,
                ),
            );
        }
    }
}
