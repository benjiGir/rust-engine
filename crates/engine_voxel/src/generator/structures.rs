//! Les structures : villages, ruines, donjons et mines abandonnées.
//!
//! # Où elles sont
//!
//! Comme les arbres, leurs emplacements ne dépendent que de la graine : le monde est découpé
//! en cases de `spacing` chunks de côté, chaque case tire (au hasard de la graine) si elle
//! contient une structure et où. Un chunk regarde les cases autour de lui, dessine sa part de
//! chaque structure qui le touche, et n'écrit que les blocs qui tombent chez lui. Le voisin
//! fait le même calcul : pas de coupure, aucun chunk ne parle à un autre.
//!
//! # Comment elles sont dessinées
//!
//! Une structure est d'abord un **plan** ([`Blueprint`]) : une liste de blocs à poser (dans
//! l'ordre : un bloc posé plus tard remplace un plus ancien), des coffres avec leur butin, et
//! les endroits où des habitants apparaissent. Le plan ne dépend que de la graine de la
//! structure et du terrain (la hauteur des colonnes), qui est lui aussi déterministe : chaque
//! chunk qui le recalcule obtient exactement le même.
//!
//! Les bâtiments sont dessinés dans un repère local (la porte du côté `z = 0`), puis tournés
//! pour que la porte regarde le chemin ([`Frame`]).

mod graph;

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use glam::IVec3;
use serde::Deserialize;

use super::{SEA_LEVEL, TerrainGenerator, hash3};
use crate::LAVA_LEVEL;
use crate::biome::{BiomeId, BiomeRegistry};
use crate::block::BlockId;
use crate::capabilities::{Inventory, ItemStack};
use crate::chunk::{BlockEntity, CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;
use crate::trees::Rng64;
use crate::world::World;

const BUILTIN_STRUCTURES: &str = include_str!("../../../../assets/core/structures.ron");

// ----- Les données -----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StructureKind {
    Village,
    Ruin,
    Dungeon,
    Mine,
    /// Un plan décrit dans les données (`templates` de `structures.ron`) : voir [`Template`].
    Template,
    /// Un donjon : des salles reliées en graphe, enterrées (`dungeons` de `structures.ron`).
    Graph,
}

