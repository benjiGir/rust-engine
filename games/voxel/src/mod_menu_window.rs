//! La part fenêtre de `mod_menu.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `mod_menu.rs`).

use crate::Mode;
use crate::VoxelGame;
use crate::menus::PausePage;
use engine_mods::ModMenu;
use engine_mods::menu::{UI_CLOSED, UI_OPEN};
use glam::IVec3;
#[allow(unused_imports)]
use voxel_sim::modding::menu::*;

impl VoxelGame {
    /// La fenêtre range le menu que le serveur décrit (seulement s'il a changé).
    pub(crate) fn receive_mod_menu(&mut self, owner: String, at: IVec3, data: Vec<u8>) {
        if self
            .client
            .ui
            .mod_menu
            .as_ref()
            .is_some_and(|m| m.owner == owner && m.at == at && m.data == data)
        {
            return;
        }
        if let Ok(menu) = ModMenu::parse(&data) {
            self.client.ui.mod_menu = Some(ModMenuState {
                owner,
                at,
                menu,
                data,
            });
        }
    }

    /// La fenêtre envoie au serveur les actions du joueur sur le menu ; le serveur rafraîchit les
    /// menus ouverts cinq fois par seconde, et transmet les pages d'options.
    pub(crate) fn flush_mod_ui(&mut self) {
        let events = std::mem::take(&mut self.client.ui.mod_events);
        for (kind, id, value) in events {
            self.send_now(voxel_sim::net::protocol::ClientMsg::Screen(
                voxel_sim::net::protocol::ScreenAction::ModEvent { kind, id, value },
            ));
        }
        let options = std::mem::take(&mut self.client.ui.options_events);
        if let (Some((owner, _)), false) = (self.client.ui.options_menu.clone(), options.is_empty())
        {
            for (kind, id, value) in options {
                self.send_now(voxel_sim::net::protocol::ClientMsg::ModOptionsEvent {
                    mod_id: owner.clone(),
                    kind,
                    id,
                    value,
                });
            }
        }
        // Le serveur écrit les réglages et rafraîchit les menus ouverts (même en pause).
        if self.hosting() {
            self.server.refresh_mod_menus();
        }
    }
}

impl VoxelGame {
    /// Le joueur ouvre la page d'options d'un mod (depuis la pause) : le script est prévenu
    /// et répond avec son menu.
    pub(crate) fn open_mod_options(&mut self, mod_id: &str) {
        self.mode = Mode::Paused(PausePage::ModOptions(mod_id.to_string()));
        self.client.ui.options_menu = None;
        let actions = self.server.mods.options_event(mod_id, UI_OPEN, 0, 0);
        self.server.apply_machine_actions(actions);
    }

    /// Un script envoie le contenu de sa page d'options (seulement si elle est ouverte).
    pub(crate) fn show_mod_options(&mut self, mod_id: String, data: &[u8]) {
        let open = matches!(&self.mode, Mode::Paused(PausePage::ModOptions(id)) if *id == mod_id);
        if !open {
            return;
        }
        match ModMenu::parse(data) {
            Ok(menu) => self.client.ui.options_menu = Some((mod_id, menu)),
            Err(err) => log::warn!("[mod {mod_id}] page d'options refusée : {err:#}"),
        }
    }

    /// Quitte la page d'options d'un mod, si elle est ouverte : le script est prévenu, et les
    /// réglages sont écrits.
    pub(crate) fn leave_mod_options(&mut self) {
        if let Mode::Paused(PausePage::ModOptions(id)) = &self.mode {
            let id = id.clone();
            let actions = self.server.mods.options_event(&id, UI_CLOSED, 0, 0);
            self.server.apply_machine_actions(actions);
        }
        self.client.ui.options_menu = None;
        self.client.ui.options_events.clear();
        self.server.mods.save_settings();
    }
}
