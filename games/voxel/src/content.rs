//! Le contenu du jeu, lu dans les fichiers de données : objets, recettes, fonte, butin.
//!
//! Tout se charge dans l'ordre des mods : `assets/core` d'abord, puis chaque mod qui a
//! ses propres fichiers (`items.ron`, `recipes.ron`, `smelting.ron`, `loot.ron`).
//! Les registres sont dans des `Arc` : partagés sans copie avec les machines (le four).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use engine_voxel::{BlockRegistry, Brewing, ItemRegistry, LootTables, Recipes, Smelting};

#[derive(Clone)]
pub struct Content {
    pub items: Arc<ItemRegistry>,
    pub recipes: Arc<Recipes>,
    pub smelting: Arc<Smelting>,
    pub brewing: Arc<Brewing>,
    pub loot: Arc<LootTables>,
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
        log::info!(
            "{} objets, {} recettes, {} recettes de fonte",
            items.len(),
            recipes.len(),
            smelting.recipes().len()
        );
        Ok(Self {
            items: Arc::new(items),
            recipes: Arc::new(recipes),
            smelting: Arc::new(smelting),
            brewing: Arc::new(brewing),
            loot: Arc::new(loot),
        })
    }
}
