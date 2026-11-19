//! La végétation du monde : plantes, algues, nénuphars, champignons, et les arbres.
//!
//! # Les arbres et les bordures de chunk
//!
//! Chaque chunk est généré seul, sur n'importe quel thread, dans n'importe quel ordre.
//! Or un arbre planté près du bord déborde sur le chunk voisin. Solution : les
//! emplacements d'arbres d'une colonne de chunks ne dépendent que de la graine et de la
//! position de cette colonne (un tirage pseudo-aléatoire). Pour générer un chunk, on
//! recalcule les arbres des 9 colonnes autour (la sienne et ses 8 voisines) et on n'écrit
//! que les blocs qui tombent chez lui. Le voisin, de son côté, fera le même calcul et
//! écrira l'autre moitié de l'arbre : aucune coupure, sans jamais se parler.
//!
//! Pour que deux arbres qui se chevauchent donnent le même résultat des deux côtés de la
//! frontière, on les parcourt toujours dans le même ordre (par colonne de chunks
//! croissante, puis dans l'ordre du tirage), et les règles d'écriture ne dépendent que
//! du chunk lui-même : un tronc remplace l'air, les feuilles et les plantes ; des feuilles
//! ne remplacent que l'air.
//!
//! # Bosquets, clairières et taches de fleurs
//!
//! Tirer chaque arbre et chaque fleur au hasard, indépendamment, donne une végétation
//! uniforme, « semée à la machine ». Dans la nature, les arbres se groupent en bosquets
//! séparés de clairières, et les fleurs poussent en taches. On module donc la probabilité
//! par un bruit lent ([`value_noise`]) : forte dans les bosquets et les taches, faible
//! ailleurs, avec la même moyenne. Les arbres gardent en plus un écart minimal entre eux
//! (comme un échantillonnage « de Poisson ») : pas deux troncs collés.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::IVec3;

use super::{Column, SEA_LEVEL, TerrainGenerator, hash3, value_noise};
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::Shape;
use crate::trees::{self, Part, Rng64, TreeKind};
use crate::world::World;
use crate::{FREEZING, LAVA_LEVEL};

/// Emplacements tirés par colonne de chunks ; un emplacement devient un arbre avec la
/// probabilité `densité × bosquet / CANDIDATES` (le facteur de bosquet monte à 2,2) : la
/// densité d'un biome, multipliée par son facteur de bosquet maximal, reste donc sous 24.
const CANDIDATES: u32 = 24;
/// Le facteur de bosquet maximal (au cœur d'un bosquet).
const MAX_GROVE: f32 = 1.0 / 0.45;
/// Les arbres s'arrêtent à cette distance (en blocs) du centre d'un village.
const VILLAGE_CLEARING: i32 = 40;

/// Un arbre à planter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeSite {
    pub kind: TreeKind,
    /// Le bas du tronc (le bloc au-dessus du sol).
    pub base: IVec3,
    pub seed: u64,
}

/// Un nombre entre 0 et 1 pour une colonne (x, z) et un sel.
fn roll(seed: u32, x: i32, z: i32, salt: u64) -> f32 {
    (hash3(seed, x, 0, z, salt) >> 40) as f32 / (1u64 << 24) as f32
}

/// Le facteur de bosquet en (x, z) : de 0 (clairière) à 2,2 (cœur d'un bosquet), 1 en
/// moyenne. `groves` (0 à 1) dose l'effet : à 0, le facteur vaut 1 partout.
fn grove_factor(seed: u32, x: i32, z: i32, groves: f32) -> f32 {
    let n = 0.7 * value_noise(seed, x, z, 28, 501) + 0.3 * value_noise(seed, x, z, 9, 502);
    // La moyenne de ce smoothstep sur le bruit est d'environ 0,45 : on la ramène à 1.
    let grove = smoothstep(0.35, 0.7, n) * MAX_GROVE;
    1.0 - groves + groves * grove
}