impl StructureKind {
    /// Jusqu'où une structure de cette sorte s'étend depuis son point (en blocs).
    pub fn radius(self) -> i32 {
        match self {
            Self::Village => 46,
            Self::Ruin => 12,
            Self::Dungeon => 10,
            Self::Mine => 70,
            // Le rayon d'un plan dépend de sa taille : `StructureDef::radius`.
            Self::Template | Self::Graph => 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructureEntry {
    id: String,
    kind: StructureKind,
    spacing: i32,
    chance: f32,
    #[serde(default)]
    biomes: Vec<String>,
    /// La table de butin (obligatoire sauf pour un plan, dont les coffres nomment la leur).
    #[serde(default)]
    loot: String,
    /// Pour `kind: Template` : le nom du plan.
    #[serde(default)]
    template: Option<String>,
    /// Pour `kind: Graph` : le nom du donjon.
    #[serde(default)]
    dungeon: Option<String>,
}

/// Un plan dans un fichier : une palette (un caractère → un bloc), des couches de texte
/// (de bas en haut ; chaque couche a une ligne par `z` et un caractère par `x`), des coffres.
///
/// Dans une couche : un caractère de la palette pose son bloc, `.` pose de l'air, un espace
/// laisse le terrain.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateEntry {
    palette: HashMap<char, String>,
    layers: Vec<Vec<String>>,
    /// Les coffres : (x, y, z) dans le plan, et leur table de butin.
    #[serde(default)]
    chests: Vec<((i32, i32, i32), String)>,
    /// Le bloc qui comble le vide sous le plan, jusqu'au sol (de la pierre taillée par défaut).
    #[serde(default)]
    foundation: Option<String>,
    /// Dégager l'air dans le volume du plan avant de le poser (oui par défaut) : sans cela, un
    /// plan posé sur une pente resterait enfoui.
    #[serde(default = "yes")]
    clear: bool,
    /// Tourner le plan au hasard d'un quart de tour (oui par défaut). À éteindre si le plan
    /// contient des blocs à états orientés (une rotation ne les tourne pas).
    #[serde(default = "yes")]
    rotate: bool,
}

fn yes() -> bool {
    true
}

/// Ce qu'un caractère d'un plan pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Air,
    Block(BlockId),
}

/// Un plan de structure, résolu : prêt à être posé par le générateur.
#[derive(Debug, Clone)]
pub struct Template {
    w: i32,
    d: i32,
    h: i32,
    /// (x, y, z) dans le plan, et ce qu'on y pose (les espaces n'y sont pas).
    cells: Vec<(IVec3, Cell)>,
    chests: Vec<(IVec3, String)>,
    foundation: BlockId,
    clear: bool,
    rotate: bool,
}

impl Template {
    /// La plus grande dimension horizontale, en blocs.
    pub fn size(&self) -> i32 {
        self.w.max(self.d)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChestLootEntry {
    pub item: String,
    pub count: (u32, u32),
    pub weight: u32,
    /// Un objet à affixes (phase « aventure ») : le coffre le range « à tirer » (voir
    /// [`pending_loot`]), et le jeu en tire la rareté et les affixes à sa première ouverture.
    #[serde(default)]
    pub affixed: bool,
    /// La rareté minimale (son numéro) d'un objet `affixed`.
    #[serde(default)]
    pub min_rarity: u8,
}

/// Les deux bits hauts de `ItemStack::damage` qui disent « objet à affixes à tirer » : le moteur
/// ne connaît ni les raretés ni les affixes, il laisse donc au jeu le soin de les tirer. Le bit 31
/// seul est déjà pris (fiche d'objet) ; avec le bit 30, le reste du nombre est le tirage : la
/// rareté minimale dans les bits 16 à 19 et une graine de 16 bits dans les bits 0 à 15.
pub const PENDING_LOOT: u32 = 0xC000_0000;

/// Le `damage` d'un objet à affixes à tirer, pour cette graine et cette rareté minimale.
pub fn pending_loot(min_rarity: u8, seed: u16) -> u32 {
    PENDING_LOOT | (u32::from(min_rarity & 0xF) << 16) | u32::from(seed)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChestLoot {
    pub rolls: (u32, u32),
    pub entries: Vec<ChestLootEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuresFile {
    structures: Vec<StructureEntry>,
    #[serde(default)]
    loot: HashMap<String, ChestLoot>,
    #[serde(default)]
    templates: HashMap<String, TemplateEntry>,
    #[serde(default)]
    dungeons: HashMap<String, graph::DungeonEntry>,
}

/// Une sorte de structure et où elle peut apparaître.
#[derive(Debug, Clone)]
pub struct StructureDef {
    pub key: String,
    pub kind: StructureKind,
    pub spacing: i32,
    pub chance: f32,
    pub biomes: Vec<BiomeId>,
    pub loot: String,
    /// Jusqu'où elle s'étend depuis son point (en blocs).
    pub radius: i32,
    /// Son plan, pour `kind: Template`.
    pub template: Option<Template>,
    /// Son graphe de salles, pour `kind: Graph`.
    pub graph: Option<graph::GraphDef>,
}

/// Toutes les structures du jeu et leurs tables de butin.
#[derive(Debug, Clone)]
pub struct StructureSet {
    defs: Vec<StructureDef>,
    loot: HashMap<String, ChestLoot>,
}

/// Le nom d'une table de butin ou d'un plan : `ruin` dans le fichier de `core` reste `ruin`
/// (les tables du jeu n'ont pas d'espace de noms), `ruin` dans celui d'un mod devient `mod:ruin`.
fn scoped(name: &str, namespace: &str) -> String {
    if name.contains(':') || namespace == "core" {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

impl StructureSet {
    pub fn builtin(biomes: &BiomeRegistry) -> Result<Self> {
        Self::parse(BUILTIN_STRUCTURES, "structures.ron intégré", biomes)
    }

    /// Un fichier de structures seul, sans plans (le jeu de base n'en a pas).
    pub fn parse(text: &str, origin: &str, biomes: &BiomeRegistry) -> Result<Self> {
        let mut set = Self {
            defs: Vec::new(),
            loot: HashMap::new(),
        };
        set.add_source("core", text, origin, biomes, None)?;
        Ok(set)
    }

    /// Ajoute les structures, tables de butin et plans d'un fichier (`core`, puis chaque mod
    /// dans l'ordre). Un plan a besoin du registre de blocs pour résoudre sa palette.
    pub fn add_source(
        &mut self,
        namespace: &str,
        text: &str,
        origin: &str,
        biomes: &BiomeRegistry,
        blocks: Option<&BlockRegistry>,
    ) -> Result<()> {
        let file: StructuresFile = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .with_context(|| format!("{origin} : RON invalide"))?;
        for (name, table) in &file.loot {
            if table.entries.is_empty() || table.entries.iter().all(|e| e.weight == 0) {
                bail!("{origin} : la table « {name} » n'a aucune entrée");
            }
            let key = scoped(name, namespace);
            if self.loot.insert(key.clone(), table.clone()).is_some() {
                bail!("{origin} : la table de butin « {key} » est définie deux fois");
            }
        }
        // Les plans de ce fichier, résolus une fois.
        let mut templates: HashMap<String, Template> = HashMap::new();
        for (name, entry) in &file.templates {
            let blocks = blocks.with_context(|| {
                format!("{origin} : les plans (« {name} ») ont besoin du registre de blocs")
            })?;
            let template = self
                .resolve_template(namespace, name, entry, blocks, origin)
                .with_context(|| format!("{origin} : plan « {name} »"))?;
            templates.insert(scoped(name, namespace), template);
        }
        // Les donjons de ce fichier : leurs plans de salles sont ceux du fichier.
        let graphs = if file.dungeons.is_empty() {
            HashMap::new()
        } else {
            let blocks = blocks.with_context(|| {
                format!("{origin} : les donjons ont besoin du registre de blocs")
            })?;
            graph::resolve(namespace, &file.dungeons, &templates, blocks)
                .with_context(|| origin.to_string())?
        };
        for entry in file.structures {
            if entry.id.split_once(':').map(|(ns, _)| ns) != Some(namespace) {
                bail!(
                    "{origin} : la structure « {} » doit commencer par « {namespace}: »",
                    entry.id
                );
            }
            if self.defs.iter().any(|d| d.key == entry.id) {
                bail!(
                    "{origin} : la structure « {} » est définie deux fois",
                    entry.id
                );
            }
            if entry.spacing < 2 || !(0.0..=1.0).contains(&entry.chance) {
                bail!(
                    "{origin} : « {} » : spacing d'au moins 2 et chance entre 0 et 1",
                    entry.id
                );
            }
            let mut graph_def = None;
            let (loot, template, radius) = if entry.kind == StructureKind::Graph {
                let name = entry
                    .dungeon
                    .as_deref()
                    .with_context(|| format!("{origin} : « {} » : `dungeon` manque", entry.id))?;
                let key = scoped(name, namespace);
                let def = graphs.get(&key).cloned().with_context(|| {
                    format!(
                        "{origin} : « {} » : donjon « {key} » inconnu (dans ce fichier)",
                        entry.id
                    )
                })?;
                let radius = def.radius();
                if entry.spacing * CHUNK_SIZE as i32 <= radius {
                    bail!(
                        "{origin} : « {} » : `spacing` d'au moins {} chunks (le donjon s'étend sur {radius} blocs)",
                        entry.id,
                        radius / CHUNK_SIZE as i32 + 1
                    );
                }
                graph_def = Some(def);
                (String::new(), None, radius)
            } else if entry.kind == StructureKind::Template {
                let name = entry
                    .template
                    .as_deref()
                    .with_context(|| format!("{origin} : « {} » : `template` manque", entry.id))?;
                let key = scoped(name, namespace);
                let template = templates.get(&key).cloned().with_context(|| {
                    format!(
                        "{origin} : « {} » : plan « {key} » inconnu (dans ce fichier)",
                        entry.id
                    )
                })?;
                let radius = template.size() + 2;
                (String::new(), Some(template), radius)
            } else {
                let key = scoped(&entry.loot, namespace);
                if !self.loot.contains_key(&key) {
                    bail!(
                        "{origin} : « {} » : la table de butin « {} » n'existe pas",
                        entry.id,
                        entry.loot
                    );
                }
                (key, None, entry.kind.radius())
            };
            let ids = entry
                .biomes
                .iter()
                .map(|key| {
                    biomes
                        .id(key)
                        .with_context(|| format!("{origin} : biome « {key} » inconnu"))
                })
                .collect::<Result<Vec<_>>>()?;
            self.defs.push(StructureDef {
                key: entry.id,
                kind: entry.kind,
                spacing: entry.spacing,
                chance: entry.chance,
                biomes: ids,
                loot,
                radius,
                template,
                graph: graph_def,
            });
        }
        // Les paliers d'un donjon sont des tables qui doivent exister.
        for def in &self.defs {
            for table in def.graph.iter().flat_map(|g| g.tiers()) {
                if !self.loot.contains_key(table) {
                    bail!(
                        "{origin} : le donjon de « {} » : la table de butin « {table} » n'existe pas",
                        def.key
                    );
                }
            }
        }
        // Les coffres des plans nomment des tables qui doivent exister (ici ou dans `core`).
        // `@tier`, `@treasure` et `@boss` sont choisies par le donjon qui pose le plan.
        for def in &self.defs {
            for (_, table) in def.template.iter().flat_map(|t| &t.chests) {
                if !table.starts_with('@') && !self.loot.contains_key(table) {
                    bail!(
                        "{origin} : le plan de « {} » : la table de butin « {table} » n'existe pas",
                        def.key
                    );
                }
            }
        }
        Ok(())
    }

    fn resolve_template(
        &self,
        namespace: &str,
        name: &str,
        entry: &TemplateEntry,
        blocks: &BlockRegistry,
        origin: &str,
    ) -> Result<Template> {
        let block = |key: &str| {
            let key = qualify_block(key, namespace);
            blocks
                .id(&key)
                .with_context(|| format!("bloc inconnu « {key} »"))
        };
        if entry.layers.is_empty() {
            bail!("aucune couche");
        }
        let d = entry.layers.iter().map(Vec::len).max().unwrap_or(0) as i32;
        let w = entry
            .layers
            .iter()
            .flatten()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0) as i32;
        if w == 0 || d == 0 || w > 64 || d > 64 || entry.layers.len() > 64 {
            bail!("taille invalide (1 à 64 blocs par côté, 64 couches au plus)");
        }
        let mut cells = Vec::new();
        for (y, layer) in entry.layers.iter().enumerate() {
            for (z, row) in layer.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    let p = IVec3::new(x as i32, y as i32, z as i32);
                    match c {
                        ' ' => {}
                        '.' => cells.push((p, Cell::Air)),
                        c => {
                            let key = entry.palette.get(&c).with_context(|| {
                                format!("« {c} » (couche {y}, ligne {z}) n'est pas dans la palette")
                            })?;
                            cells.push((p, Cell::Block(block(key)?)));
                        }
                    }
                }
            }
        }
        let mut chests = Vec::new();
        for ((x, y, z), table) in &entry.chests {
            if *x < 0 || *y < 0 || *z < 0 || *x >= w || *y >= entry.layers.len() as i32 || *z >= d {
                bail!("un coffre est hors du plan ({x}, {y}, {z})");
            }
            chests.push((
                IVec3::new(*x, *y, *z),
                if table.starts_with('@') {
                    table.clone()
                } else {
                    scoped(table, namespace)
                },
            ));
        }
        let _ = (name, origin);
        Ok(Template {
            w,
            d,
            h: entry.layers.len() as i32,
            cells,
            chests,
            foundation: match &entry.foundation {
                Some(key) => block(key)?,
                None => block("core:cobblestone")?,
            },
            clear: entry.clear,
            rotate: entry.rotate,
        })
    }

    pub fn defs(&self) -> &[StructureDef] {
        &self.defs
    }

    pub fn loot(&self, name: &str) -> Option<&ChestLoot> {
        self.loot.get(name)
    }

    /// Tous les objets cités par les tables (pour vérifier qu'ils existent).
    pub fn loot_items(&self) -> impl Iterator<Item = &str> {
        self.loot
            .values()
            .flat_map(|t| t.entries.iter().map(|e| e.item.as_str()))
    }
}

/// Un nom de bloc d'un plan : sans « : », il est dans l'espace de noms du fichier.
fn qualify_block(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

// ----- Les blocs utilisés -----

const KEYS: &[&str] = &[
    "core:stone",
    "core:cobblestone",
    "core:gravel",
    "core:sand",
    "core:sandstone",
    "core:dirt",
    "core:grass",
    "core:oak_planks",
    "core:birch_planks",
    "core:spruce_planks",
    "core:acacia_planks",
    "core:jungle_planks",
    "core:oak_log",
    "core:birch_log",
    "core:spruce_log",
    "core:acacia_log",
    "core:jungle_log",
    "core:glass",
    "core:torch",
    "core:chest",
    "core:bed",
    "core:workbench",
    "core:furnace",
    "core:water",
    "core:farmland_wet",
    "core:wheat_3",
    "core:carrot_3",
    "core:potato_3",
    "core:coal_ore",
    "core:iron_ore",
    "core:copper_ore",
];

/// Les blocs des structures, retrouvés une fois par leur nom.
#[derive(Debug, Clone)]
pub(super) struct StructureBlocks(HashMap<&'static str, BlockId>);

impl StructureBlocks {
    pub fn resolve(registry: &BlockRegistry) -> Option<Self> {
        let mut map = HashMap::new();
        for &key in KEYS {
            map.insert(key, registry.id(key)?);
        }
        Some(Self(map))
    }

    fn get(&self, key: &str) -> BlockId {
        self.0.get(key).copied().unwrap_or_default()
    }
}

// ----- Le plan -----

/// Un habitant à faire apparaître : où, et quelle créature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pub pos: IVec3,
    pub mob: &'static str,
}

/// Une structure à poser : des blocs, des coffres, des habitants.
#[derive(Debug, Clone, Default)]
pub struct Blueprint {
    ops: Vec<(IVec3, BlockId)>,
    chests: Vec<(IVec3, Inventory)>,
    pub spots: Vec<Spot>,
    min: IVec3,
    max: IVec3,
}

impl Blueprint {
    fn note(&mut self, p: IVec3) {
        if self.ops.is_empty() && self.chests.is_empty() {
            self.min = p;
            self.max = p;
        } else {
            self.min = self.min.min(p);
            self.max = self.max.max(p);
        }
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Les coffres du plan : (position, inventaire).
    pub fn chests(&self) -> &[(IVec3, Inventory)] {
        &self.chests
    }

    /// Les blocs posés, dans l'ordre.
    pub fn blocks(&self) -> &[(IVec3, BlockId)] {
        &self.ops
    }

    /// Écrit dans `chunk` la part du plan qui lui appartient.
    /// `active` dit si un bloc est une machine : elle reçoit un état vide, pour que le jeu la
    /// réveille au chargement du chunk (un piège, un générateur de créatures posé par le monde).
    pub fn apply(
        &self,
        chunk: &mut Chunk,
        chunk_pos: IVec3,
        chest: BlockId,
        active: &dyn Fn(BlockId) -> bool,
    ) {
        let origin = World::chunk_origin(chunk_pos);
        let size = IVec3::splat(CHUNK_SIZE as i32);
        if self.ops.is_empty()
            || self.max.cmplt(origin).any()
            || self.min.cmpge(origin + size).any()
        {
            return;
        }
        let inside = |p: IVec3| {
            let local = p - origin;
            (local.cmpge(IVec3::ZERO).all() && local.cmplt(size).all()).then_some(local)
        };
        for &(p, block) in &self.ops {
            if let Some(l) = inside(p) {
                chunk.set(l.x as usize, l.y as usize, l.z as usize, block);
                // Un bloc posé sur un coffre efface son état ; une machine en reçoit un, vide.
                let state = active(block).then(BlockEntity::default);
                chunk.set_block_entity(l.x as usize, l.y as usize, l.z as usize, state);
            }
        }
        for (p, inventory) in &self.chests {
            if let Some(l) = inside(*p) {
                let (x, y, z) = (l.x as usize, l.y as usize, l.z as usize);
                chunk.set(x, y, z, chest);
                let mut entity = BlockEntity::default();
                entity.caps_mut().inventory = Some(inventory.clone());
                chunk.set_block_entity(x, y, z, Some(entity));
            }
        }
    }
}

/// Un repère local pour un bâtiment : `(lx, lz)` dans un rectangle `w × d` (la porte est du
/// côté `lz = 0`), tourné de `k` quarts de tour, posé en `origin` (son coin minimal).
#[derive(Debug, Clone, Copy)]
struct Frame {
    origin: IVec3,
    k: u8,
    w: i32,
    d: i32,
}

impl Frame {
    /// Les dimensions dans le monde (x, z).
    fn extent(&self) -> (i32, i32) {
        if self.k.is_multiple_of(2) {
            (self.w, self.d)
        } else {
            (self.d, self.w)
        }
    }

    fn map(&self, lx: i32, ly: i32, lz: i32) -> IVec3 {
        let (w, d) = (self.w, self.d);
        let (x, z) = match self.k % 4 {
            0 => (lx, lz),
            1 => (lz, w - 1 - lx),
            2 => (w - 1 - lx, d - 1 - lz),
            _ => (d - 1 - lz, lx),
        };
        IVec3::new(self.origin.x + x, ly, self.origin.z + z)
    }
}

/// Les matériaux d'un style de village.
#[derive(Debug, Clone, Copy)]
struct Style {
    planks: BlockId,
    log: BlockId,
    wall: BlockId,
    roof: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Building {
    House,
    Smithy,
    Farm,
}

struct Builder<'a> {
    world: &'a TerrainGenerator,
    b: &'a StructureBlocks,
    rng: Rng64,
    bp: Blueprint,
    heights: HashMap<(i32, i32), i32>,
    loot: &'a ChestLoot,
}

impl Builder<'_> {
    fn id(&self, key: &str) -> BlockId {
        self.b.get(key)
    }

    fn height(&mut self, x: i32, z: i32) -> i32 {
        let terrain = self.world;
        *self
            .heights
            .entry((x, z))
            .or_insert_with(|| terrain.column(x, z).height)
    }

    fn put(&mut self, p: IVec3, block: BlockId) {
        self.bp.note(p);
        self.bp.ops.push((p, block));
    }

    fn set(&mut self, p: IVec3, key: &str) {
        let block = self.id(key);
        self.put(p, block);
    }

    fn air(&mut self, p: IVec3) {
        self.put(p, BlockId::AIR);
    }

    fn fill(&mut self, a: IVec3, b: IVec3, block: BlockId) {
        let (lo, hi) = (a.min(b), a.max(b));
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    self.put(IVec3::new(x, y, z), block);
                }
            }
        }
    }

    /// Pose un plan décrit dans les données, centré en (x, z), sur le sol, au besoin tourné.
    fn template(&mut self, x: i32, z: i32, t: &Template, set: &StructureSet) {
        let k = if t.rotate { self.rng.below(4) as u8 } else { 0 };
        // Le sol du plan : le point le plus haut de son emprise (il ne s'enfouit pas).
        let frame = Frame {
            origin: IVec3::ZERO,
            k,
            w: t.w,
            d: t.d,
        };
        let (ew, ed) = frame.extent();
        let (x0, z0) = (x - ew / 2, z - ed / 2);
        let ground = (0..ed)
            .flat_map(|dz| (0..ew).map(move |dx| (x0 + dx, z0 + dz)))
            .map(|(px, pz)| self.height(px, pz))
            .max()
            .unwrap_or(SEA_LEVEL);
        self.template_at(x, z, ground + 1, k, t, set, &|name| Some(name.to_string()));
    }

    /// Pose un plan centré en (x, z), son sol à la hauteur `y0`, tourné de `k` quarts de tour.
    /// `pick` choisit la table de butin de chaque coffre à partir du nom écrit dans le plan.
    #[allow(clippy::too_many_arguments)]
    fn template_at(
        &mut self,
        x: i32,
        z: i32,
        y0: i32,
        k: u8,
        t: &Template,
        set: &StructureSet,
        pick: &dyn Fn(&str) -> Option<String>,
    ) {
        let frame = Frame {
            origin: IVec3::ZERO,
            k,
            w: t.w,
            d: t.d,
        };
        let (ew, ed) = frame.extent();
        let (x0, z0) = (x - ew / 2, z - ed / 2);
        let frame = Frame {
            origin: IVec3::new(x0, 0, z0),
            ..frame
        };
        let at = |p: IVec3| {
            let mapped = frame.map(p.x, 0, p.z);
            IVec3::new(mapped.x, y0 + p.y, mapped.z)
        };
        if t.clear {
            for dz in 0..t.d {
                for dx in 0..t.w {
                    for dy in 0..t.h {
                        self.air(at(IVec3::new(dx, dy, dz)));
                    }
                }
            }
        }
        // Les fondations : sous chaque colonne dont la couche du bas pose un bloc, jusqu'au sol.
        for &(p, cell) in &t.cells {
            if p.y == 0 && matches!(cell, Cell::Block(_)) {
                let top = at(p);
                let h = self.height(top.x, top.z);
                for yy in (h + 1).max(y0 - 12)..y0 {
                    self.put(IVec3::new(top.x, yy, top.z), t.foundation);
                }
            }
        }
        for &(p, cell) in &t.cells {
            match cell {
                Cell::Air => self.air(at(p)),
                Cell::Block(block) => self.put(at(p), block),
            }
        }
        for (p, table) in &t.chests {
            if let Some(loot) = pick(table).and_then(|name| set.loot.get(&name)) {
                self.chest_with(at(*p), loot);
            }
        }
    }

    /// Un coffre rempli selon la table de butin de la structure.
    fn chest(&mut self, p: IVec3) {
        let table = self.loot;
        self.chest_with(p, table);
    }

    /// Un coffre rempli selon cette table de butin.
    fn chest_with(&mut self, p: IVec3, table: &ChestLoot) {
        let mut inventory = Inventory::new(crate::containers::CHEST_SLOTS);
        let rolls = table.rolls.0 + self.rng.below(table.rolls.1 - table.rolls.0 + 1);
        let total: u32 = table.entries.iter().map(|e| e.weight).sum();
        for _ in 0..rolls {
            let mut pick = self.rng.below(total.max(1));
            let Some(entry) = table.entries.iter().find(|e| {
                if pick < e.weight {
                    true
                } else {
                    pick -= e.weight;
                    false
                }
            }) else {
                continue;
            };
            let count = entry.count.0 + self.rng.below(entry.count.1 - entry.count.0 + 1);
            // Une case libre au hasard (au pire la première libre).
            let start = self.rng.below(inventory.slots.len() as u32) as usize;
            let len = inventory.slots.len();
            if let Some(slot) = (0..len)
                .map(|i| (start + i) % len)
                .find(|&i| inventory.slots[i].is_none())
            {
                let damage = if entry.affixed {
                    // Une graine tirée à part (pas dans `self.rng`) : le butin d'un coffre ne
                    // décale jamais le reste du plan, les structures déjà posées restent justes.
                    let at = (i64::from(p.x) * 73_856_093)
                        ^ (i64::from(p.y) * 19_349_663)
                        ^ (i64::from(p.z) * 83_492_791)
                        ^ (slot as i64 * 2_654_435_761);
                    pending_loot(
                        entry.min_rarity,
                        (Rng64::new(at as u64).next_u64() & 0xFFFF) as u16,
                    )
                } else {
                    0
                };
                inventory.slots[slot] = Some(ItemStack {
                    item: entry.item.clone(),
                    count: if entry.affixed { 1 } else { count },
                    damage,
                });
            }
        }
        self.bp.note(p);
        self.bp.chests.push((p, inventory));
    }

    /// Aplanit le terrain sous un rectangle (coordonnées monde, inclusives) : du remblai
    /// jusqu'à `y - 2`, de l'air au-dessus, et une couche de surface à `y - 1` sur la marge.
    fn flatten(&mut self, min: (i32, i32), max: (i32, i32), y: i32, margin: i32, clear: i32) {
        let (cobble, dirt, grass) = (
            self.id("core:cobblestone"),
            self.id("core:dirt"),
            self.id("core:grass"),
        );
        for z in min.1 - margin..=max.1 + margin {
            for x in min.0 - margin..=max.0 + margin {
                let h = self.height(x, z);
                let inside = (min.0..=max.0).contains(&x) && (min.1..=max.1).contains(&z);
                let (fill, top) = if inside {
                    (cobble, y - 2)
                } else {
                    (dirt, y - 2)
                };
                for yy in (h + 1)..=top {
                    self.put(IVec3::new(x, yy, z), fill);
                }
                if !inside {
                    self.put(IVec3::new(x, y - 1, z), grass);
                }
                let up = if inside { clear } else { 3 };
                for yy in y..=(y + up).max(h) {
                    self.air(IVec3::new(x, yy, z));
                }
            }
        }
    }

    /// Un morceau de chemin : le sol devient du gravier, dégagé au-dessus.
    fn path_cell(&mut self, x: i32, z: i32) {
        let h = self.height(x, z);
        if h < SEA_LEVEL {
            return;
        }
        let gravel = self.id("core:gravel");
        self.put(IVec3::new(x, h, z), gravel);
        for yy in 1..=3 {
            self.air(IVec3::new(x, h + yy, z));
        }
    }

    fn style(&self, biome: &str) -> Style {
        let (planks, log, wall, roof) = match biome {
            "core:taiga" | "core:snowy_tundra" | "core:snowy_taiga" | "core:mixed_forest"
            | "core:steppe" => (
                "core:spruce_planks",
                "core:spruce_log",
                "core:spruce_planks",
                "core:spruce_planks",
            ),
            "core:savanna" | "core:shrubland" => (
                "core:acacia_planks",
                "core:acacia_log",
                "core:acacia_planks",
                "core:acacia_planks",
            ),
            "core:desert" => (
                "core:acacia_planks",
                "core:acacia_log",
                "core:sandstone",
                "core:sandstone",
            ),
            "core:birch_forest" => (
                "core:birch_planks",
                "core:birch_log",
                "core:birch_planks",
                "core:spruce_planks",
            ),
            "core:jungle" => (
                "core:jungle_planks",
                "core:jungle_log",
                "core:jungle_planks",
                "core:jungle_planks",
            ),
            _ => (
                "core:oak_planks",
                "core:oak_log",
                "core:oak_planks",
                "core:spruce_planks",
            ),
        };
        Style {
            planks: self.id(planks),
            log: self.id(log),
            wall: self.id(wall),
            roof: self.id(roof),
        }
    }

    // ----- Bâtiments -----

    /// Une maison (ou une forge, ou une ruine). `floor` : le niveau où l'on marche.
    fn house(&mut self, f: Frame, floor: i32, style: Style, kind: Building, ruined: bool) {
        let (w, d) = (f.w, f.d);
        let wall_h = 4;
        if !ruined {
            let (ex, ez) = f.extent();
            let (x0, z0) = (f.origin.x, f.origin.z);
            self.flatten(
                (x0, z0),
                (x0 + ex - 1, z0 + ez - 1),
                floor,
                1,
                wall_h + d / 2 + 2,
            );
        }
        // Le sol, les murs et les coins.
        for lz in 0..d {
            for lx in 0..w {
                self.put(f.map(lx, floor - 1, lz), style.planks);
                let edge = lx == 0 || lx == w - 1 || lz == 0 || lz == d - 1;
                if !edge {
                    continue;
                }
                let corner = (lx == 0 || lx == w - 1) && (lz == 0 || lz == d - 1);
                for dy in 0..wall_h {
                    // Une ruine perd ses murs de plus en plus haut.
                    if ruined && dy >= 1 && self.rng.next_f32() < 0.18 * dy as f32 + 0.1 {
                        continue;
                    }
                    let block = if corner { style.log } else { style.wall };
                    self.put(f.map(lx, floor + dy, lz), block);
                }
            }
        }
        let glass = self.id("core:glass");
        if !ruined {
            // La porte (deux blocs de haut) et des fenêtres sur les trois autres côtés.
            for dy in 0..2 {
                self.air(f.map(w / 2, floor + dy, 0));
            }
            // Une torche contre le mur du fond.
            self.set(f.map(1, floor + 2, 2), "core:torch");
            for (lx, lz) in [
                (0, d / 2),
                (w - 1, d / 2),
                (w / 2, d - 1),
                (1, 0),
                (w - 2, 0),
            ] {
                self.put(f.map(lx, floor + 1, lz), glass);
            }
            // Le toit : deux pans, et des pignons pleins aux deux bouts.
            for k in 0..=((d - 1) / 2) {
                for lx in 0..w {
                    self.put(f.map(lx, floor + wall_h + k, k), style.roof);
                    self.put(f.map(lx, floor + wall_h + k, d - 1 - k), style.roof);
                }
                for lz in k..=(d - 1 - k) {
                    for lx in [0, w - 1] {
                        self.put(f.map(lx, floor + wall_h + k, lz), style.wall);
                    }
                }
            }
            // Le pignon plein au sommet.
            let mid = (d - 1) / 2;
            for lx in 0..w {
                self.put(f.map(lx, floor + wall_h + mid, mid), style.roof);
            }
        }
        // Le mobilier.
        let furnish = |this: &mut Self, lx, lz, key: &str| {
            this.set(f.map(lx, floor, lz), key);
        };
        match kind {
            Building::Smithy => {
                furnish(self, w - 2, d - 2, "core:furnace");
                furnish(self, w - 3, d - 2, "core:workbench");
                furnish(self, 1, d - 2, "core:furnace");
                self.chest(f.map(1, floor, 1));
                if !ruined {
                    self.bp.spots.push(Spot {
                        pos: f.map(w / 2, floor, d / 2),
                        mob: "core:smith",
                    });
                }
            }
            _ => {
                if ruined {
                    self.chest(f.map(w / 2, floor, d - 2));
                } else {
                    furnish(self, 1, d - 2, "core:bed");
                    furnish(self, w - 2, 1, "core:workbench");
                    if self.rng.next_f32() < 0.6 {
                        self.chest(f.map(w - 2, floor, d - 2));
                    }
                    if self.rng.next_f32() < 0.3 {
                        self.bp.spots.push(Spot {
                            pos: f.map(w / 2, floor, d / 2),
                            mob: "core:merchant",
                        });
                    }
                }
            }
        }
    }

    /// Un champ : des rangs de culture mûre séparés par un canal d'eau.
    fn farm(&mut self, f: Frame, y: i32) {
        let (ex, ez) = f.extent();
        let (x0, z0) = (f.origin.x, f.origin.z);
        self.flatten((x0, z0), (x0 + ex - 1, z0 + ez - 1), y + 1, 1, 3);
        let (soil, water) = (self.id("core:farmland_wet"), self.id("core:water"));
        let crops = ["core:wheat_3", "core:carrot_3", "core:potato_3"];
        let crop = crops[self.rng.below(3) as usize];
        let log = self.id("core:oak_log");
        for lz in 0..f.d {
            for lx in 0..f.w {
                let edge = lx == 0 || lx == f.w - 1 || lz == 0 || lz == f.d - 1;
                if edge {
                    // Une bordure basse en bois.
                    self.put(f.map(lx, y, lz), log);
                } else if lx == f.w / 2 {
                    self.put(f.map(lx, y, lz), water);
                } else {
                    self.put(f.map(lx, y, lz), soil);
                    self.set(f.map(lx, y + 1, lz), crop);
                }
            }
        }
        // Un passage dans la bordure, côté chemin, et un fermier.
        let door = f.map(f.w / 2, y, 0);
        let gravel = self.id("core:gravel");
        self.put(door, gravel);
        self.bp.spots.push(Spot {
            pos: f.map(f.w / 2 + 1, y + 1, 0),
            mob: "core:farmer",
        });
    }

    /// Le puits, au centre : une margelle en moellons et trois blocs d'eau.
    fn well(&mut self, c: IVec3) {
        self.flatten((c.x - 2, c.z - 2), (c.x + 2, c.z + 2), c.y + 1, 0, 5);
        let (cobble, water, log) = (
            self.id("core:cobblestone"),
            self.id("core:water"),
            self.id("core:oak_log"),
        );
        for dz in -1..=1 {
            for dx in -1..=1 {
                let p = c + IVec3::new(dx, 0, dz);
                if dx == 0 && dz == 0 {
                    for dy in -3..=0 {
                        self.put(p + IVec3::Y * dy, water);
                    }
                    for dy in -4..=-4 {
                        self.put(p + IVec3::Y * dy, cobble);
                    }
                } else {
                    self.put(p, cobble);
                }
            }
        }
        // Quatre poteaux et un toit.
        for (dx, dz) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            for dy in 1..=3 {
                self.put(c + IVec3::new(dx, dy, dz), log);
            }
        }
        let roof = self.id("core:spruce_planks");
        for dz in -1..=1 {
            for dx in -1..=1 {
                self.put(c + IVec3::new(dx, 4, dz), roof);
            }
        }
    }

