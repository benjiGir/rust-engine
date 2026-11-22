//! La part serveur de `guide_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `guide_ui.rs`).

use crate::quests::guide::GUIDE_ITEM;
use crate::server::screens::Screen;

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un clic droit avec le guide en main : l'ouvre. `true` si c'était le guide.
    pub fn use_guide(&mut self) -> bool {
        let Some(stack) = self.players.me_mut().inventory.selected_stack() else {
            return false;
        };
        if self.content.items.get(stack.item).key != GUIDE_ITEM {
            return false;
        }
        self.open_screen(Screen::Guide);
        true
    }
}
