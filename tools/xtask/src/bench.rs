//! `cargo xtask bench` : lance le benchmark du jeu (`VOXEL_BENCH`, voir
//! `games/voxel/src/control/bench.rs`) plusieurs fois, garde la médiane de chaque mesure, et
//! compare à un ancien résultat.
//!
//! ```text
//! cargo xtask bench                         # 3 passages, résultat dans target/bench/<commit>.csv
//! cargo xtask bench --runs 5 --only foret   # 5 passages d'un seul trajet
//! cargo xtask bench --compare target/bench/1f03aa3.csv
//! ```
//!
//! Un écart n'est signalé que s'il dépasse le bruit : la dispersion des passages (le plus
//! grand moins le plus petit) des deux résultats, et au moins 5 %.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Les mesures comparées, et si « plus grand » est mieux.
const METRICS: [(&str, &str, bool); 4] = [
    ("cpu_ms", "CPU (ms)", false),
    ("gpu_ms", "GPU (ms)", false),
    // Pas le « 1 % le plus lent » en FPS : sans VSync, il mesure surtout l'attente d'une
    // image libre auprès du système, pas le travail du jeu (voir
    // `games/voxel/src/control/bench.rs`).
    ("cpu_p99_ms", "CPU 99e (ms)", false),
    ("chunks_dessines", "chunks", false),
];

struct Options {
    runs: usize,
    out: Option<PathBuf>,
    compare: Option<PathBuf>,
    only: Option<String>,
}

fn options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        runs: 3,
        out: None,
        compare: None,
        only: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} : valeur manquante"))
        };
        match arg.as_str() {
            "--runs" => options.runs = value()?.parse().context("--runs : un nombre")?,
            "--out" => options.out = Some(value()?.into()),
            "--compare" => options.compare = Some(value()?.into()),
            "--only" => options.only = Some(value()?.clone()),
            other => {
                bail!("option inconnue « {other} » (--runs N, --out f, --compare f, --only a,b)")
            }
        }
    }
    if options.runs == 0 {
        bail!("--runs : au moins 1");
    }
    Ok(options)
}

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let options = options(args)?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    println!("compilation du jeu (release)...");
    let status = Command::new(&cargo)
        .current_dir(root)
        .args(["build", "--release", "--quiet", "--package", "voxel"])
        .status()?;
    if !status.success() {
        bail!("la compilation a échoué");
    }
    let binary = root
        .join("target/release")
        .join(format!("voxel{}", std::env::consts::EXE_SUFFIX));
    let dir = root.join("target/bench");
    std::fs::create_dir_all(&dir)?;

    let mut rows = Vec::new();
    let mut header = String::new();
    for pass in 1..=options.runs {
        println!("passage {pass}/{} (environ 2 minutes)...", options.runs);
        let file = dir.join(format!("passage-{pass}.csv"));
        let _ = std::fs::remove_file(&file);
        let mut command = Command::new(&binary);
        command
            .current_dir(root)
            .env("VOXEL_BENCH", &file)
            .env("RUST_LOG", "warn,voxel::bench=info");
        if let Some(only) = &options.only {
            command.env("VOXEL_BENCH_ONLY", only);
        }
        if !command.status()?.success() || !file.exists() {
            bail!("le passage {pass} a échoué (pas de {})", file.display());
        }
        let text = std::fs::read_to_string(&file)?;
        let mut lines = text.lines();
        header = lines.next().unwrap_or_default().to_string();
        rows.extend(lines.map(|l| format!("{pass},{l}")));
    }

    let out = match options.out {
        Some(out) => out,
        None => dir.join(format!("{}.csv", commit(root))),
    };
    let csv = format!("passage,{header}\n{}\n", rows.join("\n"));
    std::fs::write(&out, &csv)?;
    let results = parse(&csv)?;
    println!("\nrésultat ({} passages) : {}", options.runs, out.display());
    print_summary(&results);

    if let Some(old) = options.compare {
        let text = std::fs::read_to_string(&old)
            .with_context(|| format!("lecture de {}", old.display()))?;
        println!("\ncomparaison avec {} :", old.display());
        print_comparison(&parse(&text)?, &results);
    }
    Ok(())
}

/// Le commit courant (avec `-modifie` si l'arbre de travail a des changements).
fn commit(root: &Path) -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "inconnu".into());
    // Les réglages de l'éditeur (`.idea`) ne changent pas le code mesuré.
    let dirty = git(&[
        "status",
        "--porcelain",
        "--untracked-files=no",
        "--",
        ".",
        ":!.idea",
    ])
    .is_some_and(|s| !s.is_empty());
    if dirty { format!("{sha}-modifie") } else { sha }
}