    /// Un réverbère : un poteau et une torche.
    fn lamp(&mut self, x: i32, z: i32) {
        let h = self.height(x, z);
        if h < SEA_LEVEL {
            return;
        }
        let log = self.id("core:oak_log");
        for dy in 1..=3 {
            self.put(IVec3::new(x, h + dy, z), log);
        }
        self.set(IVec3::new(x, h + 4, z), "core:torch");
    }

    // ----- Les structures -----

    fn village(&mut self, cx: i32, cz: i32, biome: &str) {
        let style = self.style(biome);
        let h0 = self.height(cx, cz).max(SEA_LEVEL + 1);
        let center = IVec3::new(cx, h0, cz);
        self.well(center);
        // Quatre chemins de 3 blocs de large, en croix.
        let reach: i32 = 36;
        for t in -reach..=reach {
            for w in -1..=1 {
                if t.abs() > 2 {
                    self.path_cell(cx + t, cz + w);
                    self.path_cell(cx + w, cz + t);
                }
            }
        }
        // Des réverbères le long des chemins.
        for t in [6, 14, 22, 30] {
            for (dx, dz) in [(t, 2), (-t, -2), (2, -t), (-2, t)] {
                self.lamp(cx + dx, cz + dz);
            }
        }
        // Les emplacements : trois par côté de chaque bras, tous les 10 blocs.
        let mut smithy_done = false;
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for t in [9, 19, 29] {
                for side in [-1, 1] {
                    if self.rng.next_f32() < 0.3 {
                        continue;
                    }
                    let roll = self.rng.next_f32();
                    let kind = if roll < 0.22 {
                        Building::Farm
                    } else if roll < 0.32 && !smithy_done {
                        Building::Smithy
                    } else {
                        Building::House
                    };
                    let (w, d) = match kind {
                        Building::Farm => (9, 9),
                        Building::Smithy => (7, 6),
                        Building::House => (
                            5 + 2 * self.rng.below(2) as i32,
                            5 + self.rng.below(2) as i32 * 2,
                        ),
                    };
                    let frame = if dx != 0 {
                        let k = if side > 0 { 0 } else { 2 };
                        let x = cx + dx * t - w / 2;
                        let z = if side > 0 { cz + 3 } else { cz - 3 - d };
                        Frame {
                            origin: IVec3::new(x, 0, z),
                            k,
                            w,
                            d,
                        }
                    } else {
                        let k = if side > 0 { 1 } else { 3 };
                        let x = if side > 0 { cx + 3 } else { cx - 3 - d };
                        let z = cz + dz * t - w / 2;
                        Frame {
                            origin: IVec3::new(x, 0, z),
                            k,
                            w,
                            d,
                        }
                    };
                    let (ex, ez) = frame.extent();
                    let (mx, mz) = (frame.origin.x + ex / 2, frame.origin.z + ez / 2);
                    let y = self.height(mx, mz) + 1;
                    // Trop de pente ou dans l'eau : on saute cet emplacement.
                    if (y - 1 - h0).abs() > 4 || y <= SEA_LEVEL {
                        continue;
                    }
                    match kind {
                        Building::Farm => self.farm(frame, y - 1),
                        _ => {
                            if kind == Building::Smithy {
                                smithy_done = true;
                            }
                            self.house(frame, y, style, kind, false);
                        }
                    }
                    // Un seuil de gravier entre la porte et le chemin.
                    let step = frame.map(w / 2, 0, -1);
                    self.path_cell(step.x, step.z);
                }
            }
        }
    }

