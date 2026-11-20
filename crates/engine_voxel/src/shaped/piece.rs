//! Les pièces d'une case : ce qu'on y pose, et comment chacune s'occupe, se découpe en boîtes, se
//! casse, tourne et fusionne avec sa voisine.
//!
//! - [`Piece::Layer`] : une tranche de la case, collée à une face, épaisse de 1 à 16 seizièmes (8 : une
//!   dalle, 16 : un bloc plein). Elle porte un `offset` (la distance à la face) : une dalle de chêne
//!   peut ainsi se poser *sur* une dalle de pierre, au lieu de toujours partir d'une face.
//! - [`Piece::Slope`] : un plan incliné (voir `slope.rs`) : une base, un haut, un niveau de 1 à 16 et un
//!   coin éventuel.
//! - [`Piece::Micro`] : des microblocs sur une grille 4 × 4 × 4 (un bit par cellule de 1/64 de bloc).
//!   Un microbloc de 1/8 occupe 2 × 2 × 2 cellules alignées.
//! - [`Piece::Cuboid`] : un pavé quelconque au seizième près, avec la sorte d'objet dont il vient et
//!   combien d'objets il vaut. Les préréglages (marche, poutre, tranches, demi-couches, panneaux,
//!   planchettes) fabriquent des pavés : le maillage et la collision n'y voient que des boîtes.
//! - [`Piece::Stairs`] : un escalier (une dalle et une demi-dalle) ou sa version verticale (un coin
//!   de trois quarts de colonne).
//! - [`Piece::Connected`] : une forme qui se raccorde à ses voisins (voir `connected.rs`) ; seule dans
//!   sa case.

use super::connected::{self, Link, Neighbours};
use super::grid::{self, IntBox, N, Occupancy, boxes_occupancy};
use super::kind::{PieceKind, corner_slice_cost, slice_cost};
use super::slope::{self, Corner};
use crate::capabilities::Face;

/// Une pièce de la case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// Une tranche collée à `face`, qui commence à `offset` seizièmes de cette face et épaisse de
    /// `thickness` seizièmes (`offset + thickness <= 16`).
    Layer {
        face: Face,
        offset: u8,
        thickness: u8,
    },
    /// Une pente de `level` seizièmes (1 à 16), posée sur la face `base` et qui monte vers la face
    /// `up` (perpendiculaire). En coin, le second bord haut est celui qu'on trouve en tournant `up`
    /// d'un quart de tour autour de `base` (voir `slope.rs`).
    Slope {
        base: Face,
        up: Face,
        level: u8,
        corner: Corner,
    },
    /// Des cellules de 1/64 de bloc (grille 4 × 4 × 4) : le bit `x + 4 * z + 16 * y`.
    Micro { cells: u64 },
    /// Un pavé de (`min`, `max`) seizièmes, tiré de `count` objets de la sorte `kind` (ce qu'il rend
    /// quand on le casse).
    Cuboid {
        min: [u8; 3],
        max: [u8; 3],
        kind: PieceKind,
        count: u8,
    },
    /// Un escalier. `facing` : le côté de la haute marche (0 nord, 1 est, 2 sud, 3 ouest) ; `ceiling` :
    /// retourné au plafond ; `vertical` : un coin de trois quarts de colonne, dont `facing` (0 à 3)
    /// désigne le quart vide.
    Stairs {
        facing: u8,
        ceiling: bool,
        vertical: bool,
    },
    /// Une forme raccordée. `open` : un portillon ouvert.
    Connected { kind: Link, open: bool },
}

/// Les sortes d'objets qu'un pavé peut représenter.
fn is_cuboid_kind(kind: PieceKind) -> bool {
    matches!(
        kind,
        PieceKind::Step
            | PieceKind::Beam
            | PieceKind::Slice
            | PieceKind::CornerSlice
            | PieceKind::HalfLayer
            | PieceKind::HalfPanel
            | PieceKind::QuarterPanel
            | PieceKind::Board
    )
}

impl Piece {
    /// Une pente de la première version : `facing` (0 nord, 1 est, 2 sud, 3 ouest) est le côté haut,
    /// `ceiling` la retourne au plafond.
    pub fn floor_slope(facing: u8, ceiling: bool, level: u8) -> Piece {
        let (base, up) = slope::legacy(facing, ceiling);
        Piece::Slope {
            base,
            up,
            level,
            corner: Corner::None,
        }
    }

    /// Un pavé.
    pub fn cuboid(kind: PieceKind, min: [u8; 3], max: [u8; 3], count: u8) -> Piece {
        Piece::Cuboid {
            min,
            max,
            kind,
            count,
        }
    }

