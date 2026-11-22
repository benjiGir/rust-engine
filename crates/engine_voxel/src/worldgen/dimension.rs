//! Les dimensions : des mondes parallèles décrits en données (`dimensions.ron` d'un mod). Chacune a
//! son générateur (le terrain du jeu, un monde plat, ou le vide), son ciel (couleur, lumière fixe
//! ou cycle du jour), sa gravité, et son propre dossier de sauvegarde. On passe de l'une à l'autre
//! par un **portail** : un bloc dont `blocks.ron` dit `portal: "mon_mod:ma_dimension"`.
//!
//! Ce module ne contient que les données et leur lecture ; le générateur les applique
//! (`TerrainGenerator::with_dimension`) et le jeu s'occupe du reste (ciel, gravité, voyage).

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;
use crate::registry::BlockRegistry;
use crate::worldgen::biome::{BiomeId, BiomeRegistry};

/// L'identifiant de la dimension de départ.
pub const OVERWORLD: &str = "core:overworld";

/// Comment le monde d'une dimension est généré.
#[derive(Debug, Clone, PartialEq)]
pub enum WorldKind {
    /// Le terrain du jeu (collines, océans, grottes, structures). `biomes` : les seuls biomes de
    /// terre permis (vide : tous). Une autre `seed_offset` donne un autre monde.
    Terrain { biomes: Vec<BiomeId> },
    /// Des couches posées depuis y = 0 (de bas en haut), à l'infini, dans un seul biome.
    Flat {
        layers: Vec<(BlockId, u32)>,
        biome: BiomeId,
    },
    /// Le vide, avec au plus une plateforme carrée de 17 blocs de côté à y = 64 autour de l'origine.
    Void { platform: Option<BlockId> },
}

/// Le ciel d'une dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyDef {
    /// La couleur du ciel et du brouillard (`None` : celle du cycle du jour).
    pub color: Option<[f32; 3]>,
    /// Une lumière du ciel fixe, de 0 (noir) à 1 (plein jour) : pas de cycle jour/nuit. `None` :
    /// le cycle habituel.
    pub daylight: Option<f32>,
}

