//! Les pentes : un plan incliné posé sur une face de la case, qui monte vers une autre face, avec un
//! coin sortant ou rentrant en option.
//!
//! # L'orientation
//!
//! Une pente a une **base** (la face d'appui : le sol, le plafond ou un mur) et un **haut** (la face
//! vers laquelle elle monte, perpendiculaire à la base) : 6 × 4 = 24 orientations. Les pentes de la
//! première version sont celles dont la base est le sol (ou le plafond, retournées) et dont le haut
//! est leur ancienne orientation (voir [`Piece::floor_slope`](super::Piece::floor_slope)).
//!
//! Toute la géométrie est écrite **une fois**, dans une pente « canonique » : la base au sol (-Y), le
//! bord haut au nord (z = 0) et, pour un coin, un second bord haut à l'ouest (x = 0). Une rotation
//! propre ([`Frame`]) la remène ensuite dans l'une des 24 orientations. Aucune symétrie n'est
//! cassée : une rotation, jamais un miroir.
//!
//! # La hauteur
//!
//! Droite, la hauteur d'une colonne ne dépend que de sa distance `d` au bord haut ([`slope_height`]).
//! En coin, elle dépend des distances `dz` et `dx` aux deux bords hauts :
//!
//! - **coin sortant** (une pyramide, comme l'angle d'un toit) : `h(max(dz, dx))` : la pointe est au
//!   coin où les deux bords hauts se rejoignent ;
//! - **coin rentrant** (deux plans qui se rejoignent en creux) : `h(min(dz, dx))`.
//!
//! Les niveaux (1 à 16) s'empilent comme ceux d'une pente droite.

use glam::{IVec3, Vec3};

use super::grid::{self, IntBox, N, Occupancy};
use crate::tech::capabilities::Face;

/// Le coin d'une pente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Corner {
    /// Une pente droite.
    None,
    /// Un coin sortant : une pyramide.
    Outer,
    /// Un coin rentrant.
    Inner,
}

impl Corner {
    pub fn code(self) -> u8 {
        match self {
            Corner::None => 0,
            Corner::Outer => 1,
            Corner::Inner => 2,
        }
    }

    pub fn from_code(code: u8) -> Option<Corner> {
        match code {
            0 => Some(Corner::None),
            1 => Some(Corner::Outer),
            2 => Some(Corner::Inner),
            _ => None,
        }
    }
}

/// La hauteur, en seizièmes, de la pente `level` à la profondeur `d` (0 : au bord haut) : la hauteur
/// du plan au milieu de la colonne, arrondie. Le niveau 16 donne 16, 15... 1.
pub fn slope_height(level: u8, d: usize) -> usize {
    (usize::from(level) * (2 * N - 1 - 2 * d) + N) / (2 * N)
}

/// La hauteur de la colonne canonique (x, z) d'une pente.
fn column_height(level: u8, corner: Corner, x: usize, z: usize) -> usize {
    slope_height(
        level,
        match corner {
            Corner::None => z,
            Corner::Outer => x.max(z),
            Corner::Inner => x.min(z),
        },
    )
}

pub(crate) fn direction_of(face: Face) -> IVec3 {
    crate::FACINGS[face as usize]
}

/// Les quatre faces horizontales, dans l'ordre des orientations de la première version (nord, est,
/// sud, ouest).
pub(crate) const HORIZONTAL: [Face; 4] = [Face::NegZ, Face::PosX, Face::PosZ, Face::NegX];

/// La rotation qui remène la pente canonique dans l'orientation (base, haut) : pour chaque axe
/// canonique, l'axe du monde où il va et s'il est renversé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Frame {
    axes: [(usize, bool); 3],
}

impl Frame {
    /// `None` si la base et le haut ne sont pas perpendiculaires.
    pub(crate) fn new(base: Face, up: Face) -> Option<Frame> {
        let (b, u) = (direction_of(base), direction_of(up));
        if b.dot(u) != 0 || b == IVec3::ZERO {
            return None;
        }
        // Canonique : la base est -Y, le haut -Z, donc X = (-b) × (-u) = b × u.
        let columns = [b.cross(u), -b, -u];
        let axes = columns.map(|c| {
            let axis = (0..3).find(|&i| c[i] != 0).expect("une colonne unitaire");
            (axis, c[axis] < 0)
        });
        Some(Frame { axes })
    }

