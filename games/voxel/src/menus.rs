//! Les menus : le menu principal (avec le choix et la création des mondes), la pause et
//! les options.
//!
//! C'est une petite **machine à états** : le jeu est dans un [`Mode`] (menu principal,
//! jeu, pause), et chaque menu a ses pages. Un bouton change de page, ou demande à
//! l'application de changer de monde ([`Request`]) : charger un monde, revenir au menu,
//! quitter. L'application décharge alors proprement le monde courant (sauvegarde,
//! maillages libérés) et en crée un autre, sans relancer le programme.

use std::path::PathBuf;
use std::time::SystemTime;

use engine_render::egui::{self, Color32, RichText};

use crate::settings::Action;
use crate::{Launch, Mode, Request, VoxelGame, project_root};

/// Les pages du menu principal.
#[derive(Debug, Clone, PartialEq)]
pub enum TitlePage {
    Main,
    /// Le choix d'un monde ; `confirm` : le monde dont on confirme la suppression.
    Worlds {
        confirm: Option<String>,
    },
    /// La création d'un monde : son nom et sa graine (vide : au hasard).
    NewWorld {
        name: String,
        seed: String,
        /// 0 paisible, 1 normale, 2 difficile.
        difficulty: u8,
        error: Option<String>,
    },
    Options,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PausePage {
    Main,
    Options,
    /// La page d'options d'un mod (son identifiant).
    ModOptions(String),
}

/// Un monde sauvegardé, tel qu'affiché dans la liste.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedWorld {
    pub name: String,
    pub dir: PathBuf,
    /// La date de la dernière sauvegarde, lisible.
    pub date: String,
    pub modified: SystemTime,
    pub seed: Option<u32>,
}

/// Les mondes du dossier `saves/`, du plus récent au plus ancien.
pub fn saved_worlds() -> Vec<SavedWorld> {
    let dir = project_root().join("saves");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut worlds: Vec<SavedWorld> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("world.ron").exists())
        .map(|path| {
            let meta_file = path.join("world.ron");
            let modified = std::fs::metadata(&meta_file)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let seed = engine_voxel::SaveStore::open(&path)
                .ok()
                .and_then(|store| store.load_meta().ok().flatten())
                .map(|meta| meta.seed);
            SavedWorld {
                name: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string(),
                dir: path.clone(),
                date: format_date(modified),
                modified,
                seed,
            }
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.modified));
    worlds
}

