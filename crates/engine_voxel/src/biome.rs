//! Les biomes et les minerais : les données du générateur de terrain.
//!
//! Comme les blocs, ils sont décrits dans des fichiers (`assets/core/biomes.ron`,
//! `assets/core/ores.ron`) plutôt que dans le code : on peut régler le monde sans
//! recompiler, et un mod pourra ajouter les siens.

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;
use crate::registry::BlockRegistry;
use crate::trees::TreeKind;

const BUILTIN_BIOMES: &str = include_str!("../../../assets/core/biomes.ron");
const BUILTIN_ORES: &str = include_str!("../../../assets/core/ores.ron");

/// Le numéro d'un biome dans le registre (son rang dans `biomes.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BiomeId(pub u8);

/// La grande famille d'un biome : décidée par le relief, avant le climat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum BiomeKind {
    Ocean,
    DeepOcean,
    Beach,
    River,
    /// Les terres : le biome est choisi par le climat.
    Land,
    Mountains,
    Peaks,
}

/// Le relief propre à un biome : un décalage de hauteur, l'amplitude des collines, et des
/// terrasses (de 0 : pentes douces, à 1 : des paliers plats séparés de petites falaises, comme
/// des mesas).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Relief {
    pub offset: f64,
    pub scale: f64,
    #[serde(default)]
    pub terraces: f64,
}

