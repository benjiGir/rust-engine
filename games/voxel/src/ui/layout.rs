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
        .frame(crate::ui::skin::panel())
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

pub const SLOT_GAP: f32 = 2.0;
const SECTION_GAP: f32 = 12.0;

fn inventory_chrome(ctx: &egui::Context) -> f32 {
    crate::ui::skin::inventory_panel().total_margin().sum().x
        + ctx.global_style().spacing.scroll.allocated_width()
}

/// Neuf cases restent visibles, avec la même réserve pour le cadre et le défilement.
pub fn slot_size(ctx: &egui::Context) -> f32 {
    ((safe_rect(ctx).width() - inventory_chrome(ctx) - 8.0 * SLOT_GAP) / 9.0)
        .floor()
        .clamp(10.0, 48.0)
}

pub fn grid_width(ctx: &egui::Context, columns: usize) -> f32 {
    columns as f32 * slot_size(ctx) + columns.saturating_sub(1) as f32 * SLOT_GAP
}

pub fn slot_columns(ui: &egui::Ui) -> usize {
    ((ui.available_width() + SLOT_GAP) / (slot_size(ui.ctx()) + SLOT_GAP))
        .floor()
        .max(1.0) as usize
}

pub fn inventory_width(ctx: &egui::Context) -> f32 {
    (grid_width(ctx, 9) + inventory_chrome(ctx)).min(safe_rect(ctx).width())
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
        .max_width(width)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .frame(crate::ui::skin::inventory_panel())
        .title_bar(false)
}

/// Une colonne réserve réellement sa largeur : le contenu suivant ne dépend pas du libellé.
pub fn column<R>(
    ui: &mut egui::Ui,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            add(ui)
        },
    )
}

pub fn inventory_body(ui: &mut egui::Ui) {
    crate::ui::skin::inventory_style(ui);
    ui.set_width(grid_width(ui.ctx(), 9));
}

/// L'en-tête partage exactement les bords de la grille située en dessous.
pub fn inventory_header(ui: &mut egui::Ui, title: &str) -> bool {
    let text_width = (ui.available_width() - 36.0 - ui.spacing().item_spacing.x).max(1.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(text_width);
                ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate())
                    .on_hover_text(title);
            },
        );
        ui.add_sized([36.0, 24.0], egui::Button::new("×").small())
            .on_hover_text("Fermer · Échap")
            .clicked()
    })
    .inner
}

/// Même pas et même origine pour toutes les grilles, y compris les dernières lignes incomplètes.
pub fn slots(
    ui: &mut egui::Ui,
    count: usize,
    columns: usize,
    mut add: impl FnMut(&mut egui::Ui, usize),
) -> egui::Response {
    let columns = columns.max(1).min(count.max(1));
    column(ui, grid_width(ui.ctx(), columns), |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::splat(SLOT_GAP);
        ui.spacing_mut().interact_size.y = slot_size(ui.ctx());
        for row in (0..count).step_by(columns) {
            ui.horizontal(|ui| {
                for index in row..(row + columns).min(count) {
                    add(ui, index);
                }
            });
        }
    })
    .response
}

pub fn player_inventory(ui: &mut egui::Ui, mut add: impl FnMut(&mut egui::Ui, usize)) {
    slots(ui, 27, 9, |ui, i| add(ui, i + 9));
    ui.add_space(8.0);
    slots(ui, 9, 9, add);
}

/// La flèche et le résultat restent centrés sur la hauteur de la grille.
pub fn crafting(ui: &mut egui::Ui, size: usize, mut add: impl FnMut(&mut egui::Ui, Option<usize>)) {
    let cell = slot_size(ui.ctx());
    let height = size as f32 * cell + size.saturating_sub(1) as f32 * SLOT_GAP;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        slots(ui, size * size, size, |ui, i| add(ui, Some(i)));
        column(ui, 24.0, |ui| {
            ui.add_space((height - 24.0) * 0.5);
            ui.add_sized(
                [24.0, 24.0],
                egui::Label::new(egui::RichText::new("→").size(24.0)),
            );
        });
        column(ui, cell, |ui| {
            ui.add_space((height - cell) * 0.5);
            add(ui, None);
        });
    });
}