    /// Une cellule canonique (indices 0 à 15), dans le monde.
    pub(crate) fn cell(&self, c: [usize; 3]) -> [usize; 3] {
        let mut out = [0; 3];
        for (j, &(axis, flipped)) in self.axes.iter().enumerate() {
            out[axis] = if flipped { N - 1 - c[j] } else { c[j] };
        }
        out
    }

    /// Un sommet canonique (coordonnées 0 à 16), dans le monde.
    pub(crate) fn vertex(&self, v: [i32; 3]) -> [i32; 3] {
        let mut out = [0; 3];
        for (j, &(axis, flipped)) in self.axes.iter().enumerate() {
            out[axis] = if flipped { 16 - v[j] } else { v[j] };
        }
        out
    }

    /// Un point canonique (0 à 1), dans le monde.
    pub(crate) fn point(&self, p: Vec3) -> Vec3 {
        let mut out = Vec3::ZERO;
        for (j, &(axis, flipped)) in self.axes.iter().enumerate() {
            out[axis] = if flipped { 1.0 - p[j] } else { p[j] };
        }
        out
    }

    /// Une direction canonique, dans le monde.
    pub(crate) fn direction(&self, d: Vec3) -> Vec3 {
        let mut out = Vec3::ZERO;
        for (j, &(axis, flipped)) in self.axes.iter().enumerate() {
            out[axis] = if flipped { -d[j] } else { d[j] };
        }
        out
    }

    /// La face du monde où va la face canonique « dessus » (+Y) : celle d'où l'on voit le plan incliné
    /// (le dessus pour une pente au sol, le dessous au plafond).
    pub(crate) fn top_face(&self) -> Face {
        let (axis, flipped) = self.axes[1];
        grid::face_of(axis, !flipped)
    }

    /// La face du monde du second bord haut d'un coin (le canonique -X).
    pub(crate) fn second_edge(&self) -> Face {
        let (axis, flipped) = self.axes[0];
        // -X canonique : le côté bas de l'axe, renversé si l'axe l'est.
        grid::face_of(axis, flipped)
    }

    /// Une boîte canonique (sommets), dans le monde.
    pub(crate) fn int_box(&self, b: [i32; 6]) -> IntBox {
        let lo = self.vertex([b[0], b[1], b[2]]);
        let hi = self.vertex([b[3], b[4], b[5]]);
        std::array::from_fn(|i| {
            let (a, c) = (lo[i % 3], hi[i % 3]);
            (if i < 3 { a.min(c) } else { a.max(c) }) as u8
        })
    }
}

/// Le haut qui donne à une pente en coin ses deux bords hauts `a` et `b` sur la base `base`
/// (`None` si les deux faces ne sont pas deux côtés adjacents de la base).
pub fn corner_up(base: Face, a: Face, b: Face) -> Option<Face> {
    [(a, b), (b, a)].into_iter().find_map(|(up, other)| {
        let frame = Frame::new(base, up)?;
        (frame.second_edge() == other).then_some(up)
    })
}

/// Les seizièmes que remplit une pente.
pub(crate) fn occupancy(base: Face, up: Face, level: u8, corner: Corner) -> Occupancy {
    let mut occ = grid::empty();
    let Some(frame) = Frame::new(base, up) else {
        return occ;
    };
    for z in 0..N {
        for x in 0..N {
            for y in 0..column_height(level, corner, x, z) {
                let [wx, wy, wz] = frame.cell([x, y, z]);
                grid::set(&mut occ, wx, wy, wz);
            }
        }
    }
    occ
}

