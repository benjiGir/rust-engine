//! Les formes : le bloc `core:shaped`, une case qui contient des **pièces** (couches, pentes, escaliers,
//! microblocs, murets...) au seizième de bloc près, chacune avec son propre matériau.
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
//!   un escalier de seizièmes (au plus 16 groupes de colonnes), la marche automatique du joueur les
//!   monte ;
//! - **se simplifier** ([`Shaped::simplified`]) : vide, elle redevient de l'air ; pleine d'un seul
//!   matériau, elle redevient le vrai bloc de ce matériau ;
//! - **tourner** ([`Shaped::turned`]) avec les structures mobiles qui se reposent.
//!
//! # Les modules
//!
//! | Module | Rôle |
//! |---|---|
//! | `grid` | la grille d'occupation 16³ et les petites fonctions de boîtes et de rotations |
//! | `piece` | [`Piece`] : occupation, boîtes, rendu en objets, rotation, fusion |
//! | `slope` | les pentes : 24 orientations, coins sortants et rentrants, faces du maillage |
//! | `connected` | les formes raccordées, selon leurs voisins |
//! | `kind` | [`PieceKind`] : les sortes d'objets, leurs noms, leurs rendements |
//! | `format` | le format sauvegardé (version 2, version 1 relue) |
//! | `place` | où poser une pièce selon le point visé |
//!
//! # Les règles de la case
//!
//! - Une pièce **fusionne** avec une pièce du même matériau qui la touche : une couche s'épaissit, une
//!   pente monte d'un niveau, une demi-couche s'allonge, des microblocs se réunissent.
//! - Une pièce qui **chevauche** une autre est refusée, y compris d'un autre matériau.
//! - Une **forme raccordée** occupe seule sa case ([`AddError::Alone`]).
//! - Des **planchettes** peuvent couvrir plusieurs faces d'une même case : la seconde, aux arêtes, est
//!   rognée d'un seizième pour ne pas recouvrir la première.
//! - Une **tranche** (d'arête ou de coin) grandit d'un cran quand on en pose une autre dessus
//!   ([`Shaped::try_grow`]) ; l'objet coûte la différence de volume, casser rend le total : rien ne
//!   se perd ni ne se crée.
//!
//! # Le format
//!
//! Voir `format.rs`. [`MAX_MATERIALS`] matériaux et [`MAX_PARTS`] pièces au plus par case.

use glam::{IVec3, Vec3};

use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::registry::{BlockBox, BlockDef, Shape};

pub mod connected;
mod format;
pub(crate) mod grid;
pub mod kind;
mod piece;
mod place;
pub mod slope;
#[cfg(test)]
mod tests;

pub use connected::{Link, Neighbours};
pub use grid::Occupancy;
pub use kind::{Family, PieceKind, corner_slice_cost, slice_cost, stacks_by_damage};
pub use piece::{Piece, micro_mask};
pub use place::{facing_direction, plan_inner, plan_outer};
pub use slope::{Corner, corner_up, slope_height};

use grid::{IntBox, N, empty, full, overlaps, union, volume};

/// Le comportement (et la clé) du bloc de formes.
pub const BEHAVIOR: &str = "core:shaped";
/// Le matériau montré à la place d'un matériau que le registre ne connaît pas.
pub const UNKNOWN_MATERIAL: &str = "core:unknown";
/// Au plus ce nombre de matériaux différents dans une case.
pub const MAX_MATERIALS: usize = 16;
/// Au plus ce nombre de pièces dans une case.
pub const MAX_PARTS: usize = 32;

