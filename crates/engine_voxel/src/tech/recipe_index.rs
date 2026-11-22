//! L'index des recettes : pour chaque objet, comment le fabriquer et à quoi il sert.
//!
//! C'est le cœur d'un explorateur de recettes façon JEI (« Just Enough Items ») : toutes les
//! sortes de recettes du jeu (établi, four, combustibles, alambic, machines des mods) sont
//! ramenées à une même forme, [`RecipeView`], puis rangées deux fois :
//! - par **produit** (« recettes » : touche R dans le jeu) ;
//! - par **ingrédient** (« usages » : touche U).
//!
//! L'index se construit une fois, au chargement du contenu (core + mods) ; il ne fait
//! ensuite que des recherches dans des tables.

use std::collections::HashMap;

use crate::items::ItemId;
use crate::tech::crafting::{Brewing, Ingredient, MachineRecipes, Recipe, Recipes, Smelting};

/// Une sorte de recette : un onglet de l'explorateur.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecipeCategory {
    /// La grille d'artisanat (2 × 2 dans l'inventaire, 3 × 3 sur l'établi).
    Crafting,
    /// Le four.
    Smelting,
    /// Ce qui brûle dans le four.
    Fuel,
    /// L'alambic.
    Brewing,
    /// Une machine de mod : son type de recette (`forge:alloy`).
    Machine(String),
}

/// Une case d'ingrédient : les objets qui conviennent (plusieurs pour un groupe comme
/// `#planks` ; l'explorateur les fait défiler) et le nombre demandé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub items: Vec<ItemId>,
    pub count: u32,
}

impl Choice {
    fn of(ingredient: &Ingredient, count: u32) -> Self {
        let items = match ingredient {
            Ingredient::Item(id) => vec![*id],
            Ingredient::Any(ids) => ids.clone(),
        };
        Self { items, count }
    }

    fn one(item: ItemId) -> Self {
        Self {
            items: vec![item],
            count: 1,
        }
    }
}

/// Le dessin d'une recette.
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeLayout {
    /// Une grille `width × height` (cases vides : `None`). `shapeless` : l'ordre ne compte
    /// pas. `grid_size` : 2 si l'inventaire suffit, 3 s'il faut l'établi.
    Grid {
        width: usize,
        height: usize,
        cells: Vec<Option<Choice>>,
        shapeless: bool,
        grid_size: usize,
    },
    /// Une entrée qui cuit en `seconds`.
    Smelt { input: Choice, seconds: f32 },
    /// Un combustible qui brûle `seconds`.
    Fuel { fuel: Choice, seconds: f32 },
    /// Une fiole (`input`) et un ingrédient (`reagent`).
    Brew {
        input: Choice,
        reagent: Choice,
        seconds: f32,
    },
    /// Des entrées en vrac, un temps et une énergie (0 : aucune).
    Machine {
        inputs: Vec<Choice>,
        seconds: f32,
        energy: u64,
    },
}

/// Une recette, quelle que soit sa sorte.
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeView {
    pub category: RecipeCategory,
    pub layout: RecipeLayout,
    /// Ce qu'elle donne (vide pour un combustible).
    pub outputs: Vec<(ItemId, u32)>,
    /// La recette d'artisanat d'origine (pour la fabriquer depuis l'explorateur).
    pub crafting: Option<Recipe>,
}

impl RecipeView {
    /// Toutes les cases d'entrée, dans l'ordre du dessin.
    pub fn inputs(&self) -> Vec<&Choice> {
        match &self.layout {
            RecipeLayout::Grid { cells, .. } => cells.iter().flatten().collect(),
            RecipeLayout::Smelt { input, .. } => vec![input],
            RecipeLayout::Fuel { fuel, .. } => vec![fuel],
            RecipeLayout::Brew { input, reagent, .. } => vec![input, reagent],
            RecipeLayout::Machine { inputs, .. } => inputs.iter().collect(),
        }
    }

    /// La recette utilise-t-elle cet objet ?
    pub fn uses(&self, item: ItemId) -> bool {
        self.inputs().iter().any(|c| c.items.contains(&item))
    }

