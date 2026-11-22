//! Les grilles de blocs : le monde, et les structures mobiles (voir `motion/subgrid.rs`).
//!
//! Dans Create (contraptions) ou Valkyrien Skies (vaisseaux), un groupe de blocs se
//! détache du monde et bouge : il tourne, vole, flotte. Ses machines doivent continuer
//! de fonctionner pendant qu'il bouge. On ne peut donc pas désigner un bloc seulement
//! par ses coordonnées dans le monde : elles changent à chaque image.
//!
//! On désigne un bloc par **une grille et une position dans cette grille** ([`GridPos`]).
//! La grille [`GridId::WORLD`] est le monde ; chaque structure mobile aura la sienne
//! (phase 9d), avec ses propres coordonnées locales qui ne bougent pas, et une
//! transformation (position, rotation) qui la place dans le monde.
//!
//! Le code qui lit et écrit des blocs passe par le trait [`BlockAccess`], qu'implémentent
//! le monde aujourd'hui et les sous-grilles demain : une machine ne sait pas sur laquelle
//! elle se trouve.

use std::cell::RefCell;
use std::sync::Arc;

use glam::IVec3;
use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::registry::BlockRegistry;
use crate::streaming::ChunkStreamer;

/// L'identifiant d'une grille de blocs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct GridId(pub u32);

impl GridId {
    /// Le monde lui-même.
    pub const WORLD: GridId = GridId(0);
}

/// Un bloc : une grille et une position dans cette grille.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GridPos {
    pub grid: GridId,
    pub pos: IVec3,
}

impl GridPos {
    pub const fn new(grid: GridId, pos: IVec3) -> Self {
        Self { grid, pos }
    }

    /// Un bloc du monde.
    pub const fn world(pos: IVec3) -> Self {
        Self::new(GridId::WORLD, pos)
    }

    /// Le bloc voisin dans la même grille.
    pub fn offset(self, d: IVec3) -> Self {
        Self::new(self.grid, self.pos + d)
    }

    /// Une clé triable (pour un ordre déterministe).
    pub fn key(self) -> (u32, [i32; 3]) {
        (self.grid.0, self.pos.to_array())
    }
}

// Un ordre total (grille, puis x, y, z) : nécessaire pour la file triée des ticks.
// `IVec3` n'en a pas (comparer deux vecteurs n'a pas de sens général), on le définit ici.
impl Ord for GridPos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for GridPos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Lire et écrire les blocs d'une grille, quelle qu'elle soit.
pub trait BlockAccess {
    fn registry(&self) -> &Arc<BlockRegistry>;
    fn block(&self, pos: IVec3) -> BlockId;
    /// Change un bloc sans prévenir personne (les machines sont prévenues par l'appelant).
    /// `false` si la position n'existe pas (chunk non chargé, hors de la structure).
    fn set_block(&mut self, pos: IVec3, block: BlockId) -> bool;
    fn block_entity(&self, pos: IVec3) -> Option<&BlockEntity>;
    fn block_entity_mut(&mut self, pos: IVec3) -> Option<&mut BlockEntity>;
    fn set_block_entity(&mut self, pos: IVec3, entity: Option<BlockEntity>) -> bool;
    /// Refaire le maillage après un changement visuel de l'entité, sans remplacer le bloc.
    fn invalidate_visuals(&mut self, pos: IVec3);

    /// L'état actif du bloc, créé vide s'il n'existe pas encore.
    fn ensure_block_entity(&mut self, pos: IVec3) -> Option<&mut BlockEntity> {
        if self.block_entity(pos).is_none()
            && !self.set_block_entity(pos, Some(BlockEntity::default()))
        {
            return None;
        }
        self.block_entity_mut(pos)
    }
}

