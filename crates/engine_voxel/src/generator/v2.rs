//! Le générateur de blocs v2 (lot 4 du plan) : le relief macro (géographie du lot 2, drainage du
//! lot 3) matérialisé en blocs, avec une densité 3D, des surfaces selon la pente, des strates
//! régionales, des cavernes et des nappes liées au drainage.
//!
//! `GenerationVersion::V2` n'est **pas** activée pour les nouveaux mondes (lot 9) : on la choisit
//! pour un monde de test avec `VOXEL_GENERATION=2` à la création (voir `generation.rs`), et un
//! monde enregistré en v2 se rouvre en v2 (`WorldMeta::generation`).
//!
//! # Le contexte de colonne
//!
//! Tout part d'un [`ColumnContext`], calculé **une fois** par colonne de chunks (32 × 32 colonnes
//! de blocs, plus une marge de [`MARGIN`] blocs) et partagé par les six chunks de la verticale, le
//! placement des arbres et des structures, et les outils. Il tient dans un cache borné à clés
//! explicites (graine, dimension, version, cx, cz : voir `column_cache.rs`), calculé hors verrou,
//! évincé du plus ancien : un cache accélère une valeur, il ne la décide jamais.
//!
//! Le contexte contient, par colonne : la hauteur du relief (valléé et lits creusés, remblai des
//! berges), le niveau de l'eau d'un cours ou d'un lac, la **réservation** (un poids de 0 à 1 qui
//! interdit tout détail), la pente, le biome, les règles de surface, les escarpements et leurs entailles, la
//! nappe et la profondeur permise aux cavernes (voir `v2_context.rs`). Chaque valeur est une
//! fonction pure de la position : un chunk voisin ou un cache vide donnent les mêmes blocs.
//!
//! # Le déroulement d'un chunk
//!
//! 1. solide ou vide d'après la densité (`density.rs`) : le relief, plus l'entaille du pied des
//!    parois d'escarpement (des surplombs) ; les colonnes réservées (lits, berges, lacs) n'ont que le
//!    relief ;
//! 2. matériaux de la roche, du sol et de la surface (`surface.rs`), eau des mers, cours et lacs ;
//! 3. cavernes, nappes et lave (`underground.rs`) ;
//! 4. minerais (la fonction de la v1, `ores.ron`), plantes, strates régionales, arbres, structures.
//!
//! Aucune de ces passes n'écrit dans une colonne réservée un bloc qui ferme un lit : les tests de
//! `tests/generation_v2.rs` le vérifient sur les blocs finals.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use glam::IVec3;

use super::column_cache::{ColumnCache, ColumnKey};
use super::density;
use super::surface::SurfaceRule;
use super::{
    Column, DEEPSTONE_LEVEL, FREEZING, LAVA_LEVEL, MacroWorld, SEA_LEVEL, TerrainGenerator, hash3,
};
use crate::biome::{BiomeId, BiomeKind};
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::generation::GenerationVersion;
use crate::registry::BlockRegistry;
use crate::world::World;

/// Marge (blocs) autour de la colonne de chunks : la dilatation des zones réservées (arbres de
/// 8 blocs de rayon), la pente (±2) et les remblais (±1) y lisent leurs voisines.
pub(super) const MARGIN: i32 = 8;
/// Côté de la zone calculée, marge comprise.
pub(super) const WIDE: usize = CHUNK_SIZE + 2 * MARGIN as usize;
/// Pas d'eau : la colonne n'a pas de cours ni de lac.
pub(super) const NO_WATER: i16 = i16::MIN;
/// Nombre maximal de contextes gardés (environ 90 Kio chacun : une quarantaine de Mio).
pub(super) const CONTEXT_CACHE_LIMIT: usize = 512;
/// Relèvement maximal (blocs) du terrain au bord d'un cours ou d'un lac dont l'eau le dépasse. Le
/// prototype macro le plafonnait à 2 pour ses cartes ; les blocs ont besoin d'un vrai remblai
/// (une berge plus basse que l'eau la laisserait déborder), les berges encore trop basses sont
/// fermées colonne par colonne (voir `v2_context.rs`).
pub(super) const FILL: f32 = 10.0;