    /// Une ruine : une petite maison en moellons dont les murs s'effondrent, et un coffre.
    fn ruin(&mut self, x: i32, z: i32) {
        let (w, d) = (7, 7);
        let k = self.rng.below(4) as u8;
        let y = self.height(x, z) + 1;
        let frame = Frame {
            origin: IVec3::new(x - 3, 0, z - 3),
            k,
            w,
            d,
        };
        // Un sol en moellons enterré dans la pente, pour ne pas flotter.
        let cobble = self.id("core:cobblestone");
        for lz in 0..d {
            for lx in 0..w {
                let p = frame.map(lx, 0, lz);
                let h = self.height(p.x, p.z);
                for yy in (h + 1)..y {
                    self.put(IVec3::new(p.x, yy, p.z), cobble);
                }
                for yy in y..=(y + 4).max(h) {
                    self.air(IVec3::new(p.x, yy, p.z));
                }
            }
        }
        let style = Style {
            planks: cobble,
            log: self.id("core:stone"),
            wall: cobble,
            roof: cobble,
        };
        self.house(frame, y, style, Building::House, true);
    }

    /// Une salle de donjon enterrée : quatre murs de pierre, du gravier, des coffres.
    fn dungeon(&mut self, x: i32, z: i32) {
        // Sous le point le plus bas de la salle : sur une pente, elle reste enterrée.
        let surface = [(0, 0), (-5, -5), (5, -5), (-5, 5), (5, 5)]
            .iter()
            .map(|&(dx, dz)| self.height(x + dx, z + dz))
            .min()
            .unwrap_or(0);
        let y = (surface - 12 - self.rng.below(10) as i32).max(LAVA_LEVEL + 6);
        if surface - y < 8 {
            return;
        }
        let (cobble, gravel, stone) = (
            self.id("core:cobblestone"),
            self.id("core:gravel"),
            self.id("core:stone"),
        );
        let (lo, hi) = (
            IVec3::new(x - 4, y - 1, z - 4),
            IVec3::new(x + 4, y + 4, z + 4),
        );
        self.fill(lo, hi, cobble);
        self.fill(
            lo + IVec3::new(1, 1, 1),
            hi - IVec3::new(1, 1, 1),
            BlockId::AIR,
        );
        // Un sol de gravier et de pierre mêlés.
        for dz in -3..=3 {
            for dx in -3..=3 {
                let block = if self.rng.next_f32() < 0.5 {
                    gravel
                } else {
                    stone
                };
                self.put(IVec3::new(x + dx, y - 1, z + dz), block);
            }
        }
        // Des coffres dans deux coins, un pilier au milieu.
        self.chest(IVec3::new(x - 3, y, z - 3));
        self.chest(IVec3::new(x + 3, y, z + 3));
        if self.rng.next_f32() < 0.5 {
            self.chest(IVec3::new(x + 3, y, z - 3));
        }
        for dy in 0..3 {
            self.put(IVec3::new(x, y + dy, z), cobble);
        }
    }

