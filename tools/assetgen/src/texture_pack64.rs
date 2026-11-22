//! Pack optionnel 64 × 64. Les PNG sont des surcharges visuelles des ressources du jeu.

use crate::texture_pack32::{quantize, save};
use crate::{machine_faces, magic_items, ore_veins, textures};
use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};

mod canopy;
mod meadow;
mod minerals;
mod terrain;
mod tools;
mod workshop;

const SIDE: usize = 64;
const FOLDER: &str = "texture_packs/earth-copper-64";
type Pixel = [u8; 4];

fn color([r, g, b]: [u8; 3]) -> Pixel {
    [r, g, b, 255]
}

fn shade(c: [u8; 3], factor: f32) -> [u8; 3] {
    c.map(|value| (f32::from(value) * factor).round().clamp(0., 255.) as u8)
}

fn grade([r, g, b, a]: Pixel) -> Pixel {
    [
        quantize(r.into()),
        quantize(g.into()),
        quantize(b.into()),
        a,
    ]
}

struct Pack {
    folder: PathBuf,
    paths: Vec<PathBuf>,
}

impl Pack {
    fn write(&mut self, ns: &str, kind: &str, name: &str, pixels: &[Pixel]) -> Result<()> {
        let path = self
            .folder
            .join("assets")
            .join(ns)
            .join(kind)
            .join(format!("{name}.png"));
        let packed: Vec<_> = pixels.iter().copied().map(grade).collect();
        save(&path, SIDE as u32, &packed)?;
        self.paths.push(path);
        Ok(())
    }
}

// Scale2x conserve les aplats et affine les diagonales, transparence comprise.
fn scale_contours(image: &[Pixel], side: usize) -> Vec<Pixel> {
    let mut result = vec![[0; 4]; 4 * side * side];
    for y in 0..side {
        for x in 0..side {
            let at = |x, y| image[y * side + x];
            let c = at(x, y);
            let (n, s) = (at(x, y.saturating_sub(1)), at(x, (y + 1).min(side - 1)));
            let (w, e) = (at(x.saturating_sub(1), y), at((x + 1).min(side - 1), y));
            for (dy, vertical) in [n, s].into_iter().enumerate() {
                for (dx, horizontal) in [w, e].into_iter().enumerate() {
                    result[(2 * y + dy) * 2 * side + 2 * x + dx] =
                        if n != s && w != e && vertical == horizontal {
                            vertical
                        } else {
                            c
                        };
                }
            }
        }
    }
    result
}

pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    generate_at(root, root.join(FOLDER))
}

