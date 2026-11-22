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
use super::v3_rules::{BiomeTable, Climate6, Material, SurfaceRules, SurfaceSpot};
use super::{CHUNK_SIZE, Chunk, Column, FREEZING, SEA_LEVEL, TerrainGenerator, World, hash3};
use crate::BlockId;
use crate::registry::BlockRegistry;
use crate::worldgen::biome::{BiomeId, BiomeRegistry};
use crate::worldgen::density_graph::{CELL_HEIGHT, CELL_WIDTH, DensityGraph, NodeId};
use crate::worldgen::generator::{MAX_CHUNK_Y, MIN_CHUNK_Y};

/// Le relief v3, embarqué dans le binaire (comme les autres données de génération du jeu de base).
const TERRAIN: &str = include_str!("../../../../../assets/core/worldgen/terrain_v3.ron");
/// Les biomes par paramètres et les règles de surface (lot 4).
const BIOMES: &str = include_str!("../../../../../assets/core/worldgen/biomes_v3.ron");
const SURFACE: &str = include_str!("../../../../../assets/core/worldgen/surface_v3.ron");

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
    variety: NodeId,
    water_table: NodeId,
    /// La déformation des frontières de régions de nappe (les mêmes bruits que le relief).
    shift_x: NodeId,
    shift_z: NodeId,
    /// Les niveaux des régions de nappe déjà calculés (centre et niveau) : une région sert à neuf
    /// colonnes de chunks, et son niveau lit la surface en plusieurs points.
    regions: std::sync::Mutex<std::collections::HashMap<(i32, i32), RegionLevel>>,
    /// Les planchers des structures enterrées déjà choisis (par site).
    floors: std::sync::Mutex<std::collections::HashMap<(i32, i32), i32>>,
    offset: NodeId,
    sloped: NodeId,
    /// La hauteur des volcans (en blocs, par carrés de 4) et leur champ de cônes (centres, cratères).
    volcano: NodeId,
    volcano_cones: NodeId,
    springs: NodeId,
    /// Le biome des cônes, et celui des sources chaudes (`None` dans un registre qui ne les a pas).
    volcano_biome: Option<BiomeId>,
    springs_biome: Option<BiomeId>,
    /// Les biomes souterrains : où il y en a, lequel, et leurs blocs (`None` dans un registre qui ne
    /// les a pas).
    cave_amount: NodeId,
    cave_kind: NodeId,
    caves: Option<CaveBiomes>,
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
            variety: id("variety")?,
            water_table: id("water_table")?,
            shift_x: id("shift_x")?,
            shift_z: id("shift_z")?,
            regions: std::sync::Mutex::new(std::collections::HashMap::new()),
            floors: std::sync::Mutex::new(std::collections::HashMap::new()),
            offset: id("offset")?,
            sloped: id("sloped")?,
            volcano: id("volcano")?,
            volcano_cones: id("volcano_cones")?,
            springs: id("springs")?,
            volcano_biome: biomes.id("core:volcano"),
            cave_amount: id("cave_amount")?,
            cave_kind: id("cave_kind")?,
            caves: CaveBiomes::resolve(blocks, biomes),
            springs_biome: biomes.id("core:hot_springs"),
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
            variety: at(self.variety),
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
    /// La variante d'un climat (forêt claire ou sombre, verger...), indépendante du relief.
    pub variety: f64,
}

