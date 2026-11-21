//! L'écran du livre de quêtes : une fenêtre qui prend tout l'écran (le jeu continue de tourner
//! derrière, comme avec l'inventaire), ouverte par la touche L ou le bouton « Quêtes » de
//! l'inventaire, fermée par Échap.
//!
//! - **en haut** : le titre du chapitre et sa progression, « Tout récupérer », le zoom ;
//! - **à gauche** : les groupes (repliables) et leurs chapitres ;
//! - **au centre** : le plan du chapitre (`quest_ui/map.rs`) ;
//! - **à droite** : la fiche de la quête choisie (`quest_ui/sheet.rs`).
//!
//! Hors du livre, `quest_ui/hud.rs` affiche les quêtes épinglées. La géométrie (cadrage, zoom,
//! traits, empilement des épingles) est dans `quest_ui/geometry.rs`, les textes dans
//! `quest_ui/labels.rs` : tout ce qui se décide sans dessiner y est testé.
//!
//! Le dessin ne change rien pendant qu'il tourne : les clics deviennent des [`Act`], appliqués
//! une fois l'image décrite (récupérer, remettre, épingler, changer de chapitre...). Les
//! changements de progression passent par `quest_play.rs`, comme les commandes `ctl`.

mod geometry;
mod hud;
mod labels;
mod map;
mod sheet;

use crate::protocol::{ClientMsg, QuestAct, ScreenAction};
use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, bail};
use engine_render::egui::{
    self, Align, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, vec2,
};
use engine_voxel::ItemId;

use crate::VoxelGame;
use crate::quest_progress::{Progress, Status};
use crate::quests::Quests;
use crate::recipe_browser::Lookup;
use crate::skin;
use crate::ui::Screen;
use geometry::MapView;

/// La hauteur de la barre du haut.
const TOP_HEIGHT: f32 = 40.0;
/// La largeur de la colonne des chapitres, au plus.
const CHAPTERS_WIDTH: f32 = 214.0;
/// La largeur de la fiche, au plus.
const SHEET_WIDTH: f32 = 350.0;
const GAP: f32 = 8.0;
/// La hauteur d'une ligne de chapitre.
const CHAPTER_ROW: f32 = 52.0;

/// Où l'on en est dans le livre (gardé d'une ouverture à l'autre).
#[derive(Debug, Default)]
pub struct QuestScreen {
    /// Le chapitre montré (numéro dans `Quests::chapters`).
    pub chapter: usize,
    /// La quête dont la fiche est ouverte.
    pub selected: Option<usize>,
    pub view: MapView,
    /// Cadrer le plan sur ses quêtes à la prochaine image (ouverture, changement de chapitre).
    pub fit: bool,
    /// Le plan est resté tel que le cadrage l'a laissé (ni glissé ni zoomé) : il se recadre quand
    /// la fenêtre change de taille (la fiche s'ouvre ou se ferme).
    auto_fit: bool,
    /// Les groupes repliés (leur identifiant).
    collapsed: HashSet<String>,
    /// Où la fenêtre du plan était à la dernière image.
    viewport: Option<Rect>,
    /// La taille de la fenêtre du jeu à la dernière image (pour distinguer la fiche qui s'ouvre
    /// d'une fenêtre qu'on redimensionne).
    screen: egui::Vec2,
    /// Les quêtes dessinées à la dernière image : (numéro, centre à l'écran, rayon), pour `ctl`.
    drawn: Vec<(usize, Pos2, f32)>,
}

/// Ce que le joueur a demandé pendant l'image, appliqué après le dessin.
#[derive(Debug, Clone, PartialEq)]
enum Act {
    Chapter(usize),
    Select(Option<usize>),
    /// Aller à une quête, même d'un autre chapitre.
    GoTo(usize),
    Claim(usize),
    ClaimAll,
    Check(usize, usize),
    HandIn(usize, usize),
    Choose(usize, usize, usize),
    Pin(usize),
    /// Ouvrir ce chapitre du guide.
    Guide(String),
    Recipes(ItemId),
    ToggleGroup(String),
    Fit,
    Zoom(f32),
    Close,
}

