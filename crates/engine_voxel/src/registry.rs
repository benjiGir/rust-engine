//! Le registre des blocs : quels blocs existent et quelles sont leurs propriétés.
//!
//! Les blocs ne sont pas écrits dans le code : ils sont décrits dans des fichiers de
//! données (`assets/core/blocks.ron`, puis `mods/<nom>/blocks.ron`), lus au démarrage.
//! Ajouter un bloc ne demande donc pas de recompiler le jeu : c'est la base des mods.
//!
//! # Deux identifiants par bloc
//!
//! - Un **identifiant texte** avec espace de noms, comme Minecraft : `core:stone`,
//!   `example:ruby`. L'espace de noms (le dossier du mod) évite les collisions :
//!   deux mods peuvent chacun avoir un bloc `ruby` sans se gêner. C'est lui qui est
//!   stable dans le temps et qu'on utilise dans les fichiers.
//! - Un **identifiant numérique** [`BlockId`] (`u16`), attribué au chargement dans
//!   l'ordre de lecture (core, puis les mods par ordre alphabétique). C'est lui qu'on
//!   stocke dans les chunks : 2 octets par bloc, comparaisons instantanées.
//!
//! Le numéro dépend des mods installés : ajouter ou retirer un mod décale les numéros
//! des suivants. Il ne faut donc **jamais** écrire ces numéros sur disque tels quels.
//! Une sauvegarde (phase 8) devra écrire aussi la table « numéro → identifiant texte »,
//! et la retraduire au chargement avec le registre du moment.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;

/// L'espace de noms du jeu lui-même. Aucun mod ne peut l'utiliser.
pub const CORE_NAMESPACE: &str = "core";

/// Les blocs de base, intégrés au binaire : utilisés par les tests et si le dossier
/// `assets` est introuvable. `include_str!` lit le fichier à la compilation.
const BUILTIN_CORE: &str = include_str!("../../../assets/core/blocks.ron");

/// Les propriétés d'un bloc.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDef {
    /// Identifiant texte, par exemple `core:stone`.
    pub key: String,
    /// Nom affiché.
    pub name: String,
    pub color: [f32; 3],
    /// Le joueur ne peut pas le traverser.
    pub solid: bool,
    /// On voit à travers : les faces des blocs voisins restent dessinées.
    pub transparent: bool,
    /// Le joueur peut le viser et le casser.
    pub selectable: bool,
    /// Il apparaît dans la barre de blocs.
    pub placeable: bool,
    /// D'où vient la définition, pour les messages d'erreur.
    pub source: String,
}

/// Un bloc tel qu'écrit dans un fichier `blocks.ron`.
///
/// `Deserialize` (de serde) génère le code qui lit cette struct depuis le fichier.
/// `deny_unknown_fields` refuse un champ mal orthographié au lieu de l'ignorer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFileEntry {
    id: String,
    name: String,
    color: (f32, f32, f32),
    #[serde(default = "yes")]
    solid: bool,
    #[serde(default)]
    transparent: bool,
    #[serde(default = "yes")]
    selectable: bool,
    #[serde(default = "yes")]
    placeable: bool,
}

fn yes() -> bool {
    true
}

/// Tous les blocs connus. L'indice dans `defs` est le `BlockId`.
#[derive(Debug, Clone)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_key: HashMap<String, BlockId>,
}

impl BlockRegistry {
    /// Un registre qui ne contient que l'air (id 0, réservé).
    pub fn empty() -> Self {
        let air = BlockDef {
            key: format!("{CORE_NAMESPACE}:air"),
            name: "Air".into(),
            color: [0.0; 3],
            solid: false,
            transparent: true,
            selectable: false,
            placeable: false,
            source: "intégré".into(),
        };
        Self {
            by_key: HashMap::from([(air.key.clone(), BlockId::AIR)]),
            defs: vec![air],
        }
    }

    /// Les blocs de base intégrés au binaire, sans mod.
    ///
    /// `OnceLock` : le registre est construit au premier appel puis partagé.
    /// Les appels suivants (de n'importe quel thread) reçoivent le même `Arc`.
    pub fn builtin() -> Arc<Self> {
        static BUILTIN: OnceLock<Arc<BlockRegistry>> = OnceLock::new();
        BUILTIN
            .get_or_init(|| {
                let mut registry = Self::empty();
                registry
                    .add_source(CORE_NAMESPACE, BUILTIN_CORE, "core intégré")
                    .expect("assets/core/blocks.ron invalide");
                Arc::new(registry)
            })
            .clone()
    }

