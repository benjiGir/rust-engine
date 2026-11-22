//! Les données de l'aventure : raretés, affixes, gemmes, paliers du monde et augmentations des
//! élites. Tout est dans des fichiers RON, `assets/core/<nom>.ron` puis ceux des mods (même
//! nom, dans leur dossier), qui **ajoutent** à la suite : un mod peut donc créer ses raretés,
//! ses affixes, ses gemmes, ses paliers et ses augmentations sans toucher au jeu de base.
//!
//! | Fichier | Contenu | Voir |
//! | --- | --- | --- |
//! | `rarities.ron` | les raretés (couleur, poids, affixes, emplacements, coûts) |
//! `adventure/affixes.rs` |
//! | `affixes.ron` | les affixes (nom, catégories, attribut, valeurs par rareté) |
//! `adventure/affixes.rs` |
//! | `gems.ron` | les sortes de gemmes et leur bonus par catégorie | `adventure/gems.rs` |
//! | `tiers.ron` | les paliers du monde et leurs critères de déblocage | `adventure/tiers.rs` |
//! | `elites.ron` | les augmentations des élites | `mobs/elites.rs` |
//! | `enchantments.ron` | les enchantements (niveaux, cibles, effets) | `enchanting/data.rs` |
//! | `charms.ron` | les charmes de potion | `adventure/charms.rs` |
//!
//! # Les numéros sont des places
//!
//! Une fiche d'objet sauvegarde le **numéro** de sa rareté, `world.ron` celui du palier : ces
//! numéros sont la place dans la liste (core d'abord, puis les mods dans l'ordre de chargement).
//! On n'en déplace donc jamais un, on en ajoute à la fin. Les identifiants des affixes et des
//! gemmes, eux, sont des textes (`core:sharp`, `core:ruby`) : une fiche qui cite un affixe
//! disparu garde son bonus (il est écrit dans la fiche), seul son nom n'est plus reconnu.
//!
//! # Le jeu de base intégré
//!
//! Les tests et le jeu de secours lisent les fichiers de `core` intégrés au binaire
//! ([`AdventureData::builtin`]). Le jeu, lui, charge les fichiers du disque (core et mods) puis
//! les installe pour tout le processus ([`install`]) : les modules purs (`player/gear.rs`,
//! `ui/info.rs`) y accèdent par [`current`] sans qu'on ait à leur faire suivre un registre.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use anyhow::{Context as _, Result, bail};
use engine_voxel::ItemRegistry;
use serde::Deserialize;

use crate::player::attributes::Attribute;

const BUILTIN_RARITIES: &str = include_str!("../../../../assets/core/rarities.ron");
const BUILTIN_AFFIXES: &str = include_str!("../../../../assets/core/affixes.ron");
const BUILTIN_GEMS: &str = include_str!("../../../../assets/core/gems.ron");
const BUILTIN_TIERS: &str = include_str!("../../../../assets/core/tiers.ron");
const BUILTIN_ELITES: &str = include_str!("../../../../assets/core/elites.ron");
const BUILTIN_ENCHANTMENTS: &str = include_str!("../../../../assets/core/enchantments.ron");
const BUILTIN_CHARMS: &str = include_str!("../../../../assets/core/charms.ron");

// ----- Les raretés -----

/// Ce que rend la table de récupération pour un objet de cette rareté.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Salvage {
    pub shards: (u32, u32),
    #[serde(default)]
    pub dust: (u32, u32),
}

