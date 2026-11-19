//! Exporte des cartes d'une graine sans ouvrir de fenêtre : `cargo xtask map` (qui appelle cet
//! exemple en release). Voir `engine_voxel::MapSamples` pour ce que montre chaque carte.
//!
//! ```text
//! cargo xtask map                                   # graine 42, 512 × 512 pixels à 4 blocs, toutes les cartes
//! cargo xtask map --seed 7 --x -2048 --z -2048 --size 1024 --step 4
//! cargo xtask map --maps altitude,biomes --out target/maps/essai --ppm
//! cargo xtask map --generator v2 --size 1024 --step 8 --maps terre-mer,drainage,bassins
//! cargo xtask map --generator v2 --size 512 --step 16 --maps biomes-v2,humidite-v2,temperature-v2,canopee,ombre-pluie
//! cargo xtask map --generator v2 --size 256 --step 4 --maps hauteur-blocs,materiau,eau-blocs,surplombs
//! cargo xtask map --generator v2 --section z=0 --from -256 --to 256 --ymin 40 --ymax 192 --scale 3
//! ```
//!
//! Options : `--generator v1|v2` (v1 : le monde réel ; v2 : le relief macro des lots 2 et 3, cartes
//! `terre-mer`, `rivage`, `altitude-macro`, `altitude`, `chaines`, `familles`, `bassins`, `drainage`, `lacs`,
//! et, issues des **blocs** de la génération v2 (lot 4), `hauteur-blocs`, `materiau`, `eau-blocs`, `parois`,
//! `surplombs`, et le climat et les biomes de la génération v2 (lot 5) : `ombre-pluie`, `maritime`,
//! `humidite-regionale` (géographie seule, peu coûteuses) et `biomes-v2`, `temperature-v2`, `humidite-v2`,
//! `canopee` (lues dans les colonnes du générateur ; la légende de `biomes-v2` est écrite dans
//! `biomes-v2.txt`) ; `--section x=N|z=N --from A --to B [--ymin 0 --ymax 192 --scale 2]` dessine la coupe
//! verticale d'une ligne, un bloc par `scale` pixels, dans `coupe-<axe><N>.png`), `--seed N` (42), `--x X --z Z` (coin de la zone, en blocs ; 0 0), `--size L[xH]`
//! (pixels ; 512), `--step B` (blocs par pixel ; 4), `--maps a,b` (altitude, pente, humidite,
//! temperature, continentalite, biomes ; toutes), `--out dossier` (target/maps/<graine>), `--ppm`
//! (du PPM au lieu du PNG), `--relief` (cartes et coupes de blocs du relief seul : sans cavernes, arbres ni structures). Les fichiers sont nommés `<carte>.png` ; la légende des biomes est
//! affichée et écrite dans `biomes.txt`.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use engine_voxel::{
    BlockMapKind, BlockMapSamples, ClimateMapKind, ClimateMapSamples, GenerationVersion,
    MacroMapKind, MacroMapSamples, MacroWorld, MapArea, MapKind, MapSamples, Raster, SectionAxis,
    SectionSpec, TerrainGenerator, render_section,
};

/// Le générateur dont on dessine les cartes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Generator {
    /// La génération historique (v1) : les cartes du monde réel.
    V1,
    /// Le prototype macro v2 (lots 2 et 3) : géographie et drainage, sans blocs.
    V2,
    /// La génération v3 (densité 3D) : les cartes du monde réel et les cartes de blocs.
    V3,
}

