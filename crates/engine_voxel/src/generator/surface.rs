//! Les matériaux du générateur v2 : surface selon la pente, l'altitude, la température, l'exposition
//! et la proximité de l'eau, et strates régionales continues. Uniquement des blocs existants.
//!
//! # Règles de surface (par colonne, voir [`TerrainGenerator::surface_rule`])
//!
//! Dans l'ordre, la première qui s'applique décide :
//!
//! 1. **lit d'un cours ou d'un lac** : gravier en montagne (au-dessus de `SEA + 45`), sable dans un
//!    grand cours (largeur de 14 blocs et plus), argile par taches, sable ou argile au fond d'un lac ;
//! 2. **fond marin** : le bloc « sous l'eau » du biome (comme la v1) ;
//! 3. **falaise** : pente de 1,0 blocs par bloc et plus, ou marche de 3 blocs : la roche nue, sur
//!    quatre blocs ;
//! 4. **éboulis** : pente de 0,75 à 1,0 (ou marche de 2) : du gravier en altitude (au-dessus de
//!    `SEA + 35`) par taches, sinon de la terre grossière (ou la roche sous un sable) ;
//! 5. **neige** : température ressentie sous 0,15 (celle du biome, moins le froid de l'altitude et
//!    l'ombre des pentes exposées au pôle), sauf sous un sable ; la neige ne tient pas dans les pentes
//!    raides (les règles 3 et 4 l'ont déjà prise) ;
//! 6. **berges** : à deux blocs au plus de l'eau d'un cours ou d'un lac, et à moins de trois blocs de
//!    son niveau : du gravier en altitude, du sable au bord d'un grand cours ou d'un lac, de l'argile
//!    par taches, sinon le sol du biome ;
//! 7. **roche affleurante** (`rock_above` du biome) ;
//! 8. sinon le sol du biome (surface, taches de sol, sous-sol, strates de sous-sol), comme en v1.
//!
//! La pente est celle de la hauteur de surface (différence sur ±2 blocs), la même pour les cartes,
//! les surplombs et les tests : **pas d'herbe sur une pente de plus de 0,75**.
//!
//! # Strates régionales
//!
//! La roche se range en couches de 3 blocs dont l'altitude ondule **continûment** (deux bruits de
//! valeurs, 110 et 29 blocs) ; la couche `k` est de la pierre, du grès ou de la roche volcanique selon
//! un tirage par couche et deux champs régionaux lents (700 et 900 blocs) qui dosent la part de grès
//! (régions sédimentaires) et de roche volcanique. Les couches se suivent donc d'un chunk à l'autre
//! et changent lentement de région en région : une falaise a son identité (bandes de grès, filons
//! sombres). Les parois ont aussi une *saillie* de couche par altitude ([`StrataColumn::ledge`]) : dans
//! l'entaille du pied d'une paroi (`density.rs`), certaines couches reculent, d'autres saillent.
//!
//! Elles sont posées **après** les minerais (les filons ne remplacent que la pierre et la pierre
//! profonde, comme en v1) et ne touchent ni la pierre profonde, ni l'eau, ni le sol.

use super::v2::ColumnContext;
use super::{DEEPSTONE_LEVEL, FREEZING, SEA_LEVEL, TerrainGenerator, value_noise};
use crate::biome::BiomeId;
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use glam::IVec3;

/// Une matière de surface : un bloc, ou « la roche du lieu » (pierre, puis strates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mat {
    Block(BlockId),
    Rock,
}

/// Les matériaux d'une colonne : le bloc de surface, celui des couches suivantes, et jusqu'où le
/// sol descend (la roche commence à `sub_depth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SurfaceRule {
    pub top: Mat,
    pub sub: Mat,
    pub sub_depth: u8,
    /// Les couches de sous-sol suivent les strates horizontales du biome (plateaux rouges).
    pub banded: bool,
}

/// Ce que décide une règle de surface.
pub(super) struct SurfaceInputs {
    pub x: i32,
    pub z: i32,
    pub biome: BiomeId,
    /// y du dernier bloc du relief.
    pub top: i32,
    pub slope: f32,
    /// La plus grande marche (blocs) vers une des quatre colonnes voisines.
    pub step: i32,
    /// La température ressentie (celle du biome, moins l'altitude et l'ombre).
    pub temperature: f32,
    /// Drapeaux de lit (`v2::flag`).
    pub bed: u8,
    pub river_width: f32,
    /// Niveau de l'eau d'un cours ou lac à deux blocs au plus, hors de la colonne.
    pub bank: Option<i32>,
    /// La rive est celle d'un grand cours ou d'un lac.
    pub bank_wide: bool,
}

/// Pente à partir de laquelle la roche est nue, et celle de l'éboulis.
pub(super) const CLIFF_SLOPE: f32 = 1.0;
pub(super) const SCREE_SLOPE: f32 = 0.75;

