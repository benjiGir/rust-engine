//! La génération v3, lot 2 du plan (`docs/plan-generation-v3.md`) : le **relief**.
//!
//! Le terrain est la densité `final` du fichier `assets/core/worldgen/terrain_v3.ron` (voir
//! `density_graph`) : positive dans la matière, négative dans le vide. Elle est calculée aux coins
//! des cellules de 4 × 8 × 4 blocs de toute une colonne de chunks (9 × 49 × 9 coins, de y = -128 à
//! 256), une seule fois, puis interpolée pour chaque bloc. Le même calcul donne les hauteurs de
//! surface (pour le jeu, les arbres, le lointain) : elles collent donc toujours aux blocs.
//!
//! Ce lot pose la roche, l'eau sous le niveau de la mer, une surface simple selon le biome, les
//! minerais, les plantes et les arbres. Les grottes et aquifères (lot 3), les règles de surface et
//! les biomes multi-paramètres (lot 4) viendront ensuite ; les structures ne sont pas encore posées.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use glam::IVec3;

use super::column_cache::{ColumnCache, ColumnKey};
use super::v3_rules::{BiomeTable, Climate5, Material, SurfaceRules, SurfaceSpot};
use super::{CHUNK_SIZE, Chunk, Column, FREEZING, SEA_LEVEL, TerrainGenerator, World, hash3};
use crate::biome::{BiomeId, BiomeRegistry};
use crate::density_graph::{CELL_HEIGHT, CELL_WIDTH, DensityGraph, NodeId};
use crate::generator::{MAX_CHUNK_Y, MIN_CHUNK_Y};
use crate::registry::BlockRegistry;

/// Le relief v3, embarqué dans le binaire (comme les autres données de génération du jeu de base).
const TERRAIN: &str = include_str!("../../../../assets/core/worldgen/terrain_v3.ron");
/// Les biomes par paramètres et les règles de surface (lot 4).
const BIOMES: &str = include_str!("../../../../assets/core/worldgen/biomes_v3.ron");
const SURFACE: &str = include_str!("../../../../assets/core/worldgen/surface_v3.ron");

const SIZE: i32 = CHUNK_SIZE as i32;
/// Le bas et le haut du monde v3, en blocs.
const BOTTOM: i32 = MIN_CHUNK_Y * SIZE;
const TOP: i32 = MAX_CHUNK_Y * SIZE;
/// Coins par axe d'une colonne de chunks : 9 en x et z, 49 en y.
const CX: usize = (SIZE / CELL_WIDTH) as usize + 1;
const CY: usize = ((TOP - BOTTOM) / CELL_HEIGHT) as usize + 1;
/// Les contextes de colonne gardés (16 Kio de densité chacun).
const CONTEXT_LIMIT: usize = 512;

/// L'état de la génération v3 : le graphe compilé pour la graine, et les contextes de colonne.
pub(super) struct V3State {
    graph: DensityGraph,
    final_density: NodeId,
    continents: NodeId,
    erosion: NodeId,
    pv: NodeId,
    temperature: NodeId,
    humidity: NodeId,
    water_table: NodeId,
    offset: NodeId,
    sloped: NodeId,
    biome_table: BiomeTable,
    surface: SurfaceRules,
    contexts: ColumnCache<V3Column>,
}

impl std::fmt::Debug for V3State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V3State")
            .field("nodes", &self.graph.len())
            .finish()
    }
}

impl V3State {
    pub(super) fn new(seed: u32, blocks: &BlockRegistry, biomes: &BiomeRegistry) -> Result<Self> {
        let graph = DensityGraph::parse(TERRAIN, u64::from(seed)).context("relief v3")?;
        let id = |name: &str| {
            graph
                .function(name)
                .with_context(|| format!("relief v3 : fonction « {name} » absente"))
        };
        Ok(Self {
            final_density: id("final")?,
            continents: id("continents")?,
            erosion: id("erosion")?,
            pv: id("pv")?,
            temperature: id("temperature")?,
            humidity: id("humidity")?,
            water_table: id("water_table")?,
            offset: id("offset")?,
            sloped: id("sloped")?,
            graph,
            biome_table: BiomeTable::parse(BIOMES, biomes)?,
            surface: SurfaceRules::parse(SURFACE, u64::from(seed), blocks, biomes)?,
            contexts: ColumnCache::with_limit(CONTEXT_LIMIT),
        })
    }

