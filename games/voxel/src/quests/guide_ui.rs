//! L'écran du Guide du voyageur : les chapitres à gauche, la page à droite, et sous le texte les
//! objets dont un clic ouvre les recettes dans l'explorateur. Le contenu est dans
//! `quests/guide.rs`.

use engine_render::egui;
#[allow(unused_imports)]
use voxel_sim::quests::guide_ui::*;

use crate::VoxelGame;
use crate::ui::recipe_browser::Lookup;
use crate::ui::{Press, Screen};
use voxel_sim::player::inventory::ItemStack;
use voxel_sim::quests::guide::expand_keys;

/// Où l'on en est dans le guide (gardé d'une ouverture à l'autre).
#[derive(Debug, Default)]
pub struct GuideState {
    pub chapter: usize,
    pub page: usize,
}

/// La largeur de la colonne des chapitres.
const CHAPTERS_WIDTH: f32 = 152.0;

impl VoxelGame {
    /// Ouvre le guide à la page où on l'avait laissé.
    pub(crate) fn open_guide(&mut self) {
        self.client.ui.screen = Screen::Guide;
        self.client.ui.inventory_open = true;
    }

    /// Le corps de l'écran du guide.
    pub(crate) fn guide_panel(&mut self, ui: &mut egui::Ui) {
        let guide = self.client.content.guide.clone();
        if guide.chapters.is_empty() {
            ui.label("Ce guide est vide.");
            return;
        }
        let state = &mut self.client.ui.guide;
        state.chapter = state.chapter.min(guide.chapters.len() - 1);
        let chapter = &guide.chapters[state.chapter];
        state.page = state.page.min(chapter.pages.len() - 1);
        let (mut chapter_index, mut page_index) = (state.chapter, state.page);
        let height = (ui.ctx().content_rect().height() * 0.7).clamp(220.0, 600.0);
        let mut open = None;
        ui.horizontal_top(|ui| {
            // Les chapitres.
            ui.vertical(|ui| {
                ui.set_width(CHAPTERS_WIDTH);
                egui::ScrollArea::vertical()
                    .id_salt("guide_chapters")
                    .max_height(height)
                    .show(ui, |ui| {
                        for (i, c) in guide.chapters.iter().enumerate() {
                            if ui.selectable_label(i == chapter_index, &c.title).clicked() {
                                chapter_index = i;
                                page_index = 0;
                            }
                        }
                    });
            });
            ui.separator();
            // La page.
            ui.vertical(|ui| {
                let chapter = &guide.chapters[chapter_index];
                let page = &chapter.pages[page_index.min(chapter.pages.len() - 1)];
                ui.horizontal(|ui| {
                    if let Some(icon) = chapter.icon {
                        self.stack_widget(ui, Some(ItemStack::new(icon, 1)), false);
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&chapter.title).weak());
                        ui.heading(&page.title);
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("guide_page", chapter_index, page_index))
                    .max_height(height - 110.0)
                    .show(ui, |ui| {
                        ui.label(expand_keys(&page.text, &self.settings.keys));
                        if !page.items.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("Clic sur un objet : ses recettes.").weak(),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for &item in &page.items {
                                    if let Some(Press::Primary | Press::Secondary) =
                                        self.stack_widget(ui, Some(ItemStack::new(item, 1)), false)
                                    {
                                        open = Some(item);
                                    }
                                }
                            });
                        }
                    });
                ui.separator();
                // Les pages du chapitre.
                let pages = chapter.pages.len();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(page_index > 0, egui::Button::new("◀"))
                        .clicked()
                    {
                        page_index -= 1;
                    }
                    ui.label(format!("Page {} / {pages}", page_index + 1));
                    if ui
                        .add_enabled(page_index + 1 < pages, egui::Button::new("▶"))
                        .clicked()
                    {
                        page_index += 1;
                    }
                });
            });
        });
        self.client.ui.guide.chapter = chapter_index;
        self.client.ui.guide.page = page_index;
        if let Some(item) = open {
            self.client.ui.browser.open(item, Lookup::Recipes);
        }
    }
}