impl V3Parameters {
    /// Les paramètres de la table des biomes.
    fn climate(self) -> Climate6 {
        Climate6 {
            temperature: self.temperature,
            humidity: self.humidity,
            continentalness: self.continentalness,
            erosion: self.erosion,
            pv: self.pv,
            variety: self.variety,
        }
    }
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
    /// La nappe de chaque colonne (x + 32 z) : son niveau (le vide souterrain plus bas est noyé), et le
    /// barrage de roche qui sépare deux régions de niveaux différents, du niveau le plus bas au plus haut
    /// (`(i32::MIN, i32::MIN)` : aucun).
    water_table: Vec<(i32, (i32, i32))>,
    /// Les colonnes sur une pente raide (x + 32 z) : plus de 3 blocs de dénivelé sur 2.
    steep: Vec<bool>,
    /// Le haut de la lave d'un cratère, ou de l'eau d'un bassin de source chaude, au-dessus du sol de
    /// la colonne (x + 32 z) ; `i32::MIN` : rien.
    lava: Vec<i32>,
    pool: Vec<i32>,
    /// La mer peut remplir la colonne (x + 32 z) au-dessus de son sol : le relief sans grottes y est
    /// sous l'eau, ou le sol est à peine sous la mer.
    sea: Vec<bool>,
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
            water_table: vec![(i32::MIN, (i32::MIN, i32::MIN)); CHUNK_SIZE * CHUNK_SIZE],
            steep: vec![false; CHUNK_SIZE * CHUNK_SIZE],
            lava: vec![i32::MIN; CHUNK_SIZE * CHUNK_SIZE],
            pool: vec![i32::MIN; CHUNK_SIZE * CHUNK_SIZE],
            sea: vec![true; CHUNK_SIZE * CHUNK_SIZE],
        };
        // Les paramètres climatiques sur la grille de 4 blocs (ils ne varient guère plus vite).
        // Les paramètres et la hauteur des volcans, par point de la grille.
        let climate: Vec<(V3Parameters, f64)> = {
            let ids = [
                state.continents,
                state.erosion,
                state.pv,
                state.temperature,
                state.humidity,
                state.variety,
                state.volcano,
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
                        V3Parameters {
                            continentalness: values[0][i],
                            erosion: values[1][i],
                            pv: values[2][i],
                            temperature: values[3][i],
                            humidity: values[4][i],
                            variety: values[5][i],
                        },
                        values[6][i],
                    )
                })
                .collect()
        };
        let mut max_y = BOTTOM - 1;
        // La nappe de chaque colonne et de ses voisines (une marge de `BANK_REACH` blocs) : la région la
        // plus proche du point déformé (des frontières sinueuses, pas des plans droits).
        const M: i32 = BANK_REACH;
        const W: i32 = SIZE + 2 * M;
        let mut known = std::collections::HashMap::new();
        let levels: Vec<i32> = {
            let positions = || {
                (-M..SIZE + M).flat_map(move |lz| {
                    (-M..SIZE + M).map(move |lx| IVec3::new(x0 + lx, 0, z0 + lz))
                })
            };
            let sx = state.graph.sample_many(state.shift_x, positions());
            let sz = state.graph.sample_many(state.shift_z, positions());
            positions()
                .zip(sx.into_iter().zip(sz))
                .map(|(p, (wx, wz))| {
                    self.v3_water_table(&mut known, f64::from(p.x) + wx, f64::from(p.z) + wz)
                })
                .collect()
        };
        let level_at = |lx: i32, lz: i32| levels[((lx + M) + (lz + M) * W) as usize];
        // Les berges : du côté bas d'une frontière entre deux nappes, de la roche monte jusqu'au niveau de
        // la nappe haute sur la première colonne (l'eau ne touche jamais l'air), puis descend de
        // `BANK_SLOPE` blocs par bloc (une distance de chanfrein en deux passes). Une région sèche compte
        // comme une nappe au haut de la lave.
        let floor_of = |level: i32| level.max(LAVA_TOP);
        let mut bank: Vec<i32> = levels.iter().map(|&l| floor_of(l)).collect();
        let (orth, diag) = (BANK_SLOPE, BANK_SLOPE * 3 / 2);
        let idx = |x: i32, z: i32| (x + z * W) as usize;
        for z in 0..W {
            for x in 0..W {
                let mut v = bank[idx(x, z)];
                for (dx, dz, cost) in [(-1, 0, orth), (0, -1, orth), (-1, -1, diag), (1, -1, diag)]
                {
                    let (nx, nz) = (x + dx, z + dz);
                    if (0..W).contains(&nx) && (0..W).contains(&nz) {
                        v = v.max(bank[idx(nx, nz)] - cost);
                    }
                }
                bank[idx(x, z)] = v;
            }
        }
        for z in (0..W).rev() {
            for x in (0..W).rev() {
                let mut v = bank[idx(x, z)];
                for (dx, dz, cost) in [(1, 0, orth), (0, 1, orth), (1, 1, diag), (-1, 1, diag)] {
                    let (nx, nz) = (x + dx, z + dz);
                    if (0..W).contains(&nx) && (0..W).contains(&nz) {
                        v = v.max(bank[idx(nx, nz)] - cost);
                    }
                }
                bank[idx(x, z)] = v;
            }
        }
        let bank_at = |lx: i32, lz: i32| bank[idx(lx + M, lz + M)];
        // Le haut de la mer libre : la mer ne remplit une colonne que si le relief sans grottes y est sous
        // l'eau (une grotte ou un puits ouverts sous le niveau de la mer restent secs).
        let sea_open: Vec<f64> = state.graph.sample_many(
            state.sloped,
            (0..CX as i32).flat_map(move |k| {
                (0..CX as i32)
                    .map(move |i| IVec3::new(x0 + i * CELL_WIDTH, SEA_LEVEL, z0 + k * CELL_WIDTH))
            }),
        );
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let height = ctx.top(lx, lz, highest);
                max_y = max_y.max(height);
                let (params, volcano) =
                    climate[(lx / CELL_WIDTH) as usize + CX * (lz / CELL_WIDTH) as usize];
                // La berge : de la roche dans le vide des grottes, du niveau de la colonne jusqu'à la
                // berge (le niveau de la nappe haute voisine, moins la pente) ; la première colonne au
                // contact d'une nappe plus haute va jusqu'à son niveau.
                let own = level_at(lx, lz);
                let mut neighbour = i32::MIN;
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        neighbour = neighbour.max(level_at(lx + dx, lz + dz));
                    }
                }
                let top = if neighbour > own {
                    neighbour
                } else {
                    bank_at(lx, lz)
                };
                let dam = if top > floor_of(own) {
                    (floor_of(own), top)
                } else {
                    (i32::MIN, i32::MIN)
                };
                ctx.water_table[lx as usize + lz as usize * CHUNK_SIZE] = (own, dam);
                // La mer libre : le relief sans grottes au niveau de la mer, interpolé entre les coins.
                let (tx, tz) = (
                    f64::from(lx % CELL_WIDTH) / f64::from(CELL_WIDTH),
                    f64::from(lz % CELL_WIDTH) / f64::from(CELL_WIDTH),
                );
                let (ci, ck) = ((lx / CELL_WIDTH) as usize, (lz / CELL_WIDTH) as usize);
                let corner = |i: usize, k: usize| sea_open[i + CX * k];
                let a = corner(ci, ck) + tx * (corner(ci + 1, ck) - corner(ci, ck));
                let b = corner(ci, ck + 1) + tx * (corner(ci + 1, ck + 1) - corner(ci, ck + 1));
                ctx.sea[lx as usize + lz as usize * CHUNK_SIZE] =
                    a + tz * (b - a) < 0.3 || height >= SEA_LEVEL - 3;
                let biome = self.v3_biome_with_volcano(params, volcano);
                // Il fait plus froid en altitude : la neige descend des sommets.
                // Même règle que `plants::is_cold` (la neige que posent les ticks aléatoires) : sinon la
                // génération et le jeu se contrediraient et l'herbe changerait sans fin.
                let altitude = (height - (SEA_LEVEL + 40)).max(0) as f32;
                ctx.columns[lx as usize][lz as usize] = Column {
                    height,
                    biome,
                    temperature: self.biomes.get(biome).temperature - altitude * 0.012,
                    climate_temperature: params.temperature as f32,
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
        self.v3_crater_lava(&mut ctx, x0, z0);
        self.v3_spring_pools(&mut ctx, x0, z0);
        ctx
    }

    /// La lave des cratères qui touchent la colonne de chunks : un lac plat, deux blocs sous le bord le
    /// plus bas du cratère (lu sur huit points du bord, aux coins des cellules comme la densité), dans
    /// les colonnes du cratère dont le sol est plus bas. Le même niveau pour toutes les colonnes de
    /// chunks du cratère : il ne dépend que du volcan.
    fn v3_crater_lava(&self, ctx: &mut V3Column, x0: i32, z0: i32) {
        let state = self.v3_state();
        let Some(cones) = state.graph.cones(state.volcano_cones) else {
            return;
        };
        let reach = cones.crater_radius.ceil() as i32 + 1;
        let sites = state.graph.cone_sites(
            state.volcano_cones,
            (x0 - reach, z0 - reach),
            (x0 + SIZE + reach, z0 + SIZE + reach),
        );
        for (cx, cz) in sites {
            if cx + reach < x0
                || cx - reach >= x0 + SIZE
                || cz + reach < z0
                || cz - reach >= z0 + SIZE
            {
                continue;
            }
            let rim = cones.crater_radius + 3.0;
            let level = (0..8)
                .map(|k| {
                    let a = std::f64::consts::TAU * f64::from(k) / 8.0;
                    let x = cx + (rim * a.cos()).round() as i32;
                    let z = cz + (rim * a.sin()).round() as i32;
                    self.v3_surface_estimate(x.div_euclid(4) * 4, z.div_euclid(4) * 4)
                })
                .min()
                .unwrap_or(BOTTOM)
                - 2;
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    let (dx, dz) = (f64::from(x0 + lx - cx), f64::from(z0 + lz - cz));
                    let i = lx as usize + lz as usize * CHUNK_SIZE;
                    if (dx * dx + dz * dz).sqrt() < cones.crater_radius
                        && ctx.columns[lx as usize][lz as usize].height < level
                    {
                        ctx.lava[i] = level;
                    }
                }
            }
        }
    }

    /// Les bassins des sources chaudes : sur un sol bien plat (les 5 × 5 colonnes autour à la même
    /// hauteur, loin du bord de la colonne de chunks), là où le bruit des bassins est fort, un ou deux
    /// blocs d'eau à la place du sol. La hauteur de la colonne devient le fond du bassin.
    fn v3_spring_pools(&self, ctx: &mut V3Column, x0: i32, z0: i32) {
        let state = self.v3_state();
        let Some(springs) = state.springs_biome else {
            return;
        };
        let heights: Vec<i32> = (0..SIZE * SIZE)
            .map(|i| ctx.columns[(i % SIZE) as usize][(i / SIZE) as usize].height)
            .collect();
        let height = |lx: i32, lz: i32| heights[(lx + lz * SIZE) as usize];
        for lz in 2..SIZE - 2 {
            for lx in 2..SIZE - 2 {
                let column = &mut ctx.columns[lx as usize][lz as usize];
                if column.biome != springs || column.height <= SEA_LEVEL {
                    continue;
                }
                let h = height(lx, lz);
                let flat = (-2..=2).all(|dz| (-2..=2).all(|dx| height(lx + dx, lz + dz) == h));
                if !flat {
                    continue;
                }
                let v = state.graph.sample(state.springs, x0 + lx, 0, z0 + lz);
                if v < 0.25 {
                    continue;
                }
                let depth = if v > 0.4 { 2 } else { 1 };
                column.height = h - depth;
                ctx.pool[lx as usize + lz as usize * CHUNK_SIZE] = h;
            }
        }
    }

    /// La surface en (x, z) sans les grottes : le zéro de la densité « sloped », cherché tous les 8
    /// blocs puis en ligne droite (exact aux coins des cellules, comme la densité interpolée).
    fn v3_surface_estimate(&self, x: i32, z: i32) -> i32 {
        let state = self.v3_state();
        let positions = (0..CY as i32)
            .rev()
            .map(|j| IVec3::new(x, BOTTOM + j * CELL_HEIGHT, z));
        let values = state.graph.sample_many(state.sloped, positions);
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
                return y_above - CELL_HEIGHT + (t * CELL_HEIGHT as f64).floor() as i32;
            }
        }
        BOTTOM
    }

    /// Le biome d'une colonne, les volcans compris : au-dessus de 12 blocs de cône, c'est le volcan.
    fn v3_biome_with_volcano(&self, p: V3Parameters, volcano: f64) -> BiomeId {
        match self.v3_state().volcano_biome {
            Some(id) if volcano > 12.0 => id,
            // Le biome le plus proche des paramètres dans `biomes_v3.ron`.
            _ => self.v3_state().biome_table.pick(p.climate()),
        }
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
        let height = self.v3_surface_estimate(x, z);
        let p = state.parameters(x, z);
        let biome = self.v3_biome_with_volcano(p, state.graph.sample(state.volcano, x, 0, z));
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
        // Les structures qui touchent ce chunk (villages, ruines, donjons : les mêmes que la v1, posées
        // sur les hauteurs v3) ; un toit peut dépasser du terrain.
        let plans = self.structure_plans(chunk_pos);
        // Les arbres dépassent la surface d'une trentaine de blocs, les toits d'une quarantaine.
        let above = if plans.is_empty() { 32 } else { 40 };
        if origin.y > ctx.max_y.max(SEA_LEVEL) + above {
            return None;
        }
        let mut chunk = Chunk::new();
        chunk.set_biomes(ctx.biomes.clone());
        self.fill_v3(&mut chunk, origin, &ctx);
        self.place_ores(&mut chunk, chunk_pos, &ctx.columns);
        if let Some(grid) = self.v3_cave_grid(origin) {
            self.v3_cave_decor(&mut chunk, origin, &ctx, &grid);
        }
        self.place_plants(&mut chunk, chunk_pos, &ctx.columns);
        self.place_trees(&mut chunk, chunk_pos);
        self.place_structures(&mut chunk, chunk_pos, &plans);
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
                let (table, (dam_low, dam_high)) =
                    ctx.water_table[lx as usize + lz as usize * CHUNK_SIZE];
                let steep = ctx.steep[lx as usize + lz as usize * CHUNK_SIZE];
                let lava = ctx.lava[lx as usize + lz as usize * CHUNK_SIZE];
                let pool = ctx.pool[lx as usize + lz as usize * CHUNK_SIZE];
                let sea = ctx.sea[lx as usize + lz as usize * CHUNK_SIZE];
                // Le bassin d'une source chaude remplace le haut du sol par de l'eau.
                let solid_at =
                    |y: i32| ctx.density(lx, y, lz) > 0.0 && !(y > column.height && y <= pool);
                // Combien de blocs pleins d'affilée au-dessus (jusqu'à l'air ou l'eau) : la surface,
                // puis la sous-couche, puis la roche. On part du haut du chunk.
                let mut solid_above = {
                    let mut n = 0;
                    let mut y = origin.y + SIZE;
                    while n < 8 && solid_at(y) && y < TOP {
                        n += 1;
                        y += 1;
                    }
                    n
                };
                for ly in (0..SIZE).rev() {
                    let y = origin.y + ly;
                    let solid = solid_at(y);
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
                                underwater: column.height < SEA_LEVEL || pool > column.height,
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
                        } else if y >= dam_low && y < dam_high {
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
                    } else if y <= lava {
                        // Le lac de lave d'un cratère.
                        solid_above = 0;
                        Some(b.lava)
                    } else if y <= pool {
                        solid_above = 0;
                        Some(b.water)
                    } else {
                        solid_above = 0;
                        (y <= SEA_LEVEL && sea).then_some(if y == SEA_LEVEL && frozen {
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

    /// Le niveau d'une région de nappe : la nappe au centre de la région, toujours au moins `TABLE_DEPTH`
    /// blocs sous le sol **le plus bas** de la région et de ses abords (la surface approchée sur une
    /// grille, plus la vraie surface en neuf points) : une vallée, un ravin ou l'ouverture d'une grotte
    /// ne voient jamais l'eau monter jusqu'à eux. `i32::MIN` si la région est sèche.
    fn v3_region_level(&self, region: (i32, i32)) -> RegionLevel {
        let state = self.v3_state();
        if let Some(&known) = state
            .regions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&region)
        {
            return known;
        }
        let h = |salt: u64| hash3(self.seed, region.0, 0, region.1, salt);
        let center = (
            region.0 * REGION + 16 + (h(80) % (REGION as u64 - 32)) as i32,
            region.1 * REGION + 16 + (h(81) % (REGION as u64 - 32)) as i32,
        );
        let table = state.graph.sample(state.water_table, center.0, 0, center.1);
        let (x0, z0) = (region.0 * REGION - 24, region.1 * REGION - 24);
        let span = REGION + 48;
        let mut ground = i32::MAX;
        for j in 0..=6 {
            for i in 0..=6 {
                let (x, z) = (x0 + i * span / 6, z0 + j * span / 6);
                ground = ground.min(self.v3_preliminary_surface(x, z));
                if i % 3 == 0 && j % 3 == 0 {
                    ground = ground
                        .min(self.v3_surface_estimate(x.div_euclid(4) * 4, z.div_euclid(4) * 4));
                }
            }
        }
        let level = (table.round() as i32).min(ground - TABLE_DEPTH);
        let level = if level < BOTTOM + 30 { i32::MIN } else { level };
        let result = (center.0, center.1, level);
        let mut cache = state.regions.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(region, result);
        result
    }

    /// La surface sans le bruit 3D : là où la pente verticale et le décalage s'annulent.
    fn v3_preliminary_surface(&self, x: i32, z: i32) -> i32 {
        let state = self.v3_state();
        64 + (128.0 * state.graph.sample(state.offset, x, 0, z)).round() as i32
    }

    /// Le niveau de nappe au point (x, z) déformé : celui de la région la plus proche (les barrages sont
    /// calculés par la colonne, en comparant ses voisines).
    fn v3_water_table(
        &self,
        known: &mut std::collections::HashMap<(i32, i32), RegionLevel>,
        x: f64,
        z: f64,
    ) -> i32 {
        let home = ((x as i32).div_euclid(REGION), (z as i32).div_euclid(REGION));
        let mut best = (f64::MAX, i32::MIN);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let key = (home.0 + dx, home.1 + dz);
                let (cx, cz, level) = *known
                    .entry(key)
                    .or_insert_with(|| self.v3_region_level(key));
                let d = (f64::from(cx) - x).powi(2) + (f64::from(cz) - z).powi(2);
                if d < best.0 {
                    best = (d, level);
                }
            }
        }
        best.1
    }

    /// Le plancher d'une structure enterrée (donjon) du site `site` : en v3, les grandes salles des
    /// grottes traverseraient une structure posée à profondeur fixe et la laisseraient flotter dans le
    /// vide. Depuis `floor` (le plancher voulu), on descend de 3 en 3 et l'on garde la hauteur où le moins
    /// de points de l'emprise (`points`, sous le plancher, à mi-hauteur et au-dessus du plafond de
    /// `height` blocs) tombent dans le vide ; la première sans aucun vide l'emporte. Le reste du vide est
    /// comblé par la structure elle-même (voir `encase`). Hors de la v3, le plancher voulu.
    pub(super) fn buried_floor(
        &self,
        site: (i32, i32),
        points: &[(i32, i32)],
        floor: i32,
        height: i32,
    ) -> i32 {
        let Some(state) = self.v3.as_ref() else {
            return floor;
        };
        if let Some(&y) = state
            .floors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&site)
        {
            return y;
        }
        let voids = |y: i32| {
            let positions = points.iter().flat_map(|&(x, z)| {
                [y - 2, y + height / 2, y + height + 1].map(|py| IVec3::new(x, py, z))
            });
            state
                .graph
                .sample_many(state.final_density, positions)
                .iter()
                .filter(|&&d| d <= 0.0)
                .count()
        };
        let mut best = (usize::MAX, floor);
        for y in ((LAVA_TOP + 6)..=floor).rev().step_by(3) {
            let n = voids(y);
            if n < best.0 {
                best = (n, y);
            }
            if n == 0 {
                break;
            }
        }
        let chosen = best.1;
        let mut cache = state.floors.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(site, chosen);
        chosen
    }

    /// Les structures enterrées de la v3 s'entourent de roche (le vide d'une grotte qui les traverse
    /// est comblé autour d'elles) ; pas en v1, dont les mondes ne changent pas.
    pub(super) fn encases_buried_structures(&self) -> bool {
        self.v3.is_some()
    }

    /// Les centres des volcans de la génération v3 dont le carré touche le rectangle `[min, max]`
    /// (vide hors de la v3).
    pub fn volcano_sites(&self, min: (i32, i32), max: (i32, i32)) -> Vec<(i32, i32)> {
        self.v3.as_ref().map_or_else(Vec::new, |state| {
            state.graph.cone_sites(state.volcano_cones, min, max)
        })
    }

    /// Les paramètres climatiques v3 en (x, z) ; `None` hors de la génération v3.
    pub fn v3_parameters(&self, x: i32, z: i32) -> Option<V3Parameters> {
        Some(self.v3.as_ref()?.parameters(x, z))
    }
}