/// Drapeaux d'une colonne de la zone large.
pub(super) mod flag {
    /// Dans le lit d'un cours (distance à l'axe inférieure à la demi-largeur).
    pub const BED_RIVER: u8 = 1;
    /// Dans la cuvette d'un lac.
    pub const BED_LAKE: u8 = 2;
    /// Eau de cours au-dessus du lit.
    pub const WATER_RIVER: u8 = 4;
    /// Eau de lac au-dessus du fond.
    pub const WATER_LAKE: u8 = 8;
}

/// Une colonne de la zone large (marge comprise) : le relief et l'eau, sans rien de lié au climat.
#[derive(Debug, Clone, Copy)]
pub(super) struct WideCell {
    /// y du bloc le plus haut du relief (une colonne réservée n'a pas d'autre solide).
    pub top: i16,
    /// y du dernier bloc d'eau d'un cours ou d'un lac, ou [`NO_WATER`].
    pub water: i16,
    pub flags: u8,
    /// Réservation, de 0 à 255 : 255 dans le lit et sur les berges, décroissante au-delà.
    pub protect: u8,
    /// Poids de la vallée modelée, de 0 à 255.
    pub valley: u8,
    /// Le pied de la paroi d'un escarpement (relief sans son ressaut) ; égal à `top` hors escarpement.
    pub base: i16,
    /// La hauteur (blocs) du ressaut d'escarpement de la colonne, 0 hors des massifs.
    pub jump: u8,
    /// Largeur du cours le plus proche (blocs, au plus 255), 0 sans cours.
    pub river_width: u8,
    /// Niveau de la nappe avant les bornes de surface (blocs, flottant).
    pub table: f32,
}

/// Une colonne du cœur (la colonne de chunks elle-même).
#[derive(Debug, Clone, Copy)]
pub(super) struct CoreCell {
    /// y du bloc solide le plus haut, entailles de paroi comprises.
    pub top: i16,
    /// Pente (gradient du relief, blocs par bloc).
    pub slope: f32,
    /// Le pied de la paroi (relief sans son ressaut) : l'entaille ne descend jamais dessous.
    pub base: i16,
    /// La hauteur (blocs) du ressaut d'escarpement ; 0 hors de la zone des escarpements.
    pub jump: f32,
    /// Distance (blocs) à la ligne de rupture de l'escarpement, positive du côté haut.
    pub dist: f32,
    /// Profondeur maximale (blocs) de l'entaille du pied de la paroi ; 0 : pas de surplomb possible.
    pub reach: f32,
    /// Forme (exposant du profil) et hauteur relative (fraction de la paroi) de l'entaille.
    pub shape: f32,
    pub lid: f32,
    pub biome: BiomeId,
    pub rule: SurfaceRule,
    /// Niveau de la nappe souterraine, ou [`NO_WATER`].
    pub table: i16,
    /// Altitude la plus haute des cavernes de la colonne.
    pub cave_limit: i16,
    /// Distance (Chebyshev, jusqu'à 9) à la zone réservée ou à l'eau la plus proche : les arbres et
    /// les structures s'en tiennent à distance.
    pub clear: u8,
    /// L'humidité locale, de -1 à 1 : le climat, l'eau proche et les zones basses humides.
    pub humidity: f32,
    /// La température du climat (altitude comprise), de -1 à 1.
    pub climate: f32,
    /// Le coefficient de végétation, de 0 à `MAX_CANOPY` : voir `canopy_factor`.
    pub canopy: f32,
}