    /// Charge `assets/core/blocks.ron`, puis `mods/<nom>/blocks.ron` pour chaque
    /// dossier de `mods_dir` (s'il existe), par ordre alphabétique.
    pub fn load(assets_dir: &Path, mods_dir: Option<&Path>) -> Result<Self> {
        let mut registry = Self::empty();
        registry.add_file(CORE_NAMESPACE, &assets_dir.join("core").join("blocks.ron"))?;

        let Some(mods_dir) = mods_dir.filter(|dir| dir.is_dir()) else {
            return Ok(registry);
        };
        // `collect::<Result<Vec<_>, _>>()` : la première erreur interrompt tout.
        let mut mods = std::fs::read_dir(mods_dir)
            .with_context(|| format!("lecture de {}", mods_dir.display()))?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()?;
        mods.retain(|path| path.is_dir());
        // Trier rend l'ordre (donc les numéros) identique sur toutes les machines :
        // `read_dir` ne garantit aucun ordre.
        mods.sort();
        for dir in mods {
            let file = dir.join("blocks.ron");
            if !file.exists() {
                continue; // Un mod sans nouveaux blocs.
            }
            let namespace = dir
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("nom de mod illisible : {}", dir.display()))?;
            if namespace == CORE_NAMESPACE {
                bail!(
                    "le mod {} ne peut pas s'appeler « core », ce nom est réservé",
                    dir.display()
                );
            }
            registry.add_file(namespace, &file)?;
        }
        Ok(registry)
    }

    fn add_file(&mut self, namespace: &str, path: &Path) -> Result<()> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        self.add_source(namespace, &source, &path.display().to_string())
    }

    /// Ajoute les blocs décrits par `source` (texte RON). `origin` sert aux messages d'erreur.
    pub fn add_source(&mut self, namespace: &str, source: &str, origin: &str) -> Result<()> {
        let entries: Vec<BlockFileEntry> =
            ron::from_str(source).with_context(|| format!("{origin} : fichier RON invalide"))?;
        for entry in entries {
            validate_key(&entry.id, namespace).with_context(|| origin.to_string())?;
            if let Some(&existing) = self.by_key.get(&entry.id) {
                bail!(
                    "le bloc « {} » est défini deux fois : dans {} puis dans {origin}",
                    entry.id,
                    self.def(existing).source
                );
            }
            // `try_from` échoue au-delà de 65 535 : pas de débordement silencieux.
            let id = u16::try_from(self.defs.len())
                .map(BlockId)
                .context("trop de blocs (65 535 au maximum)")?;
            let (r, g, b) = entry.color;
            self.by_key.insert(entry.id.clone(), id);
            self.defs.push(BlockDef {
                key: entry.id,
                name: entry.name,
                color: [r, g, b],
                solid: entry.solid,
                transparent: entry.transparent,
                selectable: entry.selectable,
                placeable: entry.placeable,
                source: origin.to_string(),
            });
        }
        Ok(())
    }

    /// Le numéro d'un bloc à partir de son identifiant texte.
    pub fn id(&self, key: &str) -> Option<BlockId> {
        self.by_key.get(key).copied()
    }

    /// Les propriétés d'un bloc. Un numéro inconnu donne celles de l'air.
    #[inline]
    pub fn def(&self, id: BlockId) -> &BlockDef {
        self.defs.get(id.0 as usize).unwrap_or(&self.defs[0])
    }

    #[inline]
    pub fn is_solid(&self, id: BlockId) -> bool {
        self.def(id).solid
    }

    #[inline]
    pub fn is_transparent(&self, id: BlockId) -> bool {
        self.def(id).transparent
    }

    #[inline]
    pub fn is_selectable(&self, id: BlockId) -> bool {
        self.def(id).selectable
    }

    /// Les blocs de la barre de blocs, dans l'ordre du registre.
    pub fn placeable(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.iter()
            .filter(|(_, def)| def.placeable)
            .map(|(id, _)| id)
    }

    /// Tous les blocs, air compris, avec leur numéro.
    pub fn iter(&self) -> impl Iterator<Item = (BlockId, &BlockDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, def)| (BlockId(i as u16), def))
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Un identifiant valide ressemble à `espace:nom`, en minuscules, chiffres et `_`,
/// et son espace de noms est celui du fichier qui le définit.
fn validate_key(key: &str, namespace: &str) -> Result<()> {
    let Some((ns, name)) = key.split_once(':') else {
        bail!("« {key} » : il manque l'espace de noms (attendu « {namespace}:nom »)");
    };
    if ns != namespace {
        bail!("« {key} » : ce fichier ne peut définir que des blocs « {namespace}:... »");
    }
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    if !valid(name) {
        bail!("« {key} » : le nom ne doit contenir que a-z, 0-9 et _");
    }
    Ok(())
}