    /// Les paramètres climatiques en (x, z) (pour les cartes et les tests).
    pub(super) fn parameters(&self, x: i32, z: i32) -> V3Parameters {
        let at = |id| self.graph.sample(id, x, 0, z);
        V3Parameters {
            continentalness: at(self.continents),
            erosion: at(self.erosion),
            pv: at(self.pv),
            temperature: at(self.temperature),
            humidity: at(self.humidity),
        }
    }
}

/// Les paramètres climatiques d'un point (de -1 à 1 environ).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct V3Parameters {
    pub continentalness: f64,
    pub erosion: f64,
    /// Pics et vallées : -1 au fond des vallées, +1 sur les crêtes.
    pub pv: f64,
    pub temperature: f64,
    pub humidity: f64,
}

/// Ce qu'on sait d'une colonne de chunks : la densité à ses coins, ses hauteurs, ses biomes.
pub(super) struct V3Column {
    /// Densité aux coins : x + 9 (z + 9 y), y de -128 à 256 par pas de 8.
    corners: Vec<f32>,
    /// Le plus haut y plein de toute la colonne de chunks.
    pub(super) max_y: i32,
    pub(super) columns: Box<[[Column; CHUNK_SIZE]; CHUNK_SIZE]>,
    /// Les biomes (x + 32 z), comme `Chunk::set_biomes` les attend.
    pub(super) biomes: Vec<u8>,
    /// La nappe de chaque colonne (x + 32 z) : son niveau (le vide souterrain plus bas est noyé) et le
    /// haut du barrage de roche qui sépare deux régions de niveaux différents (`i32::MIN` : aucun).
    water_table: Vec<(i32, i32)>,
    /// Les colonnes sur une pente raide (x + 32 z) : plus de 3 blocs de dénivelé sur 2.
    steep: Vec<bool>,
}

impl V3Column {
    /// La densité interpolée au bloc local (lx, y, lz) de la colonne.
    #[inline]
    fn density(&self, lx: i32, y: i32, lz: i32) -> f32 {
        let ly = (y - BOTTOM).clamp(0, TOP - BOTTOM - 1);
        let (cx, cy, cz) = (
            (lx / CELL_WIDTH) as usize,
            (ly / CELL_HEIGHT) as usize,
            (lz / CELL_WIDTH) as usize,
        );
        let tx = (lx % CELL_WIDTH) as f32 / CELL_WIDTH as f32;
        let ty = (ly % CELL_HEIGHT) as f32 / CELL_HEIGHT as f32;
        let tz = (lz % CELL_WIDTH) as f32 / CELL_WIDTH as f32;
        let at = |x: usize, y: usize, z: usize| self.corners[x + CX * (z + CX * y)];
        let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
        let x00 = lerp(tx, at(cx, cy, cz), at(cx + 1, cy, cz));
        let x10 = lerp(tx, at(cx, cy + 1, cz), at(cx + 1, cy + 1, cz));
        let x01 = lerp(tx, at(cx, cy, cz + 1), at(cx + 1, cy, cz + 1));
        let x11 = lerp(tx, at(cx, cy + 1, cz + 1), at(cx + 1, cy + 1, cz + 1));
        lerp(tz, lerp(ty, x00, x10), lerp(ty, x01, x11))
    }

    /// Le plus haut y plein de la colonne de blocs (lx, lz), ou `BOTTOM - 1` s'il n'y en a pas.
    fn top(&self, lx: i32, lz: i32, from: i32) -> i32 {
        let mut y = from;
        while y >= BOTTOM {
            if self.density(lx, y, lz) > 0.0 {
                return y;
            }
            y -= 1;
        }
        BOTTOM - 1
    }
}

