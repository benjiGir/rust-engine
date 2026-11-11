//! Le manifeste d'un mod (`mods/<id>/mod.ron`) et l'ordre de chargement.
//!
//! Un mod peut dépendre d'autres mods (« mon mod d'usine a besoin de `energy` en 2.x »).
//! Ils doivent donc être chargés dans un ordre où chaque mod passe après ses dépendances :
//! c'est un **tri topologique** du graphe des dépendances. S'il y a un cycle (A dépend
//! de B qui dépend de A), aucun ordre n'existe : c'est une erreur, à expliquer clairement.
//!
//! Les versions suivent le « versionnage sémantique » (semver), comme Cargo :
//! `MAJEUR.MINEUR.CORRECTIF`. Une exigence `^1.2` accepte 1.2.0 à 1.x, pas 2.0.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use semver::{Version, VersionReq};
use serde::Deserialize;

/// L'identifiant du jeu de base, traité comme un mod comme les autres.
pub const CORE: &str = "core";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub id: String,
    /// Une exigence semver : `"^1.0"`, `">=0.2, <0.4"`, `"*"`...
    pub version: VersionReq,
}

/// Le contenu de `mod.ron`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModManifest {
    /// L'identifiant : aussi le nom du dossier et l'espace de noms de ses blocs.
    pub id: String,
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    /// Le script WebAssembly du mod, relatif à son dossier (facultatif).
    #[serde(default)]
    pub script: Option<String>,
    /// Le mod a une page d'options (`Mod::options` dans son script), dans le menu pause.
    #[serde(default)]
    pub options: bool,
}

/// Un mod trouvé sur le disque.
#[derive(Debug, Clone, PartialEq)]
pub struct ModInfo {
    pub manifest: ModManifest,
    /// Son dossier (`assets/core` pour le jeu de base).
    pub dir: PathBuf,
}

impl ModInfo {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    /// Le chemin du script `.wasm`, s'il y en a un.
    pub fn script_path(&self) -> Option<PathBuf> {
        self.manifest.script.as_ref().map(|s| self.dir.join(s))
    }
}

/// Le manifeste du jeu de base (il n'a pas de fichier : c'est le jeu).
pub fn core_mod(assets_core: &Path) -> ModInfo {
    ModInfo {
        manifest: ModManifest {
            id: CORE.into(),
            name: "Jeu de base".into(),
            version: Version::parse(env!("CARGO_PKG_VERSION")).expect("version du moteur valide"),
            authors: Vec::new(),
            description: String::new(),
            dependencies: Vec::new(),
            script: None,
            options: false,
        },
        dir: assets_core.to_path_buf(),
    }
}

/// Lit le manifeste d'un dossier de mod.
pub fn read_manifest(dir: &Path) -> Result<ModInfo> {
    let path = dir.join("mod.ron");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("lecture de {}", path.display()))?;
    // `IMPLICIT_SOME` : `script: "x.wasm"` au lieu de `script: Some("x.wasm")`.
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    let manifest: ModManifest = options
        .from_str(&text)
        .with_context(|| format!("{} invalide", path.display()))?;
    let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if manifest.id != folder {
        bail!(
            "{} : l'identifiant « {} » doit être le nom du dossier (« {folder} »)",
            path.display(),
            manifest.id
        );
    }
    if manifest.id == CORE {
        bail!("{} : « core » est réservé au jeu de base", path.display());
    }
    Ok(ModInfo {
        manifest,
        dir: dir.to_path_buf(),
    })
}

/// Tous les mods de `mods_dir` (les dossiers qui contiennent un `mod.ron`).
pub fn discover(mods_dir: &Path) -> Result<Vec<ModInfo>> {
    if !mods_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(mods_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("mod.ron").exists())
        .collect();
    dirs.sort();
    dirs.iter().map(|d| read_manifest(d)).collect()
}

