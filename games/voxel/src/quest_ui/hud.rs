//! Les quêtes épinglées, en jeu : en haut à droite (sous les potions), le titre et les tâches qui
//! restent. Trois au plus (`quest_progress::MAX_PINNED`). La place se calcule dans `geometry.rs`.

use engine_render::egui::{self, Color32, RichText};

use super::geometry;
use super::labels::{self, Namer};
use crate::VoxelGame;
use crate::quest_progress::Status;
use crate::quests::Quests;
use crate::skin;

/// Au-delà de ce nombre de tâches, la fin est résumée (« + 2 autres »).
const MAX_LINES: usize = 3;

impl VoxelGame {
    pub(crate) fn draw_pinned_quests(&mut self, ctx: &egui::Context) {
        let (Some(quests), Some(progress)) = (
            self.players.me_mut().quest_book.quests(),
            self.players.me_mut().quest_book.progress().cloned(),
        ) else {
            return;
        };
        let pinned: Vec<usize> = progress
            .pinned
            .iter()
            .filter_map(|id| quests.index(id))
            .take(crate::quest_progress::MAX_PINNED)
            .collect();
        if pinned.is_empty() {
            return;
        }
        let screen = ctx.content_rect().width();
        let offset = geometry::pin_offset(
            self.players.me_mut().effects.list().len(),
            screen,
            self.ui.toast.is_some(),
            self.ui.target_bottom,
        );
        let width = geometry::pin_width(screen);
        let mobs = self
            .ecs
            .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
            .clone();
        let namer = Namer {
            items: &self.content.items,
            blocks: &self.registry,
            creatures: &mobs,
        };
        egui::Area::new(egui::Id::unique("pinned_quests"))
            .anchor(egui::Align2::RIGHT_TOP, offset)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 6.0;
                for index in pinned {
                    pin_card(ui, &quests, &progress, index, &namer, width);
                }
            });
    }
}

/// Le cadre d'une quête épinglée.
fn pin_card(
    ui: &mut egui::Ui,
    quests: &Quests,
    progress: &crate::quest_progress::Progress,
    index: usize,
    namer: &Namer,
    width: f32,
) {
    let quest = &quests.quests[index];
    let status = progress.status(quests, index);
    egui::Frame::new()
        .fill(Color32::from_black_alpha(150))
        .stroke(egui::Stroke::new(1.0, skin::COPPER))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(width - 16.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(&quest.title)
                    .size(14.0)
                    .strong()
                    .color(skin::GOLD),
            );
            match status {
                Status::Done => {
                    ui.label(
                        RichText::new("Terminée : à récupérer")
                            .size(12.0)
                            .color(skin::SAGE),
                    );
                }
                Status::Claimed => {
                    ui.label(RichText::new("Terminée").size(12.0).color(skin::MUTED));
                }
                _ => {
                    let remaining = labels::remaining(quest, progress, namer);
                    for (task, have, need) in remaining.iter().take(MAX_LINES) {
                        let line = if *need > 1 {
                            format!("• {task} {have}/{need}")
                        } else {
                            format!("• {task}")
                        };
                        ui.label(RichText::new(line).size(12.0));
                    }
                    if remaining.len() > MAX_LINES {
                        ui.label(
                            RichText::new(format!("+ {} autres", remaining.len() - MAX_LINES))
                                .size(11.0)
                                .color(skin::MUTED),
                        );
                    }
                }
            }
        });
}