    /// Une mine abandonnée : une rampe depuis la surface, un couloir étayé, des branches,
    /// du minerai dans les parois et des coffres au bout.
    fn mine(&mut self, x: i32, z: i32) {
        let k = self.rng.below(4);
        let rot = |lx: i32, lz: i32| match k {
            0 => (lx, lz),
            1 => (-lz, lx),
            2 => (-lx, -lz),
            _ => (lz, -lx),
        };
        let at = |lx: i32, y: i32, lz: i32| {
            let (dx, dz) = rot(lx, lz);
            IVec3::new(x + dx, y, z + dz)
        };
        let h0 = self.height(x, z);
        let depth_y = (h0 - 24).max(LAVA_LEVEL + 8);
        let ramp = h0 - depth_y;
        if ramp < 6 {
            return;
        }
        let planks = self.id("core:oak_planks");
        let log = self.id("core:oak_log");
        let ores = [
            self.id("core:coal_ore"),
            self.id("core:iron_ore"),
            self.id("core:copper_ore"),
        ];
        let torch = self.id("core:torch");
        // La rampe : un bloc de plus bas à chaque pas, large de 5.
        for i in 0..=ramp {
            let floor = h0 - i;
            for lz in -2..=2 {
                for dy in 1..=3 {
                    self.air(at(i, floor + dy, lz));
                }
            }
            if i % 4 == 0 && i > 0 {
                self.support(&at, i, floor, planks, torch);
            }
        }
        // Un portique en bois à l'entrée.
        for lz in [-2, 2] {
            for dy in 1..=3 {
                self.put(at(0, h0 + dy, lz), log);
            }
        }
        for lz in -2..=2 {
            self.put(at(0, h0 + 4, lz), log);
        }
        // Le couloir principal, de niveau.
        let length = 26;
        let end = ramp + length;
        for i in (ramp + 1)..=end {
            for lz in -2..=2 {
                for dy in 1..=3 {
                    self.air(at(i, depth_y + dy, lz));
                }
            }
            if i % 4 == 0 {
                self.support(&at, i, depth_y, planks, torch);
            }
            // Du minerai dans la paroi, de temps en temps.
            if self.rng.next_f32() < 0.12 {
                let side = if self.rng.next_f32() < 0.5 { -3 } else { 3 };
                let ore = ores[self.rng.below(3) as usize];
                let dy = 1 + self.rng.below(3) as i32;
                self.put(at(i, depth_y + dy, side), ore);
            }
        }
        self.chest(at(end, depth_y + 1, 0));
        // Deux branches latérales, plus étroites, avec un coffre au bout.
        for (i, side) in [(ramp + 9, 1), (ramp + 18, -1)] {
            for step in 3..=11 {
                for lz in -1..=1 {
                    for dy in 1..=3 {
                        self.air(at(i + lz, depth_y + dy, side * step));
                    }
                }
            }
            self.chest(at(i, depth_y + 1, side * 11));
        }
    }

