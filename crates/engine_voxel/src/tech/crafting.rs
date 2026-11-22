//! L'artisanat, la fonte et le butin : des règles pures, lues dans des fichiers RON.
//!
//! - [`Recipes`] : les recettes d'artisanat (`recipes.ron`), avec forme ou sans forme ;
//! - [`Smelting`] : les recettes du four et les combustibles (`smelting.ron`) ;
//! - [`LootTables`] : ce que lâche un bloc cassé (`loot.ron`).
//!
//! Rien ici ne touche au monde ni à l'interface : on donne une grille, on reçoit un
//! résultat. C'est ce qui rend ces règles faciles à tester.

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::items::{ItemId, ItemRegistry, ToolKind};
use crate::registry::BlockRegistry;
use crate::worldgen::trees::Rng64;

const BUILTIN_RECIPES: &str = include_str!("../../../../assets/core/recipes.ron");
const BUILTIN_SMELTING: &str = include_str!("../../../../assets/core/smelting.ron");
const BUILTIN_BREWING: &str = include_str!("../../../../assets/core/brewing.ron");
const BUILTIN_LOOT: &str = include_str!("../../../../assets/core/loot.ron");

/// Ce qu'accepte une case de recette : un objet précis, ou n'importe quel membre d'un groupe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingredient {
    Item(ItemId),
    Any(Vec<ItemId>),
}

impl Ingredient {
    pub fn matches(&self, item: ItemId) -> bool {
        match self {
            Self::Item(id) => *id == item,
            Self::Any(ids) => ids.contains(&item),
        }
    }

    /// Un objet qui convient (pour afficher la recette).
    pub fn example(&self) -> Option<ItemId> {
        match self {
            Self::Item(id) => Some(*id),
            Self::Any(ids) => ids.first().copied(),
        }
    }

    /// `stick` ou `#planks`, dans l'espace de noms donné.
    fn parse(text: &str, namespace: &str, items: &ItemRegistry) -> Result<Self> {
        match text.strip_prefix('#') {
            Some(tag) => {
                let members = items.tag(tag);
                if members.is_empty() {
                    bail!("groupe inconnu ou vide : « #{tag} »");
                }
                Ok(Self::Any(members.to_vec()))
            }
            None => {
                let key = qualify(text, namespace);
                items
                    .id(&key)
                    .map(Self::Item)
                    .with_context(|| format!("objet inconnu : « {key} »"))
            }
        }
    }
}

fn qualify(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

/// Une recette d'artisanat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipe {
    /// Une forme : `cells[y * width + x]`, `None` = case vide.
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<Ingredient>>,
        result: (ItemId, u32),
    },
    Shapeless {
        ingredients: Vec<Ingredient>,
        result: (ItemId, u32),
    },
}

impl Recipe {
    pub fn result(&self) -> (ItemId, u32) {
        match self {
            Self::Shaped { result, .. } | Self::Shapeless { result, .. } => *result,
        }
    }

    /// La plus petite grille où elle tient (2 : l'inventaire suffit ; 3 : il faut l'établi).
    pub fn grid_size(&self) -> usize {
        match self {
            Self::Shaped { width, height, .. } => (*width).max(*height),
            Self::Shapeless { ingredients, .. } => {
                if ingredients.len() <= 4 {
                    2
                } else {
                    3
                }
            }
        }
    }

    /// Les ingrédients, sans leur place (pour le livre de recettes).
    pub fn ingredients(&self) -> Vec<&Ingredient> {
        match self {
            Self::Shaped { cells, .. } => cells.iter().flatten().collect(),
            Self::Shapeless { ingredients, .. } => ingredients.iter().collect(),
        }
    }

