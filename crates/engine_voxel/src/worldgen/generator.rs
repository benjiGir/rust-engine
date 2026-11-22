//! Génération procédurale du terrain : relief, biomes, sous-sol, grottes et minerais.
//!
//! # Le bruit
//!
//! Le bruit de Perlin donne, pour chaque point, une valeur entre -1 et 1 qui varie
//! doucement : deux points proches ont des valeurs proches. En superposant plusieurs
//! « octaves » (de plus en plus fines et de moins en moins fortes), on obtient un
//! relief naturel : c'est le fBm (« fractional Brownian motion »).
//!
//! Tout part d'une graine (`seed`) : la même graine donne toujours le même monde,
//! sur toutes les machines. On peut donc générer n'importe quel chunk, n'importe
//! quand, dans n'importe quel ordre et sur n'importe quel thread.
//!
//! # Le climat et le relief
//!
//! Pour chaque colonne, on lit plusieurs cartes de bruit très étalées :
//! - la **continentalité** : de l'océan profond (très négatif) à l'intérieur des terres ;
//! - l'**érosion** : faible érosion = montagnes ;
//! - la **température** et l'**humidité** : le climat, qui choisit le biome des terres ;
//! - une carte des **rivières** : elles coulent là où cette carte passe par zéro.
//!
//! La hauteur vient de la continentalité (une courbe : océan → côte → plateau), plus
//! des collines dont l'amplitude dépend du biome, plus des crêtes de montagne là où
//! l'érosion est faible. Le relief des biomes est **mélangé** selon la distance
//! climatique : à la frontière entre une plaine et une forêt, le relief passe de l'un
//! à l'autre en douceur, sans falaise.
//!
//! # Interpolation
//!
//! Évaluer une dizaine de bruits à plusieurs octaves pour chacun des 1 024 blocs d'un
//! chunk serait coûteux. On ne les évalue que tous les 4 blocs (une grille de 9 × 9 par
//! chunk) et on **interpole** entre ces points : le résultat est presque identique (ces
//! cartes varient lentement) et 12 fois moins cher. Même chose en 3D pour les grottes.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use glam::IVec3;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;
use crate::world::World;
use crate::worldgen::biome::{BiomeDef, BiomeId, BiomeKind, BiomeRegistry};
use crate::worldgen::generation::GenerationVersion;
use crate::worldgen::trees::TreeBlocks;

/// Hauteur utile du monde, en chunks : les blocs vont de y = 0 à y = 32 × 6 - 1.
pub const WORLD_HEIGHT_CHUNKS: i32 = 6;
/// Les bornes de stockage du monde, en couches de chunks (`MIN_CHUNK_Y..MAX_CHUNK_Y`) : y de -128 à
/// 255. Chaque génération n'en occupe qu'une partie (voir [`TerrainGenerator::chunk_range`]) : les
///      mondes v1 reste de 0 à 191 (`WORLD_HEIGHT_CHUNKS` couches).
pub const MIN_CHUNK_Y: i32 = -4;
pub const MAX_CHUNK_Y: i32 = 8;
const MAX_HEIGHT: i32 = WORLD_HEIGHT_CHUNKS * CHUNK_SIZE as i32 - 2;
/// Niveau de la mer : les creux du terrain sous cette altitude se remplissent d'eau.
pub const SEA_LEVEL: i32 = 62;
/// Sous cette altitude (environ), la pierre devient de la pierre profonde.
pub const DEEPSTONE_LEVEL: i32 = 16;
/// Les grottes sous cette altitude sont remplies de lave.
pub const LAVA_LEVEL: i32 = 10;
/// En dessous de cette température, il neige et l'eau gèle.
pub const FREEZING: f32 = 0.15;

/// Pas de la grille d'interpolation, en blocs.
pub(crate) const STEP: i32 = 4;
/// Points de la grille par côté de chunk (32 / 4 + 1, les deux bords compris).
const GRID: usize = CHUNK_SIZE / STEP as usize + 1;

/// Le vide : la plateforme (si elle existe) couvre de -8 à 8 en x et z, à cette hauteur.
const VOID_PLATFORM: i32 = 8;
const VOID_PLATFORM_Y: i32 = 64;

/// Ce que le générateur sait d'une colonne (x, z) : sa hauteur et son biome.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Column {
    /// y du bloc le plus haut de la colonne.
    pub height: i32,
    pub biome: BiomeId,
    /// Température ressentie à la surface (celle du biome, moins le froid de l'altitude).
    pub temperature: f32,
    /// La valeur brute de température du climat (de -1 à 1) : gèle aussi les océans froids.
    pub climate_temperature: f32,
}

/// Les cartes de climat en un point (au bloc), pour les outils d'observation : voir
/// [`TerrainGenerator::sample`]. Les valeurs sont celles que lit la génération, avant le choix
/// du biome et de la hauteur.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Climate {
    /// De l'océan profond (très négatif) à l'intérieur des terres.
    pub continentalness: f32,
    /// Faible érosion : montagnes.
    pub erosion: f32,
    /// De -1 (glacial) à 1 (brûlant).
    pub temperature: f32,
    /// De -1 (aride) à 1 (détrempé).
    pub humidity: f32,
    pub variety: f32,
    /// La carte des rivières : elles coulent là où elle passe par zéro.
    pub river: f32,
}

/// Les cartes de bruit en un point, avant interprétation.
#[derive(Debug, Clone, Copy, Default)]
struct Fields {
    continentalness: f64,
    erosion: f64,
    temperature: f64,
    humidity: f64,
    /// La « variété » : choisit entre un biome et ses variantes (prairie fleurie, forêt
    /// sombre, collines boisées...) sous un même climat.
    variety: f64,
    river: f64,
    hills: f64,
    ridge: f64,
    /// Le relief des biomes des terres, déjà mélangé selon le climat (décalage, amplitude).
    /// Calculé aux points de grille seulement : le mélange coûte une exponentielle par biome.
    relief_offset: f64,
    relief_scale: f64,
    relief_terraces: f64,
}

impl Fields {
    /// Interpolation bilinéaire entre quatre coins (`fx`, `fz` entre 0 et 1).
    fn bilerp(a: Fields, b: Fields, c: Fields, d: Fields, fx: f64, fz: f64) -> Fields {
        let mix = |a: f64, b: f64, c: f64, d: f64| {
            let top = a + (b - a) * fx;
            let bottom = c + (d - c) * fx;
            top + (bottom - top) * fz
        };
        Fields {
            continentalness: mix(
                a.continentalness,
                b.continentalness,
                c.continentalness,
                d.continentalness,
            ),
            erosion: mix(a.erosion, b.erosion, c.erosion, d.erosion),
            temperature: mix(a.temperature, b.temperature, c.temperature, d.temperature),
            humidity: mix(a.humidity, b.humidity, c.humidity, d.humidity),
            variety: mix(a.variety, b.variety, c.variety, d.variety),
            river: mix(a.river, b.river, c.river, d.river),
            hills: mix(a.hills, b.hills, c.hills, d.hills),
            ridge: mix(a.ridge, b.ridge, c.ridge, d.ridge),
            relief_offset: mix(
                a.relief_offset,
                b.relief_offset,
                c.relief_offset,
                d.relief_offset,
            ),
            relief_scale: mix(
                a.relief_scale,
                b.relief_scale,
                c.relief_scale,
                d.relief_scale,
            ),
            relief_terraces: mix(
                a.relief_terraces,
                b.relief_terraces,
                c.relief_terraces,
                d.relief_terraces,
            ),
        }
    }
}

