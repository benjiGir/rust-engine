//! Le plan d'un chapitre : les quêtes à leur place, reliées par leurs dépendances, dans la couleur
//! de leur état. On le fait glisser (clic gauche ou milieu), on le zoome à la molette, un clic sur
//! une quête ouvre sa fiche, un clic dans le vide la ferme. Ce qui se décide (où sont les choses,
//! l'ordre des traits, quelle quête est sous la souris) est dans `geometry.rs`.

use engine_render::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};

use super::geometry::{self, EdgeState, MapView, PlanNode};
use super::labels::{self, Namer};
use super::{Act, fit_text};
use crate::VoxelGame;
use crate::quest_progress::Status;
use crate::quests::{Quest, Quests, Shape};
use crate::skin;

/// Les couleurs d'une quête : (remplissage, contour, teinte de l'icône).
fn node_colors(status: Status) -> (Color32, Color32, Color32) {
    match status {
        Status::Locked => (
            Color32::from_rgb(30, 31, 30),
            Color32::from_rgb(80, 83, 80),
            Color32::from_white_alpha(70),
        ),
        Status::Available => (
            Color32::from_rgb(50, 54, 56),
            Color32::from_rgb(225, 223, 210),
            Color32::WHITE,
        ),
        Status::Started => (Color32::from_rgb(62, 55, 38), skin::GOLD, Color32::WHITE),
        Status::Done => (Color32::from_rgb(48, 70, 46), skin::SAGE, Color32::WHITE),
        Status::Claimed => (
            Color32::from_rgb(34, 50, 36),
            Color32::from_rgb(103, 140, 84),
            Color32::from_white_alpha(215),
        ),
    }
}

fn edge_color(state: EdgeState) -> Color32 {
    match state {
        EdgeState::Locked => Color32::from_rgb(76, 79, 76),
        EdgeState::Open => Color32::from_rgb(205, 203, 190),
        EdgeState::Done => skin::SAGE,
    }
}

/// Le dessin d'une forme (`stroke` au bord).
fn paint_shape(
    painter: &egui::Painter,
    shape: Shape,
    center: Pos2,
    radius: f32,
    fill: Color32,
    stroke: Stroke,
) {
    if let Some(points) = geometry::polygon(shape, center, radius) {
        painter.add(egui::Shape::convex_polygon(points, fill, stroke));
        return;
    }
    match shape {
        Shape::Square => {
            painter.rect(
                Rect::from_center_size(center, egui::Vec2::splat(2.0 * radius)),
                0,
                fill,
                stroke,
                StrokeKind::Inside,
            );
        }
        Shape::RoundSquare => {
            painter.rect(
                Rect::from_center_size(center, egui::Vec2::splat(2.0 * radius)),
                (radius * 0.35) as u8,
                fill,
                stroke,
                StrokeKind::Inside,
            );
        }
        _ => {
            painter.circle(center, radius, fill, stroke);
        }
    }
}

/// Une flèche au bout d'un trait.
fn arrow_head(painter: &egui::Painter, tip: Pos2, from: Pos2, size: f32, color: Color32) {
    let along = (tip - from).normalized();
    let side = vec2(-along.y, along.x);
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            tip - along * size + side * size * 0.55,
            tip - along * size - side * size * 0.55,
        ],
        color,
        Stroke::NONE,
    ));
}

/// Un petit cadenas (quête verrouillée).
fn padlock(painter: &egui::Painter, at: Pos2, size: f32) {
    let ink = Color32::from_rgb(150, 152, 146);
    let body = Rect::from_center_size(at + vec2(0.0, size * 0.2), vec2(size, size * 0.75));
    let stroke = Stroke::new((size * 0.16).max(1.0), ink);
    painter.line_segment(
        [
            body.left_top() + vec2(size * 0.2, 0.0),
            body.left_top() + vec2(size * 0.2, -size * 0.35),
        ],
        stroke,
    );
    painter.line_segment(
        [
            body.right_top() - vec2(size * 0.2, 0.0),
            body.right_top() - vec2(size * 0.2, size * 0.35),
        ],
        stroke,
    );
    painter.line_segment(
        [
            body.left_top() + vec2(size * 0.2, -size * 0.35),
            body.right_top() - vec2(size * 0.2, size * 0.35),
        ],
        stroke,
    );
    painter.rect_filled(body, 0, ink);
}

