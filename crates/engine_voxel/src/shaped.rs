//! Les formes : le bloc `core:shaped`, une case qui contient des **pièces** (couches, pentes,
//! microblocs) au seizième de bloc près, chacune avec son propre matériau.
//!
//! # Le principe
//!
//! Il n'y a qu'un bloc dans le registre, `core:shaped`. Ce qu'il contient est dans son entité de
//! bloc (`BlockEntity::data`), pas dans des variantes : aucun bloc à déclarer par matériau, et
//! n'importe quel cube plein (du jeu ou d'un mod) peut être taillé. La case sait :
//!
//! - **s'occuper** ([`Shaped::occupancy`]) : 16 × 16 × 16 bits, de quoi refuser un recouvrement,
//!   savoir si la case est pleine et cacher les faces intérieures au maillage ;
//! - **se découper en boîtes** ([`Shaped::boxes`]) pour la collision et la visée : les pentes sont
//!   un escalier de seizièmes (au plus 16 boîtes), la marche automatique du joueur les monte ;
//! - **se simplifier** ([`Shaped::simplified`]) : vide, elle redevient de l'air ; pleine d'un seul
//!   matériau, elle redevient le vrai bloc de ce matériau ;
//! - **tourner** ([`Shaped::turned`]) avec les structures mobiles qui se reposent.
//!
//! # Les pièces
//!
//! - [`Piece::Layer`] : une tranche de la case, collée à une face, épaisse de 1 à 16 seizièmes
//!   (8 : une dalle, 16 : un bloc plein). Par rapport à la spécification d'origine, elle porte en
//!   plus un `offset` (la distance à la face, en seizièmes) : une dalle de chêne peut ainsi se poser
//!   *sur* une dalle de pierre, au lieu de toujours partir d'une face.
//! - [`Piece::Slope`] : un plan incliné qui monte de 0 (bord bas) à `level` seizièmes (bord haut)
//!   sur toute la case. Le niveau 16 est la pente pleine (45°, un demi-bloc en volume). Elle
//!   se pose au sol ou, retournée, au plafond.
//! - [`Piece::Micro`] : des microblocs sur une grille 4 × 4 × 4 (un bit par cellule de 1/64 de bloc).
//!   Un microbloc de 1/8 occupe 2 × 2 × 2 cellules alignées.
//!
//! # Le format
//!
//! `encode` / `decode` : un octet de version, la liste des matériaux en **identifiants texte**
//! (la case reste lisible quand la liste des mods change : un matériau inconnu garde son nom, le
//! maillage le montre en `core:unknown`), puis les pièces. Au plus [`MAX_MATERIALS`] matériaux et
//! [`MAX_PARTS`] pièces par case. Aucun numéro de bloc n'est écrit.

use glam::{IVec3, Vec3};

use crate::block::BlockId;
use crate::capabilities::Face;
use crate::chunk::BlockEntity;
use crate::registry::{BlockBox, BlockDef, Shape};

/// Le comportement (et la clé) du bloc de formes.
pub const BEHAVIOR: &str = "core:shaped";
/// Le matériau montré à la place d'un matériau que le registre ne connaît pas.
pub const UNKNOWN_MATERIAL: &str = "core:unknown";
/// Au plus ce nombre de matériaux différents dans une case.
pub const MAX_MATERIALS: usize = 16;
/// Au plus ce nombre de pièces dans une case.
pub const MAX_PARTS: usize = 32;
const VERSION: u8 = 1;

/// Les seizièmes d'un côté de case.
const N: usize = 16;
/// L'occupation d'une case : un bit par seizième de bloc (16³). L'indice est `y * 16 + z`, le bit
/// est `x`.
pub type Occupancy = [u16; 256];

const FULL_ROW: u16 = u16::MAX;

fn full() -> Occupancy {
    [FULL_ROW; 256]
}

fn row(y: usize, z: usize) -> usize {
    y * N + z
}

pub(crate) fn is_set(occ: &Occupancy, x: usize, y: usize, z: usize) -> bool {
    occ[row(y, z)] & (1 << x) != 0
}

fn set(occ: &mut Occupancy, x: usize, y: usize, z: usize) {
    occ[row(y, z)] |= 1 << x;
}

fn overlaps(a: &Occupancy, b: &Occupancy) -> bool {
    a.iter().zip(b).any(|(a, b)| a & b != 0)
}

fn union(a: &Occupancy, b: &Occupancy) -> Occupancy {
    std::array::from_fn(|i| a[i] | b[i])
}

fn volume(occ: &Occupancy) -> u32 {
    occ.iter().map(|r| r.count_ones()).sum()
}

/// L'axe (0 : x, 1 : y, 2 : z) d'une face et si elle est du côté des coordonnées fortes.
fn face_axis(face: Face) -> (usize, bool) {
    (face as usize / 2, (face as usize).is_multiple_of(2))
}

fn face_of(axis: usize, positive: bool) -> Face {
    Face::ALL[axis * 2 + usize::from(!positive)]
}

/// Les quatre orientations horizontales, comme l'état `Facing4` des blocs : 0 nord (-Z), 1 est (+X),
/// 2 sud (+Z), 3 ouest (-X).
const FACING_DIRECTIONS: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];

/// La hauteur, en seizièmes, de la pente `level` à la profondeur `d` (0 : au bord haut) : la hauteur
/// du plan au milieu de la colonne, arrondie. Le niveau 16 donne 16, 15... 1.
pub fn slope_height(level: u8, d: usize) -> usize {
    (usize::from(level) * (2 * N - 1 - 2 * d) + N) / (2 * N)
}

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
    /// Une pente de `level` seizièmes (1 à 16). `facing` : le côté haut (0 nord, 1 est, 2 sud,
    /// 3 ouest) ; `ceiling` : accrochée au plafond.
    Slope {
        facing: u8,
        ceiling: bool,
        level: u8,
    },
    /// Des cellules de 1/64 de bloc (grille 4 × 4 × 4) : le bit `x + 4 * z + 16 * y`.
    Micro { cells: u64 },
}

impl Piece {
    /// Les valeurs sont-elles dans leurs bornes ?
    pub fn is_valid(&self) -> bool {
        match *self {
            Piece::Layer {
                offset, thickness, ..
            } => thickness >= 1 && usize::from(offset) + usize::from(thickness) <= N,
            Piece::Slope { facing, level, .. } => facing < 4 && (1..=16).contains(&level),
            Piece::Micro { cells } => cells != 0,
        }
    }

