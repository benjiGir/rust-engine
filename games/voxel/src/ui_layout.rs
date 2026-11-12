//! Tailles partagées : fenêtres bornées au viewport, cases et lignes adaptatives.
use engine_render::egui::{self, Vec2};

pub fn safe_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.content_rect().shrink(8.0)
}

pub fn window<'a>(ctx: &egui::Context, title: &'a str, width: f32) -> egui::Window<'a> {
    let safe = safe_rect(ctx);
    let size = safe.size().max(egui::vec2(80.0, 60.0));
    let width = width.min(size.x);
    egui::Window::new(title)
        .frame(crate::skin::panel())
        .fixed_pos(egui::pos2(safe.center().x - width * 0.5, safe.top()))
        .constrain_to(safe)
        .auto_sized()
        .min_size(Vec2::ZERO)
        .min_width(width)
        .default_width(width)
        .max_size(size)
        .resizable(false)
        .collapsible(false)
        .scroll([false, true])
}

/// Neuf cases restent visibles, y compris quand la fenêtre est étroite.
pub fn slot_size(ctx: &egui::Context) -> f32 {
    ((safe_rect(ctx).width() - 64.0 - 8.0 * 4.0) / 9.0).clamp(10.0, 36.0)
}

pub fn columns(ui: &egui::Ui, cell: f32) -> usize {
    ((ui.available_width() + ui.spacing().item_spacing.x) / (cell + ui.spacing().item_spacing.x))
        .floor()
        .max(1.0) as usize
}

pub fn inventory_width(ctx: &egui::Context) -> f32 {
    (9.0 * slot_size(ctx) + 8.0 * 2.0 + 56.0).min(safe_rect(ctx).width())
}

pub fn inventory_window<'a>(ctx: &egui::Context, title: &'a str, id: egui::Id) -> egui::Window<'a> {
    let safe = safe_rect(ctx);
    let width = inventory_width(ctx);
    let height = ctx
        .memory(|m| m.area_rect(id))
        .map_or(400.0, |r| r.height())
        .min(safe.height());
    let y = (safe.center().y - height * 0.5).max(safe.top());
    window(ctx, title, width)
        .id(id)
        .fixed_pos(egui::pos2(safe.center().x - width * 0.5, y))
        .max_height(safe.bottom() - y)
        .frame(crate::skin::inventory_panel())
        .title_bar(false)
}

pub enum InventorySection {
    Armor,
    Craft,
    Recipes,
}

/// Même composition pour l'inventaire en jeu et sa planche de contrôle.
pub fn inventory_sections(ui: &mut egui::Ui, mut add: impl FnMut(&mut egui::Ui, InventorySection)) {
    let width = ui.available_width();
    if width < 200.0 {
        add(ui, InventorySection::Armor);
        ui.separator();
        add(ui, InventorySection::Craft);
        ui.collapsing("Recettes", |ui| add(ui, InventorySection::Recipes));
    } else {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width((width * 0.37).min(118.0));
                add(ui, InventorySection::Armor);
            });
            ui.vertical(|ui| {
                ui.label("Artisanat");
                add(ui, InventorySection::Craft);
                ui.add_space(6.0);
                ui.collapsing("Recettes", |ui| add(ui, InventorySection::Recipes));
            });
        });
    }
}

pub fn menu(ctx: &egui::Context, title: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let safe = safe_rect(ctx);
    let main = title == "Voxel";
    let p = ctx.layer_painter(egui::LayerId::background());
    p.rect_filled(
        ctx.content_rect(),
        0,
        egui::Color32::from_black_alpha(if main { 65 } else { 155 }),
    );
    let width = width.min(safe.width());
    let y = if main {
        (safe.center().y - 175.0).max(safe.top())
    } else {
        safe.top() + 12.0
    };
    window(ctx, title, width)
        .frame(egui::Frame::NONE.inner_margin(8))
        .fixed_pos(egui::pos2(safe.center().x - width * 0.5, y))
        .fixed_size(egui::vec2(width, safe.bottom() - y))
        .vscroll(title != "Options")
        .title_bar(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if main {
                    crate::skin::logo(ui);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("TERRES SAUVAGES · ATELIERS · AVENTURES")
                            .size(10.0)
                            .color(crate::skin::TEXT),
                    );
                    ui.add_space(if safe.height() > 400.0 {
                        42.0
                    } else if safe.height() < 300.0 {
                        8.0
                    } else {
                        20.0
                    });
                } else {
                    ui.label(
                        egui::RichText::new(title)
                            .monospace()
                            .size(22.0)
                            .strong()
                            .color(crate::skin::TEXT),
                    );
                    ui.add_space(if safe.height() < 300.0 { 6.0 } else { 12.0 });
                }
                add(ui);
                if !main && safe.height() > 400.0 {
                    ui.add_space(8.0);
                }
            });
        });
}

