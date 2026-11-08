//! Génération procédurale du terrain avec du bruit de Perlin.
//!
//! Le bruit de Perlin donne, pour chaque point, une valeur entre -1 et 1 qui varie
//! doucement : deux points proches ont des valeurs proches. En superposant plusieurs
//! « octaves » (de plus en plus fines et de moins en moins fortes), on obtient un
//! relief naturel : c'est le fBm (« fractional Brownian motion »).
//!
//! Tout part d'une graine (`seed`) : la même graine donne toujours le même monde,
//! sur toutes les machines. On peut donc générer n'importe quel chunk, n'importe
//! quand, dans n'importe quel ordre et sur n'importe quel thread.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use glam::IVec3;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;
use crate::world::World;

/// Hauteur utile du monde, en chunks : les blocs vont de y = 0 à y = 32 × 5 - 1.
pub const WORLD_HEIGHT_CHUNKS: i32 = 5;
const MAX_HEIGHT: i32 = WORLD_HEIGHT_CHUNKS * CHUNK_SIZE as i32 - 2;
/// Au-dessus de cette altitude, le sommet des colonnes est enneigé.
const SNOW_LINE: i32 = 100;
/// Niveau de la mer : les creux du terrain sous cette altitude se remplissent d'eau.
/// À 38, environ 16 % des colonnes sont sous l'eau (mesuré sur 6 000 × 6 000 blocs).
pub const SEA_LEVEL: i32 = 38;

/// Le type de paysage d'une colonne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Plains,
    Desert,
    Mountains,
}

/// Ce que le générateur sait d'une colonne (x, z) : sa hauteur et son biome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    /// y du bloc le plus haut de la colonne.
    pub height: i32,
    pub biome: Biome,
}

/// Les blocs dont le générateur a besoin, retrouvés par leur nom au démarrage.
#[derive(Debug, Clone, Copy)]
struct TerrainBlocks {
    grass: BlockId,
    dirt: BlockId,
    stone: BlockId,
    sand: BlockId,
    snow: BlockId,
    water: BlockId,
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
            grass: id("core:grass")?,
            dirt: id("core:dirt")?,
            stone: id("core:stone")?,
            sand: id("core:sand")?,
            snow: id("core:snow")?,
            water: id("core:water")?,
        })
    }
}

/// Le générateur de terrain. Il ne contient que des tables de bruit en lecture seule :
/// il est donc `Send + Sync` et peut être partagé entre threads avec un `Arc`
/// (voir `streaming.rs`), sans verrou.
#[derive(Debug, Clone)]
pub struct TerrainGenerator {
    seed: u32,
    /// Relief de base : plaines et collines.
    hills: Fbm<Perlin>,
    /// Où sont les montagnes (valeur haute = montagne).
    mountain_mask: Fbm<Perlin>,
    /// Les crêtes des montagnes.
    ridges: Fbm<Perlin>,
    /// Chaud = désert, sinon plaine.
    temperature: Fbm<Perlin>,
    /// Deux champs 3D : une grotte passe là où les deux sont proches de zéro.
    cave_a: Perlin,
    cave_b: Perlin,
    registry: Arc<BlockRegistry>,
    blocks: TerrainBlocks,
}

impl TerrainGenerator {
    /// Un générateur avec les blocs de base intégrés.
    pub fn new(seed: u32) -> Self {
        Self::with_registry(seed, BlockRegistry::builtin())
            .expect("les blocs de base intégrés sont complets")
    }

    /// Un générateur qui utilise ce registre (core + mods). Échoue s'il manque
    /// un des blocs de base dont le terrain a besoin.
    pub fn with_registry(seed: u32, registry: Arc<BlockRegistry>) -> Result<Self> {
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
        Ok(Self {
            seed,
            hills: fbm(0, 5, 1.0 / 300.0),
            mountain_mask: fbm(1, 3, 1.0 / 700.0),
            ridges: fbm(2, 4, 1.0 / 180.0),
            temperature: fbm(3, 2, 1.0 / 900.0),
            cave_a: Perlin::new(seed.wrapping_add(4)),
            cave_b: Perlin::new(seed.wrapping_add(5)),
            registry,
            blocks,
        })
    }

