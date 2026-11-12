//! Les menus des machines de mods : un mod décrit son menu (voir `engine_mods::menu`), le jeu
//! le dessine (`ui.rs`) et renvoie au script les actions du joueur. Rien ne passe par le
//! script tant que le joueur n'agit pas, hormis un rafraîchissement cinq fois par seconde.

use std::time::{Duration, Instant};

use engine_mods::menu::{UI_BUTTON, UI_CLOSED, UI_OPEN, UI_REFRESH, UI_SLIDER, UI_TOGGLE};
use engine_mods::{ModMenu, Widget};
use engine_render::egui;
use engine_voxel::GridPos;
use glam::IVec3;

use crate::Mode;
use crate::VoxelGame;
use crate::menus::PausePage;
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
        let options = std::mem::take(&mut self.ui.options_events);
        if let (Some((owner, _)), false) = (self.ui.options_menu.clone(), options.is_empty()) {
            for (kind, id, value) in options {
                let actions = self.mods.options_event(&owner, kind, id, value);
                self.apply_machine_actions(actions);
            }
        }
        // Les réglages changés sont écrits au plus une fois par seconde (un curseur qu'on
        // fait glisser ne doit pas écrire à chaque image).
        if self.ui.settings_saved.elapsed() >= Duration::from_secs(1) {
            self.ui.settings_saved = Instant::now();
            self.mods.save_settings();
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

/// Dessine les éléments d'un menu. Les actions du joueur partent dans `events` (type, id,
/// valeur) ; les valeurs des cases et curseurs sont mises à jour sur place. `slots` dessine
/// des cases d'inventaire (première case, nombre, colonnes) : une page d'options n'en a pas.
pub fn draw_widgets(
    ui: &mut egui::Ui,
    widgets: &mut [Widget],
    events: &mut Vec<(i32, i32, i32)>,
    slots: &mut dyn FnMut(&mut egui::Ui, usize, usize, usize),
) {
    let end = widgets.len();
    draw_range(ui, widgets, 0..end, events, slots);
}

fn draw_range(
    ui: &mut egui::Ui,
    widgets: &mut [Widget],
    range: std::ops::Range<usize>,
    events: &mut Vec<(i32, i32, i32)>,
    slots: &mut dyn FnMut(&mut egui::Ui, usize, usize, usize),
) {
    let mut i = range.start;
    while i < range.end {
        match &mut widgets[i] {
            Widget::Label(text) => {
                ui.label(text.as_str());
            }
            Widget::Separator => {
                ui.separator();
            }
            Widget::Bar { text, value } => {
                ui.add(
                    egui::ProgressBar::new(*value)
                        .corner_radius(0)
                        .text(text.as_str())
                        .desired_width(200.0),
                );
            }
            Widget::Button { id, text } => {
                if ui.button(text.as_str()).clicked() {
                    events.push((UI_BUTTON, *id as i32, 0));
                }
            }
            Widget::Toggle { id, text, on } => {
                if ui.checkbox(on, text.as_str()).changed() {
                    events.push((UI_TOGGLE, *id as i32, *on as i32));
                }
            }
            Widget::Slider {
                id,
                text,
                min,
                max,
                value,
            } => {
                if ui
                    .add(egui::Slider::new(value, *min..=*max).text(text.as_str()))
                    .changed()
                {
                    events.push((UI_SLIDER, *id as i32, value.to_bits() as i32));
                }
            }
            Widget::Slots {
                first,
                count,
                columns,
            } => {
                let (first, count, columns) = (*first, *count, *columns);
                ui.vertical(|ui| slots(ui, first, count, columns));
            }
            Widget::RowBegin => {
                // Jusqu'à la fin de la ligne (l'analyse du menu garantit qu'il y en a une).
                let end = widgets[i + 1..]
                    .iter()
                    .position(|w| *w == Widget::RowEnd)
                    .map_or(range.end, |p| i + 1 + p);
                ui.horizontal(|ui| draw_range(ui, widgets, i + 1..end, events, slots));
                i = end;
            }
            Widget::RowEnd => {}
        }
        i += 1;
    }
}

impl VoxelGame {
    /// Le joueur ouvre la page d'options d'un mod (depuis la pause) : le script est prévenu
    /// et répond avec son menu.
    pub(crate) fn open_mod_options(&mut self, mod_id: &str) {
        self.mode = Mode::Paused(PausePage::ModOptions(mod_id.to_string()));
        self.ui.options_menu = None;
        let actions = self.mods.options_event(mod_id, UI_OPEN, 0, 0);
        self.apply_machine_actions(actions);
    }

    /// Un script envoie le contenu de sa page d'options (seulement si elle est ouverte).
    pub(crate) fn show_mod_options(&mut self, mod_id: String, data: &[u8]) {
        let open = matches!(&self.mode, Mode::Paused(PausePage::ModOptions(id)) if *id == mod_id);
        if !open {
            return;
        }
        match ModMenu::parse(data) {
            Ok(menu) => self.ui.options_menu = Some((mod_id, menu)),
            Err(err) => log::warn!("[mod {mod_id}] page d'options refusée : {err:#}"),
        }
    }

    /// Quitte la page d'options d'un mod, si elle est ouverte : le script est prévenu, et les
    /// réglages sont écrits.
    pub(crate) fn leave_mod_options(&mut self) {
        if let Mode::Paused(PausePage::ModOptions(id)) = &self.mode {
            let id = id.clone();
            let actions = self.mods.options_event(&id, UI_CLOSED, 0, 0);
            self.apply_machine_actions(actions);
        }
        self.ui.options_menu = None;
        self.ui.options_events.clear();
        self.mods.save_settings();
    }
}
