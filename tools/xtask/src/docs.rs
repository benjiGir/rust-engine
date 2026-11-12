//! `cargo xtask docs` : la référence du SDK des mods, générée depuis ses commentaires de
//! documentation (`cargo doc`). Le guide à lire d'abord est `docs/modding.md` ; cette page
//! détaille chaque fonction, chaque type et chaque méthode du trait `Mod`.
//!
//! `--check` échoue au moindre avertissement de rustdoc (un lien cassé vers une fonction qui
//! n'existe plus, par exemple) : la documentation ne peut pas dériver du code sans qu'on le voie.

use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let check = args.iter().any(|a| a == "--check");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args(["doc", "--package", "mod_sdk", "--no-deps"]);
    if check {
        command.env("RUSTDOCFLAGS", "-D warnings");
    }
    if !command.status()?.success() {
        bail!("la documentation du SDK a des erreurs (voir ci-dessus)");
    }
    let index = root.join("target/doc/mod_sdk/index.html");
    println!("référence du SDK : {}", index.display());
    Ok(())
}
