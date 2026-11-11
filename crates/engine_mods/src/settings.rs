//! Les réglages des mods : des paires clé → texte par mod, que le joueur règle dans la page
//! d'options du mod et que le script relit. Ils vivent dans un fichier à la racine du projet
//! (`mod_settings.ron`), à part des options du jeu et valables pour tous les mondes.
//!
//! Les valeurs sont du **texte** : lisible et modifiable à la main, et le SDK sait les lire
//! en nombre ou en booléen. Les tailles sont bornées pour qu'un mod ne remplisse pas le disque.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Longueur maximale d'une clé et d'une valeur (octets), et nombre de clés par mod.
pub const MAX_KEY: usize = 64;
pub const MAX_VALUE: usize = 1024;
pub const MAX_KEYS: usize = 256;
/// Les données d'un monde (`mod_data.ron` dans son dossier) : plus grandes que des réglages.
pub const MAX_WORLD_VALUE: usize = 64 * 1024;
pub const MAX_WORLD_KEYS: usize = 1024;

/// Les bornes d'un magasin : longueur d'une valeur et nombre de clés par mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Limits {
    value: usize,
    keys: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            value: MAX_VALUE,
            keys: MAX_KEYS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModSettings {
    values: BTreeMap<String, BTreeMap<String, String>>,
    /// Modifié depuis la dernière écriture sur disque.
    #[serde(skip)]
    dirty: bool,
    #[serde(skip)]
    limits: Limits,
}

impl ModSettings {
    /// Les mêmes données, avec les bornes des données d'un monde (valeurs jusqu'à 64 Kio,
    /// 1 024 clés par mod).
    pub fn for_world(mut self) -> Self {
        self.limits = Limits {
            value: MAX_WORLD_VALUE,
            keys: MAX_WORLD_KEYS,
        };
        self
    }

    /// Lit le fichier ; un fichier absent ou illisible donne des réglages vides.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => ron::from_str(&text).unwrap_or_else(|err| {
                log::warn!(
                    "{} illisible ({err}) : réglages de mods vides",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Écrit le fichier s'il y a eu des changements.
    pub fn save_if_dirty(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        match ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()) {
            Ok(text) => {
                if let Err(err) = engine_voxel::write_atomic(path, text.as_bytes()) {
                    log::error!("{} : {err:#}", path.display());
                }
            }
            Err(err) => log::error!("réglages de mods : {err}"),
        }
    }

    pub fn get(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.values.get(mod_id)?.get(key).map(String::as_str)
    }

    /// Range une valeur. `false` si la clé ou la valeur est trop longue, ou s'il y a trop de clés.
    pub fn set(&mut self, mod_id: &str, key: &str, value: &str) -> bool {
        if key.is_empty() || key.len() > MAX_KEY || value.len() > self.limits.value {
            return false;
        }
        let entry = self.values.entry(mod_id.to_string()).or_default();
        if !entry.contains_key(key) && entry.len() >= self.limits.keys {
            return false;
        }
        if entry.get(key).map(String::as_str) != Some(value) {
            entry.insert(key.to_string(), value.to_string());
            self.dirty = true;
        }
        true
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_per_mod_and_bounded() {
        let mut s = ModSettings::default();
        assert!(s.set("a", "speed", "3.5"));
        assert!(s.set("b", "speed", "9"));
        assert_eq!(s.get("a", "speed"), Some("3.5"));
        assert_eq!(s.get("b", "speed"), Some("9"));
        assert_eq!(s.get("a", "absent"), None);
        assert!(!s.set("a", "", "x"));
        assert!(!s.set("a", &"k".repeat(MAX_KEY + 1), "x"));
        assert!(!s.set("a", "big", &"v".repeat(MAX_VALUE + 1)));
        for i in 0..MAX_KEYS {
            s.set("c", &format!("k{i}"), "1");
        }
        assert!(!s.set("c", "one_more", "1"));
        assert!(
            s.set("c", "k0", "2"),
            "une clé existante se modifie toujours"
        );
    }

    #[test]
    fn world_data_allows_bigger_values() {
        let mut data = ModSettings::default().for_world();
        let big = "v".repeat(MAX_WORLD_VALUE);
        assert!(data.set("a", "table", &big));
        assert!(!data.set("a", "table2", &"v".repeat(MAX_WORLD_VALUE + 1)));
        let mut settings = ModSettings::default();
        assert!(
            !settings.set("a", "table", &big),
            "les réglages restent petits"
        );
    }

    #[test]
    fn only_a_real_change_marks_dirty_and_the_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("mod_settings_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mod_settings.ron");
        let mut s = ModSettings::default();
        s.set("a", "k", "v");
        assert!(s.is_dirty());
        s.save_if_dirty(&path);
        assert!(!s.is_dirty());
        s.set("a", "k", "v");
        assert!(!s.is_dirty(), "même valeur : rien à écrire");
        let back = ModSettings::load(&path);
        assert_eq!(back.get("a", "k"), Some("v"));
        // Un fichier illisible ne bloque pas le jeu.
        std::fs::write(&path, "pas du ron {").unwrap();
        assert_eq!(ModSettings::load(&path), ModSettings::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