impl TerrainGenerator {
    /// La règle de surface d'une colonne (voir la documentation du module).
    pub(super) fn surface_rule(&self, i: &SurfaceInputs) -> SurfaceRule {
        let v = self.v2_blocks();
        let b = self.blocks;
        let biome = self.biomes.get(i.biome);
        let uniform = |block: BlockId, depth: u8| SurfaceRule {
            top: Mat::Block(block),
            sub: Mat::Block(block),
            sub_depth: depth,
            banded: false,
        };
        let rock = SurfaceRule {
            top: Mat::Rock,
            sub: Mat::Rock,
            sub_depth: 0,
            banded: false,
        };
        let patch = |cell: i32, salt: u64| value_noise(self.seed, i.x, i.z, cell, salt);
        // 1. Le lit.
        if i.bed & super::v2::flag::BED_RIVER != 0 {
            let block = if i.top > SEA_LEVEL + 45 {
                v.gravel
            } else if i.river_width >= 14.0 {
                if patch(9, 80) > 0.72 { v.clay } else { b.sand }
            } else if patch(7, 81) > 0.75 {
                b.sand
            } else if i.top > SEA_LEVEL + 25 {
                v.gravel
            } else {
                v.clay
            };
            return uniform(block, 3);
        }
        if i.bed & super::v2::flag::BED_LAKE != 0 {
            return uniform(if patch(11, 82) > 0.55 { v.clay } else { b.sand }, 3);
        }
        // 2. Le fond marin.
        if i.top < SEA_LEVEL {
            return uniform(biome.underwater, biome.subsurface_depth.clamp(1, 8) as u8);
        }
        let soil_like = |block: BlockId| {
            let p = self.plant_blocks;
            p.is_soil(block)
        };
        // 3. Falaise.
        if i.slope >= CLIFF_SLOPE || i.step >= 3 {
            return SurfaceRule {
                top: Mat::Rock,
                sub: Mat::Rock,
                sub_depth: 4,
                banded: false,
            };
        }
        // 4. Éboulis.
        if i.slope >= SCREE_SLOPE || i.step >= 2 {
            if i.top > SEA_LEVEL + 35 && patch(5, 83) > 0.4 {
                return uniform(v.gravel, 2);
            }
            return if soil_like(biome.surface) {
                SurfaceRule {
                    top: Mat::Block(v.coarse_dirt),
                    sub: Mat::Block(biome.subsurface),
                    sub_depth: 3,
                    banded: false,
                }
            } else {
                rock
            };
        }
        let sub_of = |sub: BlockId| SurfaceRule {
            top: Mat::Block(sub),
            sub: Mat::Block(sub),
            sub_depth: 3,
            banded: false,
        };
        // 5. Neige.
        if i.temperature < FREEZING && biome.surface != b.sand {
            return SurfaceRule {
                top: Mat::Block(b.snow),
                sub: Mat::Block(biome.subsurface),
                sub_depth: biome.subsurface_depth.clamp(1, 12) as u8,
                banded: false,
            };
        }
        // 6. Berges.
        if let Some(level) = i.bank
            && (i.top - level).abs() <= 2
        {
            if i.top > SEA_LEVEL + 35 {
                return sub_of(v.gravel);
            }
            if i.bank_wide && patch(6, 84) > 0.25 {
                return sub_of(b.sand);
            }
            if patch(8, 85) > 0.7 {
                return sub_of(v.clay);
            }
            if patch(5, 86) > 0.8 {
                return sub_of(v.gravel);
            }
        }
        // 7. La roche affleurante des montagnes.
        if biome.rock_above.is_some_and(|h| i.top > h) {
            return rock;
        }
        // 8. Le sol du biome.
        SurfaceRule {
            top: Mat::Block(self.surface_patch(biome, i.x, i.z).unwrap_or(biome.surface)),
            sub: Mat::Block(biome.subsurface),
            sub_depth: biome.subsurface_depth.clamp(1, 255) as u8,
            banded: !biome.bands.is_empty(),
        }
    }

    /// Le bloc d'un solide à `depth` blocs sous l'air, selon la règle ; `None` : la roche.
    pub(super) fn rule_block(
        &self,
        rule: &SurfaceRule,
        biome: BiomeId,
        x: i32,
        y: i32,
        z: i32,
        depth: i32,
    ) -> Option<BlockId> {
        let mat = if depth == 0 {
            rule.top
        } else if depth < rule.sub_depth as i32 {
            if rule.banded && depth > 1 {
                return Some(self.band(self.biomes.get(biome), x, y, z));
            }
            rule.sub
        } else {
            return None;
        };
        match mat {
            Mat::Block(block) => Some(block),
            Mat::Rock => None,
        }
    }

    /// Le bloc du sol d'une colonne, pour poser un arbre ou une plante.
    pub(super) fn surface_block_of(&self, core: &super::v2::CoreCell, x: i32, z: i32) -> BlockId {
        let _ = (x, z);
        match core.rule.top {
            Mat::Block(block) => block,
            Mat::Rock => self.blocks.stone,
        }
    }
}

