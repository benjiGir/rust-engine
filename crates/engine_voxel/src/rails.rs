//! Les rails : leur géométrie, calculée depuis les rails voisins, et leur comportement.
//!
//! Un rail est un bloc dont la définition porte `rail: Normal | Booster | Detector | Activator |
//! Stop` (`blocks.ron`) : un mod peut donc ajouter les siens. La **forme** du rail (dix
//! possibilités : deux droites, quatre montées, quatre virages) n'est pas une variante de bloc :
//! elle est calculée à la pose et à chaque changement d'un voisin (comme les raccords des
//! conduits, voir `connections.rs`), puis rangée dans `BlockEntity::data` :
//!
//! - `data[0]` : l'indice de la forme ([`RailShape::index`]) ;
//! - `data[1]` : 1 si le rail reçoit du signal (accélérateur, activateur, butoir, aiguillage).
//!
//! # Les formes
//!
//! Une forme est un chemin dans le bloc, avec deux **extrémités** : chacune est posée au milieu
//! d'un côté du bloc, à ras du sol ou un bloc plus haut (la montée). Le chemin est un segment
//! (droite, montée) ou un quart de cercle de rayon 0,5 (virage). [`RailShape::point`] et
//! [`RailShape::tangent`] donnent la position et le sens de la marche à une abscisse `s` le long
//! du chemin ; [`follow`] trouve le rail qui continue après une extrémité, une marche plus haut
//! ou plus bas si besoin.
//!
//! # Les liens
//!
//! [`choose_shape`] regarde les quatre côtés : un voisin à la même hauteur, un cran plus haut
//! (montée) ou plus bas, **s'il accepte le lien** (il pointe déjà vers nous, ou une de ses
//! extrémités ne mène nulle part : sans cela, deux voies parallèles se tordraient l'une vers
//! l'autre). Un virage n'existe que sur le rail normal. Avec trois liens ou plus, c'est un
//! aiguillage : tout droit, et un virage vers la branche quand il reçoit du signal.

use glam::{IVec3, Vec3};
use serde::Deserialize;

use crate::grid::{BlockAccess, GridPos};
use crate::machines::{BlockBehavior, MachineContext, received_power};

/// La hauteur du dessus du rail dans son bloc : là où roule le wagon.
pub const RAIL_HEIGHT: f32 = 0.06;

/// Les quatre côtés d'un rail : est, sud, ouest, nord (les indices de [`RailEnd::side`]).
pub const SIDES: [IVec3; 4] = [IVec3::X, IVec3::Z, IVec3::NEG_X, IVec3::NEG_Z];

const EAST: usize = 0;
const SOUTH: usize = 1;
const WEST: usize = 2;
const NORTH: usize = 3;

/// La sorte d'un rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum RailKind {
    /// Le rail ordinaire : seul à faire des virages.
    Normal,
    /// Pousse les wagons quand il est alimenté, les retient sinon.
    Booster,
    /// Émet un signal tant qu'un wagon roule dessus.
    Detector,
    /// Alimenté : fait descendre le passager et coupe la trémie.
    Activator,
    /// Le butoir : arrête les wagons au milieu du bloc (sauf s'il est alimenté).
    Stop,
}

impl RailKind {
    /// Peut-il tourner ? Seul le rail normal fait des virages.
    pub fn curves(self) -> bool {
        self == Self::Normal
    }

    /// Peut-il monter ? Le butoir reste à plat.
    pub fn climbs(self) -> bool {
        self != Self::Stop
    }
}

/// Les dix formes d'un rail, dans l'ordre où elles sont rangées dans `BlockEntity::data[0]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailShape {
    /// Le long de Z.
    NorthSouth = 0,
    /// Le long de X.
    EastWest = 1,
    /// Monte vers +X (l'est est plus haut).
    AscendEast = 2,
    AscendWest = 3,
    AscendNorth = 4,
    AscendSouth = 5,
    /// Les virages, nommés d'après les deux côtés qu'ils relient.
    SouthEast = 6,
    SouthWest = 7,
    NorthWest = 8,
    NorthEast = 9,
}

