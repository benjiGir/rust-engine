//! Poser une pièce : de la sorte d'objet tenue, du point visé, de la face visée et du regard, à la
//! pièce à ajouter à la case.
//!
//! Deux cas, comme au clic droit :
//!
//! - [`plan_inner`] : la face visée est **à l'intérieur** de la case (le dessus d'une dalle) : la
//!   pièce va dans cette même case, au contact de la surface visée ;
//! - [`plan_outer`] : la face visée est au bord de la case : la pièce va dans la **case voisine**,
//!   collée à la face visée.
//!
//! Les pièces qui ne dépendent pas d'un plan (marche, poutre, tranches, escaliers, pentes) lisent le
//! point visé comme une position dans la case : la moitié ou le quart où il tombe décide du coin, de
//! l'arête ou de la moitié choisis.

use glam::{IVec3, Vec3};

use super::grid::face_of;
use super::kind::PieceKind;
use super::piece::{Piece, micro_mask};
use super::slope::{self, Corner};
use super::{Part, Shaped};
use crate::tech::capabilities::Face;

/// Où et comment poser une pièce quand on vise un point. `point` est le point visé, en blocs, relatif
/// à la case visée (0 à 1) ; `normal` la normale de la face visée (un axe) ; `look` le regard du
/// joueur.
///
/// Renvoie la pièce à ajouter **dans la case visée** quand la face visée est à l'intérieur de la case
/// (le dessus d'une dalle, par exemple), et `None` quand elle est au bord de la case : la pièce va
/// alors dans la case voisine ([`plan_outer`]). Une forme raccordée n'a jamais de place dans une case
/// déjà occupée : `None` aussi.
pub fn plan_inner(
    kind: PieceKind,
    existing: &Shaped,
    point: Vec3,
    normal: IVec3,
    look: Vec3,
) -> Option<Piece> {
    if kind.is_connected() {
        return None;
    }
    let axis = axis_of(normal)?;
    let c = point[axis];
    let at_edge = if normal[axis] > 0 {
        c >= 0.999
    } else {
        c <= 0.001
    };
    if at_edge {
        return None;
    }
    // La position du plan visé, en seizièmes.
    let plane = (c * 16.0).round().clamp(0.0, 16.0) as u8;
    let positive = normal[axis] > 0;
    // Le point, poussé d'un peu dans la case de la pièce (du côté de la normale).
    let beyond = (point + normal.as_vec3() * 0.125).clamp(Vec3::ZERO, Vec3::ONE);
    match kind {
        PieceKind::Layer | PieceKind::Slab => {
            let thickness = if kind == PieceKind::Slab { 8 } else { 1 };
            let (face, offset) = if positive {
                (face_of(axis, false), plane)
            } else {
                (face_of(axis, true), 16 - plane)
            };
            Some(Piece::Layer {
                face,
                offset,
                thickness,
            })
        }
        PieceKind::SlopeLayer
        | PieceKind::Slope
        | PieceKind::CornerSlopeLayer
        | PieceKind::InnerSlopeLayer => {
            let corner = slope_corner(kind);
            let level = if kind == PieceKind::Slope { 16 } else { 1 };
            // Sur une pente de la case : la même orientation, pour qu'elle monte de niveau.
            let aimed = existing.part_hit(point, normal.as_vec3());
            if let Some(Part {
                piece:
                    Piece::Slope {
                        base,
                        up,
                        corner: other,
                        ..
                    },
                ..
            }) = aimed.map(|i| existing.parts[i])
                && other == corner
            {
                return Some(Piece::Slope {
                    base,
                    up,
                    level,
                    corner,
                });
            }
            Some(slope_piece(corner, level, point, normal, look, false))
        }
        PieceKind::Micro | PieceKind::Nano => {
            let size = if kind == PieceKind::Micro { 2 } else { 1 };
            Some(micro_at(beyond, size))
        }
        PieceKind::HalfLayer | PieceKind::HalfPanel | PieceKind::QuarterPanel => {
            let thickness = if kind == PieceKind::HalfLayer { 1 } else { 2 };
            let span = if positive {
                (plane + thickness <= 16).then_some((plane, plane + thickness))?
            } else {
                (plane >= thickness).then_some((plane - thickness, plane))?
            };
            Some(panel(kind, axis, span, beyond))
        }
        PieceKind::Board => {
            // Collée au mur de la case que regarde la surface visée.
            let span = if positive { (15, 16) } else { (0, 1) };
            Some(panel(kind, axis, span, beyond))
        }
        _ => Some(boxed(kind, axis, beyond, normal, look)),
    }
}