/// Tout ce qu'une colonne de chunks sait d'elle-même (voir la documentation du module).
#[derive(Debug)]
pub(super) struct ColumnContext {
    /// `(lz + MARGIN) × WIDE + lx + MARGIN`.
    pub wide: Vec<WideCell>,
    /// `lx + 32 lz`.
    pub core: Vec<CoreCell>,
    /// Les colonnes au format de la v1, `[x][z]` : la hauteur est celle du solide le plus haut.
    pub columns: Box<[[Column; CHUNK_SIZE]; CHUNK_SIZE]>,
    /// Les biomes, `x + 32 z`.
    pub biomes: Vec<u8>,
    /// Le plus haut bloc non vide (eau comprise) de la colonne de chunks.
    pub max_y: i32,
    /// L'altitude la plus haute où une caverne peut être creusée dans la colonne de chunks.
    pub cave_ceiling: i32,
}

impl ColumnContext {
    #[inline]
    pub fn wide_index(lx: i32, lz: i32) -> usize {
        ((lz + MARGIN) as usize) * WIDE + (lx + MARGIN) as usize
    }

    #[inline]
    pub fn wide_at(&self, lx: i32, lz: i32) -> &WideCell {
        &self.wide[Self::wide_index(lx, lz)]
    }

    #[inline]
    pub fn core_at(&self, lx: usize, lz: usize) -> &CoreCell {
        &self.core[lx + lz * CHUNK_SIZE]
    }

    /// Le niveau du dernier bloc d'eau de la colonne : un cours ou un lac, sinon la mer au-dessus
    /// d'un relief immergé, sinon [`NO_WATER`].
    #[inline]
    pub fn water_top(&self, lx: usize, lz: usize) -> i32 {
        let cell = self.wide_at(lx as i32, lz as i32);
        if cell.water != NO_WATER {
            cell.water as i32
        } else if (cell.top as i32) < SEA_LEVEL {
            SEA_LEVEL
        } else {
            NO_WATER as i32
        }
    }
}

/// Un instantané de ce que le générateur v2 sait d'une colonne de blocs, pour les outils et les
/// tests (voir [`TerrainGenerator::surface_info`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceInfo {
    /// y du bloc solide le plus haut.
    pub top: i32,
    /// y du dernier bloc d'eau (cours, lac ou mer), `None` sur la terre.
    pub water: Option<i32>,
    /// Dans le lit d'un cours.
    pub river_bed: bool,
    /// Dans la cuvette d'un lac.
    pub lake_bed: bool,
    /// La zone est réservée (lit, berges, rives d'un lac) : aucun détail n'y est permis.
    pub reserved: bool,
    pub biome: BiomeId,
    pub slope: f32,
    /// Profondeur maximale (blocs) de l'entaille du pied d'une paroi : 0 sans surplomb possible.
    pub overhang: f32,
    /// La hauteur (blocs) du ressaut d'escarpement de la colonne : 0 hors de leur zone.
    pub escarpment: f32,
    /// Distance (blocs) à la ligne de rupture de l'escarpement, positive du côté haut.
    pub wall_distance: f32,
    /// Le relief continu (le dernier bloc plein sans les entailles) : `top` s'en écarte sous un surplomb.
    pub relief_top: i32,
    /// L'humidité locale, de -1 à 1.
    pub humidity: f32,
    /// La température du climat (altitude comprise), de -1 à 1.
    pub climate: f32,
    /// Le coefficient de végétation : 0 (aucun arbre) à `MAX_CANOPY`.
    pub canopy: f32,
}

/// Les blocs que la v2 emploie en plus de ceux de la v1.
#[derive(Debug, Clone, Copy)]
pub(super) struct V2Blocks {
    pub gravel: BlockId,
    pub clay: BlockId,
    pub sandstone: BlockId,
    pub volcanic_rock: BlockId,
    pub coarse_dirt: BlockId,
}