/// Une extrémité d'un chemin : le côté du bloc (indice dans [`SIDES`]) et sa hauteur (0 : à ras
/// du sol, 1 : un bloc plus haut).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailEnd {
    pub side: usize,
    pub rise: i32,
}

const fn end(side: usize, rise: i32) -> RailEnd {
    RailEnd { side, rise }
}

impl RailShape {
    pub const ALL: [RailShape; 10] = [
        Self::NorthSouth,
        Self::EastWest,
        Self::AscendEast,
        Self::AscendWest,
        Self::AscendNorth,
        Self::AscendSouth,
        Self::SouthEast,
        Self::SouthWest,
        Self::NorthWest,
        Self::NorthEast,
    ];

    pub fn index(self) -> u8 {
        self as u8
    }

    /// La forme d'un indice rangé dans les données (une valeur inconnue donne la droite nord-sud).
    pub fn from_index(index: u8) -> Self {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(Self::NorthSouth)
    }

    pub fn is_curve(self) -> bool {
        self.index() >= 6
    }

    pub fn is_ascent(self) -> bool {
        (2..6).contains(&self.index())
    }

    /// Les deux extrémités : le chemin va de la première (abscisse 0) à la seconde. Une montée va
    /// de son pied à son sommet.
    pub fn ends(self) -> [RailEnd; 2] {
        match self {
            Self::NorthSouth => [end(NORTH, 0), end(SOUTH, 0)],
            Self::EastWest => [end(WEST, 0), end(EAST, 0)],
            Self::AscendEast => [end(WEST, 0), end(EAST, 1)],
            Self::AscendWest => [end(EAST, 0), end(WEST, 1)],
            Self::AscendNorth => [end(SOUTH, 0), end(NORTH, 1)],
            Self::AscendSouth => [end(NORTH, 0), end(SOUTH, 1)],
            Self::SouthEast => [end(SOUTH, 0), end(EAST, 0)],
            Self::SouthWest => [end(SOUTH, 0), end(WEST, 0)],
            Self::NorthWest => [end(NORTH, 0), end(WEST, 0)],
            Self::NorthEast => [end(NORTH, 0), end(EAST, 0)],
        }
    }

    /// Le point d'une extrémité, dans le repère du bloc.
    fn end_point(end: RailEnd) -> Vec3 {
        Vec3::new(0.5, RAIL_HEIGHT + end.rise as f32, 0.5) + SIDES[end.side].as_vec3() * 0.5
    }

    /// Le centre du quart de cercle d'un virage : le coin entre ses deux côtés.
    fn corner(self) -> Vec3 {
        let [a, b] = self.ends();
        Vec3::new(0.5, RAIL_HEIGHT, 0.5) + (SIDES[a.side] + SIDES[b.side]).as_vec3() * 0.5
    }

    /// Les angles de départ et d'arrivée d'un virage autour de son coin.
    fn arc(self) -> (f32, f32) {
        let [a, b] = self.ends();
        let corner = self.corner();
        let angle = |e: RailEnd| {
            let v = Self::end_point(e) - corner;
            v.z.atan2(v.x)
        };
        let (from, to) = (angle(a), angle(b));
        let mut delta = to - from;
        while delta > std::f32::consts::PI {
            delta -= std::f32::consts::TAU;
        }
        while delta < -std::f32::consts::PI {
            delta += std::f32::consts::TAU;
        }
        (from, from + delta)
    }

    /// La longueur du chemin, en blocs.
    pub fn length(self) -> f32 {
        if self.is_curve() {
            std::f32::consts::FRAC_PI_4
        } else if self.is_ascent() {
            std::f32::consts::SQRT_2
        } else {
            1.0
        }
    }

    /// Le point du chemin à l'abscisse `s` (de 0 à [`length`](Self::length)), dans le repère du
    /// bloc.
    pub fn point(self, s: f32) -> Vec3 {
        let [a, b] = self.ends();
        let t = (s / self.length()).clamp(0.0, 1.0);
        if self.is_curve() {
            let (from, to) = self.arc();
            let angle = from + (to - from) * t;
            self.corner() + Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.5
        } else {
            Self::end_point(a).lerp(Self::end_point(b), t)
        }
    }

