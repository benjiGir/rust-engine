//! Les menus des machines de mods : un mod décrit son menu (voir `engine_mods::menu`), le jeu
//! le dessine (`ui.rs`) et renvoie au script les actions du joueur. Rien ne passe par le
//! script tant que le joueur n'agit pas, hormis un rafraîchissement cinq fois par seconde.

use std::time::{Duration, Instant};

use engine_mods::ModMenu;
use engine_mods::menu::{UI_CLOSED, UI_OPEN, UI_REFRESH};
use engine_voxel::GridPos;
use glam::IVec3;

use crate::Mode;
use crate::VoxelGame;
use crate::menus::PausePage;
use crate::ui::Screen;

/// Le menu de mod ouvert, tel que la fenêtre le dessine (analysé des octets que le serveur envoie
/// dans `ScreenView::Mod`).
#[derive(Debug, Clone)]
pub struct ModMenuState {
    /// Le mod qui l'a ouvert.
    pub owner: String,
    /// La machine du monde dont c'est le menu (ses cases d'inventaire sont les siennes).
    pub at: IVec3,
    pub menu: ModMenu,
    /// Les octets reçus : un menu n'est réanalysé que s'ils changent.
    pub data: Vec<u8>,
}

/// Le menu de mod qu'un joueur a ouvert, côté serveur : le mod qui l'a ouvert (il reçoit les
/// actions), la machine, le menu tel que le script l'a décrit, et le dernier rafraîchissement.
#[derive(Debug, Clone)]
pub struct OpenModMenu {
    pub owner: String,
    pub at: IVec3,
    pub data: Vec<u8>,
    pub last_refresh: Instant,
}

/// Le délai entre deux rafraîchissements (5 par seconde).
const REFRESH: Duration = Duration::from_millis(200);

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
            self.send_now(crate::protocol::ClientMsg::Screen(
                crate::protocol::ScreenAction::ModEvent { kind, id, value },
            ));
        }
        let options = std::mem::take(&mut self.client.ui.options_events);
        if let (Some((owner, _)), false) = (self.client.ui.options_menu.clone(), options.is_empty())
        {
            for (kind, id, value) in options {
                self.send_now(crate::protocol::ClientMsg::ModOptionsEvent {
                    mod_id: owner.clone(),
                    kind,
                    id,
                    value,
                });
            }
        }
        // Le serveur écrit les réglages et rafraîchit les menus ouverts (même en pause).
        self.server.refresh_mod_menus();
    }
}

impl crate::server_side::VoxelServer {
    /// Les réglages des mods changés sont écrits au plus une fois par seconde, et les menus de mods
    /// ouverts se rafraîchissent cinq fois par seconde.
    pub(crate) fn refresh_mod_menus(&mut self) {
        // Les réglages changés sont écrits au plus une fois par seconde (un curseur qu'on
        // fait glisser ne doit pas écrire à chaque image).
        if self.mod_settings_saved.elapsed() >= Duration::from_secs(1) {
            self.mod_settings_saved = Instant::now();
            self.mods.save_settings();
        }
        let due: Vec<_> = self
            .players
            .iter()
            .filter(|p| {
                p.mod_menu
                    .as_ref()
                    .is_some_and(|m| m.last_refresh.elapsed() >= REFRESH)
            })
            .map(|p| p.id)
            .collect();
        for id in due {
            let before = self.players.set_acting(Some(id));
            if let Some(m) = &mut self.players.me_mut().mod_menu {
                m.last_refresh = Instant::now();
            }
            self.send_mod_ui(UI_REFRESH, 0, 0);
            self.players.set_acting(before);
        }
    }
}

pub use crate::ui_widgets::draw_widgets;

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

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un script demande d'ouvrir ou de mettre à jour son menu, pour le joueur courant. L'ouverture
    /// n'est permise que pendant un clic droit du joueur (un mod ne surgit pas à l'écran quand il
    /// veut) ; la mise à jour, seulement pour le menu que ce mod a lui-même ouvert, sur la même
    /// machine.
    pub(crate) fn show_mod_menu(&mut self, mod_id: String, at: GridPos, data: &[u8]) {
        if let Err(err) = ModMenu::parse(data) {
            log::warn!("[mod {mod_id}] menu refusé : {err:#}");
            return;
        }
        let open = self.players.me().mod_menu.clone();
        let updating = open
            .as_ref()
            .is_some_and(|m| m.owner == mod_id && m.at == at.pos);
        if !updating && !self.allow_mod_menu {
            return;
        }
        if at.grid != engine_voxel::GridId::WORLD {
            return;
        }
        let last_refresh = open.map_or_else(Instant::now, |m| m.last_refresh);
        self.players.me_mut().mod_menu = Some(OpenModMenu {
            owner: mod_id,
            at: at.pos,
            data: data.to_vec(),
            last_refresh,
        });
        if !updating {
            self.open_screen(Screen::Mod(at.pos));
        }
    }

    /// Un script ferme son menu : chez chaque joueur qui l'a ouvert.
    pub(crate) fn close_mod_menu_of(&mut self, mod_id: &str) {
        let ids: Vec<_> = self
            .players
            .iter()
            .filter(|p| p.mod_menu.as_ref().is_some_and(|m| m.owner == mod_id))
            .map(|p| p.id)
            .collect();
        for id in ids {
            let before = self.players.set_acting(Some(id));
            self.forget_mod_menu();
            self.players.me_mut().screen = None;
            self.players.set_acting(before);
            self.replies
                .push((id, crate::protocol::ServerMsg::CloseScreen));
        }
    }

    /// Transmet une action au mod du menu ouvert par le joueur courant.
    pub(crate) fn send_mod_ui(&mut self, kind: i32, id: i32, value: i32) {
        let Some((owner, at)) = self
            .players
            .me()
            .mod_menu
            .as_ref()
            .map(|m| (m.owner.clone(), m.at))
        else {
            return;
        };
        let actions = self
            .mods
            .ui_event(&owner, kind, GridPos::world(at), id, value);
        self.apply_machine_actions(actions);
    }

    /// Dit au mod que le menu du joueur courant est fermé, et l'oublie (`CloseMenu`).
    pub(crate) fn forget_mod_menu(&mut self) {
        let Some(state) = self.players.me_mut().mod_menu.take() else {
            return;
        };
        let actions = self
            .mods
            .ui_event(&state.owner, UI_CLOSED, GridPos::world(state.at), 0, 0);
        self.apply_machine_actions(actions);
    }
}
