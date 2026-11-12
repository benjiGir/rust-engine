//! Les structures mobiles : un groupe de blocs détaché du monde, qui a sa propre position et
//! sa propre rotation (un vaisseau, un bateau, une contraption).
//!
//! # Le principe
//!
//! 1. **Extraction** ([`extract`]) : les blocs d'une structure (un parcours à partir d'un bloc,
//!    qui s'arrête au terrain) sont **retirés du monde** et copiés, avec leur état actif, dans une
//!    [`SubGrid`] : son propre petit monde, en coordonnées locales qui ne changent jamais.
//! 2. **Transformation** : la sous-grille a une position (celle de son pivot, dans le monde) et
//!    un lacet (une rotation autour de la verticale). Ses machines, ses ticks programmés et ses
//!    réseaux (voir `networks.rs`) ne savent rien de ce mouvement : ils continuent de tourner en
//!    coordonnées locales, désignés par un [`GridPos`].
//! 3. **Réassemblage** ([`land`]) : la structure se repose dans le monde, alignée sur la grille
//!    (rotation arrondie au quart de tour). Il faut de la place ; sinon elle refuse.
//!
//! Une sous-grille contient un vrai [`World`] : le mailleur, la physique du joueur et le lancer de
//! rayon marchent dessus tels quels, en coordonnées locales. C'est ce qui permet au joueur de
//! marcher sur un pont qui bouge : sa physique se fait dans le repère de la structure.
//!
//! La rotation est limitée au lacet (autour de l'axe vertical) : un vaisseau ne se retourne pas.
//! Les angles suivent la même convention que les créatures : `yaw = 0` regarde vers `+x`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use glam::{IVec3, Vec3};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::circuits::FACINGS;
use crate::grid::{BlockAccess, GridId};
use crate::registry::{BlockRegistry, Shape};
use crate::streaming::ChunkStreamer;
use crate::world::World;

/// Une structure ne dépasse pas ce nombre de blocs (au-delà, l'extraction refuse).
pub const MAX_BLOCKS: usize = 4_096;

/// Fait tourner un vecteur de `yaw` radians autour de l'axe vertical (même convention que les
/// créatures et le shader des chunks).
pub fn rotate_y(v: Vec3, yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(v.x * c - v.z * s, v.y, v.x * s + v.z * c)
}

/// Une structure détachée du monde.
#[derive(Debug)]
pub struct SubGrid {
    pub id: GridId,
    world: World,
    /// Toutes les cases non vides, en coordonnées locales.
    cells: HashSet<IVec3>,
    /// Le centre de rotation, en coordonnées locales (continues).
    pub pivot: Vec3,
    /// Où est le pivot dans le monde.
    pub position: Vec3,
    pub yaw: f32,
    pub velocity: Vec3,
    pub yaw_rate: f32,
    /// Augmenté à chaque changement de blocs : le rendu remaille quand il change.
    revision: u64,
}

impl SubGrid {
    pub fn new(id: GridId, registry: Arc<BlockRegistry>) -> Self {
        Self {
            id,
            world: World::with_registry(registry),
            cells: HashSet::default(),
            pivot: Vec3::ZERO,
            position: Vec3::ZERO,
            yaw: 0.0,
            velocity: Vec3::ZERO,
            yaw_rate: 0.0,
            revision: 0,
        }
    }

    /// Le monde local : les blocs en coordonnées locales.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn cells(&self) -> impl Iterator<Item = IVec3> + '_ {
        self.cells.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Un point local (continu) dans le monde.
    pub fn to_world(&self, local: Vec3) -> Vec3 {
        self.position + rotate_y(local - self.pivot, self.yaw)
    }

    /// Un point du monde dans le repère local.
    pub fn to_local(&self, world: Vec3) -> Vec3 {
        rotate_y(world - self.position, -self.yaw) + self.pivot
    }

    /// Une direction locale dans le monde (sans la position).
    pub fn direction_to_world(&self, local: Vec3) -> Vec3 {
        rotate_y(local, self.yaw)
    }