/// Les blocs fixes du générateur, retrouvés par leur nom au démarrage.
#[derive(Debug, Clone, Copy)]
struct TerrainBlocks {
    stone: BlockId,
    deepstone: BlockId,
    bedrock: BlockId,
    snow: BlockId,
    water: BlockId,
    ice: BlockId,
    lava: BlockId,
    sand: BlockId,
}

impl TerrainBlocks {
    fn resolve(registry: &BlockRegistry) -> Result<Self> {
        // Une closure qui transforme un `None` en erreur explicite.
        let id = |key: &str| {
            registry
                .id(key)
                .with_context(|| format!("le générateur a besoin du bloc « {key} »"))
        };
        Ok(Self {
            stone: id("core:stone")?,
            deepstone: id("core:deepstone")?,
            bedrock: id("core:bedrock")?,
            snow: id("core:snow")?,
            water: id("core:water")?,
            ice: id("core:ice")?,
            lava: id("core:lava")?,
            sand: id("core:sand")?,
        })
    }
}

/// Le générateur de terrain. Il ne contient que des tables de bruit et des données en
/// lecture seule : il est donc `Send + Sync` et peut être partagé entre threads avec un
/// `Arc` (voir `streaming.rs`), sans verrou.
#[derive(Debug, Clone)]
pub struct TerrainGenerator {
    seed: u32,
    /// Comment le terrain est calculé (voir `worldgen/generation.rs`).
    version: GenerationVersion,
    /// L'état de la génération v3 (densité 3D, voir `v3.rs`) ; `None` sinon.
    v3: Option<Arc<v3::V3State>>,
    continentalness: Fbm<Perlin>,
    erosion: Fbm<Perlin>,
    temperature: Fbm<Perlin>,
    humidity: Fbm<Perlin>,
    variety: Fbm<Perlin>,
    /// Deux cartes qui décalent le point où l'on lit le climat : les frontières entre biomes
    /// se tordent au lieu de suivre des courbes trop lisses (« domain warping »).
    warp_x: Fbm<Perlin>,
    warp_z: Fbm<Perlin>,
    river: Fbm<Perlin>,
    /// Les collines, dont l'amplitude dépend du biome.
    hills: Fbm<Perlin>,
    /// Les crêtes des montagnes.
    ridges: Fbm<Perlin>,
    /// Deux champs 3D : un tunnel passe là où les deux sont proches de zéro.
    cave_a: Perlin,
    cave_b: Perlin,
    /// Les grandes cavernes (« fromage ») : là où ce champ est haut.
    cheese: Fbm<Perlin>,
    /// Les nappes d'eau souterraines.
    aquifer: Perlin,
    registry: Arc<BlockRegistry>,
    biomes: Arc<BiomeRegistry>,
    /// Les biomes des terres, choisis par le climat.
    land: Vec<BiomeId>,
    /// Les biomes de chaque autre famille (océans, plages, montagnes...), eux aussi choisis
    /// par le climat parmi ceux de leur famille.
    by_kind: Vec<(BiomeKind, Vec<BiomeId>)>,
    blocks: TerrainBlocks,
    plant_blocks: PlantBlocks,
    /// `None` si un bloc d'arbre manque (un registre de test réduit) : pas d'arbres.
    tree_blocks: Option<TreeBlocks>,
    /// Les arbres déjà tirés, partagés entre les threads (et entre les clones du générateur).
    tree_cache: Arc<features::TreeCache>,
    /// Les sites de structures déjà évalués (voir `structures.rs` : `site_in_cell`).
    site_cache: Arc<structures::SiteCache>,
    /// Les colonnes déjà calculées, partagées de la même façon (voir `column_cache.rs`).
    column_cache: Arc<ColumnCache>,
    /// Une plage de couches imposée (voir [`TerrainGenerator::with_chunk_range`]), sinon celle de
    /// la version.
    chunk_range: Option<std::ops::Range<i32>>,
    /// Ce qui distingue la dimension de ce générateur dans les clés du cache des colonnes.
    dimension_tag: u64,
    /// Villages, ruines, donjons, mines (phase O9) ; `None` si un bloc manque.
    structures: Option<StructureSet>,
    structure_blocks: Option<StructureBlocks>,
    /// Le genre de monde : le terrain du jeu (par défaut), un monde plat, ou le vide.
    world: crate::worldgen::dimension::WorldKind,
}

impl TerrainGenerator {
    /// Un générateur avec les blocs et les biomes de base intégrés.
    pub fn new(seed: u32) -> Self {
        Self::with_registry(seed, BlockRegistry::builtin())
            .expect("les blocs de base intégrés sont complets")
    }

    /// Un générateur qui utilise ce registre de blocs (core + mods) et les biomes de base.
    pub fn with_registry(seed: u32, registry: Arc<BlockRegistry>) -> Result<Self> {
        let biomes = Arc::new(BiomeRegistry::builtin(&registry)?);
        Self::with_biomes(seed, registry, biomes)
    }

    /// Un générateur qui utilise aussi les minerais, biomes et structures des mods (leurs
    /// `ores.ron`, `biomes.ron` et `structures.ron`, dans l'ordre de chargement). Un fichier de
    /// mod invalide est une erreur.
    pub fn with_mods(
        seed: u32,
        registry: Arc<BlockRegistry>,
        mods: &[(String, std::path::PathBuf)],
    ) -> Result<Self> {
        let biomes = Arc::new(BiomeRegistry::load_mods(&registry, mods)?);
        let sources = structures::mod_sources(mods)?;
        Self::build(seed, registry, biomes, &sources)
    }

    /// Le générateur d'une dimension : sa graine est celle du monde plus `seed_offset`, et son
    /// genre de monde (terrain, plat, vide) vient de sa description.
    pub fn with_dimension(
        seed: u32,
        registry: Arc<BlockRegistry>,
        mods: &[(String, std::path::PathBuf)],
        dimension: &crate::worldgen::dimension::DimensionDef,
    ) -> Result<Self> {
        let mut generator =
            Self::with_mods(seed.wrapping_add(dimension.seed_offset), registry, mods)?;
        generator.apply_world(&dimension.kind);
        Ok(generator)
    }

    /// Règle le genre de monde. Un monde plat ou vide n'a ni structures ni biomes à choisir ; un
    /// terrain peut se limiter à certains biomes de terre.
    fn apply_world(&mut self, kind: &crate::worldgen::dimension::WorldKind) {
        use crate::worldgen::dimension::WorldKind;
        match kind {
            WorldKind::Terrain { biomes } if !biomes.is_empty() => {
                self.land.retain(|b| biomes.contains(b));
                if self.land.is_empty() {
                    self.land = biomes.clone();
                }
            }
            WorldKind::Terrain { .. } => {}
            WorldKind::Flat { .. } | WorldKind::Void { .. } => {
                self.structures = None;
            }
        }
        self.world = kind.clone();
        // Les arbres et les colonnes dépendent des biomes, du genre de monde et des villages : on
        // repart de caches vides (et neufs, pour ne rien partager avec un clone qui n'a pas
        // changé).
        self.tree_cache = Arc::default();
        self.site_cache = Arc::default();
        self.column_cache = Arc::default();
        self.dimension_tag = self.compute_dimension_tag();
        if self.v3.is_some() {
            self.v3 = v3::V3State::new(self.seed, &self.registry, &self.biomes)
                .ok()
                .map(Arc::new);
        }
    }