/// Les biomes souterrains n'existent que sous ce niveau, et à plus de `CAVE_DEPTH` blocs sous la surface.
const CAVE_TOP: i32 = 30;
const CAVE_DEPTH: i32 = 16;
/// Au-dessus de ce seuil de « cave_amount », une grotte est dans un biome souterrain.
const CAVE_AMOUNT: f64 = 0.3;

/// Les biomes souterrains et leurs blocs.
#[derive(Debug, Clone, Copy)]
pub(super) struct CaveBiomes {
    crystal: BiomeId,
    glowing: BiomeId,
    fungal: BiomeId,
    stone: BlockId,
    deepstone: BlockId,
    crystal_block: BlockId,
    crystal_cluster: BlockId,
    calcite: BlockId,
    glow_moss: BlockId,
    glowcap: BlockId,
    mycelium: BlockId,
    stem: BlockId,
    cap: BlockId,
    red_mushroom: BlockId,
    brown_mushroom: BlockId,
}

impl CaveBiomes {
    fn resolve(blocks: &BlockRegistry, biomes: &BiomeRegistry) -> Option<Self> {
        let b = |key: &str| blocks.id(key);
        Some(Self {
            crystal: biomes.id("core:crystal_cavern")?,
            glowing: biomes.id("core:glowing_grotto")?,
            fungal: biomes.id("core:fungal_cave")?,
            stone: b("core:stone")?,
            deepstone: b("core:deepstone")?,
            crystal_block: b("core:crystal_block")?,
            crystal_cluster: b("core:crystal_cluster")?,
            calcite: b("core:calcite")?,
            glow_moss: b("core:glow_moss")?,
            glowcap: b("core:glowcap")?,
            mycelium: b("core:mycelium")?,
            stem: b("core:mushroom_stem")?,
            cap: b("core:mushroom_cap")?,
            red_mushroom: b("core:red_mushroom")?,
            brown_mushroom: b("core:brown_mushroom")?,
        })
    }