    /// Le sens de la marche (vecteur unitaire) à l'abscisse `s`, dans le sens des abscisses
    /// croissantes.
    pub fn tangent(self, s: f32) -> Vec3 {
        let [a, b] = self.ends();
        if self.is_curve() {
            let (from, to) = self.arc();
            let angle = from + (to - from) * (s / self.length()).clamp(0.0, 1.0);
            Vec3::new(-angle.sin(), 0.0, angle.cos()) * (to - from).signum()
        } else {
            (Self::end_point(b) - Self::end_point(a)).normalize()
        }
    }

    /// L'abscisse du point du chemin le plus proche de `local` (repère du bloc).
    pub fn locate(self, local: Vec3) -> f32 {
        let [a, b] = self.ends();
        if self.is_curve() {
            let (from, to) = self.arc();
            let v = local - self.corner();
            let mut angle = v.z.atan2(v.x);
            // L'angle le plus proche de l'arc, modulo un tour.
            let mid = (from + to) * 0.5;
            while angle - mid > std::f32::consts::PI {
                angle -= std::f32::consts::TAU;
            }
            while angle - mid < -std::f32::consts::PI {
                angle += std::f32::consts::TAU;
            }
            (((angle - from) / (to - from)).clamp(0.0, 1.0)) * self.length()
        } else {
            let (pa, pb) = (Self::end_point(a), Self::end_point(b));
            ((local - pa).dot(pb - pa) / (pb - pa).length()).clamp(0.0, self.length())
        }
    }

    /// Le rail droit qui relie `side` et son opposé, montant vers `climb` s'il y en a un.
    fn straight(side: usize, climb: Option<usize>) -> Self {
        match climb {
            Some(EAST) => Self::AscendEast,
            Some(WEST) => Self::AscendWest,
            Some(NORTH) => Self::AscendNorth,
            Some(SOUTH) => Self::AscendSouth,
            _ if side.is_multiple_of(2) => Self::EastWest,
            _ => Self::NorthSouth,
        }
    }

    /// Le virage qui relie deux côtés voisins.
    fn curve(a: usize, b: usize) -> Self {
        let has = |s: usize| a == s || b == s;
        match (has(SOUTH), has(NORTH), has(EAST)) {
            (true, _, true) => Self::SouthEast,
            (true, _, false) => Self::SouthWest,
            (_, true, true) => Self::NorthEast,
            _ => Self::NorthWest,
        }
    }
}

/// Ce que l'on sait d'un rail posé : sa sorte, sa forme et s'il est alimenté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rail {
    pub kind: RailKind,
    pub shape: RailShape,
    pub powered: bool,
}

/// Le rail en `pos` (`None` si ce n'est pas un rail).
pub fn rail_at(grid: &(impl BlockAccess + ?Sized), pos: IVec3) -> Option<Rail> {
    let kind = grid.registry().def(grid.block(pos)).rail?;
    let data = grid.block_entity(pos).map(|e| e.data.as_slice());
    let byte = |i: usize| data.and_then(|d| d.get(i).copied()).unwrap_or(0);
    Some(Rail {
        kind,
        shape: RailShape::from_index(byte(0)),
        powered: byte(1) != 0,
    })
}

/// Le rail qui continue après l'extrémité `end` (0 ou 1) du rail `shape` en `pos` : sa position,
/// le rail, et son extrémité d'entrée. Il peut être un cran plus haut ou plus bas, selon la
/// hauteur de l'extrémité. `None` : la voie s'arrête là.
pub fn follow(
    pos: IVec3,
    shape: RailShape,
    end: usize,
    rails: &dyn Fn(IVec3) -> Option<Rail>,
) -> Option<(IVec3, Rail, usize)> {
    let out = shape.ends()[end];
    let side = SIDES[out.side];
    [0, 1, -1].into_iter().find_map(|k| {
        let next = pos + side + IVec3::Y * k;
        let rail = rails(next)?;
        rail.shape
            .ends()
            .iter()
            .position(|e| e.side == (out.side + 2) % 4 && e.rise == out.rise - k)
            .map(|entry| (next, rail, entry))
    })
}