/// Le facteur de tache d'une plante en (x, z) : chaque plante a ses propres taches (le sel
/// vient de son bloc). 1 en moyenne ; `patches` (0 à 1) dose l'effet.
fn patch_factor(seed: u32, x: i32, z: i32, plant: BlockId, patches: f32) -> f32 {
    if patches <= 0.0 {
        return 1.0;
    }
    let salt = 610 + plant.0 as u64 * 2;
    let n = 0.65 * value_noise(seed, x, z, 10, salt) + 0.35 * value_noise(seed, x, z, 4, salt + 1);
    // Moyenne d'environ 0,2 : les taches sont 5 fois plus denses que la moyenne.
    let patch = smoothstep(0.55, 0.75, n) / 0.2;
    1.0 - patches + patches * patch
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// L'écart minimal entre deux troncs, selon la forme : les grands arbres se gênent plus.
fn spacing(kind: TreeKind) -> i32 {
    match kind {
        TreeKind::Cactus | TreeKind::Bush | TreeKind::JungleBush => 2,
        TreeKind::Oak
        | TreeKind::Birch
        | TreeKind::Spruce
        | TreeKind::TallSpruce
        | TreeKind::Fir
        | TreeKind::Boulder => 3,
        TreeKind::Fruit | TreeKind::Cypress | TreeKind::Olive => 4,
        TreeKind::Swamp | TreeKind::Acacia | TreeKind::Maple => 5,
        TreeKind::BigOak | TreeKind::Jungle | TreeKind::Redwood => 6,
    }
}

/// Les emplacements d'arbres déjà calculés, par colonne de chunks.
///
/// Chaque chunk a besoin des arbres des 9 colonnes autour de lui, et une colonne compte 6
/// chunks en hauteur : sans mémoire, les arbres d'une colonne seraient tirés 54 fois. Le
/// résultat ne dépend que de la graine et de la colonne : le garder ne change rien au monde.
/// Le verrou n'est tenu que le temps d'une recherche ou d'une insertion.
#[derive(Debug, Default)]
pub(super) struct TreeCache(Mutex<HashMap<(i32, i32), TreeSites>>);

/// Les arbres d'une colonne de chunks, partagés sans copie.
pub type TreeSites = Arc<[TreeSite]>;

/// Au-delà, on vide le cache (un monde parcouru en garde surtout les colonnes récentes).
const TREE_CACHE_LIMIT: usize = 8192;

impl TerrainGenerator {
    /// Vide les caches de calcul (arbres, colonnes, sites) : le générateur repart à froid. Le monde
    /// généré ne change pas, seulement le temps du prochain calcul. Utile aux mesures à froid et
    /// aux tests qui comparent un cache vide et un cache plein.
    pub fn clear_caches(&self) {
        self.tree_cache
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.site_cache.clear();
        self.column_cache.clear();
    }

    /// Les arbres plantés dans la colonne de chunks (cx, cz), depuis le cache si possible.
    pub fn tree_sites(&self, cx: i32, cz: i32) -> TreeSites {
        let lock = || self.tree_cache.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sites) = lock().get(&(cx, cz)) {
            return sites.clone();
        }
        let sites: TreeSites = self.compute_tree_sites(cx, cz).into();
        let mut cache = lock();
        if cache.len() >= TREE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert((cx, cz), sites.clone());
        sites
    }

    /// Le tirage des arbres d'une colonne de chunks.
    fn compute_tree_sites(&self, cx: i32, cz: i32) -> Vec<TreeSite> {
        if self.tree_blocks.is_none() {
            return Vec::new();
        }
        let mut rng = Rng64::new(hash3(self.seed, cx, 0, cz, 500));
        let size = CHUNK_SIZE as u32;
        let mut sites: Vec<TreeSite> = Vec::new();
        // Pas d'arbres dans un village : on les cherche une fois pour la colonne de chunks.
        let villages = self.village_clearings(cx, cz);
        // La plus forte chance d'arbre, tous biomes confondus : un tirage au-dessus est rejeté
        // sans évaluer la colonne (le plus cher).
        let best = self
            .biomes
            .iter()
            .filter(|(_, b)| !b.trees.is_empty())
            .map(|(_, b)| b.tree_density * (1.0 - b.groves + b.groves * MAX_GROVE))
            .fold(0.0, f32::max)
            / CANDIDATES as f32;
        for _ in 0..CANDIDATES {
            // Tous les tirages d'abord, quoi qu'il arrive : la suite reste la même pour
            // chaque emplacement, que les précédents soient retenus ou non.
            let x = cx * CHUNK_SIZE as i32 + rng.below(size) as i32;
            let z = cz * CHUNK_SIZE as i32 + rng.below(size) as i32;
            let accept = rng.next_f32();
            let pick = rng.next_f32();
            let seed = rng.next_u64();

            if accept >= best {
                continue;
            }
            if villages.iter().any(|&(vx, vz)| {
                (x - vx).abs() <= VILLAGE_CLEARING && (z - vz).abs() <= VILLAGE_CLEARING
            }) {
                continue;
            }
            let column = self.column(x, z);
            let biome = self.biomes.get(column.biome);
            if biome.trees.is_empty() {
                continue;
            }
            let grove = grove_factor(self.seed, x, z, biome.groves);
            if accept >= biome.tree_density * grove / CANDIDATES as f32 {
                continue;
            }
            if column.height < SEA_LEVEL {
                continue;
            }
            // La forme, tirée selon les poids du biome.
            let total: f32 = biome.trees.iter().map(|(_, w)| w).sum();
            let mut target = pick * total;
            let kind = biome
                .trees
                .iter()
                .find(|(_, w)| {
                    target -= w;
                    target < 0.0
                })
                .map_or(biome.trees[0].0, |(k, _)| *k);
            // Pas trop près d'un arbre déjà planté dans cette colonne de chunks.
            let crowded = sites.iter().any(|s| {
                let gap = spacing(kind).max(spacing(s.kind));
                let (dx, dz) = (s.base.x - x, s.base.z - z);
                dx * dx + dz * dz < gap * gap
            });
            if crowded {
                continue;
            }
            // Le sol doit convenir : du sable pour le cactus, n'importe quel sol plein pour un
            // rocher, de la terre ou de l'herbe (éventuellement enneigée) pour les autres.
            let ground = self.ground_block(x, z, column);
            let ok = match kind {
                TreeKind::Cactus => {
                    ground == self.blocks.sand || ground == self.plant_blocks.red_sand
                }
                TreeKind::Boulder => {
                    let def = self.registry.def(ground);
                    def.solid && def.shape == Shape::Cube && !def.gravity
                }
                _ => self.plant_blocks.is_soil(ground) || ground == self.blocks.snow,
            };
            if ok {
                sites.push(TreeSite {
                    kind,
                    base: IVec3::new(x, column.height + 1, z),
                    seed,
                });
            }
        }
        sites
    }

    /// Plante les arbres des 9 colonnes autour, pour la partie qui tombe dans ce chunk.
    pub(super) fn place_trees(&self, chunk: &mut Chunk, chunk_pos: IVec3) {
        let Some(blocks) = &self.tree_blocks else {
            return;
        };
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;
        // Un arbre fait au plus 30 blocs de haut.
        for cx in chunk_pos.x - 1..=chunk_pos.x + 1 {
            for cz in chunk_pos.z - 1..=chunk_pos.z + 1 {
                for &site in self.tree_sites(cx, cz).iter() {
                    let near = |a: i32, o: i32| {
                        a + trees::MAX_RADIUS >= o && a - trees::MAX_RADIUS < o + size
                    };
                    if !near(site.base.x, origin.x)
                        || !near(site.base.z, origin.z)
                        || site.base.y > origin.y + size
                        || site.base.y + 32 < origin.y
                    {
                        continue;
                    }
                    let mut rng = Rng64::new(site.seed);
                    trees::grow(
                        site.kind,
                        site.base,
                        &mut rng,
                        blocks,
                        &mut |p, block, part| {
                            let local = p - origin;
                            if local.cmplt(IVec3::ZERO).any()
                                || local.cmpge(IVec3::splat(size)).any()
                            {
                                return;
                            }
                            let (x, y, z) = (local.x as usize, local.y as usize, local.z as usize);
                            let existing = chunk.get(x, y, z);
                            let replace = match part {
                                Part::Leaves => existing.is_air(),
                                Part::Log => {
                                    let def = self.registry.def(existing);
                                    existing.is_air() || def.shape != Shape::Cube || def.cutout
                                }
                            };
                            if replace {
                                chunk.set(x, y, z, block);
                            }
                        },
                    );
                }
            }
        }
    }

    /// Les petites plantes de ce chunk : herbes, fleurs, algues, nénuphars, et les
    /// champignons des grottes. Une plante ne déborde jamais : pas besoin des voisins.
    pub(super) fn place_plants(
        &self,
        chunk: &mut Chunk,
        chunk_pos: IVec3,
        columns: &[[Column; CHUNK_SIZE]; CHUNK_SIZE],
    ) {
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;
        let b = self.plant_blocks;
        let inside = |y: i32| (origin.y..origin.y + size).contains(&y);
        for (lx, row) in columns.iter().enumerate() {
            for (lz, &column) in row.iter().enumerate() {
                let (x, z) = (origin.x + lx as i32, origin.z + lz as i32);
                let biome = self.biomes.get(column.biome);
                let r = roll(self.seed, x, z, 600);
                // Sous l'eau : la mer.
                let (wet, water_top) = (column.height < SEA_LEVEL, SEA_LEVEL);
                if !wet {
                    // Sur la terre : au plus une plante par colonne. Chaque plante du biome a
                    // son propre tirage, modulé par ses taches ; les plus rares passent
                    // d'abord (le registre les trie), pour que l'herbe ne les étouffe pas.
                    let y = column.height + 1;
                    if !inside(y) || biome.plants.is_empty() {
                        continue;
                    }
                    let ground = self.ground_block(x, z, column);
                    for (i, plant) in biome.plants.iter().enumerate() {
                        let chance = plant.chance
                            * patch_factor(self.seed, x, z, plant.block, plant.patches);
                        if roll(self.seed, x, z, 620 + i as u64) >= chance {
                            continue;
                        }
                        let ok = if plant.block == b.dead_bush {
                            ground == self.blocks.sand
                                || ground == b.coarse_dirt
                                || self.registry.def(ground).gravity
                        } else {
                            // Les biomes de la v3 fleurissent aussi sur leur propre sol (les fleurs du
                            // désert fleuri sur le sable) ; la v1 ne change pas.
                            b.is_soil(ground) || (biome.v3_only && ground == biome.surface)
                        };
                        let ly = (y - origin.y) as usize;
                        if ok && chunk.get(lx, ly, lz).is_air() {
                            chunk.set(lx, ly, lz, plant.block);
                        }
                        break;
                    }
                } else {
                    let depth = water_top - column.height;
                    let frozen =
                        column.temperature < FREEZING || column.climate_temperature < -0.55;
                    // Des algues au fond, jamais jusqu'à la surface.
                    if depth >= 3 && r < biome.seaweed {
                        let length =
                            1 + (hash3(self.seed, x, 1, z, 601) % (depth as u64 - 2)) as i32;
                        for k in 1..=length {
                            let y = column.height + k;
                            if inside(y) {
                                chunk.set(lx, (y - origin.y) as usize, lz, b.seaweed);
                            }
                        }
                    }
                    // Des nénuphars sur l'eau calme et peu profonde.
                    let y = water_top + 1;
                    if depth <= 3
                        && !frozen
                        && roll(self.seed, x, z, 602) < biome.lily_pads
                        && inside(y)
                    {
                        chunk.set(lx, (y - origin.y) as usize, lz, b.lily_pad);
                    }
                }
            }
        }
        self.place_cave_mushrooms(chunk, origin);
    }

    /// Des champignons sur le sol des grottes, à l'ombre.
    fn place_cave_mushrooms(&self, chunk: &mut Chunk, origin: IVec3) {
        let low = (LAVA_LEVEL + 2).max(origin.y + 1);
        let high = (SEA_LEVEL - 10).min(origin.y + CHUNK_SIZE as i32 - 1);
        let b = self.plant_blocks;
        for y in low..=high {
            let ly = (y - origin.y) as usize;
            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    if !chunk.get(lx, ly, lz).is_air() {
                        continue;
                    }
                    let below = chunk.get(lx, ly - 1, lz);
                    if below != self.blocks.stone && below != self.blocks.deepstone {
                        continue;
                    }
                    let h = hash3(
                        self.seed,
                        origin.x + lx as i32,
                        y,
                        origin.z + lz as i32,
                        603,
                    );
                    if h.is_multiple_of(400) {
                        let mushroom = if h.is_multiple_of(3) {
                            b.red_mushroom
                        } else {
                            b.brown_mushroom
                        };
                        chunk.set(lx, ly, lz, mushroom);
                    }
                }
            }
        }
    }
}