    /// Un cadre de soutènement : deux poteaux, une poutre, parfois une torche.
    fn support(
        &mut self,
        at: &dyn Fn(i32, i32, i32) -> IVec3,
        i: i32,
        floor: i32,
        planks: BlockId,
        torch: BlockId,
    ) {
        for lz in [-2, 2] {
            for dy in 1..=2 {
                self.put(at(i, floor + dy, lz), planks);
            }
        }
        for lz in -2..=2 {
            self.put(at(i, floor + 3, lz), planks);
        }
        if self.rng.next_f32() < 0.34 {
            self.put(at(i, floor + 2, 1), torch);
        }
    }
}

// ----- Les emplacements -----

/// Où se trouve une structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureSite {
    /// L'indice de sa sorte dans [`StructureSet::defs`].
    pub def: usize,
    pub kind: StructureKind,
    pub x: i32,
    pub z: i32,
    pub seed: u64,
}

impl TerrainGenerator {
    pub fn structures(&self) -> Option<&StructureSet> {
        self.structures.as_ref()
    }

    /// La structure de la case (`cx`, `cz`) de la sorte `d`, si elle existe et si le terrain
    /// lui convient.
    fn site_in_cell(&self, d: usize, cx: i32, cz: i32) -> Option<StructureSite> {
        let set = self.structures.as_ref()?;
        let def = set.defs.get(d)?;
        let cell = def.spacing * CHUNK_SIZE as i32;
        let mut rng = Rng64::new(hash3(self.seed, cx, d as i32, cz, 700 + d as u64));
        let present = rng.next_f32() < def.chance;
        // Loin des bords de la case : deux structures voisines ne se touchent pas.
        let margin = (cell / 4).min(def.radius);
        let span = (cell - 2 * margin).max(1) as u32;
        let x = cx * cell + margin + rng.below(span) as i32;
        let z = cz * cell + margin + rng.below(span) as i32;
        let seed = rng.next_u64();
        if !present {
            return None;
        }
        let column = self.column(x, z);
        if !def.biomes.is_empty() && !def.biomes.contains(&column.biome) {
            return None;
        }
        let ok = match def.kind {
            StructureKind::Village => {
                column.height > SEA_LEVEL
                    && [
                        (18, 0),
                        (-18, 0),
                        (0, 18),
                        (0, -18),
                        (13, 13),
                        (-13, 13),
                        (13, -13),
                        (-13, -13),
                    ]
                    .iter()
                    .all(|&(dx, dz)| {
                        let h = self.column(x + dx, z + dz).height;
                        h > SEA_LEVEL && (h - column.height).abs() <= 7
                    })
            }
            StructureKind::Ruin => column.height > SEA_LEVEL,
            StructureKind::Dungeon => column.height > SEA_LEVEL - 2,
            StructureKind::Mine => column.height > SEA_LEVEL + 3,
            StructureKind::Template => column.height > SEA_LEVEL,
            StructureKind::Graph => {
                column.height > SEA_LEVEL + 3
                    && def
                        .graph
                        .as_ref()
                        .is_some_and(|g| column.height - g.depth() >= 12)
            }
        };
        // Génération v2 : une structure ne se pose ni dans l'eau, ni sur un lit, ni sur une berge.
        (ok && self.v2_site_clear(x, z, def.radius)).then_some(StructureSite {
            def: d,
            kind: def.kind,
            x,
            z,
            seed,
        })
    }

    /// Les structures dont l'étendue touche le rectangle (en blocs, inclusif).
    pub fn structure_sites_in(&self, min: (i32, i32), max: (i32, i32)) -> Vec<StructureSite> {
        let Some(set) = &self.structures else {
            return Vec::new();
        };
        let mut sites = Vec::new();
        for (d, def) in set.defs.iter().enumerate() {
            let cell = def.spacing * CHUNK_SIZE as i32;
            let reach = def.radius;
            for cz in (min.1 - reach).div_euclid(cell)..=(max.1 + reach).div_euclid(cell) {
                for cx in (min.0 - reach).div_euclid(cell)..=(max.0 + reach).div_euclid(cell) {
                    if let Some(site) = self.site_in_cell(d, cx, cz)
                        && (site.x + reach >= min.0 && site.x - reach <= max.0)
                        && (site.z + reach >= min.1 && site.z - reach <= max.1)
                    {
                        sites.push(site);
                    }
                }
            }
        }
        sites
    }

    /// Les structures à moins de `radius` blocs de (x, z).
    pub fn structure_sites_near(&self, x: i32, z: i32, radius: i32) -> Vec<StructureSite> {
        self.structure_sites_in((x - radius, z - radius), (x + radius, z + radius))
            .into_iter()
            .filter(|s| (s.x - x).abs().max((s.z - z).abs()) <= radius)
            .collect()
    }

    /// Le plan complet d'une structure.
    pub fn blueprint(&self, site: &StructureSite) -> Option<Blueprint> {
        self.blueprint_with(self.structures.as_ref()?, site)
    }

    /// Le plan d'une structure d'un jeu de structures donné (celui du générateur, ou un autre).
    pub(crate) fn blueprint_with(
        &self,
        set: &StructureSet,
        site: &StructureSite,
    ) -> Option<Blueprint> {
        let blocks = self.structure_blocks.as_ref()?;
        let def = set.defs.get(site.def)?;
        // Un plan n'a pas de table propre : ses coffres nomment les leurs.
        static EMPTY_LOOT: std::sync::OnceLock<ChestLoot> = std::sync::OnceLock::new();
        let loot = match set.loot.get(&def.loot) {
            Some(loot) => loot,
            None if def.template.is_some() || def.graph.is_some() => {
                EMPTY_LOOT.get_or_init(|| ChestLoot {
                    rolls: (0, 0),
                    entries: Vec::new(),
                })
            }
            None => return None,
        };
        let mut builder = Builder {
            world: self,
            b: blocks,
            rng: Rng64::new(site.seed),
            bp: Blueprint::default(),
            heights: HashMap::new(),
            loot,
        };
        let biome_key = self
            .biomes
            .get(self.column(site.x, site.z).biome)
            .key
            .clone();
        match site.kind {
            StructureKind::Village => builder.village(site.x, site.z, &biome_key),
            StructureKind::Ruin => builder.ruin(site.x, site.z),
            StructureKind::Dungeon => builder.dungeon(site.x, site.z),
            StructureKind::Mine => builder.mine(site.x, site.z),
            StructureKind::Template => {
                if let Some(template) = &def.template {
                    builder.template(site.x, site.z, template, set);
                }
            }
            StructureKind::Graph => {
                if let Some(graph) = &def.graph {
                    builder.graph(site.x, site.z, graph, set);
                }
            }
        }
        Some(builder.bp)
    }

