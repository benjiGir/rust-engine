//! `cargo xtask check-arch` : les règles d'architecture, vérifiées à chaque CI.
//!
//! 1. **Le moteur ne connaît pas le jeu** : aucune crate de `crates/` ne dépend d'un
//!    paquet de `games/`, `mods/`, `tools/` ou `exercises/`. Sinon le moteur ne pourrait
//!    plus servir à un autre jeu (le city builder).
//! 2. **La simulation ne dépend pas de l'affichage** : le monde, l'ECS et les mods
//!    tournent sans fenêtre ni GPU (tests, serveur un jour). Ils ne dépendent ni de
//!    winit, ni de wgpu, ni d'egui, ni de `engine_core` / `engine_render`.
//! 3. **Certaines crates restent sans dépendance** (`engine_trace`, `mod_sdk`) :
//!    toutes les autres s'en servent, ou elles sont compilées en WebAssembly.
//! 4. **Pas de fichier géant** : un fichier `.rs` de plus de [`MAX_LINES`] lignes est
//!    difficile à lire et ralentit la compilation incrémentale. On le découpe en modules.
//!
//! Idée reprise d'Horizon (`tools/check_layers.py`, `tools/check_file_size.py`).
//! Les manifestes sont lus ligne à ligne, sans bibliothèque TOML : xtask reste léger.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// Au-delà, un fichier doit être découpé.
const MAX_LINES: usize = 2500;

/// Dossiers dont les paquets ne doivent jamais être une dépendance du moteur.
const NOT_ENGINE: [&str; 4] = ["games", "mods", "tools", "exercises"];

/// Crates de simulation, et ce qu'elles n'ont pas le droit d'utiliser.
const HEADLESS: [&str; 6] = [
    "engine_voxel",
    "engine_ecs",
    "engine_mods",
    "engine_trace",
    "engine_net",
    "engine_inspect",
];
const DISPLAY: [&str; 6] = [
    "winit",
    "wgpu",
    "egui",
    "egui-wgpu",
    "egui-winit",
    "engine_render",
];
/// `engine_core` est la fenêtre et la boucle : à part de `DISPLAY` pour un message clair.
const WINDOW: &str = "engine_core";

/// Crates sans aucune dépendance (hors tests).
const NO_DEPENDENCIES: [&str; 2] = ["engine_trace", "mod_sdk"];

/// Un paquet du workspace et ses dépendances.
struct Package {
    name: String,
    dir: PathBuf,
    /// `[dependencies]` (et `[target.*.dependencies]`).
    normal: Vec<String>,
    /// Toutes, y compris `[dev-dependencies]` et `[build-dependencies]`.
    all: Vec<String>,
}

pub fn run(root: &Path) -> Result<()> {
    let packages = packages(root)?;
    let mut errors = Vec::new();

    let outside: Vec<&str> = packages
        .iter()
        .filter(|p| NOT_ENGINE.iter().any(|d| p.dir.starts_with(root.join(d))))
        .map(|p| p.name.as_str())
        .collect();
    for package in packages
        .iter()
        .filter(|p| p.dir.starts_with(root.join("crates")))
    {
        for dep in &package.all {
            if outside.contains(&dep.as_str()) {
                errors.push(format!(
                    "{} dépend de {dep} : le moteur ne doit pas connaître le jeu, les mods ni les outils",
                    package.name
                ));
            }
        }
        if HEADLESS.contains(&package.name.as_str()) {
            for dep in &package.normal {
                if DISPLAY.contains(&dep.as_str()) || dep == WINDOW {
                    errors.push(format!(
                        "{} dépend de {dep} : la simulation doit tourner sans fenêtre ni GPU",
                        package.name
                    ));
                }
            }
        }
        if NO_DEPENDENCIES.contains(&package.name.as_str()) && !package.normal.is_empty() {
            errors.push(format!(
                "{} doit rester sans dépendance, mais utilise : {}",
                package.name,
                package.normal.join(", ")
            ));
        }
    }

    let mut files = Vec::new();
    for dir in ["crates", "games", "mods", "tools", "exercises"] {
        rust_files(&root.join(dir), &mut files)?;
    }
    files.sort();
    for file in &files {
        let lines = std::fs::read_to_string(file)
            .with_context(|| format!("lecture de {}", file.display()))?
            .lines()
            .count();
        if lines > MAX_LINES {
            errors.push(format!(
                "{} fait {lines} lignes (maximum {MAX_LINES}) : à découper en modules",
                file.strip_prefix(root).unwrap_or(file).display()
            ));
        }
    }

    if !errors.is_empty() {
        for error in &errors {
            eprintln!("✗ {error}");
        }
        bail!("{} règle(s) d'architecture non respectée(s)", errors.len());
    }
    println!(
        "architecture : {} paquets et {} fichiers .rs vérifiés, tout est en ordre",
        packages.len(),
        files.len()
    );
    Ok(())
}

