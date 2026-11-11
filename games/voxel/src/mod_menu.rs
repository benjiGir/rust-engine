//! Les menus des machines de mods : un mod décrit son menu (voir `engine_mods::menu`), le jeu
//! le dessine (`ui.rs`) et renvoie au script les actions du joueur. Rien ne passe par le
//! script tant que le joueur n'agit pas, hormis un rafraîchissement cinq fois par seconde.

use std::time::{Duration, Instant};

use engine_mods::ModMenu;
use engine_mods::menu::{UI_CLOSED, UI_REFRESH};
use engine_voxel::GridPos;
use glam::IVec3;

use crate::VoxelGame;
use crate::ui::Screen;

/// Le menu de mod ouvert.
#[derive(Debug, Clone)]
pub struct ModMenuState {
    /// Le mod qui l'a ouvert : il reçoit les actions du joueur.
    pub owner: String,
    /// La machine du monde dont c'est le menu (ses cases d'inventaire sont les siennes).
    pub at: IVec3,
    pub menu: ModMenu,
    pub last_refresh: Instant,
}

/// Le délai entre deux rafraîchissements (5 par seconde).
const REFRESH: Duration = Duration::from_millis(200);

impl VoxelGame {
    /// Un script demande d'ouvrir ou de mettre à jour son menu. L'ouverture n'est permise que
    /// pendant un clic droit du joueur (un mod ne surgit pas à l'écran quand il veut) ; la mise
    /// à jour, seulement pour le menu que ce mod a lui-même ouvert, sur la même machine.
    pub(crate) fn show_mod_menu(&mut self, mod_id: String, at: GridPos, data: &[u8]) {
        let menu = match ModMenu::parse(data) {
            Ok(menu) => menu,
            Err(err) => {
                log::warn!("[mod {mod_id}] menu refusé : {err:#}");
                return;
            }
        };
        let updating = self
            .ui
            .mod_menu
            .as_ref()
            .is_some_and(|m| m.owner == mod_id && m.at == at.pos);
        if !updating && !self.ui.allow_mod_menu {
            return;
        }
        if at.grid != engine_voxel::GridId::WORLD {
            return;
        }
        let last_refresh = self
            .ui
            .mod_menu
            .as_ref()
            .map_or_else(Instant::now, |m| m.last_refresh);
        self.ui.mod_menu = Some(ModMenuState {
            owner: mod_id,
            at: at.pos,
            menu,
            last_refresh,
        });
        if !updating {
            self.ui.screen = Screen::Mod(at.pos);
            self.ui.inventory_open = true;
        }
    }

    /// Un script ferme son menu.
    pub(crate) fn close_mod_menu_of(&mut self, mod_id: &str) {
        if self.ui.mod_menu.as_ref().is_some_and(|m| m.owner == mod_id) {
            self.close_screen();
        }
    }

    /// Dit au mod que son menu est fermé, et l'oublie. Appelé par `close_screen`.
    pub(crate) fn forget_mod_menu(&mut self) {
        let Some(state) = self.ui.mod_menu.take() else {
            return;
        };
        let actions = self
            .mods
            .ui_event(&state.owner, UI_CLOSED, GridPos::world(state.at), 0, 0);
        self.apply_machine_actions(actions);
    }

    /// Transmet au script les actions du joueur depuis la dernière image, puis le
    /// rafraîchissement périodique.
    pub(crate) fn flush_mod_ui(&mut self) {
        let events = std::mem::take(&mut self.ui.mod_events);
        for (kind, id, value) in events {
            self.send_mod_ui(kind, id, value);
        }
        let due = self
            .ui
            .mod_menu
            .as_ref()
            .is_some_and(|m| m.last_refresh.elapsed() >= REFRESH);
        if due {
            if let Some(m) = &mut self.ui.mod_menu {
                m.last_refresh = Instant::now();
            }
            self.send_mod_ui(UI_REFRESH, 0, 0);
        }
    }

    fn send_mod_ui(&mut self, kind: i32, id: i32, value: i32) {
        let Some((owner, at)) = self.ui.mod_menu.as_ref().map(|m| (m.owner.clone(), m.at)) else {
            return;
        };
        let actions = self
            .mods
            .ui_event(&owner, kind, GridPos::world(at), id, value);
        self.apply_machine_actions(actions);
    }
}
