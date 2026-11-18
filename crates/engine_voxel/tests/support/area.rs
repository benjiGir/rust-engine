//! Une zone de blocs générés par la génération v2, lue bloc par bloc : partagée par les tests de
//! `generation_v2.rs` et de `relief_v2.rs`.

use std::collections::HashMap;

use engine_voxel::{BlockId, Chunk, MacroWorld, SEA_LEVEL, TerrainGenerator, WORLD_HEIGHT_CHUNKS};
use glam::IVec3;

/// Les six chunks (de bas en haut) d'une colonne de chunks.
pub type Verticale = Vec<Option<Chunk>>;

/// Les colonnes de chunks `cx0..=cx1` × `cz0..=cz1`, générées sur plusieurs threads.
pub struct Area<'a> {
    pub generator: &'a TerrainGenerator,
    pub columns: HashMap<(i32, i32), Verticale>,
    pub min: (i32, i32),
    pub max: (i32, i32),
}

impl<'a> Area<'a> {
    pub fn new(generator: &'a TerrainGenerator, min: (i32, i32), max: (i32, i32)) -> Self {
        let wanted: Vec<(i32, i32)> = (min.1..=max.1)
            .flat_map(|cz| (min.0..=max.0).map(move |cx| (cx, cz)))
            .collect();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let results: Vec<((i32, i32), Verticale)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let wanted = &wanted;
                    scope.spawn(move || {
                        wanted
                            .iter()
                            .skip(t)
                            .step_by(threads)
                            .map(|&(cx, cz)| {
                                let chunks = (0..WORLD_HEIGHT_CHUNKS)
                                    .map(|cy| generator.generate(IVec3::new(cx, cy, cz)))
                                    .collect();
                                ((cx, cz), chunks)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("thread de génération"))
                .collect()
        });
        Self {
            generator,
            columns: results.into_iter().collect(),
            min: (min.0 * 32, min.1 * 32),
            max: (max.0 * 32 + 31, max.1 * 32 + 31),
        }
    }

    pub fn contains(&self, x: i32, z: i32) -> bool {
        (self.min.0..=self.max.0).contains(&x) && (self.min.1..=self.max.1).contains(&z)
    }

    /// Le bloc (x, y, z) : de l'air hors de la zone ou du monde.
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockId {
        if !(0..WORLD_HEIGHT_CHUNKS * 32).contains(&y) || !self.contains(x, z) {
            return BlockId::AIR;
        }
        self.columns[&(x.div_euclid(32), z.div_euclid(32))][(y / 32) as usize]
            .as_ref()
            .map_or(BlockId::AIR, |c| {
                c.get(
                    x.rem_euclid(32) as usize,
                    (y % 32) as usize,
                    z.rem_euclid(32) as usize,
                )
            })
    }

    pub fn key(&self, block: BlockId) -> &str {
        &self.generator.registry().def(block).key
    }

    pub fn is_water(&self, block: BlockId) -> bool {
        self.key(block) == "core:water"
    }

    pub fn is_solid(&self, block: BlockId) -> bool {
        let def = self.generator.registry().def(block);
        def.solid && !def.liquid
    }

    /// Le dernier bloc d'eau vu en descendant : le premier bloc plein ou liquide rencontré (les
    /// plantes, nénuphars et l'air sont sautés) doit être de l'eau.
    pub fn water_surface(&self, x: i32, z: i32) -> Option<i32> {
        for y in (0..WORLD_HEIGHT_CHUNKS * 32).rev() {
            let b = self.block(x, y, z);
            if b.is_air() {
                continue;
            }
            let def = self.generator.registry().def(b);
            if def.liquid {
                return self.is_water(b).then_some(y);
            }
            if def.solid {
                return None;
            }
        }
        None
    }

    /// Le bloc plein le plus haut (hors bois et feuillage), et son y.
    pub fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockId)> {
        for y in (0..WORLD_HEIGHT_CHUNKS * 32).rev() {
            let b = self.block(x, y, z);
            if b.is_air() || !self.is_solid(b) {
                continue;
            }
            let k = self.key(b);
            if k.ends_with("_leaves") || k.ends_with("_log") || k == "core:cactus" {
                continue;
            }
            return Some((y, b));
        }
        None
    }
}

/// La zone de 5 × 5 colonnes de chunks centrée sur un point.
pub fn around(generator: &TerrainGenerator, spot: (i32, i32)) -> Area<'_> {
    around_by(generator, spot, 2)
}

/// La zone de `(2 reach + 1)²` colonnes de chunks centrée sur un point.
pub fn around_by(generator: &TerrainGenerator, (x, z): (i32, i32), reach: i32) -> Area<'_> {
    let (cx, cz) = (x.div_euclid(32), z.div_euclid(32));
    Area::new(
        generator,
        (cx - reach, cz - reach),
        (cx + reach, cz + reach),
    )
}

/// Des points de massif (chaîne haute, altitude), espacés d'au moins 400 blocs, en spirale depuis l'origine.
pub fn mountain_spots(seed: u32, wanted: usize) -> Vec<(i32, i32)> {
    let world = MacroWorld::new(seed);
    let mut spots: Vec<(i32, i32)> = Vec::new();
    for ring in 0..60 {
        let r = ring * 128;
        for d in (-r..=r).step_by(128) {
            for (x, z) in [(d, -r), (d, r), (-r, d), (r, d)] {
                let g = world.geography().sample(x as f64, z as f64);
                if g.chain_height > 40.0
                    && g.elevation > SEA_LEVEL as f32 + 45.0
                    && spots
                        .iter()
                        .all(|&(sx, sz)| (sx - x).abs().max((sz - z).abs()) > 400)
                {
                    spots.push((x, z));
                    if spots.len() >= wanted {
                        return spots;
                    }
                }
            }
        }
    }
    spots
}