    /// L'empreinte du genre de monde et des biomes de terre permis, pour les clés de cache.
    fn compute_dimension_tag(&self) -> u64 {
        crate::worldgen::generation::fnv1a(format!("{:?}|{:?}", self.world, self.land).as_bytes())
    }

    /// La colonne d'un monde plat ou vide (la même partout, sauf la plateforme du vide).
    fn constant_column(&self, x: i32, z: i32) -> Option<Column> {
        use crate::worldgen::dimension::WorldKind;
        let (height, biome) = match &self.world {
            WorldKind::Terrain { .. } => return None,
            WorldKind::Flat { layers, biome } => (
                layers.iter().map(|(_, n)| *n as i32).sum::<i32>() - 1,
                *biome,
            ),
            WorldKind::Void { platform } => (
                if platform.is_some() && x.abs() <= VOID_PLATFORM && z.abs() <= VOID_PLATFORM {
                    VOID_PLATFORM_Y
                } else {
                    0
                },
                self.biomes.first_of(BiomeKind::Land),
            ),
        };
        let temperature = self.biomes.get(biome).temperature;
        Some(Column {
            height,
            biome,
            temperature,
            climate_temperature: 0.0,
        })
    }

    /// Un chunk d'un monde plat ou vide ; `None` pour le terrain (voir [`Self::generate`]).
    fn generate_constant(&self, chunk_pos: IVec3) -> Option<Option<Chunk>> {
        use crate::worldgen::dimension::WorldKind;
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;
        let mut chunk = Chunk::new();
        let mut any = false;
        match &self.world {
            WorldKind::Terrain { .. } => return None,
            WorldKind::Flat { layers, .. } => {
                if !self.chunk_range().contains(&chunk_pos.y) {
                    return Some(None);
                }
                let mut bottom = 0;
                for &(block, count) in layers {
                    let top = bottom + count as i32;
                    for y in bottom.max(origin.y)..top.min(origin.y + size) {
                        any = true;
                        for lz in 0..CHUNK_SIZE {
                            for lx in 0..CHUNK_SIZE {
                                chunk.set(lx, (y - origin.y) as usize, lz, block);
                            }
                        }
                    }
                    bottom = top;
                }
            }
            WorldKind::Void { platform } => {
                let Some(block) = platform else {
                    return Some(None);
                };
                let y = VOID_PLATFORM_Y;
                if (origin.y..origin.y + size).contains(&y) {
                    for lz in 0..size {
                        for lx in 0..size {
                            let (x, z) = (origin.x + lx, origin.z + lz);
                            if x.abs() <= VOID_PLATFORM && z.abs() <= VOID_PLATFORM {
                                any = true;
                                chunk.set(
                                    lx as usize,
                                    (y - origin.y) as usize,
                                    lz as usize,
                                    *block,
                                );
                            }
                        }
                    }
                }
            }
        }
        if !any {
            return Some(None);
        }
        chunk.set_biomes(self.biome_columns(chunk_pos));
        Some(Some(chunk))
    }

    /// Un générateur avec ses propres biomes. Échoue s'il manque un des blocs de base.
    pub fn with_biomes(
        seed: u32,
        registry: Arc<BlockRegistry>,
        biomes: Arc<BiomeRegistry>,
    ) -> Result<Self> {
        Self::build(seed, registry, biomes, &[])
    }

    fn build(
        seed: u32,
        registry: Arc<BlockRegistry>,
        biomes: Arc<BiomeRegistry>,
        structure_sources: &[(String, String, String)],
    ) -> Result<Self> {
        let blocks = TerrainBlocks::resolve(&registry)?;
        // Chaque bruit reçoit une graine différente, dérivée de la graine du monde,
        // pour qu'ils ne se ressemblent pas. `wrapping_add` : pas de panique si ça déborde.
        let fbm = |offset: u32, octaves: usize, frequency: f64| {
            Fbm::<Perlin>::new(seed.wrapping_add(offset))
                .set_octaves(octaves)
                .set_frequency(frequency)
                .set_lacunarity(2.0)
                .set_persistence(0.5)
        };
        // Les biomes réservés à la v3 n'entrent pas dans les choix de la v1 (dont les mondes ne
        // doivent pas changer) ; la v3 a sa propre table (`biomes_v3.ron`).
        let land = biomes
            .iter()
            .filter(|(_, b)| b.kind == BiomeKind::Land && !b.v3_only)
            .map(|(id, _)| id)
            .collect();
        let by_kind = [
            BiomeKind::Ocean,
            BiomeKind::DeepOcean,
            BiomeKind::Beach,
            BiomeKind::River,
            BiomeKind::Mountains,
            BiomeKind::Peaks,
        ]
        .into_iter()
        .map(|kind| {
            let ids = biomes
                .iter()
                .filter(|(_, b)| b.kind == kind && !b.v3_only)
                .map(|(id, _)| id)
                .collect();
            (kind, ids)
        })
        .collect();
        let (structures, structure_blocks) =
            structures::load(&registry, &biomes, structure_sources)?;
        let world = crate::worldgen::dimension::WorldKind::Terrain { biomes: Vec::new() };
        let mut generator = Self {
            seed,
            version: GenerationVersion::V1,
            v3: None,
            structures,
            structure_blocks,
            continentalness: fbm(0, 4, 1.0 / 1600.0),
            erosion: fbm(1, 3, 1.0 / 900.0),
            temperature: fbm(2, 3, 1.0 / 3200.0),
            humidity: fbm(3, 3, 1.0 / 2600.0),
            variety: fbm(11, 2, 1.0 / 1400.0),
            warp_x: fbm(12, 3, 1.0 / 520.0),
            warp_z: fbm(13, 3, 1.0 / 520.0),
            river: fbm(4, 3, 1.0 / 700.0),
            hills: fbm(5, 4, 1.0 / 220.0),
            ridges: fbm(6, 4, 1.0 / 180.0),
            cave_a: Perlin::new(seed.wrapping_add(7)),
            cave_b: Perlin::new(seed.wrapping_add(8)),
            cheese: fbm(9, 2, 1.0),
            aquifer: Perlin::new(seed.wrapping_add(10)),
            biomes,
            land,
            by_kind,
            blocks,
            plant_blocks: PlantBlocks::resolve(&registry)?,
            tree_blocks: TreeBlocks::resolve(&registry).ok(),
            tree_cache: Arc::default(),
            site_cache: Arc::default(),
            column_cache: Arc::default(),
            chunk_range: None,
            dimension_tag: 0,
            world,
            registry,
        };
        generator.dimension_tag = generator.compute_dimension_tag();
        Ok(generator)
    }

    /// Les couches de chunks que cette génération remplit, de bas en haut : en dehors, il n'y a que
    /// de l'air (au-dessus) ou du vide (en dessous), et le streamer ne les demande pas.
    pub fn chunk_range(&self) -> std::ops::Range<i32> {
        if let Some(range) = &self.chunk_range {
            return range.clone();
        }
        if self.v3.is_some()
            && matches!(
                self.world,
                crate::worldgen::dimension::WorldKind::Terrain { .. }
            )
        {
            return MIN_CHUNK_Y..MAX_CHUNK_Y;
        }
        0..WORLD_HEIGHT_CHUNKS
    }

    /// Impose une plage de couches (bornée au stockage, `MIN_CHUNK_Y..MAX_CHUNK_Y`). Sert aux tests
    /// du monde haut : la génération v1 n'étant pas définie sous 0, elle y met de la pierre creusée
    /// d'une salle fermée par chunk. La v3 aura la sienne.
    pub fn with_chunk_range(mut self, range: std::ops::Range<i32>) -> Self {
        let start = range.start.max(MIN_CHUNK_Y);
        let end = range.end.min(MAX_CHUNK_Y).max(start + 1);
        self.chunk_range = Some(start..end);
        self
    }