pub(crate) use grid::is_set;

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
    /// Une forme raccordée occupe seule sa case : rien ne s'y ajoute, et elle ne s'ajoute à rien.
    Alone,
    /// Il manque des objets pour agrandir une tranche.
    NotEnough,
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

    /// La forme raccordée qui occupe cette case, s'il y en a une.
    pub fn link(&self) -> Option<Link> {
        match self.parts.as_slice() {
            [
                Part {
                    piece: Piece::Connected { kind, .. },
                    ..
                },
            ] => Some(*kind),
            _ => None,
        }
    }

    /// La forme raccordée d'une case encodée (lecture légère, sans les noms de matériaux).
    pub fn link_of(data: &[u8]) -> Option<Link> {
        format::parse(data, false)?.link()
    }

    /// Tout ce que remplissent les pièces.
    pub fn occupancy(&self) -> Occupancy {
        self.occupancy_where(|_| true)
    }

    /// Ce que remplissent les pièces dont le matériau satisfait `keep` (le maillage ne cache une
    /// face que derrière une pièce opaque).
    pub fn occupancy_where(&self, keep: impl Fn(&str) -> bool) -> Occupancy {
        self.occupancy_with(&Neighbours::NONE, keep)
    }

    /// Comme [`Shaped::occupancy_where`], une forme raccordée ayant les voisins `n`.
    pub fn occupancy_with(&self, n: &Neighbours, keep: impl Fn(&str) -> bool) -> Occupancy {
        self.parts
            .iter()
            .filter(|p| keep(self.material_of(p)))
            .fold(empty(), |acc, p| union(&acc, &p.piece.occupancy_with(n)))
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
    /// Ce qui chevauche une autre pièce est refusé ; une planchette est rognée contre une autre.
    pub fn try_add(&mut self, piece: Piece, material: &str) -> Result<(), AddError> {
        if !piece.is_valid() {
            return Err(AddError::Invalid);
        }
        // Une forme raccordée occupe seule sa case.
        let connected = matches!(piece, Piece::Connected { .. });
        if (connected && !self.parts.is_empty()) || self.link().is_some() {
            return Err(AddError::Alone);
        }
        let piece = self.fitted(piece).ok_or(AddError::Overlap)?;
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

    /// Une planchette qui toucherait une autre planchette (collée à une autre face) est rognée d'un
    /// seizième du côté de l'arête commune. `None` si elle ne tient plus.
    fn fitted(&self, piece: Piece) -> Option<Piece> {
        let Piece::Cuboid {
            mut min,
            mut max,
            kind: PieceKind::Board,
            count,
        } = piece
        else {
            return Some(piece);
        };
        // La planchette est fine selon un seul axe.
        let thin = |min: &[u8; 3], max: &[u8; 3]| (0..3).find(|&a| max[a] - min[a] == 1);
        let a = thin(&min, &max)?;
        for part in &self.parts {
            let Piece::Cuboid {
                min: other_min,
                max: other_max,
                kind: PieceKind::Board,
                ..
            } = part.piece
            else {
                continue;
            };
            let Some(b) = thin(&other_min, &other_max).filter(|&b| b != a) else {
                continue;
            };
            let touching = (0..3).all(|i| min[i] < other_max[i] && max[i] > other_min[i]);
            if !touching {
                continue;
            }
            if other_min[b] == 0 {
                min[b] = min[b].max(other_max[b]);
            } else {
                max[b] = max[b].min(other_min[b]);
            }
        }
        let fitted = Piece::Cuboid {
            min,
            max,
            kind: PieceKind::Board,
            count,
        };
        fitted.is_valid().then_some(fitted)
    }

    fn add_part(&mut self, piece: Piece, material: u8) -> Result<(), AddError> {
        let merged = self.parts.iter().enumerate().find_map(|(i, p)| {
            (p.material == material)
                .then(|| piece::merge(p.piece, piece))
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
            .fold(empty(), |acc, (_, p)| union(&acc, &p.piece.occupancy()))
    }

    /// Agrandit d'un cran la tranche `index` (voir [`Piece::grown`]) et rend le nombre d'objets
    /// consommés. `available` : combien d'objets le joueur tient.
    pub fn try_grow(&mut self, index: usize, available: u32) -> Result<u32, AddError> {
        let part = *self.parts.get(index).ok_or(AddError::Invalid)?;
        let (grown, extra) = part.piece.grown().ok_or(AddError::Invalid)?;
        if extra > available {
            return Err(AddError::NotEnough);
        }
        if overlaps(&self.occupancy_without(Some(index)), &grown.occupancy()) {
            return Err(AddError::Overlap);
        }
        self.parts[index].piece = grown;
        Ok(extra)
    }

    /// La tranche de la sorte `kind` que touche le rayon (celle qu'on agrandit en en posant une autre).
    pub fn resizable_hit(&self, point: Vec3, normal: Vec3, kind: PieceKind) -> Option<usize> {
        let index = self.part_hit(point, normal)?;
        matches!(
            self.parts[index].piece,
            Piece::Cuboid { kind: k, .. } if k == kind
        )
        .then_some(index)
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
                    if let Some(merged) = piece::merge(a, b) {
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

    /// Les boîtes de la case, en blocs (0 à 1) : la collision et la visée (une forme raccordée sans
    /// voisins).
    pub fn boxes(&self) -> Vec<BlockBox> {
        self.boxes_with(&Neighbours::NONE)
    }

    /// Les boîtes de la case, une forme raccordée ayant les voisins `n`.
    pub fn boxes_with(&self, n: &Neighbours) -> Vec<BlockBox> {
        self.parts
            .iter()
            .flat_map(|p| p.piece.int_boxes_with(n))
            .map(to_block_box)
            .collect()
    }

    /// Les boîtes de collision de la case, une forme raccordée ayant les voisins `n` : une barrière
    /// monte à 1,5 bloc, un portillon ouvert n'en a plus.
    pub fn collision_with(&self, n: &Neighbours) -> Vec<BlockBox> {
        self.parts
            .iter()
            .flat_map(|p| p.piece.collision_boxes_with(n))
            .map(to_block_box)
            .collect()
    }

    /// Les boîtes d'une pièce, en blocs.
    pub fn part_boxes(&self, index: usize) -> Vec<BlockBox> {
        self.part_boxes_with(index, &Neighbours::NONE)
    }

    /// Les boîtes d'une pièce, en blocs, une forme raccordée ayant les voisins `n`.
    pub fn part_boxes_with(&self, index: usize, n: &Neighbours) -> Vec<BlockBox> {
        self.parts
            .get(index)
            .map(|p| {
                p.piece
                    .int_boxes_with(n)
                    .into_iter()
                    .map(to_block_box)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// La pièce que contient ce point (en blocs, 0 à 1), à un 1/64 de bloc près : la plus proche.
    pub fn part_at(&self, point: Vec3) -> Option<usize> {
        self.part_at_with(point, &Neighbours::NONE)
    }

    /// Comme [`Shaped::part_at`], une forme raccordée ayant les voisins `n`.
    pub fn part_at_with(&self, point: Vec3, n: &Neighbours) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, _) in self.parts.iter().enumerate() {
            for b in self.part_boxes_with(i, n) {
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
        format::encode(self)
    }

    /// Relit une case. `None` si les octets sont d'une version inconnue ou abîmés (les versions 1 et
    /// 2 se relisent).
    pub fn decode(data: &[u8]) -> Option<Shaped> {
        format::parse(data, true)
    }

    /// Les boîtes d'une case encodée, sans en lire les noms de matériaux (la collision et la
    /// visée les demandent souvent), une forme raccordée sans voisins.
    pub fn boxes_of(data: &[u8]) -> Option<Vec<BlockBox>> {
        format::parse(data, false).map(|s| s.boxes())
    }

    /// Les données d'une case après `k` quarts de tour (structures mobiles). Des octets illisibles
    /// sont rendus tels quels.
    pub fn turned_data(data: &[u8], k: u8) -> Vec<u8> {
        Self::decode(data).map_or_else(|| data.to_vec(), |s| s.turned(k).encode())
    }
}

fn to_block_box(b: IntBox) -> BlockBox {
    let v = |a: u8| f32::from(a) / 16.0;
    BlockBox {
        min: Vec3::new(v(b[0]), v(b[1]), v(b[2])),
        max: Vec3::new(v(b[3]), v(b[4]), v(b[5])),
    }
}

/// Les boîtes d'une case de formes **en place** dans le monde, selon ses voisins : `collision` demande
/// celles de la collision (une barrière monte à 1,5 bloc ; un portillon ouvert n'a plus aucune boîte,
/// ce que l'appelant lit comme « on traverse ») plutôt que celles de la visée. `None` si les octets
/// sont illisibles.
pub(crate) fn world_boxes<'a>(
    registry: &crate::BlockRegistry,
    pos: IVec3,
    data: &[u8],
    block: impl Fn(IVec3) -> BlockId,
    entity: impl Fn(IVec3) -> Option<&'a BlockEntity>,
    collision: bool,
) -> Option<Vec<BlockBox>> {
    let shaped = format::parse(data, false)?;
    let neighbours = shaped.link().map_or(Neighbours::NONE, |link| {
        connected::neighbours_at(registry, link, pos, block, entity)
    });
    Some(if collision {
        shaped.collision_with(&neighbours)
    } else {
        shaped.boxes_with(&neighbours)
    })
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