    /// Le biome souterrain de ces valeurs de « cave_amount » et « cave_kind ».
    fn pick(&self, amount: f64, kind: f64) -> Option<BiomeId> {
        (amount >= CAVE_AMOUNT).then_some(if kind < -0.2 {
            self.crystal
        } else if kind > 0.25 {
            self.fungal
        } else {
            self.glowing
        })
    }
}

/// « cave_amount » et « cave_kind » sur un chunk, lus tous les 8 blocs et interpolés.
pub(super) struct CaveGrid {
    values: Vec<(f64, f64)>,
}

impl CaveGrid {
    const STEP: i32 = 8;
    const N: usize = (SIZE / Self::STEP) as usize + 1;

    fn at(&self, lx: i32, ly: i32, lz: i32) -> (f64, f64) {
        let s = Self::STEP;
        let (cx, cy, cz) = ((lx / s) as usize, (ly / s) as usize, (lz / s) as usize);
        let (tx, ty, tz) = (
            f64::from(lx % s) / f64::from(s),
            f64::from(ly % s) / f64::from(s),
            f64::from(lz % s) / f64::from(s),
        );
        let n = Self::N;
        let v = |x: usize, y: usize, z: usize| self.values[x + n * (z + n * y)];
        let lerp =
            |t: f64, a: (f64, f64), b: (f64, f64)| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        let x00 = lerp(tx, v(cx, cy, cz), v(cx + 1, cy, cz));
        let x10 = lerp(tx, v(cx, cy + 1, cz), v(cx + 1, cy + 1, cz));
        let x01 = lerp(tx, v(cx, cy, cz + 1), v(cx + 1, cy, cz + 1));
        let x11 = lerp(tx, v(cx, cy + 1, cz + 1), v(cx + 1, cy + 1, cz + 1));
        lerp(tz, lerp(ty, x00, x10), lerp(ty, x01, x11))
    }
}