/// Les boîtes d'une pente pour la collision et la visée : un escalier de seizièmes (au plus 16
/// groupes de colonnes de même hauteur ; deux boîtes par groupe en coin).
pub(crate) fn boxes(base: Face, up: Face, level: u8, corner: Corner) -> Vec<IntBox> {
    let Some(frame) = Frame::new(base, up) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut m0 = 0;
    while m0 < N {
        let height = slope_height(level, m0) as i32;
        let mut m1 = m0 + 1;
        while m1 < N && slope_height(level, m1) as i32 == height {
            m1 += 1;
        }
        if height > 0 {
            let (a, b) = (m0 as i32, m1 as i32);
            let canonical: Vec<[i32; 6]> = match corner {
                Corner::None => vec![[0, 0, a, 16, height, b]],
                Corner::Outer => {
                    let mut v = vec![[0, 0, a, b, height, b]];
                    if a > 0 {
                        v.push([a, 0, 0, b, height, a]);
                    }
                    v
                }
                Corner::Inner => {
                    let mut v = vec![[a, 0, a, 16, height, b]];
                    if b < 16 {
                        v.push([a, 0, b, b, height, 16]);
                    }
                    v
                }
            };
            out.extend(canonical.into_iter().map(|c| frame.int_box(c)));
        }
        m0 = m1;
    }
    out
}

/// Une face de la pente canonique : un polygone de 3 ou 4 coins, sa normale, et si c'est le plan
/// incliné (les autres sont posées sur le bord de la case).
#[derive(Debug, Clone)]
pub(crate) struct Facet {
    pub corners: Vec<Vec3>,
    pub normal: Vec3,
    pub incline: bool,
}

/// Les faces de la pente canonique de niveau `level`.
pub(crate) fn facets(level: u8, corner: Corner) -> Vec<Facet> {
    let h = f32::from(level) / 16.0;
    let v = Vec3::new;
    let facet = |corners: &[Vec3], normal: Vec3, incline: bool| Facet {
        corners: corners.to_vec(),
        normal: normal.normalize(),
        incline,
    };
    let bottom = facet(
        &[
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(1.0, 0.0, 1.0),
            v(0.0, 0.0, 1.0),
        ],
        Vec3::NEG_Y,
        false,
    );
    // Les plans inclinés : montent vers -Z (normale (0, 1, h)) ou vers -X (normale (h, 1, 0)).
    let along_z = Vec3::new(0.0, 1.0, h);
    let along_x = Vec3::new(h, 1.0, 0.0);
    match corner {
        Corner::None => vec![
            facet(
                &[
                    v(0.0, h, 0.0),
                    v(1.0, h, 0.0),
                    v(1.0, 0.0, 1.0),
                    v(0.0, 0.0, 1.0),
                ],
                along_z,
                true,
            ),
            facet(
                &[
                    v(0.0, 0.0, 0.0),
                    v(0.0, h, 0.0),
                    v(1.0, h, 0.0),
                    v(1.0, 0.0, 0.0),
                ],
                Vec3::NEG_Z,
                false,
            ),
            bottom,
            facet(
                &[v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(0.0, h, 0.0)],
                Vec3::NEG_X,
                false,
            ),
            facet(
                &[v(1.0, 0.0, 0.0), v(1.0, h, 0.0), v(1.0, 0.0, 1.0)],
                Vec3::X,
                false,
            ),
        ],
        Corner::Outer => vec![
            facet(
                &[v(0.0, h, 0.0), v(0.0, 0.0, 1.0), v(1.0, 0.0, 1.0)],
                along_z,
                true,
            ),
            facet(
                &[v(0.0, h, 0.0), v(1.0, 0.0, 0.0), v(1.0, 0.0, 1.0)],
                along_x,
                true,
            ),
            bottom,
            facet(
                &[v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(0.0, h, 0.0)],
                Vec3::NEG_X,
                false,
            ),
            facet(
                &[v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, h, 0.0)],
                Vec3::NEG_Z,
                false,
            ),
        ],
        Corner::Inner => vec![
            facet(
                &[v(0.0, h, 0.0), v(1.0, h, 0.0), v(1.0, 0.0, 1.0)],
                along_z,
                true,
            ),
            facet(
                &[v(0.0, h, 0.0), v(0.0, h, 1.0), v(1.0, 0.0, 1.0)],
                along_x,
                true,
            ),
            bottom,
            facet(
                &[
                    v(0.0, 0.0, 0.0),
                    v(0.0, 0.0, 1.0),
                    v(0.0, h, 1.0),
                    v(0.0, h, 0.0),
                ],
                Vec3::NEG_X,
                false,
            ),
            facet(
                &[
                    v(0.0, 0.0, 0.0),
                    v(0.0, h, 0.0),
                    v(1.0, h, 0.0),
                    v(1.0, 0.0, 0.0),
                ],
                Vec3::NEG_Z,
                false,
            ),
            facet(
                &[v(1.0, 0.0, 0.0), v(1.0, h, 0.0), v(1.0, 0.0, 1.0)],
                Vec3::X,
                false,
            ),
            facet(
                &[v(0.0, 0.0, 1.0), v(1.0, 0.0, 1.0), v(0.0, h, 1.0)],
                Vec3::Z,
                false,
            ),
        ],
    }
}

