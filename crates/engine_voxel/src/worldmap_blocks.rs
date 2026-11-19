//! Les cartes et les coupes issues des **blocs** d'un générateur (utile surtout pour la v2) : la
//! hauteur réelle de la surface, son matériau, l'eau, les surplombs, et des coupes verticales
//! (profil d'une ligne en x ou en z, un bloc par pixel agrandi).
//!
//! Contrairement à [`crate::MacroMapSamples`] (le relief macro avant tout bloc), tout vient ici de
//! [`TerrainGenerator::generate`] : ce que le joueur verrait. La commande est
//! `cargo xtask map --generator v2 --maps hauteur-blocs,materiau,...` et
//! `cargo xtask map --generator v2 --section z=0 --from -256 --to 256` (voir
//! `crates/engine_voxel/examples/worldmap.rs`).

use glam::IVec3;
use rayon::prelude::*;

use crate::block::BlockId;
use crate::chunk::CHUNK_SIZE;
use crate::generator::{SEA_LEVEL, TerrainGenerator};
use crate::worldmap::{MapArea, Raster, altitude_color};

/// Les chunks (de bas en haut, sur `TerrainGenerator::chunk_range`) d'une colonne de chunks.
type ColumnChunks = Vec<Option<crate::chunk::Chunk>>;

/// Ce que montre une carte de blocs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMapKind {
    /// La hauteur du sol réel (bloc plein le plus haut, feuillage et eau exclus), ombrée.
    Height,
    /// Le matériau du sol (couleur du bloc), teinté de vert sous un arbre.
    Material,
    /// L'eau : mer (bleu), cours et lacs (cyan), plus foncé selon la profondeur ; la terre en gris.
    Water,
    /// Les vides sous la peau (surplombs, poches, entrées) : nombre de blocs vides dans les 14
    /// blocs sous le sol, de noir (aucun) à blanc (beaucoup).
    Overhangs,
    /// Les parois : le dénivelé du sol sur un pixel (blocs, de rouge sombre à jaune de 8 à 40 blocs),
    /// et en bleu les colonnes dont le sol réel recouvre du vide ouvert sur le côté (surplombs). Les
    /// autres pixels sont en gris ombré. À lire avec `--step 1` ou `2` : au-delà, un pixel saute les parois.
    Walls,
}

impl BlockMapKind {
    pub const ALL: [BlockMapKind; 5] = [
        BlockMapKind::Height,
        BlockMapKind::Material,
        BlockMapKind::Water,
        BlockMapKind::Overhangs,
        BlockMapKind::Walls,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Height => "hauteur-blocs",
            Self::Material => "materiau",
            Self::Water => "eau-blocs",
            Self::Overhangs => "surplombs",
            Self::Walls => "parois",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Ce que les blocs disent d'une colonne.
#[derive(Debug, Clone, Copy)]
struct Pixel {
    /// y du bloc plein le plus haut hors feuillage et bois, -1 s'il n'y en a pas.
    ground: i32,
    ground_block: BlockId,
    /// y du dernier bloc d'eau (le premier liquide rencontré en descendant), -1 sans eau.
    water: i32,
    canopy: bool,
    /// Blocs vides dans les 14 blocs sous le sol.
    hollow: u8,
}

/// Les colonnes d'une zone, lues dans les blocs générés (avec une bordure d'un pixel).
pub struct BlockMapSamples<'a> {
    generator: &'a TerrainGenerator,
    area: MapArea,
    pixels: Vec<Pixel>,
}

impl<'a> BlockMapSamples<'a> {
    /// Génère les colonnes de chunks nécessaires (en parallèle) et lit chaque pixel.
    pub fn compute(generator: &'a TerrainGenerator, area: MapArea) -> Self {
        let step = area.step.max(1) as i32;
        let (w, h) = (area.width as i32 + 2, area.height as i32 + 2);
        let coord = |i: i32, origin: i32| origin + (i - 1) * step;
        // Les colonnes de chunks qui contiennent au moins un pixel.
        let size = CHUNK_SIZE as i32;
        let mut wanted: Vec<(i32, i32)> = (0..h)
            .flat_map(|j| (0..w).map(move |i| (i, j)))
            .map(|(i, j)| {
                (
                    coord(i, area.x).div_euclid(size),
                    coord(j, area.z).div_euclid(size),
                )
            })
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let generated: Vec<((i32, i32), ColumnChunks)> = wanted
            .par_iter()
            .map(|&(cx, cz)| {
                let chunks = generator
                    .chunk_range()
                    .map(|cy| generator.generate(IVec3::new(cx, cy, cz)))
                    .collect();
                ((cx, cz), chunks)
            })
            .collect();
        let (bottom, top) = (generator.chunk_range().start, generator.chunk_range().end);
        let by_column: std::collections::HashMap<(i32, i32), &ColumnChunks> =
            generated.iter().map(|(k, v)| (*k, v)).collect();
        let registry = generator.registry();
        let skipped = |block: BlockId| {
            let key = &registry.def(block).key;
            key.ends_with("_leaves") || key.ends_with("_log") || key == "core:cactus"
        };
        let pixels = (0..w * h)
            .into_par_iter()
            .map(|n| {
                let (x, z) = (coord(n % w, area.x), coord(n / w, area.z));
                let chunks = by_column[&(x.div_euclid(size), z.div_euclid(size))];
                let (lx, lz) = (x.rem_euclid(size) as usize, z.rem_euclid(size) as usize);
                let block_at = |y: i32| -> BlockId {
                    chunks[(y.div_euclid(size) - bottom) as usize]
                        .as_ref()
                        .map_or(BlockId::AIR, |c| c.get(lx, y.rem_euclid(size) as usize, lz))
                };
                let mut pixel = Pixel {
                    ground: -1,
                    ground_block: BlockId::AIR,
                    water: -1,
                    canopy: false,
                    hollow: 0,
                };
                for y in (bottom * size..top * size).rev() {
                    let block = block_at(y);
                    if block.is_air() {
                        continue;
                    }
                    let def = registry.def(block);
                    if def.liquid {
                        if pixel.water < 0 {
                            pixel.water = y;
                        }
                        continue;
                    }
                    if !def.solid {
                        continue;
                    }
                    if skipped(block) {
                        pixel.canopy = true;
                        continue;
                    }
                    pixel.ground = y;
                    pixel.ground_block = block;
                    break;
                }
                if pixel.ground > 0 {
                    pixel.hollow = (pixel.ground - 14..pixel.ground)
                        .filter(|&y| y > 0 && block_at(y).is_air())
                        .count() as u8;
                }
                pixel
            })
            .collect();
        Self {
            generator,
            area,
            pixels,
        }
    }

