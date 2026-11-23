//! Matières originales des créatures, dessinées à la résolution 16 ou 32 du pack.
use anyhow::Result;
use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};

const SKINS: &[(&str, &str)] = &[
    ("core", "mob_pig_skin"),
    ("core", "mob_sheep_wool"),
    ("core", "mob_sheep_face"),
    ("core", "mob_alpaca_wool"),
    ("core", "mob_alpaca_fur"),
    ("core", "mob_chicken_feathers"),
    ("core", "mob_fish_scales"),
    ("core", "mob_fish_fins"),
    ("core", "mob_cow_hide"),
    ("core", "mob_cow_head"),
    ("core", "mob_cow_leg"),
    ("core", "mob_cow_highland_hide"),
    ("core", "mob_cow_highland_head"),
    ("core", "mob_cow_zebu_hide"),
    ("core", "mob_cow_zebu_head"),
    ("core", "mob_hoof"),
    ("core", "mob_horn"),
    ("core", "mob_prowler_cloth"),
    ("core", "mob_prowler_leather"),
    ("core", "mob_spore_cap"),
    ("core", "mob_spore_gills"),
    ("core", "mob_spore_stem"),
    ("example", "mob_slime_gel"),
    ("example", "mob_guard_plate"),
    ("example", "mob_guard_leather"),
    ("donjons", "mob_warden_stone"),
    ("donjons", "mob_warden_plate"),
];

fn hash(x: i32, y: i32, seed: u64) -> u64 {
    let mut n = seed ^ (x as u64).wrapping_mul(0x9e3779b9) ^ (y as u64).wrapping_mul(0x85ebca6b);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb352d);
    n ^ (n >> 15)
}
fn noise(u: f32, v: f32, cells: i32, seed: u64) -> f32 {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let at = |dx: i32, dy: i32| {
        (hash(
            (ix + dx).rem_euclid(cells),
            (iy + dy).rem_euclid(cells),
            seed,
        ) % 1000) as f32
            / 999.0
    };
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (a, b) = (ease(x.fract()), ease(y.fract()));
    (at(0, 0) * (1.0 - a) + at(1, 0) * a) * (1.0 - b) + (at(0, 1) * (1.0 - a) + at(1, 1) * a) * b
}
fn shade(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| (v as f32 * f).clamp(0.0, 255.0) as u8)
}
fn quantize(c: u8) -> u8 {
    ((c as f32 / 255.0 * 31.0).round() / 31.0 * 255.0).round() as u8
}