impl TerrainGenerator {
    /// La grille des biomes souterrains du chunk d'origine `origin`, s'il peut en avoir un.
    fn v3_cave_grid(&self, origin: IVec3) -> Option<CaveGrid> {
        let state = self.v3_state();
        state.caves?;
        if origin.y >= CAVE_TOP {
            return None;
        }
        let n = CaveGrid::N as i32;
        let positions: Vec<IVec3> = (0..n)
            .flat_map(|j| {
                (0..n).flat_map(move |k| {
                    (0..n).map(move |i| origin + IVec3::new(i, j, k) * CaveGrid::STEP)
                })
            })
            .collect();
        let amount = state
            .graph
            .sample_many(state.cave_amount, positions.iter().copied());
        if amount.iter().all(|&a| a < CAVE_AMOUNT) {
            return None;
        }
        let kind = state.graph.sample_many(state.cave_kind, positions);
        Some(CaveGrid {
            values: amount.into_iter().zip(kind).collect(),
        })
    }

    /// Le biome souterrain au bloc (x, y, z) de la génération v3, s'il y en a un (le même choix que
    /// la génération des blocs).
    pub fn v3_cave_biome(&self, x: i32, y: i32, z: i32) -> Option<BiomeId> {
        let caves = self.v3.as_ref()?.caves?;
        let origin = World::chunk_origin(IVec3::new(x, y, z).div_euclid(IVec3::splat(SIZE)));
        if y >= CAVE_TOP {
            return None;
        }
        // La grille d'abord (peu chère), la colonne (un contexte de colonne de chunks) ensuite.
        let grid = self.v3_cave_grid(origin)?;
        let (amount, kind) = grid.at(x - origin.x, y - origin.y, z - origin.z);
        let biome = caves.pick(amount, kind)?;
        (y < self.column_v3(x, z).height - CAVE_DEPTH).then_some(biome)
    }