    /// Les seizièmes que la pièce remplit.
    pub fn occupancy(&self) -> Occupancy {
        let mut occ = [0; 256];
        match *self {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => {
                let (axis, positive) = face_axis(face);
                let (offset, thickness) = (usize::from(offset), usize::from(thickness));
                let (lo, hi) = if positive {
                    (N - offset - thickness, N - offset)
                } else {
                    (offset, offset + thickness)
                };
                for y in 0..N {
                    for z in 0..N {
                        for x in 0..N {
                            if (lo..hi).contains(&[x, y, z][axis]) {
                                set(&mut occ, x, y, z);
                            }
                        }
                    }
                }
            }
            Piece::Slope {
                facing,
                ceiling,
                level,
            } => {
                for z in 0..N {
                    for x in 0..N {
                        let height = slope_height(level, slope_depth(facing, x, z));
                        for k in 0..height {
                            let y = if ceiling { N - 1 - k } else { k };
                            set(&mut occ, x, y, z);
                        }
                    }
                }
            }
            Piece::Micro { cells } => {
                for bit in 0..64usize {
                    if cells & (1 << bit) == 0 {
                        continue;
                    }
                    let (cx, cz, cy) = (bit % 4, (bit / 4) % 4, bit / 16);
                    for y in cy * 4..cy * 4 + 4 {
                        for z in cz * 4..cz * 4 + 4 {
                            for x in cx * 4..cx * 4 + 4 {
                                set(&mut occ, x, y, z);
                            }
                        }
                    }
                }
            }
        }
        occ
    }

    /// Les boîtes de la pièce, en seizièmes : (x0, y0, z0, x1, y1, z1).
    pub(crate) fn int_boxes(&self) -> Vec<[u8; 6]> {
        match *self {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => {
                let (axis, positive) = face_axis(face);
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
                facing,
                ceiling,
                level,
            } => {
                let mut boxes: Vec<[u8; 6]> = Vec::new();
                let mut d = 0;
                while d < N {
                    let height = slope_height(level, d);
                    let mut end = d + 1;
                    while end < N && slope_height(level, end) == height {
                        end += 1;
                    }
                    if height > 0 {
                        // Les colonnes d..end : leur place le long de l'axe de la pente.
                        let (lo, hi) = match facing {
                            0 | 3 => (d as u8, end as u8),
                            _ => ((N - end) as u8, (N - d) as u8),
                        };
                        let (y0, y1) = if ceiling {
                            ((N - height) as u8, N as u8)
                        } else {
                            (0, height as u8)
                        };
                        boxes.push(if facing % 2 == 0 {
                            [0, y0, lo, 16, y1, hi]
                        } else {
                            [lo, y0, 0, hi, y1, 16]
                        });
                    }
                    d = end;
                }
                boxes
            }
            Piece::Micro { cells } => micro_boxes(cells),
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
            Piece::Slope { level, .. } => {
                if level == 16 {
                    push(PieceKind::Slope, 1);
                } else {
                    push(PieceKind::SlopeLayer, u32::from(level));
                }
            }
            Piece::Micro { cells } => {
                let micros = micro_blocks(cells);
                let rest = cells & !micros.iter().fold(0u64, |m, (_, mask)| m | mask);
                push(PieceKind::Micro, micros.len() as u32);
                push(PieceKind::Nano, rest.count_ones());
            }
        }
        out
    }

    fn turned(self, k: u8) -> Piece {
        let k = k % 4;
        match self {
            Piece::Layer {
                face,
                offset,
                thickness,
            } => Piece::Layer {
                face: turn_face(face, k),
                offset,
                thickness,
            },
            Piece::Slope {
                facing,
                ceiling,
                level,
            } => Piece::Slope {
                facing: (facing + k) % 4,
                ceiling,
                level,
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
        }
    }
}

/// La profondeur d'une colonne (x, z) par rapport au bord haut d'une pente : 0 contre lui.
fn slope_depth(facing: u8, x: usize, z: usize) -> usize {
    match facing {
        0 => z,
        1 => N - 1 - x,
        2 => N - 1 - z,
        _ => x,
    }
}

/// Une face après `k` quarts de tour autour de la verticale (+X devient +Z, comme la rotation des
/// structures mobiles).
fn turn_face(face: Face, k: u8) -> Face {
    let mut face = face;
    for _ in 0..k {
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

/// Les boîtes d'un ensemble de cellules de 1/64, fusionnées (en seizièmes).
fn micro_boxes(cells: u64) -> Vec<[u8; 6]> {
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

/// Une pièce de la case, avec son matériau (un indice dans [`Shaped::materials`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub piece: Piece,
    pub material: u8,
}

/// Pourquoi une pièce ne s'ajoute pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddError {
    /// Elle chevauche une pièce déjà là.
    Overlap,
    /// La case a déjà 32 pièces.
    TooManyParts,
    /// La case a déjà 16 matériaux.
    TooManyMaterials,
    /// Valeurs hors bornes.
    Invalid,
}

/// Ce qu'une case de formes devient une fois simplifiée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Simplified<'a> {
    /// Plus rien : de l'air.
    Empty,
    /// Pleine d'un seul matériau : le bloc de ce matériau.
    Full(&'a str),
    /// Elle reste une case de formes.
    Shaped,
}

/// Le contenu d'une case de formes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shaped {
    /// Les matériaux (identifiants texte), indexés par [`Part::material`].
    pub materials: Vec<String>,
    pub parts: Vec<Part>,
}

impl Shaped {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Le matériau d'une pièce.
    pub fn material_of(&self, part: &Part) -> &str {
        self.materials
            .get(usize::from(part.material))
            .map_or(UNKNOWN_MATERIAL, String::as_str)
    }

    /// Tout ce que remplissent les pièces.
    pub fn occupancy(&self) -> Occupancy {
        self.occupancy_where(|_| true)
    }

    /// Ce que remplissent les pièces dont le matériau satisfait `keep` (le maillage ne cache une
    /// face que derrière une pièce opaque).
    pub fn occupancy_where(&self, keep: impl Fn(&str) -> bool) -> Occupancy {
        self.parts
            .iter()
            .filter(|p| keep(self.material_of(p)))
            .fold([0; 256], |acc, p| union(&acc, &p.piece.occupancy()))
    }

    /// La case est-elle entièrement remplie ?
    pub fn is_full(&self) -> bool {
        self.occupancy() == full()
    }