    fn at(&self, px: i32, pz: i32) -> &Pixel {
        let w = self.area.width as usize + 2;
        &self.pixels[(pz + 1) as usize * w + (px + 1) as usize]
    }

    pub fn render(&self, kind: BlockMapKind) -> Raster {
        let (width, height) = (self.area.width, self.area.height);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pz in 0..height as i32 {
            for px in 0..width as i32 {
                rgb.extend_from_slice(&self.color(kind, px, pz));
            }
        }
        Raster { width, height, rgb }
    }

    /// Les hauteurs de sol voisines servent à l'ombrage.
    fn shade(&self, px: i32, pz: i32) -> f32 {
        let step = self.area.step.max(1) as f32;
        let h = |dx: i32, dz: i32| self.at(px + dx, pz + dz).ground.max(0) as f32;
        let (dx, dz) = (
            (h(1, 0) - h(-1, 0)) / (2.0 * step),
            (h(0, 1) - h(0, -1)) / (2.0 * step),
        );
        // Lumière venue du nord-ouest.
        (-(dx + dz) * 0.6).clamp(-1.0, 1.0)
    }

    fn color(&self, kind: BlockMapKind, px: i32, pz: i32) -> [u8; 3] {
        let p = self.at(px, pz);
        match kind {
            BlockMapKind::Height => {
                if p.water >= 0 {
                    let depth = (p.water - p.ground.max(0)).clamp(0, 40) as f32;
                    let k = 1.0 - depth / 60.0;
                    let base: [f32; 3] = if p.water == SEA_LEVEL {
                        [40.0, 100.0, 200.0]
                    } else {
                        [90.0, 190.0, 235.0]
                    };
                    return base.map(|c| (c * k) as u8);
                }
                altitude_color(p.ground, self.shade(px, pz))
            }
            BlockMapKind::Material => {
                if p.water >= 0 {
                    let depth = (p.water - p.ground.max(0)).clamp(0, 40) as f32;
                    let base = self.block_color(p.ground_block);
                    let k = (0.55 - depth / 100.0).clamp(0.15, 0.55);
                    let water = [40.0, 90.0, 200.0];
                    return std::array::from_fn(|c| {
                        (base[c] as f32 * (1.0 - k) + water[c] * k) as u8
                    });
                }
                let base = self.block_color(p.ground_block);
                let shade = 1.0 + 0.3 * self.shade(px, pz);
                let tint = if p.canopy { [0.55, 0.9, 0.5] } else { [1.0; 3] };
                std::array::from_fn(|c| (base[c] as f32 * shade * tint[c]).clamp(0.0, 255.0) as u8)
            }
            BlockMapKind::Water => {
                if p.water < 0 {
                    let g = (60 + p.ground.max(0) / 2).min(200) as u8;
                    return [g, g, g];
                }
                let depth = (p.water - p.ground.max(0)).clamp(0, 30) as f32;
                let k = 1.0 - 0.7 * depth / 30.0;
                let base: [f32; 3] = if p.water == SEA_LEVEL {
                    [40.0, 110.0, 230.0]
                } else {
                    [60.0, 230.0, 220.0]
                };
                base.map(|c| (c * k) as u8)
            }
            BlockMapKind::Overhangs => {
                let g = (p.hollow as f32 / 14.0 * 255.0) as u8;
                [g, g, g]
            }
            BlockMapKind::Walls => {
                if p.water >= 0 {
                    return [30, 50, 90];
                }
                // Les voisins sous l'eau ne comptent pas : un fond marin n'est pas le pied d'une paroi.
                let drop = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .map(|&(dx, dz)| self.at(px + dx, pz + dz))
                    .filter(|n| n.water < 0)
                    .map(|n| p.ground - n.ground)
                    .max()
                    .unwrap_or(0);
                // Un vide ouvert sur le côté sous le sol : au moins un pixel voisin plus bas que
                // le vide, et du vide dans les 14 blocs sous le sol.
                if p.hollow > 0 && drop >= 3 {
                    return [60, 140, 255];
                }
                if drop >= 8 {
                    let k = ((drop - 8) as f32 / 32.0).clamp(0.0, 1.0);
                    return [(150.0 + 105.0 * k) as u8, (30.0 + 200.0 * k) as u8, 20];
                }
                let g = (70.0 + 70.0 * (1.0 + self.shade(px, pz)) * 0.5) as u8;
                [g, g, g]
            }
        }
    }