struct Options {
    generator: Generator,
    seed: u32,
    area: MapArea,
    /// Les noms de cartes demandés ; `None` : toutes celles du générateur.
    maps: Option<Vec<String>>,
    out: Option<PathBuf>,
    ppm: bool,
    /// `--relief` : le relief seul (ni cavernes, ni arbres, ni structures).
    relief: bool,
    /// `--section x=N|z=N` : la coupe verticale demandée.
    section: Option<(SectionAxis, i32)>,
    from: i32,
    to: i32,
    y_min: i32,
    y_max: i32,
    scale: u32,
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
        relief: false,
        section: None,
        from: -128,
        to: 128,
        y_min: 0,
        y_max: 192,
        scale: 2,
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
                    "v3" => Generator::V3,
                    other => bail!("--generator : v1 ou v2, pas « {other} »"),
                };
            }
            "--maps" => {
                options.maps = Some(value()?.split(',').map(|n| n.trim().to_string()).collect());
            }
            "--out" => options.out = Some(value()?.into()),
            "--ppm" => options.ppm = true,
            "--relief" => options.relief = true,
            "--section" => {
                let text = value()?;
                let (axis, n) = text.split_once('=').context("--section : x=N ou z=N")?;
                let axis = match axis {
                    // `x=N` : le plan x = N (le profil suit z) ; `z=N` : le plan z = N (il suit x).
                    "x" => SectionAxis::Z,
                    "z" => SectionAxis::X,
                    other => bail!("--section : x=N ou z=N, pas « {other} »"),
                };
                options.section = Some((axis, n.parse().context("--section : un entier")?));
            }
            "--from" => options.from = value()?.parse().context("--from : un entier")?,
            "--to" => options.to = value()?.parse().context("--to : un entier")?,
            "--ymin" => options.y_min = value()?.parse().context("--ymin : un entier")?,
            "--ymax" => options.y_max = value()?.parse().context("--ymax : un entier")?,
            "--scale" => options.scale = value()?.parse().context("--scale : un entier")?,
            other => bail!(
                "option inconnue « {other} » (--generator, --seed, --x, --z, --size, --step, --maps, --out, --ppm, --relief, --section, --from, --to, --ymin, --ymax, --scale)"
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

/// Le générateur v2 de la graine demandée (le relief seul avec `--relief`).
fn v2_generator(options: &Options) -> Result<TerrainGenerator> {
    let generator = TerrainGenerator::new(options.seed).with_version(GenerationVersion::V2)?;
    Ok(if options.relief {
        generator.relief_only()
    } else {
        generator
    })
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
    if options.generator != Generator::V1
        && let Some((axis, fixed)) = options.section
    {
        let generator = if options.generator == Generator::V3 {
            TerrainGenerator::new(options.seed).with_version(GenerationVersion::V3)?
        } else {
            v2_generator(&options)?
        };
        if options.to <= options.from || options.to - options.from > 4096 {
            bail!("--from < --to, sur 4096 blocs au plus");
        }
        let started = std::time::Instant::now();
        let raster = render_section(
            &generator,
            &SectionSpec {
                axis,
                fixed,
                from: options.from,
                to: options.to,
                y_min: options.y_min,
                y_max: options.y_max,
                scale: options.scale.clamp(1, 8),
            },
        );
        let name = match axis {
            SectionAxis::X => format!("coupe-z{fixed}"),
            SectionAxis::Z => format!("coupe-x{fixed}"),
        };
        println!(
            "graine {}, coupe {name} de {} à {} : {} ms",
            options.seed,
            options.from,
            options.to,
            started.elapsed().as_millis()
        );
        save(&out, &name, &raster, options.ppm)?;
        return Ok(());
    }
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
        // Les cartes de blocs (génération v2) et les cartes macro se demandent par leur nom.
        let (block_names, other_names): (Vec<&String>, Vec<&String>) = options
            .maps
            .iter()
            .flatten()
            .partition(|n| BlockMapKind::from_name(n).is_some());
        let (climate_names, macro_names): (Vec<&String>, Vec<&String>) = other_names
            .into_iter()
            .partition(|n| ClimateMapKind::from_name(n).is_some());
        if !climate_names.is_empty() {
            let kinds: Vec<ClimateMapKind> = climate_names
                .iter()
                .map(|n| ClimateMapKind::from_name(n).expect("nom vérifié par la partition"))
                .collect();
            let world = MacroWorld::new(options.seed);
            let generator = kinds
                .iter()
                .any(|k| k.needs_columns())
                .then(|| v2_generator(&options))
                .transpose()?;
            let started = std::time::Instant::now();
            let samples = ClimateMapSamples::compute(&world, generator.as_ref(), area);
            println!("climat v2 : {} ms", started.elapsed().as_millis());
            for kind in kinds {
                save(&out, kind.name(), &samples.render(kind), options.ppm)?;
                if kind == ClimateMapKind::Biomes {
                    let legend: String = samples
                        .biome_legend()
                        .iter()
                        .map(|(key, c)| format!("{key}\t#{:02x}{:02x}{:02x}\n", c[0], c[1], c[2]))
                        .collect();
                    std::fs::write(out.join("biomes-v2.txt"), legend)?;
                }
            }
            if block_names.is_empty() && macro_names.is_empty() {
                return Ok(());
            }
        }
        if !block_names.is_empty() {
            let generator = v2_generator(&options)?;
            let started = std::time::Instant::now();
            let samples = BlockMapSamples::compute(&generator, area);
            println!(
                "générateur v2 (blocs) : {} ms",
                started.elapsed().as_millis()
            );
            for name in block_names {
                let kind = BlockMapKind::from_name(name).expect("nom vérifié par la partition");
                save(&out, kind.name(), &samples.render(kind), options.ppm)?;
            }
            if macro_names.is_empty() {
                return Ok(());
            }
        }
        let kinds: Vec<MacroMapKind> = match &options.maps {
            None => MacroMapKind::ALL.to_vec(),
            Some(_) => macro_names
                .iter()
                .map(|n| {
                    MacroMapKind::from_name(n).with_context(|| {
                        format!(
                            "carte v2 inconnue « {n} » ({}, {})",
                            MacroMapKind::ALL.map(|k| k.name()).join(", "),
                            BlockMapKind::ALL.map(|k| k.name()).join(", ")
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
    // En v3, les cartes de blocs (`hauteur-blocs`, `materiau`, `eau-blocs`...) se demandent aussi.
    let (block_names, map_names): (Vec<String>, Vec<String>) =
        options.maps.iter().flatten().cloned().partition(|n| {
            options.generator == Generator::V3 && BlockMapKind::from_name(n).is_some()
        });
    if !block_names.is_empty() {
        let generator = TerrainGenerator::new(options.seed).with_version(GenerationVersion::V3)?;
        let started = std::time::Instant::now();
        let samples = BlockMapSamples::compute(&generator, area);
        println!(
            "générateur v3 (blocs) : {} ms",
            started.elapsed().as_millis()
        );
        for name in &block_names {
            let kind = BlockMapKind::from_name(name).expect("nom vérifié par la partition");
            save(&out, kind.name(), &samples.render(kind), options.ppm)?;
        }
        if map_names.is_empty() {
            return Ok(());
        }
    }
    let kinds: Vec<MapKind> = match &options.maps {
        None => MapKind::ALL.to_vec(),
        Some(_) => map_names
            .iter()
            .map(|n| MapKind::from_name(n).with_context(|| format!("carte inconnue « {n} »")))
            .collect::<Result<_>>()?,
    };
    let generator = if options.generator == Generator::V3 {
        TerrainGenerator::new(options.seed).with_version(GenerationVersion::V3)?
    } else {
        TerrainGenerator::new(options.seed)
    };
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