    /// Les centres des villages qui touchent la colonne de chunks (cx, cz).
    pub(super) fn village_clearings(&self, cx: i32, cz: i32) -> Vec<(i32, i32)> {
        if self.structures.is_none() {
            return Vec::new();
        }
        let size = CHUNK_SIZE as i32;
        self.structure_sites_in(
            (cx * size, cz * size),
            (cx * size + size - 1, cz * size + size - 1),
        )
        .iter()
        .filter(|s| s.kind == StructureKind::Village)
        .map(|s| (s.x, s.z))
        .collect()
    }

    /// Les structures qui touchent ce chunk : leur plan, prêt à être écrit.
    pub(super) fn structure_plans(&self, chunk_pos: IVec3) -> Vec<Blueprint> {
        if self.structures.is_none() {
            return Vec::new();
        }
        let origin = World::chunk_origin(chunk_pos);
        let size = CHUNK_SIZE as i32;
        self.structure_sites_in(
            (origin.x, origin.z),
            (origin.x + size - 1, origin.z + size - 1),
        )
        .iter()
        .filter_map(|site| self.blueprint(site))
        .collect()
    }

    /// Écrit dans le chunk la part des structures qui le touchent.
    pub(super) fn place_structures(
        &self,
        chunk: &mut Chunk,
        chunk_pos: IVec3,
        plans: &[Blueprint],
    ) {
        let Some(blocks) = &self.structure_blocks else {
            return;
        };
        let chest = blocks.get("core:chest");
        let registry = &self.registry;
        let active = |b: BlockId| registry.def(b).behavior.is_some();
        for plan in plans {
            plan.apply(chunk, chunk_pos, chest, &active);
        }
    }

    /// Les structures sont-elles actives dans ce monde ? (Elles manquent d'un bloc dans un
    /// registre de test réduit.)
    pub fn has_structures(&self) -> bool {
        self.structures.is_some() && self.structure_blocks.is_some()
    }
}

/// Les fichiers `structures.ron` des mods : (espace de noms, texte, origine).
pub(super) fn mod_sources(
    mods: &[(String, std::path::PathBuf)],
) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    for (namespace, dir) in mods {
        let path = dir.join("structures.ron");
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("lecture de {}", path.display()))?;
            out.push((namespace.clone(), text, path.display().to_string()));
        }
    }
    Ok(out)
}