/// « 2026-11-09 12:45 » (heure UTC), sans bibliothèque de dates : on compte les jours
/// depuis 1970 et on les convertit en année, mois, jour (l'algorithme « civil from days »
/// de Howard Hinnant).
pub fn format_date(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// Un nom de dossier sûr : lettres, chiffres, `-` et `_` (les autres caractères deviennent `_`).
pub fn sanitize(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Une graine : un nombre, ou un texte quelconque (haché), ou le hasard si vide.
pub fn parse_seed(text: &str) -> u32 {
    let text = text.trim();
    if text.is_empty() {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        return (now.as_nanos() as u32) ^ 0x9E37_79B9;
    }
    text.parse().unwrap_or_else(|_| {
        // FNV-1a : un hachage simple et stable, « montagne » donne toujours le même monde.
        text.bytes().fold(0x811C_9DC5u32, |h, b| {
            (h ^ b as u32).wrapping_mul(0x0100_0193)
        })
    })
}

/// Un gros bouton de menu.
pub(crate) fn button(ui: &mut egui::Ui, text: &str) -> bool {
    crate::skin::menu_button(ui, text)
}

impl VoxelGame {
    /// Le menu ouvert (menu principal ou pause), par-dessus le monde.
    pub(crate) fn draw_menus(&mut self, ctx: &egui::Context) {
        match self.mode.clone() {
            Mode::Title(page) => self.draw_title(ctx, page),
            Mode::Paused(page) => self.draw_pause(ctx, page),
            Mode::Dead(message) => self.draw_dead(ctx, &message),
            Mode::Playing => {}
        }
    }

    pub(crate) fn menu_window(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        egui::Window::new(title)
            .frame(crate::skin::panel())
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    crate::skin::mark(ui);
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(title)
                            .size(30.0)
                            .strong()
                            .color(crate::skin::TEXT),
                    );
                    ui.add_space(12.0);
                    add(ui);
                    ui.add_space(8.0);
                });
            });
    }

    fn draw_title(&mut self, ctx: &egui::Context, page: TitlePage) {
        match page {
            TitlePage::Main => Self::menu_window(ctx, "Voxel", |ui| {
                ui.label(
                    RichText::new("TERRES SAUVAGES · ATELIERS · AVENTURES")
                        .size(11.0)
                        .color(crate::skin::MUTED),
                );
                ui.add_space(12.0);
                if button(ui, "Jouer") {
                    self.worlds = saved_worlds();
                    self.mode = Mode::Title(TitlePage::Worlds { confirm: None });
                }
                if button(ui, "Nouveau monde") {
                    self.mode = Mode::Title(TitlePage::NewWorld {
                        name: "Nouveau monde".into(),
                        seed: String::new(),
                        difficulty: 1,
                        error: None,
                    });
                }
                if button(ui, "Options") {
                    self.mode = Mode::Title(TitlePage::Options);
                }
                if button(ui, "Quitter") {
                    self.request = Some(Request::Quit);
                }
            }),
            TitlePage::Worlds { confirm } => Self::menu_window(ctx, "Mondes", |ui| {
                if self.worlds.is_empty() {
                    ui.label("Aucun monde sauvegardé.");
                }
                let mut delete = None;
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for world in self.worlds.clone() {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&world.name).strong().size(16.0));
                                    let seed = world
                                        .seed
                                        .map_or(String::new(), |s| format!(" — graine {s}"));
                                    ui.label(format!("{}{seed}", world.date));
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if confirm.as_deref() == Some(world.name.as_str()) {
                                            if ui.button("Annuler").clicked() {
                                                self.mode = Mode::Title(TitlePage::Worlds {
                                                    confirm: None,
                                                });
                                            }
                                            if ui
                                                .button(
                                                    RichText::new("Supprimer définitivement")
                                                        .color(crate::skin::RED),
                                                )
                                                .clicked()
                                            {
                                                delete = Some(world.clone());
                                            }
                                        } else {
                                            if ui.button("Supprimer").clicked() {
                                                self.mode = Mode::Title(TitlePage::Worlds {
                                                    confirm: Some(world.name.clone()),
                                                });
                                            }
                                            if ui.button(RichText::new("Jouer").strong()).clicked()
                                            {
                                                self.request = Some(Request::Play(Launch {
                                                    world: Some(world.dir.clone()),
                                                    seed: None,
                                                    title: false,
                                                    difficulty: None,
                                                    dimension: None,
                                                }));
                                            }
                                        }
                                    },
                                );
                            });
                            ui.separator();
                        }
                    });
                if let Some(world) = delete {
                    match std::fs::remove_dir_all(&world.dir) {
                        Ok(()) => log::info!("monde « {} » supprimé", world.name),
                        Err(err) => log::error!("suppression de {} : {err}", world.dir.display()),
                    }
                    self.worlds = saved_worlds();
                    self.mode = Mode::Title(TitlePage::Worlds { confirm: None });
                }
                if button(ui, "Retour") {
                    self.mode = Mode::Title(TitlePage::Main);
                }
            }),
            TitlePage::NewWorld {
                mut name,
                mut seed,
                mut difficulty,
                error,
            } => {
                let mut create = false;
                let mut back = false;
                Self::menu_window(ctx, "Nouveau monde", |ui| {
                    ui.label("Nom");
                    ui.add(egui::TextEdit::singleline(&mut name).desired_width(260.0));
                    ui.label("Graine (vide : au hasard ; un nombre ou un mot)");
                    ui.add(egui::TextEdit::singleline(&mut seed).desired_width(260.0));
                    ui.label("Difficulté");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut difficulty, 0, "Paisible");
                        ui.selectable_value(&mut difficulty, 1, "Normale");
                        ui.selectable_value(&mut difficulty, 2, "Difficile");
                    });
                    ui.label(match difficulty {
                        0 => "Aucune créature hostile.",
                        2 => "Les créatures frappent plus fort.",
                        _ => "Le jeu tel qu'il est prévu.",
                    });
                    if let Some(error) = &error {
                        ui.label(RichText::new(error).color(crate::skin::RED));
                    }
                    ui.add_space(8.0);
                    create = button(ui, "Créer");
                    back = button(ui, "Retour");
                });
                let mut page = TitlePage::NewWorld {
                    name: name.clone(),
                    seed: seed.clone(),
                    difficulty,
                    error,
                };
                if create {
                    let folder = sanitize(&name);
                    let dir = project_root().join("saves").join(&folder);
                    if folder.is_empty() {
                        page = TitlePage::NewWorld {
                            name,
                            seed,
                            difficulty,
                            error: Some("Donne un nom au monde.".into()),
                        };
                    } else if dir.exists() {
                        page = TitlePage::NewWorld {
                            name,
                            seed,
                            difficulty,
                            error: Some(format!("« {folder} » existe déjà.")),
                        };
                    } else {
                        self.request = Some(Request::Play(Launch {
                            world: Some(dir),
                            seed: Some(parse_seed(&seed)),
                            title: false,
                            difficulty: Some(difficulty),
                            dimension: None,
                        }));
                    }
                }
                self.mode = Mode::Title(if back { TitlePage::Main } else { page });
            }
            TitlePage::Options => Self::menu_window(ctx, "Options", |ui| {
                if self.options_panel(ui) {
                    self.mode = Mode::Title(TitlePage::Main);
                }
            }),
        }
    }

    fn draw_pause(&mut self, ctx: &egui::Context, page: PausePage) {
        // Un voile sombre sur le monde arrêté.
        ctx.layer_painter(egui::LayerId::background()).rect_filled(
            ctx.content_rect(),
            0.0,
            Color32::from_black_alpha(110),
        );
        match page {
            PausePage::Main => Self::menu_window(ctx, "Pause", |ui| {
                if button(ui, "Reprendre") {
                    self.mode = Mode::Playing;
                }
                if button(ui, "Options") {
                    self.mode = Mode::Paused(PausePage::Options);
                }
                // Les mods qui ont une page d'options.
                let mods = self.mods.options_mods.clone();
                for (id, name) in mods {
                    if button(ui, &format!("Options : {name}")) {
                        self.open_mod_options(&id);
                    }
                }
                let label = if self.store.is_some() {
                    "Sauvegarder et retourner au menu"
                } else {
                    "Retourner au menu"
                };
                if button(ui, label) {
                    self.request = Some(Request::Title);
                }
            }),
            PausePage::Options => Self::menu_window(ctx, "Options", |ui| {
                if self.options_panel(ui) {
                    self.mode = Mode::Paused(PausePage::Main);
                }
            }),
            PausePage::ModOptions(id) => {
                let title = self
                    .mods
                    .options_mods
                    .iter()
                    .find(|(m, _)| *m == id)
                    .map_or_else(|| id.clone(), |(_, name)| name.clone());
                Self::menu_window(ctx, &title, |ui| {
                    // Le menu est sorti de l'état pendant le dessin ; les actions sont mises en file.
                    if let Some((owner, mut menu)) = self.ui.options_menu.take() {
                        let mut events = Vec::new();
                        crate::mod_menu::draw_widgets(
                            ui,
                            &mut menu.widgets,
                            &mut events,
                            &mut |ui, _, _, _| {
                                ui.label("(cases d'inventaire indisponibles ici)");
                            },
                        );
                        self.ui.options_menu = Some((owner, menu));
                        self.ui.options_events.extend(events);
                    } else {
                        ui.label("Ce mod n'a pas de réglages à afficher.");
                    }
                    ui.add_space(8.0);
                    if button(ui, "Retour") {
                        self.leave_mod_options();
                        self.mode = Mode::Paused(PausePage::Main);
                    }
                });
            }
        }
    }

    /// Les options, communes au menu principal et à la pause. Renvoie `true` quand on
    /// clique « Retour » (les options sont alors enregistrées et appliquées).
    fn options_panel(&mut self, ui: &mut egui::Ui) -> bool {
        let s = &mut self.settings;
        egui::Grid::new("options")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Distance de rendu (chunks)");
                ui.add(egui::Slider::new(&mut s.render_distance, 4..=32))
                    .on_hover_text("Appliquée au prochain monde chargé");
                ui.end_row();
                ui.label("Terrain lointain (chunks, 0 : aucun)");
                ui.add(egui::Slider::new(&mut s.far_distance, 0..=192))
                    .on_hover_text("Décor simplifié au-delà des chunks détaillés. Appliqué au prochain monde chargé");
                ui.end_row();
                ui.label("Champ de vision (°)");
                ui.add(egui::Slider::new(&mut s.fov, 50.0..=110.0));
                ui.end_row();
                ui.label("Sensibilité de la souris");
                ui.add(egui::Slider::new(&mut s.mouse_sensitivity, 0.2..=3.0));
                ui.end_row();
                ui.label("Volume général");
                ui.add(egui::Slider::new(&mut s.volume_master, 0.0..=1.0));
                ui.end_row();
                ui.label("Musique et ambiance");
                ui.add(egui::Slider::new(&mut s.volume_music, 0.0..=1.0));
                ui.end_row();
                ui.label("Effets");
                ui.add(egui::Slider::new(&mut s.volume_effects, 0.0..=1.0));
                ui.end_row();
                ui.label("Synchronisation verticale");
                ui.checkbox(&mut s.vsync, "");
                ui.end_row();
                ui.label("Afficher le bras");
                ui.checkbox(&mut s.show_arm, "");
                ui.end_row();
            });
        ui.separator();
        ui.label(RichText::new("Touches").strong());
        egui::Grid::new("keys")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for action in Action::ALL {
                    ui.label(action.label());
                    let text = if self.waiting_key == Some(action) {
                        "appuie sur une touche…".to_string()
                    } else {
                        self.settings.keys.name(action).to_string()
                    };
                    if ui
                        .add_sized([180.0, 22.0], egui::Button::new(text))
                        .clicked()
                    {
                        self.waiting_key = Some(action);
                    }
                    ui.end_row();
                }
            });
        if ui.button("Touches par défaut").clicked() {
            self.settings.keys = Default::default();
        }
        // Les réglages visibles tout de suite : le champ de vision, les volumes, le bras.
        self.apply_settings();
        ui.add_space(8.0);
        if button(ui, "Retour") {
            self.waiting_key = None;
            self.settings.save(&project_root());
            self.renderer.set_vsync(self.settings.vsync);
            return true;
        }
        false
    }

    /// Applique les options qui ne demandent pas de recharger le monde.
    pub(crate) fn apply_settings(&mut self) {
        self.camera.fov_y = self.settings.fov.to_radians();
        self.hand.visible = self.settings.show_arm && !matches!(self.mode, Mode::Title(_));
        if let Some(audio) = &mut self.audio {
            let (effects, music) = self.settings.volumes();
            audio.set_volumes(effects, music);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_formatted() {
        let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_762_692_300);
        assert_eq!(format_date(time), "2025-11-09 12:45");
        assert_eq!(format_date(SystemTime::UNIX_EPOCH), "1970-01-01 00:00");
    }

    #[test]
    fn names_and_seeds() {
        assert_eq!(sanitize(" Mon monde / 2 "), "Mon_monde___2");
        assert_eq!(parse_seed("42"), 42);
        assert_eq!(parse_seed("montagne"), parse_seed("montagne"));
        assert_ne!(parse_seed("montagne"), parse_seed("vallée"));
    }
}