/// Le voisin en `next` accepte-t-il un lien depuis `from` ?
fn accepts(next: IVec3, rail: Rail, from: IVec3, rails: &dyn Fn(IVec3) -> Option<Rail>) -> bool {
    let ends = rail.shape.ends();
    let toward = |e: &RailEnd| {
        let target = next + SIDES[e.side];
        target.x == from.x && target.z == from.z && (target.y - from.y).abs() <= 1
    };
    // Il pointe déjà vers nous.
    if ends.iter().any(toward) {
        return true;
    }
    // Une de ses extrémités ne mène nulle part : il peut s'y tourner.
    ends.iter().any(|e| {
        let target = next + SIDES[e.side];
        [0, 1, -1]
            .into_iter()
            .all(|k| rails(target + IVec3::Y * k).is_none())
    })
}

/// Pour chacun des quatre côtés : `Some(k)` si un rail voisin (à la hauteur `k` de -1 à 1 par
/// rapport à `pos`) accepte le lien.
pub fn links(pos: IVec3, rails: &dyn Fn(IVec3) -> Option<Rail>) -> [Option<i32>; 4] {
    std::array::from_fn(|i| {
        [0, 1, -1].into_iter().find(|&k| {
            let next = pos + SIDES[i] + IVec3::Y * k;
            rails(next).is_some_and(|rail| accepts(next, rail, pos, rails))
        })
    })
}

/// La forme d'un rail de sorte `kind` selon ses liens, son alimentation, et sa forme précédente
/// (gardée quand il n'a aucun lien : un rail seul ne tourne pas sur lui-même).
pub fn choose_shape(
    kind: RailKind,
    links: [Option<i32>; 4],
    powered: bool,
    previous: RailShape,
) -> RailShape {
    let connected: Vec<usize> = (0..4).filter(|&i| links[i].is_some()).collect();
    // Vers quel côté monte-t-on (s'il le faut) ?
    let climb = |sides: &[usize]| {
        kind.climbs()
            .then(|| sides.iter().copied().find(|&i| links[i] == Some(1)))
            .flatten()
    };
    let through = |a: usize| {
        let pair = [a, (a + 2) % 4];
        RailShape::straight(a, climb(&pair))
    };
    match connected.as_slice() {
        [] => {
            let axis = previous.ends()[0].side;
            if previous.is_curve() || previous.is_ascent() {
                RailShape::straight(0, None)
            } else {
                RailShape::straight(axis, None)
            }
        }
        [one] => through(*one),
        [a, b] if (a + 2) % 4 == *b => through(*a),
        [a, b] => {
            if kind.curves() {
                RailShape::curve(*a, *b)
            } else {
                through(*a)
            }
        }
        many => {
            // Un aiguillage : l'axe qui a ses deux côtés (est-ouest de préférence), et une branche.
            let axis = if many.contains(&EAST) && many.contains(&WEST) {
                EAST
            } else {
                SOUTH
            };
            let branch = [SOUTH, NORTH, EAST, WEST]
                .into_iter()
                .find(|s| many.contains(s) && *s % 2 != axis % 2);
            match branch {
                Some(branch) if powered && kind.curves() => RailShape::curve(branch, axis),
                _ => through(axis),
            }
        }
    }
}

// ----- Le comportement -----

/// Le comportement de tous les rails : garder à jour la forme et l'alimentation.
pub struct RailBehavior;

/// Les cases autour de `at` où un rail peut être relié : quatre côtés, à trois hauteurs.
fn ring(at: IVec3) -> impl Iterator<Item = IVec3> {
    SIDES
        .into_iter()
        .flat_map(move |s| (-1..=1).map(move |k| at + s + IVec3::Y * k))
}