/// La pièce à poser dans la **case voisine** de la case visée (derrière la face de normale
/// `normal`). `point` est le point visé, relatif à la case visée.
pub fn plan_outer(kind: PieceKind, point: Vec3, normal: IVec3, look: Vec3) -> Piece {
    let axis = axis_of(normal).unwrap_or(1);
    let positive = normal[axis] > 0;
    // Le point, dans la case voisine : de l'autre côté du plan, serré contre lui.
    let mut local = point - normal.as_vec3();
    local[axis] = if positive { 0.001 } else { 0.999 };
    let anchor = face_of(axis, normal[axis] < 0);
    let wall_span = |t: u8| if positive { (0, t) } else { (16 - t, 16) };
    match kind {
        PieceKind::Layer | PieceKind::Slab => Piece::Layer {
            face: anchor,
            offset: 0,
            thickness: if kind == PieceKind::Slab { 8 } else { 1 },
        },
        PieceKind::SlopeLayer
        | PieceKind::Slope
        | PieceKind::CornerSlopeLayer
        | PieceKind::InnerSlopeLayer => slope_piece(
            slope_corner(kind),
            if kind == PieceKind::Slope { 16 } else { 1 },
            point,
            normal,
            look,
            true,
        ),
        PieceKind::Micro => micro_at(local, 2),
        PieceKind::Nano => micro_at(local, 1),
        PieceKind::HalfLayer | PieceKind::Board => panel(kind, axis, wall_span(1), local),
        PieceKind::HalfPanel | PieceKind::QuarterPanel => panel(kind, axis, wall_span(2), local),
        kind if kind.is_connected() => Piece::Connected {
            kind: match kind {
                PieceKind::Wall => super::Link::Wall,
                PieceKind::Fence => super::Link::Fence,
                PieceKind::Gate => super::Link::Gate,
                PieceKind::Pane => super::Link::Pane,
                _ => super::Link::FlatPane,
            },
            open: false,
        },
        _ => boxed(kind, axis, local, normal, look),
    }
}

fn slope_corner(kind: PieceKind) -> Corner {
    match kind {
        PieceKind::CornerSlopeLayer => Corner::Outer,
        PieceKind::InnerSlopeLayer => Corner::Inner,
        _ => Corner::None,
    }
}

/// Une pente de `level` niveaux. Au sol quand on vise un dessus, au plafond quand on vise un dessous ;
/// sur un mur, la pente se colle à lui si on vise le milieu de la face (`outer`), sinon elle se
/// pose au sol (bas de la face) ou au plafond (haut de la face), comme avant.
///
/// Le haut d'une pente droite est le regard (la face inclinée regarde le joueur) ; sur un mur, le
/// bord le plus proche du point visé. Un coin se place sur le quart de la face visé.
fn slope_piece(
    corner: Corner,
    level: u8,
    point: Vec3,
    normal: IVec3,
    look: Vec3,
    outer: bool,
) -> Piece {
    let axis = axis_of(normal).unwrap_or(1);
    let edge = |a: usize| face_of(a, point[a] >= 0.5);
    let wall = outer && normal.y == 0 && (0.3..=0.7).contains(&point.y);
    let (base, up) = if wall {
        // Collée au mur : sa base est la face de la case contre ce mur.
        let base = face_of(axis, normal[axis] < 0);
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let up = match corner {
            Corner::None => {
                if (point[u] - 0.5).abs() >= (point[v] - 0.5).abs() {
                    edge(u)
                } else {
                    edge(v)
                }
            }
            _ => slope::corner_up(base, edge(u), edge(v)).unwrap_or(edge(u)),
        };
        (base, up)
    } else {
        let ceiling = normal.y < 0 || (normal.y == 0 && point.y >= 0.5);
        let base = if ceiling { Face::PosY } else { Face::NegY };
        let up = match corner {
            Corner::None => slope::legacy_up(facing_of(look)),
            _ => slope::corner_up(base, edge(0), edge(2)).unwrap_or(Face::NegZ),
        };
        (base, up)
    };
    Piece::Slope {
        base,
        up,
        level,
        corner,
    }
}