impl V2Blocks {
    fn resolve(registry: &BlockRegistry) -> Result<Self> {
        let id = |key: &str| {
            registry
                .id(key)
                .with_context(|| format!("la génération v2 a besoin du bloc « {key} »"))
        };
        Ok(Self {
            gravel: id("core:gravel")?,
            clay: id("core:clay")?,
            sandstone: id("core:sandstone")?,
            volcanic_rock: id("core:volcanic_rock")?,
            coarse_dirt: id("core:coarse_dirt")?,
        })
    }
}

/// L'état de la génération v2 d'un générateur : le contexte macro de la graine (ses caches),
/// les bruits de densité, le cache des contextes de colonne.
pub(super) struct V2State {
    pub world: MacroWorld,
    /// Mesures et outils : ni cavernes, ni arbres, ni structures (voir [`TerrainGenerator::relief_only`]).
    pub relief_only: bool,
    pub contexts: ColumnCache<ColumnContext>,
    pub blocks: V2Blocks,
}

impl std::fmt::Debug for V2State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2State")
            .field("seed", &self.world.seed())
            .field("contexts", &self.contexts)
            .finish_non_exhaustive()
    }
}

impl V2State {
    fn new(seed: u32, registry: &BlockRegistry) -> Result<Self> {
        Ok(Self {
            world: MacroWorld::new(seed),
            relief_only: false,
            contexts: ColumnCache::with_limit(CONTEXT_CACHE_LIMIT),
            blocks: V2Blocks::resolve(registry)?,
        })
    }
}

/// La version la plus haute des bruits de grotte de la v1, pour les chunks du ciel.
const SKY_MARGIN: i32 = 1;

impl TerrainGenerator {
    /// Ce générateur, réglé sur une version de génération. `V1` ne change rien ; `V2` branche le
    /// relief macro et la densité 3D (voir la documentation de `generator/v2.rs`). Les clés des
    /// caches contiennent la version : deux versions ne se voient jamais leurs valeurs.
    pub fn with_version(mut self, version: GenerationVersion) -> Result<Self> {
        self.version = version;
        self.v2 = match version {
            GenerationVersion::V1 => None,
            GenerationVersion::V2 => Some(self.with_version_state()?),
        };
        // Les arbres et les colonnes dépendent de la version : on repart de caches neufs.
        self.tree_cache = Arc::default();
        self.site_cache = Arc::default();
        self.column_cache = Arc::default();
        Ok(self)
    }

    /// Ce générateur v2 réduit au **relief** : ni cavernes, ni arbres, ni structures. Pour les mesures
    /// des parois et des surplombs (les cavernes et les ouvrages n'y comptent pas) et les cartes de
    /// relief ; sans effet en v1. Les caches repartent à neuf.
    pub fn relief_only(mut self) -> Self {
        if self.v2.is_some() {
            let mut state = V2State::new(self.seed, &self.registry).expect("v2 déjà initialisée");
            state.relief_only = true;
            self.v2 = Some(Arc::new(state));
            self.tree_cache = Arc::default();
            self.site_cache = Arc::default();
            self.column_cache = Arc::default();
        }
        self
    }

    /// Un état de génération v2 neuf pour ce générateur (graine, blocs), caches vides.
    pub(super) fn with_version_state(&self) -> Result<Arc<V2State>> {
        Ok(Arc::new(V2State::new(self.seed, &self.registry)?))
    }

    /// Comme [`Self::with_dimension`], avec une version de génération (celle de `world.ron`).
    pub fn with_dimension_versioned(
        seed: u32,
        registry: Arc<BlockRegistry>,
        mods: &[(String, std::path::PathBuf)],
        dimension: &crate::dimension::DimensionDef,
        version: GenerationVersion,
    ) -> Result<Self> {
        Self::with_dimension(seed, registry, mods, dimension)?.with_version(version)
    }