    pub fn registry(&self) -> &Arc<BlockRegistry> {
        &self.registry
    }

    pub fn biomes(&self) -> &Arc<BiomeRegistry> {
        &self.biomes
    }

    pub fn biome(&self, id: BiomeId) -> &BiomeDef {
        self.biomes.get(id)
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// La version de génération de ce générateur.
    pub fn version(&self) -> GenerationVersion {
        self.version
    }

    /// Les cartes de bruit au point de grille (gx, gz), soit au bloc (4 gx, 4 gz).
    fn fields(&self, gx: i32, gz: i32) -> Fields {
        let p = [(gx * STEP) as f64, (gz * STEP) as f64];
        // Le climat est lu un peu à côté, d'un décalage qui varie : ses frontières ondulent.
        const WARP: f64 = 120.0;
        let q = [
            p[0] + WARP * self.warp_x.get(p),
            p[1] + WARP * self.warp_z.get(p),
        ];
        // Les cartes du climat sont un peu amplifiées : un fBm reste souvent près de 0,
        // on veut des climats extrêmes (déserts, toundras) assez fréquents.
        let mut fields = Fields {
            // Décalé vers le haut : environ un tiers du monde sous la mer.
            continentalness: self.continentalness.get(p) * 1.4 + 0.08,
            erosion: self.erosion.get(p) * 1.4,
            temperature: (self.temperature.get(q) * 1.8).clamp(-1.0, 1.0),
            humidity: (self.humidity.get(q) * 1.8).clamp(-1.0, 1.0),
            variety: (self.variety.get(q) * 1.4).clamp(-1.0, 1.0),
            river: self.river.get(p),
            hills: self.hills.get(p),
            ridge: 1.0 - self.ridges.get(p).abs(),
            relief_offset: 0.0,
            relief_scale: 0.0,
            relief_terraces: 0.0,
        };
        (
            fields.relief_offset,
            fields.relief_scale,
            fields.relief_terraces,
        ) = self.land_relief(fields.temperature, fields.humidity, fields.variety);
        fields
    }

    /// Le relief des terres pour ce climat, mélangé entre les biomes. Le poids d'un biome
    /// décroît avec sa distance climatique (une gaussienne) : les poids varient continûment,
    /// donc la hauteur aussi.
    fn land_relief(&self, temperature: f64, humidity: f64, variety: f64) -> (f64, f64, f64) {
        let (mut offset, mut scale, mut terraces, mut total) = (0.0, 0.0, 0.0, 0.0);
        for &id in &self.land {
            let b = self.biomes.get(id);
            let d2 = climate_distance(b, temperature, humidity, variety);
            let w = (-d2 / 0.06).exp() + 1e-9;
            offset += w * b.relief.offset;
            scale += w * b.relief.scale;
            terraces += w * b.relief.terraces;
            total += w;
        }
        (offset / total, scale / total, terraces / total)
    }

    /// Les cartes interpolées au bloc (x, z), depuis les 4 points de grille autour.
    fn fields_at(&self, x: i32, z: i32, grid: impl Fn(i32, i32) -> Fields) -> Fields {
        let (gx, gz) = (x.div_euclid(STEP), z.div_euclid(STEP));
        let fx = x.rem_euclid(STEP) as f64 / STEP as f64;
        let fz = z.rem_euclid(STEP) as f64 / STEP as f64;
        Fields::bilerp(
            grid(gx, gz),
            grid(gx + 1, gz),
            grid(gx, gz + 1),
            grid(gx + 1, gz + 1),
            fx,
            fz,
        )
    }

    /// Le biome au bloc (x, y, z) : celui d'une grotte souterraine (génération v3) s'il y en a un,
    /// sinon celui de la colonne.
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> BiomeId {
        self.v3_cave_biome(x, y, z)
            .unwrap_or_else(|| self.column(x, z).biome)
    }

    /// Hauteur et biome d'une colonne.
    pub fn column(&self, x: i32, z: i32) -> Column {
        if let Some(column) = self.constant_column(x, z) {
            return column;
        }
        if self.v3.is_some() {
            return self.column_v3(x, z);
        }
        let fields = self.fields_at(x, z, |gx, gz| self.fields(gx, gz));
        self.interpret(fields)
    }

    /// La colonne et le climat au bloc (x, z), pour les outils d'observation (cartes exportées
    /// par `cargo xtask map`). La colonne est exactement celle de [`Self::column`] ; le climat
    /// est nul dans un monde plat ou vide.
    pub fn sample(&self, x: i32, z: i32) -> (Column, Climate) {
        if let Some(column) = self.constant_column(x, z) {
            return (column, Climate::default());
        }
        if self.v3.is_some() {
            return (self.column_v3(x, z), Climate::default());
        }
        let f = self.fields_at(x, z, |gx, gz| self.fields(gx, gz));
        let climate = Climate {
            continentalness: f.continentalness as f32,
            erosion: f.erosion as f32,
            temperature: f.temperature as f32,
            humidity: f.humidity as f32,
            variety: f.variety as f32,
            river: f.river as f32,
        };
        (self.interpret(f), climate)
    }

    /// La colonne au point de grille (gx, gz), soit au bloc (4 gx, 4 gz) : les cartes y sont
    /// évaluées une seule fois, sans interpolation. Quatre fois moins cher que [`Self::column`] :
    /// pour le terrain lointain, qui échantillonne sur cette grille.
    pub fn column_at_grid_point(&self, gx: i32, gz: i32) -> Column {
        if let Some(column) = self.constant_column(gx * STEP, gz * STEP) {
            return column;
        }
        if self.v3.is_some() {
            // Le terrain lointain : une recherche sur la seule verticale, sans contexte de colonne.
            return self.column_far_v3(gx * STEP, gz * STEP);
        }
        self.interpret(self.fields(gx, gz))
    }

    /// Les cartes deviennent une hauteur et un biome.
    fn interpret(&self, f: Fields) -> Column {
        let sea = SEA_LEVEL as f64;
        let c = f.continentalness;

        // 1. La forme des continents : une courbe par morceaux (océan profond, côte, plateau).
        let base = spline(
            c,
            &[
                (-1.0, -38.0),
                (-0.5, -30.0),
                (-0.3, -15.0),
                (-0.15, -5.0),
                (-0.06, -1.0),
                (0.02, 2.0),
                (0.3, 9.0),
                (0.7, 20.0),
                (1.2, 30.0),
            ],
        );

        // 2. Le relief des terres, mélangé entre les biomes selon le climat (voir
        //    `land_relief`).
        let (offset, scale) = (f.relief_offset, f.relief_scale);
        // Sur les côtes et en mer, le relief du biome s'efface.
        let inland = smoothstep(-0.12, 0.05, c);
        let mut land = offset + scale * 12.0 * f.hills;
        // Les terrasses : des paliers de 6 blocs, plats sur les trois quarts de leur largeur,
        // puis une petite falaise jusqu'au palier suivant.
        if f.relief_terraces > 0.0 {
            const STEP: f64 = 6.0;
            let k = land / STEP;
            let stepped = (k.floor() + smoothstep(0.7, 1.0, k.fract())) * STEP;
            land += (stepped - land) * f.relief_terraces.clamp(0.0, 1.0);
        }
        let mut height = sea + base + inland * land + (1.0 - inland) * 3.0 * f.hills;

        // 3. Les montagnes : là où l'érosion est faible, dans les terres. Une crête
        //    (1 - |bruit|, pointue au milieu) au carré donne des sommets aigus.
        let mountain = smoothstep(-0.05, -0.55, f.erosion) * smoothstep(0.0, 0.3, c);
        height += mountain * (12.0 + 90.0 * f.ridge * f.ridge);

        // 4. Les rivières : là où leur carte passe par zéro, on creuse jusque sous la mer.
        //    Pas en mer, et de moins en moins en altitude (pas de canyon en montagne).
        let channel = smoothstep(0.05, 0.012, f.river.abs())
            * smoothstep(-0.12, -0.02, c)
            * smoothstep(sea + 45.0, sea + 18.0, height);
        height += (sea - 4.0 - height) * channel;

        let height = (height.round() as i32).clamp(1, MAX_HEIGHT);

        // 5. Le biome : d'abord la famille (par le relief), puis le climat pour les terres.
        let kind = if height < SEA_LEVEL {
            if channel > 0.35 && c > -0.12 {
                BiomeKind::River
            } else if c < -0.42 {
                BiomeKind::DeepOcean
            } else {
                BiomeKind::Ocean
            }
        } else if mountain > 0.5 && height > SEA_LEVEL + 10 {
            if height > SEA_LEVEL + 85 {
                BiomeKind::Peaks
            } else {
                BiomeKind::Mountains
            }
        } else if c < 0.03 && height <= SEA_LEVEL + 2 && channel < 0.35 {
            BiomeKind::Beach
        } else {
            BiomeKind::Land
        };
        let candidates = match kind {
            BiomeKind::Land => &self.land,
            other => self
                .by_kind
                .iter()
                .find(|(k, _)| *k == other)
                .map_or(&self.land, |(_, ids)| ids),
        };
        let biome = self.nearest(candidates, f.temperature, f.humidity, f.variety);
        // Il fait plus froid en altitude : la neige descend des sommets.
        let altitude = (height - (SEA_LEVEL + 40)).max(0) as f32;
        let temperature = self.biomes.get(biome).temperature - altitude * 0.012;
        Column {
            height,
            biome,
            temperature,
            climate_temperature: f.temperature as f32,
        }
    }

    /// La colonne la plus proche de `(x, z)` (en spirale, tous les `step` blocs, jusqu'à
    /// `radius`) qui vérifie `wanted`.
    pub fn find_column(
        &self,
        x: i32,
        z: i32,
        step: i32,
        radius: i32,
        wanted: impl Fn(Column) -> bool,
    ) -> Option<(i32, i32)> {
        if wanted(self.column(x, z)) {
            return Some((x, z));
        }
        (1..=radius / step).find_map(|ring| {
            let r = ring * step;
            (-r..=r).step_by(step as usize).find_map(|d| {
                [
                    (x + d, z - r),
                    (x + d, z + r),
                    (x - r, z + d),
                    (x + r, z + d),
                ]
                .into_iter()
                .find(|&(cx, cz)| wanted(self.column(cx, cz)))
            })
        })
    }

    /// Un point d'apparition : la terre ferme la plus proche de l'origine (pas l'océan,
    /// ni une montagne), avec un peu de marge autour.
    pub fn find_spawn(&self) -> (i32, i32) {
        if !matches!(
            self.world,
            crate::worldgen::dimension::WorldKind::Terrain { .. }
        ) {
            return (0, 0);
        }
        let ok = |c: Column| {
            self.biome(c.biome).kind == BiomeKind::Land
                && c.height > SEA_LEVEL + 1
                && c.height < SEA_LEVEL + 30
        };
        self.find_column(0, 0, 16, 20_000, ok)
            .and_then(|(x, z)| {
                // Pas au bord de l'eau : on veut aussi de la terre à 8 blocs autour.
                let around = [(8, 0), (-8, 0), (0, 8), (0, -8)];
                if around
                    .iter()
                    .all(|(dx, dz)| ok(self.column(x + dx, z + dz)))
                {
                    Some((x, z))
                } else {
                    self.find_column(x + 24, z + 24, 8, 2000, ok)
                }
            })
            .unwrap_or((0, 0))
    }

    /// Parmi `candidates`, le biome dont le climat est le plus proche.
    fn nearest(
        &self,
        candidates: &[BiomeId],
        temperature: f64,
        humidity: f64,
        variety: f64,
    ) -> BiomeId {
        let distance =
            |id: &BiomeId| climate_distance(self.biomes.get(*id), temperature, humidity, variety);
        candidates
            .iter()
            .copied()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .unwrap_or_default()
    }

    /// Un chunk de pierre avec une salle fermée au centre (voir [`Self::with_chunk_range`]).
    fn deep_test_chunk(&self) -> Chunk {
        let mut chunk = Chunk::new();
        let size = CHUNK_SIZE;
        for y in 0..size {
            for z in 0..size {
                for x in 0..size {
                    let hall = (8..24).contains(&x) && (8..24).contains(&z) && (8..20).contains(&y);
                    if !hall {
                        chunk.set(x, y, z, self.blocks.stone);
                    }
                }
            }
        }
        chunk
    }

    /// Le bloc d'une colonne à l'altitude `y`, sans les grottes ni les minerais.
    fn layer(&self, x: i32, y: i32, z: i32, column: Column) -> BlockId {
        let b = self.blocks;
        // Le socle : la couche 0 partout, puis de moins en moins jusqu'à y = 3.
        if y == 0 || (y <= 3 && hash3(self.seed, x, y, z, 1).is_multiple_of(y as u64 + 1)) {
            return b.bedrock;
        }
        let biome = self.biomes.get(column.biome);
        let depth = column.height - y;
        let underwater = column.height < SEA_LEVEL;
        let rocky = biome.rock_above.is_some_and(|h| column.height > h);
        let cold = column.temperature < FREEZING;
        if depth == 0 {
            return if underwater {
                biome.underwater
            } else if cold && !rocky && biome.surface != b.sand {
                b.snow
            } else if rocky {
                if cold { b.snow } else { b.stone }
            } else {
                self.surface_patch(biome, x, z).unwrap_or(biome.surface)
            };
        }
        if depth < biome.subsurface_depth {
            return if underwater {
                biome.underwater
            } else if rocky {
                b.stone
            } else if !biome.bands.is_empty() && depth > 1 {
                self.band(biome, x, y, z)
            } else {
                biome.subsurface
            };
        }
        // La roche : de la pierre profonde en bas, avec une frontière irrégulière.
        let jitter = (hash3(self.seed, x, y, z, 2) % 8) as i32;
        if y < DEEPSTONE_LEVEL - 4 + jitter {
            b.deepstone
        } else {
            b.stone
        }
    }

    /// Le sol d'une tache en (x, z), s'il y en a une : chaque sorte de tache a son bruit, et
    /// couvre à peu près la part du sol demandée.
    fn surface_patch(&self, biome: &BiomeDef, x: i32, z: i32) -> Option<BlockId> {
        biome
            .ground_patches
            .iter()
            .enumerate()
            .find_map(|(i, &(block, part))| {
                let salt = 40 + 2 * i as u64;
                let n = 0.7 * value_noise(self.seed, x, z, 9, salt)
                    + 0.3 * value_noise(self.seed, x, z, 3, salt + 1);
                // Le bruit se concentre autour de 0,5 : seuil approché pour couvrir `part` du sol.
                let threshold = 0.5 + (0.5 - part) * 0.55;
                (n > threshold).then_some(block)
            })
    }

    /// Une strate du sous-sol : elles sont horizontales, d'épaisseur 2 à 4, et ondulent
    /// doucement d'un endroit à l'autre.
    fn band(&self, biome: &BiomeDef, x: i32, y: i32, z: i32) -> BlockId {
        let wave = (value_noise(self.seed, x, z, 48, 44) * 6.0) as i32;
        let k = (y + wave).div_euclid(3);
        let pick = hash3(self.seed, 0, k, 0, 45) % biome.bands.len() as u64;
        biome.bands[pick as usize]
    }

    /// Les 32 × 32 colonnes d'un chunk, `[x][z]` : les cartes de bruit sur la grille de
    /// 9 × 9 points du chunk, puis interpolées pour chaque colonne. Elles viennent du cache des
    /// colonnes (voir [`Self::column_grid`]) : les six chunks d'une verticale les partagent.
    pub fn columns(&self, chunk_pos: IVec3) -> Box<[[Column; CHUNK_SIZE]; CHUNK_SIZE]> {
        if self.v3.is_some()
            && matches!(
                self.world,
                crate::worldgen::dimension::WorldKind::Terrain { .. }
            )
        {
            return self.v3_column(chunk_pos.x, chunk_pos.z).columns.clone();
        }
        Box::new(self.column_grid(chunk_pos.x, chunk_pos.z).columns)
    }

    /// Les colonnes de la colonne de chunks (cx, cz), depuis le cache si possible. Un monde plat ou
    /// vide n'a pas besoin du cache (ses colonnes sont constantes) : il les recalcule.
    fn column_grid(&self, cx: i32, cz: i32) -> Arc<ColumnGrid> {
        let chunk_pos = IVec3::new(cx, 0, cz);
        if !matches!(
            self.world,
            crate::worldgen::dimension::WorldKind::Terrain { .. }
        ) {
            return Arc::new(ColumnGrid::new(*self.compute_columns(chunk_pos)));
        }
        let key = ColumnKey {
            seed: self.seed,
            dimension: self.dimension_tag,
            version: self.version,
            cx,
            cz,
        };
        self.column_cache
            .get_or_compute(key, || ColumnGrid::new(*self.compute_columns(chunk_pos)))
    }

    /// Le calcul des colonnes d'un chunk, sans cache.
    fn compute_columns(&self, chunk_pos: IVec3) -> Box<[[Column; CHUNK_SIZE]; CHUNK_SIZE]> {
        let origin = World::chunk_origin(chunk_pos);
        if !matches!(
            self.world,
            crate::worldgen::dimension::WorldKind::Terrain { .. }
        ) {
            let mut columns = Box::new([[Column::default(); CHUNK_SIZE]; CHUNK_SIZE]);
            for (lx, row) in columns.iter_mut().enumerate() {
                for (lz, column) in row.iter_mut().enumerate() {
                    *column = self
                        .constant_column(origin.x + lx as i32, origin.z + lz as i32)
                        .unwrap_or_default();
                }
            }
            return columns;
        }
        let (g0x, g0z) = (origin.x / STEP, origin.z / STEP);
        let mut grid = [[Fields::default(); GRID]; GRID];
        for (i, row) in grid.iter_mut().enumerate() {
            for (j, f) in row.iter_mut().enumerate() {
                *f = self.fields(g0x + i as i32, g0z + j as i32);
            }
        }
        let lookup = |gx: i32, gz: i32| grid[(gx - g0x) as usize][(gz - g0z) as usize];
        let mut columns = Box::new([[Column::default(); CHUNK_SIZE]; CHUNK_SIZE]);
        for (lx, row) in columns.iter_mut().enumerate() {
            for (lz, column) in row.iter_mut().enumerate() {
                let fields = self.fields_at(origin.x + lx as i32, origin.z + lz as i32, lookup);
                *column = self.interpret(fields);
            }
        }
        columns
    }

    /// Le nombre maximal de colonnes gardées dans le cache des colonnes.
    pub const COLUMN_CACHE_LIMIT: usize = column_cache::COLUMN_CACHE_LIMIT;

    /// Le nombre de colonnes gardées dans le cache (pour les mesures et les tests).
    pub fn cached_columns(&self) -> usize {
        self.column_cache.len()
    }

    /// Les biomes des colonnes d'un chunk (x + 32 z), comme [`Chunk::set_biomes`] les attend.
    pub fn biome_columns(&self, chunk_pos: IVec3) -> Vec<u8> {
        if self.v3.is_some()
            && matches!(
                self.world,
                crate::worldgen::dimension::WorldKind::Terrain { .. }
            )
        {
            return self.v3_column(chunk_pos.x, chunk_pos.z).biomes.clone();
        }
        self.column_grid(chunk_pos.x, chunk_pos.z).biomes.clone()
    }

    /// Génère un chunk. Renvoie `None` s'il ne contient que de l'air
    /// (le ciel, ou hors de la hauteur du monde) : inutile de le stocker.
    pub fn generate(&self, chunk_pos: IVec3) -> Option<Chunk> {
        if let Some(constant) = self.generate_constant(chunk_pos) {
            return constant;
        }
        if self.v3.is_some() {
            return self.generate_v3(chunk_pos);
        }
        if !self.chunk_range().contains(&chunk_pos.y) {
            return None;
        }
        // Une plage imposée sous 0 (tests du monde haut) : la génération v1 n'y est pas définie ;
        // on y met de la pierre creusée d'une salle fermée par chunk.
        if chunk_pos.y < 0 {
            return Some(self.deep_test_chunk());
        }
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;

        // Les colonnes viennent du cache : les six chunks de la verticale les partagent.
        let grid = self.column_grid(chunk_pos.x, chunk_pos.z);
        let columns = &grid.columns;
        let highest = grid.max_height.max(SEA_LEVEL);
        // Les structures qui touchent ce chunk : un toit peut dépasser du terrain.
        let plans = self.structure_plans(chunk_pos);
        let highest = if plans.is_empty() {
            highest
        } else {
            highest + 40
        };
        if origin.y > highest {
            return None; // Tout le chunk est au-dessus du sol et de la mer.
        }

        // Les bruits de grottes ne servent qu'aux blocs sous le plafond des grottes : un chunk
        // entièrement au-dessus (le haut des reliefs, le ciel) ne les calcule pas.
        let caves = (origin.y <= grid.max_cave_ceiling).then(|| CaveGrid::new(self, origin));
        let mut chunk = Chunk::new();
        chunk.set_biomes(grid.biomes.clone());
        for (lx, row) in columns.iter().enumerate() {
            for (lz, &column) in row.iter().enumerate() {
                let (x, z) = (origin.x + lx as i32, origin.z + lz as i32);
                // Pas de grotte juste sous l'eau, ni juste sous la surface (voir `cave_ceiling`).
                let cave_ceiling = column_cache::cave_ceiling(&column);
                let frozen = column.temperature < FREEZING || column.climate_temperature < -0.55;
                for ly in 0..size {
                    let y = origin.y + ly;
                    let block = if y > column.height {
                        // Au-dessus du sol : de l'eau jusqu'au niveau de la mer, puis de l'air.
                        if y > SEA_LEVEL {
                            break;
                        }
                        if y == SEA_LEVEL && frozen {
                            self.blocks.ice
                        } else {
                            self.blocks.water
                        }
                    } else if y > 0
                        && y <= cave_ceiling
                        && let Some(fill) = caves
                            .as_ref()
                            .and_then(|caves| caves.carve(self, lx, ly as usize, lz, y, column))
                    {
                        match fill {
                            CaveFill::Air => continue,
                            CaveFill::Water => self.blocks.water,
                            CaveFill::Lava => self.blocks.lava,
                        }
                    } else {
                        self.layer(x, y, z, column)
                    };
                    chunk.set(lx, ly as usize, lz, block);
                }
            }
        }
        self.place_ores(&mut chunk, chunk_pos, columns);
        self.place_plants(&mut chunk, chunk_pos, columns);
        self.place_trees(&mut chunk, chunk_pos);
        self.place_structures(&mut chunk, chunk_pos, &plans);
        // Les grottes ont pu tout vider.
        (!chunk.is_empty()).then_some(chunk)
    }

    /// Les filons de minerai : des marches aléatoires dans la roche du chunk.
    ///
    /// Le hasard vient d'un générateur pseudo-aléatoire initialisé avec (graine du monde,
    /// position du chunk, numéro du minerai) : le même chunk reçoit toujours les mêmes filons.
    fn place_ores(
        &self,
        chunk: &mut Chunk,
        chunk_pos: IVec3,
        columns: &[[Column; CHUNK_SIZE]; CHUNK_SIZE],
    ) {
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;
        for (index, ore) in self.biomes.ores().iter().enumerate() {
            // Les altitudes du minerai dans cette génération (la v3 descend sous 0 ; certains
            // minerais n'existent qu'en v3).
            let Some(height) = ore.range(self.v3.is_some()) else {
                continue;
            };
            // La part de l'intervalle d'altitude du minerai qui tombe dans ce chunk.
            let low = height.0.max(origin.y);
            let high = height.1.min(origin.y + size - 1);
            if low > high {
                continue;
            }
            let mut rng = ChunkRng::new(self.seed, chunk_pos, index as u64 + 100);
            let expected = ore.per_chunk * (high - low + 1) as f32 / size as f32;
            let veins = expected.floor() as u32 + u32::from(rng.next_f32() < expected.fract());
            for _ in 0..veins {
                let mut p = IVec3::new(
                    rng.below(size as u32) as i32,
                    low - origin.y + rng.below((high - low + 1) as u32) as i32,
                    rng.below(size as u32) as i32,
                );
                if let Some(biomes) = &ore.biomes {
                    let biome = columns[p.x as usize][p.z as usize].biome;
                    if !biomes.contains(&biome) {
                        continue;
                    }
                }
                for _ in 0..ore.size {
                    let inside = p.cmpge(IVec3::ZERO).all() && p.cmplt(IVec3::splat(size)).all();
                    // Le filon ne sort pas de ses altitudes, même en serpentant.
                    if inside && (height.0..=height.1).contains(&(origin.y + p.y)) {
                        let (x, y, z) = (p.x as usize, p.y as usize, p.z as usize);
                        if ore.replace.contains(&chunk.get(x, y, z)) {
                            chunk.set(x, y, z, ore.block);
                        }
                    }
                    // Un pas dans une des 6 directions.
                    let axis = rng.below(6);
                    let sign = if axis.is_multiple_of(2) { 1 } else { -1 };
                    p[(axis / 2) as usize] += sign;
                }
            }
        }
    }

    /// Génère d'un coup une région de `chunks_x × chunks_z` colonnes de chunks
    /// (sur toute la hauteur du monde). Pratique pour les tests et les benchmarks.
    pub fn generate_region(&self, chunks_x: i32, chunks_z: i32) -> World {
        self.generate_area(IVec3::ZERO, chunks_x, chunks_z)
    }

    /// Comme [`TerrainGenerator::generate_region`], à partir de la colonne de chunks `from`.
    pub fn generate_area(&self, from: IVec3, chunks_x: i32, chunks_z: i32) -> World {
        let mut world = World::with_registry(self.registry.clone());
        world.set_biomes(self.biomes.clone());
        for x in 0..chunks_x {
            for z in 0..chunks_z {
                for y in self.chunk_range() {
                    let pos = IVec3::new(from.x + x, y, from.z + z);
                    if let Some(chunk) = self.generate(pos) {
                        world.insert(pos, chunk.into());
                    }
                }
            }
        }
        world
    }
}

/// La distance (au carré) entre un climat et celui d'un biome. La variété compte un peu
/// moins que la température et l'humidité : elle départage des biomes de climat voisin.
fn climate_distance(b: &BiomeDef, temperature: f64, humidity: f64, variety: f64) -> f64 {
    (temperature - b.climate.0).powi(2)
        + (humidity - b.climate.1).powi(2)
        + 0.6 * (variety - b.variety).powi(2)
}

/// Ce qui remplit une grotte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaveFill {
    Air,
    /// Une nappe souterraine.
    Water,
    /// Le fond du monde.
    Lava,
}

