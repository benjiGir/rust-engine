//! Exporte des cartes d'une graine sans ouvrir de fenêtre : `cargo xtask map` (qui appelle cet
//! exemple en release). Voir `engine_voxel::MapSamples` pour ce que montre chaque carte.
//!
//! ```text
//! cargo xtask map                                   # graine 42, 512 × 512 pixels à 4 blocs, toutes les cartes
//! cargo xtask map --seed 7 --x -2048 --z -2048 --size 1024 --step 4
//! cargo xtask map --maps altitude,biomes --out target/maps/essai --ppm
//! cargo xtask map --generator v2 --size 1024 --step 8 --maps terre-mer,drainage,bassins
//! ```
//!
//! Options : `--generator v1|v2` (v1 : le monde réel ; v2 : le prototype macro des lots 2 et 3, sans blocs ;
//! cartes `terre-mer`, `rivage`, `altitude-macro`, `altitude`, `chaines`, `familles`, `bassins`, `drainage`,
//! `lacs`), `--seed N` (42), `--x X --z Z` (coin de la zone, en blocs ; 0 0), `--size L[xH]`
//! (pixels ; 512), `--step B` (blocs par pixel ; 4), `--maps a,b` (altitude, pente, humidite,
//! temperature, continentalite, biomes ; toutes), `--out dossier` (target/maps/<graine>), `--ppm`
//! (du PPM au lieu du PNG). Les fichiers sont nommés `<carte>.png` ; la légende des biomes est
//! affichée et écrite dans `biomes.txt`.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use engine_voxel::{
    MacroMapKind, MacroMapSamples, MacroWorld, MapArea, MapKind, MapSamples, Raster,
    TerrainGenerator,
};

/// Le générateur dont on dessine les cartes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generator {
    /// La génération historique (v1) : les cartes du monde réel.
    V1,
    /// Le prototype macro v2 (lots 2 et 3) : géographie et drainage, sans blocs.
    V2,
}

struct Options {
    generator: Generator,
    seed: u32,
    area: MapArea,
    /// Les noms de cartes demandés ; `None` : toutes celles du générateur.
    maps: Option<Vec<String>>,
    out: Option<PathBuf>,
    ppm: bool,
}

fn options() -> Result<Options> {
    let mut options = Options {
        generator: Generator::V1,
        seed: 42,
        area: MapArea {
            x: 0,
            z: 0,
            width: 512,
            height: 512,
            step: 4,
        },
        maps: None,
        out: None,
        ppm: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .with_context(|| format!("{arg} : valeur manquante"))
        };
        match arg.as_str() {
            "--seed" => options.seed = value()?.parse().context("--seed : un entier positif")?,
            "--x" => options.area.x = value()?.parse().context("--x : un entier")?,
            "--z" => options.area.z = value()?.parse().context("--z : un entier")?,
            "--step" => options.area.step = value()?.parse().context("--step : un entier")?,
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').unwrap_or((&text, &text));
                options.area.width = w.parse().context("--size : L ou LxH")?;
                options.area.height = h.parse().context("--size : L ou LxH")?;
            }
            "--generator" => {
                options.generator = match value()?.as_str() {
                    "v1" => Generator::V1,
                    "v2" => Generator::V2,
                    other => bail!("--generator : v1 ou v2, pas « {other} »"),
                };
            }
            "--maps" => {
                options.maps = Some(value()?.split(',').map(|n| n.trim().to_string()).collect());
            }
            "--out" => options.out = Some(value()?.into()),
            "--ppm" => options.ppm = true,
            other => bail!(
                "option inconnue « {other} » (--generator, --seed, --x, --z, --size, --step, --maps, --out, --ppm)"
            ),
        }
    }
    let area = options.area;
    if area.width == 0 || area.height == 0 || area.step == 0 || area.width.max(area.height) > 8192 {
        bail!("--size de 1 à 8192 pixels et --step d'au moins 1");
    }
    Ok(options)
}

fn write_png(path: &std::path::Path, raster: &Raster) -> Result<()> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path)?),
        raster.width,
        raster.height,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&raster.rgb)?;
    Ok(())
}

fn save(out: &std::path::Path, name: &str, raster: &Raster, ppm: bool) -> Result<()> {
    let path = out.join(format!("{name}.{}", if ppm { "ppm" } else { "png" }));
    if ppm {
        std::fs::write(&path, raster.to_ppm())?;
    } else {
        write_png(&path, raster)?;
    }
    println!("  {}", path.display());
    Ok(())
}

fn main() -> Result<()> {
    let options = options()?;
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("target/maps/{}", options.seed)));
    std::fs::create_dir_all(&out)?;
    let area = options.area;
    println!(
        "graine {}, zone ({}, {}) à ({}, {}) blocs, {} × {} pixels de {} blocs",
        options.seed,
        area.x,
        area.z,
        area.x + (area.width * area.step) as i32,
        area.z + (area.height * area.step) as i32,
        area.width,
        area.height,
        area.step
    );
    if options.generator == Generator::V2 {
        let kinds: Vec<MacroMapKind> = match &options.maps {
            None => MacroMapKind::ALL.to_vec(),
            Some(names) => names
                .iter()
                .map(|n| {
                    MacroMapKind::from_name(n).with_context(|| {
                        format!(
                            "carte v2 inconnue « {n} » ({})",
                            MacroMapKind::ALL.map(|k| k.name()).join(", ")
                        )
                    })
                })
                .collect::<Result<_>>()?,
        };
        let world = MacroWorld::new(options.seed);
        let started = std::time::Instant::now();
        let samples = MacroMapSamples::compute(&world, area);
        println!(
            "générateur v2 (macro) : {} ms, {:?}",
            started.elapsed().as_millis(),
            world.hydrology().stats()
        );
        for kind in kinds {
            save(&out, kind.name(), &samples.render(kind), options.ppm)?;
        }
        return Ok(());
    }
    let kinds: Vec<MapKind> = match &options.maps {
        None => MapKind::ALL.to_vec(),
        Some(names) => names
            .iter()
            .map(|n| MapKind::from_name(n).with_context(|| format!("carte inconnue « {n} »")))
            .collect::<Result<_>>()?,
    };
    let generator = TerrainGenerator::new(options.seed);
    let samples = MapSamples::compute(&generator, area);
    for kind in &kinds {
        save(&out, kind.name(), &samples.render(*kind), options.ppm)?;
    }
    if kinds.contains(&MapKind::Biomes) {
        let legend: String = samples
            .biome_legend()
            .iter()
            .map(|(key, [r, g, b])| format!("#{r:02x}{g:02x}{b:02x} {key}\n"))
            .collect();
        std::fs::write(out.join("biomes.txt"), &legend)?;
        print!("légende des biomes :\n{legend}");
    }
    Ok(())
}
