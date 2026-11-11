//! `cargo xtask new-mod <id>` : le squelette d'un mod qui compile et se teste tout de suite.
//! `cargo xtask test-mod <id>` : compile le script puis lance ses tests sans fenêtre.

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Un identifiant de mod : minuscules, chiffres et `_`, qui commence par une lettre.
fn validate(id: &str) -> Result<()> {
    let ok = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !ok {
        bail!(
            "identifiant « {id} » : minuscules, chiffres et _ seulement, en commençant par une lettre"
        );
    }
    if id == "core" {
        bail!("« core » est réservé au jeu de base");
    }
    Ok(())
}

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let id = args
        .first()
        .context("usage : cargo xtask new-mod <identifiant>")?;
    validate(id)?;
    let dir = root.join("mods").join(id);
    if dir.exists() {
        bail!("{} existe déjà", dir.display());
    }
    let write = |relative: &str, text: String| -> Result<()> {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().context("chemin")?)?;
        std::fs::write(&path, text).with_context(|| format!("écriture de {}", path.display()))
    };

    write(
        "mod.ron",
        format!(
            r#"// Le manifeste du mod. Voir docs/modding.md.
(
    id: "{id}",
    name: "{id}",
    version: "0.1.0",
    authors: [],
    description: "",
    dependencies: [(id: "core", version: "^0.1")],
    // Construit par `cargo xtask build-mods` depuis `script/`.
    script: "{id}.wasm",
    // `true` : une page d'options dans le menu pause (`Mod::options` dans le script).
    options: false,
)
"#
        ),
    )?;
    write(
        "blocks.ron",
        format!(
            r#"// Les blocs du mod : tous les identifiants commencent par `{id}:`.
// Une texture se met dans `textures/<nom>.png` (16 x 16) et s'indique avec `texture: "<nom>"`.
// Un bloc peut aussi avoir des boîtes (`boxes`) et des états (`states`) : voir docs/modding.md.
[
    (id: "{id}:example", name: "Bloc d'exemple", color: (0.3, 0.6, 0.9),
     behavior: "example"),
]
"#
        ),
    )?;
    write(
        "script/Cargo.toml",
        format!(
            r#"[package]
name = "{id}_mod"
description = "Le script du mod {id}."
edition.workspace = true
version.workspace = true
license.workspace = true
publish = false

# `cdylib` : une bibliothèque autonome, ici un module WebAssembly (`cargo xtask build-mods`).
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
mod_sdk = {{ path = "../../../crates/mod_sdk" }}

# Pour les tests sans fenêtre (`cargo xtask test-mod {id}`).
[dev-dependencies]
engine_mods = {{ path = "../../../crates/engine_mods" }}
engine_voxel = {{ path = "../../../crates/engine_voxel" }}
glam = {{ workspace = true }}

[lints.clippy]
all = {{ level = "warn", priority = -1 }}
"#
        ),
    )?;
    write(
        "script/src/lib.rs",
        format!(
            r#"//! Le script du mod `{id}`, compilé en WebAssembly par `cargo xtask build-mods`.
//!
//! Une machine (un bloc avec `behavior`) est réveillée par lot : `Mod::machines` reçoit toutes
//! celles à traiter d'un coup. Voir docs/modding.md pour tout ce que le jeu fournit.

use mod_sdk::ui::Menu;
use mod_sdk::{{MachineCall, Mod, Pos}};

#[derive(Default)]
struct Example;

impl Mod for Example {{
    fn init(&mut self) {{
        mod_sdk::log("prêt");
    }}

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {{
        for &pos in positions {{
            if call == MachineCall::Placed {{
                // Un tick par seconde (20 ticks par seconde).
                mod_sdk::schedule_tick(pos, 20);
            }}
        }}
    }}

    /// Clic droit sur la machine : ouvre un menu (le script le décrit, le jeu le dessine).
    fn used(&mut self, pos: Pos) -> bool {{
        mod_sdk::show_menu(pos, &Menu::new("Exemple").label("Bonjour !"));
        true
    }}
}}

mod_sdk::export_mod!(Example);
"#
        ),
    )?;
    write(
        "script/tests/smoke.rs",
        format!(
            r#"//! Les tests du mod, sans fenêtre : le vrai script compilé, dans un petit monde.
//! Lancer : `cargo xtask test-mod {id}`.

use engine_mods::testing::Harness;
use engine_voxel::MachineAction;
use glam::IVec3;

#[test]
fn right_click_opens_the_menu() {{
    let mut h = Harness::for_mod("{id}").unwrap();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "{id}:example");
    let (taken, requests) = h.use_block(at).unwrap();
    assert!(taken);
    assert!(requests
        .actions
        .iter()
        .any(|a| matches!(a, MachineAction::ShowMenu {{ .. }})));
}}
"#
        ),
    )?;

    // Le script rejoint le workspace Cargo.
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)?;
    let anchor = "    \"mods/example/script\",\n";
    if !text.contains(anchor) {
        bail!("Cargo.toml : pas de ligne « mods/example/script » où ajouter le nouveau mod");
    }
    let member = format!("{anchor}    \"mods/{id}/script\",\n");
    std::fs::write(&manifest, text.replacen(anchor, &member, 1))?;

    println!("mod « {id} » créé dans {}", dir.display());
    println!("  cargo xtask build-mods      compile le script");
    println!("  cargo xtask test-mod {id}   le compile et lance ses tests");
    println!("Lancer le jeu : le mod est chargé automatiquement (blocs rechargés à chaud).");
    Ok(())
}

/// Compile les scripts, puis lance les tests du paquet du mod.
pub fn test(root: &Path, args: &[String], build: impl FnOnce() -> Result<()>) -> Result<()> {
    let id = args
        .first()
        .context("usage : cargo xtask test-mod <identifiant>")?;
    validate(id)?;
    if !root
        .join("mods")
        .join(id)
        .join("script/Cargo.toml")
        .exists()
    {
        bail!("mods/{id}/script/Cargo.toml introuvable (cargo xtask new-mod {id} ?)");
    }
    build()?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let package = format!("{id}_mod");
    let status = Command::new(cargo)
        .current_dir(root)
        .args(["test", "--package", &package])
        .args(&args[1..])
        .status()?;
    if !status.success() {
        bail!("les tests de {package} échouent");
    }
    Ok(())
}