    /// Décore les grottes des biomes souterrains du chunk : sols, plafonds et parois de roche à l'air
    /// libre, plantes et champignons géants. Le chunk porte aussi le biome (lu au milieu du chunk).
    fn v3_cave_decor(&self, chunk: &mut Chunk, origin: IVec3, ctx: &V3Column, grid: &CaveGrid) {
        let Some(c) = self.v3_state().caves else {
            return;
        };
        let size = SIZE as usize;
        let air = |chunk: &Chunk, x: i32, y: i32, z: i32| {
            (0..SIZE).contains(&x)
                && (0..SIZE).contains(&y)
                && (0..SIZE).contains(&z)
                && chunk.get(x as usize, y as usize, z as usize).is_air()
        };
        let mut biomes = ctx.biomes.clone();
        for lz in 0..SIZE {
            for lx in 0..SIZE {
                let column = ctx.columns[lx as usize][lz as usize];
                let (amount, kind) = grid.at(lx, SIZE / 2, lz);
                if origin.y + SIZE / 2 < column.height - CAVE_DEPTH
                    && let Some(id) = c.pick(amount, kind)
                {
                    biomes[lx as usize + lz as usize * size] = id.0;
                }
                for ly in 0..SIZE {
                    let (x, y, z) = (origin.x + lx, origin.y + ly, origin.z + lz);
                    if y >= CAVE_TOP || y >= column.height - CAVE_DEPTH {
                        break;
                    }
                    let here = chunk.get(lx as usize, ly as usize, lz as usize);
                    if here != c.stone && here != c.deepstone {
                        continue;
                    }
                    let (amount, kind) = grid.at(lx, ly, lz);
                    let Some(biome) = c.pick(amount, kind) else {
                        continue;
                    };
                    let up = air(chunk, lx, ly + 1, lz);
                    let down = air(chunk, lx, ly - 1, lz);
                    let side = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .any(|&(dx, dz)| air(chunk, lx + dx, ly, lz + dz));
                    if !(up || down || side) {
                        continue;
                    }
                    let roll = |salt: u64| (hash3(self.seed, x, y, z, salt) % 1000) as f32 / 1000.0;
                    let mut set = |dy: i32, block: BlockId| {
                        chunk.set(lx as usize, (ly + dy) as usize, lz as usize, block);
                    };
                    if biome == c.crystal {
                        if up {
                            if roll(700) < 0.08 {
                                set(0, c.crystal_block);
                            }
                            if roll(701) < 0.12 {
                                set(1, c.crystal_cluster);
                            }
                        } else if down {
                            if roll(700) < 0.1 {
                                set(0, c.crystal_block);
                            } else if roll(701) < 0.16 {
                                set(-1, c.crystal_cluster);
                            }
                        } else if hash3(
                            self.seed,
                            x.div_euclid(3),
                            y.div_euclid(3),
                            z.div_euclid(3),
                            702,
                        )
                        .is_multiple_of(7)
                        {
                            set(0, c.calcite);
                        } else if roll(700) < 0.05 {
                            set(0, c.crystal_block);
                        }
                    } else if biome == c.glowing {
                        let moss = if up {
                            0.75
                        } else if down {
                            0.35
                        } else {
                            0.25
                        };
                        if roll(710) < moss {
                            set(0, c.glow_moss);
                        }
                        if up && roll(711) < 0.1 {
                            set(1, c.glowcap);
                        }
                    } else if up {
                        // La grotte fongique : un tapis de mycélium, des champignons, quelques géants.
                        if roll(720) < 0.85 {
                            set(0, c.mycelium);
                        }
                        let r = roll(721);
                        if r < 0.03 {
                            self.v3_giant_mushroom(chunk, lx, ly + 1, lz, &c);
                        } else if r < 0.1 {
                            set(
                                1,
                                if r < 0.05 {
                                    c.red_mushroom
                                } else {
                                    c.brown_mushroom
                                },
                            );
                        }
                    }
                }
            }
        }
        chunk.set_biomes(biomes);
    }