/// Les valeurs des quatre bruits 3D des grottes aux points de grille d'un chunk.
struct CaveBuffers {
    tunnel_a: [f32; GRID * GRID * GRID],
    tunnel_b: [f32; GRID * GRID * GRID],
    cheese: [f32; GRID * GRID * GRID],
    aquifer: [f32; GRID * GRID * GRID],
}

thread_local! {
    /// Les tampons de travail de ce thread. Un worker génère des centaines de chunks : il
    /// réemploie les mêmes (environ 12 Kio) au lieu d'en allouer quatre par chunk. Chaque thread a
    /// les siens, sans verrou ; ils ne gardent aucune valeur d'un chunk à l'autre (chaque case est
    /// réécrite avant d'être lue).
    static CAVE_BUFFERS: std::cell::RefCell<Option<Box<CaveBuffers>>> =
        const { std::cell::RefCell::new(None) };
}

/// Les bruits 3D des grottes, évalués tous les 4 blocs puis interpolés.
struct CaveGrid {
    /// Rendus au thread quand la grille est détruite.
    buffers: Option<Box<CaveBuffers>>,
}

impl Drop for CaveGrid {
    fn drop(&mut self) {
        if let Some(buffers) = self.buffers.take() {
            // `try_with` : le thread peut déjà être en train de se terminer.
            let _ = CAVE_BUFFERS.try_with(|slot| *slot.borrow_mut() = Some(buffers));
        }
    }
}