    /// Les valeurs sont-elles dans leurs bornes ?
    pub fn is_valid(&self) -> bool {
        match *self {
            Piece::Layer {
                offset, thickness, ..
            } => thickness >= 1 && usize::from(offset) + usize::from(thickness) <= N,
            Piece::Slope {
                base, up, level, ..
            } => (1..=16).contains(&level) && base as usize / 2 != up as usize / 2,
            Piece::Micro { cells } => cells != 0,
            Piece::Cuboid {
                min,
                max,
                kind,
                count,
            } => {
                is_cuboid_kind(kind)
                    && count >= 1
                    && (0..3).all(|a| min[a] < max[a] && usize::from(max[a]) <= N)
            }
            Piece::Stairs {
                facing,
                ceiling,
                vertical,
            } => facing < 4 && !(vertical && ceiling),
            Piece::Connected { .. } => true,
        }
    }

    /// Les seizièmes que la pièce remplit (une forme raccordée, sans voisins).
    pub fn occupancy(&self) -> Occupancy {
        self.occupancy_with(&Neighbours::NONE)
    }

    /// Les seizièmes que la pièce remplit, une forme raccordée ayant les voisins `n`.
    pub fn occupancy_with(&self, n: &Neighbours) -> Occupancy {
        match *self {
            Piece::Layer { .. } | Piece::Stairs { .. } | Piece::Cuboid { .. } => {
                boxes_occupancy(&self.int_boxes_with(n))
            }
            Piece::Slope {
                base,
                up,
                level,
                corner,
            } => slope::occupancy(base, up, level, corner),
            Piece::Micro { cells } => {
                let mut occ = grid::empty();
                for b in micro_boxes(cells) {
                    grid::fill_box(&mut occ, &b);
                }
                occ
            }
            Piece::Connected { kind, open } => connected::occupancy(kind, open, n),
        }
    }

    /// Les boîtes de la pièce, en seizièmes : (x0, y0, z0, x1, y1, z1).
    #[cfg(test)]
    pub(crate) fn int_boxes(&self) -> Vec<IntBox> {
        self.int_boxes_with(&Neighbours::NONE)
    }

    /// Les boîtes de la pièce, une forme raccordée ayant les voisins `n`.
    pub(crate) fn int_boxes_with(&self, n: &Neighbours) -> Vec<IntBox> {
        match *self {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => {
                let (axis, positive) = grid::face_axis(face);
                let (lo, hi) = if positive {
                    (16 - offset - thickness, 16 - offset)
                } else {
                    (offset, offset + thickness)
                };
                let mut b = [0, 0, 0, 16, 16, 16];
                b[axis] = lo;
                b[axis + 3] = hi;
                vec![b]
            }
            Piece::Slope {
                base,
                up,
                level,
                corner,
            } => slope::boxes(base, up, level, corner),
            Piece::Micro { cells } => micro_boxes(cells),
            Piece::Cuboid { min, max, .. } => {
                vec![[min[0], min[1], min[2], max[0], max[1], max[2]]]
            }
            Piece::Stairs {
                facing,
                ceiling,
                vertical,
            } => stairs_boxes(facing, ceiling, vertical),
            Piece::Connected { kind, open } => kind.visual_boxes(open, n),
        }
    }

    /// Les boîtes de collision : celles de la forme, sauf pour les formes raccordées (une barrière
    /// monte à 24 seizièmes ; un portillon ouvert ne bloque plus).
    pub(crate) fn collision_boxes_with(&self, n: &Neighbours) -> Vec<IntBox> {
        match *self {
            Piece::Connected { kind, open } => kind.collision_boxes(open, n),
            _ => self.int_boxes_with(n),
        }
    }

