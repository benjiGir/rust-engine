//! Pack optionnel 32 × 32 : dessins natifs, matières et silhouettes distinctes du classique.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};
mod accessories;
mod detail;
mod draw;
mod fixtures;
mod items;
mod minerals;
mod terrain;
mod wood;

const SIDE: usize = 32;
type Pixel = [u8; 4];
fn hash(x: usize, y: usize, seed: u64) -> u64 {
    let mut n = seed ^ (x as u64).wrapping_mul(0x9e3779b9) ^ (y as u64).wrapping_mul(0x85ebca6b);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb352d);
    n ^ (n >> 15)
}
pub(crate) fn read(path: &Path) -> Result<(usize, Vec<Pixel>)> {
    let mut decoder = png::Decoder::new(BufReader::new(fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().context("taille PNG")?];
    let info = reader.next_frame(&mut buf)?;
    ensure!(
        info.width == info.height && matches!(info.width, 16 | 32 | 64),
        "PNG source attendu en 16, 32 ou 64 : {}",
        path.display()
    );
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].as_chunks::<4>().0.to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => anyhow::bail!("PNG source non RGB : {}", path.display()),
    };
    Ok((info.width as usize, pixels))
}
pub(crate) fn save(path: &Path, side: u32, pixels: &[Pixel]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let mut enc = png::Encoder::new(BufWriter::new(fs::File::create(path)?), side, side);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?
        .write_image_data(&pixels.iter().flatten().copied().collect::<Vec<_>>())?;
    Ok(())
}
pub(crate) fn quantize(c: i32) -> u8 {
    ((c.clamp(0, 255) as f32 / 255.0 * 31.0).round() / 31.0 * 255.0).round() as u8
}
fn linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn mean(pixels: &[Pixel], channel: usize) -> f64 {
    let mut count = 0;
    let mut sum = 0.0;
    for p in pixels.iter().filter(|p| p[3] > 0) {
        sum += linear(p[channel]);
        count += 1;
    }
    sum / count.max(1) as f64
}
/// Le dessin peut changer complètement tout en restant accordé aux teintes de repli
/// et du terrain lointain. On cale sa moyenne linéaire, jamais ses anciens motifs.
fn harmonize(
    canvas: &mut draw::Canvas,
    source: &[Pixel],
    trim: Option<crate::machine_faces::Trim>,
) {
    let in_trim = |index| trim.is_some_and(|trim| trim.contains(index, SIDE));
    let pixels = &mut canvas.0;
    let count = pixels.iter().filter(|p| p[3] > 0).count().max(1) as f64;
    for channel in 0..3 {
        let target = mean(source, channel);
        let mut best = (f64::MAX, 0);
        let (mut low, mut high) = (-255, 255);
        while low <= high {
            let bias = (low + high) / 2;
            let actual = pixels
                .iter()
                .enumerate()
                .filter(|(_, p)| p[3] > 0)
                .map(|(i, p)| {
                    let delta = if in_trim(i) { 0 } else { bias };
                    linear(quantize(p[channel] as i32 + delta))
                })
                .sum::<f64>()
                / count;
            let error = (actual - target).abs();
            if error < best.0 {
                best = (error, bias);
            }
            if actual < target {
                low = bias + 1;
            } else {
                high = bias - 1;
            }
        }
        for (i, p) in pixels.iter_mut().enumerate() {
            if p[3] > 0 && !in_trim(i) {
                p[channel] = quantize(p[channel] as i32 + best.1);
            }
        }
    }
}
fn refine(source: &[Pixel], name: &str, item: bool) -> Vec<Pixel> {
    if !item && name == "torch" {
        return crate::textures::torch_wood(SIDE)
            .into_iter()
            .map(crate::textures::finish_pixel)
            .collect();
    }
    if !item && let Some(pixels) = crate::machine_faces::draw(name, SIDE) {
        // La même palette sur tout le meuble conserve la couleur des jonctions.
        let mut pixels: Vec<_> = pixels
            .into_iter()
            .map(crate::textures::finish_pixel)
            .collect();
        crate::machine_faces::join_trim(name, &mut pixels, SIDE);
        return pixels;
    }
    if !item && let Some(ore) = crate::ore_veins::ORES.iter().find(|ore| ore.name == name) {
        // Fixer la teinte de la roche avant d'ajouter le filon. Une balance globale
        // après composition décalerait aussi les pixels de pierre aux raccords.
        let mut stone = terrain::stone();
        harmonize(&mut stone, &crate::textures::stone_pixels(), None);
        ore.paint(&mut stone.0, SIDE);
        return stone.finish();
    }
    let mut image = if item {
        crate::fauna_items::draw(name, 32)
            .map(draw::Canvas)
            .or_else(|| crate::magic_items::draw(name, 32).map(draw::Canvas))
            .or_else(|| accessories::item(name))
            .or_else(|| minerals::item(name))
            .or_else(|| items::draw(source, name))
    } else {
        crate::creature_skins::draw(name, 32)
            .map(draw::Canvas)
            .or_else(|| crate::doors::draw(name, 32).map(draw::Canvas))
            .or_else(|| terrain::draw(source, name))
            .or_else(|| wood::draw(name))
            .or_else(|| fixtures::draw(name))
            .or_else(|| (name == "rune_table").then(accessories::rune_table))
            .or_else(|| detail::plant(source, name))
    }
    .unwrap_or_else(|| detail::accessory(source, name, item));
    let trim = if item {
        None
    } else {
        crate::machine_faces::join_trim(name, &mut image.0, SIDE)
    };
    harmonize(&mut image, source, trim);
    image.finish()
}