/// Les blocs de la végétation, retrouvés par leur nom.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlantBlocks {
    pub grass: BlockId,
    pub dirt: BlockId,
    pub podzol: BlockId,
    pub coarse_dirt: BlockId,
    pub red_sand: BlockId,
    pub dead_bush: BlockId,
    pub seaweed: BlockId,
    pub lily_pad: BlockId,
    pub red_mushroom: BlockId,
    pub brown_mushroom: BlockId,
}

impl PlantBlocks {
    /// Un sol où poussent les plantes et les arbres : herbe, terre, podzol, terre grossière.
    pub fn is_soil(&self, block: BlockId) -> bool {
        [self.grass, self.dirt, self.podzol, self.coarse_dirt].contains(&block)
    }

    pub fn resolve(registry: &crate::registry::BlockRegistry) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        let id = |key: &str| {
            registry
                .id(key)
                .with_context(|| format!("la végétation a besoin du bloc « {key} »"))
        };
        Ok(Self {
            grass: id("core:grass")?,
            dirt: id("core:dirt")?,
            podzol: id("core:podzol")?,
            coarse_dirt: id("core:coarse_dirt")?,
            red_sand: id("core:red_sand")?,
            dead_bush: id("core:dead_bush")?,
            seaweed: id("core:seaweed")?,
            lily_pad: id("core:lily_pad")?,
            red_mushroom: id("core:red_mushroom")?,
            brown_mushroom: id("core:brown_mushroom")?,
        })
    }
}