    pub fn registry(&self) -> &Arc<BlockRegistry> {
        &self.registry
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Hauteur et biome d'une colonne.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let p = [x as f64, z as f64];

        // 1. Collines : entre environ 26 et 54 blocs.
        let mut height = 40.0 + 14.0 * self.hills.get(p);

        // 2. Montagnes : une crête (1 - |bruit|, pointue au milieu) n'apparaît que là
        //    où le masque est haut. `smoothstep` fait une transition douce de 0 à 1.
        let mask = smoothstep(0.1, 0.45, self.mountain_mask.get(p));
        let ridge = 1.0 - self.ridges.get(p).abs();
        height += mask * 90.0 * ridge * ridge;

        let height = (height.round() as i32).clamp(1, MAX_HEIGHT);
        let biome = if mask > 0.5 {
            Biome::Mountains
        } else if self.temperature.get(p) > 0.15 {
            Biome::Desert
        } else {
            Biome::Plains
        };
        Column { height, biome }
    }

    /// Vrai s'il y a une grotte en (x, y, z).
    ///
    /// Grottes « spaghetti » : chaque bruit 3D vaut zéro sur une surface tordue ;
    /// là où deux de ces surfaces se croisent, on obtient un tunnel.
    fn is_cave(&self, x: i32, y: i32, z: i32) -> bool {
        if y < 2 {
            return false; // Un fond solide sous le monde.
        }
        // On étire le bruit horizontalement : tunnels plutôt horizontaux.
        let p = [x as f64 / 48.0, y as f64 / 28.0, z as f64 / 48.0];
        let a = self.cave_a.get(p);
        let b = self.cave_b.get(p);
        a * a + b * b < 0.006
    }

    /// Le bloc d'une colonne à l'altitude `y` (sans les grottes).
    fn layer(&self, column: Column, y: i32) -> BlockId {
        let b = self.blocks;
        let depth = column.height - y;
        match column.biome {
            _ if column.height >= SNOW_LINE && depth == 0 => b.snow,
            Biome::Desert if depth < 4 => b.sand,
            // Plages et fonds de lacs : du sable au bord et sous l'eau.
            _ if column.height <= SEA_LEVEL + 1 && depth < 3 => b.sand,
            Biome::Mountains if column.height > 75 => b.stone,
            Biome::Plains | Biome::Mountains if depth == 0 => b.grass,
            Biome::Plains | Biome::Mountains if depth < 4 => b.dirt,
            _ => b.stone,
        }
    }

    /// Génère un chunk. Renvoie `None` s'il ne contient que de l'air
    /// (le ciel, ou hors de la hauteur du monde) : inutile de le stocker.
    pub fn generate(&self, chunk_pos: IVec3) -> Option<Chunk> {
        if !(0..WORLD_HEIGHT_CHUNKS).contains(&chunk_pos.y) {
            return None;
        }
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;

        // Les colonnes d'abord : elles disent si le chunk touche le sol.
        let mut columns = [[Column {
            height: 0,
            biome: Biome::Plains,
        }; CHUNK_SIZE]; CHUNK_SIZE];
        for (lx, row) in columns.iter_mut().enumerate() {
            for (lz, column) in row.iter_mut().enumerate() {
                *column = self.column(origin.x + lx as i32, origin.z + lz as i32);
            }
        }
        let highest = columns
            .iter()
            .flatten()
            .map(|c| c.height)
            .max()
            .unwrap_or(0)
            .max(SEA_LEVEL);
        if origin.y > highest {
            return None; // Tout le chunk est au-dessus du sol et de la mer.
        }

        let mut chunk = Chunk::new();
        for (lx, row) in columns.iter().enumerate() {
            for (lz, &column) in row.iter().enumerate() {
                let (x, z) = (origin.x + lx as i32, origin.z + lz as i32);
                // Pas de grotte juste sous un lac : elle viderait l'eau (qui ne coule pas encore).
                let cave_ceiling = if column.height < SEA_LEVEL + 2 {
                    column.height - 6
                } else {
                    column.height
                };
                for ly in 0..size {
                    let y = origin.y + ly;
                    if y > column.height {
                        // Au-dessus du sol : de l'eau jusqu'au niveau de la mer, puis de l'air.
                        if y > SEA_LEVEL {
                            break;
                        }
                        chunk.set(lx, ly as usize, lz, self.blocks.water);
                    } else if y > cave_ceiling || !self.is_cave(x, y, z) {
                        chunk.set(lx, ly as usize, lz, self.layer(column, y));
                    }
                }
            }
        }
        // Les grottes ont pu tout vider.
        (!chunk.is_empty()).then_some(chunk)
    }