fn refine_core_ground(dir: &Path, name: &str) -> Result<Option<Vec<Pixel>>> {
    let (top, podzol) = match name {
        "grass_side" => ("grass_top", false),
        "podzol_side" => ("podzol_top", true),
        "grass_side_tint" => {
            let mut mask = vec![[0; 4]; SIDE * SIDE];
            crate::textures::grass_side_tint(&mut mask, SIDE);
            return Ok(Some(mask));
        }
        _ => return Ok(None),
    };
    let read_refined = |name| -> Result<_> {
        let (size, pixels) = read(&dir.join(format!("{name}.png")))?;
        Ok(if size == SIDE {
            pixels
        } else {
            refine(&pixels, name, false)
        })
    };
    // Composer avec les couleurs finales des deux matériaux, après harmonisation.
    let mut side = read_refined("dirt")?;
    crate::textures::ground_side(&mut side, &read_refined(top)?, SIDE, podzol);
    Ok(Some(side))
}

pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let pack = root.join("texture_packs/earth-copper-32");
    let mut namespaces = vec![("core".to_string(), root.join("assets/core"))];
    if root.join("mods").is_dir() {
        for entry in fs::read_dir(root.join("mods"))? {
            let p = entry?.path();
            if p.is_dir() {
                namespaces.push((p.file_name().unwrap().to_string_lossy().into_owned(), p));
            }
        }
    }
    namespaces.sort_by(|a, b| a.0.cmp(&b.0));
    let mut paths = Vec::new();
    for (namespace, base) in namespaces {
        for kind in ["textures", "items"] {
            let dir = base.join(kind);
            if !dir.is_dir() {
                continue;
            }
            let mut sources: Vec<_> = fs::read_dir(&dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "png"))
                .collect();
            sources.sort();
            for source in sources {
                let name = source.file_stem().unwrap().to_string_lossy();
                let (size, pixels) = read(&source)?;
                let native = if namespace == "core" && kind == "textures" {
                    if let Some(crop) = crate::texture_pack64::draw_cultivated(&name, SIDE) {
                        let mut image = draw::Canvas(crop);
                        harmonize(&mut image, &pixels, None);
                        Some(image.finish())
                    } else {
                        refine_core_ground(&dir, &name)?
                    }
                } else {
                    None
                };
                let pixels = if let Some(native) = native {
                    native
                } else if size == SIDE {
                    pixels
                } else {
                    refine(&pixels, &name, kind == "items")
                };
                let target = pack
                    .join("assets")
                    .join(&namespace)
                    .join(kind)
                    .join(source.file_name().unwrap());
                save(&target, SIDE as u32, &pixels)?;
                paths.push(target);
            }
        }
    }
    fs::create_dir_all(&pack)?;
    let manifest = pack.join("pack.ron");
    fs::write(
        &manifest,
        "(id: \"earth-copper-32\", name: \"Terre, ardoise et cuivre\", resolution: 32, description: \"Dessins natifs : herbe en touffes, roche éclatée, bois veiné, ferrures et outils forgés.\")\n",
    )?;
    paths.push(manifest);
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_chest_textures_join_across_the_centre_in_both_packs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for pack in ["", "texture_packs/earth-copper-32"] {
            let dir = root.join(pack).join("assets/core/textures");
            for left in
                crate::machine_faces::paired_chest_names().filter(|name| name.contains("_left_"))
            {
                let right = left.replace("_left_", "_right_");
                let (size, left_pixels) = read(&dir.join(format!("{left}.png"))).unwrap();
                let (other_size, right_pixels) = read(&dir.join(format!("{right}.png"))).unwrap();
                assert_eq!(size, other_size);
                assert_ne!(
                    left_pixels, right_pixels,
                    "{left} doit avoir son propre dessin"
                );
                for y in 0..size {
                    assert_eq!(
                        right_pixels[(y + 1) * size - 1],
                        left_pixels[y * size],
                        "{left} {size}, ligne {y}"
                    );
                }
            }
        }
    }

    #[test]
    fn case_edges_and_lid_bands_align_in_both_texture_packs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for pack in ["", "texture_packs/earth-copper-32"] {
            for (folder, top, front, side, back, rim) in [
                (
                    "assets/core/textures",
                    "chest_top",
                    "chest_side",
                    "chest_side_panel",
                    "chest_back_panel",
                    2,
                ),
                (
                    "assets/core/textures",
                    "iron_chest_top",
                    "iron_chest_side",
                    "iron_chest_side_panel",
                    "iron_chest_back_panel",
                    2,
                ),
                (
                    "assets/core/textures",
                    "furnace_top",
                    "furnace_front",
                    "furnace_side_panel",
                    "furnace_back_panel",
                    0,
                ),
            ] {
                let load = |name: &str| {
                    read(&root.join(pack).join(folder).join(format!("{name}.png"))).unwrap()
                };
                let (size, top_pixels) = load(top);
                let (_, front_pixels) = load(front);
                let (_, side_pixels) = load(side);
                let (_, back_pixels) = load(back);
                let scale = size / 16;
                for y in 0..size {
                    let corners = [scale, 14 * scale].map(|x| x + y * size);
                    for index in corners {
                        assert_eq!(
                            front_pixels[index], side_pixels[index],
                            "{front} {size}, angle"
                        );
                        assert_eq!(
                            back_pixels[index], side_pixels[index],
                            "{back} {size}, angle"
                        );
                    }
                }
                let line = rim * scale * size;
                let section = line + rim * scale..line + (16 - rim) * scale;
                assert_eq!(
                    top_pixels[section.clone()],
                    front_pixels[section],
                    "{front} {size}, bord supérieur"
                );
            }
        }
    }

    #[test]
    fn ore_veins_preserve_the_host_stone_and_its_edges_in_both_packs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for directory in [
            "assets/core/textures",
            "texture_packs/earth-copper-32/assets/core/textures",
        ] {
            let dir = root.join(directory);
            let (size, stone) = read(&dir.join("stone.png")).unwrap();
            for ore in &crate::ore_veins::ORES {
                let (ore_size, pixels) = read(&dir.join(format!("{}.png", ore.name))).unwrap();
                assert_eq!(ore_size, size);
                for i in 0..size {
                    for edge in [i, i + size * (size - 1), i * size, i * size + size - 1] {
                        assert_eq!(pixels[edge], stone[edge], "{} {size}, raccord", ore.name);
                    }
                }
                let altered = pixels.iter().zip(&stone).filter(|(a, b)| a != b).count();
                assert!(
                    (size / 2..size * size / 5).contains(&altered),
                    "{} {size}, couverture du gisement : {altered}",
                    ore.name
                );
            }
        }
    }

    #[test]
    fn grass_and_podzol_wrap_over_all_four_edges_in_both_packs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for directory in [
            "assets/core/textures",
            "texture_packs/earth-copper-32/assets/core/textures",
        ] {
            let load =
                |name: &str| read(&root.join(directory).join(format!("{name}.png"))).unwrap();
            let (size, dirt) = load("dirt");
            let (_, mask) = load("grass_side_tint");
            for (name, podzol) in [("grass", false), ("podzol", true)] {
                let (_, top) = load(&format!("{name}_top"));
                let (_, side) = load(&format!("{name}_side"));
                for x in 0..size {
                    for edge in [x, x + size * (size - 1), x * size, x * size + size - 1] {
                        assert_eq!(side[x], top[edge], "{name} {size}, arête du dessus");
                    }
                    let depth = crate::textures::ground_cover_depth(x, size, podzol);
                    for y in 0..size {
                        let i = x + y * size;
                        assert_eq!(side[i], if y < depth { top[i] } else { dirt[i] });
                        assert_eq!(side[y * size], side[y * size + size - 1]);
                        if !podzol {
                            assert_eq!(mask[i][3], if y < depth { 255 } else { 0 });
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn main_materials_change_the_drawing_even_after_removing_fine_grain() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "grass_top",
            "dirt",
            "stone",
            "oak_planks",
            "oak_log",
            "copper_ore",
        ] {
            let (_, source) = read(&root.join(format!("assets/core/textures/{name}.png"))).unwrap();
            let out = refine(&source, name, false);
            // Comparer à résolution commune 8 × 8 : un grain de 1 pixel ajouté à
            // l'ancien dessin ne suffit plus à valider la nouvelle direction artistique.
            let mut difference = 0.0;
            for y in 0..8 {
                for x in 0..8 {
                    for channel in 0..3 {
                        let mut before = 0.0;
                        let mut after = 0.0;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                before +=
                                    source[x * 2 + dx + (y * 2 + dy) * 16][channel] as f64 / 4.0;
                            }
                        }
                        for dy in 0..4 {
                            for dx in 0..4 {
                                after += out[x * 4 + dx + (y * 4 + dy) * 32][channel] as f64 / 16.0;
                            }
                        }
                        difference += (before - after).abs() / (8.0 * 8.0 * 3.0);
                    }
                }
            }
            assert!(
                difference > 3.0,
                "{name} garde trop l'ancien motif : {difference}"
            );
        }
    }

    #[test]
    fn equipment_silhouettes_use_the_native_grid_and_the_furnace_keeps_its_flame_window() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "iron_pickaxe",
            "iron_axe",
            "iron_shovel",
            "iron_sword",
            "iron_chestplate",
        ] {
            let (_, source) = read(&root.join(format!("assets/core/items/{name}.png"))).unwrap();
            let out = refine(&source, name, true);
            let changed_mask = (0..32)
                .flat_map(|y| (0..32).map(move |x| (x, y)))
                .filter(|&(x, y)| (out[x + y * 32][3] > 0) != (source[x / 2 + (y / 2) * 16][3] > 0))
                .count();
            assert!(changed_mask > 20, "{name} n'a pas sa silhouette 32");
            assert_eq!(out[0], [0; 4]);
        }
        let (_, source) = read(&root.join("assets/core/textures/furnace_front.png")).unwrap();
        let out = refine(&source, "furnace_front", false);
        for y in 16..28 {
            for x in 8..24 {
                assert!(out[x + y * 32][..3].iter().all(|&c| c < 90));
            }
        }
    }

    #[test]
    fn real_32_details_keep_periodic_edges_alpha_and_palette() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for name in [
            "grass_top",
            "dirt",
            "stone",
            "sand",
            "snow",
            "ice",
            "oak_log",
            "oak_leaves",
        ] {
            let (_, source) = read(&root.join(format!("assets/core/textures/{name}.png"))).unwrap();
            let out = refine(&source, name, false);
            let mean = |pixels: &[Pixel]| {
                let opaque: Vec<_> = pixels.iter().filter(|p| p[3] > 0).collect();
                std::array::from_fn::<_, 3, _>(|c| {
                    opaque
                        .iter()
                        .map(|p| {
                            let s = p[c] as f64 / 255.0;
                            if s <= 0.04045 {
                                s / 12.92
                            } else {
                                ((s + 0.055) / 1.055).powf(2.4)
                            }
                        })
                        .sum::<f64>()
                        / opaque.len() as f64
                })
            };
            let (before, after) = (mean(&source), mean(&out));
            assert!(
                (0..3).all(|c| (before[c] - after[c]).abs() < 0.02),
                "{name} : la teinte lointaine doit rester cohérente"
            );
            assert_eq!(out.len(), 1024);
            if (0..16).all(|y| source[y * 16] == source[y * 16 + 15]) {
                assert!((0..32).all(|y| out[y * 32] == out[y * 32 + 31]));
            }
            if (0..16).all(|x| source[x] == source[x + 15 * 16]) {
                assert!((0..32).all(|x| out[x] == out[x + 31 * 32]));
            }
            assert!(out.iter().any(|p| p[3] > 0));
            assert!(
                (0..32).any(|y| (0..32).any(|x| out[x + y * 32] != source[x / 2 + (y / 2) * 16])),
                "{name} doit avoir des détails 32, pas seulement un agrandissement"
            );
            assert!(
                out.iter()
                    .all(|p| p[3] == 0 || p[..3].iter().all(|&c| c == quantize(c as i32)))
            );
        }
        let (_, source) = read(&root.join("assets/core/items/iron_pickaxe.png")).unwrap();
        let out = refine(&source, "iron_pickaxe", true);
        assert_eq!(out[0], [0; 4]);
        assert!(out.iter().any(|p| p[3] == 255));
    }

    #[test]
    fn all_versioned_pack_files_are_reproducible_from_the_original_art_generators() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let temporary =
            std::env::temp_dir().join(format!("assetgen-pack32-{}", std::process::id()));
        crate::textures::generate(&temporary).unwrap();
        let generated = generate(&temporary).unwrap();
        assert!(generated.len() > 300);
        for path in generated {
            let relative = path.strip_prefix(&temporary).unwrap();
            assert_eq!(
                fs::read(&path).unwrap(),
                fs::read(root.join(relative)).unwrap(),
                "pack non régénéré : {}",
                relative.display()
            );
            if path.extension().is_some_and(|e| e == "png") {
                let assets = temporary.join("texture_packs/earth-copper-32/assets");
                let mut components = path.strip_prefix(assets).unwrap().components();
                let namespace = components.next().unwrap().as_os_str();
                let original = if namespace == "core" {
                    temporary.join("assets/core")
                } else {
                    temporary.join("mods").join(namespace)
                }
                .join(components.as_path());
                let (_, source) = read(&original).unwrap();
                let (size, packed) = read(&path).unwrap();
                assert_eq!(size, 32);
                for channel in 0..3 {
                    assert!(
                        (mean(&source, channel) - mean(&packed, channel)).abs() < 0.02,
                        "moyenne de repli : {}",
                        relative.display()
                    );
                }
                assert!(
                    packed
                        .iter()
                        .all(|p| p[3] == 0 || p[..3].iter().all(|&c| c == quantize(c as i32)))
                );
            }
        }
        fs::remove_dir_all(temporary).unwrap();
    }
}