/// Une coche (quête récupérée).
pub(super) fn check_mark(painter: &egui::Painter, at: Pos2, size: f32) {
    painter.circle(
        at,
        size * 0.62,
        Color32::from_rgb(36, 60, 38),
        Stroke::new(1.0, skin::SAGE),
    );
    let stroke = Stroke::new((size * 0.2).max(1.5), skin::SAGE);
    let a = at + vec2(-size * 0.3, 0.0);
    let b = at + vec2(-size * 0.05, size * 0.27);
    let c = at + vec2(size * 0.34, -size * 0.25);
    painter.line_segment([a, b], stroke);
    painter.line_segment([b, c], stroke);
}

impl VoxelGame {
    /// Les quêtes montrées du chapitre, prêtes pour la géométrie.
    fn plan_nodes(&self, quests: &Quests, statuses: &[Status]) -> Vec<PlanNode> {
        let visible: Vec<bool> = match &self.client.view.me.quests {
            Some(progress) => (0..quests.quests.len())
                .map(|i| progress.visible(quests, i))
                .collect(),
            None => vec![true; quests.quests.len()],
        };
        quests.chapters[self.client.ui.quests.chapter]
            .quests
            .iter()
            .filter(|&&q| visible[q])
            .map(|&q| {
                let quest = &quests.quests[q];
                PlanNode {
                    index: q,
                    x: quest.x,
                    y: quest.y,
                    size: quest.size,
                    shape: quest.shape,
                    status: statuses[q],
                    deps: quest.deps.clone(),
                }
            })
            .collect()
    }

