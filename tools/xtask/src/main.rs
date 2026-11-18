//! Les tâches du projet, en Rust plutôt qu'en script shell : elles marchent pareil
//! sous macOS, Linux et Windows. Lancement : `cargo xtask <commande>` (alias défini
//! dans `.cargo/config.toml`).
//!
//! - `build-mods` : compile le script de chaque mod (`mods/<id>/script`) en WebAssembly
//!   et copie le `.wasm` à l'endroit indiqué par son `mod.ron`.
//! - `new-mod <id>` : crée le squelette d'un mod ; `test-mod <id>` : compile et lance ses tests
//!   sans fenêtre (voir `new_mod.rs`).
//! - `docs [--check]` : génère la référence du SDK des mods (`target/doc/mod_sdk`) ; `--check`
//!   refuse le moindre avertissement de documentation (lien cassé...).
//! - `check-arch` : vérifie les règles d'architecture (voir `check_arch.rs`).
//! - `ctl <commande>...` : pilote le jeu lancé avec `VOXEL_CONTROL` (voir `ctl.rs`).
//! - `bench [--runs N] [--compare ancien.csv]` : le benchmark reproductible (voir `bench.rs`).
//! - `map [options]` : exporte sans fenêtre des cartes PNG d'une graine (altitude, pente, humidité,
//!   température, continentalité, biomes) ; les options sont décrites dans
//!   `crates/engine_voxel/examples/worldmap.rs`.
//! - `hooks [--check]` : génère l'interface des mods depuis son catalogue (voir `hooks.rs`).

mod bench;
mod check_arch;
mod ctl;
mod docs;
mod hooks;
mod new_mod;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

const TARGET: &str = "wasm32-unknown-unknown";

fn main() -> Result<()> {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "build-mods" => build_mods(),
        "check-arch" => check_arch::run(&root()),
        "ctl" => ctl::run(std::env::args().skip(2).collect()),
        "new-mod" => new_mod::run(&root(), &std::env::args().skip(2).collect::<Vec<_>>()),
        "test-mod" => new_mod::test(
            &root(),
            &std::env::args().skip(2).collect::<Vec<_>>(),
            build_mods,
        ),
        "docs" => docs::run(&root(), &std::env::args().skip(2).collect::<Vec<_>>()),
        "hooks" => hooks::run(&root(), &std::env::args().skip(2).collect::<Vec<_>>()),
        "bench" => bench::run(&root(), &std::env::args().skip(2).collect::<Vec<_>>()),
        "map" => map(&std::env::args().skip(2).collect::<Vec<_>>()),
        _ => {
            eprintln!(
                "usage : cargo xtask build-mods | new-mod <id> | test-mod <id> | docs | check-arch | ctl <commande>... | bench | hooks | map"
            );
            bail!("tâche inconnue : « {task} »")
        }
    }
}

/// `cargo xtask map [options]` : lance l'exemple `worldmap` du moteur en release (le calcul du
/// terrain en debug serait lent) et lui passe les options telles quelles.
fn map(args: &[String]) -> Result<()> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root())
        .args([
            "run",
            "--release",
            "--quiet",
            "--package",
            "engine_voxel",
            "--example",
            "worldmap",
            "--",
        ])
        .args(args)
        .status()?;
    if !status.success() {
        bail!("l'export des cartes a échoué");
    }
    Ok(())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Le paquet Cargo d'un dossier de script : la ligne `name = "..."` de son Cargo.toml.
fn package_name(script_dir: &Path) -> Result<String> {
    let manifest = std::fs::read_to_string(script_dir.join("Cargo.toml"))?;
    manifest
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("name = \"")?
                .strip_suffix('"')
                .map(str::to_string)
        })
        .context("pas de `name` dans le Cargo.toml du script")
}

/// Le fichier attendu par `mod.ron` (`script: "example.wasm"`), lu sans désérialiser
/// tout le manifeste : xtask reste sans dépendance lourde.
fn wasm_name(mod_dir: &Path) -> Result<String> {
    let manifest = std::fs::read_to_string(mod_dir.join("mod.ron"))?;
    manifest
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("script: \"")?
                .split('"')
                .next()
                .map(str::to_string)
        })
        .context("pas de `script` dans mod.ron")
}

fn build_mods() -> Result<()> {
    let root = root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut built = 0;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("mods"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("script/Cargo.toml").exists())
        .collect();
    dirs.sort();
    for mod_dir in dirs {
        let package = package_name(&mod_dir.join("script"))?;
        println!("compilation de {package} pour {TARGET}...");
        let status = Command::new(&cargo)
            .current_dir(&root)
            .args([
                "build",
                "--release",
                "--target",
                TARGET,
                "--package",
                &package,
            ])
            .status()?;
        if !status.success() {
            bail!("la compilation de {package} a échoué");
        }
        // Cargo nomme le fichier d'après le paquet, tirets remplacés par des soulignés.
        let output = root
            .join("target")
            .join(TARGET)
            .join("release")
            .join(format!("{}.wasm", package.replace('-', "_")));
        let destination = mod_dir.join(wasm_name(&mod_dir)?);
        std::fs::copy(&output, &destination).with_context(|| {
            format!(
                "copie de {} vers {}",
                output.display(),
                destination.display()
            )
        })?;
        let size = std::fs::metadata(&destination)?.len();
        println!("  → {} ({} Kio)", destination.display(), size / 1024);
        built += 1;
    }
    println!("{built} mod(s) compilé(s)");
    Ok(())
}