/// Le haut d'une pente de la première version (0 nord, 1 est, 2 sud, 3 ouest).
pub(crate) fn legacy_up(facing: u8) -> Face {
    HORIZONTAL[usize::from(facing % 4)]
}

/// L'orientation de la première version, `(base, haut)`.
pub(crate) fn legacy(facing: u8, ceiling: bool) -> (Face, Face) {
    (
        if ceiling { Face::PosY } else { Face::NegY },
        legacy_up(facing),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_24_orientations_are_proper_rotations() {
        let mut seen = Vec::new();
        for base in Face::ALL {
            for up in Face::ALL {
                let Some(frame) = Frame::new(base, up) else {
                    assert!(
                        base as usize / 2 == up as usize / 2,
                        "{base:?} {up:?} devraient être perpendiculaires"
                    );
                    continue;
                };
                // La base canonique (-Y) va sur la base, le haut canonique (-Z) sur le haut.
                assert_eq!(frame.direction(Vec3::NEG_Y), direction_of(base).as_vec3());
                assert_eq!(frame.direction(Vec3::NEG_Z), direction_of(up).as_vec3());
                seen.push(frame);
            }
        }
        assert_eq!(seen.len(), 24);
        seen.dedup();
        for (i, a) in seen.iter().enumerate() {
            assert!(!seen[i + 1..].contains(a));
        }
    }

    #[test]
    fn the_second_edge_follows_the_rotation() {
        // Canonique : base au sol, haut au nord, second bord à l'ouest.
        let frame = Frame::new(Face::NegY, Face::NegZ).unwrap();
        assert_eq!(frame.second_edge(), Face::NegX);
        // Tourné d'un quart (haut à l'est) : le second bord est au nord.
        let frame = Frame::new(Face::NegY, Face::PosX).unwrap();
        assert_eq!(frame.second_edge(), Face::NegZ);
        assert_eq!(
            corner_up(Face::NegY, Face::NegZ, Face::NegX),
            Some(Face::NegZ)
        );
        assert_eq!(
            corner_up(Face::NegY, Face::NegX, Face::NegZ),
            Some(Face::NegZ)
        );
        assert_eq!(
            corner_up(Face::NegY, Face::PosX, Face::NegZ),
            Some(Face::PosX)
        );
        // Deux faces opposées ne font pas un coin.
        assert_eq!(corner_up(Face::NegY, Face::PosX, Face::NegX), None);
    }

    #[test]
    fn corner_heights_at_the_corners() {
        let h = |corner, x, z| column_height(16, corner, x, z);
        // Sortant : la pointe au coin des deux bords hauts, zéro au coin opposé.
        assert_eq!(h(Corner::Outer, 0, 0), 16);
        assert_eq!(h(Corner::Outer, 15, 15), 1);
        assert_eq!(h(Corner::Outer, 15, 0), h(Corner::Outer, 0, 15));
        // Rentrant : haut sur les deux bords hauts, bas au coin opposé.
        assert_eq!(h(Corner::Inner, 0, 15), 16);
        assert_eq!(h(Corner::Inner, 15, 0), 16);
        assert_eq!(h(Corner::Inner, 15, 15), 1);
        // Le sortant est toujours plus bas que le rentrant.
        for x in 0..N {
            for z in 0..N {
                assert!(h(Corner::Outer, x, z) <= h(Corner::None, x, z));
                assert!(h(Corner::None, x, z) <= h(Corner::Inner, x, z));
            }
        }
    }
}