impl RailBehavior {
    /// Recalcule la forme et l'alimentation du rail en `at`, et refait son maillage si elles ont changé.
    fn refresh(ctx: &mut MachineContext, at: GridPos) {
        let Some(grid) = ctx.grids.get(at.grid) else {
            return;
        };
        let (shape, powered, changed) = {
            let g = grid.borrow();
            let Some(rail) = rail_at(&*g, at.pos) else {
                return;
            };
            let lookup = |p: IVec3| rail_at(&*g, p);
            let powered = received_power(&*g, at.pos) > 0;
            let shape = choose_shape(rail.kind, links(at.pos, &lookup), powered, rail.shape);
            let stored = g.block_entity(at.pos).is_some_and(|e| e.data.len() >= 2);
            (
                shape,
                powered,
                !stored || rail.shape != shape || rail.powered != powered,
            )
        };
        if !changed {
            return;
        }
        ctx.ensure_block_entity(at);
        ctx.with_block_entity(at, |e| {
            // Garder la puissance émise (le détecteur) : on ne touche qu'aux deux premiers octets.
            if e.data.len() < 2 {
                e.data.resize(2, 0);
            }
            e.data[0] = shape.index();
            e.data[1] = powered as u8;
        });
        grid.borrow_mut().invalidate_visuals(at.pos);
    }
}