impl CaveGrid {
    fn new(generator: &TerrainGenerator, origin: IVec3) -> Self {
        let mut buffers = CAVE_BUFFERS
            .try_with(|slot| slot.borrow_mut().take())
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                Box::new(CaveBuffers {
                    tunnel_a: [0.0; GRID * GRID * GRID],
                    tunnel_b: [0.0; GRID * GRID * GRID],
                    cheese: [0.0; GRID * GRID * GRID],
                    aquifer: [0.0; GRID * GRID * GRID],
                })
            });
        let mut n = 0;
        for i in 0..GRID {
            for j in 0..GRID {
                for k in 0..GRID {
                    let p = origin + IVec3::new(i as i32, j as i32, k as i32) * STEP;
                    let (x, y, z) = (p.x as f64, p.y as f64, p.z as f64);
                    // On étire les tunnels horizontalement : des galeries plutôt horizontales.
                    let tunnel = [x / 48.0, y / 28.0, z / 48.0];
                    buffers.tunnel_a[n] = generator.cave_a.get(tunnel) as f32;
                    buffers.tunnel_b[n] = generator.cave_b.get(tunnel) as f32;
                    buffers.cheese[n] = generator.cheese.get([x / 90.0, y / 45.0, z / 90.0]) as f32;
                    buffers.aquifer[n] =
                        generator.aquifer.get([x / 96.0, y / 40.0, z / 96.0]) as f32;
                    n += 1;
                }
            }
        }
        Self {
            buffers: Some(buffers),
        }
    }

    fn buffers(&self) -> &CaveBuffers {
        self.buffers
            .as_ref()
            .expect("tampons présents jusqu'au Drop")
    }

    /// Interpolation trilinéaire d'un des bruits au bloc local (lx, ly, lz).
    fn sample(values: &[f32], lx: usize, ly: usize, lz: usize) -> f32 {
        let s = STEP as usize;
        let (i, j, k) = (lx / s, ly / s, lz / s);
        let (fx, fy, fz) = (
            (lx % s) as f32 / s as f32,
            (ly % s) as f32 / s as f32,
            (lz % s) as f32 / s as f32,
        );
        let at = |di: usize, dj: usize, dk: usize| {
            values[((i + di) * GRID + (j + dj)) * GRID + (k + dk)]
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(at(0, 0, 0), at(1, 0, 0), fx);
        let x10 = lerp(at(0, 1, 0), at(1, 1, 0), fx);
        let x01 = lerp(at(0, 0, 1), at(1, 0, 1), fx);
        let x11 = lerp(at(0, 1, 1), at(1, 1, 1), fx);
        lerp(lerp(x00, x10, fy), lerp(x01, x11, fy), fz)
    }

    /// `Some` si le bloc est dans une grotte, avec ce qui la remplit.
    fn carve(
        &self,
        _: &TerrainGenerator,
        lx: usize,
        ly: usize,
        lz: usize,
        y: i32,
        column: Column,
    ) -> Option<CaveFill> {
        // Tunnels « spaghetti » : chaque bruit vaut zéro sur une surface tordue ;
        // là où deux de ces surfaces se croisent, on obtient un tunnel.
        let buffers = self.buffers();
        let a = Self::sample(&buffers.tunnel_a, lx, ly, lz);
        let b = Self::sample(&buffers.tunnel_b, lx, ly, lz);
        let tunnel = a * a + b * b < 0.006;
        // Cavernes « fromage » : de grands vides là où le bruit est haut. Plus rares
        // près de la surface, pour que le sol ne s'effondre pas.
        let near_surface = (y - (column.height - 24)).max(0) as f32;
        let cheese =
            !tunnel && Self::sample(&buffers.cheese, lx, ly, lz) > 0.42 + near_surface * 0.02;
        if !tunnel && !cheese {
            return None;
        }
        Some(if y <= LAVA_LEVEL {
            CaveFill::Lava
        } else if y < SEA_LEVEL - 12 && Self::sample(&buffers.aquifer, lx, ly, lz) > 0.35 {
            CaveFill::Water
        } else {
            CaveFill::Air
        })
    }
}