    /// Vide, pleine d'un seul matériau, ou toujours une case de formes.
    pub fn simplified(&self) -> Simplified<'_> {
        let Some(first) = self.parts.first() else {
            return Simplified::Empty;
        };
        if self.is_full() && self.parts.iter().all(|p| p.material == first.material) {
            Simplified::Full(self.material_of(first))
        } else {
            Simplified::Shaped
        }
    }

    /// Le matériau qui remplit le plus de volume (le plus ancien en cas d'égalité).
    pub fn dominant_material(&self) -> Option<&str> {
        let mut volumes = vec![0u32; self.materials.len().max(1)];
        for p in &self.parts {
            if let Some(v) = volumes.get_mut(usize::from(p.material)) {
                *v += volume(&p.piece.occupancy());
            }
        }
        let best = volumes
            .iter()
            .enumerate()
            .rev()
            .max_by_key(|&(_, v)| *v)
            .map(|(i, _)| i)?;
        self.materials.get(best).map(String::as_str)
    }

    /// Ajoute une pièce du matériau `material`. Une pièce qui en touche une du même matériau s'y
    /// fusionne : une couche s'épaissit, une pente monte de niveau, des microblocs se réunissent.
    /// Ce qui chevauche une autre pièce est refusé.
    pub fn try_add(&mut self, piece: Piece, material: &str) -> Result<(), AddError> {
        if !piece.is_valid() {
            return Err(AddError::Invalid);
        }
        let index = match self.materials.iter().position(|m| m == material) {
            Some(i) => i,
            None => {
                if self.materials.len() >= MAX_MATERIALS {
                    return Err(AddError::TooManyMaterials);
                }
                self.materials.push(material.to_string());
                self.materials.len() - 1
            }
        } as u8;
        let result = self.add_part(piece, index);
        if result.is_err() {
            self.prune_materials();
        } else {
            self.coalesce();
        }
        result
    }

    fn add_part(&mut self, piece: Piece, material: u8) -> Result<(), AddError> {
        let merged = self.parts.iter().enumerate().find_map(|(i, p)| {
            (p.material == material)
                .then(|| merge(p.piece, piece))
                .flatten()
                .map(|m| (i, m))
        });
        if let Some((i, merged)) = merged {
            let others = self.occupancy_without(Some(i));
            if overlaps(&others, &merged.occupancy()) {
                return Err(AddError::Overlap);
            }
            self.parts[i].piece = merged;
            return Ok(());
        }
        if overlaps(&self.occupancy(), &piece.occupancy()) {
            return Err(AddError::Overlap);
        }
        if self.parts.len() >= MAX_PARTS {
            return Err(AddError::TooManyParts);
        }
        self.parts.push(Part { piece, material });
        Ok(())
    }

    fn occupancy_without(&self, skip: Option<usize>) -> Occupancy {
        self.parts
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != skip)
            .fold([0; 256], |acc, (_, p)| union(&acc, &p.piece.occupancy()))
    }

    /// Réunit les couches et les microblocs voisins du même matériau (jusqu'à ce que plus rien ne
    /// bouge) : trois dalles minces collées n'en font qu'une.
    fn coalesce(&mut self) {
        'again: loop {
            for i in 0..self.parts.len() {
                for j in i + 1..self.parts.len() {
                    if self.parts[i].material != self.parts[j].material {
                        continue;
                    }
                    let (a, b) = (self.parts[i].piece, self.parts[j].piece);
                    if matches!(a, Piece::Slope { .. }) {
                        continue;
                    }
                    if let Some(merged) = merge(a, b) {
                        self.parts[i].piece = merged;
                        self.parts.remove(j);
                        continue 'again;
                    }
                }
            }
            return;
        }
    }

    /// Retire les matériaux que plus aucune pièce n'utilise, et renumérote les pièces.
    fn prune_materials(&mut self) {
        let used: Vec<bool> = (0..self.materials.len())
            .map(|m| self.parts.iter().any(|p| usize::from(p.material) == m))
            .collect();
        let mut remap = vec![0u8; self.materials.len()];
        let mut kept = Vec::new();
        for (i, name) in self.materials.iter().enumerate() {
            if used[i] {
                remap[i] = kept.len() as u8;
                kept.push(name.clone());
            }
        }
        for p in &mut self.parts {
            p.material = remap[usize::from(p.material)];
        }
        self.materials = kept;
    }

    /// Les boîtes de la case, en blocs (0 à 1) : la collision et la visée.
    pub fn boxes(&self) -> Vec<BlockBox> {
        self.parts
            .iter()
            .flat_map(|p| p.piece.int_boxes())
            .map(to_block_box)
            .collect()
    }

    /// Les boîtes d'une pièce, en blocs.
    pub fn part_boxes(&self, index: usize) -> Vec<BlockBox> {
        self.parts
            .get(index)
            .map(|p| p.piece.int_boxes().into_iter().map(to_block_box).collect())
            .unwrap_or_default()
    }

    /// La pièce que contient ce point (en blocs, 0 à 1), à un 1/64 de bloc près : la plus proche.
    pub fn part_at(&self, point: Vec3) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, _) in self.parts.iter().enumerate() {
            for b in self.part_boxes(i) {
                let d = point.distance_squared(point.clamp(b.min, b.max));
                if d < 1.0 / 4096.0 && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        best.map(|(i, _)| i)
    }

    /// La pièce touchée par un rayon au point `point` (sur la face de normale `normal`) : on entre
    /// un peu dans la boîte avant de chercher.
    pub fn part_hit(&self, point: Vec3, normal: Vec3) -> Option<usize> {
        self.part_at(point - normal * 0.004)
    }

    /// Retire la pièce `index` et la rend, avec son matériau.
    pub fn remove_part(&mut self, index: usize) -> Option<(Piece, String)> {
        if index >= self.parts.len() {
            return None;
        }
        let part = self.parts.remove(index);
        let material = self.material_of(&part).to_string();
        self.prune_materials();
        Some((part.piece, material))
    }

    /// Retire la pièce visée (voir [`Shaped::part_hit`]).
    pub fn remove_at(&mut self, point: Vec3, normal: Vec3) -> Option<(Piece, String)> {
        let index = self.part_hit(point, normal)?;
        self.remove_part(index)
    }

    /// La case après `k` quarts de tour autour de la verticale.
    pub fn turned(&self, k: u8) -> Shaped {
        Shaped {
            materials: self.materials.clone(),
            parts: self
                .parts
                .iter()
                .map(|p| Part {
                    piece: p.piece.turned(k),
                    material: p.material,
                })
                .collect(),
        }
    }

    /// Les octets de la case, pour `BlockEntity::data`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![VERSION, self.materials.len() as u8];
        for name in &self.materials {
            let bytes = name.as_bytes();
            out.push(bytes.len().min(255) as u8);
            out.extend_from_slice(&bytes[..bytes.len().min(255)]);
        }
        out.push(self.parts.len() as u8);
        for part in &self.parts {
            out.push(part.material);
            match part.piece {
                Piece::Layer {
                    face,
                    offset,
                    thickness,
                } => out.extend([0, face as u8, offset, thickness]),
                Piece::Slope {
                    facing,
                    ceiling,
                    level,
                } => out.extend([1, facing, u8::from(ceiling), level]),
                Piece::Micro { cells } => {
                    out.push(2);
                    out.extend(cells.to_le_bytes());
                }
            }
        }
        out
    }

    /// Relit une case. `None` si les octets sont d'une autre version ou abîmés.
    pub fn decode(data: &[u8]) -> Option<Shaped> {
        parse(data, true)
    }

    /// Les boîtes d'une case encodée, sans en lire les noms de matériaux (la collision et la
    /// visée les demandent souvent).
    pub fn boxes_of(data: &[u8]) -> Option<Vec<BlockBox>> {
        parse(data, false).map(|s| s.boxes())
    }

    /// Les données d'une case après `k` quarts de tour (structures mobiles). Des octets illisibles
    /// sont rendus tels quels.
    pub fn turned_data(data: &[u8], k: u8) -> Vec<u8> {
        Self::decode(data).map_or_else(|| data.to_vec(), |s| s.turned(k).encode())
    }
}