    /// Les objets que la pièce rend quand on la casse.
    pub fn drops(&self) -> Vec<(PieceKind, u32)> {
        let mut out = Vec::new();
        let mut push = |kind: PieceKind, n: u32| {
            if n > 0 {
                out.push((kind, n));
            }
        };
        match *self {
            Piece::Layer { thickness, .. } => {
                push(PieceKind::Slab, u32::from(thickness / 8));
                push(PieceKind::Layer, u32::from(thickness % 8));
            }
            Piece::Slope { level, corner, .. } => match corner {
                Corner::None if level == 16 => push(PieceKind::Slope, 1),
                Corner::None => push(PieceKind::SlopeLayer, u32::from(level)),
                Corner::Outer => push(PieceKind::CornerSlopeLayer, u32::from(level)),
                Corner::Inner => push(PieceKind::InnerSlopeLayer, u32::from(level)),
            },
            Piece::Micro { cells } => {
                let micros = micro_blocks(cells);
                let rest = cells & !micros.iter().fold(0u64, |m, (_, mask)| m | mask);
                push(PieceKind::Micro, micros.len() as u32);
                push(PieceKind::Nano, rest.count_ones());
            }
            Piece::Cuboid { kind, count, .. } => push(kind, u32::from(count)),
            Piece::Stairs { vertical, .. } => push(
                if vertical {
                    PieceKind::VerticalStairs
                } else {
                    PieceKind::Stairs
                },
                1,
            ),
            Piece::Connected { kind, .. } => push(
                match kind {
                    Link::Wall => PieceKind::Wall,
                    Link::Fence => PieceKind::Fence,
                    Link::Gate => PieceKind::Gate,
                    Link::Pane => PieceKind::Pane,
                    Link::FlatPane => PieceKind::FlatPane,
                },
                1,
            ),
        }
        out
    }

    /// La pièce après `k` quarts de tour autour de la verticale.
    pub(crate) fn turned(self, k: u8) -> Piece {
        let k = k % 4;
        match self {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => Piece::Layer {
                face: grid::turn_face(face, k),
                offset,
                thickness,
            },
            Piece::Slope {
                base,
                up,
                level,
                corner,
            } => Piece::Slope {
                base: grid::turn_face(base, k),
                up: grid::turn_face(up, k),
                level,
                corner,
            },
            Piece::Micro { cells } => {
                let mut out = 0u64;
                for bit in 0..64usize {
                    if cells & (1 << bit) == 0 {
                        continue;
                    }
                    let (mut x, y, mut z) = (bit % 4, bit / 16, (bit / 4) % 4);
                    for _ in 0..k {
                        (x, z) = (3 - z, x);
                    }
                    out |= 1 << (x + 4 * z + 16 * y);
                }
                Piece::Micro { cells: out }
            }
            Piece::Cuboid {
                min,
                max,
                kind,
                count,
            } => {
                let b = grid::turn_box([min[0], min[1], min[2], max[0], max[1], max[2]], k);
                Piece::Cuboid {
                    min: [b[0], b[1], b[2]],
                    max: [b[3], b[4], b[5]],
                    kind,
                    count,
                }
            }
            Piece::Stairs {
                facing,
                ceiling,
                vertical,
            } => Piece::Stairs {
                facing: (facing + k) % 4,
                ceiling,
                vertical,
            },
            connected @ Piece::Connected { .. } => connected,
        }
    }

    /// Cette pièce, une tranche, avec un cran de plus : la pièce agrandie et le nombre d'objets de
    /// plus qu'il faut pour la payer (voir `kind.rs`, `slice_cost`). `None` si ce n'est pas une
    /// tranche ou si elle est déjà à sa taille maximale (8).
    pub fn grown(&self) -> Option<(Piece, u32)> {
        let Piece::Cuboid {
            min,
            max,
            kind,
            count,
        } = *self
        else {
            return None;
        };
        let len = |a: usize| max[a] - min[a];
        let (axes, cost): (Vec<usize>, fn(u32) -> u32) = match kind {
            PieceKind::Slice => {
                let long = (0..3).find(|&a| len(a) == 16)?;
                ((0..3).filter(|&a| a != long).collect(), slice_cost)
            }
            PieceKind::CornerSlice => ((0..3).collect(), corner_slice_cost),
            _ => return None,
        };
        let n = u32::from(len(axes[0]));
        if n >= 8 || axes.iter().any(|&a| u32::from(len(a)) != n) {
            return None;
        }
        let (mut min, mut max) = (min, max);
        for a in axes {
            if min[a] == 0 {
                max[a] += 1;
            } else {
                min[a] -= 1;
            }
        }
        let new_count = u8::try_from(cost(n + 1)).ok()?;
        Some((
            Piece::Cuboid {
                min,
                max,
                kind,
                count: new_count,
            },
            u32::from(new_count).saturating_sub(u32::from(count)),
        ))
    }