    pub fn direction_to_local(&self, world: Vec3) -> Vec3 {
        rotate_y(world, -self.yaw)
    }

    /// Le centre de la case `cell` dans le monde.
    pub fn cell_center_in_world(&self, cell: IVec3) -> Vec3 {
        self.to_world(cell.as_vec3() + Vec3::splat(0.5))
    }

    /// Les coins (minimum, maximum) des cases occupées.
    pub fn local_bounds(&self) -> Option<(IVec3, IVec3)> {
        let mut it = self.cells.iter().copied();
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), c| (lo.min(c), hi.max(c))))
    }

    /// Recalcule le pivot : le centre de masse (chaque bloc pèse pareil).
    pub fn recenter(&mut self) {
        if self.cells.is_empty() {
            return;
        }
        let sum: Vec3 = self
            .cells
            .iter()
            .map(|c| c.as_vec3() + Vec3::splat(0.5))
            .sum();
        self.pivot = sum / self.cells.len() as f32;
    }

    fn touch(&mut self, pos: IVec3, block: BlockId) {
        if block.is_air() {
            self.cells.remove(&pos);
        } else {
            self.cells.insert(pos);
        }
        self.revision += 1;
    }
}

impl BlockAccess for SubGrid {
    fn invalidate_visuals(&mut self, _pos: IVec3) {
        self.revision += 1;
    }
    fn registry(&self) -> &Arc<BlockRegistry> {
        self.world.registry()
    }

    fn block(&self, pos: IVec3) -> BlockId {
        self.world.block(pos)
    }

    fn set_block(&mut self, pos: IVec3, block: BlockId) -> bool {
        if self.world.block(pos) == block {
            return true;
        }
        self.world.set_block(pos, block);
        // Un bloc remplacé perd son état actif, comme dans le monde.
        let (chunk, [x, y, z]) = World::split(pos);
        if self.world.chunk(chunk).is_some() {
            self.world.chunk_mut(chunk).set_block_entity(x, y, z, None);
        }
        self.touch(pos, block);
        true
    }

    fn block_entity(&self, pos: IVec3) -> Option<&BlockEntity> {
        let (chunk, [x, y, z]) = World::split(pos);
        self.world.chunk(chunk)?.block_entity(x, y, z)
    }

    fn block_entity_mut(&mut self, pos: IVec3) -> Option<&mut BlockEntity> {
        let (chunk, [x, y, z]) = World::split(pos);
        self.world.chunk(chunk)?;
        self.world.chunk_mut(chunk).block_entity_mut(x, y, z)
    }

    fn set_block_entity(&mut self, pos: IVec3, entity: Option<BlockEntity>) -> bool {
        // Seulement sur une case occupée : un état sans bloc n'a pas de sens.
        if self.world.block(pos).is_air() {
            return false;
        }
        if crate::connections::ports(self.block_entity(pos).and_then(|e| e.caps.as_deref()))
            != crate::connections::ports(entity.as_ref().and_then(|e| e.caps.as_deref()))
        {
            self.revision += 1;
        }
        let (chunk, [x, y, z]) = World::split(pos);
        self.world
            .chunk_mut(chunk)
            .set_block_entity(x, y, z, entity);
        true
    }
}

/// Toutes les structures mobiles. Chacune dans sa `RefCell` : on en emprunte une sans bloquer
/// les autres (les machines d'un vaisseau et la physique d'un autre).
#[derive(Debug)]
pub struct SubGrids {
    grids: HashMap<GridId, RefCell<SubGrid>>,
    next: u32,
    registry: Arc<BlockRegistry>,
}

impl SubGrids {
    pub fn new(registry: Arc<BlockRegistry>) -> Self {
        Self {
            grids: HashMap::default(),
            next: 1,
            registry,
        }
    }

    pub fn registry(&self) -> &Arc<BlockRegistry> {
        &self.registry
    }