/// Un texte qui tient dans `width` points sur `rows` lignes au plus (« … » au bout).
fn fit_text(
    painter: &egui::Painter,
    text: &str,
    size: f32,
    color: Color32,
    width: f32,
    rows: usize,
) -> Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple(text.into(), FontId::monospace(size), color, width.max(8.0));
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('…');
    painter.layout_job(job)
}

impl VoxelGame {
    /// La touche des quêtes : ouvre le livre, ou le referme.
    pub(crate) fn toggle_quests_screen(&mut self) {
        if self.client.ui.inventory_open && self.client.ui.screen == Screen::Quests {
            self.close_screen();
        } else if self.client.mouse_captured && !self.client.ui.inventory_open {
            let _ = self.open_quest_book(None);
        }
    }

    /// Ouvre le livre, au chapitre demandé ou, sinon, au dernier regardé (le premier à défaut).
    pub(crate) fn open_quest_book(&mut self, chapter: Option<&str>) -> Result<()> {
        self.open_quests();
        let quests = self
            .server
            .players
            .me_mut()
            .quest_book
            .quests()
            .ok_or_else(|| anyhow::anyhow!("quêtes pas chargées"))?;
        let wanted = match chapter {
            Some(id) => Some(
                quests
                    .chapters
                    .iter()
                    .position(|c| c.id == id)
                    .ok_or_else(|| anyhow::anyhow!("chapitre inconnu « {id} » (voir `quests`)"))?,
            ),
            None => None,
        };
        // Une revue tout de suite : les objets portés comptent dès l'ouverture.
        self.refresh_quests();
        let last = self
            .server
            .players
            .me_mut()
            .quest_book
            .progress()
            .and_then(|p| p.last_chapter.clone())
            .and_then(|id| quests.chapters.iter().position(|c| c.id == id));
        let index = wanted.or(last).unwrap_or(0);
        self.client.ui.screen = Screen::Quests;
        self.client.ui.inventory_open = true;
        self.client.mouse_captured = false;
        self.show_quest_chapter(index, &quests);
        Ok(())
    }

    /// Montre un chapitre : le plan se cadre à l'image suivante.
    fn show_quest_chapter(&mut self, chapter: usize, quests: &Quests) {
        if quests.chapters.is_empty() {
            return;
        }
        let chapter = chapter.min(quests.chapters.len() - 1);
        self.client.ui.quests.chapter = chapter;
        self.client.ui.quests.selected = None;
        self.client.ui.quests.fit = true;
        self.send_now(ClientMsg::Screen(ScreenAction::Quest(
            QuestAct::LastChapter(quests.chapters[chapter].id.clone()),
        )));
    }

    /// Va à une quête : son chapitre, le plan centré dessus, sa fiche ouverte.
    pub(crate) fn go_to_quest(&mut self, quest: usize, quests: &Quests) {
        let target = &quests.quests[quest];
        if target.chapter != self.client.ui.quests.chapter {
            self.show_quest_chapter(target.chapter, quests);
        }
        let state = &mut self.client.ui.quests;
        state.selected = Some(quest);
        state.fit = false;
        state.auto_fit = false;
        // La fiche s'ouvre peut-être : le plan reste centré sur la quête, sans se décaler.
        state.viewport = None;
        state.view.center_on(egui::pos2(target.x, target.y));
        state.view.zoom = state.view.zoom.max(0.8);
    }

    /// Le bouton « Quêtes » de l'écran d'inventaire, et ce qui attend d'être récupéré.
    pub(crate) fn quests_button(&mut self, ui: &mut egui::Ui) {
        // Ce qui attend d'être récupéré, d'après la progression que le serveur a envoyée.
        let waiting = match (
            self.server.players.local().quest_book.quests(),
            &self.client.view.me.quests,
        ) {
            (Some(quests), Some(progress)) => Some(progress.pending(&quests).len()),
            _ => None,
        }
        .filter(|&n| n > 0);
        ui.horizontal(|ui| {
            if ui.small_button("Quêtes").clicked() {
                let _ = self.open_quest_book(None);
            }
            if let Some(n) = waiting {
                ui.label(
                    RichText::new(if n == 1 {
                        "1 récompense attend".to_string()
                    } else {
                        format!("{n} récompenses attendent")
                    })
                    .size(11.0)
                    .strong()
                    .color(Color32::from_rgb(138, 78, 18)),
                );
            }
        });
    }