    /// La forme que montre l'icône d'une sorte de pièce (et que les essais posent).
    pub fn sample(kind: PieceKind) -> Piece {
        let slope = |level, corner| {
            let up = slope::corner_up(Face::NegY, Face::NegZ, Face::NegX).expect("un coin");
            Piece::Slope {
                base: Face::NegY,
                up: if corner == Corner::None {
                    Face::NegZ
                } else {
                    up
                },
                level,
                corner,
            }
        };
        match kind {
            PieceKind::Layer => Piece::Layer {
                face: Face::NegY,
                offset: 0,
                thickness: 2,
            },
            PieceKind::Slab => Piece::Layer {
                face: Face::NegY,
                offset: 0,
                thickness: 8,
            },
            PieceKind::SlopeLayer => slope(6, Corner::None),
            PieceKind::Slope => slope(16, Corner::None),
            PieceKind::Micro => Piece::Micro {
                cells: micro_mask(1, 0, 1, 2),
            },
            PieceKind::Nano => Piece::Micro {
                cells: micro_mask(1, 0, 2, 1),
            },
            PieceKind::Stairs => Piece::Stairs {
                facing: 0,
                ceiling: false,
                vertical: false,
            },
            PieceKind::VerticalStairs => Piece::Stairs {
                facing: 0,
                ceiling: false,
                vertical: true,
            },
            PieceKind::Step => Piece::cuboid(kind, [0, 0, 0], [8, 16, 8], 1),
            PieceKind::Beam => Piece::cuboid(kind, [4, 0, 4], [12, 16, 12], 1),
            PieceKind::Slice => Piece::cuboid(kind, [0, 0, 0], [2, 16, 2], 4),
            PieceKind::CornerSlice => Piece::cuboid(kind, [0, 0, 0], [4, 4, 4], 8),
            PieceKind::HalfLayer => Piece::cuboid(kind, [0, 0, 0], [16, 2, 8], 2),
            PieceKind::HalfPanel => Piece::cuboid(kind, [0, 0, 0], [16, 8, 2], 1),
            PieceKind::QuarterPanel => Piece::cuboid(kind, [0, 0, 0], [8, 8, 2], 1),
            PieceKind::Board => Piece::cuboid(kind, [0, 0, 0], [16, 16, 1], 1),
            PieceKind::CornerSlopeLayer => slope(12, Corner::Outer),
            PieceKind::InnerSlopeLayer => slope(12, Corner::Inner),
            PieceKind::Wall => Piece::Connected {
                kind: Link::Wall,
                open: false,
            },
            PieceKind::Fence => Piece::Connected {
                kind: Link::Fence,
                open: false,
            },
            PieceKind::Gate => Piece::Connected {
                kind: Link::Gate,
                open: false,
            },
            PieceKind::Pane => Piece::Connected {
                kind: Link::Pane,
                open: false,
            },
            PieceKind::FlatPane => Piece::Connected {
                kind: Link::FlatPane,
                open: false,
            },
        }
    }
}

/// Les boîtes d'un escalier.
fn stairs_boxes(facing: u8, ceiling: bool, vertical: bool) -> Vec<IntBox> {
    let facing = facing % 4;
    if vertical {
        // Le quart `facing` est vide : x fort pour 1 et 2, z fort pour 2 et 3.
        let (xhi, zhi) = (facing == 1 || facing == 2, facing >= 2);
        let (full_x, empty_x) = if xhi {
            ((0, 8), (8, 16))
        } else {
            ((8, 16), (0, 8))
        };
        let other_z = if zhi { (0, 8) } else { (8, 16) };
        return vec![
            [full_x.0, 0, 0, full_x.1, 16, 16],
            [empty_x.0, 0, other_z.0, empty_x.1, 16, other_z.1],
        ];
    }
    let (slab, step) = if ceiling {
        ((8, 16), (0, 8))
    } else {
        ((0, 8), (8, 16))
    };
    let step_box = match facing {
        0 => [0, step.0, 0, 16, step.1, 8],
        1 => [8, step.0, 0, 16, step.1, 16],
        2 => [0, step.0, 8, 16, step.1, 16],
        _ => [0, step.0, 0, 8, step.1, 16],
    };
    vec![[0, slab.0, 0, 16, slab.1, 16], step_box]
}