impl TerrainGenerator {
    pub(super) fn v3_state(&self) -> &V3State {
        self.v3.as_ref().expect("génération v3")
    }

    /// Le contexte de la colonne de chunks (cx, cz), depuis le cache si possible.
    pub(super) fn v3_column(&self, cx: i32, cz: i32) -> Arc<V3Column> {
        let key = ColumnKey {
            seed: self.seed,
            dimension: self.dimension_tag,
            version: self.version,
            cx,
            cz,
        };
        self.v3_state()
            .contexts
            .get_or_compute(key, || self.build_v3_column(cx, cz))
    }

    fn build_v3_column(&self, cx: i32, cz: i32) -> V3Column {
        let state = self.v3_state();
        let (x0, z0) = (cx * SIZE, cz * SIZE);
        let positions = (0..CY as i32).flat_map(|j| {
            (0..CX as i32).flat_map(move |k| {
                (0..CX as i32).map(move |i| {
                    IVec3::new(
                        x0 + i * CELL_WIDTH,
                        BOTTOM + j * CELL_HEIGHT,
                        z0 + k * CELL_WIDTH,
                    )
                })
            })
        });
        let corners: Vec<f32> = state
            .graph
            .sample_many(state.final_density, positions)
            .into_iter()
            .map(|v| v as f32)
            .collect();
        // Le plus haut niveau de coins où quelque chose est plein : on ne cherche la surface qu'en dessous.
        let highest = (0..CY)
            .rev()
            .find(|&j| {
                corners[j * CX * CX..(j + 1) * CX * CX]
                    .iter()
                    .any(|&d| d > 0.0)
            })
            .map_or(BOTTOM, |j| {
                (BOTTOM + (j as i32 + 1) * CELL_HEIGHT).min(TOP - 1)
            });
        let mut ctx = V3Column {
            corners,
            max_y: BOTTOM - 1,
            columns: Box::new([[Column::default(); CHUNK_SIZE]; CHUNK_SIZE]),
            biomes: vec![0; CHUNK_SIZE * CHUNK_SIZE],
            water_table: vec![(i32::MIN, i32::MIN); CHUNK_SIZE * CHUNK_SIZE],
            steep: vec![false; CHUNK_SIZE * CHUNK_SIZE],
        };
        // Les paramètres climatiques sur la grille de 4 blocs (ils ne varient guère plus vite).
        let climate: Vec<(f64, f64, f64, f64, f64)> = {
            let ids = [
                state.continents,
                state.erosion,
                state.pv,
                state.temperature,
                state.humidity,
            ];
            let grid = |id| {
                state.graph.sample_many(
                    id,
                    (0..CX as i32).flat_map(move |k| {
                        (0..CX as i32)
                            .map(move |i| IVec3::new(x0 + i * CELL_WIDTH, 0, z0 + k * CELL_WIDTH))
                    }),
                )
            };
            let values: Vec<Vec<f64>> = ids.iter().map(|&id| grid(id)).collect();
            (0..CX * CX)
                .map(|i| {
                    (
                        values[0][i],
                        values[1][i],
                        values[2][i],
                        values[3][i],
                        values[4][i],
                    )
                })
                .collect()
        };
        let mut max_y = BOTTOM - 1;
        let mut regions = std::collections::HashMap::new();
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let height = ctx.top(lx, lz, highest);
                max_y = max_y.max(height);
                let (c, e, pv, t, h) =
                    climate[(lx / CELL_WIDTH) as usize + CX * (lz / CELL_WIDTH) as usize];
                let table = self.v3_water_table(&mut regions, x0 + lx, z0 + lz);
                ctx.water_table[lx as usize + lz as usize * CHUNK_SIZE] = table;
                let biome = self.v3_biome(c, e, pv, t, h);
                // Il fait plus froid en altitude : la neige descend des sommets.
                // Même règle que `plants::is_cold` (la neige que posent les ticks aléatoires) : sinon la
                // génération et le jeu se contrediraient et l'herbe changerait sans fin.
                let altitude = (height - (SEA_LEVEL + 40)).max(0) as f32;
                ctx.columns[lx as usize][lz as usize] = Column {
                    height,
                    biome,
                    temperature: self.biomes.get(biome).temperature - altitude * 0.012,
                    climate_temperature: t as f32,
                };
                ctx.biomes[lx as usize + lz as usize * CHUNK_SIZE] = biome.0;
            }
        }
        ctx.max_y = max_y;
        // La pente : l'écart de hauteur entre les deux voisins de chaque axe (vers l'intérieur au bord
        // de la colonne de chunks).
        let height = |lx: i32, lz: i32| ctx.columns[lx as usize][lz as usize].height;
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let (x0, x1) = ((lx - 1).max(0), (lx + 1).min(SIZE - 1));
                let (z0, z1) = ((lz - 1).max(0), (lz + 1).min(SIZE - 1));
                let dx = (height(x1, lz) - height(x0, lz)).abs() * 2 / (x1 - x0);
                let dz = (height(lx, z1) - height(lx, z0)).abs() * 2 / (z1 - z0);
                ctx.steep[lx as usize + lz as usize * CHUNK_SIZE] = dx.max(dz) > 3;
            }
        }
        ctx
    }

    /// Le biome d'une colonne : le plus proche de ses paramètres dans `biomes_v3.ron`.
    fn v3_biome(&self, c: f64, e: f64, pv: f64, t: f64, h: f64) -> BiomeId {
        self.v3_state().biome_table.pick(Climate5 {
            temperature: t,
            humidity: h,
            continentalness: c,
            erosion: e,
            pv,
        })
    }

    /// La colonne (x, z) de la génération v3.
    pub(super) fn column_v3(&self, x: i32, z: i32) -> Column {
        let ctx = self.v3_column(x.div_euclid(SIZE), z.div_euclid(SIZE));
        ctx.columns[x.rem_euclid(SIZE) as usize][z.rem_euclid(SIZE) as usize]
    }

    /// Une colonne approchée pour le terrain lointain : la surface est cherchée sur la seule verticale
    /// (x, z), tous les 8 blocs puis par interpolation, dans la densité sans grottes ; le biome vient des
    /// paramètres du point. Bien moins cher qu'un contexte de colonne de chunks (qui calcule aussi les
    /// grottes de 81 verticales), et sans évincer du cache les contextes dont les chunks ont besoin.
    pub(super) fn column_far_v3(&self, x: i32, z: i32) -> Column {
        let state = self.v3_state();
        let positions = (0..CY as i32)
            .rev()
            .map(|j| IVec3::new(x, BOTTOM + j * CELL_HEIGHT, z));
        let values = state.graph.sample_many(state.sloped, positions);
        let mut height = BOTTOM;
        for (k, pair) in values.windows(2).enumerate() {
            let (above, below) = (pair[0], pair[1]);
            if below > 0.0 {
                let y_above = BOTTOM + (CY as i32 - 1 - k as i32) * CELL_HEIGHT;
                // Le zéro entre les deux niveaux, en ligne droite.
                let t = if above < 0.0 {
                    below / (below - above)
                } else {
                    1.0
                };
                height = y_above - CELL_HEIGHT + (t * CELL_HEIGHT as f64).floor() as i32;
                break;
            }
        }
        let p = state.parameters(x, z);
        let biome = self.v3_biome(
            p.continentalness,
            p.erosion,
            p.pv,
            p.temperature,
            p.humidity,
        );
        // Même règle que `plants::is_cold` (la neige que posent les ticks aléatoires) : sinon la
        // génération et le jeu se contrediraient et l'herbe changerait sans fin.
        let altitude = (height - (SEA_LEVEL + 40)).max(0) as f32;
        Column {
            height,
            biome,
            temperature: self.biomes.get(biome).temperature - altitude * 0.012,
            climate_temperature: p.temperature as f32,
        }
    }

    /// Un chunk de la génération v3.
    pub(super) fn generate_v3(&self, chunk_pos: IVec3) -> Option<Chunk> {
        if !self.chunk_range().contains(&chunk_pos.y) {
            return None;
        }
        let origin = World::chunk_origin(chunk_pos);
        let ctx = self.v3_column(chunk_pos.x, chunk_pos.z);
        // Les arbres dépassent la surface d'une trentaine de blocs.
        if origin.y > ctx.max_y.max(SEA_LEVEL) + 32 {
            return None;
        }
        let mut chunk = Chunk::new();
        chunk.set_biomes(ctx.biomes.clone());
        self.fill_v3(&mut chunk, origin, &ctx);
        self.place_ores(&mut chunk, chunk_pos, &ctx.columns);
        self.place_plants(&mut chunk, chunk_pos, &ctx.columns);
        self.place_trees(&mut chunk, chunk_pos);
        (!chunk.is_empty()).then_some(chunk)
    }

    fn fill_v3(&self, chunk: &mut Chunk, origin: IVec3, ctx: &V3Column) {
        let b = self.blocks;
        let state = self.v3_state();
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let (x, z) = (origin.x + lx, origin.z + lz);
                let column = ctx.columns[lx as usize][lz as usize];
                let biome = self.biomes.get(column.biome);
                let frozen = column.temperature < FREEZING;
                let (table, dam) = ctx.water_table[lx as usize + lz as usize * CHUNK_SIZE];
                let steep = ctx.steep[lx as usize + lz as usize * CHUNK_SIZE];
                // Combien de blocs pleins d'affilée au-dessus (jusqu'à l'air ou l'eau) : la surface,
                // puis la sous-couche, puis la roche. On part du haut du chunk.
                let mut solid_above = {
                    let mut n = 0;
                    let mut y = origin.y + SIZE;
                    while n < 8 && ctx.density(lx, y, lz) > 0.0 && y < TOP {
                        n += 1;
                        y += 1;
                    }
                    n
                };
                for ly in (0..SIZE).rev() {
                    let y = origin.y + ly;
                    let solid = ctx.density(lx, y, lz) > 0.0;
                    let block = if y == BOTTOM
                        || (y < BOTTOM + 4
                            && hash3(self.seed, x, y, z, 1).is_multiple_of((y - BOTTOM) as u64 + 1))
                    {
                        Some(b.bedrock)
                    } else if solid {
                        let depth = solid_above;
                        solid_above += 1;
                        // Les matériaux de surface ne vont qu'au sol du dessus, plein sans interruption
                        // depuis le haut de la colonne : le plancher et le plafond d'une grotte restent
                        // en roche (de l'herbe dans le noir redeviendrait de la terre, bloc après bloc,
                        // et ferait remailler sans fin). Les règles viennent de `surface_v3.ron`.
                        let near_top = column.height - y == depth;
                        let rule = if near_top && y >= 0 {
                            // Même règle que `plants::is_cold` : la neige ne bougera pas.
                            let altitude = (y - (SEA_LEVEL + 40)).max(0) as f32;
                            state.surface.material(&SurfaceSpot {
                                x,
                                y,
                                z,
                                depth,
                                subsurface_depth: biome.subsurface_depth,
                                biome: column.biome,
                                underwater: column.height < SEA_LEVEL,
                                cold: biome.temperature - altitude * 0.012 < FREEZING,
                                steep,
                            })
                        } else {
                            None
                        };
                        Some(match rule {
                            Some(Material::Block(block)) => block,
                            Some(Material::BiomeSurface) => {
                                self.surface_patch(biome, x, z).unwrap_or(biome.surface)
                            }
                            Some(Material::BiomeSubsurface) => biome.subsurface,
                            Some(Material::BiomeUnderwater) => biome.underwater,
                            Some(Material::Band) if !biome.bands.is_empty() => {
                                self.band(biome, x, y, z)
                            }
                            Some(Material::Band) => biome.subsurface,
                            None if y < 0 => {
                                // La pierre profonde, avec une frontière irrégulière vers 0.
                                let jitter = (hash3(self.seed, x, y, z, 2) % 8) as i32;
                                if y < -4 + jitter {
                                    b.deepstone
                                } else {
                                    b.stone
                                }
                            }
                            None => b.stone,
                        })
                    } else if y < column.height {
                        // Sous la surface : une grotte, dont l'eau (ou la lave) vient de l'aquifère.
                        if y < LAVA_TOP {
                            solid_above = 0;
                            Some(b.lava)
                        } else if y < dam {
                            // Le barrage entre deux nappes : de la roche.
                            solid_above += 1;
                            Some(b.stone)
                        } else if y < table {
                            solid_above = 0;
                            Some(b.water)
                        } else {
                            solid_above = 0;
                            None
                        }
                    } else {
                        solid_above = 0;
                        (y <= SEA_LEVEL).then_some(if y == SEA_LEVEL && frozen {
                            b.ice
                        } else {
                            b.water
                        })
                    };
                    if let Some(block) = block {
                        chunk.set(lx as usize, ly as usize, lz as usize, block);
                    }
                }
            }
        }
    }

    /// Le niveau d'une région de nappe : la nappe au centre de la région, toujours une vingtaine de
    /// blocs sous la surface approchée ; `i32::MIN` si la région est sèche.
    fn v3_region_level(&self, region: (i32, i32)) -> (i32, i32, i32) {
        let h = |salt: u64| hash3(self.seed, region.0, 0, region.1, salt);
        let center = (
            region.0 * REGION + 16 + (h(80) % (REGION as u64 - 32)) as i32,
            region.1 * REGION + 16 + (h(81) % (REGION as u64 - 32)) as i32,
        );
        let state = self.v3_state();
        let table = state.graph.sample(state.water_table, center.0, 0, center.1);
        let ground = self.v3_preliminary_surface(center.0, center.1);
        let level = (table.round() as i32).min(ground - 20);
        let level = if level < BOTTOM + 30 { i32::MIN } else { level };
        (center.0, center.1, level)
    }

    /// La surface sans le bruit 3D : là où la pente verticale et le décalage s'annulent.
    fn v3_preliminary_surface(&self, x: i32, z: i32) -> i32 {
        let state = self.v3_state();
        64 + (128.0 * state.graph.sample(state.offset, x, 0, z)).round() as i32
    }

    /// La nappe de la colonne (x, z) : le niveau de la région la plus proche, et le haut d'un barrage
    /// si la colonne est à la frontière de deux régions de niveaux différents.
    fn v3_water_table(
        &self,
        regions: &mut std::collections::HashMap<(i32, i32), (i32, i32, i32)>,
        x: i32,
        z: i32,
    ) -> (i32, i32) {
        let home = (x.div_euclid(REGION), z.div_euclid(REGION));
        let (mut first, mut second) = ((f32::MAX, i32::MIN), (f32::MAX, i32::MIN));
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = (home.0 + dx, home.1 + dz);
                let (cx, cz, level) = *regions
                    .entry(key)
                    .or_insert_with(|| self.v3_region_level(key));
                let d = (((cx - x) * (cx - x) + (cz - z) * (cz - z)) as f32).sqrt();
                if d < first.0 {
                    second = first;
                    first = (d, level);
                } else if d < second.0 {
                    second = (d, level);
                }
            }
        }
        let dam = if first.1 != second.1 && second.0 - first.0 < DAM_WIDTH {
            first.1.max(second.1)
        } else {
            i32::MIN
        };
        (first.1, dam)
    }

    /// Les paramètres climatiques v3 en (x, z) ; `None` hors de la génération v3.
    pub fn v3_parameters(&self, x: i32, z: i32) -> Option<V3Parameters> {
        Some(self.v3.as_ref()?.parameters(x, z))
    }
}

/// Sous ce niveau, le vide souterrain est de la lave (le fond du monde).
const LAVA_TOP: i32 = BOTTOM + 20;
/// La taille d'une région de nappe (en blocs) : le niveau d'eau y est plat.
const REGION: i32 = 96;
/// L'épaisseur d'un barrage entre deux régions de nappe (écart de distance aux deux centres).
const DAM_WIDTH: f32 = 3.0;