/// Une dimension.
#[derive(Debug, Clone)]
pub struct DimensionDef {
    pub key: String,
    pub name: String,
    pub kind: WorldKind,
    /// Ajoutée à la graine du monde : le même monde a une autre carte dans chaque dimension.
    pub seed_offset: u32,
    pub sky: SkyDef,
    /// Un multiple de la gravité normale (1 : normale, 0,5 : lune).
    pub gravity: f32,
    /// Le bloc posé là où le joueur arrive (typiquement le portail de retour), s'il y en a un.
    pub arrival: Option<BlockId>,
    /// Le dossier de sa sauvegarde dans celui du monde : `None` pour la dimension de départ
    /// (à la racine), `dimensions/<nom>` sinon.
    pub folder: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
enum GeneratorEntry {
    Terrain {
        #[serde(default)]
        biomes: Vec<String>,
    },
    Flat {
        layers: Vec<(String, u32)>,
        biome: String,
    },
    Void {
        #[serde(default)]
        platform: Option<String>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkyEntry {
    #[serde(default)]
    color: Option<(f32, f32, f32)>,
    #[serde(default)]
    daylight: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DimensionEntry {
    id: String,
    name: String,
    generator: GeneratorEntry,
    #[serde(default)]
    seed_offset: u32,
    #[serde(default)]
    sky: SkyEntry,
    #[serde(default = "one")]
    gravity: f32,
    #[serde(default)]
    arrival: Option<String>,
}

fn one() -> f32 {
    1.0
}

/// Toutes les dimensions connues, la dimension de départ en premier.
#[derive(Debug, Clone)]
pub struct DimensionRegistry {
    defs: Vec<DimensionDef>,
}

impl DimensionRegistry {
    /// La seule dimension de départ : le terrain du jeu, le cycle du jour, la gravité normale.
    pub fn builtin() -> Self {
        Self {
            defs: vec![DimensionDef {
                key: OVERWORLD.into(),
                name: "Le monde".into(),
                kind: WorldKind::Terrain { biomes: Vec::new() },
                seed_offset: 0,
                sky: SkyDef {
                    color: None,
                    daylight: None,
                },
                gravity: 1.0,
                arrival: None,
                folder: None,
            }],
        }
    }

    /// La dimension de départ, puis celles du `dimensions.ron` de chaque mod (dans l'ordre).
    pub fn load_mods(
        blocks: &BlockRegistry,
        biomes: &BiomeRegistry,
        mods: &[(String, PathBuf)],
    ) -> Result<Self> {
        let mut registry = Self::builtin();
        for (namespace, dir) in mods {
            let path = dir.join("dimensions.ron");
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("lecture de {}", path.display()))?;
                registry.add_source(
                    namespace,
                    &text,
                    &path.display().to_string(),
                    blocks,
                    biomes,
                )?;
            }
        }
        Ok(registry)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        text: &str,
        origin: &str,
        blocks: &BlockRegistry,
        biomes: &BiomeRegistry,
    ) -> Result<()> {
        let entries: Vec<DimensionEntry> = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .with_context(|| format!("{origin} : RON invalide"))?;
        for e in entries {
            let id = e.id.clone();
            let def = self
                .resolve(namespace, e, blocks, biomes)
                .with_context(|| format!("{origin} : dimension « {id} »"))?;
            self.defs.push(def);
        }
        Ok(())
    }

    fn resolve(
        &self,
        namespace: &str,
        e: DimensionEntry,
        blocks: &BlockRegistry,
        biomes: &BiomeRegistry,
    ) -> Result<DimensionDef> {
        if e.id.split_once(':').map(|(ns, _)| ns) != Some(namespace) {
            bail!("l'identifiant doit commencer par « {namespace}: »");
        }
        if self.get(&e.id).is_some() {
            bail!("définie deux fois");
        }
        let name = e.id.split_once(':').map_or("", |(_, n)| n);
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("le nom ne doit contenir que a-z, 0-9 et _");
        }
        if !(0.05..=4.0).contains(&e.gravity) {
            bail!("la gravité va de 0,05 à 4 (1 : normale)");
        }
        if e.sky.daylight.is_some_and(|d| !(0.0..=1.0).contains(&d)) {
            bail!("la lumière du ciel va de 0 à 1");
        }
        let block = |key: &str| {
            let key = qualify(key, namespace);
            blocks
                .id(&key)
                .with_context(|| format!("bloc inconnu « {key} »"))
        };
        let biome = |key: &str| {
            let key = qualify(key, namespace);
            biomes
                .id(&key)
                .with_context(|| format!("biome inconnu « {key} »"))
        };
        let kind = match e.generator {
            GeneratorEntry::Terrain { biomes: list } => WorldKind::Terrain {
                biomes: list.iter().map(|b| biome(b)).collect::<Result<_>>()?,
            },
            GeneratorEntry::Flat { layers, biome: b } => {
                if layers.is_empty() || layers.iter().any(|(_, n)| *n == 0) {
                    bail!("un monde plat a au moins une couche, chacune d'au moins 1 bloc");
                }
                if layers.iter().map(|(_, n)| *n as i64).sum::<i64>() > 200 {
                    bail!("un monde plat fait 200 blocs de haut au plus");
                }
                WorldKind::Flat {
                    layers: layers
                        .iter()
                        .map(|(k, n)| Ok((block(k)?, *n)))
                        .collect::<Result<_>>()?,
                    biome: biome(&b)?,
                }
            }
            GeneratorEntry::Void { platform } => WorldKind::Void {
                platform: platform.as_deref().map(block).transpose()?,
            },
        };
        let folder = PathBuf::from("dimensions").join(e.id.replace(':', "_"));
        Ok(DimensionDef {
            key: e.id,
            name: e.name,
            kind,
            seed_offset: e.seed_offset,
            sky: SkyDef {
                color: e.sky.color.map(|(r, g, b)| [r, g, b]),
                daylight: e.sky.daylight,
            },
            gravity: e.gravity,
            arrival: e.arrival.as_deref().map(block).transpose()?,
            folder: Some(folder),
        })
    }

