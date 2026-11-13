//! Options regroupées par usage ; les contrôles s'empilent sur les fenêtres étroites.
use crate::settings::{Action, Settings};
use engine_render::egui::{self, RichText};

fn row(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    ui.push_id(label, |ui| {
        if width >= 420.0 {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width * 0.48, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(width * 0.48);
                        ui.label(label);
                    },
                );
                let remaining = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(remaining, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(remaining);
                        control(ui);
                    },
                );
            });
        } else {
            ui.label(label);
            control(ui);
        }
    });
}

/// Corps défilant et bouton Retour toujours hors de la zone défilante.
pub fn panel(ui: &mut egui::Ui, s: &mut Settings, waiting: &mut Option<Action>) -> bool {
    let tab = categories(ui);
    egui::ScrollArea::vertical()
        .id_salt("options_body")
        .max_height((ui.ctx().content_rect().height() - 210.0).max(30.0))
        .min_scrolled_height(0.0)
        .show(ui, |ui| controls(ui, s, waiting, tab));
    ui.add_space(4.0);
    crate::skin::menu_button(ui, "Retour")
}

fn categories(ui: &mut egui::Ui) -> usize {
    let id = ui.scope_id().with("options_category");
    let mut tab = ui.ctx().data_mut(|d| d.get_temp::<usize>(id).unwrap_or(0));
    ui.horizontal_wrapped(|ui| {
        let width = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
        for (i, label) in ["Affichage", "Audio", "Commandes"].into_iter().enumerate() {
            ui.vertical(|ui| {
                ui.set_width(width);
                if crate::skin::menu_button_sized(ui, label, width) {
                    tab = i;
                }
                let (r, _) = ui.allocate_exact_size(egui::vec2(width, 2.0), egui::Sense::hover());
                if tab == i {
                    ui.painter().rect_filled(r, 0, crate::skin::TEXT);
                }
            });
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, tab));
    ui.add_space(6.0);
    tab
}

fn controls(ui: &mut egui::Ui, s: &mut Settings, waiting: &mut Option<Action>, tab: usize) {
    ui.spacing_mut().slider_width = (ui.available_width() * 0.45 - 50.0).clamp(40.0, 140.0);
    match tab {
        0 => {
            row(ui, "Distance de rendu (chunks)", |ui| {
                ui.add(egui::Slider::new(&mut s.render_distance, 4..=32))
                    .on_hover_text("Appliquée au prochain monde chargé");
            });
            row(ui, "Terrain lointain (0 : aucun)", |ui| {
                ui.add(egui::Slider::new(&mut s.far_distance, 0..=192))
                    .on_hover_text("Appliqué au prochain monde chargé");
            });
            row(ui, "Champ de vision (°)", |ui| {
                ui.add(egui::Slider::new(&mut s.fov, 50.0..=110.0));
            });
            row(ui, "Échelle de l’interface", |ui| {
                ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.5).step_by(0.05));
            });
            ui.checkbox(&mut s.vsync, "Synchronisation verticale");
            ui.checkbox(
                &mut s.target_info,
                "Identifier le bloc ou la créature visée",
            );
            ui.checkbox(&mut s.show_arm, "Afficher le bras");
        }
        1 => {
            row(ui, "Volume général", |ui| {
                ui.add(egui::Slider::new(&mut s.volume_master, 0.0..=1.0));
            });
            row(ui, "Musique et ambiance", |ui| {
                ui.add(egui::Slider::new(&mut s.volume_music, 0.0..=1.0));
            });
            row(ui, "Effets", |ui| {
                ui.add(egui::Slider::new(&mut s.volume_effects, 0.0..=1.0));
            });
        }
        _ => {
            row(ui, "Sensibilité de la souris", |ui| {
                ui.add(egui::Slider::new(&mut s.mouse_sensitivity, 0.2..=3.0));
            });
            ui.label(RichText::new("Touches").strong());
            for action in Action::ALL {
                let text = if *waiting == Some(action) {
                    "appuie sur une touche…".to_string()
                } else {
                    s.keys.name(action).to_string()
                };
                row(ui, action.label(), |ui| {
                    if ui
                        .add_sized(
                            [ui.available_width().min(180.0), 24.0],
                            egui::Button::new(text).wrap(),
                        )
                        .clicked()
                    {
                        *waiting = Some(action);
                    }
                });
            }
            if ui.button("Touches par défaut").clicked() {
                s.keys = Default::default();
            }
        }
    }
    ui.add_space(12.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_share_a_column_regardless_of_label_length() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        for width in [600.0, 300.0] {
            let mut controls = Vec::new();
            let mut value = 0.5;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    for label in [
                        "Effets",
                        "Musique et ambiance",
                        "Distance de rendu (chunks)",
                    ] {
                        row(ui, label, |ui| {
                            controls.push(ui.add(egui::Slider::new(&mut value, 0.0..=1.0)).rect);
                        });
                    }
                },
            );
            assert!(
                controls
                    .iter()
                    .all(|r| (r.left() - controls[0].left()).abs() < 0.1),
                "{width}: {controls:?}"
            );
            output.textures_delta.clear();
        }
    }

    #[test]
    fn complete_options_page_keeps_its_header_and_footer_inside_small_windows() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let mut settings = Settings::default();
        let mut waiting = None;
        for (viewport, size) in [
            egui::vec2(320.0, 240.0),
            egui::vec2(640.0, 360.0),
            egui::vec2(320.0, 240.0),
        ]
        .into_iter()
        .enumerate()
        {
            for frame in 0..20 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some((viewport * 20 + frame) as f64 * 0.1),
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui_layout::menu(ui.ctx(), "Options", 640.0, |ui| {
                            panel(ui, &mut settings, &mut waiting);
                        })
                    },
                );
                output.textures_delta.clear();
                if frame > 1 {
                    for name in ["Options", "Retour"] {
                        let visible = output.shapes.iter().any(|s| match &s.shape {
                            egui::Shape::Text(t) => {
                                t.galley.text() == name
                                    && s.clip_rect.contains_rect(t.visual_bounding_rect())
                                    && egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                                        .contains_rect(t.visual_bounding_rect())
                            }
                            _ => false,
                        });
                        assert!(visible, "{name} dépasse le viewport {size:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn categories_can_be_changed_by_click_and_return_stays_visible() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let mut settings = Settings::default();
        let mut waiting = None;
        let mut events = Vec::new();
        let mut audio_pos = None;
        let mut saw_volume = false;
        for frame in 0..8 {
            if frame == 3 || frame == 4 {
                let pos = audio_pos.expect("onglet Audio visible");
                events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 3,
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 360.0),
                    )),
                    time: Some(frame as f64 * 0.1),
                    events: std::mem::take(&mut events),
                    ..Default::default()
                },
                |ui| {
                    crate::ui_layout::menu(ui.ctx(), "Options", 640.0, |ui| {
                        panel(ui, &mut settings, &mut waiting);
                    })
                },
            );
            let mut back_visible = false;
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    let bounds = text.visual_bounding_rect();
                    match text.galley.text() {
                        "Audio" => audio_pos = Some(bounds.center()),
                        "Volume général" => saw_volume = true,
                        "Retour" => back_visible = shape.clip_rect.contains_rect(bounds),
                        _ => {}
                    }
                }
            }
            if frame > 1 {
                assert!(back_visible, "Retour doit rester accessible sans défiler");
            }
            output.textures_delta.clear();
        }
        assert!(saw_volume, "le clic doit ouvrir les réglages audio");
    }
}
