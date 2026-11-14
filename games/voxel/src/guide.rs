//! Le Guide du voyageur : un livre qui explique le jeu, chapitre par chapitre.
//!
//! Le contenu est en données : `assets/core/guide.ron`, puis le `guide.ron` de chaque mod, dont les
//! chapitres s'ajoutent à la suite. Chaque page a un texte et des objets ; l'écran (`guide_ui.rs`)
//! ouvre leurs recettes dans l'explorateur d'un clic.
//!
//! Le texte peut citer une touche : `{touche:inventory}` devient la touche choisie dans les
//! options (voir [`expand_keys`]).

use anyhow::{Context as _, Result, bail};
use engine_voxel::{ItemId, ItemRegistry};
use serde::Deserialize;

use crate::settings::{Action, Keys};

/// L'objet du livre lui-même.
pub const GUIDE_ITEM: &str = "core:guide";

#[derive(Debug, Deserialize)]
struct GuideFile {
    chapters: Vec<ChapterEntry>,
}

#[derive(Debug, Deserialize)]
struct ChapterEntry {
    id: String,
    title: String,
    icon: String,
    pages: Vec<PageEntry>,
}

#[derive(Debug, Deserialize)]
struct PageEntry {
    title: String,
    text: String,
    #[serde(default)]
    items: Vec<String>,
}

/// Une page : un titre, un texte, et les objets montrés dessous.
#[derive(Debug, Clone)]
pub struct Page {
    pub title: String,
    pub text: String,
    pub items: Vec<ItemId>,
}

#[derive(Debug, Clone)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    pub icon: Option<ItemId>,
    pub pages: Vec<Page>,
}

/// Tous les chapitres, core d'abord puis les mods dans l'ordre de chargement.
#[derive(Debug, Clone, Default)]
pub struct Guide {
    pub chapters: Vec<Chapter>,
}

/// `stick` → `core:stick` (dans le fichier de `core`) ; `forge:gear` reste tel quel.
fn scoped(key: &str, namespace: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("{namespace}:{key}")
    }
}