/// Trajet → mesure → valeurs des passages.
type Results = BTreeMap<String, BTreeMap<String, Vec<f64>>>;

fn parse(csv: &str) -> Result<Results> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<&str> = lines.next().context("fichier vide")?.split(',').collect();
    let route = header
        .iter()
        .position(|h| *h == "trajet")
        .context("pas de colonne « trajet »")?;
    let mut results = Results::new();
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        let Some(name) = cells.get(route) else {
            continue;
        };
        let metrics = results.entry(name.to_string()).or_default();
        for (column, value) in header.iter().zip(&cells) {
            if let Ok(value) = value.parse::<f64>() {
                metrics.entry(column.to_string()).or_default().push(value);
            }
        }
    }
    Ok(results)
}

/// Médiane et dispersion (plus grand moins plus petit).
fn stats(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    // Nombre pair de passages : la moyenne des deux du milieu (sinon, avec 2 passages, on
    // prendrait le plus grand, et un seul passage perturbé ferait croire à un recul).
    let n = sorted.len();
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    };
    Some((median, sorted[sorted.len() - 1] - sorted[0]))
}

fn print_summary(results: &Results) {
    print!("{:<12}", "trajet");
    for (_, label, _) in METRICS {
        print!("{label:>22}");
    }
    println!();
    for (route, metrics) in results {
        print!("{route:<12}");
        for (key, _, _) in METRICS {
            let cell = match metrics.get(key).and_then(|v| stats(v)) {
                Some((median, spread)) => format!("{median:.2} ±{:.2}", spread / 2.0),
                None => "-".into(),
            };
            print!("{cell:>22}");
        }
        println!();
    }
}

/// Le verdict d'un écart : `None` s'il reste dans le bruit.
fn verdict(old: &[f64], new: &[f64], higher_is_better: bool) -> Option<(f64, &'static str)> {
    let (old_median, old_spread) = stats(old)?;
    let (new_median, new_spread) = stats(new)?;
    if old_median == 0.0 {
        return None;
    }
    let change = (new_median - old_median) / old_median;
    let noise = ((old_spread + new_spread) / old_median.abs()).max(0.05);
    if change.abs() <= noise {
        return Some((change, "≈"));
    }
    let better = (change > 0.0) == higher_is_better;
    Some((change, if better { "mieux" } else { "pire" }))
}

fn print_comparison(old: &Results, new: &Results) {
    for (route, metrics) in new {
        let Some(before) = old.get(route) else {
            println!("{route:<12} (absent de l'ancien résultat)");
            continue;
        };
        print!("{route:<12}");
        for (key, label, higher) in METRICS {
            let cell = match (before.get(key), metrics.get(key)) {
                (Some(a), Some(b)) => match verdict(a, b, higher) {
                    Some((change, word)) => format!("{label} {:+.0} % {word}", change * 100.0),
                    None => format!("{label} -"),
                },
                _ => format!("{label} -"),
            };
            print!("  {cell:<28}");
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_are_grouped_by_route() {
        let csv = "passage,trajet,cpu_ms,gpu_ms\n1,foret,2.0,5.0\n2,foret,2.4,\n3,foret,2.2,5.2\n";
        let results = parse(csv).unwrap();
        assert_eq!(results["foret"]["cpu_ms"], [2.0, 2.4, 2.2]);
        assert_eq!(
            results["foret"]["gpu_ms"],
            [5.0, 5.2],
            "les cases vides sont ignorées"
        );
        let (median, spread) = stats(&results["foret"]["cpu_ms"]).unwrap();
        assert!((median - 2.2).abs() < 1e-9 && (spread - 0.4).abs() < 1e-9);
        let (median, _) = stats(&[1.0, 3.0]).unwrap();
        assert!((median - 2.0).abs() < 1e-9, "deux passages : la moyenne");
    }

    #[test]
    fn only_changes_beyond_the_noise_count() {
        // ±10 % de bruit : 5 % d'écart ne compte pas, 50 % si.
        let old = [1.9, 2.0, 2.1];
        assert_eq!(
            verdict(&old, &[2.0, 2.1, 2.2], false).map(|v| v.1),
            Some("≈")
        );
        assert_eq!(
            verdict(&old, &[2.9, 3.0, 3.1], false).map(|v| v.1),
            Some("pire")
        );
        assert_eq!(
            verdict(&old, &[0.9, 1.0, 1.1], false).map(|v| v.1),
            Some("mieux")
        );
        assert_eq!(
            verdict(&old, &[2.9, 3.0, 3.1], true).map(|v| v.1),
            Some("mieux")
        );
    }
}