    /// Une grille neuve (vide), avec un identifiant libre.
    pub fn create(&mut self) -> GridId {
        let id = GridId(self.next);
        self.next += 1;
        self.grids
            .insert(id, RefCell::new(SubGrid::new(id, self.registry.clone())));
        id
    }

    /// Ajoute une grille déjà construite (elle garde son identifiant).
    pub fn insert(&mut self, grid: SubGrid) {
        self.next = self.next.max(grid.id.0 + 1);
        self.grids.insert(grid.id, RefCell::new(grid));
    }

    pub fn remove(&mut self, id: GridId) -> Option<SubGrid> {
        self.grids.remove(&id).map(RefCell::into_inner)
    }

    pub fn get(&self, id: GridId) -> Option<&RefCell<SubGrid>> {
        self.grids.get(&id)
    }

    /// Les identifiants, dans l'ordre.
    pub fn ids(&self) -> Vec<GridId> {
        let mut ids: Vec<GridId> = self.grids.keys().copied().collect();
        ids.sort();
        ids
    }

    pub fn len(&self) -> usize {
        self.grids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grids.is_empty()
    }
}

// ----- Extraction et réassemblage -----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyError {
    NothingHere,
    TooBig,
    Blocked(IVec3),
    NotLoaded,
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingHere => write!(f, "rien à assembler ici"),
            Self::TooBig => write!(f, "la structure dépasse {MAX_BLOCKS} blocs"),
            Self::Blocked(p) => write!(f, "pas de place : {p} est occupé"),
            Self::NotLoaded => write!(f, "une partie du monde n'est pas chargée"),
        }
    }
}

impl std::error::Error for AssemblyError {}

/// Les blocs qui font partie d'une structure : connectés par une face, hors terrain.
fn connected_structure(
    streamer: &ChunkStreamer,
    seed: IVec3,
    is_natural: &dyn Fn(BlockId) -> bool,
) -> Result<Vec<IVec3>, AssemblyError> {
    let world = streamer.world();
    let part = |p: IVec3| {
        let b = world.block(p);
        !b.is_air() && !is_natural(b)
    };
    if !part(seed) {
        return Err(AssemblyError::NothingHere);
    }
    let mut seen: HashSet<IVec3> = HashSet::default();
    seen.insert(seed);
    let mut cells = vec![seed];
    let mut queue = VecDeque::from([seed]);
    while let Some(p) = queue.pop_front() {
        for d in FACINGS {
            let n = p + d;
            if !seen.contains(&n) && part(n) {
                if cells.len() >= MAX_BLOCKS {
                    return Err(AssemblyError::TooBig);
                }
                seen.insert(n);
                cells.push(n);
                queue.push_back(n);
            }
        }
    }
    Ok(cells)
}

/// Détache du monde la structure qui contient `seed`. `is_natural` dit quels blocs sont du
/// terrain (la pierre, la terre, l'eau...) : la structure s'arrête à eux.
///
/// Les blocs sont retirés du monde (en silence : le jeu remaille et prévient les machines),
/// et la grille est créée dans `grids` avec son pivot au centre de masse.
pub fn extract(
    streamer: &mut ChunkStreamer,
    grids: &mut SubGrids,
    seed: IVec3,
    is_natural: &dyn Fn(BlockId) -> bool,
) -> Result<GridId, AssemblyError> {
    let cells = connected_structure(streamer, seed, is_natural)?;
    let min = cells.iter().fold(cells[0], |m, c| m.min(*c));
    let id = grids.create();
    let mut grid = grids.remove(id).expect("grille créée");
    for &world_pos in &cells {
        let local = world_pos - min;
        let block = streamer.world().block(world_pos);
        let entity = streamer.block_entity(world_pos).cloned();
        grid.set_block(local, block);
        if entity.is_some() {
            grid.set_block_entity(local, entity);
        }
    }
    for &world_pos in &cells {
        if !streamer.set_block_quiet(world_pos, BlockId::AIR) {
            return Err(AssemblyError::NotLoaded);
        }
    }
    grid.recenter();
    // Le pivot est au même endroit qu'avant dans le monde : la structure ne saute pas.
    grid.position = grid.pivot + min.as_vec3();
    grid.yaw = 0.0;
    grids.insert(grid);
    Ok(id)
}