fn machine_label(ui: &mut egui::Ui, text: &str, width: f32) {
    ui.add_sized(
        [width, 20.0],
        egui::Label::new(egui::RichText::new(text).size(11.0)).truncate(),
    )
    .on_hover_text(text);
}

pub fn furnace(ui: &mut egui::Ui, burn: f32, cook: f32, mut add: impl FnMut(&mut egui::Ui, usize)) {
    use engine_voxel::{FURNACE_FUEL, FURNACE_INPUT, FURNACE_OUTPUT};
    let cell = slot_size(ui.ctx());
    let available = ui.available_width();
    let gap = (cell * 0.5).min(24.0);
    let side = ((available - 2.0 * gap - 28.0) * 0.5).min(80.0).max(cell);
    let width = (available - 2.0 * side - 2.0 * gap).clamp(28.0, 112.0);
    let total = 2.0 * side + 2.0 * gap + width;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let inset = ((available - total) * 0.5).max(0.0);
        if inset > 0.0 {
            ui.add_space(inset);
        }
        column(ui, side, |ui| {
            ui.vertical_centered(|ui| {
                machine_label(ui, "Entrée", side);
                add(ui, FURNACE_INPUT);
                ui.add(
                    egui::ProgressBar::new(burn)
                        .desired_width(cell)
                        .desired_height(8.0)
                        .corner_radius(0)
                        .fill(crate::ui::skin::COPPER),
                );
                add(ui, FURNACE_FUEL);
                machine_label(ui, "Combustible", side);
            });
        });
        column(ui, width, |ui| {
            ui.add_space(20.0 + (cell - 16.0) * 0.5 + ui.spacing().item_spacing.y);
            ui.add(
                egui::ProgressBar::new(cook)
                    .desired_width(width)
                    .desired_height(16.0)
                    .corner_radius(0)
                    .show_percentage(),
            );
        });
        column(ui, side, |ui| {
            ui.vertical_centered(|ui| {
                machine_label(ui, "Sortie", side);
                add(ui, FURNACE_OUTPUT);
            });
        });
    });
}

pub fn brewing(ui: &mut egui::Ui, progress: f32, mut add: impl FnMut(&mut egui::Ui, usize)) {
    use engine_voxel::{BREW_INPUT, BREW_OUTPUT, BREW_REAGENT};
    let cell = slot_size(ui.ctx());
    let available = ui.available_width();
    let gap = (cell * 0.25).min(12.0);
    let side = ((available - 3.0 * gap - 24.0) / 3.0).min(80.0).max(cell);
    let width = (available - 3.0 * side - 3.0 * gap).clamp(24.0, 100.0);
    let total = 3.0 * side + 3.0 * gap + width;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let inset = ((available - total) * 0.5).max(0.0);
        if inset > 0.0 {
            ui.add_space(inset);
        }
        for (index, text) in [
            (BREW_INPUT, "Eau"),
            (BREW_REAGENT, "Ingrédient"),
            (BREW_OUTPUT, "Potion"),
        ] {
            if index == BREW_OUTPUT {
                column(ui, width, |ui| {
                    ui.add_space(20.0 + (cell - 14.0) * 0.5 + ui.spacing().item_spacing.y);
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(width)
                            .desired_height(14.0)
                            .corner_radius(0)
                            .fill(crate::ui::skin::MAGIC),
                    );
                });
            }
            column(ui, side, |ui| {
                ui.vertical_centered(|ui| {
                    machine_label(ui, text, side);
                    add(ui, index);
                });
            });
        }
    });
}

pub enum InventorySection {
    Armor,
    Craft,
    Recipes,
}

