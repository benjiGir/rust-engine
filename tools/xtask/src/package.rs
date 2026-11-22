//! `cargo xtask package [--target <triple>]` : fabrique le dossier du jeu à distribuer,
//! `dist/Voxel-<version>-<système>-<processeur>/`, et son archive `.zip` (voir
//! `docs/plan-installation.md`).
//!
//! 1. compile en release `voxel` et `voxel-server` ;
//! 2. compile les scripts des mods (`build-mods`) ;
//! 3. copie le jeu de base (`assets/core` → `game/core`), chaque mod **sans** `script/` ni `target/`,
//!    les packs de textures et de shaders livrés ;
//! 4. écrit `LISEZMOI.txt` ;
//! 5. zippe le dossier.
//!
//! `settings.ron`, `saves/`, `logs/`... ne sont pas livrés : le jeu les crée au premier lancement,
//! avec les réglages par défaut.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Les dossiers d'un mod qui ne servent qu'à le fabriquer.
const MOD_SOURCES: [&str; 2] = ["script", "target"];

pub fn run(root: &Path, args: &[String], build_mods: fn() -> Result<()>) -> Result<()> {
    let target = match args {
        [] => None,
        [flag, triple] if flag == "--target" => Some(triple.as_str()),
        _ => bail!("usage : cargo xtask package [--target <triple>]"),
    };
    let (os, arch) = platform(target);
    let windows = os == "windows";
    let name = format!("Voxel-{}-{os}-{arch}", env!("CARGO_PKG_VERSION"));
    let dist = root.join("dist");
    let out = dist.join(&name);

    // 1. Les deux exécutables.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut build = Command::new(cargo);
    build.current_dir(root).args([
        "build",
        "--release",
        "--package",
        "voxel",
        "--bin",
        "voxel",
        "--package",
        "voxel_sim",
        "--bin",
        "voxel-server",
    ]);
    if let Some(triple) = target {
        build.args(["--target", triple]);
    }
    println!("compilation en release de voxel et voxel-server...");
    if !build.status()?.success() {
        bail!("la compilation des exécutables a échoué");
    }
    let release = match target {
        Some(triple) => root.join("target").join(triple).join("release"),
        None => root.join("target/release"),
    };

    // 2. Les scripts des mods, à jour.
    build_mods()?;

    // 3. Le dossier, refait de zéro.
    if out.exists() {
        std::fs::remove_dir_all(&out)
            .with_context(|| format!("suppression de l'ancien {}", out.display()))?;
    }
    std::fs::create_dir_all(&out)?;
    let suffix = if windows { ".exe" } else { "" };
    for exe in ["voxel", "voxel-server"] {
        let file = format!("{exe}{suffix}");
        std::fs::copy(release.join(&file), out.join(&file))
            .with_context(|| format!("copie de {}", release.join(&file).display()))?;
    }
    copy_dir(&root.join("assets/core"), &out.join("game/core"), &[])?;
    let mods = copy_each(
        &root.join("mods"),
        &out.join("mods"),
        "mod.ron",
        &MOD_SOURCES,
    )?;
    let textures = copy_each(
        &root.join("texture_packs"),
        &out.join("texture_packs"),
        "pack.ron",
        &[],
    )?;
    let shaders = copy_each(
        &root.join("shader_packs"),
        &out.join("shader_packs"),
        "pack.ron",
        &[],
    )?;
    // Un mod dont le script n'est pas compilé ne se chargerait pas : mieux vaut s'arrêter ici.
    for id in &mods {
        let dir = out.join("mods").join(id);
        if let Ok(wasm) = crate::wasm_name(&dir)
            && !dir.join(&wasm).is_file()
        {
            bail!("le mod {id} n'a pas son script compilé ({wasm})");
        }
    }

    // 4. Le mode d'emploi.
    std::fs::write(out.join("LISEZMOI.txt"), readme(&name, windows))?;

    // 5. L'archive.
    let zip = dist.join(format!("{name}.zip"));
    let _ = std::fs::remove_file(&zip);
    zip_dir(&dist, &name, &zip)?;

    println!(
        "{} : {} mods ({}), {} packs de textures, {} packs de shaders, {:.0} Mo ; archive {} ({:.0} Mo)",
        out.display(),
        mods.len(),
        mods.join(", "),
        textures.len(),
        shaders.len(),
        size(&out) as f64 / 1e6,
        zip.display(),
        std::fs::metadata(&zip).map_or(0, |m| m.len()) as f64 / 1e6,
    );
    Ok(())
}

/// Le système et le processeur visés : ceux du triple, sinon ceux de cette machine.
fn platform(target: Option<&str>) -> (&'static str, String) {
    let Some(triple) = target else {
        let os = match std::env::consts::OS {
            "macos" => "macos",
            "windows" => "windows",
            "linux" => "linux",
            _ => "autre",
        };
        return (os, std::env::consts::ARCH.to_string());
    };
    let os = if triple.contains("windows") {
        "windows"
    } else if triple.contains("apple") {
        "macos"
    } else if triple.contains("linux") {
        "linux"
    } else {
        "autre"
    };
    let arch = triple.split('-').next().unwrap_or("inconnu").to_string();
    (os, arch)
}

/// Copie chaque sous-dossier de `from` qui contient `marker` (un mod, un pack) ; rend leurs noms.
fn copy_each(from: &Path, to: &Path, marker: &str, skip: &[&str]) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(names);
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(marker).is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        copy_dir(&dir, &to.join(&name), skip)?;
        names.push(name);
    }
    Ok(names)
}