    /// La recette produit-elle cet objet ?
    pub fn makes(&self, item: ItemId) -> bool {
        self.outputs.iter().any(|(id, _)| *id == item)
    }
}

/// Les recettes indexées par produit et par ingrédient.
#[derive(Debug, Clone, Default)]
pub struct RecipeIndex {
    recipes: Vec<RecipeView>,
    by_output: HashMap<ItemId, Vec<usize>>,
    by_input: HashMap<ItemId, Vec<usize>>,
}

impl RecipeIndex {
    pub fn new(
        recipes: &Recipes,
        smelting: &Smelting,
        brewing: &Brewing,
        machines: &MachineRecipes,
    ) -> Self {
        let mut index = Self::default();
        for recipe in recipes.iter() {
            index.push(crafting_view(recipe));
        }
        let seconds = |ticks: u32| ticks as f32 / 20.0;
        for r in smelting.recipes() {
            index.push(RecipeView {
                category: RecipeCategory::Smelting,
                layout: RecipeLayout::Smelt {
                    input: Choice::of(&r.input, 1),
                    seconds: seconds(r.ticks),
                },
                outputs: vec![(r.output, 1)],
                crafting: None,
            });
        }
        for (fuel, ticks) in smelting.fuels() {
            index.push(RecipeView {
                category: RecipeCategory::Fuel,
                layout: RecipeLayout::Fuel {
                    fuel: Choice::of(fuel, 1),
                    seconds: seconds(*ticks),
                },
                outputs: Vec::new(),
                crafting: None,
            });
        }
        for r in brewing.recipes() {
            index.push(RecipeView {
                category: RecipeCategory::Brewing,
                layout: RecipeLayout::Brew {
                    input: Choice::one(r.input),
                    reagent: Choice::one(r.reagent),
                    seconds: seconds(r.ticks),
                },
                outputs: vec![(r.output, 1)],
                crafting: None,
            });
        }
        for kind in machines.kinds() {
            for r in machines.recipes(kind) {
                index.push(RecipeView {
                    category: RecipeCategory::Machine(kind.to_string()),
                    layout: RecipeLayout::Machine {
                        inputs: r.inputs.iter().map(|(i, n)| Choice::of(i, *n)).collect(),
                        seconds: seconds(r.ticks),
                        energy: r.energy,
                    },
                    outputs: r.outputs.clone(),
                    crafting: None,
                });
            }
        }
        index
    }

    fn push(&mut self, view: RecipeView) {
        let at = self.recipes.len();
        for (item, _) in &view.outputs {
            let list = self.by_output.entry(*item).or_default();
            if list.last() != Some(&at) {
                list.push(at);
            }
        }
        for choice in view.inputs() {
            for item in &choice.items {
                let list = self.by_input.entry(*item).or_default();
                if list.last() != Some(&at) {
                    list.push(at);
                }
            }
        }
        self.recipes.push(view);
    }

    /// Les recettes qui produisent `item`.
    pub fn recipes_for(&self, item: ItemId) -> Vec<&RecipeView> {
        self.lookup(&self.by_output, item)
    }

    /// Les recettes qui utilisent `item` (comme ingrédient ou comme combustible).
    pub fn uses_of(&self, item: ItemId) -> Vec<&RecipeView> {
        self.lookup(&self.by_input, item)
    }

    fn lookup(&self, table: &HashMap<ItemId, Vec<usize>>, item: ItemId) -> Vec<&RecipeView> {
        table
            .get(&item)
            .map(|list| list.iter().map(|&i| &self.recipes[i]).collect())
            .unwrap_or_default()
    }

    /// Les recettes de `list` rangées par sorte (dans l'ordre des onglets).
    pub fn by_category<'a>(list: &[&'a RecipeView]) -> Vec<(RecipeCategory, Vec<&'a RecipeView>)> {
        let mut groups: Vec<(RecipeCategory, Vec<&RecipeView>)> = Vec::new();
        for view in list {
            match groups.iter_mut().find(|(c, _)| *c == view.category) {
                Some((_, views)) => views.push(view),
                None => groups.push((view.category.clone(), vec![view])),
            }
        }
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups
    }

    pub fn len(&self) -> usize {
        self.recipes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.recipes.is_empty()
    }
}

