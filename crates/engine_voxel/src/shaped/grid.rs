//! La grille d'occupation d'une case : 16 × 16 × 16 bits, un par seizième de bloc.
//!
//! Chaque pièce sait se « poser » sur cette grille ; la case s'en sert pour refuser un recouvrement,
//! savoir si elle est pleine et cacher les faces intérieures au maillage. Les fonctions sont
//! volontairement petites et sans état : les pièces les composent.

use crate::tech::capabilities::Face;

/// Les seizièmes d'un côté de case.
pub(crate) const N: usize = 16;

/// L'occupation d'une case : un bit par seizième de bloc (16³). L'indice est `y * 16 + z`, le bit
/// est `x`.
pub type Occupancy = [u16; 256];

pub(crate) const FULL_ROW: u16 = u16::MAX;

pub(crate) fn full() -> Occupancy {
    [FULL_ROW; 256]
}

pub(crate) fn empty() -> Occupancy {
    [0; 256]
}

fn row(y: usize, z: usize) -> usize {
    y * N + z
}

pub(crate) fn is_set(occ: &Occupancy, x: usize, y: usize, z: usize) -> bool {
    occ[row(y, z)] & (1 << x) != 0
}

pub(crate) fn set(occ: &mut Occupancy, x: usize, y: usize, z: usize) {
    occ[row(y, z)] |= 1 << x;
}

pub(crate) fn overlaps(a: &Occupancy, b: &Occupancy) -> bool {
    a.iter().zip(b).any(|(a, b)| a & b != 0)
}

pub(crate) fn union(a: &Occupancy, b: &Occupancy) -> Occupancy {
    std::array::from_fn(|i| a[i] | b[i])
}

pub(crate) fn volume(occ: &Occupancy) -> u32 {
    occ.iter().map(|r| r.count_ones()).sum()
}

/// Une boîte en seizièmes : (x0, y0, z0, x1, y1, z1), bornes hautes exclues.
pub(crate) type IntBox = [u8; 6];

/// Remplit la grille avec une boîte (en seizièmes).
pub(crate) fn fill_box(occ: &mut Occupancy, b: &IntBox) {
    let [x0, y0, z0, x1, y1, z1] = b.map(usize::from);
    let mask: u16 = (x0..x1.min(N)).fold(0, |m, x| m | (1 << x));
    for y in y0..y1.min(N) {
        for z in z0..z1.min(N) {
            occ[row(y, z)] |= mask;
        }
    }
}

/// L'occupation d'une liste de boîtes.
pub(crate) fn boxes_occupancy(boxes: &[IntBox]) -> Occupancy {
    let mut occ = empty();
    for b in boxes {
        fill_box(&mut occ, b);
    }
    occ
}

/// L'axe (0 : x, 1 : y, 2 : z) d'une face et si elle est du côté des coordonnées fortes.
pub(crate) fn face_axis(face: Face) -> (usize, bool) {
    (face as usize / 2, (face as usize).is_multiple_of(2))
}

pub(crate) fn face_of(axis: usize, positive: bool) -> Face {
    Face::ALL[axis * 2 + usize::from(!positive)]
}

/// Une face après `k` quarts de tour autour de la verticale (+X devient +Z, comme la rotation des
/// structures mobiles).
pub(crate) fn turn_face(face: Face, k: u8) -> Face {
    let mut face = face;
    for _ in 0..k % 4 {
        face = match face {
            Face::PosX => Face::PosZ,
            Face::PosZ => Face::NegX,
            Face::NegX => Face::NegZ,
            Face::NegZ => Face::PosX,
            other => other,
        };
    }
    face
}

/// Une boîte après `k` quarts de tour autour de la verticale : (x, z) devient (16 - z, x).
pub(crate) fn turn_box(b: IntBox, k: u8) -> IntBox {
    let mut b = b;
    for _ in 0..k % 4 {
        let [x0, y0, z0, x1, y1, z1] = b;
        b = [16 - z1, y0, x0, 16 - z0, y1, x1];
    }
    b
}
