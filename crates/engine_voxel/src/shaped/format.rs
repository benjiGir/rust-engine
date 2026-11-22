//! Le format d'une case de formes (`BlockEntity::data`).
//!
//! Un octet de version, la liste des matériaux en **identifiants texte** (la case reste lisible quand
//! la liste des mods change : un matériau inconnu garde son nom, le maillage le montre en
//! `core:unknown`), puis les pièces. Au plus [`MAX_MATERIALS`](super::MAX_MATERIALS) matériaux et
//! [`MAX_PARTS`](super::MAX_PARTS) pièces par case. Aucun numéro de bloc n'est écrit.
//!
//! # Les versions
//!
//! - **1** : couche (étiquette 0), pente `facing, ceiling, level` (1), microblocs (2).
//! - **2** : la pente devient `base, up, level, corner` (3) et trois pièces s'ajoutent : le pavé (4),
//!   l'escalier (5) et la forme raccordée (6). La version 1 se relit toujours : sa pente est une
//!   pente droite posée au sol (ou au plafond) ; on écrit toujours en version 2.

use super::kind::PieceKind;
use super::slope::{self, Corner};
use super::{MAX_MATERIALS, MAX_PARTS, Part, Piece, Shaped};
use crate::tech::capabilities::Face;

/// La version écrite.
pub const VERSION: u8 = 2;

/// Les octets d'une case.
pub(super) fn encode(shaped: &Shaped) -> Vec<u8> {
    let mut out = vec![VERSION, shaped.materials.len() as u8];
    for name in &shaped.materials {
        let bytes = name.as_bytes();
        out.push(bytes.len().min(255) as u8);
        out.extend_from_slice(&bytes[..bytes.len().min(255)]);
    }
    out.push(shaped.parts.len() as u8);
    for part in &shaped.parts {
        out.push(part.material);
        match part.piece {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => out.extend([0, face as u8, offset, thickness]),
            Piece::Micro { cells } => {
                out.push(2);
                out.extend(cells.to_le_bytes());
            }
            Piece::Slope {
                base,
                up,
                level,
                corner,
            } => out.extend([3, base as u8, up as u8, level, corner.code()]),
            Piece::Cuboid {
                min,
                max,
                kind,
                count,
            } => {
                out.push(4);
                out.extend(min);
                out.extend(max);
                out.extend([kind.code(), count]);
            }
            Piece::Stairs {
                facing,
                ceiling,
                vertical,
            } => out.extend([5, facing, u8::from(ceiling), u8::from(vertical)]),
            Piece::Connected { kind, open } => out.extend([6, kind.code(), u8::from(open)]),
        }
    }
    out
}

/// Un lecteur d'octets qui s'arrête proprement au bout des données.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.data.get(self.at)?;
        self.at += 1;
        Some(byte)
    }

    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let slice = self.data.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(slice)
    }

    fn triple(&mut self) -> Option<[u8; 3]> {
        Some([self.byte()?, self.byte()?, self.byte()?])
    }

    fn face(&mut self) -> Option<Face> {
        Face::from_index(i32::from(self.byte()?))
    }
}

/// Lit les octets d'une case. Sans `names`, les noms des matériaux ne sont pas copiés.
pub(super) fn parse(data: &[u8], names: bool) -> Option<Shaped> {
    let mut r = Reader { data, at: 0 };
    let version = r.byte()?;
    if version != 1 && version != VERSION {
        return None;
    }
    let count = usize::from(r.byte()?);
    if count > MAX_MATERIALS {
        return None;
    }
    let mut materials = Vec::with_capacity(count);
    for _ in 0..count {
        let len = usize::from(r.byte()?);
        let bytes = r.bytes(len)?;
        materials.push(if names {
            String::from_utf8(bytes.to_vec()).ok()?
        } else {
            String::new()
        });
    }
    let n_parts = usize::from(r.byte()?);
    if n_parts > MAX_PARTS {
        return None;
    }
    let mut parts = Vec::with_capacity(n_parts);
    for _ in 0..n_parts {
        let material = r.byte()?;
        if usize::from(material) >= materials.len() {
            return None;
        }
        let piece = match (version, r.byte()?) {
            (_, 0) => Piece::Layer {
                face: r.face()?,
                offset: r.byte()?,
                thickness: r.byte()?,
            },
            (1, 1) => {
                let (facing, ceiling, level) = (r.byte()?, r.byte()? != 0, r.byte()?);
                if facing >= 4 {
                    return None;
                }
                let (base, up) = slope::legacy(facing, ceiling);
                Piece::Slope {
                    base,
                    up,
                    level,
                    corner: Corner::None,
                }
            }
            (_, 2) => Piece::Micro {
                cells: u64::from_le_bytes(r.bytes(8)?.try_into().ok()?),
            },
            (2, 3) => Piece::Slope {
                base: r.face()?,
                up: r.face()?,
                level: r.byte()?,
                corner: Corner::from_code(r.byte()?)?,
            },
            (2, 4) => {
                let (min, max) = (r.triple()?, r.triple()?);
                Piece::Cuboid {
                    min,
                    max,
                    kind: PieceKind::from_code(r.byte()?)?,
                    count: r.byte()?,
                }
            }
            (2, 5) => Piece::Stairs {
                facing: r.byte()?,
                ceiling: r.byte()? != 0,
                vertical: r.byte()? != 0,
            },
            (2, 6) => Piece::Connected {
                kind: super::connected::Link::from_code(r.byte()?)?,
                open: r.byte()? != 0,
            },
            _ => return None,
        };
        if !piece.is_valid() {
            return None;
        }
        parts.push(Part { piece, material });
    }
    // Une forme raccordée occupe seule sa case.
    if parts.len() > 1
        && parts
            .iter()
            .any(|p| matches!(p.piece, Piece::Connected { .. }))
    {
        return None;
    }
    Some(Shaped { materials, parts })
}