fn to_block_box(b: [u8; 6]) -> BlockBox {
    let v = |a: u8| f32::from(a) / 16.0;
    BlockBox {
        min: Vec3::new(v(b[0]), v(b[1]), v(b[2])),
        max: Vec3::new(v(b[3]), v(b[4]), v(b[5])),
    }
}

/// Fusionne deux pièces du même matériau quand l'union en est une seule : couches de la même face
/// qui se touchent, pentes de même orientation (les niveaux s'ajoutent), microblocs disjoints.
fn merge(old: Piece, new: Piece) -> Option<Piece> {
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
                facing: fa,
                ceiling: ca,
                level: la,
            },
            Piece::Slope {
                facing: fb,
                ceiling: cb,
                level: lb,
            },
        ) if fa == fb && ca == cb && la + lb <= 16 => Some(Piece::Slope {
            facing: fa,
            ceiling: ca,
            level: la + lb,
        }),
        (Piece::Micro { cells: a }, Piece::Micro { cells: b }) if a & b == 0 => {
            Some(Piece::Micro { cells: a | b })
        }
        _ => None,
    }
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
}

/// Lit les octets d'une case. Sans `names`, les noms des matériaux ne sont pas copiés.
fn parse(data: &[u8], names: bool) -> Option<Shaped> {
    let mut r = Reader { data, at: 0 };
    if r.byte()? != VERSION {
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
        let piece = match r.byte()? {
            0 => Piece::Layer {
                face: Face::from_index(i32::from(r.byte()?))?,
                offset: r.byte()?,
                thickness: r.byte()?,
            },
            1 => Piece::Slope {
                facing: r.byte()?,
                ceiling: r.byte()? != 0,
                level: r.byte()?,
            },
            2 => Piece::Micro {
                cells: u64::from_le_bytes(r.bytes(8)?.try_into().ok()?),
            },
            _ => return None,
        };
        if !piece.is_valid() {
            return None;
        }
        parts.push(Part { piece, material });
    }
    Some(Shaped { materials, parts })
}

/// Un bloc peut-il être taillé ? Un cube plein ou transparent (verre), ni liquide, sans
/// comportement, qu'on peut poser : bois, pierres, verre, laine, minerais, blocs de mods. Un coffre,
/// un four, une herbe haute ou l'eau, non.
pub fn is_shapeable(def: &BlockDef) -> bool {
    why_not_shapeable(def).is_none()
}

/// Pourquoi ce bloc ne se taille pas (`None` : il se taille).
pub fn why_not_shapeable(def: &BlockDef) -> Option<&'static str> {
    if def.shape != Shape::Cube {
        Some("seuls les cubes pleins se taillent (pas les plantes, les modèles, les rails)")
    } else if def.liquid {
        Some("un liquide ne se taille pas")
    } else if !(def.solid || def.transparent) {
        Some("ce bloc se traverse")
    } else if def.behavior.is_some() || def.base.is_some() {
        Some("un bloc qui agit (machine, herbe, feuilles, coffre...) ne se taille pas")
    } else if !def.placeable || def.key == UNKNOWN_MATERIAL {
        Some("ce bloc n'existe pas en objet")
    } else {
        None
    }
}

/// Ce bloc est-il celui des formes ?
pub fn is_shaped_block(def: &BlockDef) -> bool {
    def.behavior.as_deref() == Some(BEHAVIOR)
}

// ----- Lire et écrire une case -----

/// Le contenu de la case dont l'entité est `entity` (vide si elle n'en a pas, ou si ses octets sont
/// illisibles).
pub fn read(entity: Option<&BlockEntity>) -> Shaped {
    entity
        .and_then(|e| Shaped::decode(&e.data))
        .unwrap_or_default()
}

/// Écrit le contenu d'une case dans le monde : vide, elle devient de l'air ; pleine d'un seul
/// matériau, le bloc de ce matériau (s'il existe) ; sinon une case de formes, dont l'entité porte
/// les pièces. `false` si le chunk n'est pas chargé ou si le registre n'a pas le bloc de formes.
pub fn store(streamer: &mut crate::ChunkStreamer, pos: IVec3, shaped: &Shaped) -> bool {
    let registry = streamer.world().registry().clone();
    let simplified = match shaped.simplified() {
        Simplified::Empty => return streamer.set_block(pos, BlockId::AIR),
        Simplified::Full(material) => registry
            .id(material)
            .filter(|&id| is_shapeable(registry.def(id))),
        Simplified::Shaped => None,
    };
    if let Some(block) = simplified {
        return streamer.set_block(pos, block);
    }
    let Some(shaped_block) = registry.id(BEHAVIOR) else {
        return false;
    };
    if streamer.world().block(pos) != shaped_block && !streamer.set_block(pos, shaped_block) {
        return false;
    }
    streamer.set_block_entity(
        pos,
        Some(BlockEntity {
            data: shaped.encode(),
            ..BlockEntity::default()
        }),
    )
}

