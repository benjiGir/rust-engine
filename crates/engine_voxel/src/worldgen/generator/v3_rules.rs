//! Les données de choix de la génération v3 (lot 4 du plan) : les **biomes** par paramètres
//! (`assets/core/worldgen/biomes_v3.ron`) et les **règles de surface** (`surface_v3.ron`).
//!
//! - [`BiomeTable`] : chaque biome occupe une case dans l'espace (température, humidité,
//!   continentalité, érosion, pics et vallées, variété) ; une colonne prend le biome le plus proche (distance
//!   nulle dans la case, carré de l'écart dehors, plus un petit décalage propre au biome). C'est le
//!   choix « multi-bruits » de Minecraft moderne, avec nos cases.
//! - [`SurfaceRules`] : une liste ordonnée de règles (conditions, matériau) ; pour un bloc du sol du
//!   dessus, la première règle dont toutes les conditions tiennent donne le matériau.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;
use crate::registry::BlockRegistry;
use crate::worldgen::biome::{BiomeId, BiomeRegistry};
use crate::worldgen::density_graph::noise::{NoiseDef, NormalNoise};

fn any() -> (f64, f64) {
    (-2.0, 2.0)
}

#[derive(Debug, Clone, Deserialize)]
struct BiomeEntry {
    biome: String,
    #[serde(default = "any")]
    temperature: (f64, f64),
    #[serde(default = "any")]
    humidity: (f64, f64),
    #[serde(default = "any")]
    continentalness: (f64, f64),
    #[serde(default = "any")]
    erosion: (f64, f64),
    #[serde(default = "any")]
    pv: (f64, f64),
    #[serde(default = "any")]
    variety: (f64, f64),
    #[serde(default)]
    offset: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct BiomeFile {
    biomes: Vec<BiomeEntry>,
}

/// Les paramètres d'une colonne, dans l'ordre de la table.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Climate6 {
    pub temperature: f64,
    pub humidity: f64,
    pub continentalness: f64,
    pub erosion: f64,
    pub pv: f64,
    /// La variante d'un climat, à large échelle et indépendante du relief.
    pub variety: f64,
}

/// Une case de la table : le biome, ses plages (dans l'ordre de [`Climate6`]) et son décalage.
type Cell = (BiomeId, [(f64, f64); 6], f64);

/// Le choix des biomes par paramètres.
#[derive(Debug, Clone)]
pub(super) struct BiomeTable {
    entries: Vec<Cell>,
}

impl BiomeTable {
    pub(super) fn parse(text: &str, biomes: &BiomeRegistry) -> Result<Self> {
        let file: BiomeFile = ron::from_str(text).context("biomes_v3.ron illisible")?;
        let mut entries = Vec::new();
        for e in file.biomes {
            // Un biome d'un mod absent est ignoré (la table du jeu de base reste valable).
            let Some(id) = biomes.id(&e.biome) else {
                log::warn!("biomes v3 : biome « {} » inconnu, ignoré", e.biome);
                continue;
            };
            entries.push((
                id,
                [
                    e.temperature,
                    e.humidity,
                    e.continentalness,
                    e.erosion,
                    e.pv,
                    e.variety,
                ],
                e.offset,
            ));
        }
        if entries.is_empty() {
            bail!("biomes v3 : aucun biome connu");
        }
        Ok(Self { entries })
    }

    /// Le biome le plus proche des paramètres.
    pub(super) fn pick(&self, p: Climate6) -> BiomeId {
        let values = [
            p.temperature,
            p.humidity,
            p.continentalness,
            p.erosion,
            p.pv,
            p.variety,
        ];
        let distance = |ranges: &[(f64, f64); 6], offset: f64| {
            let mut d = offset * offset;
            for (v, (lo, hi)) in values.iter().zip(ranges) {
                let gap = if v < lo {
                    lo - v
                } else if v > hi {
                    v - hi
                } else {
                    0.0
                };
                d += gap * gap;
            }
            d
        };
        self.entries
            .iter()
            .min_by(|a, b| distance(&a.1, a.2).total_cmp(&distance(&b.1, b.2)))
            .map(|e| e.0)
            .expect("table non vide")
    }
}

#[derive(Debug, Clone, Deserialize)]
enum Condition {
    Depth { max: i32 },
    Subsurface,
    Biomes(Vec<String>),
    Underwater,
    Cold,
    Steep,
    AboveY(i32),
    BelowY(i32),
    Noise { noise: String, min: f64, max: f64 },
    Not(Box<Condition>),
}

#[derive(Debug, Clone, Deserialize)]
enum MaterialDef {
    Block(String),
    BiomeSurface,
    BiomeSubsurface,
    BiomeUnderwater,
    Band,
}