impl Default for Relief {
    fn default() -> Self {
        Self {
            offset: 0.0,
            scale: 1.0,
            terraces: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BiomeEntry {
    id: String,
    name: String,
    kind: BiomeKind,
    #[serde(default)]
    climate: (f64, f64),
    #[serde(default)]
    variety: f64,
    temperature: f32,
    surface: String,
    subsurface: String,
    #[serde(default = "three")]
    subsurface_depth: i32,
    #[serde(default)]
    ground_patches: Vec<(String, f32)>,
    #[serde(default)]
    bands: Vec<String>,
    underwater: String,
    #[serde(default)]
    rock_above: Option<i32>,
    #[serde(default)]
    relief: Relief,
    grass_color: (f32, f32, f32),
    foliage_color: (f32, f32, f32),
    water_color: (f32, f32, f32),
    #[serde(default)]
    trees: Vec<(TreeKind, f32)>,
    #[serde(default)]
    tree_density: f32,
    #[serde(default = "default_groves")]
    groves: f32,
    #[serde(default)]
    plants: Vec<PlantEntry>,
    #[serde(default)]
    seaweed: f32,
    #[serde(default)]
    lily_pads: f32,
    #[serde(default)]
    v3_only: bool,
    #[serde(default)]
    underground: bool,
}

fn three() -> i32 {
    3
}

fn default_groves() -> f32 {
    0.6
}

/// Une plante dans `biomes.ron` : `("poppy", 0.01)`, ou `("poppy", 0.01, 0.9)` pour dire
/// à quel point elle pousse en taches (0 : partout pareil, 1 : seulement en taches).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
enum PlantEntry {
    Patchy(String, f32, f32),
    Plain(String, f32),
}

/// Les fleurs et champignons poussent en taches par défaut.
const DEFAULT_PATCHES: f32 = 0.85;

/// Une plante d'un biome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantDef {
    pub block: BlockId,
    /// La probabilité moyenne par colonne.
    pub chance: f32,
    /// De 0 (répartie au hasard) à 1 (seulement en taches, plus denses).
    pub patches: f32,
}

/// Un biome, ses blocs déjà traduits en numéros.
#[derive(Debug, Clone, PartialEq)]
pub struct BiomeDef {
    pub key: String,
    pub name: String,
    pub kind: BiomeKind,
    /// (température, humidité) du biome, entre -1 et 1 : sa place dans le climat.
    pub climate: (f64, f64),
    /// La variété du biome, entre -1 et 1 (0 : le biome « normal » de son climat) : elle
    /// départage les variantes d'un même climat.
    pub variety: f64,
    /// En dessous de 0,15, il neige et l'eau gèle.
    pub temperature: f32,
    pub surface: BlockId,
    pub subsurface: BlockId,
    pub subsurface_depth: i32,
    /// Des taches d'un autre sol en surface : (bloc, part du sol couverte, de 0 à 1).
    pub ground_patches: Vec<(BlockId, f32)>,
    /// Si non vide, le sous-sol est fait de ces blocs en strates horizontales (falaises
    /// rayées des plateaux arides), à la place de `subsurface`.
    pub bands: Vec<BlockId>,
    pub underwater: BlockId,
    pub rock_above: Option<i32>,
    pub relief: Relief,
    pub grass_color: [f32; 3],
    pub foliage_color: [f32; 3],
    pub water_color: [f32; 3],
    /// Les formes d'arbres du biome et leur poids (leur part relative).
    pub trees: Vec<(TreeKind, f32)>,
    /// Le nombre moyen d'arbres par colonne de chunk (32 × 32 blocs).
    pub tree_density: f32,
    /// De 0 (arbres répartis au hasard) à 1 (bosquets serrés et clairières).
    pub groves: f32,
    /// Les plantes posées sur le sol.
    pub plants: Vec<PlantDef>,
    /// La probabilité d'algues par colonne sous l'eau, et de nénuphars en surface.
    pub seaweed: f32,
    pub lily_pads: f32,
    /// Un biome de la génération v3 seulement (vagues du lot 5) : la génération v1 ne le
    /// choisissent jamais, leurs mondes ne changent pas.
    pub v3_only: bool,
    /// Un biome souterrain (vague 4 du lot 5) : choisi en trois dimensions dans les grottes profondes,
    /// jamais pour une colonne de surface.
    pub underground: bool,
}

/// Un type de filon.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct OreEntry {
    block: String,
    height: (i32, i32),
    size: u32,
    per_chunk: f32,
    #[serde(default)]
    replace: Option<Vec<String>>,
    #[serde(default)]
    biomes: Option<Vec<String>>,
    #[serde(default)]
    deep: Option<(i32, i32)>,
    #[serde(default)]
    v3_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OreDef {
    pub block: BlockId,
    /// Altitudes (min, max) incluses.
    pub height: (i32, i32),
    pub size: u32,
    pub per_chunk: f32,
    /// Les blocs qu'un filon peut remplacer.
    pub replace: Vec<BlockId>,
    /// `None` : partout.
    pub biomes: Option<Vec<BiomeId>>,
    /// Les altitudes dans un monde v3 (de -128 à 255), si elles diffèrent : les minerais profonds y
    /// descendent sous 0. `None` : les mêmes que `height`.
    pub deep: Option<(i32, i32)>,
    /// Un minerai de la génération v3 seulement : les mondes v1 ne changent pas.
    pub v3_only: bool,
}

impl OreDef {
    /// Les altitudes du minerai pour une génération (`None` : il n'y est pas).
    pub fn range(&self, v3: bool) -> Option<(i32, i32)> {
        match (v3, self.v3_only) {
            (true, _) => Some(self.deep.unwrap_or(self.height)),
            (false, true) => None,
            (false, false) => Some(self.height),
        }
    }
}

/// Les biomes et les minerais.
#[derive(Debug, Clone, Default)]
pub struct BiomeRegistry {
    biomes: Vec<BiomeDef>,
    by_key: HashMap<String, BiomeId>,
    ores: Vec<OreDef>,
}

/// Le lecteur RON : `rock_above: 120` plutôt que `rock_above: Some(120)`.
fn ron_options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

/// `grass` → `core:grass` (un nom sans espace de noms est dans celui du fichier).
fn qualify(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

impl BiomeRegistry {
    /// Les biomes et minerais de base, intégrés au binaire.
    pub fn builtin(blocks: &BlockRegistry) -> Result<Self> {
        let mut registry = Self::default();
        registry.add_biomes("core", BUILTIN_BIOMES, "biomes.ron intégré", blocks)?;
        registry.add_ores("core", BUILTIN_ORES, "ores.ron intégré", blocks)?;
        registry.check()?;
        Ok(registry)
    }

    /// Les biomes et minerais de base, puis ceux de chaque mod (`biomes.ron` et `ores.ron` de son
    /// dossier), dans l'ordre de chargement.
    pub fn load_mods(
        blocks: &BlockRegistry,
        mods: &[(String, std::path::PathBuf)],
    ) -> Result<Self> {
        let mut registry = Self::default();
        registry.add_biomes("core", BUILTIN_BIOMES, "biomes.ron intégré", blocks)?;
        registry.add_ores("core", BUILTIN_ORES, "ores.ron intégré", blocks)?;
        for (namespace, dir) in mods {
            let biomes = dir.join("biomes.ron");
            if biomes.exists() {
                let text = std::fs::read_to_string(&biomes)
                    .with_context(|| format!("lecture de {}", biomes.display()))?;
                registry.add_biomes(namespace, &text, &biomes.display().to_string(), blocks)?;
            }
            let ores = dir.join("ores.ron");
            if ores.exists() {
                let text = std::fs::read_to_string(&ores)
                    .with_context(|| format!("lecture de {}", ores.display()))?;
                registry.add_ores(namespace, &text, &ores.display().to_string(), blocks)?;
            }
        }
        registry.check()?;
        Ok(registry)
    }

    pub fn add_biomes(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        blocks: &BlockRegistry,
    ) -> Result<()> {
        let entries: Vec<BiomeEntry> = ron_options()
            .from_str(source)
            .with_context(|| format!("{origin} : RON invalide"))?;
        let block = |name: &str, biome: &str| {
            let key = qualify(name, namespace);
            blocks.id(&key).with_context(|| {
                format!("{origin} : le biome « {biome} » utilise le bloc inconnu « {key} »")
            })
        };
        for e in entries {
            if self.by_key.contains_key(&e.id) {
                bail!("{origin} : le biome « {} » est défini deux fois", e.id);
            }
            let id =
                BiomeId(u8::try_from(self.biomes.len()).context("trop de biomes (256 au plus)")?);
            let color = |(r, g, b): (f32, f32, f32)| [r, g, b];
            let mut def = BiomeDef {
                surface: block(&e.surface, &e.id)?,
                subsurface: block(&e.subsurface, &e.id)?,
                underwater: block(&e.underwater, &e.id)?,
                key: e.id.clone(),
                name: e.name,
                kind: e.kind,
                climate: e.climate,
                variety: e.variety,
                temperature: e.temperature,
                subsurface_depth: e.subsurface_depth,
                ground_patches: e
                    .ground_patches
                    .iter()
                    .map(|(name, part)| Ok((block(name, &e.id)?, part.clamp(0.0, 1.0))))
                    .collect::<Result<Vec<_>>>()?,
                bands: e
                    .bands
                    .iter()
                    .map(|name| block(name, &e.id))
                    .collect::<Result<Vec<_>>>()?,
                rock_above: e.rock_above,
                relief: e.relief,
                grass_color: color(e.grass_color),
                foliage_color: color(e.foliage_color),
                water_color: color(e.water_color),
                plants: e
                    .plants
                    .iter()
                    .map(|entry| {
                        let (name, chance, patches) = match entry {
                            PlantEntry::Plain(name, chance) => (name, *chance, DEFAULT_PATCHES),
                            PlantEntry::Patchy(name, chance, patches) => {
                                (name, *chance, patches.clamp(0.0, 1.0))
                            }
                        };
                        Ok(PlantDef {
                            block: block(name, &e.id)?,
                            chance,
                            patches,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                trees: e.trees,
                tree_density: e.tree_density,
                groves: e.groves.clamp(0.0, 1.0),
                seaweed: e.seaweed,
                lily_pads: e.lily_pads,
                v3_only: e.v3_only,
                underground: e.underground,
            };
            // Les plantes les plus rares d'abord : la végétation les tire dans cet ordre.
            def.plants.sort_by(|a, b| a.chance.total_cmp(&b.chance));
            self.by_key.insert(e.id, id);
            self.biomes.push(def);
        }
        Ok(())
    }

    pub fn add_ores(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        blocks: &BlockRegistry,
    ) -> Result<()> {
        let entries: Vec<OreEntry> = ron_options()
            .from_str(source)
            .with_context(|| format!("{origin} : RON invalide"))?;
        let block = |name: &str| {
            let key = qualify(name, namespace);
            blocks
                .id(&key)
                .with_context(|| format!("{origin} : bloc inconnu « {key} »"))
        };
        for e in entries {
            let replace = match &e.replace {
                Some(list) => list.iter().map(|n| block(n)).collect::<Result<Vec<_>>>()?,
                None => vec![block("core:stone")?, block("core:deepstone")?],
            };
            let biomes = match &e.biomes {
                Some(list) => Some(
                    list.iter()
                        .map(|n| {
                            let key = qualify(n, namespace);
                            self.id(&key)
                                .with_context(|| format!("{origin} : biome inconnu « {key} »"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
                None => None,
            };
            if e.height.0 > e.height.1 || e.deep.is_some_and(|d| d.0 > d.1) {
                bail!("{origin} : « {} » : height (min, max) à l'envers", e.block);
            }
            self.ores.push(OreDef {
                block: block(&e.block)?,
                height: e.height,
                size: e.size,
                per_chunk: e.per_chunk,
                replace,
                biomes,
                deep: e.deep,
                v3_only: e.v3_only,
            });
        }
        Ok(())
    }

    /// Le générateur a besoin d'au moins un biome de chaque famille.
    fn check(&self) -> Result<()> {
        use BiomeKind::*;
        for kind in [Ocean, DeepOcean, Beach, River, Land, Mountains, Peaks] {
            if !self.biomes.iter().any(|b| b.kind == kind) {
                bail!("aucun biome de type {kind:?}");
            }
        }
        Ok(())
    }

    pub fn id(&self, key: &str) -> Option<BiomeId> {
        self.by_key.get(key).copied()
    }

    pub fn get(&self, id: BiomeId) -> &BiomeDef {
        &self.biomes[id.0 as usize]
    }

    /// Le premier biome d'une famille (pour les familles choisies par le relief).
    pub fn first_of(&self, kind: BiomeKind) -> BiomeId {
        let index = self.biomes.iter().position(|b| b.kind == kind).unwrap_or(0);
        BiomeId(index as u8)
    }

    pub fn iter(&self) -> impl Iterator<Item = (BiomeId, &BiomeDef)> {
        self.biomes
            .iter()
            .enumerate()
            .map(|(i, b)| (BiomeId(i as u8), b))
    }

    pub fn len(&self) -> usize {
        self.biomes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.biomes.is_empty()
    }

    /// Les couleurs de référence de l'herbe et du feuillage (celles des plaines) : les
    /// textures sont peintes dans ces couleurs, la teinte d'un biome est son écart à elles.
    pub fn tint_reference(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let plains = self
            .id("core:plains")
            .map(|id| self.get(id))
            .or_else(|| self.biomes.iter().find(|b| b.kind == BiomeKind::Land));
        plains.map_or(([1.0; 3], [1.0; 3], [1.0; 3]), |b| {
            (b.grass_color, b.foliage_color, b.water_color)
        })
    }

    /// Le multiplicateur de teinte d'une couleur de biome, par rapport à sa référence. Les
    /// sommets stockent leur couleur entre 0 et 1 : un canal au-dessus de 1 serait coupé et la
    /// teinte perdrait sa nuance (une savane presque aussi verte qu'une plaine). On ramène alors
    /// le plus fort canal à 1 en gardant les proportions : même nuance, un peu plus sombre.
    pub fn tint_factor(color: [f32; 3], reference: [f32; 3]) -> [f32; 3] {
        let ratio: [f32; 3] = std::array::from_fn(|c| color[c] / reference[c].max(0.01));
        let top = ratio.iter().copied().fold(1.0, f32::max);
        ratio.map(|r| r / top)
    }

    pub fn ores(&self) -> &[OreDef] {
        &self.ores
    }
}
