//! Le contenu du jeu, lu dans les fichiers de données : objets, recettes, fonte, butin.
//!
//! Tout se charge dans l'ordre des mods : `assets/core` d'abord, puis chaque mod qui a
//! ses propres fichiers (`items.ron`, `recipes.ron`, `smelting.ron`, `loot.ron`).
//! Les registres sont dans des `Arc` : partagés sans copie avec les machines (le four).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use engine_voxel::{
    BlockRegistry, Brewing, CompactTable, ItemRegistry, LootTables, MachineRecipes, RecipeIndex,
    Recipes, Smelting,
};

#[derive(Clone)]
pub struct Content {
    pub items: Arc<ItemRegistry>,
    pub recipes: Arc<Recipes>,
    pub smelting: Arc<Smelting>,
    pub brewing: Arc<Brewing>,
    pub loot: Arc<LootTables>,
    pub machine_recipes: Arc<MachineRecipes>,
    /// Toutes les recettes, par produit et par ingrédient (l'explorateur, voir `recipe_browser.rs`).
    pub recipe_index: Arc<RecipeIndex>,
    /// Ce que l'amélioration « compactage » sait regrouper (lu dans les recettes).
    pub compact: Arc<CompactTable>,
    /// Le Guide du voyageur (voir `guide.rs`).
    pub guide: Arc<crate::guide::Guide>,
}

/// Les fichiers `name` de core puis des mods, avec leur espace de noms.
fn sources(
    assets: &Path,
    mods: &[(String, PathBuf)],
    name: &str,
) -> Result<Vec<(String, String, String)>> {
    let mut dirs = vec![("core".to_string(), assets.join("core"))];
    dirs.extend(mods.iter().cloned());
    let mut out = Vec::new();
    for (namespace, dir) in dirs {
        let path = dir.join(name);
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("lecture de {}", path.display()))?;
            out.push((namespace, text, path.display().to_string()));
        }
    }
    Ok(out)
}

impl Content {
    pub fn load(blocks: &BlockRegistry, assets: &Path, mods: &[(String, PathBuf)]) -> Result<Self> {
        let items = ItemRegistry::load_mods(blocks, assets, mods)?;
        let mut recipes = Recipes::default();
        for (ns, text, origin) in sources(assets, mods, "recipes.ron")? {
            recipes.add_source(&ns, &text, &origin, &items)?;
        }
        let mut smelting = Smelting::default();
        for (ns, text, origin) in sources(assets, mods, "smelting.ron")? {
            smelting.add_source(&ns, &text, &origin, &items)?;
        }
        let mut brewing = Brewing::default();
        for (ns, text, origin) in sources(assets, mods, "brewing.ron")? {
            brewing.add_source(&ns, &text, &origin, &items)?;
        }
        let mut loot = LootTables::default();
        for (ns, text, origin) in sources(assets, mods, "loot.ron")? {
            loot.add_source(&ns, &text, &origin, blocks, &items)?;
        }
        let mut machine_recipes = MachineRecipes::default();
        for (ns, text, origin) in sources(assets, mods, "machine_recipes.ron")? {
            machine_recipes.add_source(&ns, &text, &origin, &items)?;
        }
        log::info!(
            "{} objets, {} recettes, {} recettes de fonte",
            items.len(),
            recipes.len(),
            smelting.recipes().len()
        );
        let mut guide = crate::guide::Guide::default();
        for (ns, text, origin) in sources(assets, mods, "guide.ron")? {
            guide.add_source(&ns, &text, &origin, &items)?;
        }
        let recipe_index = RecipeIndex::new(&recipes, &smelting, &brewing, &machine_recipes);
        let compact = CompactTable::from_recipes(&recipes, &items);
        // L'aventure : les fichiers de core et des mods, vérifiés contre les objets, puis installés
        // pour les modules purs (voir `adventure_data::install`).
        let adventure = crate::adventure_data::AdventureData::load(assets, mods)
            .context("chargement des données d'aventure")?;
        adventure
            .validate_items(&items)
            .context("données d'aventure")?;
        crate::adventure_data::install(Arc::new(adventure));
        Ok(Self {
            compact: Arc::new(compact),
            recipe_index: Arc::new(recipe_index),
            items: Arc::new(items),
            recipes: Arc::new(recipes),
            smelting: Arc::new(smelting),
            brewing: Arc::new(brewing),
            loot: Arc::new(loot),
            machine_recipes: Arc::new(machine_recipes),
            guide: Arc::new(guide),
        })
    }
}