#[derive(Debug, Clone, Deserialize)]
struct RuleDef {
    when: Vec<Condition>,
    then: MaterialDef,
}

#[derive(Debug, Clone, Deserialize)]
struct SurfaceFile {
    #[serde(default)]
    noises: BTreeMap<String, NoiseDef>,
    rules: Vec<RuleDef>,
}

#[derive(Debug, Clone)]
enum Test {
    Depth(i32),
    Subsurface,
    Biomes(Vec<bool>),
    Underwater,
    Cold,
    Steep,
    AboveY(i32),
    BelowY(i32),
    Noise(usize, f64, f64),
    Not(Box<Test>),
}

/// Le matériau d'une règle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Material {
    Block(BlockId),
    BiomeSurface,
    BiomeSubsurface,
    BiomeUnderwater,
    Band,
}

/// Ce qu'on sait d'un bloc du sol du dessus.
#[derive(Debug, Clone, Copy)]
pub(super) struct SurfaceSpot {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Blocs sous le sommet de la colonne (0 : le bloc du dessus).
    pub depth: i32,
    pub subsurface_depth: i32,
    pub biome: BiomeId,
    pub underwater: bool,
    pub cold: bool,
    pub steep: bool,
}

/// Les règles de surface compilées.
#[derive(Debug, Clone)]
pub(super) struct SurfaceRules {
    rules: Vec<(Vec<Test>, Material)>,
    noises: Vec<NormalNoise>,
}

impl SurfaceRules {
    pub(super) fn parse(
        text: &str,
        seed: u64,
        blocks: &BlockRegistry,
        biomes: &BiomeRegistry,
    ) -> Result<Self> {
        let file: SurfaceFile = ron::from_str(text).context("surface_v3.ron illisible")?;
        let names: Vec<String> = file.noises.keys().cloned().collect();
        let noises = file
            .noises
            .iter()
            .map(|(name, def)| NormalNoise::new(seed, &format!("surface/{name}"), def))
            .collect();
        fn test(c: &Condition, names: &[String], biomes: &BiomeRegistry) -> Result<Test> {
            Ok(match c {
                Condition::Depth { max } => Test::Depth(*max),
                Condition::Subsurface => Test::Subsurface,
                Condition::Biomes(keys) => {
                    let mut set = vec![false; 256];
                    for key in keys {
                        match biomes.id(key) {
                            Some(id) => set[id.0 as usize] = true,
                            None => log::warn!("surface v3 : biome « {key} » inconnu"),
                        }
                    }
                    Test::Biomes(set)
                }
                Condition::Underwater => Test::Underwater,
                Condition::Cold => Test::Cold,
                Condition::Steep => Test::Steep,
                Condition::AboveY(y) => Test::AboveY(*y),
                Condition::BelowY(y) => Test::BelowY(*y),
                Condition::Noise { noise, min, max } => Test::Noise(
                    names
                        .iter()
                        .position(|n| n == noise)
                        .with_context(|| format!("surface v3 : bruit « {noise} » inconnu"))?,
                    *min,
                    *max,
                ),
                Condition::Not(inner) => Test::Not(Box::new(test(inner, names, biomes)?)),
            })
        }
        let mut rules = Vec::new();
        for rule in &file.rules {
            let tests = rule
                .when
                .iter()
                .map(|c| test(c, &names, biomes))
                .collect::<Result<Vec<_>>>()?;
            let material = match &rule.then {
                MaterialDef::Block(key) => Material::Block(
                    blocks
                        .id(key)
                        .with_context(|| format!("surface v3 : bloc « {key} » inconnu"))?,
                ),
                MaterialDef::BiomeSurface => Material::BiomeSurface,
                MaterialDef::BiomeSubsurface => Material::BiomeSubsurface,
                MaterialDef::BiomeUnderwater => Material::BiomeUnderwater,
                MaterialDef::Band => Material::Band,
            };
            rules.push((tests, material));
        }
        Ok(Self { rules, noises })
    }

    fn holds(&self, test: &Test, s: &SurfaceSpot) -> bool {
        match test {
            Test::Depth(max) => s.depth <= *max,
            Test::Subsurface => s.depth <= s.subsurface_depth,
            Test::Biomes(set) => set[s.biome.0 as usize],
            Test::Underwater => s.underwater,
            Test::Cold => s.cold,
            Test::Steep => s.steep,
            Test::AboveY(y) => s.y >= *y,
            Test::BelowY(y) => s.y < *y,
            Test::Noise(i, min, max) => {
                let v = self.noises[*i].sample(f64::from(s.x), 0.0, f64::from(s.z));
                v >= *min && v < *max
            }
            Test::Not(inner) => !self.holds(inner, s),
        }
    }