/// Construit les structures d'un monde (celles du jeu, puis celles des mods), ou `None` si un
/// bloc de base manque. Un fichier de mod invalide est une erreur : il est refusé en entier.
pub(super) fn load(
    registry: &BlockRegistry,
    biomes: &BiomeRegistry,
    sources: &[(String, String, String)],
) -> Result<(Option<StructureSet>, Option<StructureBlocks>)> {
    let blocks = StructureBlocks::resolve(registry);
    let set = StructureSet::builtin(biomes).ok();
    let (Some(blocks), Some(mut set)) = (blocks, set) else {
        return Ok((None, None));
    };
    for (namespace, text, origin) in sources {
        set.add_source(namespace, text, origin, biomes, Some(registry))?;
    }
    Ok((Some(set), Some(blocks)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::ItemRegistry;

    fn generator() -> TerrainGenerator {
        TerrainGenerator::new(42)
    }

    /// Un site de cette sorte, quelque part dans une grande zone.
    fn find(generator: &TerrainGenerator, kind: StructureKind) -> StructureSite {
        generator
            .structure_sites_in((-4000, -4000), (4000, 4000))
            .into_iter()
            .find(|s| s.kind == kind)
            .unwrap_or_else(|| panic!("aucune structure {kind:?}"))
    }

    #[test]
    fn the_data_file_is_valid_and_its_loot_exists() {
        let generator = generator();
        assert!(generator.has_structures());
        let set = generator.structures().unwrap();
        assert_eq!(set.defs().len(), 4);
        let items = ItemRegistry::builtin(generator.registry()).unwrap();
        for key in set.loot_items() {
            assert!(
                items.id(key).is_some(),
                "objet inconnu dans le butin : {key}"
            );
        }
        assert!(
            StructureSet::parse(
                r#"(structures: [(id: "core:x", kind: Ruin, spacing: 5, chance: 0.5, loot: "nope")], loot: {})"#,
                "test",
                generator.biomes()
            )
            .is_err()
        );
    }

    #[test]
    fn every_kind_exists_and_sites_depend_only_on_the_seed() {
        let a = generator();
        let b = generator();
        for kind in [
            StructureKind::Village,
            StructureKind::Ruin,
            StructureKind::Dungeon,
            StructureKind::Mine,
        ] {
            assert_eq!(find(&a, kind), find(&b, kind));
        }
        let other = TerrainGenerator::new(43);
        assert_ne!(
            a.structure_sites_in((-2000, -2000), (2000, 2000)),
            other.structure_sites_in((-2000, -2000), (2000, 2000))
        );
    }

    #[test]
    fn a_village_has_houses_a_well_chests_and_villagers() {
        let generator = generator();
        let site = find(&generator, StructureKind::Village);
        let plan = generator.blueprint(&site).unwrap();
        assert!(plan.len() > 800, "{} blocs", plan.len());
        let id = |key: &str| generator.registry().id(key).unwrap();
        let count = |block: BlockId| plan.blocks().iter().filter(|(_, b)| *b == block).count();
        assert!(count(id("core:glass")) >= 2, "des fenêtres");
        assert!(count(id("core:gravel")) > 40, "des chemins");
        assert!(count(id("core:bed")) + count(id("core:workbench")) >= 1);
        assert!(!plan.spots.is_empty(), "au moins un habitant");
        let kinds: Vec<_> = plan.spots.iter().map(|s| s.mob).collect();
        assert!(kinds.iter().all(|k| k.starts_with("core:")));
    }

    #[test]
    fn a_dungeon_is_buried_with_chests_and_a_mine_reaches_down() {
        let generator = generator();
        let dungeon = find(&generator, StructureKind::Dungeon);
        let plan = generator.blueprint(&dungeon).unwrap();
        assert!(plan.chests().len() >= 2);
        let surface = generator.column(dungeon.x, dungeon.z).height;
        assert!(plan.chests().iter().all(|(p, _)| p.y < surface - 6));
        for (_, inventory) in plan.chests() {
            assert!(inventory.slots.iter().flatten().count() >= 4, "du butin");
        }
        let mine = find(&generator, StructureKind::Mine);
        let plan = generator.blueprint(&mine).unwrap();
        let surface = generator.column(mine.x, mine.z).height;
        let deepest = plan.blocks().iter().map(|(p, _)| p.y).min().unwrap();
        assert!(
            deepest < surface - 15,
            "la mine descend : {deepest} / {surface}"
        );
        assert!(plan.chests().len() >= 3);
    }

    /// Les coffres des donjons, ruines et mines du jeu de base contiennent des objets à affixes (à tirer
    /// à l'ouverture) et des gemmes : les données de `structures.ron` sont bien branchées.
    #[test]
    fn core_chests_hold_pending_affixed_items_and_gems() {
        let generator = generator();
        let (mut pending, mut gems) = (0, 0);
        for site in generator
            .structure_sites_in((-4000, -4000), (4000, 4000))
            .into_iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    StructureKind::Dungeon | StructureKind::Ruin | StructureKind::Mine
                )
            })
            .take(24)
        {
            let plan = generator.blueprint(&site).unwrap();
            for (_, inventory) in plan.chests() {
                for stack in inventory.slots.iter().flatten() {
                    pending += usize::from(stack.damage & PENDING_LOOT == PENDING_LOOT);
                    gems += usize::from(stack.item.starts_with("core:gem_"));
                    if stack.damage & PENDING_LOOT == PENDING_LOOT {
                        assert_eq!(stack.count, 1);
                    } else {
                        assert_eq!(stack.damage, 0, "{}", stack.item);
                    }
                }
            }
        }
        assert!(
            pending >= 3,
            "{pending} objets à affixes dans 24 structures"
        );
        assert!(gems >= 2, "{gems} gemmes dans 24 structures");
    }

    /// Un butin « à affixes » est rangé comme un marqueur à tirer (graine et rareté minimale) : le
    /// même coffre redonne le même marqueur, et le plan qui suit n'est pas décalé (le générateur
    /// aléatoire du plan avance comme avec un butin ordinaire).
    #[test]
    fn affixed_chest_loot_is_a_stable_pending_marker_that_leaves_the_plan_alone() {
        let generator = generator();
        let blocks = generator.structure_blocks.clone().unwrap();
        let table = |affixed: bool| ChestLoot {
            rolls: (3, 3),
            entries: vec![ChestLootEntry {
                item: "core:iron_sword".into(),
                count: (2, 2),
                weight: 1,
                affixed,
                min_rarity: 2,
            }],
        };
        let fill = |loot: &ChestLoot| {
            let mut builder = Builder {
                world: &generator,
                b: &blocks,
                rng: Rng64::new(9),
                bp: Blueprint::default(),
                heights: HashMap::new(),
                loot,
            };
            builder.chest_with(IVec3::new(4, 20, 7), loot);
            let after = builder.rng.next_u64();
            (builder.bp.chests()[0].1.clone(), after)
        };
        let (plain, plain_after) = fill(&table(false));
        let (marked, marked_after) = fill(&table(true));
        assert_eq!(plain_after, marked_after, "le plan n'est pas décalé");
        assert_eq!(
            marked,
            fill(&table(true)).0,
            "le même coffre, les mêmes marqueurs"
        );
        let stacks: Vec<_> = marked.slots.iter().flatten().collect();
        assert_eq!(stacks.len(), 3);
        for stack in stacks {
            assert_eq!(stack.item, "core:iron_sword");
            assert_eq!(stack.count, 1, "un objet à affixes ne s'empile pas");
            assert_eq!(stack.damage & PENDING_LOOT, PENDING_LOOT);
            assert_eq!((stack.damage >> 16) & 0xF, 2, "la rareté minimale");
        }
        assert!(
            plain
                .slots
                .iter()
                .flatten()
                .all(|s| s.damage == 0 && s.count == 2)
        );
        // Les marqueurs de deux emplacements diffèrent (la graine dépend de la case).
        let seeds: std::collections::HashSet<_> = marked
            .slots
            .iter()
            .flatten()
            .map(|s| s.damage & 0xFFFF)
            .collect();
        assert!(seeds.len() >= 2);
    }

    /// Chaque chunk dessine sa part seul : le monde généré chunk par chunk contient bien le
    /// plan entier, sans coupure aux frontières.
    #[test]
    fn chunks_generated_alone_agree_with_the_blueprint() {
        let generator = generator();
        let site = find(&generator, StructureKind::Village);
        let plan = generator.blueprint(&site).unwrap();
        // Le dernier bloc posé à chaque position gagne.
        let mut last: HashMap<IVec3, BlockId> = HashMap::new();
        for &(p, b) in plan.blocks() {
            last.insert(p, b);
        }
        let chest = generator.registry().id("core:chest").unwrap();
        for (p, _) in plan.chests() {
            last.insert(*p, chest);
        }
        let mut chunks: HashMap<IVec3, Option<Chunk>> = HashMap::new();
        let mut wrong = 0;
        let mut checked = 0;
        for (p, block) in last.iter().step_by(7) {
            let (cpos, [x, y, z]) = World::split(*p);
            if !(0..crate::WORLD_HEIGHT_CHUNKS).contains(&cpos.y) {
                continue;
            }
            let chunk = chunks
                .entry(cpos)
                .or_insert_with(|| generator.generate(cpos));
            let found = chunk.as_ref().map_or(BlockId::AIR, |c| c.get(x, y, z));
            checked += 1;
            if found != *block {
                wrong += 1;
            }
        }
        assert!(checked > 200);
        assert_eq!(wrong, 0, "{wrong} blocs sur {checked} diffèrent du plan");
        // Les chunks voisins ont tous reçu leur part : plusieurs chunks touchés.
        assert!(chunks.len() >= 6, "{} chunks", chunks.len());
    }

    #[test]
    fn generated_chests_carry_their_loot() {
        let generator = generator();
        let site = find(&generator, StructureKind::Ruin);
        let plan = generator.blueprint(&site).unwrap();
        let (at, inventory) = plan.chests().first().expect("un coffre").clone();
        let (cpos, [x, y, z]) = World::split(at);
        let chunk = generator.generate(cpos).expect("un chunk");
        let entity = chunk.block_entity(x, y, z).expect("l'état du coffre");
        let stored = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert_eq!(stored, &inventory);
        assert_eq!(
            chunk.get(x, y, z),
            generator.registry().id("core:chest").unwrap()
        );
    }

    #[test]
    fn buildings_turn_so_that_the_door_faces_the_road() {
        // Un rectangle 5 × 7 : la porte (lz = 0, lx = 2) tourne avec le repère.
        for k in 0..4 {
            let f = Frame {
                origin: IVec3::new(10, 0, 20),
                k,
                w: 5,
                d: 7,
            };
            let (ex, ez) = f.extent();
            let mut seen = std::collections::HashSet::new();
            for lz in 0..f.d {
                for lx in 0..f.w {
                    let p = f.map(lx, 0, lz);
                    assert!((10..10 + ex).contains(&p.x) && (20..20 + ez).contains(&p.z));
                    assert!(seen.insert((p.x, p.z)), "deux cases au même endroit");
                }
            }
            assert_eq!(seen.len(), (ex * ez) as usize);
        }
    }

    const HUT: &str = r#"(
        structures: [(id: "test:hut", kind: Template, template: "hut", spacing: 6, chance: 1.0)],
        templates: {"hut": (
            palette: {'=': "core:cobblestone", 'g': "core:glass"},
            layers: [["===", "==="], ["=g=", "=.="]],
            chests: [((1, 1, 1), "stash")],
        )},
        loot: {"stash": (rolls: (1, 1), entries: [(item: "core:coal", count: (1, 1), weight: 1)])},
    )"#;

    fn with_hut(text: &str) -> Result<StructureSet> {
        let generator = generator();
        let mut set = StructureSet::builtin(generator.biomes())?;
        set.add_source(
            "test",
            text,
            "test",
            generator.biomes(),
            Some(generator.registry()),
        )?;
        Ok(set)
    }

    #[test]
    fn a_template_from_a_mod_is_resolved_and_sized() {
        let set = with_hut(HUT).unwrap();
        let hut = set.defs().iter().find(|d| d.key == "test:hut").unwrap();
        assert_eq!(hut.kind, StructureKind::Template);
        let template = hut.template.as_ref().unwrap();
        assert_eq!(template.size(), 3);
        assert_eq!(hut.radius, 5);
        // Les tables du mod sont à son nom ; celles du jeu restent accessibles.
        assert!(set.loot("test:stash").is_some() && set.loot("village").is_some());
    }

    #[test]
    fn broken_templates_are_refused_with_a_reason() {
        let bad = |text: &str| format!("{:#}", with_hut(text).unwrap_err());
        // Une lettre hors de la palette.
        assert!(bad(&HUT.replace("\"=g=\"", "\"=x=\"")).contains("palette"));
        // Un bloc inconnu.
        assert!(bad(&HUT.replace("core:glass", "core:nope")).contains("core:nope"));
        // Un coffre hors du plan, ou avec une table absente.
        assert!(bad(&HUT.replace("(1, 1, 1)", "(9, 1, 1)")).contains("hors du plan"));
        assert!(bad(&HUT.replace("\"stash\")]", "\"absent\")]")).contains("absent"));
        // Un plan inconnu, une structure hors de l'espace de noms du fichier.
        assert!(bad(&HUT.replace("template: \"hut\"", "template: \"autre\"")).contains("inconnu"));
        assert!(bad(&HUT.replace("test:hut", "other:hut")).contains("commencer par"));
    }

    #[test]
    fn a_template_is_placed_on_the_ground_with_its_chest_and_foundation() {
        let generator = generator();
        let mut set = StructureSet::builtin(generator.biomes()).unwrap();
        set.add_source(
            "test",
            HUT,
            "test",
            generator.biomes(),
            Some(generator.registry()),
        )
        .unwrap();
        let def = set.defs().iter().position(|d| d.key == "test:hut").unwrap();
        let site = StructureSite {
            def,
            kind: StructureKind::Template,
            x: 100,
            z: 100,
            seed: 5,
        };
        // Le générateur de ce test n'a pas la structure : on construit le plan à la main.
        let blocks = generator.structure_blocks.clone().unwrap();
        let loot = set.loot("village").unwrap();
        let mut builder = Builder {
            world: &generator,
            b: &blocks,
            rng: Rng64::new(site.seed),
            bp: Blueprint::default(),
            heights: HashMap::new(),
            loot,
        };
        builder.template(
            site.x,
            site.z,
            set.defs()[def].template.as_ref().unwrap(),
            &set,
        );
        let plan = builder.bp;
        let cobble = generator.registry().id("core:cobblestone").unwrap();
        assert!(plan.blocks().iter().filter(|&&(_, b)| b == cobble).count() >= 6);
        assert_eq!(plan.chests().len(), 1);
        // Le plan est posé au-dessus du sol (couche 0 à `ground + 1`).
        let ground = (0..3)
            .flat_map(|dx| (0..3).map(move |dz| (99 + dx, 99 + dz)))
            .map(|(x, z)| generator.column(x, z).height)
            .max()
            .unwrap();
        let lowest_floor = plan
            .blocks()
            .iter()
            .filter(|&&(_, b)| b == cobble)
            .map(|&(p, _)| p.y)
            .max()
            .unwrap();
        assert!(lowest_floor > ground - 1);
    }
}