fn generate_at(root: &Path, folder: PathBuf) -> Result<Vec<PathBuf>> {
    let mut pack = Pack {
        folder,
        paths: Vec::new(),
    };
    pack.write(
        "core",
        "textures",
        "torch",
        &crate::textures::torch_wood(SIDE),
    )?;
    let rock = terrain::rock("stone");
    pack.write("core", "textures", "stone", &rock)?;
    for ore in ore_veins::ORES {
        let mut pixels = rock.clone();
        minerals::paint(ore.name, &mut pixels);
        pack.write("core", "textures", ore.name, &pixels)?;
    }
    let masonry = terrain::rock("cobblestone");
    pack.write("core", "textures", "deepstone", &terrain::rock("deepstone"))?;
    pack.write("core", "textures", "cobblestone", &masonry)?;
    let dirt = terrain::dirt();
    pack.write("core", "textures", "dirt", &dirt)?;
    for (top, side, podzol) in [
        ("grass_top", "grass_side", false),
        ("podzol_top", "podzol_side", true),
    ] {
        let cover = terrain::turf(podzol);
        pack.write("core", "textures", top, &cover)?;
        let mut wall = dirt.clone();
        textures::ground_side(&mut wall, &cover, SIDE, podzol);
        pack.write("core", "textures", side, &wall)?;
    }
    let mut tint = vec![[0; 4]; SIDE * SIDE];
    textures::grass_side_tint(&mut tint, SIDE);
    pack.write("core", "textures", "grass_side_tint", &tint)?;
    for name in ["sand", "red_sand", "snow", "ice", "gravel"] {
        pack.write("core", "textures", name, &terrain::loose(name))?;
    }
    for species in terrain::WOODS {
        for surface in ["planks", "log", "log_top"] {
            let name = format!("{species}_{surface}");
            pack.write("core", "textures", &name, &terrain::wood(species, surface))?;
        }
        pack.write(
            "core",
            "textures",
            &format!("{species}_leaves"),
            &canopy::leaves(species),
        )?;
    }
    for species in ["blossom", "maple", "fir", "olive"] {
        pack.write(
            "core",
            "textures",
            &format!("{species}_leaves"),
            &canopy::leaves(species),
        )?;
    }
    for name in machine_faces::NAMES
        .iter()
        .map(|name| name.to_string())
        .chain(machine_faces::paired_chest_names())
        .chain(["workbench_top".into(), "workbench_side".into()])
    {
        pack.write("core", "textures", &name, &workshop::draw(&name, &masonry))?;
    }
    for (namespace, faces) in [
        ("forge", machine_faces::FORGE_FACES),
        ("entrepot", &["vault_side", "vault_top"][..]),
    ] {
        for &name in faces {
            pack.write(namespace, "textures", name, &workshop::draw(name, &masonry))?;
        }
    }
    for material in ["wooden", "iron"] {
        for face in ["lower", "upper", "window"] {
            let name = format!("{material}_door_{face}");
            pack.write("core", "textures", &name, &workshop::door(&name))?;
        }
    }
    for tier in tools::MATERIALS {
        for kind in tools::TOOLS {
            let name = format!("{tier}_{kind}");
            pack.write("core", "items", &name, &tools::draw(tier, kind))?;
        }
    }
    let wand = magic_items::draw("wand", SIDE).expect("baguette connue");
    pack.write("magie", "items", "wand", &wand)?;

    let mut bases = vec![("core".to_owned(), root.join("assets/core"))];
    for entry in fs::read_dir(root.join("mods"))? {
        let entry = entry?;
        bases.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
        ));
    }
    for (ns, base) in bases {
        for kind in ["textures", "items"] {
            let dir = base.join(kind);
            if !dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(dir)? {
                let source = entry?.path();
                if source.extension().is_none_or(|ext| ext != "png") {
                    continue;
                }
                let name = source.file_stem().unwrap().to_string_lossy();
                let relative = Path::new(&ns).join(kind).join(source.file_name().unwrap());
                if pack
                    .paths
                    .contains(&pack.folder.join("assets").join(&relative))
                {
                    continue;
                }
                let meadow = (ns == "core" && kind == "textures")
                    .then(|| meadow::draw(&name))
                    .flatten();
                let pixels = if let Some(pixels) = meadow {
                    pixels
                } else {
                    let detailed = root
                        .join("texture_packs/earth-copper-32/assets")
                        .join(relative);
                    let (mut side, mut image) =
                        crate::texture_pack32::read(if detailed.is_file() {
                            &detailed
                        } else {
                            &source
                        })?;
                    while side < SIDE {
                        image = scale_contours(&image, side);
                        side *= 2;
                    }
                    image
                };
                pack.write(&ns, kind, &name, &pixels)?;
            }
        }
    }
    let manifest = pack.folder.join("pack.ron");
    fs::write(
        &manifest,
        "(id: \"earth-copper-64\", name: \"Sous-bois et forge\", resolution: 64, description: \"Roches fracturées, sous-bois moussus, bois vieilli, maçonnerie et métal patinés.\")\n",
    )?;
    pack.paths.push(manifest);
    Ok(pack.paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture_pack32::read;

    fn root() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
    }

    fn pixels(name: &str) -> Vec<Pixel> {
        read(
            &root()
                .join(FOLDER)
                .join("assets/core/textures")
                .join(format!("{name}.png")),
        )
        .unwrap()
        .1
    }

    #[test]
    fn the_versioned_pack_is_native_and_reproducible() {
        let temporary =
            std::env::temp_dir().join(format!("voxel-assetgen-64-{}", std::process::id()));
        let mut generated = generate_at(root(), temporary.clone()).unwrap();
        generated.sort();
        generated.dedup();
        for path in generated {
            let target = root()
                .join(FOLDER)
                .join(path.strip_prefix(&temporary).unwrap());
            assert_eq!(
                fs::read(&path).unwrap(),
                fs::read(&target).unwrap(),
                "{}",
                target.display()
            );
            if path.extension().is_some_and(|ext| ext == "png") {
                let (size, pixels) = read(&path).unwrap();
                assert_eq!(size, SIDE, "{}", target.display());
                assert!(pixels.iter().any(|p| p[3] > 0), "{}", target.display());
                assert!(
                    pixels
                        .iter()
                        .all(|p| p[3] == 0 || p[..3].iter().all(|&c| c == quantize(c.into())))
                );
            }
        }
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn the_new_soils_ores_and_double_chests_preserve_their_joins() {
        let rock = pixels("stone");
        let dirt = pixels("dirt");
        let grass = pixels("grass_top");
        for tile in [&rock, &dirt, &grass] {
            for i in 0..SIDE {
                assert_eq!(tile[i], tile[i + SIDE * (SIDE - 1)]);
                assert_eq!(tile[i * SIDE], tile[i * SIDE + SIDE - 1]);
            }
        }
        for (cover, wall, podzol) in [
            (grass, pixels("grass_side"), false),
            (pixels("podzol_top"), pixels("podzol_side"), true),
        ] {
            for x in 0..SIDE {
                for y in 0..SIDE {
                    let at = x + y * SIDE;
                    let expected = if y < textures::ground_cover_depth(x, SIDE, podzol) {
                        cover[at]
                    } else {
                        dirt[at]
                    };
                    assert_eq!(wall[at], expected);
                }
            }
        }
        for ore in ore_veins::ORES {
            let tile = pixels(ore.name);
            let distinct = tile
                .iter()
                .zip(&rock)
                .filter(|(inclusion, host)| {
                    (0..3).any(|channel| inclusion[channel].abs_diff(host[channel]) >= 44)
                })
                .count();
            assert!(
                distinct >= SIDE * SIDE / 20,
                "{} : seulement {distinct} pixels d'inclusion nettement distincts de la pierre",
                ore.name
            );
            for i in 0..SIDE {
                for edge in [i, i + SIDE * (SIDE - 1), i * SIDE, i * SIDE + SIDE - 1] {
                    assert_eq!(rock[edge], tile[edge], "{}", ore.name);
                }
            }
        }
        for name in machine_faces::paired_chest_names().filter(|name| name.contains("_right_")) {
            let right = pixels(&name);
            let left = pixels(&name.replace("_right_", "_left_"));
            for y in 0..SIDE {
                assert_eq!(right[y * SIDE + SIDE - 1], left[y * SIDE], "{name}");
            }
        }
        let kiln = pixels("furnace_front");
        for y in 32..56 {
            for x in 16..48 {
                let [r, g, b, a] = kiln[x + y * SIDE];
                assert!(r < 42 && g < 42 && b < 42 && a == 255);
            }
        }
        for wood in terrain::WOODS {
            let leaf = pixels(&format!("{wood}_leaves"));
            assert!(leaf.iter().any(|p| p[3] == 255));
            assert!(leaf.iter().any(|p| p[3] == 0));
        }
    }
}