/// L'ordre de chargement : chaque mod après ses dépendances, `core` en premier.
/// À dépendances égales, l'ordre alphabétique départage : le résultat ne dépend pas de
/// l'ordre de lecture des dossiers, il est le même sur toutes les machines.
pub fn load_order(core: ModInfo, mods: Vec<ModInfo>) -> Result<Vec<ModInfo>> {
    let mut all: BTreeMap<String, ModInfo> = BTreeMap::new();
    all.insert(core.id().to_string(), core);
    for m in mods {
        if all.contains_key(m.id()) {
            bail!("deux mods ont l'identifiant « {} »", m.id());
        }
        all.insert(m.id().to_string(), m);
    }

    // 1. Toutes les dépendances existent, dans une version acceptée.
    for m in all.values() {
        for dep in &m.manifest.dependencies {
            let Some(found) = all.get(&dep.id) else {
                bail!(
                    "le mod « {} » a besoin de « {} » ({}), qui n'est pas installé",
                    m.id(),
                    dep.id,
                    dep.version
                );
            };
            if !dep.version.matches(&found.manifest.version) {
                bail!(
                    "le mod « {} » a besoin de « {} » en version {}, mais la version installée est {}",
                    m.id(),
                    dep.id,
                    dep.version,
                    found.manifest.version
                );
            }
        }
    }

    // 2. Tri topologique (algorithme de Kahn) : on prend, parmi les mods dont toutes les
    //    dépendances sont déjà placées, le premier dans l'ordre alphabétique.
    //    `core` passe toujours en premier : tout mod en dépend implicitement.
    let mut order: Vec<String> = Vec::new();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    while placed.len() < all.len() {
        let next = all.values().find(|m| {
            !placed.contains(m.id())
                && (m.id() == CORE || placed.contains(CORE))
                && m.manifest
                    .dependencies
                    .iter()
                    .all(|d| placed.contains(&d.id))
        });
        let Some(next) = next else {
            // Plus personne n'est prêt : les mods restants se bloquent mutuellement.
            let stuck: Vec<String> = all
                .keys()
                .filter(|id| !placed.contains(*id))
                .cloned()
                .collect();
            bail!(
                "dépendances circulaires entre les mods : {}",
                stuck.join(", ")
            );
        };
        placed.insert(next.id().to_string());
        order.push(next.id().to_string());
    }
    Ok(order.into_iter().filter_map(|id| all.remove(&id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, version: &str, deps: &[(&str, &str)]) -> ModInfo {
        ModInfo {
            manifest: ModManifest {
                id: id.into(),
                name: id.into(),
                version: Version::parse(version).unwrap(),
                authors: vec![],
                description: String::new(),
                dependencies: deps
                    .iter()
                    .map(|(id, req)| Dependency {
                        id: (*id).into(),
                        version: VersionReq::parse(req).unwrap(),
                    })
                    .collect(),
                script: None,
                options: false,
            },
            dir: PathBuf::from(id),
        }
    }

    fn core() -> ModInfo {
        info(CORE, "0.1.0", &[])
    }

    fn ids(order: &[ModInfo]) -> Vec<&str> {
        order.iter().map(|m| m.id()).collect()
    }

    #[test]
    fn dependencies_load_first_then_alphabetical() {
        let mods = vec![
            info("factory", "1.0.0", &[("energy", "^2.0")]),
            info("energy", "2.3.1", &[]),
            info("aardvark", "0.1.0", &[]),
        ];
        let order = load_order(core(), mods).unwrap();
        assert_eq!(ids(&order), ["core", "aardvark", "energy", "factory"]);
    }

    #[test]
    fn missing_dependency_is_explained() {
        let err = load_order(
            core(),
            vec![info("factory", "1.0.0", &[("energy", "^2.0")])],
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("factory")
                && message.contains("energy")
                && message.contains("pas installé"),
            "{message}"
        );
    }

    #[test]
    fn wrong_version_is_explained() {
        let mods = vec![
            info("factory", "1.0.0", &[("energy", "^2.0")]),
            info("energy", "1.9.0", &[]),
        ];
        let message = load_order(core(), mods).unwrap_err().to_string();
        assert!(
            message.contains("^2.0") && message.contains("1.9.0"),
            "{message}"
        );
    }

    #[test]
    fn cycles_are_detected() {
        let mods = vec![
            info("a", "1.0.0", &[("b", "*")]),
            info("b", "1.0.0", &[("a", "*")]),
            info("c", "1.0.0", &[]),
        ];
        let message = load_order(core(), mods).unwrap_err().to_string();
        assert!(
            message.contains("circulaires") && message.contains("a, b"),
            "{message}"
        );
    }

    #[test]
    fn mods_may_depend_on_core_with_a_version() {
        let order = load_order(core(), vec![info("x", "1.0.0", &[("core", ">=0.1")])]).unwrap();
        assert_eq!(ids(&order), ["core", "x"]);
        let err = load_order(core(), vec![info("x", "1.0.0", &[("core", "^2")])]);
        assert!(err.is_err());
    }

    #[test]
    fn the_example_mod_manifest_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods");
        let mods = discover(&dir).unwrap();
        let example = mods
            .iter()
            .find(|m| m.id() == "example")
            .expect("mods/example/mod.ron");
        assert!(example.manifest.script.is_some());
        load_order(core(), mods).unwrap();
    }
}