    /// Un champignon géant posé en (lx, ly, lz) du chunk : un pied de 4 à 6 blocs, un chapeau plat de
    /// rayon 2 surmonté d'une couche de rayon 1. Seulement s'il tient dans le chunk et dans le vide.
    fn v3_giant_mushroom(&self, chunk: &mut Chunk, lx: i32, ly: i32, lz: i32, c: &CaveBiomes) {
        let height = 4 + (hash3(self.seed, lx, ly, lz, 722) % 3) as i32;
        let mut blocks = Vec::new();
        for k in 0..height {
            blocks.push((IVec3::new(lx, ly + k, lz), c.stem));
        }
        for (dy, r) in [(height, 2i32), (height + 1, 1)] {
            for dz in -r..=r {
                for dx in -r..=r {
                    if r == 2 && dx.abs() == 2 && dz.abs() == 2 {
                        continue;
                    }
                    blocks.push((IVec3::new(lx + dx, ly + dy, lz + dz), c.cap));
                }
            }
        }
        let fits = blocks.iter().all(|(p, _)| {
            p.cmpge(IVec3::ZERO).all()
                && p.cmplt(IVec3::splat(SIZE)).all()
                && chunk.get(p.x as usize, p.y as usize, p.z as usize).is_air()
        });
        if fits {
            for (p, block) in blocks {
                chunk.set(p.x as usize, p.y as usize, p.z as usize, block);
            }
        }
    }
}

/// Sous ce niveau, le vide souterrain est de la lave (le fond du monde).
const LAVA_TOP: i32 = BOTTOM + 20;
/// La taille d'une région de nappe (en blocs) : le niveau d'eau y est plat.
const REGION: i32 = 96;
/// Le centre d'une région de nappe (x, z) et son niveau (`i32::MIN` : sèche).
type RegionLevel = (i32, i32, i32);

/// La portée des berges entre deux nappes (en blocs) et leur pente (blocs de descente par bloc) : une
/// berge descend jusqu'à 48 blocs sous la nappe haute ; plus loin, le bas d'une très haute retenue reste
/// un mur.
const BANK_REACH: i32 = 24;
const BANK_SLOPE: i32 = 2;
/// La nappe d'une région reste au moins à cette profondeur sous le sol le plus bas de la région.
const TABLE_DEPTH: i32 = 24;