/// Même composition pour l'inventaire en jeu et sa planche de contrôle.
pub fn inventory_sections(ui: &mut egui::Ui, mut add: impl FnMut(&mut egui::Ui, InventorySection)) {
    let width = ui.available_width();
    let armor_width = slot_size(ui.ctx()) + 82.0;
    let craft_width = grid_width(ui.ctx(), 3) + 44.0;
    if width < armor_width + SECTION_GAP + craft_width {
        add(ui, InventorySection::Armor);
        ui.separator();
        add(ui, InventorySection::Craft);
        ui.collapsing("Recettes", |ui| add(ui, InventorySection::Recipes));
    } else {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = SECTION_GAP;
            column(ui, armor_width, |ui| {
                add(ui, InventorySection::Armor);
            });
            column(ui, width - armor_width - SECTION_GAP, |ui| {
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
        .vscroll(!matches!(
            title,
            "Options" | "Packs de textures" | "Packs de shaders"
        ))
        .title_bar(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if main {
                    if safe.height() < 300.0 {
                        ui.allocate_ui_with_layout(
                            egui::vec2(128.0, 32.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                ui.set_width(128.0);
                                crate::ui::skin::logo(ui);
                            },
                        );
                        ui.add_space(4.0);
                    } else {
                        crate::ui::skin::logo(ui);
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("TERRES SAUVAGES · ATELIERS · AVENTURES")
                                .size(10.0)
                                .color(crate::ui::skin::TEXT),
                        );
                        ui.add_space(if safe.height() > 400.0 { 42.0 } else { 20.0 });
                    }
                } else {
                    ui.label(
                        egui::RichText::new(title)
                            .monospace()
                            .size(22.0)
                            .strong()
                            .color(crate::ui::skin::TEXT),
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
        let a = crate::ui::skin::menu_button_sized(ui, left, half);
        let b = crate::ui::skin::menu_button_sized(ui, right, half);
        (a, b)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_machine_title_keeps_the_close_button_visible_and_clickable() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        let size = egui::vec2(320.0, 240.0);
        let mut button = None;
        let mut clicked = false;
        for frame in 0..8 {
            let mut events = Vec::new();
            if frame == 4 || frame == 5 {
                let pos = button.expect("bouton de fermeture visible");
                events.push(egui::Event::PointerMoved(pos));
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 4,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events, ..Default::default()
            }, |ui| {
                inventory_window(ui.ctx(), "Machine", egui::Id::unique("test_long_header")).show(ui.ctx(), |ui| {
                    inventory_body(ui);
                    clicked |= inventory_header(ui, "Très longue description d'une machine de mod : traitement des matériaux");
                });
            });
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == "×"
                {
                    let rect = text.visual_bounding_rect();
                    if shape.clip_rect.contains_rect(rect) {
                        button = Some(rect.center());
                    }
                }
            }
            output.textures_delta.clear();
        }
        assert!(
            clicked,
            "le nom de la machine ne doit pas repousser la fermeture"
        );
    }

    #[test]
    fn container_and_player_columns_align_and_icons_fill_the_slots_after_resizing() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        for size in [
            egui::vec2(1280.0, 720.0),
            egui::vec2(320.0, 240.0),
            egui::vec2(640.0, 360.0),
            egui::vec2(213.0, 160.0),
        ] {
            for _ in 0..5 {
                let mut chest = Vec::new();
                let mut player = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        inventory_window(ui.ctx(), "Coffre", egui::Id::unique("test_columns"))
                            .show(ui.ctx(), |ui| {
                                inventory_body(ui);
                                let cell = slot_size(ui.ctx());
                                slots(ui, 27, 9, |ui, _| {
                                    chest.push(
                                        ui.allocate_exact_size(
                                            Vec2::splat(cell),
                                            egui::Sense::click(),
                                        )
                                        .0,
                                    );
                                });
                                ui.separator();
                                player_inventory(ui, |ui, index| {
                                    player.push((
                                        index,
                                        ui.allocate_exact_size(
                                            Vec2::splat(cell),
                                            egui::Sense::click(),
                                        )
                                        .0,
                                    ));
                                });
                            });
                    },
                );
                output.textures_delta.clear();
                assert_eq!((chest.len(), player.len()), (27, 36));
                for (index, rect) in &player {
                    assert!((rect.left() - chest[index % 9].left()).abs() < 0.1);
                    assert!(rect.left() >= 0.0 && rect.right() <= size.x);
                    let icon = crate::ui::skin::icon_rect(*rect);
                    assert!(
                        icon.width() >= rect.width() * 0.88,
                        "l'icône doit remplir la case : {rect:?} / {icon:?}"
                    );
                }
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn furnace_brewing_and_crafting_keep_their_slots_aligned() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        for size in [egui::vec2(1280.0, 720.0), egui::vec2(320.0, 240.0)] {
            for kind in 0..3 {
                for _ in 0..5 {
                    let mut rects = Vec::new();
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            inventory_window(
                                ui.ctx(),
                                "Machine",
                                egui::Id::unique(("test_machine", kind)),
                            )
                            .show(ui.ctx(), |ui| {
                                inventory_body(ui);
                                let mut slot = |ui: &mut egui::Ui, _| {
                                    let cell = slot_size(ui.ctx());
                                    rects.push(
                                        ui.allocate_exact_size(
                                            Vec2::splat(cell),
                                            egui::Sense::click(),
                                        )
                                        .0,
                                    );
                                };
                                match kind {
                                    0 => furnace(ui, 0.5, 0.5, &mut slot),
                                    1 => brewing(ui, 0.5, &mut slot),
                                    _ => crafting(ui, 3, |ui, _| slot(ui, 0)),
                                }
                            });
                        },
                    );
                    output.textures_delta.clear();
                    assert!(
                        rects.iter().all(|r| r.left() >= 0.0 && r.right() <= size.x),
                        "{size:?}: {rects:?}"
                    );
                    match kind {
                        0 => {
                            assert!((rects[0].top() - rects[2].top()).abs() < 0.1);
                            assert_eq!(rects[0].left(), rects[1].left());
                        }
                        1 => assert!(rects.iter().all(|r| (r.top() - rects[0].top()).abs() < 0.1)),
                        _ => assert!((rects[9].center().y - rects[4].center().y).abs() < 0.1),
                    }
                    output.textures_delta.clear();
                }
            }
        }
    }
    #[test]
    fn title_actions_stay_visible_without_scroll_on_small_windows() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
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
                            crate::ui::skin::menu_button(ui, "Jouer");
                            crate::ui::skin::menu_button(ui, "Nouveau monde");
                            crate::ui::skin::menu_button(ui, "Packs de textures…");
                            ui.add_space(10.0);
                            paired_buttons(ui, "Options…", "Quitter");
                        })
                    },
                );
                if frame > 0 {
                    for label in [
                        "Jouer",
                        "Nouveau monde",
                        "Packs de textures…",
                        "Options…",
                        "Quitter",
                    ] {
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
        crate::ui::skin::apply(&ctx);
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
                            crate::ui::skin::inventory_style(ui);
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
        crate::ui::skin::apply(&ctx);
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
    fn display_options_open_the_shader_selector() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        let mut settings = voxel_sim::settings::Settings::default();
        let mut target = None;
        let mut opened = false;
        for frame in 0..6 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                ..Default::default()
            };
            if (3..=4).contains(&frame) {
                let pos = target.expect("bouton des shaders visible");
                input.events.push(egui::Event::PointerMoved(pos));
                input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 3,
                    modifiers: Default::default(),
                });
            }
            let mut output = ctx.run_ui(input, |ui| {
                menu(ui.ctx(), "Options", 640., |ui| {
                    opened |= crate::ui::options::panel(ui, &mut settings, &mut None)
                        == crate::ui::options::Navigation::Shaders;
                });
            });
            output.textures_delta.clear();
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape
                    && text.galley.text() == "Packs de shaders…"
                    && shape.clip_rect.contains_rect(text.visual_bounding_rect())
                {
                    target = Some(text.visual_bounding_rect().center());
                }
            }
        }
        assert!(opened);
    }
    #[test]
    fn scrolling_a_short_options_window_reaches_the_return_button() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        let mut s = voxel_sim::settings::Settings::default();
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
                        crate::ui::options::panel(ui, &mut s, &mut waiting);
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
        crate::ui::skin::apply(&ctx);
        let mut settings = voxel_sim::settings::Settings::default();
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
                                crate::ui::options::panel(ui, &mut settings, &mut waiting);
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
