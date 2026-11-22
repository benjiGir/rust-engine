//! La sauvegarde des wagons : `carts.bin` dans le dossier de la dimension, à côté des structures
//! mobiles (`grids.bin`, voir `motion/subgrid.rs`).
//!
//! Le moteur ne connaît pas l'ECS du jeu : il range des [`SavedCart`], que le jeu remplit depuis
//! ses entités et relit au chargement. Le format est du postcard ; les objets sont rangés par
//! identifiant texte (comme les inventaires des blocs), la sorte par son nom : une sauvegarde
//! reste valable quand la liste des mods change. Le fichier disparaît quand il n'y a plus de wagon.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::tech::capabilities::ItemStack;

const CARTS_FILE: &str = "carts.bin";

/// Le rail sur lequel roule un wagon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedRail {
    pub block: [i32; 3],
    pub s: f32,
    pub speed: f32,
    pub heading: f32,
}

/// Un wagon : tout ce qu'il faut pour le remettre sur la voie.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedCart {
    /// Le numéro du wagon, stable (les attelages le désignent).
    pub id: u32,
    /// La sorte (`minecart`, `chest_cart`, `hopper_cart`, `locomotive`).
    pub kind: String,
    pub position: [f32; 3],
    pub yaw: f32,
    /// Sur un rail : où. Sinon : `None`, et le wagon roule librement avec `velocity`.
    pub rail: Option<SavedRail>,
    pub velocity: [f32; 3],
    /// Le contenu (coffre, trémie) ou le combustible (locomotive).
    pub items: Vec<Option<ItemStack>>,
    /// Secondes de combustible qui brûlent encore (locomotive).
    pub burn: f32,
    /// Les numéros des wagons attelés à celui-ci.
    pub links: Vec<u32>,
}

/// Écrit les wagons dans `dir/carts.bin` (le fichier disparaît s'il n'y en a plus).
pub fn save_carts(dir: &Path, carts: &[SavedCart]) -> Result<()> {
    let path = dir.join(CARTS_FILE);
    if carts.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(());
    }
    let bytes = postcard::to_allocvec(carts).context("encodage des wagons")?;
    crate::write_atomic(&path, &bytes)
}

/// Relit les wagons d'un monde (aucun si le fichier n'existe pas).
pub fn load_carts(dir: &Path) -> Result<Vec<SavedCart>> {
    let path = dir.join(CARTS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path).with_context(|| format!("lecture de {}", path.display()))?;
    postcard::from_bytes(&bytes).context("décodage des wagons")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cart(id: u32, kind: &str, rail: bool) -> SavedCart {
        SavedCart {
            id,
            kind: kind.into(),
            position: [3.5, 70.06, -2.5],
            yaw: 1.25,
            rail: rail.then(|| SavedRail {
                block: [3, 70, -3],
                s: 0.5,
                speed: -4.25,
                heading: -1.0,
            }),
            velocity: if rail { [0.0; 3] } else { [1.0, -2.0, 0.5] },
            items: vec![
                Some(ItemStack {
                    item: "core:coal".into(),
                    count: 12,
                    damage: 0,
                }),
                None,
            ],
            burn: 17.5,
            links: vec![id + 1],
        }
    }

    #[test]
    fn carts_round_trip_through_the_file_and_the_file_goes_away_when_empty() {
        let dir = std::env::temp_dir().join(format!("voxel-carts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_carts(&dir).unwrap().is_empty());
        let carts = vec![cart(1, "locomotive", true), cart(2, "chest_cart", false)];
        save_carts(&dir, &carts).unwrap();
        assert_eq!(load_carts(&dir).unwrap(), carts);
        save_carts(&dir, &[]).unwrap();
        assert!(!dir.join(CARTS_FILE).exists());
        assert!(load_carts(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