impl BlockBehavior for RailBehavior {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            Self::refresh(ctx, at);
            // Les voisins reliables se tournent vers le nouveau rail (ou s'en détournent).
            for next in ring(at.pos) {
                if ctx.block_is_rail(GridPos::new(at.grid, next)) {
                    Self::refresh(ctx, GridPos::new(at.grid, next));
                }
            }
        }
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            Self::refresh(ctx, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    type Map = HashMap<IVec3, Rail>;

    fn lay(cells: &[(IVec3, RailKind)]) -> Map {
        // Les rails sont posés un par un : chaque pose recalcule le nouveau rail et ses voisins.
        let mut map = Map::new();
        for &(p, kind) in cells {
            map.insert(
                p,
                Rail {
                    kind,
                    shape: RailShape::NorthSouth,
                    powered: false,
                },
            );
            let mut todo = vec![p];
            todo.extend(ring(p).filter(|q| map.contains_key(q)));
            for q in todo {
                let snapshot = map.clone();
                let lookup = |x: IVec3| snapshot.get(&x).copied();
                let rail = map[&q];
                let shape = choose_shape(rail.kind, links(q, &lookup), false, rail.shape);
                map.get_mut(&q).unwrap().shape = shape;
            }
        }
        map
    }

    fn line(from: IVec3, step: IVec3, n: i32) -> Vec<(IVec3, RailKind)> {
        (0..n)
            .map(|i| (from + step * i, RailKind::Normal))
            .collect()
    }

    #[test]
    fn a_line_of_rails_runs_along_its_axis() {
        let map = lay(&line(IVec3::ZERO, IVec3::X, 4));
        assert!(map.values().all(|r| r.shape == RailShape::EastWest));
        let map = lay(&line(IVec3::ZERO, IVec3::Z, 4));
        assert!(map.values().all(|r| r.shape == RailShape::NorthSouth));
    }

    #[test]
    fn a_lone_rail_keeps_its_shape_and_a_neighbour_turns_it() {
        assert_eq!(
            choose_shape(RailKind::Normal, [None; 4], false, RailShape::EastWest),
            RailShape::EastWest
        );
        // Un virage reste droit quand il n'a plus de lien.
        assert_eq!(
            choose_shape(RailKind::Normal, [None; 4], false, RailShape::SouthEast),
            RailShape::EastWest
        );
        let map = lay(&[
            (IVec3::ZERO, RailKind::Normal),
            (IVec3::X, RailKind::Normal),
        ]);
        assert_eq!(map[&IVec3::ZERO].shape, RailShape::EastWest);
    }

    #[test]
    fn two_perpendicular_neighbours_make_a_curve_only_on_a_normal_rail() {
        let cells = |kind| {
            [
                (IVec3::ZERO, kind),
                (IVec3::X, RailKind::Normal),
                (IVec3::Z, RailKind::Normal),
            ]
        };
        assert_eq!(
            lay(&cells(RailKind::Normal))[&IVec3::ZERO].shape,
            RailShape::SouthEast
        );
        assert_eq!(
            lay(&cells(RailKind::Booster))[&IVec3::ZERO].shape,
            RailShape::EastWest
        );
        for (a, b, expected) in [
            (SOUTH, WEST, RailShape::SouthWest),
            (NORTH, WEST, RailShape::NorthWest),
            (NORTH, EAST, RailShape::NorthEast),
        ] {
            let mut links = [None; 4];
            links[a] = Some(0);
            links[b] = Some(0);
            assert_eq!(
                choose_shape(RailKind::Normal, links, false, RailShape::NorthSouth),
                expected
            );
        }
    }

    #[test]
    fn a_rail_climbs_toward_a_higher_neighbour() {
        let map = lay(&[
            (IVec3::ZERO, RailKind::Normal),
            (IVec3::new(1, 1, 0), RailKind::Normal),
            (IVec3::new(-1, 0, 0), RailKind::Normal),
        ]);
        assert_eq!(map[&IVec3::ZERO].shape, RailShape::AscendEast);
        // Le rail du haut regarde le bas et reste droit, dans le même axe.
        assert_eq!(map[&IVec3::new(1, 1, 0)].shape, RailShape::EastWest);
        // Un butoir ne monte pas.
        let map = lay(&[
            (IVec3::ZERO, RailKind::Stop),
            (IVec3::new(1, 1, 0), RailKind::Normal),
        ]);
        assert_eq!(map[&IVec3::ZERO].shape, RailShape::EastWest);
    }

    #[test]
    fn a_junction_goes_straight_and_turns_when_powered() {
        let mut links = [None; 4];
        links[EAST] = Some(0);
        links[WEST] = Some(0);
        links[SOUTH] = Some(0);
        assert_eq!(
            choose_shape(RailKind::Normal, links, false, RailShape::EastWest),
            RailShape::EastWest
        );
        let powered = choose_shape(RailKind::Normal, links, true, RailShape::EastWest);
        assert!(powered.is_curve());
        assert!(powered.ends().iter().any(|e| e.side == SOUTH));
        // Les autres rails ne bifurquent pas.
        assert_eq!(
            choose_shape(RailKind::Booster, links, true, RailShape::EastWest),
            RailShape::EastWest
        );
    }

    #[test]
    fn parallel_tracks_do_not_bend_into_each_other() {
        let mut cells = line(IVec3::ZERO, IVec3::X, 5);
        cells.extend(line(IVec3::new(1, 0, 1), IVec3::X, 3));
        let map = lay(&cells);
        assert!(
            map.values().all(|r| r.shape == RailShape::EastWest),
            "{map:?}"
        );
    }

    #[test]
    fn paths_join_up_and_have_the_announced_length() {
        for shape in RailShape::ALL {
            let [a, b] = shape.ends();
            let start = shape.point(0.0);
            let finish = shape.point(shape.length());
            let expected = |e: RailEnd| {
                Vec3::new(0.5, RAIL_HEIGHT + e.rise as f32, 0.5) + SIDES[e.side].as_vec3() * 0.5
            };
            assert!((start - expected(a)).length() < 1e-4, "{shape:?} départ");
            assert!((finish - expected(b)).length() < 1e-4, "{shape:?} arrivée");
            // La longueur mesurée par petits pas.
            let n = 400;
            let measured: f32 = (0..n)
                .map(|i| {
                    let t = |k: i32| shape.length() * k as f32 / n as f32;
                    (shape.point(t(i + 1)) - shape.point(t(i))).length()
                })
                .sum();
            assert!(
                (measured - shape.length()).abs() < 1e-3,
                "{shape:?} longueur"
            );
            // La tangente est unitaire et suit le chemin.
            for k in 0..=4 {
                let s = shape.length() * k as f32 / 4.0;
                let t = shape.tangent(s);
                assert!((t.length() - 1.0).abs() < 1e-4);
                let ahead =
                    shape.point((s + 0.01).min(shape.length())) - shape.point((s - 0.01).max(0.0));
                assert!(t.dot(ahead) > 0.0, "{shape:?} sens de la tangente en {s}");
                // `locate` retrouve l'abscisse.
                assert!(
                    (shape.locate(shape.point(s)) - s).abs() < 1e-3,
                    "{shape:?} locate"
                );
            }
        }
    }

    #[test]
    fn exits_lead_to_the_next_rail_even_a_step_up_or_down() {
        let kind = RailKind::Normal;
        let rails: Map = [
            (IVec3::ZERO, RailShape::AscendEast),
            (IVec3::new(1, 1, 0), RailShape::EastWest),
            (IVec3::new(2, 1, 0), RailShape::EastWest),
        ]
        .into_iter()
        .map(|(p, shape)| {
            (
                p,
                Rail {
                    kind,
                    shape,
                    powered: false,
                },
            )
        })
        .collect();
        let lookup = |p: IVec3| rails.get(&p).copied();
        let (next, _, entry) = follow(IVec3::ZERO, RailShape::AscendEast, 1, &lookup).unwrap();
        assert_eq!((next, entry), (IVec3::new(1, 1, 0), 0));
        // Dans l'autre sens : du haut vers le pied de la montée.
        let (back, _, entry) =
            follow(IVec3::new(1, 1, 0), RailShape::EastWest, 0, &lookup).unwrap();
        assert_eq!((back, entry), (IVec3::ZERO, 1));
        // Au bout de la voie : rien.
        assert!(follow(IVec3::new(2, 1, 0), RailShape::EastWest, 1, &lookup).is_none());
        // Un rail à plat n'est pas relié à un rail deux crans plus haut.
        let far = |p: IVec3| (p == IVec3::new(1, 2, 0)).then_some(rails[&IVec3::new(1, 1, 0)]);
        assert!(follow(IVec3::ZERO, RailShape::EastWest, 1, &far).is_none());
    }

    // ----- Dans le monde : le comportement écrit la forme dans l'état du bloc -----

    mod world {
        use std::cell::RefCell;

        use super::super::*;
        use crate::machines::Machines;
        use crate::machines::tests::loaded_streamer;
        use crate::streaming::ChunkStreamer;

        const ORIGIN: IVec3 = IVec3::new(4, 100, 4);

        struct Rig {
            s: RefCell<ChunkStreamer>,
            machines: Machines,
        }

        impl Rig {
            fn new() -> Self {
                let s = loaded_streamer();
                let machines = Machines::with_natives(s.borrow().world().registry());
                let rig = Self { s, machines };
                let stone = rig.id("core:stone");
                for x in -2..20 {
                    for z in -2..8 {
                        for y in 0..10 {
                            let block = if y < 2 { stone } else { crate::BlockId::AIR };
                            rig.s
                                .borrow_mut()
                                .set_block(ORIGIN + IVec3::new(x, y - 2, z), block);
                        }
                    }
                }
                rig
            }

            fn id(&self, key: &str) -> crate::BlockId {
                self.s.borrow().world().registry().id(key).unwrap()
            }

            fn at(x: i32, y: i32, z: i32) -> IVec3 {
                ORIGIN + IVec3::new(x, y, z)
            }

            fn place(&mut self, pos: IVec3, key: &str) {
                let block = self.id(key);
                assert!(self.s.borrow_mut().set_block(pos, block));
                self.machines.block_changed(&self.s, pos, &mut Vec::new());
                self.tick(1);
            }

            fn remove(&mut self, pos: IVec3) {
                self.s.borrow_mut().set_block(pos, crate::BlockId::AIR);
                self.machines.block_changed(&self.s, pos, &mut Vec::new());
                self.tick(1);
            }

            fn tick(&mut self, n: usize) {
                for _ in 0..n {
                    self.machines.run_tick(&self.s, &mut Vec::new());
                }
            }

            fn rail(&self, pos: IVec3) -> Rail {
                rail_at(&*self.s.borrow(), pos).expect("un rail")
            }
        }

        #[test]
        fn placing_rails_stores_their_shape_and_a_corner_turns_the_neighbour() {
            let mut rig = Rig::new();
            for x in 0..4 {
                rig.place(Rig::at(x, 0, 0), "core:rail");
            }
            assert!((0..4).all(|x| rig.rail(Rig::at(x, 0, 0)).shape == RailShape::EastWest));
            // Un rail au sud du dernier : le dernier devient un virage.
            rig.place(Rig::at(3, 0, 1), "core:rail");
            assert_eq!(rig.rail(Rig::at(3, 0, 0)).shape, RailShape::SouthWest);
            assert_eq!(rig.rail(Rig::at(3, 0, 1)).shape, RailShape::NorthSouth);
            // On le retire : la voie redevient droite.
            rig.remove(Rig::at(3, 0, 1));
            assert_eq!(rig.rail(Rig::at(3, 0, 0)).shape, RailShape::EastWest);
        }

        #[test]
        fn a_diagonal_rail_notices_its_neighbour_appearing_and_disappearing() {
            let mut rig = Rig::new();
            rig.place(Rig::at(0, 0, 0), "core:rail");
            rig.place(Rig::at(-1, 0, 0), "core:rail");
            // Un rail un cran plus haut, à l'est : le premier monte, sans être un voisin direct.
            rig.place(Rig::at(1, 1, 0), "core:rail");
            rig.place(Rig::at(1, 0, 0), "core:stone");
            assert_eq!(rig.rail(Rig::at(0, 0, 0)).shape, RailShape::AscendEast);
            rig.remove(Rig::at(1, 1, 0));
            assert_eq!(rig.rail(Rig::at(0, 0, 0)).shape, RailShape::EastWest);
        }

        #[test]
        fn a_booster_reads_its_power_and_a_junction_switches() {
            let mut rig = Rig::new();
            for x in 0..4 {
                rig.place(Rig::at(x, 0, 0), "core:booster_rail");
            }
            assert!(!rig.rail(Rig::at(1, 0, 0)).powered);
            rig.place(Rig::at(1, 0, 1), "core:lever");
            assert!(
                rig.machines
                    .use_block(&rig.s, Rig::at(1, 0, 1), &mut Vec::new())
            );
            rig.tick(3);
            assert!(rig.rail(Rig::at(1, 0, 0)).powered);
            assert!(!rig.rail(Rig::at(3, 0, 0)).powered);
            // Une voie en T : droite sans signal, virage avec.
            for x in 0..4 {
                rig.place(Rig::at(x, 0, 4), "core:rail");
            }
            rig.place(Rig::at(2, 0, 5), "core:rail");
            assert_eq!(rig.rail(Rig::at(2, 0, 4)).shape, RailShape::EastWest);
            rig.place(Rig::at(2, 0, 3), "core:lever");
            assert!(
                rig.machines
                    .use_block(&rig.s, Rig::at(2, 0, 3), &mut Vec::new())
            );
            rig.tick(3);
            assert!(rig.rail(Rig::at(2, 0, 4)).shape.is_curve());
        }

        #[test]
        fn a_detector_emits_power_to_its_neighbours() {
            let mut rig = Rig::new();
            rig.place(Rig::at(0, 0, 0), "core:detector_rail");
            rig.place(Rig::at(1, 0, 0), "core:booster_rail");
            let at = crate::GridPos::world(Rig::at(0, 0, 0));
            assert!(rig.machines.set_power(crate::Grids::new(&rig.s), at, 15));
            rig.tick(2);
            assert!(
                rig.rail(Rig::at(1, 0, 0)).powered,
                "le détecteur alimente son voisin"
            );
            assert!(rig.machines.set_power(crate::Grids::new(&rig.s), at, 0));
            rig.tick(2);
            assert!(!rig.rail(Rig::at(1, 0, 0)).powered);
        }
    }
}