    /// Le contexte de la colonne de chunks (cx, cz), depuis le cache si possible. Panique sans
    /// génération v2 : l'appelant a vérifié `self.v2`.
    pub(super) fn v2_context(&self, cx: i32, cz: i32) -> Arc<ColumnContext> {
        let v2 = self.v2.as_ref().expect("génération v2 active");
        let key = ColumnKey {
            seed: self.seed,
            dimension: self.dimension_tag,
            version: self.version,
            cx,
            cz,
        };
        v2.contexts
            .get_or_compute(key, || self.build_context(v2, cx, cz))
    }

    /// La colonne (x, z) de la génération v2 : la hauteur est celle du bloc solide le plus haut.
    pub(super) fn column_v2(&self, x: i32, z: i32) -> Column {
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        ctx.columns[x.rem_euclid(size) as usize][z.rem_euclid(size) as usize]
    }

    /// Ce que la génération v2 sait de la colonne (x, z) ; `None` hors de la génération v2.
    pub fn surface_info(&self, x: i32, z: i32) -> Option<SurfaceInfo> {
        self.v2.as_ref()?;
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        let (lx, lz) = (x.rem_euclid(size), z.rem_euclid(size));
        let (core, wide) = (ctx.core_at(lx as usize, lz as usize), ctx.wide_at(lx, lz));
        let water = ctx.water_top(lx as usize, lz as usize);
        Some(SurfaceInfo {
            top: core.top as i32,
            water: (water != NO_WATER as i32).then_some(water),
            river_bed: wide.flags & flag::BED_RIVER != 0,
            lake_bed: wide.flags & flag::BED_LAKE != 0,
            reserved: wide.protect > 0 || wide.water != NO_WATER,
            biome: core.biome,
            slope: core.slope,
            overhang: core.reach,
            escarpment: core.jump,
            wall_distance: core.dist,
            relief_top: wide.top as i32,
            humidity: core.humidity,
            climate: core.climate,
            canopy: core.canopy,
        })
    }

    /// Le nombre maximal de contextes de colonne gardés par la génération v2.
    pub const CONTEXT_CACHE_LIMIT: usize = CONTEXT_CACHE_LIMIT;

    /// Le nombre de contextes de colonne gardés (0 hors génération v2 ; pour les mesures et les tests).
    pub fn cached_contexts(&self) -> usize {
        self.v2.as_ref().map_or(0, |v2| v2.contexts.len())
    }

    /// Le contexte macro de la génération v2 (pour les outils et les tests).
    pub fn macro_world(&self) -> Option<&MacroWorld> {
        self.v2.as_ref().map(|v2| &v2.world)
    }

    /// Distance (Chebyshev, de 0 à 9) de la colonne (x, z) à la zone réservée la plus proche (lit,
    /// berges, lacs) : les arbres et les structures s'en tiennent éloignés. 9 sans génération v2.
    pub(super) fn v2_clearance(&self, x: i32, z: i32) -> u8 {
        if self.v2.is_none() {
            return 9;
        }
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        ctx.core_at(x.rem_euclid(size) as usize, z.rem_euclid(size) as usize)
            .clear
    }

    /// Le coefficient de végétation de la colonne (x, z) : 1 sans génération v2 (rien ne change).
    pub(super) fn v2_canopy(&self, x: i32, z: i32) -> f32 {
        if self.v2.is_none() {
            return 1.0;
        }
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        ctx.core_at(x.rem_euclid(size) as usize, z.rem_euclid(size) as usize)
            .canopy
    }

    /// Vrai si la colonne (x, z) est une colonne à surplomb (l'entaille du pied d'une paroi passe
    /// sous elle) : ni arbre ni structure n'y prennent place. Faux sans génération v2.
    pub(super) fn v2_overhanging(&self, x: i32, z: i32) -> bool {
        if self.v2.is_none() {
            return false;
        }
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        ctx.core_at(x.rem_euclid(size) as usize, z.rem_euclid(size) as usize)
            .reach
            > 0.0
    }

