//! La part serveur de `tiers_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `tiers_ui.rs`).

use crate::adventure_data::{self};

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// `Tier` : le serveur change le palier du joueur courant, s'il est débloqué.
    pub fn choose_tier(&mut self, index: u8) {
        let data = adventure_data::current();
        if self
            .players
            .me_mut()
            .adventure
            .progress
            .select(index, &data.tiers)
        {
            let name = data
                .tier(index)
                .map_or("?", |t| t.name.as_str())
                .to_string();
            self.toast(&format!("Palier : {name}"));
            self.sound("core:enchant");
        }
    }
}
