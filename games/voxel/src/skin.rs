//! Palette et dessin de l'interface « terre, ardoise et cuivre ».
//! Les mêmes matières relient les menus, l'inventaire et le HUD.

use engine_render::egui::{self, Color32, FontId, Stroke};

pub const TEXT: Color32 = Color32::from_rgb(236, 227, 204);
pub const MUTED: Color32 = Color32::from_rgb(164, 178, 168);
pub const COPPER: Color32 = Color32::from_rgb(205, 148, 94);
pub const BORDER: Color32 = Color32::from_rgb(91, 110, 107);
pub const PANEL: Color32 = Color32::from_rgb(34, 46, 48);
pub const INSET: Color32 = Color32::from_rgb(23, 31, 33);
pub const BUTTON: Color32 = Color32::from_rgb(52, 67, 66);
pub const HOVER: Color32 = Color32::from_rgb(67, 84, 78);
pub const DARK: Color32 = Color32::from_rgb(14, 21, 24);
pub const SAGE: Color32 = Color32::from_rgb(151, 192, 117);
pub const RED: Color32 = Color32::from_rgb(217, 109, 86);
pub const GOLD: Color32 = Color32::from_rgb(220, 180, 104);
pub const WATER: Color32 = Color32::from_rgb(110, 182, 198);
pub const MAGIC: Color32 = Color32::from_rgb(177, 155, 206);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(MUTED);
    v.window_fill = PANEL;
    v.panel_fill = PANEL;
    v.extreme_bg_color = INSET;
    v.faint_bg_color = BUTTON;
    v.code_bg_color = INSET;
    v.window_stroke = Stroke::new(2.0, BORDER);
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::epaint::Shadow {
        offset: [4, 6],
        blur: 0,
        spread: 0,
        color: Color32::from_black_alpha(115),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = HOVER;
    v.selection.stroke = Stroke::new(1.0, COPPER);
    v.hyperlink_color = WATER;
    v.warn_fg_color = GOLD;
    v.error_fg_color = RED;
    for (widget, fill, border) in [
        (&mut v.widgets.noninteractive, PANEL, BORDER),
        (&mut v.widgets.inactive, BUTTON, BORDER),
        (&mut v.widgets.hovered, HOVER, COPPER),
        (&mut v.widgets.active, INSET, COPPER),
        (&mut v.widgets.open, HOVER, COPPER),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, border);
        widget.fg_stroke = Stroke::new(1.5, TEXT);
        widget.corner_radius = egui::CornerRadius::ZERO;
        widget.expansion = 0.0;
    }
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.interact_size = egui::vec2(36.0, 30.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::monospace(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(23.0));
    style.animation_time = 0.08;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn panel() -> egui::Frame {
    egui::Frame::window(&egui::Style::default())
        .fill(PANEL)
        .stroke(Stroke::new(2.0, BORDER))
        .corner_radius(0)
        .inner_margin(egui::Margin::same(18))
        .shadow(egui::epaint::Shadow {
            offset: [4, 6],
            blur: 0,
            spread: 0,
            color: Color32::from_black_alpha(115),
        })
}

/// Case creusée dans l'ardoise, avec une sélection couleur cuivre.
pub fn slot(p: &egui::Painter, rect: egui::Rect, selected: bool, hovered: bool) {
    p.rect_filled(rect, 0, if hovered { BUTTON } else { INSET });
    p.rect_stroke(
        rect,
        0,
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected { COPPER } else { BORDER },
        ),
        egui::StrokeKind::Inside,
    );
    let inset = rect.shrink(2.0);
    p.line_segment(
        [inset.left_top(), inset.right_top()],
        Stroke::new(1.0, DARK),
    );
    p.line_segment(
        [inset.left_top(), inset.left_bottom()],
        Stroke::new(1.0, DARK),
    );
    if selected {
        p.rect_filled(
            egui::Rect::from_min_size(
                rect.left_bottom() + egui::vec2(8.0, -4.0),
                egui::vec2(rect.width() - 16.0, 2.0),
            ),
            0,
            COPPER,
        );
    }
}

pub fn menu_button(ui: &mut egui::Ui, text: &str) -> bool {
    let response = ui.add_sized(
        [280.0, 40.0],
        egui::Button::new(egui::RichText::new(text).size(16.0)),
    );
    let rect = response.rect.shrink(2.0);
    let edge = if response.hovered() { COPPER } else { BORDER };
    ui.painter()
        .line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, edge));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(2.0, DARK),
    );
    response.clicked()
}

pub fn mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 24.0), egui::Sense::hover());
    let p = ui.painter();
    for (x, y, color) in [(2.0, 9.0, BORDER), (18.0, 1.0, COPPER), (34.0, 9.0, SAGE)] {
        let tile = egui::Rect::from_min_size(rect.min + egui::vec2(x, y), egui::vec2(14.0, 14.0));
        p.rect_filled(tile, 0, color);
        p.line_segment(
            [tile.left_bottom(), tile.right_bottom()],
            Stroke::new(3.0, DARK),
        );
    }
}

/// Losanges de vitalité, grains de réserve : des signes lisibles sans police spéciale.
pub fn meter(p: &egui::Painter, rect: egui::Rect, fill: f32, color: Color32, seed: bool) {
    let center = rect.center();
    let half = rect.width() * 0.5;
    let points = if seed {
        vec![
            center + egui::vec2(-half, 0.0),
            center + egui::vec2(0.0, -4.5),
            center + egui::vec2(half, -3.0),
            center + egui::vec2(half, 2.0),
            center + egui::vec2(0.0, 5.0),
        ]
    } else {
        vec![
            center + egui::vec2(0.0, -6.0),
            center + egui::vec2(half, 0.0),
            center + egui::vec2(0.0, 6.0),
            center + egui::vec2(-half, 0.0),
        ]
    };
    p.add(egui::Shape::convex_polygon(
        points.clone(),
        DARK,
        Stroke::new(1.0, BORDER),
    ));
    if fill > 0.0 {
        let mut clip = rect;
        clip.set_width(rect.width() * fill.clamp(0.0, 1.0));
        let painter = p.with_clip_rect(p.clip_rect().intersect(clip));
        let inner = points
            .into_iter()
            .map(|point| center + (point - center) * 0.72)
            .collect();
        painter.add(egui::Shape::convex_polygon(inner, color, Stroke::NONE));
    }
}