    pub fn get(&self, key: &str) -> Option<&DimensionDef> {
        self.defs.iter().find(|d| d.key == key)
    }

    /// La dimension de départ.
    pub fn overworld(&self) -> &DimensionDef {
        &self.defs[0]
    }

    pub fn iter(&self) -> impl Iterator<Item = &DimensionDef> {
        self.defs.iter()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

fn qualify(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> (std::sync::Arc<BlockRegistry>, BiomeRegistry) {
        let blocks = BlockRegistry::builtin();
        let biomes = BiomeRegistry::builtin(&blocks).unwrap();
        (blocks, biomes)
    }

    const SOURCE: &str = r#"[
        (id: "test:moon", name: "Lune", generator: Flat(layers: [("core:bedrock", 1), ("core:stone", 30)], biome: "core:plains"),
         sky: (color: (0.0, 0.0, 0.1), daylight: 0.5), gravity: 0.4, arrival: "core:glowstone"),
        (id: "test:void", name: "Vide", generator: Void(platform: "core:stone")),
        (id: "test:cold", name: "Froid", generator: Terrain(biomes: ["core:snowy_tundra"]), seed_offset: 9),
    ]"#;

    #[test]
    fn dimensions_are_read_and_resolved() {
        let (blocks, biomes) = data();
        let mut r = DimensionRegistry::builtin();
        r.add_source("test", SOURCE, "test", &blocks, &biomes)
            .unwrap();
        assert_eq!(r.len(), 4);
        assert_eq!(r.overworld().key, OVERWORLD);
        let moon = r.get("test:moon").unwrap();
        assert_eq!(moon.gravity, 0.4);
        assert_eq!(moon.sky.daylight, Some(0.5));
        assert_eq!(moon.arrival, blocks.id("core:glowstone"));
        assert!(matches!(&moon.kind, WorldKind::Flat { layers, .. } if layers.len() == 2));
        assert_eq!(
            moon.folder.as_deref(),
            Some(std::path::Path::new("dimensions/test_moon"))
        );
        assert!(matches!(
            &r.get("test:void").unwrap().kind,
            WorldKind::Void { platform: Some(_) }
        ));
        let cold = r.get("test:cold").unwrap();
        assert_eq!(cold.seed_offset, 9);
        assert!(matches!(&cold.kind, WorldKind::Terrain { biomes } if biomes.len() == 1));
    }

    #[test]
    fn bad_dimensions_are_refused_with_a_reason() {
        let (blocks, biomes) = data();
        let bad = |text: &str| {
            let mut r = DimensionRegistry::builtin();
            format!(
                "{:#}",
                r.add_source("test", text, "t", &blocks, &biomes)
                    .unwrap_err()
            )
        };
        let flat = |extra: &str| {
            format!(
                r#"[(id: "test:x", name: "X", generator: Flat(layers: [("core:stone", 5)], biome: "core:plains"){extra})]"#
            )
        };
        assert!(bad(&flat(", gravity: 9.0")).contains("gravité"));
        assert!(bad(&flat(", sky: (daylight: 2.0)")).contains("lumière"));
        assert!(bad(&flat("").replace("core:stone", "core:nope")).contains("core:nope"));
        assert!(bad(&flat("").replace("core:plains", "core:nowhere")).contains("core:nowhere"));
        assert!(bad(&flat("").replace("5)", "0)")).contains("couche"));
        assert!(bad(&flat("").replace("test:x", "other:x")).contains("commencer par"));
        assert!(bad(&flat("").replace("test:x", "test:Bad Name")).contains("a-z"));
        assert!(bad(&flat(", arrival: \"core:nope\"")).contains("nope"));
        // Deux fois la même.
        let mut r = DimensionRegistry::builtin();
        r.add_source("test", &flat(""), "t", &blocks, &biomes)
            .unwrap();
        assert!(
            format!(
                "{:#}",
                r.add_source("test", &flat(""), "t", &blocks, &biomes)
                    .unwrap_err()
            )
            .contains("deux fois")
        );
    }
}