/// Des raccourcis pour les tests de la crate : le numéro des blocs de base.
#[cfg(test)]
pub(crate) mod test_blocks {
    use super::*;

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }
    pub fn grass() -> BlockId {
        id("core:grass")
    }
    pub fn dirt() -> BlockId {
        id("core:dirt")
    }
    pub fn stone() -> BlockId {
        id("core:stone")
    }
    pub fn sand() -> BlockId {
        id("core:sand")
    }
    pub fn snow() -> BlockId {
        id("core:snow")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// La racine du dépôt, où sont `assets/` et `mods/`.
    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Un dossier temporaire neuf, propre à ce test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voxel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn builtin_has_the_core_blocks() {
        let registry = BlockRegistry::builtin();
        assert_eq!(registry.id("core:air"), Some(BlockId::AIR));
        let stone = registry.id("core:stone").unwrap();
        assert_eq!(registry.def(stone).name, "Pierre");
        assert!(registry.is_solid(stone) && !registry.is_transparent(stone));
        assert!(!registry.is_solid(BlockId::AIR) && registry.is_transparent(BlockId::AIR));
        assert_eq!(registry.placeable().count(), 5);
    }

    #[test]
    fn loads_core_then_mods() {
        let registry =
            BlockRegistry::load(&repo().join("assets"), Some(&repo().join("mods"))).unwrap();
        let ruby = registry
            .id("example:ruby")
            .expect("le bloc du mod est chargé");
        // Core d'abord : le rubis vient après les 5 blocs de base (et l'air).
        assert_eq!(ruby, BlockId(6));
        assert!(registry.def(ruby).placeable);
        assert!(registry.placeable().any(|id| id == ruby));
    }

    #[test]
    fn same_files_give_the_same_ids() {
        let load =
            || BlockRegistry::load(&repo().join("assets"), Some(&repo().join("mods"))).unwrap();
        let (a, b) = (load(), load());
        let keys = |r: &BlockRegistry| {
            r.iter()
                .map(|(id, d)| (id, d.key.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&a), keys(&b));
    }

    #[test]
    fn mods_load_in_alphabetical_order() {
        let root = temp_dir("order");
        write(root.join("assets/core/blocks.ron"), "[]");
        write(
            root.join("mods/zeta/blocks.ron"),
            r#"[(id: "zeta:z", name: "Z", color: (1.0, 1.0, 1.0))]"#,
        );
        write(
            root.join("mods/alpha/blocks.ron"),
            r#"[(id: "alpha:a", name: "A", color: (1.0, 1.0, 1.0))]"#,
        );
        let registry = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap();
        assert_eq!(registry.id("alpha:a"), Some(BlockId(1)));
        assert_eq!(registry.id("zeta:z"), Some(BlockId(2)));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut registry = BlockRegistry::builtin().as_ref().clone();
        let err = registry
            .add_source(
                CORE_NAMESPACE,
                r#"[(id: "core:stone", name: "Autre", color: (0.0, 0.0, 0.0))]"#,
                "doublon.ron",
            )
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains("core:stone") && message.contains("doublon.ron"),
            "{message}"
        );
    }

    #[test]
    fn wrong_namespace_and_bad_names_are_refused() {
        let mut registry = BlockRegistry::empty();
        let entry = |key: &str| format!(r#"[(id: "{key}", name: "X", color: (0.0, 0.0, 0.0))]"#);
        assert!(
            registry
                .add_source("mymod", &entry("core:hack"), "m")
                .is_err()
        );
        assert!(registry.add_source("mymod", &entry("ruby"), "m").is_err());
        assert!(
            registry
                .add_source("mymod", &entry("mymod:Ruby"), "m")
                .is_err()
        );
        assert!(
            registry
                .add_source("mymod", &entry("mymod:ruby_2"), "m")
                .is_ok()
        );
    }

    #[test]
    fn mod_named_core_is_refused() {
        let root = temp_dir("core-mod");
        write(root.join("assets/core/blocks.ron"), "[]");
        write(root.join("mods/core/blocks.ron"), "[]");
        assert!(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).is_err());
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut registry = BlockRegistry::empty();
        let source = r#"[(id: "core:x", name: "X", color: (0.0, 0.0, 0.0), solide: false)]"#;
        assert!(
            registry
                .add_source(CORE_NAMESPACE, source, "faute.ron")
                .is_err()
        );
    }
}