    /// Le matériau du bloc, ou `None` : la roche.
    pub(super) fn material(&self, s: &SurfaceSpot) -> Option<Material> {
        self.rules
            .iter()
            .find(|(tests, _)| tests.iter().all(|t| self.holds(t, s)))
            .map(|(_, m)| *m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registries() -> (std::sync::Arc<BlockRegistry>, BiomeRegistry) {
        let blocks = BlockRegistry::builtin();
        let biomes = BiomeRegistry::builtin(&blocks).unwrap();
        (blocks, biomes)
    }

    #[test]
    fn the_core_tables_only_name_known_biomes_and_blocks() {
        let (blocks, biomes) = registries();
        let text = include_str!("../../../../../assets/core/worldgen/biomes_v3.ron");
        let file: BiomeFile = ron::from_str(text).unwrap();
        for e in &file.biomes {
            assert!(biomes.id(&e.biome).is_some(), "biome inconnu : {}", e.biome);
        }
        // Chaque biome du jeu de base a au moins une case : aucun n'est oublié (sauf le volcan, qui suit
        // les cônes du relief et non le climat, et les biomes souterrains).
        for (_, def) in biomes
            .iter()
            .filter(|(_, d)| d.key != "core:volcano" && !d.underground)
        {
            assert!(
                file.biomes.iter().any(|e| e.biome == def.key),
                "{} n'a pas de case dans biomes_v3.ron",
                def.key
            );
        }
        let surface = include_str!("../../../../../assets/core/worldgen/surface_v3.ron");
        SurfaceRules::parse(surface, 1, &blocks, &biomes).unwrap();
    }

    #[test]
    fn the_nearest_case_wins_and_offsets_break_ties() {
        let (_, biomes) = registries();
        let table = BiomeTable::parse(
            r#"(biomes: [
                (biome: "core:plains", temperature: (-0.2, 0.2)),
                (biome: "core:desert", temperature: (0.6, 2.0)),
                (biome: "core:flower_meadow", temperature: (-0.2, 0.2), offset: 0.1),
            ])"#,
            &biomes,
        )
        .unwrap();
        let at = |t: f64| {
            table.pick(Climate6 {
                temperature: t,
                ..Climate6::default()
            })
        };
        assert_eq!(
            at(0.0),
            biomes.id("core:plains").unwrap(),
            "à égalité, le plus petit décalage"
        );
        assert_eq!(at(0.9), biomes.id("core:desert").unwrap());
        // Entre les deux cases, la plus proche.
        assert_eq!(at(0.35), biomes.id("core:plains").unwrap());
        assert_eq!(at(0.45), biomes.id("core:desert").unwrap());
    }

    #[test]
    fn the_first_matching_rule_gives_the_material() {
        let (blocks, biomes) = registries();
        let rules = SurfaceRules::parse(
            r#"(rules: [
                (when: [Steep, Depth(max: 1)], then: Block("core:stone")),
                (when: [Depth(max: 0), Cold], then: Block("core:snow")),
                (when: [Not(Underwater), Depth(max: 0)], then: BiomeSurface),
                (when: [Subsurface], then: BiomeSubsurface),
            ])"#,
            1,
            &blocks,
            &biomes,
        )
        .unwrap();
        let spot = SurfaceSpot {
            x: 0,
            y: 80,
            z: 0,
            depth: 0,
            subsurface_depth: 3,
            biome: biomes.id("core:plains").unwrap(),
            underwater: false,
            cold: false,
            steep: false,
        };
        let stone = blocks.id("core:stone").unwrap();
        let snow = blocks.id("core:snow").unwrap();
        assert_eq!(rules.material(&spot), Some(Material::BiomeSurface));
        assert_eq!(
            rules.material(&SurfaceSpot {
                steep: true,
                ..spot
            }),
            Some(Material::Block(stone))
        );
        assert_eq!(
            rules.material(&SurfaceSpot { cold: true, ..spot }),
            Some(Material::Block(snow))
        );
        assert_eq!(
            rules.material(&SurfaceSpot { depth: 2, ..spot }),
            Some(Material::BiomeSubsurface)
        );
        assert_eq!(
            rules.material(&SurfaceSpot { depth: 5, ..spot }),
            None,
            "plus bas : la roche"
        );
        assert_eq!(
            rules.material(&SurfaceSpot {
                underwater: true,
                ..spot
            }),
            Some(Material::BiomeSubsurface),
            "sous l'eau, la règle du dessus ne tient plus : la suivante s'applique"
        );
    }
}