/// Les paquets du workspace : chaque dossier de premier niveau sous `crates`, `games`,
/// `tools`, `exercises`, et `mods/<id>/script`.
fn packages(root: &Path) -> Result<Vec<Package>> {
    let mut dirs = Vec::new();
    for top in ["crates", "games", "tools", "exercises", "mods"] {
        let Ok(entries) = std::fs::read_dir(root.join(top)) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let dir = entry.path();
            let dir = if top == "mods" {
                dir.join("script")
            } else {
                dir
            };
            if dir.join("Cargo.toml").exists() {
                dirs.push(dir);
            }
        }
    }
    dirs.sort();
    dirs.into_iter()
        .map(|dir| {
            let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
                .with_context(|| format!("lecture de {}/Cargo.toml", dir.display()))?;
            parse_manifest(&manifest, dir)
        })
        .collect()
}

/// Le nom du paquet et ses dépendances, lus dans un `Cargo.toml`.
fn parse_manifest(manifest: &str, dir: PathBuf) -> Result<Package> {
    #[derive(PartialEq)]
    enum Section {
        Package,
        Normal,
        Other,
        Ignored,
    }
    let mut section = Section::Ignored;
    let mut name = None;
    let mut normal = Vec::new();
    let mut all = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            // `[dependencies.foo]` : une dépendance écrite comme une table à part.
            let (table, inline_dep) = match header.rsplit_once('.') {
                Some((t, d)) if t.ends_with("dependencies") => (t, Some(d)),
                _ => (header, None),
            };
            section = if header == "package" {
                Section::Package
            } else if table == "dependencies"
                || (table.starts_with("target.") && table.ends_with(".dependencies"))
            {
                Section::Normal
            } else if table.ends_with("dependencies") {
                Section::Other
            } else {
                Section::Ignored
            };
            if let Some(dep) = inline_dep {
                push_dep(dep, section == Section::Normal, &mut normal, &mut all);
                section = Section::Ignored;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        match section {
            Section::Package if key == "name" => {
                name = Some(value.trim().trim_matches('"').to_string());
            }
            // `foo = ...` ou `foo.workspace = true`.
            Section::Normal | Section::Other => {
                let dep = key.split('.').next().unwrap_or(key);
                push_dep(dep, section == Section::Normal, &mut normal, &mut all);
            }
            _ => {}
        }
    }
    let name = name.with_context(|| format!("pas de nom de paquet dans {}", dir.display()))?;
    Ok(Package {
        name,
        dir,
        normal,
        all,
    })
}

fn push_dep(dep: &str, normal: bool, normals: &mut Vec<String>, all: &mut Vec<String>) {
    let dep = dep.trim().trim_matches('"').to_string();
    if normal {
        normals.push(dep.clone());
    }
    all.push(dep);
}

/// Tous les `.rs` sous `dir`, sauf dans les dossiers `target`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                rust_files(&path, out)?;
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_dependencies_are_sorted_by_kind() {
        let manifest = r#"
[package]
name = "engine_demo"
version.workspace = true

[dependencies]
glam = { workspace = true }
engine_trace.workspace = true
"rayon" = "1"

[dev-dependencies]
voxel = { path = "../../games/voxel" }

[target.'cfg(windows)'.dependencies]
mimalloc = "0.1"

[dependencies.wgpu]
version = "30"

[lints]
workspace = true
"#;
        let package = parse_manifest(manifest, PathBuf::from("crates/engine_demo")).unwrap();
        assert_eq!(package.name, "engine_demo");
        assert_eq!(
            package.normal,
            ["glam", "engine_trace", "rayon", "mimalloc", "wgpu"]
        );
        assert_eq!(
            package.all,
            ["glam", "engine_trace", "rayon", "voxel", "mimalloc", "wgpu"]
        );
    }

    #[test]
    fn this_repository_follows_the_rules() {
        run(&crate::root()).unwrap();
    }
}
