//! Palette et dessin de l'interface « terre, ardoise et cuivre ».
//! Les mêmes matières relient les menus, l'inventaire et le HUD.

use engine_render::egui::{self, Color32, FontId, Stroke};

pub const TEXT: Color32 = Color32::from_rgb(242, 240, 230);
pub const MUTED: Color32 = Color32::from_rgb(174, 177, 171);
pub const COPPER: Color32 = Color32::from_rgb(205, 148, 94);
pub const BORDER: Color32 = Color32::from_rgb(91, 94, 91);
pub const PANEL: Color32 = Color32::from_rgb(33, 35, 34);
pub const INSET: Color32 = Color32::from_rgb(25, 26, 25);
pub const BUTTON: Color32 = Color32::from_rgb(102, 105, 99);
pub const HOVER: Color32 = Color32::from_rgb(127, 133, 119);
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
    style.spacing.item_spacing = egui::vec2(4.0, 4.0);
    style.spacing.button_padding = egui::vec2(12.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.interact_size = egui::vec2(36.0, 30.0);
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::monospace(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::monospace(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::monospace(22.0));
    style.animation_time = 0.08;
    style.wrap_mode = Some(egui::TextWrapMode::Wrap);
    style.spacing.tooltip_width = 280.0;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.interaction.tooltip_delay = 0.12;
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

pub const INVENTORY: Color32 = Color32::from_rgb(183, 186, 174);
pub const INK: Color32 = Color32::from_rgb(48, 50, 45);
pub const RIM: Color32 = Color32::from_rgb(228, 229, 215);

/// Relief carré : un liseré clair en haut/gauche, sombre en bas/droite.
pub fn bevel(p: &egui::Painter, rect: egui::Rect, light: Color32, dark: Color32, edge: f32) {
    p.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), edge)),
        0,
        light,
    );
    p.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(edge, rect.height())),
        0,
        light,
    );
    p.rect_filled(
        egui::Rect::from_min_size(
            rect.left_bottom() - egui::vec2(0.0, edge),
            egui::vec2(rect.width(), edge),
        ),
        0,
        dark,
    );
    p.rect_filled(
        egui::Rect::from_min_size(
            rect.right_top() - egui::vec2(edge, 0.0),
            egui::vec2(edge, rect.height()),
        ),
        0,
        dark,
    );
}

pub fn inventory_panel() -> egui::Frame {
    egui::Frame::new()
        .fill(INVENTORY)
        .stroke(Stroke::new(3.0, DARK))
        .inner_margin(14)
        .corner_radius(0)
}

pub fn inventory_border(response: &egui::Response) {
    bevel(
        &response.ctx.layer_painter(response.layer_id),
        response.rect.shrink(3.0),
        RIM,
        Color32::from_rgb(80, 85, 73),
        2.0,
    );
}

/// Style local : texte sombre sur le panneau clair, infobulles toujours sombres.
pub fn inventory_style(ui: &mut egui::Ui) {
    let v = ui.visuals_mut();
    v.override_text_color = Some(INK);
    v.weak_text_color = Some(Color32::from_rgb(79, 83, 73));
    v.widgets.active.fg_stroke.color = INK;
    v.widgets.noninteractive.fg_stroke.color = INK;
    v.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(131, 135, 124);
    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
}

/// Case en creux, sélection claire visible même sur les petits objets.
pub fn slot(p: &egui::Painter, rect: egui::Rect, selected: bool, hovered: bool) {
    p.rect_filled(
        rect,
        0,
        if hovered {
            Color32::from_rgb(153, 158, 143)
        } else {
            Color32::from_rgb(116, 121, 109)
        },
    );
    bevel(
        p,
        rect,
        Color32::from_rgb(57, 61, 53),
        RIM,
        (rect.width() / 18.0).min(2.0),
    );
    if selected {
        p.rect_stroke(
            rect.expand(1.0),
            0,
            Stroke::new(2.0, TEXT),
            egui::StrokeKind::Inside,
        );
    }
    if hovered {
        p.rect_filled(rect.shrink(2.0), 0, Color32::from_white_alpha(28));
    }
}

/// Deux points de bord suffisent : l'image occupe toute la surface utile de la case.
pub fn icon_rect(rect: egui::Rect) -> egui::Rect {
    rect.shrink((rect.width() / 18.0).min(2.0))
}

