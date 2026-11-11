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

use glam::IVec3;

use super::{Column, SEA_LEVEL, TerrainGenerator, hash3};
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::Shape;
use crate::trees::{self, Part, Rng64, TreeKind};
use crate::world::World;
use crate::{FREEZING, LAVA_LEVEL};

/// Emplacements tirés par colonne de chunks ; un emplacement devient un arbre avec la
/// probabilité `densité / CANDIDATES`. La densité d'un biome reste donc sous 16.
const CANDIDATES: u32 = 16;
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

impl TerrainGenerator {
    /// Les arbres plantés dans la colonne de chunks (cx, cz).
    pub fn tree_sites(&self, cx: i32, cz: i32) -> Vec<TreeSite> {
        if self.tree_blocks.is_none() {
            return Vec::new();
        }
        let mut rng = Rng64::new(hash3(self.seed, cx, 0, cz, 500));
        let size = CHUNK_SIZE as u32;
        let mut sites = Vec::new();
        // Pas d'arbres dans un village : on les cherche une fois pour la colonne de chunks.
        let villages = self.village_clearings(cx, cz);
        for _ in 0..CANDIDATES {
            // Tous les tirages d'abord, quoi qu'il arrive : la suite reste la même pour
            // chaque emplacement, que les précédents soient retenus ou non.
            let x = cx * CHUNK_SIZE as i32 + rng.below(size) as i32;
            let z = cz * CHUNK_SIZE as i32 + rng.below(size) as i32;
            let accept = rng.next_f32();
            let pick = rng.next_f32();
            let seed = rng.next_u64();

            if villages.iter().any(|&(vx, vz)| {
                (x - vx).abs() <= VILLAGE_CLEARING && (z - vz).abs() <= VILLAGE_CLEARING
            }) {
                continue;
            }
            let column = self.column(x, z);
            let biome = self.biomes.get(column.biome);
            if biome.trees.is_empty() || accept >= biome.tree_density / CANDIDATES as f32 {
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
            // Le sol doit convenir : du sable pour le cactus, de la terre ou de l'herbe
            // (éventuellement enneigée) pour les autres.
            let ground = self.layer(x, column.height, z, column);
            let ok = if kind == TreeKind::Cactus {
                ground == self.blocks.sand
            } else {
                ground == self.plant_blocks.grass
                    || ground == self.plant_blocks.dirt
                    || ground == self.blocks.snow
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
                for site in self.tree_sites(cx, cz) {
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
                if column.height >= SEA_LEVEL {
                    // Sur la terre : au plus une plante par colonne, tirée parmi celles du biome.
                    let y = column.height + 1;
                    if !inside(y) || biome.plants.is_empty() {
                        continue;
                    }
                    let ground = self.layer(x, column.height, z, column);
                    let mut target = r;
                    for &(plant, p) in &biome.plants {
                        target -= p;
                        if target < 0.0 {
                            let ok = if plant == b.dead_bush {
                                ground == self.blocks.sand
                            } else {
                                ground == b.grass || ground == b.dirt
                            };
                            let ly = (y - origin.y) as usize;
                            if ok && chunk.get(lx, ly, lz).is_air() {
                                chunk.set(lx, ly, lz, plant);
                            }
                            break;
                        }
                    }
                } else {
                    let depth = SEA_LEVEL - column.height;
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
                    let y = SEA_LEVEL + 1;
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
    pub dead_bush: BlockId,
    pub seaweed: BlockId,
    pub lily_pad: BlockId,
    pub red_mushroom: BlockId,
    pub brown_mushroom: BlockId,
}

impl PlantBlocks {
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
            dead_bush: id("core:dead_bush")?,
            seaweed: id("core:seaweed")?,
            lily_pad: id("core:lily_pad")?,
            red_mushroom: id("core:red_mushroom")?,
            brown_mushroom: id("core:brown_mushroom")?,
        })
    }
}
