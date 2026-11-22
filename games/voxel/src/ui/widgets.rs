//! Dessin des menus déclarés par les mods, partagé avec la planche de contrôle.
use engine_mods::Widget;
use engine_mods::menu::{UI_BUTTON, UI_SLIDER, UI_TOGGLE};
use engine_render::egui;

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
                ui.label(text.as_str());
                ui.add(
                    egui::ProgressBar::new(*value)
                        .corner_radius(0)
                        .desired_height(14.0)
                        .desired_width(ui.available_width().min(200.0)),
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
                let columns = columns.min(crate::ui::layout::slot_columns(ui)).max(1);
                crate::ui::layout::column(
                    ui,
                    crate::ui::layout::grid_width(ui.ctx(), columns.min(count.max(1))),
                    |ui| {
                        slots(ui, first, count, columns);
                    },
                );
            }
            Widget::RowBegin => {
                // Jusqu'à la fin de la ligne (l'analyse du menu garantit qu'il y en a une).
                let end = widgets[i + 1..]
                    .iter()
                    .position(|w| *w == Widget::RowEnd)
                    .map_or(range.end, |p| i + 1 + p);
                draw_row(ui, widgets, i + 1..end, events, slots);
                i = end;
            }
            Widget::RowEnd => {}
        }
        i += 1;
    }
}

/// Les grilles gardent leur largeur exacte ; les jauges prennent l'espace restant.
/// Une ligne de boutons peut se replier sans repousser les cases hors du panneau.
fn draw_row(
    ui: &mut egui::Ui,
    widgets: &mut [Widget],
    range: std::ops::Range<usize>,
    events: &mut Vec<(i32, i32, i32)>,
    slots: &mut dyn FnMut(&mut egui::Ui, usize, usize, usize),
) {
    let available = ui.available_width();
    let gap = 8.0;
    let mut widths: Vec<_> = range
        .clone()
        .map(|i| {
            match &widgets[i] {
                Widget::Slots { count, columns, .. } => {
                    crate::ui::layout::grid_width(ui.ctx(), (*columns).min(*count).max(1))
                }
                Widget::Button { text, .. } | Widget::Toggle { text, .. } => {
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    ui.painter()
                        .layout_no_wrap(text.clone(), font, crate::ui::skin::TEXT)
                        .size()
                        .x
                        + 28.0
                }
                _ => 180.0,
            }
            .min(available)
        })
        .collect();
    let bars = range
        .clone()
        .filter(|i| matches!(widgets[*i], Widget::Bar { .. }))
        .count();
    if bars > 0 {
        let fixed: f32 = range
            .clone()
            .zip(&widths)
            .filter(|(i, _)| !matches!(widgets[*i], Widget::Bar { .. }))
            .map(|(_, width)| width)
            .sum();
        let bar_width = ((available - fixed - gap * range.len().saturating_sub(1) as f32)
            / bars as f32)
            .clamp(32.0, 200.0);
        for (i, width) in range.clone().zip(&mut widths) {
            if matches!(widgets[i], Widget::Bar { .. }) {
                *width = bar_width;
            }
        }
    }
    // Le statut utilise toute la ligne : il ne se découpe pas entre deux petites grilles.
    for i in range.clone() {
        if let Widget::Bar { text, .. } = &widgets[i] {
            ui.label(text);
        }
    }
    let heights: Vec<_> = range
        .clone()
        .zip(&widths)
        .map(|(i, width)| match &widgets[i] {
            Widget::Slots { count, columns, .. } => {
                let cell = crate::ui::layout::slot_size(ui.ctx());
                let columns = (*columns)
                    .min(
                        ((*width + crate::ui::layout::SLOT_GAP)
                            / (cell + crate::ui::layout::SLOT_GAP))
                            .floor()
                            .max(1.0) as usize,
                    )
                    .max(1);
                let rows = count.div_ceil(columns);
                rows as f32 * cell + rows.saturating_sub(1) as f32 * crate::ui::layout::SLOT_GAP
            }
            Widget::Bar { .. } => 14.0,
            _ => 30.0,
        })
        .collect();
    let height = heights.iter().copied().fold(0.0, f32::max);
    ui.allocate_ui_with_layout(
        egui::vec2(available, height),
        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for ((i, width), height) in range.zip(widths).zip(heights) {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        if let Widget::Bar { value, .. } = &widgets[i] {
                            ui.add(
                                egui::ProgressBar::new(*value)
                                    .corner_radius(0)
                                    .desired_height(14.0)
                                    .desired_width(width),
                            );
                        } else {
                            draw_range(ui, widgets, i..i + 1, events, slots);
                        }
                    },
                );
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_rows_keep_slot_groups_on_the_same_line_with_long_status_text() {
        let ctx = egui::Context::default();
        crate::ui::skin::apply(&ctx);
        for size in [egui::vec2(1280.0, 720.0), egui::vec2(320.0, 240.0)] {
            for _ in 0..5 {
                let mut rects = Vec::new();
                let mut widgets = vec![
                    Widget::RowBegin,
                    Widget::Slots {
                        first: 0,
                        count: 3,
                        columns: 3,
                    },
                    Widget::Bar {
                        text: "Alliage : fer + charbon, ou cuivre + malachite".into(),
                        value: 0.4,
                    },
                    Widget::Slots {
                        first: 3,
                        count: 1,
                        columns: 1,
                    },
                    Widget::RowEnd,
                ];
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui::layout::inventory_window(
                            ui.ctx(),
                            "Forge",
                            egui::Id::unique("test_mod_row"),
                        )
                        .show(ui.ctx(), |ui| {
                            crate::ui::layout::inventory_body(ui);
                            draw_widgets(
                                ui,
                                &mut widgets,
                                &mut vec![],
                                &mut |ui, first, count, columns| {
                                    crate::ui::layout::slots(ui, count, columns, |ui, index| {
                                        let cell = crate::ui::layout::slot_size(ui.ctx());
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::Vec2::splat(cell),
                                            egui::Sense::click(),
                                        );
                                        rects.push((first + index, rect));
                                    });
                                },
                            );
                        });
                    },
                );
                output.textures_delta.clear();
                assert_eq!(rects.len(), 4);
                assert!(
                    (rects[0].1.center().y - rects[3].1.center().y).abs() < 1.0,
                    "{size:?}: {rects:?}"
                );
                assert!(
                    rects.windows(2).all(|p| p[0].1.right() < p[1].1.left()),
                    "{size:?}: {rects:?}"
                );
                assert!(
                    rects
                        .iter()
                        .all(|(_, r)| r.left() >= 0.0 && r.right() <= size.x)
                );
                output.textures_delta.clear();
            }
        }
    }
}