    /// Le niveau du dernier bloc d'eau de la colonne (x, z) en génération v2 (cours, lac ou mer), ou
    /// un niveau inférieur à tout relief s'il n'y en a pas.
    pub(super) fn v2_water_top(&self, x: i32, z: i32) -> i32 {
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        ctx.water_top(x.rem_euclid(size) as usize, z.rem_euclid(size) as usize)
    }

    /// Le bloc du sol de la colonne (celui où l'on pose un arbre ou une plante).
    pub(super) fn ground_block(&self, x: i32, z: i32, column: Column) -> BlockId {
        if self.v2.is_none() {
            return self.layer(x, column.height, z, column);
        }
        let size = CHUNK_SIZE as i32;
        let ctx = self.v2_context(x.div_euclid(size), z.div_euclid(size));
        let core = ctx.core_at(x.rem_euclid(size) as usize, z.rem_euclid(size) as usize);
        self.surface_block_of(core, x, z)
    }

    /// Les emprises (x, z, rayon) des structures qui touchent la colonne de chunks (cx, cz), élargie
    /// de 8 blocs (le feuillage d'un grand arbre) ; vide hors génération v2.
    pub(super) fn v2_footprints(&self, cx: i32, cz: i32) -> Vec<(i32, i32, i32)> {
        let Some(set) = self.structures.as_ref().filter(|_| self.v2.is_some()) else {
            return Vec::new();
        };
        let size = CHUNK_SIZE as i32;
        let (x0, z0) = (cx * size - 8, cz * size - 8);
        self.structure_sites_in((x0, z0), (x0 + size + 15, z0 + size + 15))
            .iter()
            .map(|s| (s.x, s.z, set.defs()[s.def].radius))
            .collect()
    }

    /// Vrai si une structure de rayon `radius` centrée en (x, z) tient hors de l'eau et des zones
    /// réservées : la grille de 6 blocs ne saute aucun cours (une rivière de 3 blocs de large, plus
    /// la distance de dégagement exigée, est toujours touchée).
    pub(super) fn v2_site_clear(&self, x: i32, z: i32, radius: i32) -> bool {
        if self.v2.is_none() {
            return true;
        }
        let reach = radius + 4;
        let mut dz = -reach;
        while dz <= reach {
            let mut dx = -reach;
            while dx <= reach {
                if self.v2_clearance(x + dx, z + dz) < 5 {
                    return false;
                }
                dx += 6;
            }
            dz += 6;
        }
        // Le sol de l'emprise est plein et loin d'une paroi : aucune colonne à surplombs ni à moins de
        // 8 blocs d'une ligne de rupture d'escarpement (un bord de falaise sous un toit) ; ailleurs la
        // densité ne creuse jamais. Une grille de 2 blocs : une paroi tient en quelques blocs.
        let mut dz = -radius;
        while dz <= radius {
            let mut dx = -radius;
            while dx <= radius {
                let column = self.surface_info(x + dx, z + dz);
                if column.is_some_and(|c| {
                    c.overhang > 0.0 || (c.escarpment > 0.0 && c.wall_distance.abs() < 8.0)
                }) {
                    return false;
                }
                dx += 2;
            }
            dz += 2;
        }
        true
    }

    /// Le générateur de blocs v2 : un chunk. `None` s'il ne contient que de l'air.
    pub(super) fn generate_v2(&self, chunk_pos: IVec3) -> Option<Chunk> {
        if !self.chunk_range().contains(&chunk_pos.y) {
            return None;
        }
        let origin = World::chunk_origin(chunk_pos);
        let ctx = self.v2_context(chunk_pos.x, chunk_pos.z);
        let plans = if self.v2.as_ref().is_some_and(|v2| v2.relief_only) {
            Vec::new()
        } else {
            self.structure_plans(chunk_pos)
        };
        let highest = ctx.max_y.max(SEA_LEVEL) + if plans.is_empty() { 0 } else { 40 };
        if origin.y > highest + SKY_MARGIN {
            return None;
        }
        let mut chunk = Chunk::new();
        chunk.set_biomes(ctx.biomes.clone());
        let relief_only = self.v2.as_ref().is_some_and(|v2| v2.relief_only);
        self.fill_terrain(&mut chunk, origin, &ctx);
        if !relief_only {
            self.carve_underground(&mut chunk, origin, &ctx);
        }
        self.place_ores(&mut chunk, chunk_pos, &ctx.columns);
        self.place_plants(&mut chunk, chunk_pos, &ctx.columns);
        self.apply_strata(&mut chunk, origin, &ctx);
        if !relief_only {
            self.place_trees(&mut chunk, chunk_pos);
            self.place_structures(&mut chunk, chunk_pos, &plans);
        }
        (!chunk.is_empty()).then_some(chunk)
    }

