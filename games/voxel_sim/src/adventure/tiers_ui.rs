//! La part serveur de `adventure/tiers_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du
//! plan multijoueur ; la fenêtre reste dans `adventure/tiers_ui.rs`).

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// `Tier` : le serveur change le palier du joueur courant, s'il est débloqué.
    pub fn choose_tier(&mut self, index: u8) {
        let data = crate::adventure::data::current();
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