    fn block_color(&self, block: BlockId) -> [u8; 3] {
        self.generator
            .registry()
            .def(block)
            .color
            .map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8)
    }
}

/// L'axe d'une coupe verticale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionAxis {
    /// Le profil suit x, à z fixé.
    X,
    /// Le profil suit z, à x fixé.
    Z,
}

/// Une coupe verticale : le profil d'une ligne de `from` à `to` (exclu) à `fixed` sur l'autre axe,
/// de `y_min` à `y_max` (exclu). Un bloc fait `scale` pixels de côté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSpec {
    pub axis: SectionAxis,
    pub fixed: i32,
    pub from: i32,
    pub to: i32,
    pub y_min: i32,
    pub y_max: i32,
    pub scale: u32,
}

/// Dessine une coupe : les blocs ont leur couleur, l'eau est teintée de bleu, le vide est un ciel
/// dégradé ; le niveau de la mer est assombri dans l'air.
pub fn render_section(generator: &TerrainGenerator, spec: &SectionSpec) -> Raster {
    let SectionSpec {
        axis,
        fixed,
        from,
        to,
        y_min,
        y_max,
        scale,
    } = *spec;
    let size = CHUNK_SIZE as i32;
    let registry = generator.registry();
    let (bottom, top) = (generator.chunk_range().start, generator.chunk_range().end);
    let (y_min, y_max) = (
        y_min.clamp(bottom * size, top * size - 1),
        y_max.clamp(bottom * size + 1, top * size),
    );
    // Les colonnes de chunks traversées.
    let along: Vec<i32> = (from.div_euclid(size)..=(to - 1).div_euclid(size)).collect();
    let across = fixed.div_euclid(size);
    let chunks: std::collections::HashMap<i32, Vec<Option<crate::chunk::Chunk>>> = along
        .par_iter()
        .map(|&c| {
            let (cx, cz) = match axis {
                SectionAxis::X => (c, across),
                SectionAxis::Z => (across, c),
            };
            (
                c,
                generator
                    .chunk_range()
                    .map(|cy| generator.generate(IVec3::new(cx, cy, cz)))
                    .collect(),
            )
        })
        .collect();
    let length = (to - from) as u32;
    let rows = (y_max - y_min) as u32;
    let (width, height) = (length * scale, rows * scale);
    let mut rgb = vec![0u8; (width * height * 3) as usize];
    let color = |block: BlockId| -> [u8; 3] {
        registry
            .def(block)
            .color
            .map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8)
    };
    for i in 0..length as i32 {
        let t = from + i;
        let column = &chunks[&t.div_euclid(size)];
        let local = t.rem_euclid(size) as usize;
        let lfix = fixed.rem_euclid(size) as usize;
        for row in 0..rows as i32 {
            let y = y_max - 1 - row;
            let block = column[(y.div_euclid(size) - bottom) as usize]
                .as_ref()
                .map_or(BlockId::AIR, |c| {
                    let (lx, lz) = match axis {
                        SectionAxis::X => (local, lfix),
                        SectionAxis::Z => (lfix, local),
                    };
                    c.get(lx, y.rem_euclid(size) as usize, lz)
                });
            let def = registry.def(block);
            let mut px: [u8; 3] = if block.is_air() {
                // Un ciel qui s'éclaircit vers le bas.
                let k = row as f32 / rows as f32;
                [
                    (150.0 + 70.0 * k) as u8,
                    (190.0 + 40.0 * k) as u8,
                    (235.0 + 15.0 * k) as u8,
                ]
            } else if def.liquid {
                let c = color(block);
                [c[0] / 2, c[1] / 2 + 40, (c[2] as u16 + 120).min(255) as u8]
            } else {
                color(block)
            };
            // Le niveau de la mer et les multiples de 32 (limites de chunks) : repères discrets.
            if y == SEA_LEVEL && block.is_air() {
                px = [px[0] / 2, px[1] / 2, px[2]];
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let o = (((row as u32 * scale + dy) * width + (i as u32 * scale + dx)) * 3)
                        as usize;
                    rgb[o..o + 3].copy_from_slice(&px);
                }
            }
        }
    }
    Raster { width, height, rgb }
}