/// Le quart de tour le plus proche d'un angle, de 0 à 3.
pub fn quarter_turns(yaw: f32) -> u8 {
    ((yaw / std::f32::consts::FRAC_PI_2).round() as i64).rem_euclid(4) as u8
}

/// La direction `facing` (indice de [`FACINGS`]) après `k` quarts de tour autour de la
/// verticale. Les orientations verticales ne changent pas.
pub fn rotate_facing(facing: u8, k: u8) -> u8 {
    let v = FACINGS[usize::from(facing) % 6];
    if v.y != 0 {
        return facing;
    }
    let rotated = rotate_y(v.as_vec3(), f32::from(k) * std::f32::consts::FRAC_PI_2).round();
    let r = rotated.as_ivec3();
    FACINGS.iter().position(|f| *f == r).unwrap_or(0) as u8
}

/// Les orientations rangées dans l'état (`data[0]`) : les comportements orientés.
fn is_oriented(registry: &BlockRegistry, block: BlockId) -> bool {
    matches!(
        registry.def(block).behavior.as_deref(),
        Some(
            "core:piston"
                | "core:sticky_piston"
                | "core:inverter"
                | "core:repeater"
                | "core:propeller"
        )
    )
}

/// Une case où l'on peut poser un bloc de la structure : de l'air, un liquide, une plante.
fn free_for_landing(registry: &BlockRegistry, block: BlockId) -> bool {
    let def = registry.def(block);
    block.is_air() || def.liquid || (!def.solid && def.shape != Shape::Cube)
}

/// Où chaque case de la grille irait si on la reposait maintenant : la rotation arrondie au
/// quart de tour, le pivot arrondi pour tomber pile sur la grille du monde. Les cases sont
/// renvoyées avec leur position d'arrivée.
pub fn landing_cells(grid: &SubGrid) -> Vec<(IVec3, IVec3)> {
    let k = quarter_turns(grid.yaw);
    let yaw = f32::from(k) * std::f32::consts::FRAC_PI_2;
    let offset = |c: IVec3| rotate_y(c.as_vec3() + Vec3::splat(0.5) - grid.pivot, yaw);
    let Some(first) = grid.cells.iter().next().copied() else {
        return Vec::new();
    };
    // Le décalage qui aligne la case de référence sur la grille du monde.
    let reference = grid.position + offset(first) - Vec3::splat(0.5);
    let shift = reference.round() - reference;
    grid.cells
        .iter()
        .map(|&c| {
            let cell = (grid.position + shift + offset(c) - Vec3::splat(0.5)).round();
            (c, cell.as_ivec3())
        })
        .collect()
}

/// Repose la structure dans le monde. Renvoie les positions des blocs posés, ou
/// [`AssemblyError::Blocked`] si le terrain gêne (rien n'est alors modifié).
pub fn land(streamer: &mut ChunkStreamer, grid: &SubGrid) -> Result<Vec<IVec3>, AssemblyError> {
    let registry = streamer.world().registry().clone();
    let cells = landing_cells(grid);
    for &(_, target) in &cells {
        let block = streamer.world().block(target);
        if !free_for_landing(&registry, block) {
            return Err(AssemblyError::Blocked(target));
        }
    }
    let k = quarter_turns(grid.yaw);
    let mut placed = Vec::with_capacity(cells.len());
    for (local, target) in cells {
        let block = grid.block(local);
        if !streamer.set_block_quiet(target, block) {
            return Err(AssemblyError::NotLoaded);
        }
        if let Some(mut entity) = grid.block_entity(local).cloned() {
            if is_oriented(&registry, block)
                && let Some(f) = entity.data.first_mut()
            {
                *f = rotate_facing(*f, k);
            }
            streamer.set_block_entity(target, Some(entity));
        }
        placed.push(target);
    }
    Ok(placed)
}

// ----- Sauvegarde -----