/// Les boîtes d'un ensemble de cellules de 1/64, fusionnées (en seizièmes).
pub(crate) fn micro_boxes(cells: u64) -> Vec<IntBox> {
    let on = |x: usize, y: usize, z: usize| cells & (1 << (x + 4 * z + 16 * y)) != 0;
    // Rangées selon x, puis fusion selon z, puis selon y.
    let mut rows: Vec<[usize; 5]> = Vec::new(); // y, z, x0, x1 (exclu), unused
    for y in 0..4 {
        for z in 0..4 {
            let mut x = 0;
            while x < 4 {
                if on(x, y, z) {
                    let start = x;
                    while x < 4 && on(x, y, z) {
                        x += 1;
                    }
                    rows.push([y, z, start, x, 0]);
                } else {
                    x += 1;
                }
            }
        }
    }
    // (x0, x1, y, z0, z1)
    let mut slabs: Vec<[usize; 5]> = Vec::new();
    for r in rows {
        let [y, z, x0, x1, _] = r;
        match slabs
            .iter_mut()
            .find(|s| s[0] == x0 && s[1] == x1 && s[2] == y && s[4] == z)
        {
            Some(s) => s[4] = z + 1,
            None => slabs.push([x0, x1, y, z, z + 1]),
        }
    }
    // (x0, x1, z0, z1, y0, y1)
    let mut columns: Vec<[usize; 6]> = Vec::new();
    for s in slabs {
        let [x0, x1, y, z0, z1] = s;
        match columns
            .iter_mut()
            .find(|c| c[0] == x0 && c[1] == x1 && c[2] == z0 && c[3] == z1 && c[5] == y)
        {
            Some(c) => c[5] = y + 1,
            None => columns.push([x0, x1, z0, z1, y, y + 1]),
        }
    }
    columns
        .into_iter()
        .map(|[x0, x1, z0, z1, y0, y1]| [x0, y0, z0, x1, y1, z1].map(|v| (v * 4) as u8))
        .collect()
}

/// Les cubes de 2 × 2 × 2 cellules (1/8 de bloc) entièrement pleins d'un ensemble de cellules :
/// leur origine et leur masque.
fn micro_blocks(cells: u64) -> Vec<((usize, usize, usize), u64)> {
    let mut out = Vec::new();
    for y in (0..4).step_by(2) {
        for z in (0..4).step_by(2) {
            for x in (0..4).step_by(2) {
                let mask = micro_mask(x, y, z, 2);
                if cells & mask == mask {
                    out.push(((x, y, z), mask));
                }
            }
        }
    }
    out
}

/// Le masque d'un cube de `size` cellules de côté dont le coin bas est en (x, y, z).
pub fn micro_mask(x: usize, y: usize, z: usize, size: usize) -> u64 {
    let mut mask = 0u64;
    for cy in y..(y + size).min(4) {
        for cz in z..(z + size).min(4) {
            for cx in x..(x + size).min(4) {
                mask |= 1 << (cx + 4 * cz + 16 * cy);
            }
        }
    }
    mask
}

/// Fusionne deux pièces du même matériau quand l'union en est une seule : couches de la même face
/// qui se touchent, pentes de même orientation (les niveaux s'ajoutent), microblocs disjoints,
/// demi-couches voisines de la même moitié.
pub(crate) fn merge(old: Piece, new: Piece) -> Option<Piece> {
    match (old, new) {
        (
            Piece::Layer {
                face: fa,
                offset: oa,
                thickness: ta,
            },
            Piece::Layer {
                face: fb,
                offset: ob,
                thickness: tb,
            },
        ) if fa == fb && (oa + ta == ob || ob + tb == oa) && ta + tb <= 16 => Some(Piece::Layer {
            face: fa,
            offset: oa.min(ob),
            thickness: ta + tb,
        }),
        (
            Piece::Slope {
                base: ba,
                up: ua,
                level: la,
                corner: ca,
            },
            Piece::Slope {
                base: bb,
                up: ub,
                level: lb,
                corner: cb,
            },
        ) if ba == bb && ua == ub && ca == cb && la + lb <= 16 => Some(Piece::Slope {
            base: ba,
            up: ua,
            level: la + lb,
            corner: ca,
        }),
        (Piece::Micro { cells: a }, Piece::Micro { cells: b }) if a & b == 0 => {
            Some(Piece::Micro { cells: a | b })
        }
        (
            Piece::Cuboid {
                min: min_a,
                max: max_a,
                kind: PieceKind::HalfLayer,
                count: count_a,
            },
            Piece::Cuboid {
                min: min_b,
                max: max_b,
                kind: PieceKind::HalfLayer,
                count: count_b,
            },
        ) => {
            // Identiques sur deux axes, et qui se touchent sur le troisième.
            let axis = (0..3).find(|&a| min_a[a] != min_b[a] || max_a[a] != max_b[a])?;
            let same_elsewhere = (0..3)
                .filter(|&a| a != axis)
                .all(|a| min_a[a] == min_b[a] && max_a[a] == max_b[a]);
            let touching = max_a[axis] == min_b[axis] || max_b[axis] == min_a[axis];
            (same_elsewhere && touching).then(|| {
                let (mut min, mut max) = (min_a, max_a);
                min[axis] = min_a[axis].min(min_b[axis]);
                max[axis] = max_a[axis].max(max_b[axis]);
                Piece::Cuboid {
                    min,
                    max,
                    kind: PieceKind::HalfLayer,
                    count: count_a.saturating_add(count_b),
                }
            })
        }
        _ => None,
    }
}