impl Guide {
    /// Ajoute les chapitres d'un fichier `guide.ron`. Un objet inconnu est une erreur : le guide
    /// ne montre que ce qui existe.
    pub fn add_source(
        &mut self,
        namespace: &str,
        text: &str,
        origin: &str,
        items: &ItemRegistry,
    ) -> Result<()> {
        let file: GuideFile = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(text)
            .with_context(|| format!("{origin} : fichier RON invalide"))?;
        let item = |key: &str| -> Result<ItemId> {
            let full = scoped(key, namespace);
            items
                .id(&full)
                .with_context(|| format!("{origin} : objet inconnu « {full} »"))
        };
        for entry in file.chapters {
            if !entry.id.starts_with(&format!("{namespace}:")) {
                bail!(
                    "{origin} : le chapitre « {} » doit commencer par « {namespace}: »",
                    entry.id
                );
            }
            if self.chapters.iter().any(|c| c.id == entry.id) {
                bail!("{origin} : chapitre « {} » en double", entry.id);
            }
            let pages = entry
                .pages
                .into_iter()
                .map(|p| {
                    Ok(Page {
                        title: p.title,
                        text: p.text,
                        items: p.items.iter().map(|k| item(k)).collect::<Result<_>>()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            if pages.is_empty() {
                bail!("{origin} : le chapitre « {} » n'a aucune page", entry.id);
            }
            self.chapters.push(Chapter {
                icon: Some(item(&entry.icon)?),
                id: entry.id,
                title: entry.title,
                pages,
            });
        }
        Ok(())
    }
}

/// Le nom d'une action dans le texte du guide (`{touche:inventory}`), comme dans `settings.ron`.
fn action_of(name: &str) -> Option<Action> {
    Some(match name {
        "forward" => Action::Forward,
        "back" => Action::Back,
        "left" => Action::Left,
        "right" => Action::Right,
        "jump" => Action::Jump,
        "descend" => Action::Descend,
        "sprint" => Action::Sprint,
        "inventory" => Action::Inventory,
        "drop" => Action::Drop,
        "fly" => Action::Fly,
        "time" => Action::Time,
        "backpack" => Action::Backpack,
        "tiers" => Action::Tiers,
        _ => return None,
    })
}

/// Le nom lisible d'une touche : `KeyE` → `E`, `Space` → `Espace`, `ControlLeft` → `Ctrl gauche`.
pub fn key_label(name: &str) -> String {
    let side = |rest: &str, base: &str| {
        let side = match rest {
            "Left" => " gauche",
            "Right" => " droite",
            _ => "",
        };
        format!("{base}{side}")
    };
    if let Some(letter) = name.strip_prefix("Key") {
        return letter.to_string();
    }
    if let Some(digit) = name.strip_prefix("Digit") {
        return digit.to_string();
    }
    if let Some(rest) = name.strip_prefix("Shift") {
        return side(rest, "Maj");
    }
    if let Some(rest) = name.strip_prefix("Control") {
        return side(rest, "Ctrl");
    }
    if let Some(rest) = name.strip_prefix("Alt") {
        return side(rest, "Alt");
    }
    match name {
        "Space" => "Espace".into(),
        "Tab" => "Tab".into(),
        "Enter" => "Entrée".into(),
        "Backspace" => "Retour arrière".into(),
        "ArrowUp" => "Flèche haut".into(),
        "ArrowDown" => "Flèche bas".into(),
        "ArrowLeft" => "Flèche gauche".into(),
        "ArrowRight" => "Flèche droite".into(),
        other => other.into(),
    }
}

/// Remplace chaque `{touche:action}` par la touche choisie, entre crochets (`[E]`). Une action
/// inconnue reste telle quelle, pour se voir.
pub fn expand_keys(text: &str, keys: &Keys) -> String {
    const OPEN: &str = "{touche:";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('}') else {
            rest = &rest[start..];
            break;
        };
        match action_of(&after[..end]) {
            Some(action) => out.push_str(&format!("[{}]", key_label(keys.name(action)))),
            None => out.push_str(&rest[start..start + OPEN.len() + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn items() -> ItemRegistry {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        ItemRegistry::load_mods(&crate::test_registry(), &assets, &[]).unwrap()
    }

    #[test]
    fn the_core_guide_loads_and_shows_only_real_items() {
        let items = items();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core/guide.ron");
        let text = std::fs::read_to_string(assets).unwrap();
        let mut guide = Guide::default();
        guide
            .add_source("core", &text, "guide.ron", &items)
            .unwrap();
        assert!(guide.chapters.len() >= 10);
        assert_eq!(guide.chapters[0].id, "core:first_steps");
        assert!(guide.chapters.iter().all(|c| !c.pages.is_empty()));
        // Chaque citation de touche correspond à une action connue.
        let keys = Keys::default();
        for page in guide.chapters.iter().flat_map(|c| &c.pages) {
            assert!(
                !expand_keys(&page.text, &keys).contains("{touche:"),
                "{}",
                page.title
            );
        }
    }

    #[test]
    fn unknown_items_and_foreign_ids_are_refused() {
        let items = items();
        let mut guide = Guide::default();
        let bad_item = r#"(chapters: [(id: "core:x", title: "X", icon: "stick",
            pages: [(title: "p", text: "t", items: ["licorne"])])])"#;
        assert!(guide.add_source("core", bad_item, "t", &items).is_err());
        let foreign = r#"(chapters: [(id: "autre:x", title: "X", icon: "stick",
            pages: [(title: "p", text: "t")])])"#;
        assert!(guide.add_source("core", foreign, "t", &items).is_err());
        let ok = r#"(chapters: [(id: "core:x", title: "X", icon: "stick",
            pages: [(title: "p", text: "t", items: ["stick", "core:coal"])])])"#;
        guide.add_source("core", ok, "t", &items).unwrap();
        assert_eq!(guide.chapters[0].pages[0].items.len(), 2);
        assert!(
            guide.add_source("core", ok, "t", &items).is_err(),
            "en double"
        );
    }

    #[test]
    fn keys_are_written_with_the_players_bindings() {
        let mut keys = Keys::default();
        assert_eq!(
            expand_keys("Ouvre avec {touche:inventory}.", &keys),
            "Ouvre avec [E]."
        );
        keys.set(
            Action::Inventory,
            engine_core::winit::keyboard::KeyCode::KeyI,
        );
        assert_eq!(expand_keys("{touche:inventory}", &keys), "[I]");
        assert_eq!(expand_keys("{touche:sprint}", &keys), "[Ctrl gauche]");
        assert_eq!(
            expand_keys("{touche:licorne} {touche:", &keys),
            "{touche:licorne} {touche:"
        );
    }
}
