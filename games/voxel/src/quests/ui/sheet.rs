//! La fiche de la quête choisie, à droite du plan : titre, texte, tâches (avec leur progression et
//! leurs boutons), récompenses, épingle, renvoi au guide, dépendances manquantes.

use engine_render::egui::{self, RichText, Sense, Stroke, StrokeKind, vec2};

use super::Act;
use super::labels::{self, Namer};
use super::map::check_mark;
use crate::VoxelGame;
use crate::ui::skin;
use voxel_sim::quests::guide::expand_keys;
use voxel_sim::quests::progress::{Progress, Status};
use voxel_sim::quests::{Quest, Quests, Requires, Reward, Task};

/// La taille des cases d'objet de la fiche.
const CELL: f32 = 36.0;

impl VoxelGame {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_quest_sheet(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        quests: &Quests,
        progress: &Progress,
        statuses: &[Status],
        index: usize,
        acts: &mut Vec<Act>,
    ) {
        ui.painter().rect_filled(rect, 0, skin::PANEL);
        ui.painter()
            .rect_stroke(rect, 0, Stroke::new(1.0, skin::BORDER), StrokeKind::Inside);
        let quest = &quests.quests[index];
        let status = statuses[index];
        let mobs = self
            .server
            .ecs
            .resource::<std::sync::Arc<voxel_sim::mobs::MobRegistry>>()
            .clone();
        let items = self.server.content.items.clone();
        let registry = self.server.registry.clone();
        let namer = Namer {
            items: &items,
            blocks: &registry,
            creatures: &mobs,
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(10.0)), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("quest_sheet")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    // Le titre, et de quoi fermer la fiche.
                    ui.horizontal_top(|ui| {
                        let title_width = ui.available_width() - 40.0;
                        ui.allocate_ui(vec2(title_width, 0.0), |ui| {
                            ui.set_width(title_width);
                            ui.label(RichText::new(&quest.title).size(19.0).strong());
                        });
                        if ui
                            .add_sized([30.0, 26.0], egui::Button::new("×"))
                            .on_hover_text("Fermer la fiche")
                            .clicked()
                        {
                            acts.push(Act::Select(None));
                        }
                    });
                    if let Some(sub) = &quest.subtitle {
                        ui.label(RichText::new(sub).color(skin::MUTED));
                    }
                    let color = match status {
                        Status::Locked => skin::MUTED,
                        Status::Available => skin::TEXT,
                        Status::Started | Status::Done => skin::GOLD,
                        Status::Claimed => skin::SAGE,
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(labels::status_label(status))
                                .size(12.0)
                                .color(color),
                        );
                        if quest.optional {
                            ui.label(RichText::new("· facultative").size(12.0).color(skin::MAGIC));
                        }
                    });
                    ui.separator();
                    for paragraph in labels::paragraphs(&quest.text) {
                        ui.label(expand_keys(paragraph, &self.settings.keys));
                    }
                    self.sheet_dependencies(ui, quests, progress, index, acts);
                    self.sheet_tasks(ui, quest, index, progress, &namer, acts);
                    self.sheet_rewards(ui, quest, index, status, progress, &namer, acts);
                    self.sheet_buttons(ui, quest, index, status, progress, acts);
                });
        });
    }

    /// Les dépendances qui manquent : un clic y mène, même dans un autre chapitre.
    fn sheet_dependencies(
        &mut self,
        ui: &mut egui::Ui,
        quests: &Quests,
        progress: &Progress,
        index: usize,
        acts: &mut Vec<Act>,
    ) {
        let quest = &quests.quests[index];
        // Une quête finie n'attend plus rien (même terminée d'office, par `ctl`).
        if matches!(
            progress.status(quests, index),
            Status::Done | Status::Claimed
        ) {
            return;
        }
        let missing: Vec<usize> = quest
            .deps
            .iter()
            .copied()
            .filter(|&d| !matches!(progress.status(quests, d), Status::Done | Status::Claimed))
            .collect();
        if missing.is_empty() {
            return;
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(if quest.requires == Requires::One {
                "À finir d'abord (une seule suffit) :"
            } else {
                "À finir d'abord :"
            })
            .strong()
            .color(skin::GOLD),
        );
        for dep in missing {
            if !progress.visible(quests, dep) {
                ui.label(RichText::new("   Une quête encore cachée").color(skin::MUTED));
                continue;
            }
            let target = &quests.quests[dep];
            let mut text = target.title.clone();
            if target.chapter != quest.chapter {
                text.push_str(&format!("  ({})", quests.chapters[target.chapter].title));
            }
            let width = ui.available_width();
            if ui
                .add_sized([width, 26.0], egui::Button::new(text).wrap())
                .on_hover_text("Aller à cette quête")
                .clicked()
            {
                acts.push(Act::GoTo(dep));
            }
        }
    }

    fn sheet_tasks(
        &mut self,
        ui: &mut egui::Ui,
        quest: &Quest,
        index: usize,
        progress: &Progress,
        namer: &Namer,
        acts: &mut Vec<Act>,
    ) {
        ui.add_space(6.0);
        ui.label(RichText::new("Tâches").strong().color(skin::COPPER));
        for (t, task) in quest.tasks.iter().enumerate() {
            let have = progress.counter(&quest.id, t);
            let done = labels::task_done(task, have);
            ui.horizontal_top(|ui| {
                // L'icône : un clic ouvre les recettes de l'objet.
                let cell = match namer.task_icon(task) {
                    Some(item) => {
                        let response =
                            self.quest_cell(ui, item, 1, CELL, false, "Clic : voir ses recettes");
                        if response.clicked() {
                            acts.push(Act::Recipes(item));
                        }
                        response.rect
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::hover());
                        ui.painter().rect_filled(rect, 0, skin::INSET);
                        ui.painter().rect_stroke(
                            rect,
                            0,
                            Stroke::new(1.0, skin::BORDER),
                            StrokeKind::Inside,
                        );
                        // Pas d'objet à montrer : un anneau, plein quand c'est fait.
                        ui.painter().circle_stroke(
                            rect.center(),
                            CELL * 0.22,
                            Stroke::new(2.0, if done { skin::SAGE } else { skin::MUTED }),
                        );
                        rect
                    }
                };
                if done {
                    check_mark(ui.painter(), cell.right_bottom() - vec2(7.0, 7.0), 9.0);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let text = namer.task(task);
                    ui.label(if done {
                        RichText::new(text).color(skin::SAGE)
                    } else {
                        RichText::new(text)
                    });
                    let need = task.target();
                    if need > 1 {
                        // « 3 / 16 » et une barre.
                        let line = labels::progress_text(task, have);
                        ui.label(RichText::new(line).size(12.0).color(skin::MUTED));
                        let (bar, _) = ui.allocate_exact_size(
                            vec2(ui.available_width().min(180.0), 5.0),
                            Sense::hover(),
                        );
                        ui.painter().rect_filled(bar, 0, skin::INSET);
                        let mut fill = bar;
                        fill.set_width(bar.width() * (have.min(need) as f32 / need as f32));
                        ui.painter().rect_filled(
                            fill,
                            0,
                            if done { skin::SAGE } else { skin::COPPER },
                        );
                    }
                    match task {
                        Task::Check => {
                            let mut on = have >= 1;
                            if ui
                                .add_enabled(!done, egui::Checkbox::new(&mut on, "C'est fait"))
                                .clicked()
                                && on
                            {
                                acts.push(Act::Check(index, t));
                            }
                        }
                        Task::Item {
                            spec,
                            consume: true,
                            ..
                        } if !done => {
                            let carried: u32 = self
                                .server
                                .players
                                .me()
                                .inventory
                                .slots
                                .iter()
                                .flatten()
                                .filter(|s| spec.matches(s.item))
                                .map(|s| s.count)
                                .sum();
                            let button = ui.add_enabled(
                                carried > 0,
                                egui::Button::new(format!("Remettre ({carried} sur toi)")),
                            );
                            if button.clicked() {
                                acts.push(Act::HandIn(index, t));
                            }
                        }
                        _ => {}
                    }
                });
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sheet_rewards(
        &mut self,
        ui: &mut egui::Ui,
        quest: &Quest,
        index: usize,
        status: Status,
        progress: &Progress,
        namer: &Namer,
        acts: &mut Vec<Act>,
    ) {
        if quest.rewards.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Récompenses").strong().color(skin::COPPER));
        let claimed = status == Status::Claimed;
        for (r, reward) in quest.rewards.iter().enumerate() {
            match reward {
                Reward::Item(item, count) => {
                    ui.horizontal(|ui| {
                        self.quest_cell(ui, *item, *count, CELL, false, "");
                        ui.label(namer.reward(reward));
                    });
                }
                Reward::Choice(options) => {
                    ui.label(if claimed {
                        "Un objet au choix : récupéré"
                    } else {
                        "Un objet au choix :"
                    });
                    let chosen = progress.choice(&quest.id, r);
                    ui.horizontal_wrapped(|ui| {
                        for (o, (item, count)) in options.iter().enumerate() {
                            let hint = if claimed {
                                ""
                            } else {
                                "Clic : choisir cet objet"
                            };
                            let response =
                                self.quest_cell(ui, *item, *count, CELL, chosen == Some(o), hint);
                            if response.clicked() && !claimed {
                                acts.push(Act::Choose(index, r, o));
                            }
                        }
                    });
                    if let Some(o) = chosen {
                        ui.label(
                            RichText::new(format!("Choisi : {}", namer.item(options[o].0)))
                                .size(12.0)
                                .color(skin::SAGE),
                        );
                    }
                }
                Reward::Xp(_) | Reward::Loot(_) => {
                    ui.label(format!("• {}", namer.reward(reward)));
                }
            }
        }
    }

    fn sheet_buttons(
        &mut self,
        ui: &mut egui::Ui,
        quest: &Quest,
        index: usize,
        status: Status,
        progress: &Progress,
        acts: &mut Vec<Act>,
    ) {
        ui.add_space(8.0);
        let width = ui.available_width();
        if status == Status::Done {
            // Un choix manque-t-il ?
            let undecided = quest.rewards.iter().enumerate().any(|(r, reward)| {
                matches!(reward, Reward::Choice(_)) && progress.choice(&quest.id, r).is_none()
            });
            ui.add_enabled_ui(!undecided, |ui| {
                if skin::menu_button_sized(ui, "Récupérer", width) {
                    acts.push(Act::Claim(index));
                }
            });
            if undecided {
                ui.label(
                    RichText::new("Choisis d'abord un objet.")
                        .size(12.0)
                        .color(skin::GOLD),
                );
            }
        }
        let pinned = progress.pinned.contains(&quest.id);
        ui.horizontal_wrapped(|ui| {
            let half = ((width - 6.0) * 0.5).max(60.0);
            if skin::menu_button_sized(ui, if pinned { "Désépingler" } else { "Épingler" }, half)
            {
                acts.push(Act::Pin(index));
            }
            if let Some(guide) = &quest.guide
                && skin::menu_button_sized(ui, "Lire dans le guide", half)
            {
                acts.push(Act::Guide(guide.clone()));
            }
        });
        if !pinned {
            ui.label(
                RichText::new("Une quête épinglée reste affichée en jeu.")
                    .size(11.0)
                    .color(skin::MUTED),
            );
        }
    }
}