fn crafting_view(recipe: &Recipe) -> RecipeView {
    let layout = match recipe {
        Recipe::Shaped {
            width,
            height,
            cells,
            ..
        } => RecipeLayout::Grid {
            width: *width,
            height: *height,
            cells: cells
                .iter()
                .map(|c| c.as_ref().map(|i| Choice::of(i, 1)))
                .collect(),
            shapeless: false,
            grid_size: recipe.grid_size(),
        },
        Recipe::Shapeless { ingredients, .. } => {
            // Les ingrédients en vrac, rangés dans la plus petite grille carrée.
            let side = recipe.grid_size();
            let mut cells: Vec<Option<Choice>> =
                ingredients.iter().map(|i| Some(Choice::of(i, 1))).collect();
            cells.resize(side * side, None);
            RecipeLayout::Grid {
                width: side,
                height: side,
                cells,
                shapeless: true,
                grid_size: side,
            }
        }
    };
    RecipeView {
        category: RecipeCategory::Crafting,
        layout,
        outputs: vec![recipe.result()],
        crafting: Some(recipe.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::ItemRegistry;
    use crate::registry::BlockRegistry;

    fn index() -> (RecipeIndex, ItemRegistry) {
        let blocks = BlockRegistry::builtin();
        let items = ItemRegistry::builtin(&blocks).unwrap();
        let recipes = Recipes::builtin(&items).unwrap();
        let smelting = Smelting::builtin(&items).unwrap();
        let brewing = Brewing::builtin(&items).unwrap();
        let mut machines = MachineRecipes::default();
        machines
            .add_source(
                "test",
                r##"(machines: {"press": [(inputs: [("core:iron_ingot", 2), ("#planks", 1)], outputs: [("core:gate", 1)], time: 4.0, energy: 500)]})"##,
                "test",
                &items,
            )
            .unwrap();
        (
            RecipeIndex::new(&recipes, &smelting, &brewing, &machines),
            items,
        )
    }

    #[test]
    fn an_item_lists_the_recipes_that_make_it_and_those_that_use_it() {
        let (index, items) = index();
        let id = |k: &str| items.id(k).unwrap();
        // Le verre : il cuit dans le four, et sert aux fioles.
        let glass = index.recipes_for(id("core:glass"));
        assert!(glass.iter().any(|r| r.category == RecipeCategory::Smelting));
        let uses = index.uses_of(id("core:glass"));
        assert!(uses.iter().any(|r| r.makes(id("core:flask"))));
        // Les bâtons : une recette d'établi qui accepte toutes les planches.
        let sticks = index.recipes_for(id("core:stick"));
        let grid = sticks
            .iter()
            .find(|r| r.category == RecipeCategory::Crafting)
            .unwrap();
        assert!(grid.uses(id("core:oak_planks")) && grid.uses(id("core:birch_planks")));
        assert!(grid.crafting.is_some());
    }

    #[test]
    fn fuels_brewing_and_mod_machines_are_indexed_too() {
        let (index, items) = index();
        let id = |k: &str| items.id(k).unwrap();
        let coal = index.uses_of(id("core:coal"));
        assert!(coal.iter().any(|r| r.category == RecipeCategory::Fuel));
        let healing = index.recipes_for(id("core:potion_healing"));
        assert!(
            healing
                .iter()
                .any(|r| r.category == RecipeCategory::Brewing)
        );
        let press = index.uses_of(id("core:birch_planks"));
        let machine = press
            .iter()
            .find(|r| r.category == RecipeCategory::Machine("test:press".into()))
            .unwrap();
        assert!(matches!(
            machine.layout,
            RecipeLayout::Machine { energy: 500, .. }
        ));
        assert!(machine.inputs().iter().any(|c| c.count == 2));
    }

    #[test]
    fn recipes_group_by_category_in_tab_order() {
        let (index, items) = index();
        let planks = items.id("core:oak_planks").unwrap();
        let uses = index.uses_of(planks);
        let groups = RecipeIndex::by_category(&uses);
        assert!(groups.len() >= 2, "{groups:?}");
        assert_eq!(groups[0].0, RecipeCategory::Crafting);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, uses.len());
    }
}