    /// La recette correspond-elle à cette grille (`size × size`, rangées de haut en bas) ?
    fn matches(&self, grid: &[Option<ItemId>], size: usize) -> bool {
        match self {
            Self::Shaped {
                width,
                height,
                cells,
                ..
            } => {
                // On découpe la grille au plus juste autour des objets posés, puis on
                // compare la forme telle quelle ou retournée gauche-droite.
                let Some((x0, y0, w, h)) = bounds(grid, size) else {
                    return false;
                };
                if w != *width || h != *height {
                    return false;
                }
                [false, true].into_iter().any(|mirror| {
                    (0..h).all(|y| {
                        (0..w).all(|x| {
                            let px = if mirror { w - 1 - x } else { x };
                            let cell = &cells[y * w + px];
                            let item = grid[(y0 + y) * size + x0 + x];
                            match (cell, item) {
                                (None, None) => true,
                                (Some(ingredient), Some(item)) => ingredient.matches(item),
                                _ => false,
                            }
                        })
                    })
                })
            }
            Self::Shapeless { ingredients, .. } => {
                let mut items: Vec<ItemId> = grid.iter().flatten().copied().collect();
                if items.len() != ingredients.len() {
                    return false;
                }
                // Chaque ingrédient prend un objet qui lui convient. Les ingrédients précis
                // d'abord, les groupes ensuite : un groupe ne « vole » pas l'objet exact
                // dont un autre ingrédient a besoin.
                let mut order: Vec<&Ingredient> = ingredients.iter().collect();
                order.sort_by_key(|i| matches!(i, Ingredient::Any(_)));
                order.into_iter().all(|ingredient| {
                    match items.iter().position(|&item| ingredient.matches(item)) {
                        Some(i) => {
                            items.swap_remove(i);
                            true
                        }
                        None => false,
                    }
                })
            }
        }
    }
}