/// Un générateur pseudo-aléatoire (splitmix64) pour un chunk : rapide, et le même
/// résultat pour les mêmes (graine, chunk, sel).
struct ChunkRng(u64);

impl ChunkRng {
    fn new(seed: u32, chunk: IVec3, salt: u64) -> Self {
        Self(hash3(seed, chunk.x, chunk.y, chunk.z, salt))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Un entier dans `0..n`.
    fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }
}

/// Un nombre pseudo-aléatoire pour une position : le même à chaque fois.
///
/// Chaque coordonnée est mélangée à fond (le « finaliseur » de splitmix64) avant la
/// suivante : avec un mélange trop léger, deux colonnes voisines en z recevaient des nombres
/// proches, et l'herbe et les fleurs poussaient en bandes droites.
pub(crate) fn hash3(seed: u32, x: i32, y: i32, z: i32, salt: u64) -> u64 {
    let mut h = (seed as u64) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for v in [x, y, z] {
        h = mix64(h ^ (v as u32 as u64));
    }
    h
}

/// Le finaliseur de splitmix64 : chaque bit d'entrée change la moitié des bits de sortie.
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Un bruit de valeurs en 2D, entre 0 et 1 : un nombre tiré au hasard à chaque nœud d'une
/// grille de `cell` blocs, et une interpolation douce entre les nœuds. Bien moins cher qu'un
/// Perlin, assez bon pour moduler la végétation (bosquets, taches de fleurs).
pub(crate) fn value_noise(seed: u32, x: i32, z: i32, cell: i32, salt: u64) -> f32 {
    let (gx, gz) = (x.div_euclid(cell), z.div_euclid(cell));
    let fx = x.rem_euclid(cell) as f32 / cell as f32;
    let fz = z.rem_euclid(cell) as f32 / cell as f32;
    let node = |i: i32, j: i32| (hash3(seed, i, 0, j, salt) >> 40) as f32 / (1u64 << 24) as f32;
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sz) = (s(fx), s(fz));
    let top = node(gx, gz) + (node(gx + 1, gz) - node(gx, gz)) * sx;
    let bottom = node(gx, gz + 1) + (node(gx + 1, gz + 1) - node(gx, gz + 1)) * sx;
    top + (bottom - top) * sz
}