    /// Roche, sol, surface, mers, cours et lacs d'un chunk (avant cavernes et minerais).
    fn fill_terrain(&self, chunk: &mut Chunk, origin: IVec3, ctx: &ColumnContext) {
        let b = self.blocks;
        let size = CHUNK_SIZE as i32;
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let (x, z) = (origin.x + lx as i32, origin.z + lz as i32);
                let core = ctx.core_at(lx, lz);
                let column = &ctx.columns[lx][lz];
                let water_top = ctx.water_top(lx, lz);
                let river_or_lake = ctx.wide_at(lx as i32, lz as i32).water != NO_WATER;
                let frozen = !river_or_lake
                    && (column.temperature < FREEZING || column.climate_temperature < -0.55);
                let strata = self.strata_column(x, z);
                let solid = density::solid_column(core, &strata, origin.y);
                // Sous une entaille : les blocs qui ont de l'air au-dessus (sol de l'entaille) ou
                // dessous (voûte) ne portent jamais un sol de surface, seulement de la roche.
                let mut covered = [false; density::SPAN];
                if core.reach > 0.0 {
                    let (mut any, mut gap) = (false, false);
                    for i in (0..density::SPAN).rev() {
                        if solid[i] {
                            covered[i] = gap;
                            any = true;
                        } else if any {
                            gap = true;
                        }
                    }
                }
                for ly in 0..size {
                    let y = origin.y + ly;
                    let idx = (ly + density::BELOW) as usize;
                    let block = if y == 0
                        || (y <= 3 && hash3(self.seed, x, y, z, 1).is_multiple_of(y as u64 + 1))
                    {
                        b.bedrock
                    } else if solid[idx] {
                        // Combien de blocs solides au-dessus de celui-ci (jusqu'à l'air).
                        let depth = if core.reach <= 0.0 {
                            core.top as i32 - y
                        } else {
                            density::depth_below_air(&solid, idx)
                        };
                        let rock_only = core.reach > 0.0 && (covered[idx] || !solid[idx - 1]);
                        self.solid_block(
                            core,
                            IVec3::new(x, y, z),
                            depth,
                            !solid[idx - 1],
                            rock_only,
                        )
                    } else if y <= water_top {
                        if y == SEA_LEVEL && frozen && water_top == SEA_LEVEL {
                            b.ice
                        } else {
                            b.water
                        }
                    } else {
                        continue;
                    };
                    chunk.set(lx, ly as usize, lz, block);
                }
            }
        }
    }

    /// Le bloc d'un solide en `pos`, à `depth` blocs sous l'air (0 : la surface). `rock_only` : la
    /// roche, jamais un sol de surface (sol ou voûte d'une entaille).
    fn solid_block(
        &self,
        core: &CoreCell,
        pos: IVec3,
        depth: i32,
        below_open: bool,
        rock_only: bool,
    ) -> BlockId {
        let (x, y, z) = (pos.x, pos.y, pos.z);
        let b = self.blocks;
        let ruled = if rock_only {
            None
        } else {
            self.rule_block(&core.rule, core.biome, x, y, z, depth)
        };
        let block = ruled.unwrap_or_else(|| {
            // La roche : de la pierre profonde en bas, avec une frontière irrégulière.
            let jitter = (hash3(self.seed, x, y, z, 2) % 8) as i32;
            if y < DEEPSTONE_LEVEL - 4 + jitter {
                b.deepstone
            } else {
                b.stone
            }
        });
        // Sable et gravier tombent : jamais au-dessus du vide (face inférieure d'un surplomb).
        if below_open && self.registry.def(block).gravity {
            self.v2_blocks().sandstone
        } else {
            block
        }
    }

    pub(super) fn v2_blocks(&self) -> V2Blocks {
        self.v2.as_ref().expect("génération v2 active").blocks
    }

    /// Le biome d'un type donné le plus proche du climat. Les biomes d'une zone humide (climat
    /// d'humidité de [`WETLAND_HUMIDITY`] ou plus : le marais) ne sont candidats que là où la
    /// géographie fait une zone basse humide (`wet`) : le climat seul ne fait pas un marais.
    pub(super) fn v2_biome(&self, kind: BiomeKind, t: f64, h: f64, v: f64, wet: bool) -> BiomeId {
        let candidates = match kind {
            BiomeKind::Land => &self.land,
            other => self
                .by_kind
                .iter()
                .find(|(k, _)| *k == other)
                .map_or(&self.land, |(_, ids)| ids),
        };
        let allowed = |id: &BiomeId| wet || self.biomes.get(*id).climate.1 < WETLAND_HUMIDITY;
        candidates
            .iter()
            .filter(|id| allowed(id))
            .min_by(|a, b| {
                let distance =
                    |id: &&BiomeId| super::climate_distance(self.biomes.get(**id), t, h, v);
                distance(a).total_cmp(&distance(b))
            })
            .copied()
            .unwrap_or_else(|| self.nearest(candidates, t, h, v))
    }

    /// Un point d'apparition pour la génération v2 : la première terre plate hors de l'eau, en
    /// spirale depuis l'origine, jugée d'après la géographie macro (peu coûteuse), puis confirmée
    /// par les vraies colonnes.
    pub(super) fn find_spawn_v2(&self) -> (i32, i32) {
        let world = &self.v2.as_ref().expect("génération v2 active").world;
        let ok_macro = |x: i32, z: i32| {
            let g = world.geography().sample(x as f64, z as f64);
            g.coast_distance > 60.0
                && g.elevation > SEA_LEVEL as f32 + 2.0
                && g.elevation < SEA_LEVEL as f32 + 24.0
                && g.chain_height < 8.0
        };
        let ok = |x: i32, z: i32| {
            self.surface_info(x, z).is_some_and(|s| {
                !s.reserved
                    && s.water.is_none()
                    && s.top > SEA_LEVEL + 1
                    && s.top < SEA_LEVEL + 30
                    && self.biome(s.biome).kind == BiomeKind::Land
            })
        };
        let step = 16;
        for ring in 0..=1500 {
            let r = ring * step;
            for d in (-r..=r).step_by(step as usize) {
                for (x, z) in [(d, -r), (d, r), (-r, d), (r, d)] {
                    if ok_macro(x, z)
                        && [(8, 0), (-8, 0), (0, 8), (0, -8), (0, 0)]
                            .iter()
                            .all(|&(dx, dz)| ok(x + dx, z + dz))
                    {
                        return (x, z);
                    }
                }
            }
        }
        (0, 0)
    }
}

/// L'humidité de climat à partir de laquelle un biome de terre est un biome de zone humide.
pub(super) const WETLAND_HUMIDITY: f64 = 0.7;

/// L'altitude de la lave et du premier bloc d'eau d'une nappe : voir `underground.rs`.
pub(super) const LAVA_GAP: i32 = 4;

/// Vérifie à la compilation que les constantes s'accordent.
const _: () = assert!(LAVA_LEVEL + LAVA_GAP < SEA_LEVEL);