/// Le rectangle occupé (x, y, largeur, hauteur) d'une grille, `None` si elle est vide.
fn bounds(grid: &[Option<ItemId>], size: usize) -> Option<(usize, usize, usize, usize)> {
    let filled = |x: usize, y: usize| grid[y * size + x].is_some();
    let xs: Vec<usize> = (0..size)
        .filter(|&x| (0..size).any(|y| filled(x, y)))
        .collect();
    let ys: Vec<usize> = (0..size)
        .filter(|&y| (0..size).any(|x| filled(x, y)))
        .collect();
    let (x0, x1) = (*xs.first()?, *xs.last()?);
    let (y0, y1) = (*ys.first()?, *ys.last()?);
    Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapedEntry {
    pattern: Vec<String>,
    key: HashMap<String, String>,
    result: (String, u32),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapelessEntry {
    ingredients: Vec<String>,
    result: (String, u32),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipesFile {
    #[serde(default)]
    shaped: Vec<ShapedEntry>,
    #[serde(default)]
    shapeless: Vec<ShapelessEntry>,
}

/// Toutes les recettes d'artisanat.
#[derive(Debug, Clone, Default)]
pub struct Recipes {
    list: Vec<Recipe>,
}

impl Recipes {
    pub fn builtin(items: &ItemRegistry) -> Result<Self> {
        let mut recipes = Self::default();
        recipes.add_source("core", BUILTIN_RECIPES, "recipes.ron intégré", items)?;
        Ok(recipes)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        items: &ItemRegistry,
    ) -> Result<()> {
        let file: RecipesFile =
            ron::from_str(source).with_context(|| format!("{origin} : RON invalide"))?;
        let result = |(key, count): &(String, u32)| -> Result<(ItemId, u32)> {
            let key = qualify(key, namespace);
            Ok((
                items
                    .id(&key)
                    .with_context(|| format!("{origin} : résultat inconnu « {key} »"))?,
                *count,
            ))
        };
        for entry in &file.shaped {
            let height = entry.pattern.len();
            let width = entry
                .pattern
                .iter()
                .map(|r| r.chars().count())
                .max()
                .unwrap_or(0);
            if !(1..=3).contains(&width) || !(1..=3).contains(&height) {
                bail!(
                    "{origin} : une forme fait au plus 3 × 3 ({:?})",
                    entry.pattern
                );
            }
            let mut cells = Vec::with_capacity(width * height);
            for row in &entry.pattern {
                let chars: Vec<char> = row.chars().collect();
                for x in 0..width {
                    match chars.get(x).copied().unwrap_or(' ') {
                        ' ' => cells.push(None),
                        c => {
                            let text = entry.key.get(&c.to_string()).with_context(|| {
                                format!("{origin} : la lettre « {c} » n'est pas dans `key`")
                            })?;
                            cells.push(Some(
                                Ingredient::parse(text, namespace, items)
                                    .with_context(|| origin.to_string())?,
                            ));
                        }
                    }
                }
            }
            self.list.push(Recipe::Shaped {
                width,
                height,
                cells,
                result: result(&entry.result)?,
            });
        }
        for entry in &file.shapeless {
            let ingredients = entry
                .ingredients
                .iter()
                .map(|t| Ingredient::parse(t, namespace, items).with_context(|| origin.to_string()))
                .collect::<Result<Vec<_>>>()?;
            if ingredients.is_empty() || ingredients.len() > 9 {
                bail!("{origin} : de 1 à 9 ingrédients");
            }
            self.list.push(Recipe::Shapeless {
                ingredients,
                result: result(&entry.result)?,
            });
        }
        Ok(())
    }

    /// La recette qui correspond à la grille (`size × size`), la première trouvée.
    pub fn find(&self, grid: &[Option<ItemId>], size: usize) -> Option<&Recipe> {
        debug_assert_eq!(grid.len(), size * size);
        self.list
            .iter()
            .find(|r| r.grid_size() <= size && r.matches(grid, size))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Recipe> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// Une recette de four.
#[derive(Debug, Clone, PartialEq)]
pub struct SmeltingRecipe {
    pub input: Ingredient,
    pub output: ItemId,
    /// En ticks de machines (20 par seconde).
    pub ticks: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeltingEntry {
    input: String,
    output: String,
    time: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeltingFile {
    recipes: Vec<SmeltingEntry>,
    fuels: Vec<(String, f32)>,
}

/// Les recettes du four et les combustibles.
#[derive(Debug, Clone, Default)]
pub struct Smelting {
    recipes: Vec<SmeltingRecipe>,
    /// Combustible → ticks de combustion.
    fuels: Vec<(Ingredient, u32)>,
}

impl Smelting {
    pub fn builtin(items: &ItemRegistry) -> Result<Self> {
        let mut smelting = Self::default();
        smelting.add_source("core", BUILTIN_SMELTING, "smelting.ron intégré", items)?;
        Ok(smelting)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        items: &ItemRegistry,
    ) -> Result<()> {
        let file: SmeltingFile =
            ron::from_str(source).with_context(|| format!("{origin} : RON invalide"))?;
        let ticks = |seconds: f32| (seconds * 20.0).round().max(1.0) as u32;
        for r in file.recipes {
            let output = qualify(&r.output, namespace);
            self.recipes.push(SmeltingRecipe {
                input: Ingredient::parse(&r.input, namespace, items)
                    .with_context(|| origin.to_string())?,
                output: items
                    .id(&output)
                    .with_context(|| format!("{origin} : objet inconnu « {output} »"))?,
                ticks: ticks(r.time),
            });
        }
        for (fuel, seconds) in file.fuels {
            self.fuels.push((
                Ingredient::parse(&fuel, namespace, items).with_context(|| origin.to_string())?,
                ticks(seconds),
            ));
        }
        Ok(())
    }

    /// Ce que donne `item` au four.
    pub fn recipe(&self, item: ItemId) -> Option<&SmeltingRecipe> {
        self.recipes.iter().find(|r| r.input.matches(item))
    }

    /// Combien de ticks brûle `item` (`None` : ce n'est pas un combustible).
    pub fn burn_ticks(&self, item: ItemId) -> Option<u32> {
        self.fuels
            .iter()
            .find(|(f, _)| f.matches(item))
            .map(|(_, t)| *t)
    }

    pub fn recipes(&self) -> &[SmeltingRecipe] {
        &self.recipes
    }

    /// Les combustibles et combien de ticks chacun brûle.
    pub fn fuels(&self) -> &[(Ingredient, u32)] {
        &self.fuels
    }
}

/// Une recette de machine de mod : des ingrédients, des résultats, un temps et une énergie.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineRecipe {
    pub inputs: Vec<(Ingredient, u32)>,
    pub outputs: Vec<(ItemId, u32)>,
    /// En ticks de machines (20 par seconde).
    pub ticks: u32,
    /// L'énergie totale consommée (libre : comme les FE).
    pub energy: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineEntry {
    inputs: Vec<(String, u32)>,
    outputs: Vec<(String, u32)>,
    #[serde(default)]
    time: f32,
    #[serde(default)]
    energy: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MachineFile {
    /// Type de recette (`press` devient `mod:press`) → ses recettes.
    machines: std::collections::BTreeMap<String, Vec<MachineEntry>>,
}

/// Les recettes des machines des mods (`machine_recipes.ron`), classées par type : un mod
/// invente ses types (`example:press`) et un autre mod peut y ajouter des recettes.
#[derive(Debug, Clone, Default)]
pub struct MachineRecipes {
    by_kind: HashMap<String, Vec<MachineRecipe>>,
}

impl MachineRecipes {
    pub fn add_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        items: &ItemRegistry,
    ) -> Result<()> {
        let file: MachineFile =
            ron::from_str(source).with_context(|| format!("{origin} : RON invalide"))?;
        for (kind, entries) in file.machines {
            let kind = qualify(&kind, namespace);
            for e in entries {
                if e.inputs.is_empty() || e.outputs.is_empty() {
                    bail!("{origin} : une recette de « {kind} » n'a pas d'entrée ou de sortie");
                }
                let inputs = e
                    .inputs
                    .iter()
                    .map(|(name, n)| Ok((Ingredient::parse(name, namespace, items)?, *n)))
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| origin.to_string())?;
                let outputs = e
                    .outputs
                    .iter()
                    .map(|(name, n)| {
                        let key = qualify(name, namespace);
                        let id = items
                            .id(&key)
                            .with_context(|| format!("{origin} : objet inconnu « {key} »"))?;
                        Ok((id, *n))
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.by_kind
                    .entry(kind.clone())
                    .or_default()
                    .push(MachineRecipe {
                        inputs,
                        outputs,
                        ticks: (e.time * 20.0).round().max(0.0) as u32,
                        energy: e.energy,
                    });
            }
        }
        Ok(())
    }

    /// La première recette de ce type que `available` (objet, nombre) permet de faire.
    pub fn find(&self, kind: &str, available: &[(ItemId, u32)]) -> Option<&MachineRecipe> {
        self.by_kind.get(kind)?.iter().find(|r| {
            r.inputs.iter().all(|(ingredient, need)| {
                let have: u32 = available
                    .iter()
                    .filter(|(item, _)| ingredient.matches(*item))
                    .map(|(_, n)| n)
                    .sum();
                have >= *need
            })
        })
    }

    /// Une recette de ce type consomme-t-elle cet objet ? (Pour les réseaux d'objets : une
    /// machine n'accepte que ce qu'elle sait transformer.)
    pub fn accepts(&self, kind: &str, item: ItemId) -> bool {
        self.recipes(kind).iter().any(|r| {
            r.inputs
                .iter()
                .any(|(ingredient, _)| ingredient.matches(item))
        })
    }

    pub fn recipes(&self, kind: &str) -> &[MachineRecipe] {
        self.by_kind.get(kind).map_or(&[], Vec::as_slice)
    }

    /// Les types de recettes, triés par nom.
    pub fn kinds(&self) -> Vec<&str> {
        let mut kinds: Vec<&str> = self.by_kind.keys().map(String::as_str).collect();
        kinds.sort_unstable();
        kinds
    }

    pub fn len(&self) -> usize {
        self.by_kind.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }
}

/// Une recette d'alambic : `input` + `reagent` donnent `output`.
#[derive(Debug, Clone, PartialEq)]
pub struct BrewRecipe {
    pub input: ItemId,
    pub reagent: ItemId,
    pub output: ItemId,
    /// En ticks de machines (20 par seconde).
    pub ticks: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrewEntry {
    input: String,
    reagent: String,
    output: String,
    time: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrewFile {
    recipes: Vec<BrewEntry>,
}

/// Les recettes de l'alambic.
#[derive(Debug, Clone, Default)]
pub struct Brewing {
    recipes: Vec<BrewRecipe>,
}

impl Brewing {
    pub fn builtin(items: &ItemRegistry) -> Result<Self> {
        let mut brewing = Self::default();
        brewing.add_source("core", BUILTIN_BREWING, "brewing.ron intégré", items)?;
        Ok(brewing)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        items: &ItemRegistry,
    ) -> Result<()> {
        let file: BrewFile =
            ron::from_str(source).with_context(|| format!("{origin} : RON invalide"))?;
        for r in file.recipes {
            let id = |name: &str| {
                let key = qualify(name, namespace);
                items
                    .id(&key)
                    .with_context(|| format!("{origin} : objet inconnu « {key} »"))
            };
            self.recipes.push(BrewRecipe {
                input: id(&r.input)?,
                reagent: id(&r.reagent)?,
                output: id(&r.output)?,
                ticks: (r.time * 20.0).round().max(1.0) as u32,
            });
        }
        Ok(())
    }

    /// La recette de ces deux objets.
    pub fn recipe(&self, input: ItemId, reagent: ItemId) -> Option<&BrewRecipe> {
        self.recipes
            .iter()
            .find(|r| r.input == input && r.reagent == reagent)
    }

    pub fn recipes(&self) -> &[BrewRecipe] {
        &self.recipes
    }
}

/// Une ligne de table de butin.
#[derive(Debug, Clone, PartialEq)]
pub struct LootEntry {
    pub item: ItemId,
    pub count: (u32, u32),
    pub chance: f32,
    /// Seulement avec cet outil en main.
    pub tool: Option<ToolKind>,
    /// Seulement sans cet outil.
    pub without: Option<ToolKind>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LootEntryFile {
    item: String,
    #[serde(default = "one_one")]
    count: (u32, u32),
    #[serde(default = "always")]
    chance: f32,
    #[serde(default)]
    tool: Option<ToolKind>,
    #[serde(default)]
    without: Option<ToolKind>,
}

fn one_one() -> (u32, u32) {
    (1, 1)
}

fn always() -> f32 {
    1.0
}

/// Les tables de butin, par bloc.
#[derive(Debug, Clone, Default)]
pub struct LootTables {
    tables: HashMap<crate::block::BlockId, Vec<LootEntry>>,
}

impl LootTables {
    pub fn builtin(blocks: &BlockRegistry, items: &ItemRegistry) -> Result<Self> {
        let mut tables = Self::default();
        tables.add_source("core", BUILTIN_LOOT, "loot.ron intégré", blocks, items)?;
        Ok(tables)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        blocks: &BlockRegistry,
        items: &ItemRegistry,
    ) -> Result<()> {
        let options = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let file: HashMap<String, Vec<LootEntryFile>> = options
            .from_str(source)
            .with_context(|| format!("{origin} : RON invalide"))?;
        for (block, entries) in file {
            let key = qualify(&block, namespace);
            let block = blocks
                .id(&key)
                .with_context(|| format!("{origin} : bloc inconnu « {key} »"))?;
            let entries = entries
                .into_iter()
                .map(|e| {
                    let item = qualify(&e.item, namespace);
                    Ok(LootEntry {
                        item: items
                            .id(&item)
                            .with_context(|| format!("{origin} : objet inconnu « {item} »"))?,
                        count: e.count,
                        chance: e.chance,
                        tool: e.tool,
                        without: e.without,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            // Les variantes d'un bloc (orientations, moitiés) lâchent le même butin.
            for (id, def) in blocks.iter() {
                if def.base == Some(block) {
                    self.tables.insert(id, entries.clone());
                }
            }
            self.tables.insert(block, entries);
        }
        Ok(())
    }

    /// Ce que lâche `block` cassé avec un outil de sorte `tool` (`None` : à la main).
    /// `harvest` : l'outil a le niveau demandé par le bloc (sinon : rien).
    /// Le hasard vient de `rng` : le même tirage donne le même butin.
    pub fn roll(
        &self,
        block: crate::block::BlockId,
        tool: Option<ToolKind>,
        harvest: bool,
        items: &ItemRegistry,
        rng: &mut Rng64,
    ) -> Vec<(ItemId, u32)> {
        if !harvest {
            return Vec::new();
        }
        let Some(entries) = self.tables.get(&block) else {
            // Pas de table : le bloc lâche son propre objet.
            return items
                .of_block(block)
                .map(|item| vec![(item, 1)])
                .unwrap_or_default();
        };
        let mut out = Vec::new();
        for e in entries {
            if e.tool.is_some_and(|t| tool != Some(t)) || e.without.is_some_and(|t| tool == Some(t))
            {
                continue;
            }
            if rng.next_f32() >= e.chance {
                continue;
            }
            let count = e.count.0 + rng.below(e.count.1 - e.count.0 + 1);
            if count > 0 {
                out.push((e.item, count));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn data() -> (Arc<BlockRegistry>, ItemRegistry, Recipes) {
        let blocks = BlockRegistry::builtin();
        let items = ItemRegistry::builtin(&blocks).unwrap();
        let recipes = Recipes::builtin(&items).unwrap();
        (blocks, items, recipes)
    }

    fn id(items: &ItemRegistry, key: &str) -> ItemId {
        items.id(key).unwrap()
    }

    #[test]
    fn shaped_recipes_match_anywhere_and_mirrored() {
        let (_, items, recipes) = data();
        let (planks, stick) = (id(&items, "core:birch_planks"), id(&items, "core:stick"));
        // Une pioche en bois sur l'établi.
        let p = Some(planks);
        let s = Some(stick);
        let grid = [p, p, p, None, s, None, None, s, None];
        let recipe = recipes.find(&grid, 3).expect("pioche");
        assert_eq!(recipe.result().0, id(&items, "core:wooden_pickaxe"));
        // Deux planches empilées, dans un coin de la grille 3 × 3 : des bâtons.
        let grid = [None, None, None, None, None, p, None, None, p];
        assert_eq!(recipes.find(&grid, 3).unwrap().result(), (stick, 4));
        // La hache, et son reflet : la même recette.
        let axe = id(&items, "core:wooden_axe");
        let left = [p, p, None, p, s, None, None, s, None];
        let right = [None, p, p, None, s, p, None, s, None];
        assert_eq!(recipes.find(&left, 3).unwrap().result().0, axe);
        assert_eq!(recipes.find(&right, 3).unwrap().result().0, axe);
        // Une forme 3 × 3 ne se fait pas dans la grille 2 × 2.
        assert!(recipes.find(&[p, p, None, s], 2).is_none());
        // Une case de trop : plus de recette.
        let extra = [p, p, p, None, s, None, s, s, None];
        assert!(recipes.find(&extra, 3).is_none());
    }

    #[test]
    fn wooden_and_iron_doors_can_be_crafted_and_glazed() {
        let (_, items, recipes) = data();
        let p = Some(id(&items, "core:birch_planks"));
        let i = Some(id(&items, "core:iron_ingot"));
        let c = Some(id(&items, "core:copper_ingot"));
        let g = Some(id(&items, "core:glass"));
        for (plain, glazed, grid) in [
            (
                "core:wooden_door",
                "core:wooden_glass_door",
                [p, p, None, p, p, None, p, p, None],
            ),
            (
                "core:iron_door",
                "core:iron_glass_door",
                [i, i, None, i, c, None, i, i, None],
            ),
        ] {
            let door = id(&items, plain);
            assert_eq!(recipes.find(&grid, 3).unwrap().result(), (door, 1));
            assert_eq!(
                recipes
                    .find(&[g, None, Some(door), None], 2)
                    .unwrap()
                    .result(),
                (id(&items, glazed), 1),
            );
        }
        let gate = [i, i, None, i, i, None, i, i, None];
        assert_eq!(
            recipes.find(&gate, 3).unwrap().result().0,
            id(&items, "core:gate")
        );
    }

    #[test]
    fn shapeless_recipes_ignore_position() {
        let (_, items, recipes) = data();
        let log = Some(id(&items, "core:spruce_log"));
        let planks = id(&items, "core:spruce_planks");
        assert_eq!(
            recipes.find(&[None, None, None, log], 2).unwrap().result(),
            (planks, 4)
        );
        assert_eq!(
            recipes.find(&[log, None, None, None], 2).unwrap().result(),
            (planks, 4)
        );
        let (flint, iron) = (
            Some(id(&items, "core:flint")),
            Some(id(&items, "core:iron_ingot")),
        );
        let torch = id(&items, "core:torch");
        assert_eq!(
            recipes
                .find(&[iron, None, None, flint], 2)
                .unwrap()
                .result(),
            (torch, 8)
        );
        assert!(
            recipes.find(&[log, log, None, None], 2).is_none(),
            "deux bûches : pas de recette"
        );
    }

    #[test]
    fn smelting_and_fuels() {
        let (_, items, _) = data();
        let smelting = Smelting::builtin(&items).unwrap();
        let recipe = smelting.recipe(id(&items, "core:raw_iron")).unwrap();
        assert_eq!(recipe.output, id(&items, "core:iron_ingot"));
        assert_eq!(recipe.ticks, 200);
        assert_eq!(smelting.burn_ticks(id(&items, "core:coal")), Some(1600));
        assert_eq!(
            smelting.burn_ticks(id(&items, "core:oak_planks")),
            Some(300)
        );
        assert!(smelting.burn_ticks(id(&items, "core:stone")).is_none());
        assert_eq!(
            smelting.recipe(id(&items, "core:oak_log")).unwrap().output,
            id(&items, "core:charcoal")
        );
    }

    #[test]
    fn machine_recipes_match_by_kind_and_quantity() {
        let (_, items, _) = data();
        let mut recipes = MachineRecipes::default();
        recipes
            .add_source(
                "core",
                r#"(machines: { "press": [
                    (inputs: [("coal", 2)], outputs: [("charcoal", 1), ("stick", 3)], time: 1.5, energy: 100),
                ] })"#,
                "test",
                &items,
            )
            .unwrap();
        let coal = id(&items, "core:coal");
        let r = recipes.find("core:press", &[(coal, 2)]).unwrap();
        assert_eq!((r.ticks, r.energy), (30, 100));
        assert_eq!(r.outputs.len(), 2);
        assert!(
            recipes.find("core:press", &[(coal, 1)]).is_none(),
            "pas assez"
        );
        assert!(
            recipes.find("core:mill", &[(coal, 2)]).is_none(),
            "autre type"
        );
        // Un objet inconnu est refusé avec un message clair.
        let err = MachineRecipes::default()
            .add_source(
                "core",
                r#"(machines: {"p": [(inputs: [("nope", 1)], outputs: [("stick", 1)])]})"#,
                "t",
                &items,
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("core:nope"));
    }

    #[test]
    fn loot_is_deterministic_and_respects_tools() {
        let (blocks, items, _) = data();
        let loot = LootTables::builtin(&blocks, &items).unwrap();
        let block = |k: &str| blocks.id(k).unwrap();
        let mut rng = Rng64::new(1);
        // La pierre lâche des moellons, mais rien sans le bon outil.
        let stone = block("core:stone");
        assert_eq!(
            loot.roll(stone, Some(ToolKind::Pickaxe), true, &items, &mut rng),
            vec![(id(&items, "core:cobblestone"), 1)]
        );
        assert!(loot.roll(stone, None, false, &items, &mut rng).is_empty());
        // Un bloc sans table lâche son objet.
        let sand = block("core:sand");
        assert_eq!(
            loot.roll(sand, None, true, &items, &mut rng),
            vec![(id(&items, "core:sand"), 1)]
        );
        // Les feuilles : elles-mêmes aux cisailles, sinon parfois une pousse.
        let leaves = block("core:oak_leaves");
        assert_eq!(
            loot.roll(leaves, Some(ToolKind::Shears), true, &items, &mut rng),
            vec![(id(&items, "core:oak_leaves"), 1)]
        );
        let sapling = id(&items, "core:oak_sapling");
        let saplings: u32 = (0..1000)
            .flat_map(|_| loot.roll(leaves, None, true, &items, &mut rng))
            .filter(|(item, _)| *item == sapling)
            .map(|(_, n)| n)
            .sum();
        assert!(
            (30..100).contains(&saplings),
            "{saplings} pousses pour 1000 feuilles"
        );
        // Déterministe : la même graine, le même butin.
        let copper = block("core:copper_ore");
        let roll = |seed| {
            loot.roll(
                copper,
                Some(ToolKind::Pickaxe),
                true,
                &items,
                &mut Rng64::new(seed),
            )
        };
        assert_eq!(roll(7), roll(7));
    }
}