/// Ce que coûte un tirage de la table de réforge pour cette rareté.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reforge {
    pub shards: u32,
    #[serde(default)]
    pub dust: u32,
    pub levels: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rarity {
    pub id: String,
    pub name: String,
    /// Rouge, vert, bleu de 0 à 255.
    pub color: (u8, u8, u8),
    pub weight: f32,
    /// Combien d'affixes (min, max).
    pub affixes: (u32, u32),
    /// Combien d'emplacements de gemme (min, max).
    pub sockets: (u32, u32),
    /// L'objet qui sert de matériau de cette rareté.
    pub shard: String,
    pub salvage: Salvage,
    pub reforge: Reforge,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RaritiesFile {
    #[serde(default)]
    max_sockets: Option<u32>,
    rarities: Vec<Rarity>,
}

// ----- Les affixes -----

/// Où le nom d'un affixe s'ajoute à celui de l'objet.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub enum Place {
    /// Un adjectif (masculin, féminin) après le nom de l'objet.
    Prefix(String, String),
    /// Un complément en fin de nom.
    Suffix(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AffixDef {
    pub id: String,
    pub place: Place,
    pub categories: Vec<String>,
    pub attribute: Attribute,
    /// Un intervalle (min, max) par rareté.
    pub flat: Vec<(f32, f32)>,
    pub percent: Vec<(f32, f32)>,
    pub weight: u32,
    pub min_rarity: u8,
}

impl AffixDef {
    /// L'intervalle du bonus fixe pour cette rareté (la dernière valeur sert au-delà).
    pub fn flat_range(&self, rarity: u8) -> Option<(f32, f32)> {
        range_for(&self.flat, rarity)
    }

    pub fn percent_range(&self, rarity: u8) -> Option<(f32, f32)> {
        range_for(&self.percent, rarity)
    }
}

fn range_for(list: &[(f32, f32)], rarity: u8) -> Option<(f32, f32)> {
    list.get(usize::from(rarity)).or(list.last()).copied()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AffixEntry {
    id: String,
    place: Place,
    categories: Vec<String>,
    attribute: String,
    #[serde(default)]
    flat: Vec<(f32, f32)>,
    #[serde(default)]
    percent: Vec<(f32, f32)>,
    weight: u32,
    #[serde(default)]
    min_rarity: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AffixesFile {
    #[serde(default)]
    feminine: Vec<String>,
    #[serde(default)]
    plural: Vec<String>,
    affixes: Vec<AffixEntry>,
}

// ----- Les gemmes -----

/// Le bonus d'une gemme sur une catégorie d'objets, une valeur par qualité.
#[derive(Debug, Clone, PartialEq)]
pub struct GemBonus {
    pub categories: Vec<String>,
    pub attribute: Attribute,
    pub flat: Vec<f32>,
    pub percent: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GemKind {
    pub id: String,
    pub name: String,
    pub feminine: bool,
    /// La clé de l'objet, avec `{q}` à la place de la qualité (1 à 5).
    pub item: String,
    pub color: (u8, u8, u8),
    pub bonuses: Vec<GemBonus>,
}

impl GemKind {
    /// La clé de l'objet de cette qualité (de 0 à 4).
    pub fn item_key(&self, quality: usize) -> String {
        self.item.replace("{q}", &(quality + 1).to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GemBonusEntry {
    categories: Vec<String>,
    attribute: String,
    #[serde(default)]
    flat: Vec<f32>,
    #[serde(default)]
    percent: Vec<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GemEntry {
    id: String,
    name: String,
    #[serde(default)]
    feminine: bool,
    item: String,
    color: (u8, u8, u8),
    bonuses: Vec<GemBonusEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GemsFile {
    #[serde(default)]
    qualities: Vec<(String, String)>,
    gems: Vec<GemEntry>,
}

// ----- Les paliers -----

/// Une condition de déblocage d'un palier.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum Criterion {
    /// Tuer ce nombre d'hostiles.
    Kills(u32),
    /// Vaincre ce nombre de boss.
    Bosses(u32),
    /// Obtenir un objet de cette rareté (son numéro) ou mieux.
    Rarity(u8),
    /// Descendre à cette profondeur sous le niveau de la mer.
    Depth(i32),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub health: f32,
    pub damage: f32,
    pub rarity_boost: f32,
    pub elite_chance: f32,
    pub drop_chance: f32,
    /// Le temps entre deux envahisseurs, en secondes (min, max) ; (0, 0) : aucun.
    pub invader: (u32, u32),
    pub unlock: Vec<Criterion>,
}

// ----- Les élites -----

/// L'effet d'une augmentation d'élite.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum Effect {
    /// La vie multipliée.
    Health(f32),
    /// Les dégâts multipliés.
    Damage(f32),
    /// La vitesse multipliée.
    Speed(f32),
    /// Des points d'armure en plus.
    Armor(f32),
    /// Des points de vie rendus par seconde.
    Regen(f32),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Augmentation {
    pub id: String,
    /// Le nom qui s'ajoute à celui de la créature, en apposition (« Zombie colosse »).
    pub title: String,
    pub color: (f32, f32, f32),
    pub effect: Effect,
    pub weight: u32,
    #[serde(default)]
    pub min_tier: u8,
}

/// La gemme du butin garanti d'une élite ou d'un envahisseur.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GemDrop {
    /// La chance qu'une gemme tombe.
    pub chance: f32,
    /// La qualité (de 0 à 4), min et max.
    pub quality: (u32, u32),
    /// La chance d'avoir aussi de la poussière de gemme.
    pub dust: f32,
}

impl Default for GemDrop {
    fn default() -> Self {
        Self {
            chance: 0.0,
            quality: (0, 0),
            dust: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElitesFile {
    #[serde(default)]
    max_picks: Option<u32>,
    #[serde(default)]
    invader_health: Option<f32>,
    #[serde(default)]
    invader_color: Option<(f32, f32, f32)>,
    #[serde(default)]
    elite_min_rarity: Option<u8>,
    #[serde(default)]
    invader_min_rarity: Option<u8>,
    #[serde(default)]
    elite_gem: Option<GemDrop>,
    #[serde(default)]
    invader_gem: Option<GemDrop>,
    #[serde(default)]
    loot: Vec<(String, u8)>,
    #[serde(default)]
    augmentations: Vec<Augmentation>,
}

// ----- L'ensemble -----

/// Toutes les données de l'aventure.
#[derive(Debug, Clone, PartialEq)]
pub struct AdventureData {
    pub rarities: Vec<Rarity>,
    pub max_sockets: u32,
    pub affixes: Vec<AffixDef>,
    /// Les mots qui ouvrent un nom d'objet féminin, pluriel (pour accorder les adjectifs).
    pub feminine: Vec<String>,
    pub plural: Vec<String>,
    /// Les qualités des gemmes, de la plus basse à la plus haute : (masculin, féminin).
    pub qualities: Vec<(String, String)>,
    pub gems: Vec<GemKind>,
    pub tiers: Vec<TierDef>,
    pub augmentations: Vec<Augmentation>,
    pub max_picks: u32,
    pub invader_health: f32,
    pub invader_color: (f32, f32, f32),
    pub elite_min_rarity: u8,
    pub invader_min_rarity: u8,
    pub elite_gem: GemDrop,
    pub invader_gem: GemDrop,
    /// Les objets de base des butins à affixes : (clé, palier minimal).
    pub loot: Vec<(String, u8)>,
    /// Les enchantements (voir `enchanting/data.rs`) et le plus qu'un objet en porte.
    pub enchantments: Vec<crate::enchanting::data::EnchantDef>,
    pub max_enchantments: usize,
    /// Les charmes de potion (voir `adventure/charms.rs`).
    pub charms: Vec<crate::adventure::charms::CharmDef>,
    gem_by_item: HashMap<String, (usize, usize)>,
}

impl Default for AdventureData {
    fn default() -> Self {
        Self {
            rarities: Vec::new(),
            max_sockets: 4,
            affixes: Vec::new(),
            feminine: Vec::new(),
            plural: Vec::new(),
            qualities: Vec::new(),
            gems: Vec::new(),
            tiers: Vec::new(),
            augmentations: Vec::new(),
            max_picks: 3,
            invader_health: 1.5,
            invader_color: (1.0, 0.3, 0.9),
            elite_min_rarity: 0,
            invader_min_rarity: 0,
            elite_gem: GemDrop::default(),
            invader_gem: GemDrop::default(),
            loot: Vec::new(),
            enchantments: Vec::new(),
            max_enchantments: 4,
            charms: Vec::new(),
            gem_by_item: HashMap::new(),
        }
    }
}

/// `true` si l'identifiant est dans l'espace de noms (`core` : sans préfixe permis).
fn scoped_ok(id: &str, namespace: &str, bare_in_core: bool) -> bool {
    id.starts_with(&format!("{namespace}:")) || (bare_in_core && namespace == "core")
}

impl AdventureData {
    /// Le jeu de base intégré au binaire (sans mod).
    pub fn builtin() -> Self {
        let mut data = Self::default();
        data.add_source("core", "rarities", BUILTIN_RARITIES, "rarities.ron intégré")
            .and_then(|()| data.add_source("core", "affixes", BUILTIN_AFFIXES, "affixes.ron"))
            .and_then(|()| data.add_source("core", "gems", BUILTIN_GEMS, "gems.ron"))
            .and_then(|()| data.add_source("core", "tiers", BUILTIN_TIERS, "tiers.ron"))
            .and_then(|()| data.add_source("core", "elites", BUILTIN_ELITES, "elites.ron"))
            .and_then(|()| {
                data.add_source(
                    "core",
                    "enchantments",
                    BUILTIN_ENCHANTMENTS,
                    "enchantments.ron",
                )
            })
            .and_then(|()| data.add_source("core", "charms", BUILTIN_CHARMS, "charms.ron"))
            .expect("les données d'aventure intégrées sont valides");
        data
    }

    /// `assets/core/*.ron`, puis les fichiers de chaque mod dans l'ordre de chargement.
    pub fn load(assets: &Path, mods: &[(String, PathBuf)]) -> Result<Self> {
        let mut data = Self::default();
        let mut dirs = vec![("core".to_string(), assets.join("core"))];
        dirs.extend(mods.iter().cloned());
        for (namespace, dir) in dirs {
            for kind in [
                "rarities",
                "affixes",
                "gems",
                "tiers",
                "elites",
                "enchantments",
                "charms",
            ] {
                let path = dir.join(format!("{kind}.ron"));
                if !path.exists() {
                    continue;
                }
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("lecture de {}", path.display()))?;
                data.add_source(&namespace, kind, &text, &path.display().to_string())?;
            }
        }
        Ok(data)
    }

    /// Ajoute un fichier (`kind` : `rarities`, `affixes`, `gems`, `tiers` ou `elites`).
    pub fn add_source(
        &mut self,
        namespace: &str,
        kind: &str,
        text: &str,
        origin: &str,
    ) -> Result<()> {
        let parse_err = || format!("{origin} : fichier RON invalide");
        match kind {
            "rarities" => {
                let file: RaritiesFile = parse(text).with_context(parse_err)?;
                if let Some(max) = file.max_sockets {
                    self.max_sockets = max;
                }
                for rarity in file.rarities {
                    if !scoped_ok(&rarity.id, namespace, true) {
                        bail!(
                            "{origin} : la rareté « {} » doit commencer par « {namespace}: »",
                            rarity.id
                        );
                    }
                    if rarity.weight <= 0.0
                        || rarity.affixes.0 > rarity.affixes.1
                        || rarity.sockets.0 > rarity.sockets.1
                    {
                        bail!(
                            "{origin} : la rareté « {} » a des nombres incohérents",
                            rarity.id
                        );
                    }
                    if self.rarities.iter().any(|r| r.id == rarity.id) {
                        bail!(
                            "{origin} : la rareté « {} » est définie deux fois",
                            rarity.id
                        );
                    }
                    self.rarities.push(rarity);
                }
            }
            "affixes" => {
                let file: AffixesFile = parse(text).with_context(parse_err)?;
                self.feminine.extend(file.feminine);
                self.plural.extend(file.plural);
                for entry in file.affixes {
                    if !scoped_ok(&entry.id, namespace, false) {
                        bail!(
                            "{origin} : l'affixe « {} » doit commencer par « {namespace}: »",
                            entry.id
                        );
                    }
                    if self.affixes.iter().any(|a| a.id == entry.id) {
                        bail!("{origin} : l'affixe « {} » est défini deux fois", entry.id);
                    }
                    if entry.flat.is_empty() && entry.percent.is_empty() {
                        bail!(
                            "{origin} : l'affixe « {} » n'a ni `flat` ni `percent`",
                            entry.id
                        );
                    }
                    let attribute = attribute(&entry.attribute)
                        .with_context(|| format!("{origin} : affixe « {} »", entry.id))?;
                    self.affixes.push(AffixDef {
                        id: entry.id,
                        place: entry.place,
                        categories: entry.categories,
                        attribute,
                        flat: entry.flat,
                        percent: entry.percent,
                        weight: entry.weight,
                        min_rarity: entry.min_rarity,
                    });
                }
            }
            "gems" => {
                let file: GemsFile = parse(text).with_context(parse_err)?;
                if self.qualities.is_empty() {
                    self.qualities = file.qualities;
                }
                for entry in file.gems {
                    if !scoped_ok(&entry.id, namespace, false) {
                        bail!(
                            "{origin} : la gemme « {} » doit commencer par « {namespace}: »",
                            entry.id
                        );
                    }
                    if self.gems.iter().any(|g| g.id == entry.id) {
                        bail!("{origin} : la gemme « {} » est définie deux fois", entry.id);
                    }
                    if !entry.item.contains("{q}") {
                        bail!(
                            "{origin} : la gemme « {} » : `item` doit contenir {{q}}",
                            entry.id
                        );
                    }
                    let mut bonuses = Vec::new();
                    for bonus in entry.bonuses {
                        let attribute = attribute(&bonus.attribute)
                            .with_context(|| format!("{origin} : gemme « {} »", entry.id))?;
                        let count = self.qualities.len();
                        let fits = |v: &Vec<f32>| v.is_empty() || v.len() == count;
                        if !fits(&bonus.flat)
                            || !fits(&bonus.percent)
                            || (bonus.flat.is_empty() && bonus.percent.is_empty())
                        {
                            bail!(
                                "{origin} : gemme « {} » : il faut une valeur par qualité ({count})",
                                entry.id
                            );
                        }
                        bonuses.push(GemBonus {
                            categories: bonus.categories,
                            attribute,
                            flat: bonus.flat,
                            percent: bonus.percent,
                        });
                    }
                    self.gems.push(GemKind {
                        id: entry.id,
                        name: entry.name,
                        feminine: entry.feminine,
                        item: entry.item,
                        color: entry.color,
                        bonuses,
                    });
                }
                self.index_gems();
            }
            "tiers" => {
                let tiers: Vec<TierDef> = parse(text).with_context(parse_err)?;
                for tier in tiers {
                    if !scoped_ok(&tier.id, namespace, true) {
                        bail!(
                            "{origin} : le palier « {} » doit commencer par « {namespace}: »",
                            tier.id
                        );
                    }
                    if self.tiers.iter().any(|t| t.id == tier.id) {
                        bail!("{origin} : le palier « {} » est défini deux fois", tier.id);
                    }
                    self.tiers.push(tier);
                }
            }
            "elites" => {
                let file: ElitesFile = parse(text).with_context(parse_err)?;
                if let Some(n) = file.max_picks {
                    self.max_picks = n;
                }
                if let Some(h) = file.invader_health {
                    self.invader_health = h;
                }
                if let Some(c) = file.invader_color {
                    self.invader_color = c;
                }
                if let Some(r) = file.elite_min_rarity {
                    self.elite_min_rarity = r;
                }
                if let Some(r) = file.invader_min_rarity {
                    self.invader_min_rarity = r;
                }
                if let Some(g) = file.elite_gem {
                    self.elite_gem = g;
                }
                if let Some(g) = file.invader_gem {
                    self.invader_gem = g;
                }
                self.loot.extend(file.loot);
                for aug in file.augmentations {
                    if !scoped_ok(&aug.id, namespace, true) {
                        bail!(
                            "{origin} : l'augmentation « {} » doit commencer par « {namespace}: »",
                            aug.id
                        );
                    }
                    if self.augmentations.iter().any(|a| a.id == aug.id) {
                        bail!(
                            "{origin} : l'augmentation « {} » est définie deux fois",
                            aug.id
                        );
                    }
                    self.augmentations.push(aug);
                }
            }
            "enchantments" => crate::enchanting::data::add_source(self, namespace, text, origin)?,
            "charms" => crate::adventure::charms::add_source(self, namespace, text, origin)?,
            other => bail!("{origin} : sorte de données inconnue « {other} »"),
        }
        Ok(())
    }

    fn index_gems(&mut self) {
        self.gem_by_item.clear();
        for (k, kind) in self.gems.iter().enumerate() {
            for q in 0..self.qualities.len() {
                self.gem_by_item.insert(kind.item_key(q), (k, q));
            }
        }
    }

    /// Les objets cités existent-ils (matériaux de rareté, gemmes) ? Appelé une fois les objets
    /// chargés.
    pub fn validate_items(&self, items: &ItemRegistry) -> Result<()> {
        for rarity in &self.rarities {
            if items.id(&rarity.shard).is_none() {
                bail!(
                    "rareté « {} » : matériau « {} » inconnu",
                    rarity.id,
                    rarity.shard
                );
            }
        }
        for (key, _) in &self.loot {
            if items.id(key).is_none() {
                bail!("butin à affixes : objet « {key} » inconnu");
            }
        }
        for kind in &self.gems {
            for q in 0..self.qualities.len() {
                let key = kind.item_key(q);
                if items.id(&key).is_none() {
                    bail!("gemme « {} » : objet « {key} » inconnu", kind.id);
                }
            }
        }
        Ok(())
    }

    pub fn rarity(&self, index: u8) -> Option<&Rarity> {
        self.rarities.get(usize::from(index))
    }

    /// Le numéro d'une rareté d'après son identifiant.
    pub fn rarity_index(&self, id: &str) -> Option<u8> {
        self.rarities
            .iter()
            .position(|r| r.id == id)
            .and_then(|i| u8::try_from(i).ok())
    }

    pub fn affix(&self, id: &str) -> Option<&AffixDef> {
        self.affixes.iter().find(|a| a.id == id)
    }

    /// La gemme que cet objet représente : (sorte, qualité de 0 à 4).
    pub fn gem_of(&self, item_key: &str) -> Option<(&GemKind, usize)> {
        self.gem_by_item
            .get(item_key)
            .map(|&(k, q)| (&self.gems[k], q))
    }

    /// L'enchantement de ce numéro (celui qu'on sauvegarde).
    pub fn enchant_by_number(&self, number: u8) -> Option<&crate::enchanting::data::EnchantDef> {
        self.enchantments.iter().find(|e| e.number == number)
    }

    pub fn enchant_by_id(&self, id: &str) -> Option<&crate::enchanting::data::EnchantDef> {
        self.enchantments.iter().find(|e| e.id == id)
    }

    /// Le charme que fait cet objet.
    pub fn charm_of(&self, item_key: &str) -> Option<&crate::adventure::charms::CharmDef> {
        self.charms.iter().find(|c| c.item == item_key)
    }

    pub fn tier(&self, index: u8) -> Option<&TierDef> {
        self.tiers.get(usize::from(index))
    }
}

/// Lit du RON où un champ facultatif s'écrit sans `Some(...)` (comme `entities.ron`).
fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ron::error::SpannedError> {
    ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
}

fn attribute(key: &str) -> Result<Attribute> {
    Attribute::ALL
        .into_iter()
        .find(|a| a.key() == key)
        .with_context(|| format!("attribut inconnu « {key} »"))
}

static CURRENT: LazyLock<RwLock<Arc<AdventureData>>> =
    LazyLock::new(|| RwLock::new(Arc::new(AdventureData::builtin())));

/// Les données du jeu ouvert (le jeu de base intégré tant qu'on n'a rien installé).
pub fn current() -> Arc<AdventureData> {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Installe les données chargées (core et mods).
pub fn install(data: Arc<AdventureData>) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = data;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_core_data_loads_with_five_rarities_five_tiers_and_six_gems() {
        let data = AdventureData::builtin();
        assert_eq!(data.rarities.len(), 5);
        assert_eq!(
            data.rarities
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["common", "uncommon", "rare", "epic", "mythic"]
        );
        assert_eq!(data.tiers.len(), 5);
        assert_eq!(data.gems.len(), 6);
        assert_eq!(data.qualities.len(), 5);
        assert!(data.affixes.len() >= 15);
        assert!(data.augmentations.len() >= 4);
        // Chaque affixe a une valeur pour chaque rareté du jeu de base.
        for affix in &data.affixes {
            for r in 0..5 {
                assert!(
                    affix.flat_range(r).is_some() || affix.percent_range(r).is_some(),
                    "{}",
                    affix.id
                );
            }
        }
    }

    #[test]
    fn the_disk_files_match_the_embedded_ones_and_the_items_exist() {
        let items = engine_voxel::ItemRegistry::builtin(&crate::test_registry()).unwrap();
        let disk = AdventureData::load(&repo().join("assets"), &[]).unwrap();
        assert_eq!(disk, AdventureData::builtin());
        disk.validate_items(&items).unwrap();
    }

    #[test]
    fn a_mod_appends_rarities_affixes_gems_tiers_and_augmentations() {
        let mut data = AdventureData::builtin();
        data.add_source(
            "tests",
            "rarities",
            r#"(rarities: [(id: "tests:relic", name: "Relique", color: (1, 2, 3), weight: 0.5,
                affixes: (5, 6), sockets: (3, 3), shard: "core:shard_mythic",
                salvage: (shards: (1, 1)), reforge: (shards: 9, levels: 20))])"#,
            "t",
        )
        .unwrap();
        assert_eq!(data.rarity_index("tests:relic"), Some(5));
        data.add_source(
            "tests",
            "affixes",
            r#"(affixes: [(id: "tests:lucky", place: Suffix("de la chance"), categories: ["weapon"],
                attribute: "loot_chance", flat: [(0.1, 0.2)], weight: 5)])"#,
            "t",
        )
        .unwrap();
        // Une seule valeur : elle sert à toutes les raretés.
        assert_eq!(
            data.affix("tests:lucky").unwrap().flat_range(5),
            Some((0.1, 0.2))
        );
        data.add_source(
            "tests",
            "gems",
            r#"(gems: [(id: "tests:pearl", name: "Perle", feminine: true, item: "tests:pearl_{q}",
                color: (9, 9, 9), bonuses: [(categories: ["weapon"], attribute: "damage",
                flat: [1.0, 2.0, 3.0, 4.0, 5.0])])])"#,
            "t",
        )
        .unwrap();
        assert_eq!(data.gem_of("tests:pearl_3").unwrap().1, 2);
        assert!(data.gem_of("core:gem_ruby_5").is_some());
        data.add_source(
            "tests",
            "tiers",
            r#"[(id: "tests:beyond", name: "Au-delà", description: "", health: 5.0, damage: 5.0,
                rarity_boost: 3.0, elite_chance: 0.5, drop_chance: 0.5, invader: (30, 60),
                unlock: [Kills(1000)])]"#,
            "t",
        )
        .unwrap();
        assert_eq!(data.tiers.len(), 6);
        data.add_source(
            "tests",
            "elites",
            r#"(augmentations: [(id: "tests:giant", title: "géant", color: (1.0, 1.0, 1.0),
                effect: Health(3.0), weight: 1)])"#,
            "t",
        )
        .unwrap();
        assert_eq!(data.augmentations.len(), 6);
    }

    #[test]
    fn bad_data_is_refused_with_a_reason() {
        let mut data = AdventureData::builtin();
        let err = |text: &str, kind: &str, data: &mut AdventureData| {
            data.add_source("tests", kind, text, "t")
                .unwrap_err()
                .to_string()
        };
        // Un identifiant hors de l'espace de noms.
        assert!(
            err(
                r#"(affixes: [(id: "core:x", place: Suffix("x"), categories: [], attribute: "damage", flat: [(1.0, 2.0)], weight: 1)])"#,
                "affixes",
                &mut data
            )
            .contains("doit commencer par")
        );
        // Un attribut inconnu.
        assert!(format!(
            "{:#}",
            data.add_source(
                "tests",
                "affixes",
                r#"(affixes: [(id: "tests:x", place: Suffix("x"), categories: [], attribute: "magie", flat: [(1.0, 2.0)], weight: 1)])"#,
                "t"
            )
            .unwrap_err()
        )
        .contains("attribut inconnu"));
        // Une gemme sans une valeur par qualité.
        assert!(
            err(
                r#"(gems: [(id: "tests:g", name: "G", item: "tests:g_{q}", color: (0, 0, 0), bonuses: [(categories: ["weapon"], attribute: "damage", flat: [1.0])])])"#,
                "gems",
                &mut data
            )
            .contains("une valeur par qualité")
        );
        // Une rareté définie deux fois, un fichier illisible.
        assert!(
            err(
                r#"(rarities: [(id: "tests:a", name: "A", color: (0, 0, 0), weight: 1.0, affixes: (1, 1), sockets: (0, 0), shard: "core:shard_common", salvage: (shards: (1, 1)), reforge: (shards: 1, levels: 1)),
                            (id: "tests:a", name: "A", color: (0, 0, 0), weight: 1.0, affixes: (1, 1), sockets: (0, 0), shard: "core:shard_common", salvage: (shards: (1, 1)), reforge: (shards: 1, levels: 1))])"#,
                "rarities",
                &mut data
            )
            .contains("deux fois")
        );
        assert!(err("pas du RON", "tiers", &mut data).contains("invalide"));
    }
}