#[derive(Debug, Serialize, Deserialize)]
struct SavedGrid {
    id: u32,
    pivot: [f32; 3],
    position: [f32; 3],
    yaw: f32,
    velocity: [f32; 3],
    yaw_rate: f32,
    palette: Vec<String>,
    blocks: Vec<([i32; 3], u16)>,
    entities: Vec<([i32; 3], BlockEntity)>,
}

const GRIDS_FILE: &str = "grids.bin";

/// Écrit les structures mobiles dans `dir/grids.bin` (le fichier disparaît s'il n'y en a plus).
/// Les blocs sont rangés par nom : la sauvegarde reste valable quand la liste des mods change.
pub fn save_grids(dir: &Path, grids: &SubGrids) -> Result<()> {
    let path = dir.join(GRIDS_FILE);
    if grids.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let mut saved = Vec::new();
    for id in grids.ids() {
        let grid = grids.get(id).expect("identifiant connu").borrow();
        let mut palette: Vec<String> = Vec::new();
        let mut blocks = Vec::new();
        let mut entities = Vec::new();
        let mut cells: Vec<IVec3> = grid.cells().collect();
        cells.sort_by_key(|c| c.to_array());
        for c in cells {
            let key = grid.registry().def(grid.block(c)).key.clone();
            let index = palette.iter().position(|k| *k == key).unwrap_or_else(|| {
                palette.push(key);
                palette.len() - 1
            });
            blocks.push((c.to_array(), index as u16));
            if let Some(e) = grid.block_entity(c) {
                entities.push((c.to_array(), e.clone()));
            }
        }
        saved.push(SavedGrid {
            id: grid.id.0,
            pivot: grid.pivot.to_array(),
            position: grid.position.to_array(),
            yaw: grid.yaw,
            velocity: grid.velocity.to_array(),
            yaw_rate: grid.yaw_rate,
            palette,
            blocks,
            entities,
        });
    }
    let bytes = postcard::to_allocvec(&saved).context("encodage des structures")?;
    // Écrire à côté puis renommer : un arrêt en plein milieu ne laisse pas un fichier tronqué.
    let temp = dir.join("grids.bin.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

/// Relit les structures mobiles d'un monde (aucune si le fichier n'existe pas). Un bloc dont le
/// mod a disparu devient le bloc inconnu.
pub fn load_grids(dir: &Path, registry: &Arc<BlockRegistry>) -> Result<SubGrids> {
    let mut grids = SubGrids::new(registry.clone());
    let path = dir.join(GRIDS_FILE);
    if !path.exists() {
        return Ok(grids);
    }
    let bytes = std::fs::read(&path).with_context(|| format!("lecture de {}", path.display()))?;
    let saved: Vec<SavedGrid> = postcard::from_bytes(&bytes).context("décodage des structures")?;
    let unknown = registry.id("core:unknown").unwrap_or_default();
    for s in saved {
        let mut grid = SubGrid::new(GridId(s.id), registry.clone());
        let ids: Vec<BlockId> = s
            .palette
            .iter()
            .map(|k| registry.id(k).unwrap_or(unknown))
            .collect();
        for (pos, index) in s.blocks {
            let block = ids.get(usize::from(index)).copied().unwrap_or(unknown);
            grid.set_block(IVec3::from_array(pos), block);
        }
        for (pos, entity) in s.entities {
            grid.set_block_entity(IVec3::from_array(pos), Some(entity));
        }
        grid.pivot = Vec3::from_array(s.pivot);
        grid.position = Vec3::from_array(s.position);
        grid.yaw = s.yaw;
        grid.velocity = Vec3::from_array(s.velocity);
        grid.yaw_rate = s.yaw_rate;
        grids.insert(grid);
    }
    Ok(grids)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::machines::tests::loaded_streamer;

    fn id(s: &RefCell<ChunkStreamer>, key: &str) -> BlockId {
        s.borrow().world().registry().id(key).unwrap()
    }

    /// Les blocs de terrain pour les tests : pierre et terre.
    fn natural(s: &RefCell<ChunkStreamer>) -> impl Fn(BlockId) -> bool {
        let (stone, dirt) = (id(s, "core:stone"), id(s, "core:dirt"));
        move |b| b == stone || b == dirt
    }

    /// Un sol de pierre et un petit radeau en planches posé dessus.
    fn raft(s: &RefCell<ChunkStreamer>) -> IVec3 {
        let (stone, planks, chest) = (
            id(s, "core:stone"),
            id(s, "core:oak_planks"),
            id(s, "core:chest"),
        );
        for x in -6..16 {
            for z in -6..16 {
                for y in 0..8 {
                    let block = if y == 0 { stone } else { BlockId::AIR };
                    s.borrow_mut()
                        .set_block(IVec3::new(4 + x, 110 + y - 1, 4 + z), block);
                }
            }
        }
        let origin = IVec3::new(4, 110, 4);
        for x in 0..4 {
            for z in 0..3 {
                s.borrow_mut()
                    .set_block(origin + IVec3::new(x, 0, z), planks);
            }
        }
        s.borrow_mut()
            .set_block(origin + IVec3::new(1, 1, 1), chest);
        let entity = BlockEntity {
            data: vec![42],
            ..BlockEntity::default()
        };
        s.borrow_mut()
            .set_block_entity(origin + IVec3::new(1, 1, 1), Some(entity));
        origin
    }

    #[test]
    fn extraction_removes_the_structure_but_not_the_ground() {
        let s = loaded_streamer();
        let origin = raft(&s);
        let nat = natural(&s);
        let mut grids = SubGrids::new(s.borrow().world().registry().clone());
        let grid_id = extract(&mut s.borrow_mut(), &mut grids, origin, &nat).unwrap();
        // Le radeau a disparu du monde ; le sol reste.
        assert!(s.borrow().world().block(origin).is_air());
        assert!(!s.borrow().world().block(origin - IVec3::Y).is_air());
        let grid = grids.get(grid_id).unwrap().borrow();
        assert_eq!(grid.len(), 13, "12 planches et un coffre");
        // Les coordonnées locales commencent à zéro, l'état du coffre a suivi.
        let (lo, hi) = grid.local_bounds().unwrap();
        assert_eq!((lo, hi), (IVec3::ZERO, IVec3::new(3, 1, 2)));
        assert_eq!(
            grid.block_entity(IVec3::new(1, 1, 1)).unwrap().data,
            vec![42]
        );
        // Le pivot est au centre, et la structure n'a pas bougé dans le monde.
        assert_eq!(grid.to_world(grid.pivot), grid.position);
        let first = grid.cell_center_in_world(IVec3::ZERO);
        assert!((first - (origin.as_vec3() + Vec3::splat(0.5))).length() < 1e-4);
    }

    #[test]
    fn the_transform_maps_points_both_ways_and_rotates_around_the_pivot() {
        let mut grid = SubGrid::new(GridId(1), crate::BlockRegistry::builtin());
        grid.pivot = Vec3::new(2.0, 1.0, 1.5);
        grid.position = Vec3::new(100.0, 70.0, 50.0);
        grid.yaw = 0.7;
        for p in [
            Vec3::ZERO,
            Vec3::new(3.2, 0.5, 2.9),
            Vec3::new(-4.0, 9.0, 1.0),
        ] {
            let back = grid.to_local(grid.to_world(p));
            assert!((back - p).length() < 1e-4, "{p} → {back}");
        }
        // Le pivot ne bouge pas quand le lacet change.
        let before = grid.to_world(grid.pivot);
        grid.yaw = 2.0;
        assert!((grid.to_world(grid.pivot) - before).length() < 1e-5);
        // Un quart de tour : +x local devient +z dans le monde.
        grid.yaw = std::f32::consts::FRAC_PI_2;
        let d = grid.direction_to_world(Vec3::X);
        assert!((d - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn landing_puts_it_back_aligned_and_refuses_when_something_is_in_the_way() {
        let s = loaded_streamer();
        let origin = raft(&s);
        let nat = natural(&s);
        let mut grids = SubGrids::new(s.borrow().world().registry().clone());
        let grid_id = extract(&mut s.borrow_mut(), &mut grids, origin, &nat).unwrap();
        // Déplacée de 5 blocs et tournée d'un quart de tour (un peu de bruit : on arrondit).
        {
            let mut grid = grids.get(grid_id).unwrap().borrow_mut();
            grid.position += Vec3::new(5.2, 0.0, 0.1);
            grid.yaw = std::f32::consts::FRAC_PI_2 + 0.05;
        }
        let grid = grids.remove(grid_id).unwrap();
        // Un obstacle de pierre sur le chemin : refus, et rien n'a changé.
        let cells = landing_cells(&grid);
        assert_eq!(cells.len(), 13);
        let target = cells[0].1;
        let stone = id(&s, "core:stone");
        let before = s.borrow().world().block(target);
        s.borrow_mut().set_block(target, stone);
        assert_eq!(
            land(&mut s.borrow_mut(), &grid),
            Err(AssemblyError::Blocked(target))
        );
        s.borrow_mut().set_block(target, before);
        // Dégagé : la structure se repose, 12 planches et le coffre avec son état.
        let placed = land(&mut s.borrow_mut(), &grid).unwrap();
        assert_eq!(placed.len(), 13);
        let planks = id(&s, "core:oak_planks");
        let count = placed
            .iter()
            .filter(|p| s.borrow().world().block(**p) == planks)
            .count();
        assert_eq!(count, 12);
        let chest = id(&s, "core:chest");
        let chest_at = placed
            .iter()
            .copied()
            .find(|p| s.borrow().world().block(*p) == chest)
            .unwrap();
        assert_eq!(s.borrow().block_entity(chest_at).unwrap().data, vec![42]);
        // Tournée d'un quart de tour : 4 de long en x devient 4 de long en z.
        let (lo, hi) = placed
            .iter()
            .fold((placed[0], placed[0]), |(l, h), p| (l.min(*p), h.max(*p)));
        let size = hi - lo + IVec3::ONE;
        assert_eq!((size.x, size.z), (3, 4), "tournée d'un quart de tour");
    }

    #[test]
    fn facings_turn_with_the_structure() {
        // +x (0) après un quart de tour devient +z (4), puis -x (1), puis -z (5).
        assert_eq!(rotate_facing(0, 1), 4);
        assert_eq!(rotate_facing(0, 2), 1);
        assert_eq!(rotate_facing(0, 3), 5);
        assert_eq!(rotate_facing(0, 0), 0);
        assert_eq!(rotate_facing(2, 3), 2, "le haut reste le haut");
        assert_eq!(quarter_turns(0.1), 0);
        assert_eq!(quarter_turns(1.6), 1);
        assert_eq!(quarter_turns(-1.6), 3);
        assert_eq!(quarter_turns(std::f32::consts::PI), 2);
    }

    #[test]
    fn grids_save_and_load_with_their_state_and_transform() {
        let s = loaded_streamer();
        let origin = raft(&s);
        let nat = natural(&s);
        let registry = s.borrow().world().registry().clone();
        let mut grids = SubGrids::new(registry.clone());
        let grid_id = extract(&mut s.borrow_mut(), &mut grids, origin, &nat).unwrap();
        {
            let mut grid = grids.get(grid_id).unwrap().borrow_mut();
            grid.position += Vec3::new(3.0, 2.0, 1.0);
            grid.yaw = 0.8;
            grid.velocity = Vec3::new(0.5, 0.0, 0.25);
        }
        let dir = std::env::temp_dir().join(format!("voxel-grids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save_grids(&dir, &grids).unwrap();
        let loaded = load_grids(&dir, &registry).unwrap();
        assert_eq!(loaded.ids(), vec![grid_id]);
        let a = grids.get(grid_id).unwrap().borrow();
        let b = loaded.get(grid_id).unwrap().borrow();
        assert_eq!(
            (a.len(), a.pivot, a.position, a.yaw),
            (b.len(), b.pivot, b.position, b.yaw)
        );
        assert_eq!(a.velocity, b.velocity);
        for c in a.cells() {
            assert_eq!(a.block(c), b.block(c));
        }
        assert_eq!(b.block_entity(IVec3::new(1, 1, 1)).unwrap().data, vec![42]);
        // Sans structure : le fichier disparaît.
        drop((a, b));
        save_grids(&dir, &SubGrids::new(registry.clone())).unwrap();
        assert!(!dir.join("grids.bin").exists());
        assert!(load_grids(&dir, &registry).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn too_big_or_empty_extractions_are_refused() {
        let s = loaded_streamer();
        let origin = raft(&s);
        let nat = natural(&s);
        let mut grids = SubGrids::new(s.borrow().world().registry().clone());
        // Une case de terrain n'est pas une structure.
        assert_eq!(
            extract(&mut s.borrow_mut(), &mut grids, origin - IVec3::Y, &nat),
            Err(AssemblyError::NothingHere)
        );
        // De l'air non plus.
        assert_eq!(
            extract(&mut s.borrow_mut(), &mut grids, origin + IVec3::Y * 5, &nat),
            Err(AssemblyError::NothingHere)
        );
        assert!(grids.is_empty());
    }

    /// Les machines d'une structure tournent comme celles du monde, avec leurs coordonnées
    /// locales, et ne se soucient pas de son mouvement.
    #[test]
    fn machines_and_networks_keep_working_inside_a_moving_structure() {
        use crate::grid::{GridPos, Grids};
        use crate::machines::Machines;
        let s = loaded_streamer();
        let registry = s.borrow().world().registry().clone();
        let mut grids = SubGrids::new(registry.clone());
        let id = grids.create();
        let key = |k: &str| registry.id(k).unwrap();
        {
            let mut grid = grids.get(id).unwrap().borrow_mut();
            grid.set_block(IVec3::ZERO, key("core:battery"));
            for x in 1..=5 {
                grid.set_block(IVec3::new(x, 0, 0), key("core:cable"));
            }
            grid.set_block(IVec3::new(6, 0, 0), key("core:power_lamp"));
            grid.recenter();
            grid.position = Vec3::new(40.0, 120.0, 40.0);
        }
        let mut machines = Machines::with_natives(&registry);
        let cells: Vec<GridPos> = grids
            .get(id)
            .unwrap()
            .borrow()
            .cells()
            .map(|c| GridPos::new(id, c))
            .collect();
        let mut actions = Vec::new();
        machines.grid_blocks_loaded(Grids::with_subgrids(&s, &grids), &cells, &mut actions);
        // Une batterie chargée.
        grids
            .get(id)
            .unwrap()
            .borrow_mut()
            .block_entity_mut(IVec3::ZERO)
            .unwrap()
            .caps_mut()
            .energy
            .as_mut()
            .unwrap()
            .amount = 50_000;
        let lit = |grids: &SubGrids| {
            grids.get(id).unwrap().borrow().block(IVec3::new(6, 0, 0)) == key("core:power_lamp_on")
        };
        for _ in 0..10 {
            machines.run_grids_tick(Grids::with_subgrids(&s, &grids), &mut actions);
        }
        assert!(lit(&grids), "la lampe de la structure est allumée");
        assert_eq!(machines.networks().stats().networks, 1);
        // La structure se déplace et tourne : rien ne change pour ses machines.
        {
            let mut grid = grids.get(id).unwrap().borrow_mut();
            grid.position += Vec3::new(10.0, 3.0, -4.0);
            grid.yaw = 1.2;
        }
        for _ in 0..10 {
            machines.run_grids_tick(Grids::with_subgrids(&s, &grids), &mut actions);
        }
        assert!(lit(&grids));
        // Et le monde n'a pas été touché.
        assert!(s.borrow().world().block(IVec3::new(40, 120, 40)).is_air());
    }
}