    /// Le livre, plein écran.
    pub(crate) fn draw_quests_screen(&mut self, ctx: &egui::Context) {
        self.client.ui.browser.hovered = None;
        self.open_quests();
        // Les quêtes sont le contenu du jeu ; la progression vient de la vue du joueur.
        let (Some(quests), Some(progress)) = (
            self.server.players.local().quest_book.quests(),
            self.client.view.me.quests.clone(),
        ) else {
            return;
        };
        let statuses: Vec<Status> = (0..quests.quests.len())
            .map(|i| progress.status(&quests, i))
            .collect();
        if !quests.chapters.is_empty() {
            self.client.ui.quests.chapter =
                self.client.ui.quests.chapter.min(quests.chapters.len() - 1);
        }
        let safe = crate::ui_layout::safe_rect(ctx);
        let frame = skin::panel().inner_margin(egui::Margin::same(10));
        let inner = safe.size() - frame.total_margin().sum();
        let mut acts = Vec::new();
        // Au fond : les bannières (`draw_toast`) et les infobulles restent par-dessus.
        egui::Area::new(egui::Id::unique("quest_book"))
            .order(egui::Order::Background)
            .fixed_pos(safe.min)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_min_size(inner);
                    ui.set_max_size(inner);
                    self.draw_quest_book(ui, &quests, &progress, &statuses, &mut acts);
                });
            });
        self.apply_quest_acts(acts, &quests);
        // Les recettes d'un objet cliqué : par-dessus le livre, au milieu.
        let width = 520.0_f32.min(safe.width());
        let area = Rect::from_min_size(
            egui::pos2(safe.center().x - width * 0.5, safe.top()),
            vec2(width, safe.height()),
        );
        self.draw_recipe_overlay(ctx, area);
    }

    fn draw_quest_book(
        &mut self,
        ui: &mut egui::Ui,
        quests: &Arc<Quests>,
        progress: &Progress,
        statuses: &[Status],
        acts: &mut Vec<Act>,
    ) {
        let rect = ui.max_rect();
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), TOP_HEIGHT));
        let body = Rect::from_min_max(egui::pos2(rect.left(), top.bottom() + GAP), rect.max);
        if quests.chapters.is_empty() {
            ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Quêtes");
                    if ui.button("×").on_hover_text("Fermer · Échap").clicked() {
                        acts.push(Act::Close);
                    }
                });
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.label("Aucune quête dans ce monde.");
            });
            return;
        }
        let chapter = self.client.ui.quests.chapter;
        let selected = self
            .client
            .ui
            .quests
            .selected
            .filter(|&q| q < quests.quests.len());
        let left_w = CHAPTERS_WIDTH.min(body.width() * 0.26);
        let right_w = if selected.is_some() {
            SHEET_WIDTH.min(body.width() * 0.4)
        } else {
            0.0
        };
        let left = Rect::from_min_size(body.min, vec2(left_w, body.height()));
        let right = Rect::from_min_max(
            egui::pos2(body.right() - right_w, body.top()),
            body.right_bottom(),
        );
        let center = Rect::from_min_max(
            egui::pos2(left.right() + GAP, body.top()),
            egui::pos2(
                if right_w > 0.0 {
                    right.left() - GAP
                } else {
                    body.right()
                },
                body.bottom(),
            ),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| {
            self.draw_quest_top(ui, quests, progress, chapter, acts);
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            self.draw_chapter_list(ui, left, quests, progress, acts);
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(center), |ui| {
            self.draw_quest_map(ui, center, quests, statuses, selected, acts);
        });
        if let Some(quest) = selected {
            ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                self.draw_quest_sheet(ui, right, quests, progress, statuses, quest, acts);
            });
        }
    }

    /// La barre du haut : le chapitre et sa progression, « Tout récupérer », zoom, fermer.
    fn draw_quest_top(
        &mut self,
        ui: &mut egui::Ui,
        quests: &Quests,
        progress: &Progress,
        chapter: usize,
        acts: &mut Vec<Act>,
    ) {
        let info = &quests.chapters[chapter];
        let (done, total) = progress.chapter_progress(quests, chapter);
        let waiting = progress.pending(quests).len();
        ui.horizontal(|ui| {
            ui.set_height(TOP_HEIGHT);
            self.quest_cell(ui, info.icon, 1, 34.0, false, "");
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(&info.title).size(20.0).strong());
                ui.label(
                    RichText::new(format!("{done} / {total} quêtes"))
                        .size(12.0)
                        .color(if done == total && total > 0 {
                            skin::SAGE
                        } else {
                            skin::MUTED
                        }),
                );
            });
            let (bar, _) = ui.allocate_exact_size(vec2(150.0, 10.0), Sense::hover());
            ui.painter().rect_filled(bar, 0, skin::INSET);
            let mut fill = bar.shrink(1.0);
            fill.set_width(fill.width() * done as f32 / total.max(1) as f32);
            ui.painter().rect_filled(fill, 0, skin::SAGE);
            ui.painter().rect_stroke(
                bar,
                0,
                egui::Stroke::new(1.0, skin::BORDER),
                egui::StrokeKind::Inside,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_sized([34.0, 30.0], egui::Button::new("×"))
                    .on_hover_text("Fermer · Échap")
                    .clicked()
                {
                    acts.push(Act::Close);
                }
                let label = if waiting > 0 {
                    format!("Tout récupérer ({waiting})")
                } else {
                    "Tout récupérer".to_string()
                };
                if ui
                    .add_enabled(waiting > 0, egui::Button::new(label))
                    .on_hover_text("Récupère les récompenses de toutes les quêtes terminées (sauf celles qui demandent un choix)")
                    .clicked()
                {
                    acts.push(Act::ClaimAll);
                }
                if ui.button("Recentrer").on_hover_text("Cadre tout le plan").clicked() {
                    acts.push(Act::Fit);
                }
                if ui
                    .add_sized([30.0, 30.0], egui::Button::new("+"))
                    .on_hover_text("Zoom avant · molette")
                    .clicked()
                {
                    acts.push(Act::Zoom(1.25));
                }
                if ui
                    .add_sized([30.0, 30.0], egui::Button::new("−"))
                    .on_hover_text("Zoom arrière · molette")
                    .clicked()
                {
                    acts.push(Act::Zoom(0.8));
                }
            });
        });
    }

    /// La colonne de gauche : les groupes et leurs chapitres.
    fn draw_chapter_list(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        quests: &Quests,
        progress: &Progress,
        acts: &mut Vec<Act>,
    ) {
        ui.painter().rect_filled(rect, 0, skin::INSET);
        ui.painter().rect_stroke(
            rect,
            0,
            egui::Stroke::new(1.0, skin::BORDER),
            egui::StrokeKind::Inside,
        );
        let group_of: Vec<usize> = quests
            .chapters
            .iter()
            .map(|c| {
                quests
                    .groups
                    .iter()
                    .position(|g| g.id == c.group)
                    .unwrap_or(0)
            })
            .collect();
        let grouped = geometry::chapters_by_group(&group_of, quests.groups.len());
        let current = self.client.ui.quests.chapter;
        let time = ui.input(|i| i.time);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(5.0)), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("quest_chapters")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (g, chapters) in grouped {
                        let group = &quests.groups[g];
                        let collapsed = self.client.ui.quests.collapsed.contains(&group.id);
                        let waiting: usize = chapters
                            .iter()
                            .map(|&c| progress.pending_in(quests, c))
                            .sum();
                        // L'en-tête du groupe : un clic le replie.
                        let (row, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                        let p = ui.painter();
                        p.rect_filled(
                            row,
                            0,
                            if response.hovered() {
                                Color32::from_rgb(52, 54, 51)
                            } else {
                                Color32::from_rgb(40, 42, 40)
                            },
                        );
                        p.text(
                            row.left_center() + vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            if collapsed { "▶" } else { "▼" },
                            FontId::monospace(11.0),
                            skin::COPPER,
                        );
                        let icon = self.client.ui.icon(
                            group.icon,
                            &self.content.items,
                            &self.registry,
                            &self.client.block_textures,
                        );
                        let icon_rect = Rect::from_center_size(
                            row.left_center() + vec2(34.0, 0.0),
                            egui::Vec2::splat(22.0),
                        );
                        ui.painter().image(
                            icon,
                            icon_rect,
                            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        let galley = fit_text(
                            ui.painter(),
                            &group.title,
                            14.0,
                            skin::TEXT,
                            row.width() - 80.0,
                            1,
                        );
                        ui.painter().galley(
                            egui::pos2(row.left() + 52.0, row.center().y - galley.size().y * 0.5),
                            galley,
                            skin::TEXT,
                        );
                        if collapsed && waiting > 0 {
                            badge(
                                ui.painter(),
                                row.right_center() - vec2(14.0, 0.0),
                                waiting,
                                time,
                            );
                        }
                        if response.clicked() {
                            acts.push(Act::ToggleGroup(group.id.clone()));
                        }
                        if collapsed {
                            continue;
                        }
                        for c in chapters {
                            if self.chapter_row(ui, quests, progress, c, c == current, time) {
                                acts.push(Act::Chapter(c));
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
        });
    }

    /// Une ligne de chapitre : icône, titre, progression, pastille. `true` si on a cliqué.
    fn chapter_row(
        &mut self,
        ui: &mut egui::Ui,
        quests: &Quests,
        progress: &Progress,
        c: usize,
        current: bool,
        time: f64,
    ) -> bool {
        let chapter = &quests.chapters[c];
        let (done, total) = progress.chapter_progress(quests, c);
        let waiting = progress.pending_in(quests, c);
        let (row, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), CHAPTER_ROW), Sense::click());
        let icon = self.client.ui.icon(
            chapter.icon,
            &self.content.items,
            &self.registry,
            &self.client.block_textures,
        );
        let p = ui.painter();
        p.rect_filled(
            row,
            0,
            if current {
                Color32::from_rgb(70, 66, 56)
            } else if response.hovered() {
                Color32::from_rgb(52, 54, 51)
            } else {
                skin::PANEL
            },
        );
        if current {
            p.rect_filled(
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                0,
                skin::COPPER,
            );
        }
        let icon_rect =
            Rect::from_center_size(row.left_center() + vec2(24.0, 0.0), egui::Vec2::splat(28.0));
        p.image(
            icon,
            icon_rect,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let text_left = row.left() + 46.0;
        let text_width = row.right() - text_left - 8.0;
        let galley = fit_text(
            p,
            &chapter.title,
            13.0,
            if current {
                skin::TEXT
            } else {
                Color32::from_rgb(220, 218, 208)
            },
            text_width - if waiting > 0 { 18.0 } else { 0.0 },
            2,
        );
        p.galley(egui::pos2(text_left, row.top() + 6.0), galley, skin::TEXT);
        // La barre de progression et « n / total ».
        let count = format!("{done} / {total}");
        let finished = total > 0 && done == total;
        let count_galley = p.layout_no_wrap(
            count,
            FontId::monospace(11.0),
            if finished { skin::SAGE } else { skin::MUTED },
        );
        let bar = Rect::from_min_size(
            egui::pos2(text_left, row.bottom() - 13.0),
            vec2((text_width - count_galley.size().x - 6.0).max(10.0), 4.0),
        );
        p.rect_filled(bar, 0, skin::INSET);
        let mut fill = bar;
        fill.set_width(bar.width() * done as f32 / total.max(1) as f32);
        p.rect_filled(fill, 0, if finished { skin::SAGE } else { skin::COPPER });
        p.galley(
            egui::pos2(
                bar.right() + 6.0,
                bar.center().y - count_galley.size().y * 0.5,
            ),
            count_galley,
            skin::MUTED,
        );
        if waiting > 0 {
            badge(
                p,
                egui::pos2(row.right() - 14.0, row.top() + 14.0),
                waiting,
                time,
            );
        }
        response.clicked()
    }

    fn send_quest_act(&mut self, act: QuestAct) {
        self.send_now(ClientMsg::Screen(ScreenAction::Quest(act)));
    }

    /// `ScreenAction::Quest` : le serveur applique un geste du livre au joueur courant.
    pub(crate) fn quest_act(&mut self, act: QuestAct) {
        let Some(quests) = self.server.players.me().quest_book.quests() else {
            return;
        };
        let count = quests.quests.len();
        let quest_ok = |q: u32| (q as usize) < count;
        match act {
            QuestAct::LastChapter(id) => {
                if let Ok(progress) = self.server.players.me_mut().quest_book.progress_mut() {
                    progress.set_last_chapter(&id);
                }
            }
            QuestAct::Claim(quest) if quest_ok(quest) => match self.claim_quest(quest as usize) {
                Ok(gave) if gave.is_empty() => self.toast("Quête récupérée"),
                Ok(gave) => self.toast(&format!("Récupéré : {gave}")),
                Err(err) => self.toast(&err.to_string()),
            },
            QuestAct::ClaimAll => {
                let claimed = self.claim_all_quests();
                self.toast(&if claimed == 0 {
                    "Rien à récupérer (ou un choix à faire)".to_string()
                } else {
                    format!("{claimed} quête(s) récupérée(s)")
                });
            }
            QuestAct::Check(quest, task) if quest_ok(quest) => {
                if let Ok(progress) = self.server.players.me_mut().quest_book.progress_mut() {
                    progress.check(&quests, quest as usize, task as usize);
                }
                self.refresh_quests();
            }
            QuestAct::HandIn(quest, task) if quest_ok(quest) => {
                // Pas de bannière : la fiche montre le compteur, et la quête finie s'annonce.
                self.hand_in_quest(quest as usize, task as usize);
            }
            QuestAct::Choose(quest, reward, option) if quest_ok(quest) => {
                if let Ok(progress) = self.server.players.me_mut().quest_book.progress_mut() {
                    progress.choose(&quests, quest as usize, reward as usize, option as usize);
                }
            }
            QuestAct::Pin(quest) if quest_ok(quest) => {
                let id = quests.quests[quest as usize].id.clone();
                let changed = self
                    .server
                    .players
                    .me_mut()
                    .quest_book
                    .progress_mut()
                    .is_ok_and(|p| p.toggle_pin(&id));
                if !changed {
                    self.toast("Trois quêtes épinglées au plus");
                }
            }
            act => log::warn!("geste du livre de quêtes refusé : {act:?}"),
        }
    }

    /// Applique ce que le joueur a demandé pendant l'image.
    fn apply_quest_acts(&mut self, acts: Vec<Act>, quests: &Arc<Quests>) {
        for act in acts {
            match act {
                Act::Chapter(c) => self.show_quest_chapter(c, quests),
                Act::Select(quest) => self.client.ui.quests.selected = quest,
                Act::GoTo(quest) => self.go_to_quest(quest, quests),
                // Ce qui change la progression part au serveur.
                Act::Claim(quest) => self.send_quest_act(QuestAct::Claim(quest as u32)),
                Act::ClaimAll => self.send_quest_act(QuestAct::ClaimAll),
                Act::Check(quest, task) => {
                    self.send_quest_act(QuestAct::Check(quest as u32, task as u32));
                }
                Act::HandIn(quest, task) => {
                    self.send_quest_act(QuestAct::HandIn(quest as u32, task as u32));
                }
                Act::Choose(quest, reward, option) => self.send_quest_act(QuestAct::Choose(
                    quest as u32,
                    reward as u32,
                    option as u32,
                )),
                Act::Pin(quest) => self.send_quest_act(QuestAct::Pin(quest as u32)),
                Act::Guide(chapter) => {
                    self.client.ui.browser.close();
                    if let Some(at) = self
                        .content
                        .guide
                        .chapters
                        .iter()
                        .position(|c| c.id == chapter)
                    {
                        self.client.ui.guide.chapter = at;
                        self.client.ui.guide.page = 0;
                    }
                    self.open_guide();
                    return;
                }
                Act::Recipes(item) => self.client.ui.browser.open(item, Lookup::Recipes),
                Act::ToggleGroup(id) => {
                    if !self.client.ui.quests.collapsed.remove(&id) {
                        self.client.ui.quests.collapsed.insert(id);
                    }
                }
                Act::Fit => self.client.ui.quests.fit = true,
                Act::Zoom(factor) => {
                    self.client.ui.quests.auto_fit = false;
                    if let Some(viewport) = self.client.ui.quests.viewport {
                        self.client
                            .ui
                            .quests
                            .view
                            .zoom_at(factor, viewport.center(), viewport);
                    }
                }
                Act::Close => {
                    self.close_screen();
                    return;
                }
            }
        }
    }

    /// Une case d'objet du livre : icône, nombre, bulle d'aide. Un clic ouvre ses recettes
    /// (l'appelant lit `clicked()`).
    pub(super) fn quest_cell(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        count: u32,
        size: f32,
        selected: bool,
        hint: &str,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        let icon = self.client.ui.icon(
            item,
            &self.content.items,
            &self.registry,
            &self.client.block_textures,
        );
        let painter = ui.painter().clone();
        skin::slot(&painter, rect, selected, response.hovered());
        skin::stack(&painter, rect, icon, count, None, None);
        if response.hovered() {
            self.client.ui.browser.hovered = Some(item);
            let mut card = crate::ui_info::item(self.content.items.get(item), count, 0);
            if !hint.is_empty() {
                card.lines.push((hint.to_string(), skin::MUTED));
            }
            crate::ui_info::hover(response.clone(), &card);
        }
        response
    }

    // ----- Commandes `ctl` ---------------------------------------------------------------

    /// Choisit une quête dans le livre ouvert (l'ouvre au besoin) : son chapitre, le plan centré
    /// dessus, sa fiche ouverte.
    pub(crate) fn select_quest(&mut self, id: &str) -> Result<()> {
        self.open_quests();
        let Some(quests) = self.server.players.me_mut().quest_book.quests() else {
            bail!("quêtes pas chargées");
        };
        let Some(index) = quests.index(id) else {
            bail!("quête inconnue « {id} »");
        };
        if !(self.client.ui.inventory_open && self.client.ui.screen == Screen::Quests) {
            self.open_quest_book(None)?;
        }
        self.go_to_quest(index, &quests);
        Ok(())
    }

    /// Règle le plan : cadrage automatique, ou zoom absolu (`fit`, `zoom 1.5`).
    pub(crate) fn set_quest_view(&mut self, zoom: Option<f32>) {
        match zoom {
            None => self.client.ui.quests.fit = true,
            Some(z) => {
                self.client.ui.quests.fit = false;
                self.client.ui.quests.auto_fit = false;
                self.client.ui.quests.view.zoom = z.clamp(geometry::MIN_ZOOM, geometry::MAX_ZOOM);
            }
        }
    }

    /// L'état de l'écran des quêtes pour `ctl` : le chapitre, la fiche, le zoom, et où chaque
    /// quête est dessinée (en points, comme `click x y`).
    pub(crate) fn quest_screen_json(&self) -> String {
        let state = &self.client.ui.quests;
        let quests = self.server.players.me().quest_book.quests();
        let id_of = |q: usize| {
            quests
                .as_ref()
                .map_or_else(String::new, |qs| qs.quests[q].id.clone())
        };
        let chapter = quests
            .as_ref()
            .and_then(|qs| qs.chapters.get(state.chapter))
            .map_or_else(String::new, |c| c.id.clone());
        let nodes: Vec<String> = state
            .drawn
            .iter()
            .map(|(q, at, r)| {
                format!(
                    "{{\"id\":{},\"x\":{:.0},\"y\":{:.0},\"r\":{:.0}}}",
                    crate::control::json_str(&id_of(*q)),
                    at.x,
                    at.y,
                    r
                )
            })
            .collect();
        format!(
            "{{\"ok\":true,\"open\":{},\"chapter\":{},\"selected\":{},\"zoom\":{:.2},\"center\":[{:.2},{:.2}],\"nodes\":[{}]}}",
            self.client.ui.inventory_open && self.client.ui.screen == Screen::Quests,
            crate::control::json_str(&chapter),
            state.selected.map_or_else(
                || "null".to_string(),
                |q| crate::control::json_str(&id_of(q))
            ),
            state.view.zoom,
            state.view.center.x,
            state.view.center.y,
            nodes.join(",")
        )
    }
}

/// Une pastille dorée avec le nombre de récompenses qui attendent ; elle pulse doucement.
pub(super) fn badge(painter: &egui::Painter, at: Pos2, count: usize, time: f64) {
    let pulse = 1.0 + 0.12 * (time * 4.0).sin() as f32;
    painter.circle(
        at,
        8.5 * pulse,
        skin::GOLD,
        egui::Stroke::new(1.5, skin::DARK),
    );
    painter.text(
        at,
        egui::Align2::CENTER_CENTER,
        if count > 9 {
            "9+".to_string()
        } else {
            count.to_string()
        },
        FontId::monospace(10.0),
        skin::DARK,
    );
}