/// Dessin commun aux inventaires, conteneurs, machines et à la pile sous la souris.
pub fn stack(
    p: &egui::Painter,
    rect: egui::Rect,
    icon: egui::TextureId,
    count: u32,
    durability: Option<f32>,
    border: Option<Color32>,
) {
    p.image(
        icon,
        icon_rect(rect),
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    if count > 1 {
        let at = rect.right_bottom() - egui::vec2(3.0, 2.0);
        let font = FontId::monospace((rect.width() * 0.3).clamp(7.0, 16.0));
        for (offset, color) in [(egui::vec2(1.0, 1.0), DARK), (egui::Vec2::ZERO, TEXT)] {
            p.text(
                at + offset,
                egui::Align2::RIGHT_BOTTOM,
                count.to_string(),
                font.clone(),
                color,
            );
        }
    }
    // Le liseré de la case : la rareté d'un objet à affixes, ou l'enchantement.
    if let Some(color) = border {
        p.rect_stroke(
            rect.shrink(2.0),
            0,
            Stroke::new(2.0, color),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(left) = durability {
        let bar = egui::Rect::from_min_size(
            rect.left_bottom() + egui::vec2(3.0, -(rect.height() * 0.2).min(7.0)),
            egui::vec2(
                (rect.width() - 6.0).max(1.0),
                (rect.height() * 0.08).clamp(1.0, 3.0),
            ),
        );
        p.rect_filled(bar, 0, Color32::BLACK);
        let mut fill = bar;
        fill.set_width(bar.width() * left);
        p.rect_filled(
            fill,
            0,
            if left > 0.5 {
                SAGE
            } else if left > 0.2 {
                GOLD
            } else {
                RED
            },
        );
    }
}

pub fn menu_button_sized(ui: &mut egui::Ui, text: &str, width: f32) -> bool {
    let width = width.min(ui.available_width()).max(20.0);
    let font = FontId::monospace(if width < 110.0 { 12.0 } else { 15.0 });
    let galley = ui
        .painter()
        .layout(text.into(), font, TEXT, (width - 16.0).max(1.0));
    let size = egui::vec2(width, (galley.size().y + 14.0).max(36.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let pressed = response.is_pointer_button_down_on();
    let hover = response.hovered() || response.has_focus();
    let p = ui.painter();
    p.rect_filled(rect, 0, DARK);
    let inner = rect.shrink(2.0);
    p.rect_filled(
        inner,
        0,
        if pressed {
            BORDER
        } else if hover {
            HOVER
        } else {
            BUTTON
        },
    );
    bevel(
        p,
        inner,
        if pressed {
            DARK
        } else {
            Color32::from_rgb(171, 175, 161)
        },
        if pressed {
            BORDER
        } else {
            Color32::from_rgb(52, 55, 49)
        },
        2.0,
    );
    if hover {
        p.rect_stroke(rect, 0, Stroke::new(1.0, TEXT), egui::StrokeKind::Inside);
    }
    let at = rect.center() - galley.size() * 0.5 + egui::vec2(0.0, if pressed { 1.0 } else { 0.0 });
    p.galley_with_override_text_color(at + egui::vec2(1.0, 2.0), galley.clone(), DARK);
    p.galley(at, galley, if ui.is_enabled() { TEXT } else { MUTED });
    response.clicked()
}

pub fn menu_button(ui: &mut egui::Ui, text: &str) -> bool {
    menu_button_sized(ui, text, ui.available_width().min(320.0))
}

/// Logo original en pixels, dessiné en géométrie : aucune police ni image externe.
pub fn logo(ui: &mut egui::Ui) {
    let scale = ((ui.available_width() - 12.0) / 29.0)
        .clamp(
            3.0,
            (ui.ctx().content_rect().height() / 35.0).clamp(3.0, 15.0),
        )
        .floor();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(29.0 * scale, 8.0 * scale), egui::Sense::hover());
    let letters = [
        [17u8, 17, 17, 17, 17, 10, 4],
        [14, 17, 17, 17, 17, 17, 14],
        [17, 17, 10, 4, 10, 17, 17],
        [31, 16, 16, 30, 16, 16, 31],
        [16, 16, 16, 16, 16, 16, 31],
    ];
    for (letter, rows) in letters.iter().enumerate() {
        for (y, row) in rows.iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) == 0 {
                    continue;
                }
                let cell = egui::Rect::from_min_size(
                    rect.min + egui::vec2((letter * 6 + x) as f32 * scale, y as f32 * scale),
                    egui::vec2(scale, scale),
                );
                ui.painter()
                    .rect_filled(cell.translate(egui::vec2(0.0, scale)), 0, DARK);
                ui.painter()
                    .rect_filled(cell, 0, if y < 3 { RIM } else { INVENTORY });
            }
        }
    }
}

#[allow(dead_code)] // Planche de contrôle historique, pas utilisée par les menus.
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

/// La couleur d'un ton de la simulation (voir `tone.rs`).
pub fn tone(tone: voxel_sim::player::tone::Tone) -> Color32 {
    use voxel_sim::player::tone::Tone;
    match tone {
        Tone::Text => TEXT,
        Tone::Muted => MUTED,
        Tone::Red => RED,
        Tone::Sage => SAGE,
        Tone::Gold => GOLD,
        Tone::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}