pub fn draw(name: &str, side: usize) -> Option<Vec<[u8; 4]>> {
    if !SKINS.iter().any(|(_, n)| *n == name) {
        return None;
    }
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
            let broad = noise(u, v, 5, 3301);
            let fine = noise(u, v, if side == 32 { 17 } else { 9 }, 3307);
            let color = match name {
                "mob_sheep_wool" => {
                    let curls = noise(u, v, if side == 32 { 12 } else { 7 }, 3401);
                    shade([213, 207, 181], 0.89 + broad * 0.10 + curls * 0.13)
                }
                "mob_alpaca_wool" => {
                    // Une laine fine et dense, aux mèches ondulées, dessinée en blanc pour être
                    // teintée par la robe de l'animal.
                    let crimp = noise(u * 1.4, v * 0.8, if side == 32 { 15 } else { 8 }, 3421);
                    let wave = ((u * 6.0 + v * 1.5 + broad) * std::f32::consts::TAU).sin();
                    shade(
                        [216, 210, 188],
                        0.90 + broad * 0.07 + crimp * 0.11 + wave * 0.025,
                    )
                }
                "mob_alpaca_fur" => {
                    // Un poil court et lisse, un peu plus clair, en fines stries verticales.
                    let strand = noise(u * 3.0, v * 0.5, if side == 32 { 18 } else { 9 }, 3431);
                    shade(
                        [222, 216, 197],
                        0.93 + broad * 0.05 + strand * 0.07 - v * 0.03,
                    )
                }
                "mob_sheep_face" => shade([126, 112, 88], 0.94 + fine * 0.08),
                "mob_chicken_feathers" => {
                    let vane = ((u * 7.0 + (v * 7.0).floor() * 0.5).fract() - 0.5).abs();
                    shade(
                        [224, 216, 183],
                        0.95 + broad * 0.04
                            + if side == 32 && vane < 0.1 {
                                -0.06
                            } else {
                                fine * 0.035
                            },
                    )
                }
                "mob_fish_scales" => {
                    let scale =
                        (u * if side == 32 { 11.0 } else { 6.0 } + (v * 8.0).floor() * 0.5).fract();
                    let base = if v < 0.45 {
                        [94, 133, 111]
                    } else {
                        [164, 182, 153]
                    };
                    shade(
                        base,
                        if scale < 0.16 {
                            0.82
                        } else if scale < 0.33 {
                            1.12
                        } else {
                            0.96 + broad * 0.07
                        },
                    )
                }
                "mob_fish_fins" => shade(
                    [148, 126, 77],
                    if (u * if side == 32 { 13.0 } else { 7.0 }).fract() < 0.22 {
                        0.7
                    } else {
                        1.0 + fine * 0.08
                    },
                ),
                "mob_pig_skin" => shade(
                    [203, 144, 126],
                    0.95 + broad * 0.055
                        + fine * 0.035
                        + if side == 32 && hash(x as i32, y as i32, 3309).is_multiple_of(71) {
                            -0.045
                        } else {
                            0.0
                        },
                ),
                "mob_cow_leg" => shade(
                    [196, 181, 143],
                    0.95 + broad * 0.05 + fine * 0.045 - v * 0.07,
                ),
                "mob_cow_hide" | "mob_cow_head" => {
                    let n = noise(u, v, 4, if name.ends_with("head") { 3319 } else { 3313 });
                    let patch = n > if name.ends_with("head") { 0.61 } else { 0.53 };
                    let base = if patch { [83, 59, 42] } else { [203, 191, 158] };
                    let hair = if side == 32 && x % 3 == 0 && y % 5 < 2 {
                        0.06
                    } else {
                        0.0
                    };
                    shade(base, 0.91 + fine * 0.14 + hair)
                }
                "mob_cow_highland_hide" | "mob_cow_highland_head" => {
                    // De longues mèches rousses qui tombent, plus courtes sur la tête.
                    let head = name.ends_with("head");
                    let strands = noise(
                        u * if head { 1.0 } else { 1.6 },
                        v * if head { 0.5 } else { 0.25 },
                        if side == 32 { 14 } else { 8 },
                        3361,
                    );
                    let wave = ((u * 9.0 + v * 2.5 + broad * 1.5) * std::f32::consts::TAU).sin();
                    shade(
                        if head { [181, 104, 56] } else { [170, 92, 47] },
                        0.80 + strands * 0.30 + wave * 0.05 + broad * 0.08,
                    )
                }
                "mob_cow_zebu_hide" | "mob_cow_zebu_head" => {
                    // Une robe claire, rase, à peine nuancée de gris.
                    let base = if name.ends_with("head") {
                        [168, 165, 158]
                    } else {
                        [214, 209, 196]
                    };
                    let grey = noise(u, v, 3, 3371);
                    shade(base, 0.92 + broad * 0.06 + fine * 0.05 - grey * 0.08)
                }
                "mob_hoof" => {
                    let cleft = (u - 0.5).abs() < 0.06;
                    shade(
                        [66, 55, 46],
                        if cleft {
                            0.52
                        } else {
                            0.80 + fine * 0.25 + (1.0 - v) * 0.2
                        },
                    )
                }
                "mob_horn" => shade(
                    [207, 191, 145],
                    0.83 + fine * 0.1 + if (v * 7.0).fract() < 0.2 { -0.07 } else { 0.12 },
                ),
                "mob_prowler_cloth" => {
                    let fold = (u * 4.0 * std::f32::consts::TAU + broad * 0.7).sin();
                    let stitch = side == 32 && (x == 4 || x == 27) && y % 6 < 2;
                    shade(
                        [75, 95, 98],
                        if stitch {
                            1.15
                        } else {
                            0.94 + fold * 0.09 + fine * 0.06
                        },
                    )
                }
                "mob_prowler_leather" | "mob_guard_leather" => {
                    let seam = u < 0.08 || u > 0.92 || v < 0.1;
                    shade(
                        [93, 70, 53],
                        if seam {
                            0.65
                        } else {
                            0.86 + broad * 0.25 + fine * 0.1
                        },
                    )
                }
                "mob_spore_cap" => {
                    let spots = noise(u, v, 6, 3331);
                    let base = if spots > 0.69 {
                        [212, 194, 149]
                    } else if spots > 0.61 {
                        [169, 119, 123]
                    } else {
                        [126, 58, 101]
                    };
                    shade(base, 0.85 + broad * 0.28 + fine * 0.08)
                }
                "mob_spore_gills" => {
                    let (dx, dy) = (u - 0.5, v - 0.5);
                    let rib = (dy.atan2(dx) * if side == 32 { 23.0 } else { 13.0 }).sin();
                    shade([169, 149, 113], if rib < 0.0 { 0.70 } else { 1.12 })
                }
                "mob_spore_stem" => {
                    let fibre = noise(u, v * 0.3, 11, 3341);
                    shade([198, 185, 140], 0.85 + fibre * 0.24 + broad * 0.07)
                }
                "mob_slime_gel" => {
                    let bubble = noise(u, v, if side == 32 { 11 } else { 7 }, 3351);
                    let edge = ((u - 0.28).powi(2) + (v - 0.25).powi(2)).sqrt();
                    shade(
                        [91, 155, 76],
                        0.88 + broad * 0.20
                            + if bubble > 0.75 {
                                0.25
                            } else if edge < 0.13 {
                                0.38
                            } else {
                                0.0
                            },
                    )
                }
                "mob_guard_plate" | "mob_warden_plate" => {
                    let base = if name.contains("warden") {
                        [96, 90, 110]
                    } else {
                        [120, 138, 144]
                    };
                    let edge = u < 0.06 || v < 0.07 || u > 0.94 || v > 0.93;
                    let scratch = side == 32 && (x + y / 2) % 23 == 0 && y % 9 < 3;
                    shade(
                        base,
                        if edge {
                            0.58
                        } else if scratch {
                            1.32
                        } else {
                            0.92 + broad * 0.13 + fine * 0.06
                        },
                    )
                }
                "mob_warden_stone" => {
                    let a = (u - (0.23 + (v * std::f32::consts::TAU).sin() * 0.12)).abs();
                    let b = (v - (0.73 + (u * std::f32::consts::TAU).sin() * 0.08)).abs();
                    let edge = a.min(b);
                    shade(
                        [136, 133, 147],
                        if edge < 0.021 {
                            0.61
                        } else if edge < 0.044 {
                            1.12
                        } else {
                            0.93 + broad * 0.12 + fine * 0.05
                        },
                    )
                }
                _ => unreachable!(),
            };
            let [r, g, b] = color.map(quantize);
            pixels.push([r, g, b, 255]);
        }
    }
    Some(pixels)
}

pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for &(namespace, name) in SKINS {
        let base = if namespace == "core" {
            root.join("assets/core")
        } else {
            root.join("mods").join(namespace)
        };
        let path = base.join("textures").join(format!("{name}.png"));
        fs::create_dir_all(path.parent().unwrap())?;
        let mut encoder = png::Encoder::new(BufWriter::new(fs::File::create(&path)?), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(
            &draw(name, 16)
                .unwrap()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )?;
        paths.push(path);
    }
    Ok(paths)
}