    /// Génère d'un coup une région de `chunks_x × chunks_z` colonnes de chunks
    /// (sur toute la hauteur du monde). Pratique pour les tests et les benchmarks.
    pub fn generate_region(&self, chunks_x: i32, chunks_z: i32) -> World {
        let mut world = World::with_registry(self.registry.clone());
        for x in 0..chunks_x {
            for z in 0..chunks_z {
                for y in 0..WORLD_HEIGHT_CHUNKS {
                    let pos = IVec3::new(x, y, z);
                    if let Some(chunk) = self.generate(pos) {
                        world.insert(pos, chunk.into());
                    }
                }
            }
        }
        world
    }
}

/// 0 avant `edge0`, 1 après `edge1`, une courbe en S entre les deux.
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_the_same_world() {
        let a = TerrainGenerator::new(42);
        let b = TerrainGenerator::new(42);
        for pos in [IVec3::new(0, 0, 0), IVec3::new(-7, 1, 13)] {
            assert_eq!(a.generate(pos), b.generate(pos));
        }
    }

    #[test]
    fn different_seeds_give_different_worlds() {
        let a = TerrainGenerator::new(1);
        let b = TerrainGenerator::new(2);
        let heights = |g: &TerrainGenerator| {
            (0..64)
                .map(|x| g.column(x * 7, 0).height)
                .collect::<Vec<_>>()
        };
        assert_ne!(heights(&a), heights(&b));
    }

    #[test]
    fn surface_matches_column_height() {
        let generator = TerrainGenerator::new(42);
        let world = generator.generate_region(1, 1);
        let water = generator.blocks.water;
        let mut checked = 0;
        for x in 0..32 {
            for z in 0..32 {
                let height = generator.column(x, z).height;
                // Au-dessus du sol : de l'air, ou de l'eau sous le niveau de la mer.
                let above = world.block(IVec3::new(x, height + 1, z));
                assert!(above.is_air() || (above == water && height < SEA_LEVEL));
                // La surface peut être percée par une grotte : on compte les autres.
                if !world.block(IVec3::new(x, height, z)).is_air() {
                    checked += 1;
                }
            }
        }
        assert!(checked > 900, "presque toutes les colonnes ont un sol");
    }

    #[test]
    fn sky_and_out_of_range_chunks_are_empty() {
        let generator = TerrainGenerator::new(42);
        assert!(generator.generate(IVec3::new(0, -1, 0)).is_none());
        assert!(
            generator
                .generate(IVec3::new(0, WORLD_HEIGHT_CHUNKS, 0))
                .is_none()
        );
        assert!(generator.generate(IVec3::new(0, 0, 0)).is_some());
    }

    /// Sur un grand territoire, on doit croiser les trois biomes, de la neige et des grottes.
    #[test]
    fn large_area_has_every_feature() {
        let generator = TerrainGenerator::new(42);
        let mut biomes = Vec::new();
        let mut snow = false;
        for x in (-4000..4000).step_by(40) {
            for z in (-4000..4000).step_by(40) {
                let column = generator.column(x, z);
                if !biomes.contains(&column.biome) {
                    biomes.push(column.biome);
                }
                snow |= column.height >= SNOW_LINE;
            }
        }
        assert_eq!(biomes.len(), 3, "biomes trouvés : {biomes:?}");
        assert!(snow, "aucun sommet enneigé");

        let caves = (0..40)
            .flat_map(|x| (0..40).map(move |z| (x, z)))
            .filter(|&(x, z)| (2..20).any(|y| generator.is_cave(x * 3, y, z * 3)))
            .count();
        assert!(caves > 0, "aucune grotte");
    }

    #[test]
    fn low_ground_is_under_water() {
        let generator = TerrainGenerator::new(42);
        // On cherche une colonne sous le niveau de la mer.
        let (x, z, column) = (-2000..2000)
            .step_by(16)
            .flat_map(|x| (-2000..2000).step_by(16).map(move |z| (x, z)))
            .map(|(x, z)| (x, z, generator.column(x, z)))
            .find(|(_, _, c)| c.height < SEA_LEVEL - 2)
            .expect("aucun lac sur 4 000 × 4 000 blocs");
        let column_pos = World::split(IVec3::new(x, 0, z)).0;
        let mut world = World::new();
        for cy in 0..WORLD_HEIGHT_CHUNKS {
            let pos = IVec3::new(column_pos.x, cy, column_pos.z);
            if let Some(chunk) = generator.generate(pos) {
                world.insert(pos, chunk.into());
            }
        }
        assert_eq!(
            world.block(IVec3::new(x, SEA_LEVEL, z)),
            generator.blocks.water
        );
        assert!(world.block(IVec3::new(x, SEA_LEVEL + 1, z)).is_air());
        assert_eq!(
            world.block(IVec3::new(x, column.height, z)),
            generator.blocks.sand
        );
    }
}