/// Un panneau mince (demi-couche, demi-panneau, panneau d'un quart, planchette) : `span` est sa
/// place le long de l'axe `axis` (en seizièmes) ; dans le plan, une moitié (demi-couche,
/// demi-panneau : celle vers où tombe `p`) ou un quart (panneau d'un quart).
fn panel(kind: PieceKind, axis: usize, span: (u8, u8), p: Vec3) -> Piece {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let half = |c: f32| if c < 0.5 { (0, 8) } else { (8, 16) };
    let mut min = [0u8; 3];
    let mut max = [16u8; 3];
    min[axis] = span.0;
    max[axis] = span.1;
    match kind {
        PieceKind::HalfLayer | PieceKind::HalfPanel => {
            // On coupe en deux selon l'axe où le point est le plus loin du centre.
            let cut = if (p[u] - 0.5).abs() >= (p[v] - 0.5).abs() {
                u
            } else {
                v
            };
            (min[cut], max[cut]) = half(p[cut]);
        }
        PieceKind::QuarterPanel => {
            (min[u], max[u]) = half(p[u]);
            (min[v], max[v]) = half(p[v]);
        }
        _ => {}
    }
    Piece::cuboid(kind, min, max, 1)
}

/// Les pièces qui ne dépendent d'aucun plan : escaliers, marche, poutre, tranches. `p` : le point, dans
/// la case de la pièce.
fn boxed(kind: PieceKind, axis: usize, p: Vec3, normal: IVec3, look: Vec3) -> Piece {
    // L'axe de la colonne : debout sur un dessus ou un dessous, couchée le long du mur sinon.
    let column = match axis {
        1 => 1,
        0 => 2,
        _ => 0,
    };
    let (a, b) = ((column + 1) % 3, (column + 2) % 3);
    let high = |c: f32| c >= 0.5;
    let half = |c: f32| if high(c) { (8, 16) } else { (0, 8) };
    let corner = |c: f32| if high(c) { (15, 16) } else { (0, 1) };
    let mut min = [0u8; 3];
    let mut max = [16u8; 3];
    match kind {
        PieceKind::Stairs | PieceKind::VerticalStairs => {
            let vertical = kind == PieceKind::VerticalStairs;
            let ceiling = !vertical && (normal.y < 0 || (normal.y == 0 && p.y >= 0.5));
            let facing = if vertical {
                match (high(p.x), high(p.z)) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (true, true) => 2,
                    (false, true) => 3,
                }
            } else {
                facing_of(look)
            };
            return Piece::Stairs {
                facing,
                ceiling,
                vertical,
            };
        }
        PieceKind::Step => {
            (min[a], max[a]) = half(p[a]);
            (min[b], max[b]) = half(p[b]);
        }
        PieceKind::Beam => {
            (min[a], max[a]) = (4, 12);
            (min[b], max[b]) = (4, 12);
        }
        PieceKind::Slice => {
            (min[a], max[a]) = corner(p[a]);
            (min[b], max[b]) = corner(p[b]);
        }
        PieceKind::CornerSlice => {
            for axis in 0..3 {
                (min[axis], max[axis]) = corner(p[axis]);
            }
        }
        _ => {}
    }
    Piece::cuboid(kind, min, max, 1)
}

fn axis_of(normal: IVec3) -> Option<usize> {
    (0..3).find(|&a| normal[a] != 0)
}

/// L'orientation du côté haut d'une pente : le regard (la face inclinée regarde le joueur).
fn facing_of(look: Vec3) -> u8 {
    let (dx, dz) = (look.x, look.z);
    if dx.abs() > dz.abs() {
        if dx > 0.0 { 1 } else { 3 }
    } else if dz > 0.0 {
        2
    } else {
        0
    }
}

/// Le cube de `size` cellules (aligné sur la grille) qui contient `p` (en blocs, 0 à 1).
fn micro_at(p: Vec3, size: usize) -> Piece {
    let cell = |v: f32| (((v * 4.0).floor().max(0.0) as usize).min(3) / size) * size;
    Piece::Micro {
        cells: micro_mask(cell(p.x), cell(p.y), cell(p.z), size),
    }
}

/// Le côté haut (0 à 3) d'une orientation, en vecteur.
pub fn facing_direction(facing: u8) -> IVec3 {
    super::slope::direction_of(slope::legacy_up(facing))
}