/// Le bloc de sol que le terrain lointain voit pour cette case : le matériau principal, si la case
/// est pleine à plus de moitié.
pub(crate) fn ground_block(
    registry: &crate::BlockRegistry,
    entity: Option<&BlockEntity>,
) -> Option<BlockId> {
    let shaped = read(entity);
    if volume(&shaped.occupancy()) * 2 <= (N * N * N) as u32 {
        return None;
    }
    registry
        .id(shaped.dominant_material()?)
        .filter(|&id| is_shapeable(registry.def(id)))
}

// ----- Les objets de pièces -----

/// Les six sortes de pièces qu'on tient en main.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PieceKind {
    /// Une couche de 1/16.
    Layer,
    /// Une dalle (8/16).
    Slab,
    /// Un niveau de pente.
    SlopeLayer,
    /// La pente pleine (niveau 16).
    Slope,
    /// Un cube de 1/8 (2 × 2 × 2 cellules).
    Micro,
    /// Un grain de 1/64.
    Nano,
}

impl PieceKind {
    pub const ALL: [PieceKind; 6] = [
        PieceKind::Layer,
        PieceKind::Slab,
        PieceKind::SlopeLayer,
        PieceKind::Slope,
        PieceKind::Micro,
        PieceKind::Nano,
    ];

    /// L'identifiant de l'objet.
    pub fn key(self) -> &'static str {
        match self {
            PieceKind::Layer => "core:shape_layer",
            PieceKind::Slab => "core:shape_slab",
            PieceKind::SlopeLayer => "core:shape_slope_layer",
            PieceKind::Slope => "core:shape_slope",
            PieceKind::Micro => "core:shape_micro",
            PieceKind::Nano => "core:shape_nano",
        }
    }

    pub fn from_key(key: &str) -> Option<PieceKind> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Le nom de la forme (le matériau s'y ajoute : « Dalle de chêne »).
    pub fn name(self) -> &'static str {
        match self {
            PieceKind::Layer => "Couche",
            PieceKind::Slab => "Dalle",
            PieceKind::SlopeLayer => "Couche de pente",
            PieceKind::Slope => "Pente",
            PieceKind::Micro => "Microbloc",
            PieceKind::Nano => "Grain",
        }
    }

    /// Combien de pièces donne un bloc plein, et combien il en faut pour le reconstituer.
    pub fn per_block(self) -> u32 {
        match self {
            PieceKind::Layer | PieceKind::SlopeLayer => 16,
            PieceKind::Slab | PieceKind::Slope => 2,
            PieceKind::Micro => 8,
            PieceKind::Nano => 64,
        }
    }

    /// Le temps de minage d'une pièce, en part du bloc plein.
    pub fn is_slope(self) -> bool {
        matches!(self, PieceKind::SlopeLayer | PieceKind::Slope)
    }
}

/// Une pile de cette sorte d'objet s'empile-t-elle par son `damage` (le numéro de matériau) ?
/// Les autres objets réservent `damage` à l'usure : seules les piles neuves s'empilent.
pub fn stacks_by_damage(item_key: &str) -> bool {
    PieceKind::from_key(item_key).is_some()
}

// ----- Poser une pièce -----

/// Où et comment poser une pièce quand on vise un point. `point` est le point visé, en blocs, relatif
/// à la case visée (0 à 1) ; `normal` la normale de la face visée (un axe) ; `look` le regard du
/// joueur.
///
/// Renvoie la pièce à ajouter **dans la case visée** quand la face visée est à l'intérieur de la case
/// (le dessus d'une dalle, par exemple), et `None` quand elle est au bord de la case : la pièce va
/// alors dans la case voisine ([`plan_outer`]).
pub fn plan_inner(
    kind: PieceKind,
    existing: &Shaped,
    point: Vec3,
    normal: IVec3,
    look: Vec3,
) -> Option<Piece> {
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
        PieceKind::SlopeLayer | PieceKind::Slope => {
            // Sur une pente de la case : la même orientation, pour qu'elle monte de niveau.
            let level = if kind == PieceKind::Slope { 16 } else { 1 };
            let aimed = existing.part_hit(point, normal.as_vec3());
            if let Some(Part {
                piece: Piece::Slope {
                    facing, ceiling, ..
                },
                ..
            }) = aimed.map(|i| existing.parts[i])
            {
                return Some(Piece::Slope {
                    facing,
                    ceiling,
                    level,
                });
            }
            let ceiling = normal.y < 0 || (normal.y == 0 && point.y >= 0.5);
            Some(Piece::Slope {
                facing: facing_of(look),
                ceiling,
                level,
            })
        }
        PieceKind::Micro | PieceKind::Nano => {
            let size = if kind == PieceKind::Micro { 2 } else { 1 };
            let beyond = point + normal.as_vec3() * 0.125;
            Some(micro_at(beyond, size))
        }
    }
}

