//! Exporte des cartes d'une graine sans ouvrir de fenêtre : `cargo xtask map` (qui appelle cet
//! exemple en release). Voir `engine_voxel::MapSamples` pour ce que montre chaque carte.
//!
//! ```text
//! cargo xtask map                                   # graine 42, 512 × 512 pixels à 4 blocs, toutes les cartes
//! cargo xtask map --seed 7 --x -2048 --z -2048 --size 1024 --step 4
//! cargo xtask map --maps altitude,biomes --out target/maps/essai --ppm
//! ```
//!
//! Options : `--seed N` (42), `--x X --z Z` (coin de la zone, en blocs ; 0 0), `--size L[xH]`
//! (pixels ; 512), `--step B` (blocs par pixel ; 4), `--maps a,b` (altitude, pente, humidite,
//! temperature, continentalite, biomes ; toutes), `--out dossier` (target/maps/<graine>), `--ppm`
//! (du PPM au lieu du PNG). Les fichiers sont nommés `<carte>.png` ; la légende des biomes est
//! affichée et écrite dans `biomes.txt`.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use engine_voxel::{MapArea, MapKind, MapSamples, Raster, TerrainGenerator};

struct Options {
    seed: u32,
    area: MapArea,
    maps: Vec<MapKind>,
    out: Option<PathBuf>,
    ppm: bool,
}

fn options() -> Result<Options> {
    let mut options = Options {
        seed: 42,
        area: MapArea {
            x: 0,
            z: 0,
            width: 512,
            height: 512,
            step: 4,
        },
        maps: MapKind::ALL.to_vec(),
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
            "--maps" => {
                options.maps = value()?
                    .split(',')
                    .map(|n| {
                        MapKind::from_name(n.trim())
                            .with_context(|| format!("carte inconnue « {n} »"))
                    })
                    .collect::<Result<_>>()?;
            }
            "--out" => options.out = Some(value()?.into()),
            "--ppm" => options.ppm = true,
            other => bail!(
                "option inconnue « {other} » (--seed, --x, --z, --size, --step, --maps, --out, --ppm)"
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

fn main() -> Result<()> {
    let options = options()?;
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("target/maps/{}", options.seed)));
    std::fs::create_dir_all(&out)?;
    let generator = TerrainGenerator::new(options.seed);
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
    let samples = MapSamples::compute(&generator, area);
    for kind in &options.maps {
        let raster = samples.render(*kind);
        let path = out.join(format!(
            "{}.{}",
            kind.name(),
            if options.ppm { "ppm" } else { "png" }
        ));
        if options.ppm {
            std::fs::write(&path, raster.to_ppm())?;
        } else {
            write_png(&path, &raster)?;
        }
        println!("  {}", path.display());
    }
    if options.maps.contains(&MapKind::Biomes) {
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
