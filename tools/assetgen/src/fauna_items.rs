//! Butins d'élevage et de pêche, dessinés aux résolutions natives 16 et 32.
use anyhow::Result;
use std::{
    fs,
    io::BufWriter,
    path::{Path, PathBuf},
};
pub const NAMES: &[&str] = &[
    "wool",
    "feather",
    "raw_mutton",
    "cooked_mutton",
    "raw_chicken",
    "cooked_chicken",
    "raw_fish",
    "cooked_fish",
];
fn shade(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| (v as f32 * f).clamp(0.0, 255.0) as u8)
}
fn q(c: u8) -> u8 {
    ((c as f32 / 255.0 * 31.0).round() / 31.0 * 255.0).round() as u8
}
pub fn draw(name: &str, side: usize) -> Option<Vec<[u8; 4]>> {
    let wool = crate::wool_palette::find(name);
    if !NAMES.contains(&name) && wool.is_none() {
        return None;
    }
    let mut pixels = vec![[0; 4]; side * side];
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (
                (x as f32 + 0.5) / side as f32,
                (y as f32 + 0.5) / side as f32,
            );
            let cooked = name.starts_with("cooked");
            let color = if let Some(wool) = wool {
                let edge = (u - 0.5).abs() + (v - 0.5).abs() * 0.55;
                if edge > 0.43 || !(0.20..0.82).contains(&v) {
                    None
                } else {
                    let tuft = ((u * if side == 32 { 9.0 } else { 5.0 }).floor()
                        + (v * if side == 32 { 11.0 } else { 6.0 }).floor())
                    .rem_euclid(3.0);
                    Some(shade(
                        [wool.rgb.0, wool.rgb.1, wool.rgb.2],
                        if tuft == 0.0 {
                            0.88
                        } else if u < 0.4 {
                            1.08
                        } else {
                            0.98
                        },
                    ))
                }
            } else if name == "feather" {
                let along = (u - v + 0.76) * std::f32::consts::FRAC_1_SQRT_2;
                let across = (u + v - 1.0) * std::f32::consts::FRAC_1_SQRT_2;
                if !(0.05..0.94).contains(&along)
                    || across.abs() > (0.17 * (1.0 - (along - 0.48).abs() * 1.6)).max(0.02)
                {
                    None
                } else {
                    Some(shade(
                        [214, 211, 183],
                        if across.abs() < 0.025 {
                            0.66
                        } else if across < 0.0 {
                            1.12
                        } else if ((along + across) * if side == 32 { 24.0 } else { 12.0 }).fract()
                            < 0.18
                        {
                            0.81
                        } else {
                            0.98
                        },
                    ))
                }
            } else if name.ends_with("fish") {
                let body = ((u - 0.54) / 0.29).powi(2) + ((v - 0.49) / 0.16).powi(2) < 1.0;
                let tail = (0.12..0.32).contains(&u) && (v - 0.49).abs() < (0.34 - u) * 0.7;
                let fin = (0.45..0.63).contains(&u) && (0.25..0.36).contains(&v);
                if !body && !tail && !fin {
                    None
                } else {
                    let base = if cooked {
                        [159, 111, 64]
                    } else if v < 0.45 {
                        [89, 133, 114]
                    } else {
                        [176, 188, 161]
                    };
                    let eye = (u - 0.73).abs() < 0.027 && (v - 0.46).abs() < 0.03;
                    let scales = side == 32 && (x + y / 2) % 5 == 0;
                    Some(if eye {
                        [27, 34, 31]
                    } else {
                        shade(
                            base,
                            if scales {
                                1.18
                            } else if tail || fin {
                                0.72
                            } else if v > 0.57 {
                                0.84
                            } else {
                                1.05
                            },
                        )
                    })
                }
            } else if name.ends_with("chicken") {
                let meat = ((u - 0.43) / 0.27).powi(2) + ((v - 0.40) / 0.24).powi(2) < 1.0;
                let bone = (u - v).abs() < 0.06 && (0.52..0.82).contains(&u);
                let joint = ((u - 0.80).powi(2) + (v - 0.76).powi(2) < 0.004)
                    || ((u - 0.75).powi(2) + (v - 0.81).powi(2) < 0.004);
                if meat {
                    Some(shade(
                        if cooked {
                            [177, 112, 59]
                        } else {
                            [214, 171, 139]
                        },
                        if u + v < 0.75 { 1.12 } else { 0.85 },
                    ))
                } else if bone || joint {
                    Some([220, 211, 178])
                } else {
                    None
                }
            } else {
                let body = ((u - 0.49) / 0.31).powi(2) + ((v - 0.47) / 0.26).powi(2) < 1.0;
                if !body {
                    None
                } else {
                    let bone = ((u - 0.56).powi(2) + (v - 0.49).powi(2)) < 0.006;
                    Some(if bone {
                        [221, 202, 166]
                    } else {
                        shade(
                            if cooked { [142, 87, 53] } else { [183, 96, 91] },
                            if u + v < 0.75 {
                                1.16
                            } else if side == 32 && (x + y) % 11 == 0 {
                                0.75
                            } else {
                                0.91
                            },
                        )
                    })
                }
            };
            if let Some(color) = color {
                let [r, g, b] = color.map(q);
                pixels[x + y * side] = [r, g, b, 255];
            }
        }
    }
    let source = pixels.clone();
    for y in 1..side - 1 {
        for x in 1..side - 1 {
            if source[x + y * side][3] == 0
                && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                    .iter()
                    .any(|&(a, b)| source[a + b * side][3] > 0)
            {
                pixels[x + y * side] = [q(36), q(32), q(29), 255];
            }
        }
    }
    Some(pixels)
}
pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join("assets/core/items");
    fs::create_dir_all(&dir)?;
    let mut paths = Vec::new();
    let names = NAMES.iter().map(|s| s.to_string()).chain(
        crate::wool_palette::colors()
            .iter()
            .filter(|c| c.item != "core:wool")
            .map(|c| c.item.strip_prefix("core:").unwrap().to_string()),
    );
    for name in names {
        let path = dir.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(BufWriter::new(fs::File::create(&path)?), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(
            &draw(&name, 16)
                .unwrap()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )?;
        paths.push(path);
    }
    Ok(paths)
}