/// Deux actions secondaires sous les grandes actions du menu.
pub fn paired_buttons(ui: &mut egui::Ui, left: &str, right: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        let width = ui.available_width().min(320.0);
        ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
        let half = (width - ui.spacing().item_spacing.x) * 0.5;
        let a = crate::skin::menu_button_sized(ui, left, half);
        let b = crate::skin::menu_button_sized(ui, right, half);
        (a, b)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_actions_stay_visible_without_scroll_on_small_windows() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        for size in [
            egui::vec2(1280.0, 720.0),
            egui::vec2(640.0, 360.0),
            egui::vec2(320.0, 240.0),
            egui::vec2(1280.0, 720.0),
        ] {
            for frame in 0..5 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        menu(ui.ctx(), "Voxel", 500.0, |ui| {
                            crate::skin::menu_button(ui, "Jouer");
                            crate::skin::menu_button(ui, "Nouveau monde");
                            ui.add_space(10.0);
                            paired_buttons(ui, "Options…", "Quitter");
                        })
                    },
                );
                if frame > 0 {
                    for label in ["Jouer", "Nouveau monde", "Options…", "Quitter"] {
                        assert!(
                            output.shapes.iter().any(|shape| match &shape.shape {
                                egui::Shape::Text(t) =>
                                    t.galley.text() == label
                                        && shape.clip_rect.contains_rect(t.visual_bounding_rect()),
                                _ => false,
                            }),
                            "{label} hors de l’écran {size:?}"
                        );
                    }
                }
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn inventory_window_keeps_its_bounds_when_resized() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        for size in [
            egui::vec2(1280.0, 720.0),
            egui::vec2(320.0, 240.0),
            egui::vec2(640.0, 360.0),
            egui::vec2(213.0, 160.0),
        ] {
            for _ in 0..5 {
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        rect = inventory_window(
                            ui.ctx(),
                            "Inventaire",
                            egui::Id::unique("test_inventory"),
                        )
                        .show(ui.ctx(), |ui| {
                            crate::skin::inventory_style(ui);
                            ui.label("Inventaire");
                            inventory_sections(ui, |ui, section| match section {
                                InventorySection::Armor => {
                                    ui.label("Armure");
                                }
                                InventorySection::Craft => {
                                    ui.horizontal(|ui| {
                                        for _ in 0..3 {
                                            ui.allocate_exact_size(
                                                egui::Vec2::splat(slot_size(ui.ctx())),
                                                egui::Sense::hover(),
                                            );
                                        }
                                    });
                                }
                                InventorySection::Recipes => {
                                    ui.label("Aucune recette");
                                }
                            });
                            for _ in 0..4 {
                                ui.horizontal(|ui| {
                                    for _ in 0..9 {
                                        ui.allocate_exact_size(
                                            egui::Vec2::splat(slot_size(ui.ctx())),
                                            egui::Sense::hover(),
                                        );
                                    }
                                });
                            }
                        })
                        .unwrap()
                        .response
                        .rect;
                    },
                );
                assert!(
                    rect.min.x >= -1.0
                        && rect.min.y >= -1.0
                        && rect.max.x <= size.x + 1.0
                        && rect.max.y <= size.y + 1.0,
                    "{size:?} : {rect:?}"
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn all_nine_slots_fit_even_on_a_small_viewport_at_large_ui_scale() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        for [w, h] in [
            [1280.0, 720.0],
            [640.0, 360.0],
            [320.0, 240.0],
            [320.0 / 1.5, 240.0 / 1.5],
        ] {
            for _ in 0..4 {
                let mut rects = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w, h),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        window(ui.ctx(), "Inventaire", inventory_width(ui.ctx())).show(
                            ui.ctx(),
                            |ui| {
                                ui.horizontal(|ui| {
                                    for _ in 0..9 {
                                        let size = slot_size(ui.ctx());
                                        rects.push(
                                            ui.allocate_exact_size(
                                                egui::vec2(size, size),
                                                egui::Sense::hover(),
                                            )
                                            .0,
                                        );
                                    }
                                });
                            },
                        );
                    },
                );
                output.textures_delta.clear();
                assert_eq!(rects.len(), 9);
                assert!(
                    rects.iter().all(|r| r.min.x >= 0.0 && r.max.x <= w),
                    "{w}x{h}: {rects:?}"
                );
            }
        }
    }

    #[test]
    fn scrolling_a_short_options_window_reaches_the_return_button() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let mut s = crate::settings::Settings::default();
        let mut waiting = None;
        let mut visible = false;
        for frame in 0..40 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(180.0, 100.0))];
            if frame > 2 {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    delta: egui::vec2(0.0, -200.0),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(frame as f64 * 0.05),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 240.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    menu(ui.ctx(), "Options", 560.0, |ui| {
                        crate::ui_options::panel(ui, &mut s, &mut waiting);
                    });
                },
            );
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == "Retour"
                    && shape.clip_rect.contains_rect(text.visual_bounding_rect())
                {
                    visible = true;
                }
            }
            output.textures_delta.clear();
        }
        assert!(
            visible,
            "la navigation doit rester accessible par défilement"
        );
    }
    #[test]
    fn options_window_stays_inside_the_viewport_including_after_resizing() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let mut settings = crate::settings::Settings::default();
        let mut waiting = None;
        for [w, h] in [
            [1280.0, 720.0],
            [640.0, 360.0],
            [480.0, 320.0],
            [320.0, 240.0],
            [1920.0, 1080.0],
            [400.0, 700.0],
        ] {
            for _ in 0..5 {
                let mut rect = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w, h),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let response = window(ui.ctx(), "Options", 560.0)
                            .show(ui.ctx(), |ui| {
                                crate::ui_options::panel(ui, &mut settings, &mut waiting);
                            })
                            .unwrap();
                        rect = response.response.rect;
                    },
                );
                output.textures_delta.clear();
                assert!(
                    rect.min.x >= 0.0
                        && rect.min.y >= 0.0
                        && rect.max.x <= w + 1.0
                        && rect.max.y <= h + 1.0,
                    "viewport {w}x{h} : {rect:?}"
                );
            }
        }
    }
}