/// Copie un dossier, sans les fichiers cachés (`.DS_Store`...) ni les sous-dossiers de `skip` (au
/// premier niveau).
fn copy_dir(from: &Path, to: &Path, skip: &[&str]) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in
        std::fs::read_dir(from).with_context(|| format!("lecture de {}", from.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || skip.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name), &[])?;
        } else {
            std::fs::copy(&path, to.join(&name))
                .with_context(|| format!("copie de {}", path.display()))?;
        }
    }
    Ok(())
}

/// La taille d'un dossier, en octets.
fn size(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .map(|e| {
                let path = e.path();
                if path.is_dir() {
                    size(&path)
                } else {
                    e.metadata().map_or(0, |m| m.len())
                }
            })
            .sum()
    })
}

/// Zippe `dist/<name>` avec l'outil du système : `ditto` sous macOS, `tar` (bsdtar) sous Windows,
/// `zip` ailleurs.
fn zip_dir(dist: &Path, name: &str, zip: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("ditto");
        c.args([
            "-c",
            "-k",
            "--norsrc",
            "--noextattr",
            "--noacl",
            "--keepParent",
        ])
        .arg(name)
        .arg(zip);
        c
    } else if cfg!(windows) {
        // Le `tar` de Windows (bsdtar, qui sait faire un .zip), pas le GNU tar qu'apporte Git.
        let tar = std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
            .filter(|tar| tar.is_file())
            .unwrap_or_else(|| PathBuf::from("tar"));
        let mut c = Command::new(tar);
        c.args(["-a", "-c", "-f"]).arg(zip).arg(name);
        c
    } else {
        let mut c = Command::new("zip");
        c.args(["-q", "-r"]).arg(zip).arg(name);
        c
    };
    let status = command
        .current_dir(dist)
        .status()
        .context("outil d'archive introuvable")?;
    if !status.success() {
        bail!("la création de {} a échoué", zip.display());
    }
    Ok(())
}

fn readme(name: &str, windows: bool) -> String {
    let (game, server) = if windows {
        ("voxel.exe", "voxel-server.exe")
    } else {
        ("./voxel", "./voxel-server")
    };
    let mac = if cfg!(target_os = "macos") && !windows {
        "\nSous macOS, le jeu n'est pas signé : au premier lancement, clic droit sur « voxel » puis\n« Ouvrir », ou dans un terminal : xattr -dr com.apple.quarantine <ce dossier>\n"
    } else {
        ""
    };
    format!(
        "{name}
{underline}

JOUER
  Lancer {game}. Les mondes, les réglages, les captures (F2) et le journal
  (logs/latest.log) s'écrivent dans ce dossier, à côté du jeu : on peut le copier
  ou le sauvegarder tel quel.
{mac}
AJOUTER UN MOD
  Déposer le dossier du mod (celui qui contient mod.ron) dans mods/. Les archives
  .zip ne sont pas lues : il faut les décompresser. Un mod illisible est écarté,
  et le journal dit pourquoi.

AJOUTER UN PACK
  Packs de textures : dans texture_packs/. Packs de shaders : dans shader_packs/.
  On les choisit dans le menu principal.

HÉBERGER UNE PARTIE
  Lancer {server}. Au premier lancement, il écrit server.ron (port, monde,
  nombre de joueurs, liste blanche...) : le modifier puis relancer. Il lit les
  mêmes mods que le jeu ; les joueurs doivent avoir les mêmes.
  --data-dir <dossier> : les mondes, les mods et server.ron de ce serveur sont
  là (plusieurs serveurs sur une machine).

DOSSIER EN LECTURE SEULE
  Si ce dossier n'est pas inscriptible (Program Files, /Applications), le jeu
  écrit dans le dossier utilisateur du système (%APPDATA%\\Voxel,
  ~/Library/Application Support/Voxel ou ~/.local/share/voxel) et y lit aussi
  mods/ et les packs. Le journal dit où il écrit.

CONTENU
  game/           le jeu de base (remplacé à chaque mise à jour)
  mods/           les mods livrés et ajoutés
  texture_packs/  les packs de textures
  shader_packs/   les packs de shaders
",
        underline = "=".repeat(name.chars().count()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_comes_from_the_triple() {
        assert_eq!(
            platform(Some("x86_64-pc-windows-msvc")),
            ("windows", "x86_64".to_string())
        );
        assert_eq!(
            platform(Some("aarch64-apple-darwin")),
            ("macos", "aarch64".to_string())
        );
        assert_eq!(
            platform(Some("x86_64-unknown-linux-gnu")),
            ("linux", "x86_64".to_string())
        );
    }

    #[test]
    fn mod_sources_and_hidden_files_are_left_out() {
        let base = std::env::temp_dir().join(format!("xtask-package-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let from = base.join("mods");
        for file in [
            "a/mod.ron",
            "a/a.wasm",
            "a/textures/x.png",
            "a/script/src/lib.rs",
            "a/target/y",
            "a/.DS_Store",
            "pas_un_mod/z",
        ] {
            let path = from.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let to = base.join("out");
        let names = copy_each(&from, &to, "mod.ron", &MOD_SOURCES).unwrap();
        assert_eq!(names, ["a"]);
        assert!(to.join("a/a.wasm").is_file() && to.join("a/textures/x.png").is_file());
        assert!(!to.join("a/script").exists() && !to.join("a/target").exists());
        assert!(!to.join("a/.DS_Store").exists() && !to.join("pas_un_mod").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
