//! La géométrie du plan des quêtes, sans egui ni ECS dans les décisions : conversion grille ↔ écran,
//! cadrage, zoom autour du curseur, clic sur une quête, ordre et tracé des traits de dépendance,
//! formes, lignes de la grille, empilement des quêtes épinglées. Tout est testé ici ; l'écran
//! (`map.rs`) ne fait que dessiner ce que ces fonctions décident.

use engine_render::egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::quest_progress::Status;
use crate::quests::Shape;

/// Une case de la grille, en points, au zoom 1.
pub const CELL: f32 = 40.0;
pub const MIN_ZOOM: f32 = 0.3;
pub const MAX_ZOOM: f32 = 3.0;
/// Le diamètre d'une quête de taille 1, en cases.
pub const NODE: f32 = 1.0;
/// Le zoom maximal du cadrage automatique : un chapitre d'une seule quête ne se colle pas à l'écran.
const FIT_MAX_ZOOM: f32 = 1.6;
/// La marge du cadrage, en cases.
const FIT_MARGIN: f32 = 0.9;

/// Ce qu'on regarde : le point de la grille au milieu de la fenêtre, et le zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    pub center: Pos2,
    pub zoom: f32,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            center: Pos2::ZERO,
            zoom: 1.0,
        }
    }
}

impl MapView {
    /// Les points par case de grille.
    pub fn scale(&self) -> f32 {
        CELL * self.zoom
    }

    pub fn to_screen(self, grid: Pos2, viewport: Rect) -> Pos2 {
        viewport.center() + (grid - self.center) * self.scale()
    }

    pub fn to_grid(self, screen: Pos2, viewport: Rect) -> Pos2 {
        self.center + (screen - viewport.center()) / self.scale()
    }

    /// Fait glisser le plan : le contenu suit la souris.
    pub fn pan_by(&mut self, screen_delta: Vec2) {
        self.center -= screen_delta / self.scale();
    }

    /// Multiplie le zoom, en gardant fixe le point de la grille sous `anchor`.
    pub fn zoom_at(&mut self, factor: f32, anchor: Pos2, viewport: Rect) {
        let before = self.to_grid(anchor, viewport);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_grid(anchor, viewport);
        self.center += before - after;
    }

    /// Le cadrage qui montre tout `bounds` (en cases) dans une fenêtre de `viewport` points.
    pub fn fit(bounds: Rect, viewport: Vec2) -> Self {
        let size = bounds.size() + Vec2::splat(2.0 * FIT_MARGIN);
        let zoom = (viewport.x / (size.x * CELL))
            .min(viewport.y / (size.y * CELL))
            .clamp(MIN_ZOOM, FIT_MAX_ZOOM);
        Self {
            center: bounds.center(),
            zoom: if zoom.is_finite() { zoom } else { 1.0 },
        }
    }

    /// Centre le plan sur ce point de la grille, sans changer le zoom.
    pub fn center_on(&mut self, grid: Pos2) {
        self.center = grid;
    }
}

/// Une quête sur le plan : ce qu'il faut pour la placer, la relier et la colorer.
#[derive(Debug, Clone)]
pub struct PlanNode {
    /// Son numéro dans `Quests::quests`.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub shape: Shape,
    pub status: Status,
    /// Ses dépendances (numéros dans `Quests::quests`).
    pub deps: Vec<usize>,
}

impl PlanNode {
    pub fn grid(&self) -> Pos2 {
        pos2(self.x, self.y)
    }

    /// Le rayon, en points.
    pub fn radius(&self, view: &MapView) -> f32 {
        NODE * self.size * view.scale() * 0.5
    }
}

/// Le rectangle de la grille (en cases) qui contient toutes les quêtes, avec leur taille.
pub fn grid_bounds(nodes: &[PlanNode]) -> Option<Rect> {
    nodes
        .iter()
        .map(|n| Rect::from_center_size(n.grid(), Vec2::splat(NODE * n.size)))
        .reduce(|a, b| a.union(b))
}