    /// Le plan, dans `viewport`.
    pub(super) fn draw_quest_map(
        &mut self,
        ui: &mut egui::Ui,
        viewport: Rect,
        quests: &Quests,
        statuses: &[Status],
        selected: Option<usize>,
        acts: &mut Vec<Act>,
    ) {
        let nodes = self.plan_nodes(quests, statuses);
        // La fenêtre a changé de taille (la fiche s'ouvre ou se ferme) : un plan resté cadré se
        // recadre, un plan déplacé garde ses quêtes à la même place à l'écran.
        if let Some(old) = self
            .client
            .ui
            .quests
            .viewport
            .filter(|old| *old != viewport)
        {
            if self.client.ui.quests.auto_fit {
                self.client.ui.quests.fit = true;
            } else if self.client.ui.quests.screen == ui.ctx().content_rect().size() {
                let scale = self.client.ui.quests.view.scale();
                self.client.ui.quests.view.center += (viewport.center() - old.center()) / scale;
            }
        }
        self.client.ui.quests.viewport = Some(viewport);
        self.client.ui.quests.screen = ui.ctx().content_rect().size();
        // Le cadrage de l'ouverture ou du changement de chapitre.
        if self.client.ui.quests.fit {
            self.client.ui.quests.fit = false;
            self.client.ui.quests.auto_fit = true;
            self.client.ui.quests.view = match geometry::grid_bounds(&nodes) {
                Some(bounds) => MapView::fit(bounds, viewport.size()),
                None => MapView::default(),
            };
        }
        let response = ui.allocate_rect(viewport, Sense::click_and_drag());
        // Glisser, zoomer.
        if response.dragged() {
            self.client.ui.quests.auto_fit = false;
            self.client.ui.quests.view.pan_by(response.drag_delta());
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll * 0.0025).exp();
            if (factor - 1.0).abs() > 1e-4 {
                self.client.ui.quests.auto_fit = false;
                self.client
                    .ui
                    .quests
                    .view
                    .zoom_at(factor, pointer, viewport);
            }
        }
        let view = self.client.ui.quests.view;
        let hovered = response
            .hover_pos()
            .and_then(|at| geometry::hit_test(&view, viewport, &nodes, at));
        if response.clicked() {
            let at = response.interact_pointer_pos();
            acts.push(Act::Select(
                at.and_then(|at| geometry::hit_test(&view, viewport, &nodes, at)),
            ));
        }
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let painter = ui.painter_at(viewport);
        painter.rect_filled(viewport, 0, Color32::from_rgb(20, 22, 22));
        // La grille, très discrète.
        let (xs, ys) = geometry::grid_lines(&view, viewport);
        let grid = Stroke::new(1.0, Color32::from_white_alpha(9));
        for x in xs {
            painter.line_segment(
                [
                    egui::pos2(x, viewport.top()),
                    egui::pos2(x, viewport.bottom()),
                ],
                grid,
            );
        }
        for y in ys {
            painter.line_segment(
                [
                    egui::pos2(viewport.left(), y),
                    egui::pos2(viewport.right(), y),
                ],
                grid,
            );
        }
        // Les traits d'abord : les quêtes les recouvrent aux extrémités.
        let width = (2.5 * view.zoom).clamp(1.5, 4.0);
        for edge in geometry::edges(&nodes) {
            let (Some(from), Some(to)) = (
                nodes.iter().find(|n| n.index == edge.from),
                nodes.iter().find(|n| n.index == edge.to),
            ) else {
                continue;
            };
            let Some((a, b)) = geometry::edge_segment(&view, viewport, from, to) else {
                continue;
            };
            let color = edge_color(edge.state);
            painter.line_segment([a, b], Stroke::new(width, color));
            arrow_head(
                &painter,
                b + (b - a).normalized() * 2.0,
                a,
                (8.0 * view.zoom).clamp(5.0, 11.0),
                color,
            );
        }
        // Les quêtes, la choisie par-dessus.
        let time = ui.input(|i| i.time);
        let mut drawn = Vec::new();
        let order = nodes
            .iter()
            .filter(|n| Some(n.index) != selected)
            .chain(nodes.iter().filter(|n| Some(n.index) == selected));
        for node in order {
            let quest = &quests.quests[node.index];
            let center = view.to_screen(node.grid(), viewport);
            let radius = node.radius(&view);
            drawn.push((node.index, center, radius));
            if !viewport.expand(radius + 30.0).contains(center) {
                continue;
            }
            self.paint_quest_node(
                &painter,
                quest,
                node,
                center,
                radius,
                Some(node.index) == selected,
                hovered == Some(node.index),
                time,
                view.zoom,
            );
        }
        self.client.ui.quests.drawn = drawn;
        if nodes.iter().any(|n| n.status == Status::Done) {
            ui.ctx().request_repaint();
        }
        painter.rect_stroke(
            viewport,
            0,
            Stroke::new(1.0, skin::BORDER),
            StrokeKind::Inside,
        );
        if nodes.is_empty() {
            painter.text(
                viewport.center(),
                egui::Align2::CENTER_CENTER,
                "Rien à voir ici pour l'instant",
                FontId::monospace(14.0),
                skin::MUTED,
            );
        }
        if let Some(index) = hovered {
            self.quest_tooltip(&response, quests, index, statuses[index]);
        }
    }

    /// Une quête : sa forme, son icône, son état.
    #[allow(clippy::too_many_arguments)]
    fn paint_quest_node(
        &mut self,
        painter: &egui::Painter,
        quest: &Quest,
        node: &PlanNode,
        center: Pos2,
        radius: f32,
        selected: bool,
        hovered: bool,
        time: f64,
        zoom: f32,
    ) {
        let (fill, border, tint) = node_colors(node.status);
        if selected {
            paint_shape(
                painter,
                node.shape,
                center,
                radius + 5.0,
                Color32::TRANSPARENT,
                Stroke::new(2.5, skin::COPPER),
            );
        }
        let border_width = match node.status {
            Status::Done => 3.0,
            _ if hovered => 3.0,
            _ => 2.0,
        };
        paint_shape(
            painter,
            node.shape,
            center,
            radius,
            fill,
            Stroke::new(border_width, if hovered { skin::TEXT } else { border }),
        );
        let icon = self.client.ui.icon(
            quest.icon,
            &self.content.items,
            &self.registry,
            &self.client.block_textures,
        );
        painter.image(
            icon,
            Rect::from_center_size(center, egui::Vec2::splat(radius * 1.2)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
        let corner = center + vec2(radius * 0.66, radius * 0.66);
        match node.status {
            Status::Locked if radius >= 12.0 => padlock(painter, corner, (radius * 0.42).max(7.0)),
            Status::Claimed if radius >= 12.0 => {
                check_mark(painter, corner, (radius * 0.42).max(7.0))
            }
            Status::Done => {
                // La récompense attend : une pastille qui pulse.
                let pulse = 1.0 + 0.18 * (time * 4.5).sin() as f32;
                let at = center + vec2(radius * 0.7, -radius * 0.7);
                let r = (radius * 0.26).max(5.0);
                painter.circle_filled(
                    at,
                    r * 1.7 * pulse,
                    Color32::from_rgba_unmultiplied(220, 180, 104, 70),
                );
                painter.circle(at, r * pulse, skin::GOLD, Stroke::new(1.5, skin::DARK));
            }
            _ => {}
        }
        // Le titre sous la quête, quand il y a la place de le lire.
        if zoom >= 0.6 {
            let galley = fit_text(
                painter,
                &quest.title,
                (11.0 * zoom).clamp(10.0, 14.0),
                if node.status == Status::Locked {
                    skin::MUTED
                } else {
                    skin::TEXT
                },
                (geometry::CELL * zoom * 2.0).max(60.0),
                2,
            );
            let at = egui::pos2(center.x - galley.size().x * 0.5, center.y + radius + 5.0);
            let shadow = galley.rect.translate(at.to_vec2()).expand(2.0);
            painter.rect_filled(shadow, 2, Color32::from_black_alpha(150));
            painter.galley(at, galley, skin::TEXT);
        }
    }

    /// L'infobulle d'une quête : titre, sous-titre, état, tâches.
    fn quest_tooltip(
        &mut self,
        response: &egui::Response,
        quests: &Quests,
        index: usize,
        status: Status,
    ) {
        let quest = &quests.quests[index];
        let progress = self.client.view.me.quests.clone().unwrap_or_default();
        let mobs = self
            .server
            .ecs
            .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
            .clone();
        let namer = Namer {
            items: &self.content.items,
            blocks: &self.registry,
            creatures: &mobs,
        };
        let remaining = labels::remaining(quest, &progress, &namer);
        let mut tooltip = egui::Tooltip::for_enabled(response).at_pointer();
        tooltip.popup = tooltip.popup.frame(
            egui::Frame::new()
                .fill(Color32::from_rgb(19, 17, 28))
                .stroke(Stroke::new(2.0, Color32::from_rgb(96, 82, 131)))
                .inner_margin(8),
        );
        tooltip.show(|ui| {
            ui.set_max_width(260.0);
            ui.label(egui::RichText::new(&quest.title).strong().size(15.0));
            if let Some(sub) = &quest.subtitle {
                ui.label(egui::RichText::new(sub).color(skin::MUTED).size(12.0));
            }
            let (text, color) = match status {
                Status::Locked => (labels::status_label(status), skin::MUTED),
                Status::Available => (labels::status_label(status), skin::TEXT),
                Status::Started => (labels::status_label(status), skin::GOLD),
                Status::Done => (labels::status_label(status), skin::GOLD),
                Status::Claimed => (labels::status_label(status), skin::SAGE),
            };
            ui.label(egui::RichText::new(text).color(color).size(12.0));
            if quest.optional {
                ui.label(
                    egui::RichText::new("Facultative")
                        .color(skin::MAGIC)
                        .size(12.0),
                );
            }
            if matches!(status, Status::Available | Status::Started) {
                for (task, have, need) in remaining.iter().take(4) {
                    let line = if *need > 1 {
                        format!("• {task} ({have} / {need})")
                    } else {
                        format!("• {task}")
                    };
                    ui.label(egui::RichText::new(line).size(12.0));
                }
            }
        });
    }
}