/// La pièce à poser dans la **case voisine** de la case visée (derrière la face de normale
/// `normal`). `point` est le point visé, relatif à la case visée.
pub fn plan_outer(kind: PieceKind, point: Vec3, normal: IVec3, look: Vec3) -> Piece {
    let axis = axis_of(normal).unwrap_or(1);
    // Le point, dans la case voisine : de l'autre côté du plan, serré contre lui.
    let mut local = point - normal.as_vec3();
    local[axis] = if normal[axis] > 0 { 0.001 } else { 0.999 };
    let anchor = face_of(axis, normal[axis] < 0);
    match kind {
        PieceKind::Layer | PieceKind::Slab => Piece::Layer {
            face: anchor,
            offset: 0,
            thickness: if kind == PieceKind::Slab { 8 } else { 1 },
        },
        PieceKind::SlopeLayer | PieceKind::Slope => Piece::Slope {
            facing: facing_of(look),
            ceiling: normal.y < 0 || (normal.y == 0 && point.y >= 0.5),
            level: if kind == PieceKind::Slope { 16 } else { 1 },
        },
        PieceKind::Micro => micro_at(local, 2),
        PieceKind::Nano => micro_at(local, 1),
    }
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
    FACING_DIRECTIONS[usize::from(facing % 4)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const OAK: &str = "core:oak_planks";
    const STONE: &str = "core:stone";

    fn bottom(offset: u8, thickness: u8) -> Piece {
        Piece::Layer {
            face: Face::NegY,
            offset,
            thickness,
        }
    }

    fn top(offset: u8, thickness: u8) -> Piece {
        Piece::Layer {
            face: Face::PosY,
            offset,
            thickness,
        }
    }

    #[test]
    fn layers_of_the_same_material_merge() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 3), OAK).unwrap();
        s.try_add(bottom(3, 5), OAK).unwrap();
        assert_eq!(s.parts.len(), 1);
        assert_eq!(s.parts[0].piece, bottom(0, 8));
        // Une couche du même côté s'épaissit aussi par l'autre bout.
        s.try_add(bottom(8, 1), OAK).unwrap();
        assert_eq!(s.parts[0].piece, bottom(0, 9));
    }

    #[test]
    fn different_materials_stay_separate() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 8), STONE).unwrap();
        s.try_add(top(0, 8), OAK).unwrap();
        assert_eq!(s.parts.len(), 2);
        assert_eq!(s.materials, [STONE, OAK]);
        assert!(s.is_full());
        // Pleine, mais de deux matériaux : elle reste une case de formes.
        assert_eq!(s.simplified(), Simplified::Shaped);
    }

    #[test]
    fn overlap_is_refused() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 8), STONE).unwrap();
        assert_eq!(s.try_add(bottom(0, 1), OAK), Err(AddError::Overlap));
        assert_eq!(s.try_add(bottom(0, 1), STONE), Err(AddError::Overlap));
        assert_eq!(s.try_add(top(4, 8), STONE), Err(AddError::Overlap));
        // Le refus ne laisse pas un matériau inutile dans la liste.
        assert_eq!(s.materials, [STONE]);
        assert_eq!(s.try_add(top(0, 0), STONE), Err(AddError::Invalid));
        assert_eq!(s.try_add(top(10, 8), STONE), Err(AddError::Invalid));
    }

    #[test]
    fn stacking_up_to_full_simplifies() {
        let mut s = Shaped::new();
        for i in 0..16 {
            assert!(s.simplified() == Simplified::Shaped || i == 0);
            s.try_add(bottom(i, 1), OAK).unwrap();
        }
        assert!(s.is_full());
        assert_eq!(s.simplified(), Simplified::Full(OAK));
        assert_eq!(Shaped::new().simplified(), Simplified::Empty);
    }

    #[test]
    fn limits() {
        let mut s = Shaped::new();
        for i in 0..16 {
            s.try_add(bottom(i, 1), &format!("mod:m{i}")).unwrap();
        }
        assert_eq!(s.try_add(top(0, 0), "mod:autre"), Err(AddError::Invalid));
        assert_eq!(
            s.try_add(Piece::Micro { cells: 1 }, "mod:autre"),
            Err(AddError::TooManyMaterials)
        );
        assert_eq!(s.materials.len(), 16);
        // Trente-deux pièces au plus.
        let mut s = Shaped::new();
        s.materials.push(OAK.to_string());
        s.parts = (0..MAX_PARTS)
            .map(|i| Part {
                piece: Piece::Micro { cells: 1 << i },
                material: 0,
            })
            .collect();
        assert_eq!(
            s.try_add(Piece::Micro { cells: 1 << 40 }, STONE),
            Err(AddError::TooManyParts)
        );
        assert_eq!(s.materials, [OAK]);
    }

    #[test]
    fn slope_occupancy_at_every_level() {
        for level in 1..=16u8 {
            let piece = Piece::Slope {
                facing: 0,
                ceiling: false,
                level,
            };
            let occ = piece.occupancy();
            // Contre le bord haut (z = 0) la colonne est la plus haute ; contre le bord bas, la plus basse.
            let column = |z: usize| (0..N).filter(|&y| is_set(&occ, 3, y, z)).count();
            assert_eq!(column(0), slope_height(level, 0));
            assert_eq!(column(15), slope_height(level, 15));
            assert!(column(0) >= column(8) && column(8) >= column(15));
            // Plein de bas en haut : aucune cellule en l'air.
            for z in 0..N {
                let h = column(z);
                assert!((0..h).all(|y| is_set(&occ, 3, y, z)));
            }
        }
        let full_slope = Piece::Slope {
            facing: 0,
            ceiling: false,
            level: 16,
        }
        .occupancy();
        // Un peu plus d'un demi-bloc : 136 cellules sur 256 dans chaque rangée de x.
        assert_eq!(volume(&full_slope), 136 * 16);
        // Retournée, la pente occupe le haut.
        let ceiling = Piece::Slope {
            facing: 0,
            ceiling: true,
            level: 16,
        }
        .occupancy();
        assert!(is_set(&ceiling, 0, 15, 15) && !is_set(&ceiling, 0, 0, 15));
        assert!(is_set(&ceiling, 0, 0, 0));
        // Chaque orientation a son bord haut de son côté.
        for (facing, x, z) in [(1u8, 15, 8), (2, 8, 15), (3, 0, 8)] {
            let occ = Piece::Slope {
                facing,
                ceiling: false,
                level: 16,
            }
            .occupancy();
            assert!(is_set(&occ, x, 15, z), "orientation {facing}");
        }
    }

    #[test]
    fn slope_boxes_are_a_staircase() {
        let piece = Piece::Slope {
            facing: 1,
            ceiling: false,
            level: 16,
        };
        let boxes = piece.int_boxes();
        assert_eq!(boxes.len(), 16);
        // Côté haut à l'est : la marche la plus haute touche x = 16.
        let tallest = boxes.iter().max_by_key(|b| b[4]).unwrap();
        assert_eq!((tallest[3], tallest[4]), (16, 16));
        // La somme des boîtes remplit exactement l'occupation.
        let mut occ = [0u16; 256];
        for b in &boxes {
            for y in b[1]..b[4] {
                for z in b[2]..b[5] {
                    for x in b[0]..b[3] {
                        set(&mut occ, usize::from(x), usize::from(y), usize::from(z));
                    }
                }
            }
        }
        assert_eq!(occ, piece.occupancy());
        // Un niveau bas fusionne les colonnes de même hauteur.
        let low = Piece::Slope {
            facing: 2,
            ceiling: true,
            level: 2,
        };
        assert!(low.int_boxes().len() <= 4);
    }

    #[test]
    fn slopes_climb_level_by_level() {
        let mut s = Shaped::new();
        let slope = |level| Piece::Slope {
            facing: 3,
            ceiling: false,
            level,
        };
        for _ in 0..16 {
            s.try_add(slope(1), STONE).unwrap();
        }
        assert_eq!(s.parts.len(), 1);
        assert_eq!(s.parts[0].piece, slope(16));
        assert_eq!(s.try_add(slope(1), STONE), Err(AddError::Overlap));
        // Une pente d'un autre matériau, au même endroit, est refusée.
        assert_eq!(s.try_add(slope(1), OAK), Err(AddError::Overlap));
    }

    #[test]
    fn micro_blocks_merge_and_box_up() {
        let mut s = Shaped::new();
        s.try_add(
            Piece::Micro {
                cells: micro_mask(0, 0, 0, 2),
            },
            OAK,
        )
        .unwrap();
        s.try_add(
            Piece::Micro {
                cells: micro_mask(2, 0, 0, 2),
            },
            OAK,
        )
        .unwrap();
        assert_eq!(s.parts.len(), 1);
        // Deux cubes voisins ne font qu'une boîte de 1/4 × 1/8... : 4 × 2 × 2 cellules.
        assert_eq!(s.boxes().len(), 1);
        assert_eq!(
            s.try_add(
                Piece::Micro {
                    cells: micro_mask(1, 1, 1, 1)
                },
                STONE
            ),
            Err(AddError::Overlap)
        );
        s.try_add(
            Piece::Micro {
                cells: micro_mask(3, 3, 3, 1),
            },
            STONE,
        )
        .unwrap();
        assert_eq!(s.parts.len(), 2);
        // Un microbloc aligné rend un objet « microbloc », un grain seul un « grain ».
        assert_eq!(s.parts[0].piece.drops(), [(PieceKind::Micro, 2)]);
        assert_eq!(s.parts[1].piece.drops(), [(PieceKind::Nano, 1)]);
    }

    #[test]
    fn drops_follow_the_piece() {
        assert_eq!(bottom(0, 5).drops(), [(PieceKind::Layer, 5)]);
        assert_eq!(
            bottom(0, 13).drops(),
            [(PieceKind::Slab, 1), (PieceKind::Layer, 5)]
        );
        let slope = |level| Piece::Slope {
            facing: 0,
            ceiling: false,
            level,
        };
        assert_eq!(slope(16).drops(), [(PieceKind::Slope, 1)]);
        assert_eq!(slope(7).drops(), [(PieceKind::SlopeLayer, 7)]);
    }

    #[test]
    fn round_trip_keeps_unknown_materials() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 4), "mod:inconnu").unwrap();
        s.try_add(
            Piece::Slope {
                facing: 2,
                ceiling: true,
                level: 6,
            },
            OAK,
        )
        .unwrap();
        s.try_add(
            Piece::Micro {
                cells: 1 << 16 | 1 << 17,
            },
            STONE,
        )
        .unwrap();
        let bytes = s.encode();
        let back = Shaped::decode(&bytes).unwrap();
        assert_eq!(back, s);
        assert!(back.materials.contains(&"mod:inconnu".to_string()));
        // La lecture légère donne les mêmes boîtes.
        assert_eq!(Shaped::boxes_of(&bytes).unwrap(), s.boxes());
        // Des octets abîmés ou d'une autre version : refusés, sans panique.
        assert!(Shaped::decode(&[]).is_none());
        assert!(Shaped::decode(&[9, 0, 0]).is_none());
        for cut in 0..bytes.len() {
            assert!(Shaped::decode(&bytes[..cut]).is_none());
        }
        // Un matériau hors liste ou une pièce hors bornes.
        assert!(Shaped::decode(&[1, 0, 1, 5, 0, 0, 0, 1]).is_none());
        assert!(Shaped::decode(&[1, 1, 1, b'a', 1, 0, 0, 0, 20]).is_none());
    }

    #[test]
    fn removing_the_aimed_piece() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 8), STONE).unwrap();
        s.try_add(top(0, 8), OAK).unwrap();
        // Visée du dessus (la face haute de la dalle de chêne) : le chêne.
        let (piece, material) = s.remove_at(Vec3::new(0.5, 1.0, 0.5), Vec3::Y).unwrap();
        assert_eq!((piece, material.as_str()), (top(0, 8), OAK));
        // Le chêne n'est plus dans la liste.
        assert_eq!(s.materials, [STONE]);
        let (piece, _) = s.remove_at(Vec3::new(0.5, 0.5, 0.5), Vec3::Y).unwrap();
        assert_eq!(piece, bottom(0, 8));
        assert!(s.is_empty());
        assert!(s.remove_at(Vec3::splat(0.5), Vec3::Y).is_none());
    }

    #[test]
    fn dominant_material_is_the_biggest_volume() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 3), STONE).unwrap();
        s.try_add(top(0, 5), OAK).unwrap();
        assert_eq!(s.dominant_material(), Some(OAK));
        assert_eq!(Shaped::new().dominant_material(), None);
    }

    #[test]
    fn turning_four_times_is_the_identity() {
        let mut s = Shaped::new();
        s.try_add(
            Piece::Layer {
                face: Face::PosX,
                offset: 0,
                thickness: 1,
            },
            OAK,
        )
        .unwrap();
        s.try_add(
            Piece::Slope {
                facing: 3,
                ceiling: true,
                level: 5,
            },
            STONE,
        )
        .unwrap();
        s.try_add(
            Piece::Micro {
                cells: micro_mask(0, 0, 3, 1) | 1 << 20,
            },
            OAK,
        )
        .unwrap();
        let mut t = s.clone();
        for _ in 0..4 {
            t = t.turned(1);
        }
        assert_eq!(t, s);
        // Un quart de tour : +X devient +Z, la pente d'ouest regarde le nord.
        let once = s.turned(1);
        assert!(matches!(
            once.parts[0].piece,
            Piece::Layer {
                face: Face::PosZ,
                ..
            }
        ));
        assert!(matches!(
            once.parts[1].piece,
            Piece::Slope {
                facing: 0,
                ceiling: true,
                ..
            }
        ));
        // L'occupation tourne avec la case : la tranche x = 15 passe en z = 15.
        let occ = once.parts[0].piece.occupancy();
        assert!(is_set(&occ, 4, 4, 15) && !is_set(&occ, 15, 4, 4));
        // Les microblocs : la cellule (x, z) va en (3 - z, x).
        let m = Piece::Micro {
            cells: 1 << (4 * 3),
        }
        .turned(1);
        assert_eq!(m, Piece::Micro { cells: 1 });
        assert_eq!(Shaped::turned_data(&[0], 1), [0]);
        let data = s.encode();
        assert_eq!(Shaped::turned_data(&data, 1), once.encode());
    }

    #[test]
    fn plans_for_a_new_neighbour_cell() {
        let look = Vec3::new(0.0, 0.0, 1.0);
        // Un mur à l'est de la case : la couche se colle à la face -X de la case voisine.
        let piece = plan_outer(PieceKind::Slab, Vec3::new(1.0, 0.4, 0.5), IVec3::X, look);
        assert_eq!(
            piece,
            Piece::Layer {
                face: Face::NegX,
                offset: 0,
                thickness: 8
            }
        );
        // Une pente au sol : son côté haut est celui du regard (la pente regarde le joueur).
        let piece = plan_outer(PieceKind::Slope, Vec3::new(0.5, 1.0, 0.5), IVec3::Y, look);
        assert_eq!(
            piece,
            Piece::Slope {
                facing: 2,
                ceiling: false,
                level: 16
            }
        );
        // Sous un plafond : retournée.
        let piece = plan_outer(
            PieceKind::SlopeLayer,
            Vec3::new(0.5, 0.0, 0.5),
            IVec3::NEG_Y,
            look,
        );
        assert!(matches!(
            piece,
            Piece::Slope {
                ceiling: true,
                level: 1,
                ..
            }
        ));
        // Un microbloc sur le coin visé.
        let piece = plan_outer(PieceKind::Micro, Vec3::new(0.9, 1.0, 0.1), IVec3::Y, look);
        assert_eq!(
            piece,
            Piece::Micro {
                cells: micro_mask(2, 0, 0, 2)
            }
        );
    }

    #[test]
    fn plans_inside_a_shaped_cell() {
        let mut s = Shaped::new();
        s.try_add(bottom(0, 8), STONE).unwrap();
        let look = Vec3::Z;
        // Le dessus de la dalle de pierre est à l'intérieur de la case : la dalle de chêne s'y pose.
        let top_face = Vec3::new(0.5, 0.5, 0.5);
        let piece = plan_inner(PieceKind::Slab, &s, top_face, IVec3::Y, look).unwrap();
        assert_eq!(piece, bottom(8, 8));
        s.try_add(piece, OAK).unwrap();
        assert!(s.is_full());
        // Le bord de la case : rien à faire ici, la pièce va dans la case voisine.
        assert!(
            plan_inner(
                PieceKind::Slab,
                &s,
                Vec3::new(0.5, 1.0, 0.5),
                IVec3::Y,
                look
            )
            .is_none()
        );
        // Un grain sur le dessus de la dalle.
        let mut s = Shaped::new();
        s.try_add(bottom(0, 8), STONE).unwrap();
        let piece = plan_inner(
            PieceKind::Nano,
            &s,
            Vec3::new(0.6, 0.5, 0.1),
            IVec3::Y,
            look,
        )
        .unwrap();
        assert_eq!(
            piece,
            Piece::Micro {
                cells: micro_mask(2, 2, 0, 1)
            }
        );
        s.try_add(piece, OAK).unwrap();
    }

    #[test]
    fn a_slope_aimed_gets_its_orientation() {
        let mut s = Shaped::new();
        let slope = Piece::Slope {
            facing: 3,
            ceiling: false,
            level: 4,
        };
        s.try_add(slope, STONE).unwrap();
        // On regarde vers le nord, mais la pente visée est tournée à l'ouest : le niveau s'ajoute.
        let piece = plan_inner(
            PieceKind::SlopeLayer,
            &s,
            Vec3::new(0.1, 0.25, 0.5),
            IVec3::Y,
            Vec3::NEG_Z,
        )
        .unwrap();
        s.try_add(piece, STONE).unwrap();
        assert_eq!(s.parts.len(), 1);
        assert!(matches!(
            s.parts[0].piece,
            Piece::Slope {
                facing: 3,
                level: 5,
                ..
            }
        ));
    }

    #[test]
    fn what_can_be_shaped() {
        let registry = crate::BlockRegistry::builtin();
        let shapeable = |key: &str| is_shapeable(registry.def(registry.id(key).unwrap()));
        assert!(shapeable("core:oak_planks"));
        assert!(shapeable("core:stone"));
        assert!(shapeable("core:glass"));
        assert!(!shapeable("core:chest"));
        assert!(!shapeable("core:furnace"));
        assert!(!shapeable("core:water"));
        assert!(!shapeable(BEHAVIOR));
    }

    #[test]
    fn the_registry_knows_the_shaped_block() {
        let registry = crate::BlockRegistry::builtin();
        let id = registry.id(BEHAVIOR).expect("core:shaped");
        let def = registry.def(id);
        assert!(is_shaped_block(def));
        assert_eq!(def.shape, Shape::Model);
        assert!(!def.placeable);
    }

    #[test]
    fn storing_a_cell_simplifies_it() {
        use crate::generator::TerrainGenerator;
        use crate::streaming::{StreamingConfig, tests::pump_until_idle};
        let config = StreamingConfig {
            view_radius: 1,
            ..StreamingConfig::default()
        };
        let mut s = crate::ChunkStreamer::new(TerrainGenerator::new(7), config);
        pump_until_idle(&mut s, Vec3::new(5.0, 100.0, 5.0));
        let id = |s: &crate::ChunkStreamer, key: &str| s.world().registry().id(key).unwrap();
        let pos = IVec3::new(8, 140, 8);
        // Une dalle : une case de formes, avec son contenu dans l'entité.
        let mut cell = Shaped::new();
        cell.try_add(bottom(0, 8), STONE).unwrap();
        assert!(store(&mut s, pos, &cell));
        assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
        assert_eq!(read(s.block_entity(pos)), cell);
        // Deux dalles de pierre : un vrai bloc de pierre, sans entité.
        cell.try_add(bottom(8, 8), STONE).unwrap();
        assert!(store(&mut s, pos, &cell));
        assert_eq!(s.world().block(pos), id(&s, STONE));
        assert!(s.block_entity(pos).is_none());
        // Pleine de deux matériaux : toujours une case de formes.
        let mut mixed = Shaped::new();
        mixed.try_add(bottom(0, 8), STONE).unwrap();
        mixed.try_add(bottom(8, 8), OAK).unwrap();
        assert!(store(&mut s, pos, &mixed));
        assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
        assert_eq!(s.world().block_boxes(pos).len(), 2);
        // Vide : de l'air.
        assert!(store(&mut s, pos, &Shaped::new()));
        assert!(s.world().block(pos).is_air());
        // Un matériau retiré du registre : la case pleine reste lisible comme case de formes.
        let mut lost = Shaped::new();
        lost.try_add(bottom(0, 16), "mod:disparu").unwrap();
        assert!(store(&mut s, pos, &lost));
        assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
        assert_eq!(read(s.block_entity(pos)).materials, ["mod:disparu"]);
    }

    #[test]
    fn stackable_pieces() {
        assert!(stacks_by_damage("core:shape_slab"));
        assert!(!stacks_by_damage("core:iron_pickaxe"));
    }
}