/// La quête sous `at` (la dernière dessinée si elles se recouvrent). Les formes carrées se
/// touchent aux angles, les autres comme un disque.
pub fn hit_test(view: &MapView, viewport: Rect, nodes: &[PlanNode], at: Pos2) -> Option<usize> {
    nodes
        .iter()
        .rev()
        .find(|n| {
            let d = at - view.to_screen(n.grid(), viewport);
            let r = n.radius(view);
            match n.shape {
                Shape::Square | Shape::RoundSquare => d.x.abs() <= r && d.y.abs() <= r,
                _ => d.length() <= r,
            }
        })
        .map(|n| n.index)
}

/// La couleur d'un trait : le chemin déjà parcouru (vert), la suite à faire (clair), le verrouillé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeState {
    Locked,
    Open,
    Done,
}

/// Un trait de dépendance, de `from` (à finir d'abord) vers `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub state: EdgeState,
}

/// Les traits du plan, dans l'ordre où on les dessine : les verrouillés d'abord, les verts en
/// dernier (ils passent par-dessus). Seules les quêtes montrées (`nodes`) sont reliées ; une
/// dépendance d'un autre chapitre n'a pas de place sur ce plan.
pub fn edges(nodes: &[PlanNode]) -> Vec<Edge> {
    let mut out = Vec::new();
    for node in nodes {
        for &dep in &node.deps {
            let Some(from) = nodes.iter().find(|n| n.index == dep) else {
                continue;
            };
            let state = match from.status {
                Status::Done | Status::Claimed => EdgeState::Done,
                Status::Locked => EdgeState::Locked,
                _ => EdgeState::Open,
            };
            out.push(Edge {
                from: dep,
                to: node.index,
                state,
            });
        }
    }
    out.sort_by_key(|e| e.state);
    out
}

/// Les deux bouts d'un trait à l'écran, raccourcis au bord des quêtes (plus un petit jeu).
/// `None` si les quêtes se touchent.
pub fn edge_segment(
    view: &MapView,
    viewport: Rect,
    from: &PlanNode,
    to: &PlanNode,
) -> Option<(Pos2, Pos2)> {
    let a = view.to_screen(from.grid(), viewport);
    let b = view.to_screen(to.grid(), viewport);
    let along = b - a;
    let length = along.length();
    let gap = 3.0;
    let (start, end) = (from.radius(view) + gap, to.radius(view) + gap + 2.0);
    if length <= start + end + 1.0 {
        return None;
    }
    let unit = along / length;
    Some((a + unit * start, b - unit * end))
}

/// Les sommets d'une forme à facettes (hexagone, pentagone, losange), la pointe en haut.
/// `None` pour les formes que le peintre sait tracer lui-même (cercle, carrés).
pub fn polygon(shape: Shape, center: Pos2, radius: f32) -> Option<Vec<Pos2>> {
    let sides = match shape {
        Shape::Hexagon => 6,
        Shape::Pentagon => 5,
        Shape::Diamond => 4,
        _ => return None,
    };
    Some(
        (0..sides)
            .map(|k| {
                let angle =
                    -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / sides as f32;
                center + vec2(angle.cos(), angle.sin()) * radius
            })
            .collect(),
    )
}

/// Les lignes de la grille visibles : leurs positions en points (verticales, horizontales).
/// Une ligne par case ; au zoom très faible, une sur deux ou plus, pour rester lisible.
pub fn grid_lines(view: &MapView, viewport: Rect) -> (Vec<f32>, Vec<f32>) {
    let mut step = 1.0_f32;
    while step * view.scale() < 14.0 && step < 64.0 {
        step *= 2.0;
    }
    let min = view.to_grid(viewport.min, viewport);
    let max = view.to_grid(viewport.max, viewport);
    let lines = |lo: f32, hi: f32, screen: &dyn Fn(f32) -> f32| {
        let mut out = Vec::new();
        let mut g = (lo / step).floor() * step;
        while g <= hi && out.len() < 400 {
            out.push(screen(g));
            g += step;
        }
        out
    };
    (
        lines(min.x, max.x, &|g| view.to_screen(pos2(g, 0.0), viewport).x),
        lines(min.y, max.y, &|g| view.to_screen(pos2(0.0, g), viewport).y),
    )
}