// ---------- strates ----------

/// Ce qu'une strate est.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stratum {
    Stone,
    Sandstone,
    Volcanic,
}

/// Les strates d'une colonne (x, z) : l'ondulation continue des couches et les doses régionales.
#[derive(Debug, Clone, Copy)]
pub(super) struct StrataColumn {
    seed: u32,
    wave: f32,
    /// L'éventail : les couches s'écartent ou se resserrent avec la profondeur, de sorte que
    /// leurs marches d'un bloc ne tombent pas toutes à la même abscisse (sinon une coupe montre
    /// de fausses failles verticales là où toutes les couches sautent ensemble).
    fan: f32,
    sandstone: f32,
    volcanic: f32,
    sandstone_block: BlockId,
    volcanic_block: BlockId,
}

/// Épaisseur d'une couche, en blocs.
const LAYER: f32 = 3.0;

/// L'altitude autour de laquelle l'éventail des couches pivote.
const FAN_PIVOT: i32 = 96;

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl TerrainGenerator {
    /// Les strates de la colonne (x, z).
    pub(super) fn strata_column(&self, x: i32, z: i32) -> StrataColumn {
        let v = self.v2_blocks();
        let s = self.seed;
        StrataColumn {
            seed: s,
            wave: (value_noise(s, x, z, 110, 62) - 0.5) * 16.0
                + (value_noise(s, x, z, 29, 63) - 0.5) * 6.0,
            fan: (value_noise(s, x, z, 83, 65) - 0.5) * 0.5,
            sandstone: 0.5 * smooth(0.55, 0.85, value_noise(s, x, z, 700, 60)),
            volcanic: 0.3 * smooth(0.65, 0.9, value_noise(s, x, z, 900, 61)),
            sandstone_block: v.sandstone,
            volcanic_block: v.volcanic_rock,
        }
    }
}

impl StrataColumn {
    /// Aucune couche de cette colonne n'est autre chose que de la pierre.
    pub fn is_plain(&self) -> bool {
        self.sandstone <= 0.0 && self.volcanic <= 0.0
    }

    /// La strate d'altitude `y`.
    pub fn stratum(&self, y: i32) -> Stratum {
        if self.is_plain() {
            return Stratum::Stone;
        }
        // `fan` reste dans ±0,25 : la position dans la pile croît toujours avec `y`, chaque
        // couche reste continue et d'un seul tenant dans une colonne.
        let depth = (y - FAN_PIVOT) as f32;
        let k = ((y as f32 + self.wave + self.fan * depth) / LAYER).floor() as i32;
        let u = (super::hash3(self.seed, 0, k, 0, 64) >> 40) as f32 / (1u64 << 24) as f32;
        if u < self.sandstone {
            Stratum::Sandstone
        } else if u > 1.0 - self.volcanic {
            Stratum::Volcanic
        } else {
            Stratum::Stone
        }
    }

    /// La saillie des couches d'altitude `y`, de -1 (une couche tendre qui recule) à 1 (une couche
    /// dure qui saille) : la *résistance apparente* de la roche dans l'entaille du pied des parois
    /// (`density.rs`). Elle ne dépend que de l'altitude (les couches d'une paroi suivent des
    /// niveaux, pas la colonne) : deux colonnes voisines reculent de la même façon à la même hauteur,
    /// ce qui garde le toit d'une entaille relié à la masse.
    pub fn ledge(&self, y: i32) -> f32 {
        let u = (super::hash3(self.seed, 0, y.div_euclid(LAYER as i32), 0, 72) >> 40) as f32
            / (1u64 << 24) as f32;
        2.0 * u - 1.0
    }

    fn block(&self, y: i32) -> Option<BlockId> {
        match self.stratum(y) {
            Stratum::Stone => None,
            Stratum::Sandstone => Some(self.sandstone_block),
            Stratum::Volcanic => Some(self.volcanic_block),
        }
    }
}

impl TerrainGenerator {
    /// Remplace, dans un chunk fini (minerais posés), la pierre par la strate de son altitude.
    pub(super) fn apply_strata(&self, chunk: &mut Chunk, origin: IVec3, ctx: &ColumnContext) {
        let _ = ctx;
        let stone = self.blocks.stone;
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let strata = self.strata_column(origin.x + lx as i32, origin.z + lz as i32);
                if strata.is_plain() {
                    continue;
                }
                for ly in 0..CHUNK_SIZE {
                    let y = origin.y + ly as i32;
                    if y < DEEPSTONE_LEVEL + 4 || chunk.get(lx, ly, lz) != stone {
                        continue;
                    }
                    if let Some(block) = strata.block(y) {
                        chunk.set(lx, ly, lz, block);
                    }
                }
            }
        }
    }
}
