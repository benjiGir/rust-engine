//! La palette de matériaux d'un monde : à quel bloc correspond le numéro rangé dans une pièce
//! taillée.
//!
//! Une pile de pièces (« Dalle de chêne ») ne porte qu'un entier, `ItemStack::damage`. On y range
//! le
//! **numéro du matériau** dans la palette du monde (`materials.ron`, à la racine du monde, à côté
//! de `items.bin`) : une liste d'identifiants texte, un numéro attribué au premier usage et
//! **jamais réutilisé** (le numéro 1 est le premier matériau taillé, le 0 ne désigne rien). Deux
//! piles de même pièce et même matériau s'empilent donc comme n'importe quels objets.
//!
//! Le fichier ne contient que des identifiants texte : il reste valable quand la liste des mods
//! change (un matériau inconnu garde son nom, la pile s'affiche avec ce nom brut). Les cases posées
//! dans le monde, elles, portent déjà leurs matériaux en toutes lettres (voir
//! `engine_voxel::shaped`).
//!
//! Comme le magasin des fiches d'objets (`player/item_data.rs`), la palette du monde ouvert est
//! unique pour le processus : les piles sont lues partout par des fonctions qui n'ont que `damage`
//! sous la main.

use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

use anyhow::{Context as _, Result};
use engine_voxel::shaped::PieceKind;
use engine_voxel::{BlockRegistry, ItemDef};
use serde::{Deserialize, Serialize};

const FILE: &str = "materials.ron";

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Saved {
    /// Le matériau numéro `n` est `materials[n - 1]`.
    materials: Vec<String>,
}

/// La palette d'un monde.
#[derive(Debug, Default, PartialEq)]
pub struct Palette {
    materials: Vec<String>,
    dirty: bool,
}

impl Palette {
    /// Le numéro de ce matériau, attribué au premier usage.
    pub fn number_of(&mut self, key: &str) -> u32 {
        if let Some(i) = self.materials.iter().position(|m| m == key) {
            return i as u32 + 1;
        }
        self.materials.push(key.to_string());
        self.dirty = true;
        self.materials.len() as u32
    }

    /// Le matériau de ce numéro.
    pub fn key_of(&self, number: u32) -> Option<&str> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        self.materials.get(index).map(String::as_str)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Relit `materials.ron` (une palette vide s'il n'existe pas). Un fichier illisible est mis de
    /// côté (`materials.ron.illisible`), jamais écrasé.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match ron::from_str::<Saved>(&text) {
            Ok(saved) => Self {
                materials: saved.materials,
                dirty: false,
            },
            Err(err) => {
                log::error!("{} illisible ({err}) : mis de côté", path.display());
                let _ = std::fs::copy(&path, dir.join("materials.ron.illisible"));
                Self::default()
            }
        }
    }

    /// Écrit `materials.ron` (le fichier n'existe qu'une fois un matériau taillé).
    pub fn save(&mut self, dir: &Path) -> Result<()> {
        if self.materials.is_empty() {
            return Ok(());
        }
        let text = ron::ser::to_string_pretty(
            &Saved {
                materials: self.materials.clone(),
            },
            ron::ser::PrettyConfig::default(),
        )
        .context("encodage de la palette de matériaux")?;
        engine_voxel::write_atomic(&dir.join(FILE), text.as_bytes())?;
        self.dirty = false;
        Ok(())
    }
}

static PALETTE: LazyLock<Mutex<Palette>> = LazyLock::new(Mutex::default);

fn lock() -> MutexGuard<'static, Palette> {
    PALETTE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Passe au monde ouvert : sa palette remplace celle du précédent.
pub fn replace_all(palette: Palette) {
    *lock() = palette;
}

/// Le numéro de ce matériau dans le monde ouvert (attribué au premier usage).
pub fn number_of(key: &str) -> u32 {
    lock().number_of(key)
}

/// Le matériau de ce numéro dans le monde ouvert.
pub fn key_of(number: u32) -> Option<String> {
    lock().key_of(number).map(str::to_string)
}

/// Sauvegarde la palette du monde ouvert dans `dir` si elle a changé.
pub fn save_if_dirty(dir: &Path) -> Result<()> {
    let mut guard = lock();
    if guard.dirty {
        guard.save(dir)?;
    }
    Ok(())
}

/// La sorte de pièce de cet objet, si c'en est une.
pub fn kind_of(def: &ItemDef) -> Option<PieceKind> {
    PieceKind::from_key(&def.key)
}

/// Le nom d'un bloc matériau, ou son identifiant brut s'il est inconnu (mod retiré).
pub fn material_name(blocks: &BlockRegistry, key: &str) -> String {
    blocks
        .id(key)
        .map_or_else(|| key.to_string(), |id| blocks.def(id).name.clone())
}

/// Le nom d'une pile de pièces : « Dalle de chêne ». `None` pour un autre objet, ou une pile sans
/// matériau (numéro 0, donnée par la console).
pub fn piece_name(blocks: &BlockRegistry, def: &ItemDef, damage: u32) -> Option<String> {
    let kind = kind_of(def)?;
    let key = key_of(damage)?;
    let material = material_name(blocks, &key);
    // « Pente de chêne » : le nom du matériau, en minuscule, sans son article.
    let material = material
        .strip_prefix("Planches de ")
        .or_else(|| material.strip_prefix("Bloc de "))
        .unwrap_or(&material);
    Some(format!("{} de {}", kind.name(), lowercase_first(material)))
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_given_on_first_use_and_never_reused() {
        let mut p = Palette::default();
        assert_eq!(p.key_of(0), None);
        assert_eq!(p.number_of("core:oak_planks"), 1);
        assert_eq!(p.number_of("core:stone"), 2);
        assert_eq!(p.number_of("core:oak_planks"), 1);
        assert_eq!(p.key_of(2), Some("core:stone"));
        assert_eq!(p.key_of(3), None);
    }

    #[test]
    fn the_palette_survives_a_save_and_keeps_unknown_materials() {
        let dir = std::env::temp_dir().join(format!("voxel-materials-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Palette::default();
        p.number_of("core:oak_planks");
        p.number_of("mod:disparu");
        p.save(&dir).unwrap();
        assert!(dir.join(FILE).exists());
        let back = Palette::load(&dir);
        assert_eq!(back.key_of(2), Some("mod:disparu"));
        assert_eq!(back.len(), 2);
        // Un fichier abîmé est mis de côté, pas écrasé.
        std::fs::write(dir.join(FILE), "pas du ron").unwrap();
        assert_eq!(Palette::load(&dir).len(), 0);
        assert!(dir.join("materials.ron.illisible").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_piece_is_named_after_its_material() {
        let blocks = engine_voxel::BlockRegistry::builtin();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let slab = items.get(items.id("core:shape_slab").unwrap());
        let n = number_of("core:stone");
        let name = piece_name(&blocks, slab, n).unwrap();
        assert!(name.starts_with("Dalle de "), "{name}");
        assert_eq!(piece_name(&blocks, slab, 0), None);
        let pick = items.get(items.id("core:iron_pickaxe").unwrap());
        assert_eq!(piece_name(&blocks, pick, n), None);
    }
}