/// 0 avant `edge0`, 1 après `edge1`, une courbe en S entre les deux. Marche aussi
/// avec `edge0 > edge1` (la courbe est alors descendante).
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Une courbe linéaire par morceaux passant par `points` (x croissants).
fn spline(x: f64, points: &[(f64, f64)]) -> f64 {
    let first = points[0];
    if x <= first.0 {
        return first.1;
    }
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1].1
}

mod column_cache;
mod features;
mod structures;
#[cfg(test)]
mod tests;
mod v3;
mod v3_rules;
pub use v3::V3Parameters;

use column_cache::{ColumnCache, ColumnGrid, ColumnKey};
use features::PlantBlocks;
pub use features::TreeSite;
use structures::StructureBlocks;
pub use structures::{
    Blueprint, ChestLoot, ChestLootEntry, PENDING_LOOT, Spot, StructureDef, StructureKind,
    StructureSet, StructureSite, pending_loot,
};

impl TerrainGenerator {
    /// Ce générateur, réglé sur une version de génération. `V1` ne change rien ; `V3` branche la
    /// densité 3D (voir `generator/v3.rs`). Les clés des caches contiennent la version : deux
    /// versions ne se voient jamais leurs valeurs.
    pub fn with_version(mut self, version: GenerationVersion) -> Result<Self> {
        self.version = version;
        self.v3 = match version {
            GenerationVersion::V3 => Some(Arc::new(v3::V3State::new(
                self.seed,
                &self.registry,
                &self.biomes,
            )?)),
            GenerationVersion::V1 => None,
        };
        // Les grands arbres : à l'échelle du relief v3 ; la v1 garde les siens.
        if let Some(blocks) = &mut self.tree_blocks {
            blocks.tall = version == GenerationVersion::V3;
        }
        // Les arbres et les colonnes dépendent de la version : on repart de caches neufs.
        self.tree_cache = Arc::default();
        self.site_cache = Arc::default();
        self.column_cache = Arc::default();
        Ok(self)
    }

    /// Comme [`Self::with_dimension`], avec une version de génération (celle de `world.ron`).
    pub fn with_dimension_versioned(
        seed: u32,
        registry: Arc<BlockRegistry>,
        mods: &[(String, std::path::PathBuf)],
        dimension: &crate::worldgen::dimension::DimensionDef,
        version: GenerationVersion,
    ) -> Result<Self> {
        Self::with_dimension(seed, registry, mods, dimension)?.with_version(version)
    }

    /// Le bloc du sol de la colonne (celui où l'on pose un arbre ou une plante).
    pub(super) fn ground_block(&self, x: i32, z: i32, column: Column) -> BlockId {
        self.layer(x, column.height, z, column)
    }
}