/// Le décalage (depuis le coin haut droit) de la pile des quêtes épinglées : sous les potions en
/// cours, et, sur un écran étroit où les potions descendent sous la cible visée, sous elles aussi.
pub fn pin_offset(
    effects: usize,
    screen_width: f32,
    toast_visible: bool,
    target_bottom: f32,
) -> Vec2 {
    let top = if screen_width < 760.0 {
        target_bottom + if toast_visible { 96.0 } else { 8.0 }
    } else {
        12.0
    };
    let potions = if effects > 0 {
        effects as f32 * 20.0 + 8.0
    } else {
        0.0
    };
    vec2(-12.0, top + potions)
}

/// La largeur de la pile des quêtes épinglées sur un écran de `screen_width` points.
pub fn pin_width(screen_width: f32) -> f32 {
    (screen_width * 0.3)
        .clamp(180.0, 280.0)
        .min(screen_width - 24.0)
        .max(40.0)
}

/// Les chapitres rangés par groupe, dans l'ordre des groupes : (numéro du groupe, chapitres).
/// Un groupe sans chapitre n'est pas montré.
pub fn chapters_by_group(group_of_chapter: &[usize], groups: usize) -> Vec<(usize, Vec<usize>)> {
    (0..groups)
        .filter_map(|g| {
            let chapters: Vec<usize> = group_of_chapter
                .iter()
                .enumerate()
                .filter(|&(_, &group)| group == g)
                .map(|(c, _)| c)
                .collect();
            (!chapters.is_empty()).then_some((g, chapters))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::from_min_size(pos2(200.0, 50.0), vec2(700.0, 500.0))
    }

    fn node(index: usize, x: f32, y: f32, status: Status, deps: &[usize]) -> PlanNode {
        PlanNode {
            index,
            x,
            y,
            size: 1.0,
            shape: Shape::Circle,
            status,
            deps: deps.to_vec(),
        }
    }

    #[test]
    fn grid_and_screen_are_inverse() {
        let view = MapView {
            center: pos2(3.5, -2.0),
            zoom: 1.7,
        };
        for p in [pos2(0.0, 0.0), pos2(-4.25, 8.5), pos2(3.5, -2.0)] {
            let back = view.to_grid(view.to_screen(p, viewport()), viewport());
            assert!((back - p).length() < 1e-4, "{p:?} → {back:?}");
        }
        // Le centre de la vue est au centre de la fenêtre.
        assert_eq!(view.to_screen(view.center, viewport()), viewport().center());
        // Une case de grille vaut CELL × zoom points.
        let a = view.to_screen(pos2(0.0, 0.0), viewport());
        let b = view.to_screen(pos2(1.0, 0.0), viewport());
        assert!((b.x - a.x - CELL * 1.7).abs() < 1e-3);
    }

    #[test]
    fn dragging_moves_the_map_with_the_mouse() {
        let mut view = MapView::default();
        let before = view.to_screen(pos2(2.0, 1.0), viewport());
        view.pan_by(vec2(30.0, -12.0));
        let after = view.to_screen(pos2(2.0, 1.0), viewport());
        assert!((after - before - vec2(30.0, -12.0)).length() < 1e-3);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let mut view = MapView {
            center: pos2(1.0, 2.0),
            zoom: 1.0,
        };
        let cursor = pos2(310.0, 130.0);
        let under = view.to_grid(cursor, viewport());
        view.zoom_at(1.8, cursor, viewport());
        assert!((view.zoom - 1.8).abs() < 1e-5);
        assert!((view.to_grid(cursor, viewport()) - under).length() < 1e-4);
        // Le zoom reste dans ses bornes.
        view.zoom_at(100.0, cursor, viewport());
        assert_eq!(view.zoom, MAX_ZOOM);
        view.zoom_at(0.0001, cursor, viewport());
        assert_eq!(view.zoom, MIN_ZOOM);
    }

    #[test]
    fn fit_shows_every_quest_inside_the_window() {
        let nodes = vec![
            node(0, -3.0, 0.0, Status::Available, &[]),
            node(1, 9.0, 6.0, Status::Locked, &[0]),
        ];
        let bounds = grid_bounds(&nodes).unwrap();
        let view = MapView::fit(bounds, viewport().size());
        let vp = viewport();
        for n in &nodes {
            let at = view.to_screen(n.grid(), vp);
            let r = n.radius(&view);
            assert!(
                vp.contains(at - vec2(r, r)) && vp.contains(at + vec2(r, r)),
                "{n:?}"
            );
        }
        assert!(view.zoom <= FIT_MAX_ZOOM);
        // Une seule quête : pas de zoom démesuré, et centrée.
        let one = vec![node(0, 4.0, 4.0, Status::Available, &[])];
        let view = MapView::fit(grid_bounds(&one).unwrap(), viewport().size());
        assert_eq!(view.zoom, FIT_MAX_ZOOM);
        assert_eq!(view.center, pos2(4.0, 4.0));
        assert!(grid_bounds(&[]).is_none());
    }

    #[test]
    fn a_click_finds_the_quest_under_it() {
        let nodes = vec![
            node(5, 0.0, 0.0, Status::Available, &[]),
            PlanNode {
                size: 2.0,
                shape: Shape::Square,
                ..node(7, 3.0, 0.0, Status::Available, &[])
            },
        ];
        let view = MapView::default();
        let vp = viewport();
        let at = |x: f32, y: f32| view.to_screen(pos2(x, y), vp);
        assert_eq!(hit_test(&view, vp, &nodes, at(0.0, 0.0)), Some(5));
        assert_eq!(hit_test(&view, vp, &nodes, at(0.3, 0.0)), Some(5));
        assert_eq!(hit_test(&view, vp, &nodes, at(1.5, 0.0)), None);
        // Un carré de taille 2 : son coin est dedans, le coin d'un disque ne l'est pas.
        assert_eq!(hit_test(&view, vp, &nodes, at(3.8, 0.8)), Some(7));
        let disc = vec![node(1, 0.0, 0.0, Status::Available, &[])];
        assert_eq!(hit_test(&view, vp, &disc, at(0.4, 0.4)), None);
    }

    #[test]
    fn edges_are_drawn_locked_first_and_green_last() {
        let nodes = vec![
            node(0, 0.0, 0.0, Status::Claimed, &[]),
            node(1, 2.0, 0.0, Status::Available, &[0]),
            node(2, 4.0, 0.0, Status::Locked, &[1]),
            node(3, 6.0, 0.0, Status::Locked, &[2, 99]),
            node(4, 2.0, 2.0, Status::Started, &[0]),
        ];
        let edges = edges(&nodes);
        // 99 n'est pas sur ce plan : pas de trait.
        assert_eq!(edges.len(), 4);
        let states: Vec<_> = edges.iter().map(|e| e.state).collect();
        let mut sorted = states.clone();
        sorted.sort();
        assert_eq!(states, sorted);
        assert_eq!(states[0], EdgeState::Locked);
        assert_eq!(*states.last().unwrap(), EdgeState::Done);
        // La dépendance finie donne un trait vert, une disponible un trait clair.
        let of = |from, to| {
            edges
                .iter()
                .find(|e| e.from == from && e.to == to)
                .unwrap()
                .state
        };
        assert_eq!(of(0, 1), EdgeState::Done);
        assert_eq!(of(1, 2), EdgeState::Open);
        assert_eq!(of(2, 3), EdgeState::Locked);
    }

    #[test]
    fn an_edge_stops_at_the_border_of_the_quests() {
        let a = node(0, 0.0, 0.0, Status::Available, &[]);
        let b = node(1, 3.0, 0.0, Status::Locked, &[0]);
        let view = MapView::default();
        let (start, end) = edge_segment(&view, viewport(), &a, &b).unwrap();
        let (ca, cb) = (
            view.to_screen(a.grid(), viewport()),
            view.to_screen(b.grid(), viewport()),
        );
        assert!(start.x > ca.x + a.radius(&view));
        assert!(end.x < cb.x - b.radius(&view));
        // Deux quêtes qui se touchent : rien à tracer.
        let close = node(2, 0.9, 0.0, Status::Locked, &[0]);
        assert!(edge_segment(&view, viewport(), &a, &close).is_none());
    }

    #[test]
    fn polygons_have_the_right_corners() {
        let c = pos2(10.0, 10.0);
        assert_eq!(polygon(Shape::Hexagon, c, 5.0).unwrap().len(), 6);
        assert_eq!(polygon(Shape::Pentagon, c, 5.0).unwrap().len(), 5);
        let diamond = polygon(Shape::Diamond, c, 5.0).unwrap();
        assert_eq!(diamond.len(), 4);
        // La pointe est en haut.
        assert!((diamond[0] - pos2(10.0, 5.0)).length() < 1e-4);
        for p in polygon(Shape::Hexagon, c, 5.0).unwrap() {
            assert!(((p - c).length() - 5.0).abs() < 1e-4);
        }
        assert!(polygon(Shape::Circle, c, 5.0).is_none());
        assert!(polygon(Shape::Square, c, 5.0).is_none());
        assert!(polygon(Shape::RoundSquare, c, 5.0).is_none());
    }

    #[test]
    fn grid_lines_stay_few_and_inside_the_window() {
        let vp = viewport();
        for zoom in [MIN_ZOOM, 0.7, 1.0, 3.0] {
            let view = MapView {
                center: pos2(2.0, 2.0),
                zoom,
            };
            let (xs, ys) = grid_lines(&view, vp);
            assert!(!xs.is_empty() && !ys.is_empty());
            assert!(xs.len() < 100 && ys.len() < 100, "zoom {zoom}");
            // Les lignes sont espacées d'au moins 14 points.
            for pair in xs.windows(2) {
                assert!(pair[1] - pair[0] >= 13.9);
            }
        }
    }

    #[test]
    fn pins_sit_under_the_potions_at_the_right_edge() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
        assert!(screen.contains(pos2(1268.0, 12.0)));
        let none = pin_offset(0, 1280.0, false, 100.0);
        let two = pin_offset(2, 1280.0, false, 100.0);
        assert_eq!(none, vec2(-12.0, 12.0));
        assert!(two.y > none.y + 40.0);
        // Écran étroit : sous la cible visée, et plus bas encore avec une bannière.
        assert!(pin_offset(0, 600.0, false, 100.0).y >= 108.0);
        assert!(pin_offset(0, 600.0, true, 100.0).y >= 196.0);
        let width = pin_width(1280.0);
        assert!((180.0..=280.0).contains(&width));
        // Le cadre tient dans l'écran, serré contre le bord droit.
        let right = screen.right() + two.x;
        assert!(right - width >= screen.left() && right <= screen.right());
        assert!(pin_width(200.0) <= 176.0);
    }

    #[test]
    fn chapters_are_grouped_in_group_order_and_empty_groups_hidden() {
        // Chapitres 0..4 rangés dans les groupes 2, 0, 2, 0 ; le groupe 1 est vide.
        let grouped = chapters_by_group(&[2, 0, 2, 0], 3);
        assert_eq!(grouped, vec![(0, vec![1, 3]), (2, vec![0, 2])]);
        assert!(chapters_by_group(&[], 3).is_empty());
    }
}