impl BlockAccess for ChunkStreamer {
    fn invalidate_visuals(&mut self, pos: IVec3) {
        self.invalidate_block_visuals(pos);
    }
    fn registry(&self) -> &Arc<BlockRegistry> {
        self.world().registry()
    }
    fn block(&self, pos: IVec3) -> BlockId {
        self.world().block(pos)
    }
    fn set_block(&mut self, pos: IVec3, block: BlockId) -> bool {
        self.set_block_quiet(pos, block)
    }
    fn block_entity(&self, pos: IVec3) -> Option<&BlockEntity> {
        ChunkStreamer::block_entity(self, pos)
    }
    fn block_entity_mut(&mut self, pos: IVec3) -> Option<&mut BlockEntity> {
        ChunkStreamer::block_entity_mut(self, pos)
    }
    fn set_block_entity(&mut self, pos: IVec3, entity: Option<BlockEntity>) -> bool {
        ChunkStreamer::set_block_entity(self, pos, entity)
    }
}

/// Toutes les grilles, pour retrouver celle d'un [`GridPos`].
///
/// Le monde, plus les structures mobiles (une table `GridId → sous-grille`) : le code des
/// machines et des mods ne change pas.
#[derive(Clone, Copy)]
pub struct Grids<'a> {
    world: &'a RefCell<ChunkStreamer>,
    /// Les structures mobiles (phase 9d), s'il y en a : une table `GridId → sous-grille`.
    subs: Option<&'a crate::motion::subgrid::SubGrids>,
}

impl<'a> Grids<'a> {
    /// Le monde seul.
    pub fn new(world: &'a RefCell<ChunkStreamer>) -> Self {
        Self { world, subs: None }
    }

    /// Le monde et ses structures mobiles : les machines d'un vaisseau tournent comme les autres.
    pub fn with_subgrids(
        world: &'a RefCell<ChunkStreamer>,
        subs: &'a crate::motion::subgrid::SubGrids,
    ) -> Self {
        Self {
            world,
            subs: Some(subs),
        }
    }

    /// Le monde lui-même, pour les comportements natifs qui ont besoin de plus que des
    /// blocs (la lumière, le biome) : les plantes.
    pub fn world(&self) -> &'a RefCell<ChunkStreamer> {
        self.world
    }

    /// La grille demandée, `None` si elle n'existe pas (ou plus : une structure démontée).
    /// `RefCell<dyn BlockAccess>` : le monde et une sous-grille s'utilisent pareil.
    pub fn get(&self, grid: GridId) -> Option<&'a RefCell<dyn BlockAccess>> {
        if grid == GridId::WORLD {
            return Some(self.world as &RefCell<dyn BlockAccess>);
        }
        self.subs?
            .get(grid)
            .map(|cell| cell as &RefCell<dyn BlockAccess>)
    }

    pub fn block(&self, at: GridPos) -> BlockId {
        self.get(at.grid)
            .map_or(BlockId::AIR, |g| g.borrow().block(at.pos))
    }

    /// Lit l'état actif d'un bloc avec `f` (l'emprunt ne dure que le temps de `f`).
    pub fn read<R>(&self, at: GridPos, f: impl FnOnce(&BlockEntity) -> R) -> Option<R> {
        let grid = self.get(at.grid)?.borrow();
        grid.block_entity(at.pos).map(f)
    }

    /// Modifie l'état actif d'un bloc avec `f`, s'il en a un.
    pub fn write<R>(&self, at: GridPos, f: impl FnOnce(&mut BlockEntity) -> R) -> Option<R> {
        let mut grid = self.get(at.grid)?.borrow_mut();
        let entity = grid.block_entity_mut(at.pos)?;
        let before = crate::blocks::connections::ports(entity.caps.as_deref());
        let result = f(entity);
        if before != crate::blocks::connections::ports(entity.caps.as_deref()) {
            grid.invalidate_visuals(at.pos);
        }
        Some(result)
    }

    /// Comme [`Grids::write`], en créant l'état s'il n'existe pas.
    pub fn write_or_create<R>(
        &self,
        at: GridPos,
        f: impl FnOnce(&mut BlockEntity) -> R,
    ) -> Option<R> {
        let mut grid = self.get(at.grid)?.borrow_mut();
        let entity = grid.ensure_block_entity(at.pos)?;
        let before = crate::blocks::connections::ports(entity.caps.as_deref());
        let result = f(entity);
        if before != crate::blocks::connections::ports(entity.caps.as_deref()) {
            grid.invalidate_visuals(at.pos);
        }
        Some(result)
    }
}
