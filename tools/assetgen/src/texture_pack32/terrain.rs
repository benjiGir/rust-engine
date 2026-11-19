//! Sols, roches et végétation redessinés en 32 : volumes, inclusions et brins lisibles.
use super::draw::{Canvas, INK, mix, noise, rgb, seed, shade};
use super::{Pixel, hash};

fn earth(name: &str) -> Canvas {
    let mut c = Canvas::new([111, 79, 53]);
    let wet = name.contains("wet");
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32 / 31.0, (y % 31) as f32 / 31.0);
            let n = noise(u, v, 5, 710);
            let clod = noise(u, v, 11, 713);
            let mut color = shade([114, 81, 55], 0.78 + n * 0.30 + clod * 0.17);
            // Petits cailloux aplatis, de plusieurs pixels, dans un limon groupé.
            let (a, b) = (x % 31, y % 31);
            for (i, (cx, cy)) in [(4, 6), (19, 10), (12, 24), (28, 22), (2, 29)]
                .iter()
                .enumerate()
            {
                let (dx, dy) = ((a - cx) as f32, (b - cy) as f32);
                if dx * dx * 0.48 + dy * dy < 4.8 {
                    color = shade([132, 117, 91], if dy < 0.0 { 1.22 } else { 0.78 });
                    if i % 2 == 0 {
                        color = mix(color, [108, 80, 52], 0.35);
                    }
                }
            }
            if name.starts_with("farmland") {
                let lane = (x % 8) as f32;
                color = shade(
                    color,
                    [0.55, 0.68, 0.9, 1.22, 1.24, 1.12, 1.0, 0.78][lane as usize],
                );
                if wet {
                    color = shade(color, 0.72);
                }
            }
            c.put(x, y, color);
        }
    }
    // Brins de racines fins, décalés par rapport aux cailloux.
    for (x, y) in [(8, 12), (23, 27), (20, 2)] {
        c.line((x, y), (x + 3, y + 1), [84, 61, 42]);
        c.line((x + 2, y + 1), (x + 3, y + 3), [143, 103, 67]);
    }
    c.periodic(true);
    c
}

fn turf() -> Canvas {
    let mut c = Canvas::new([95, 132, 59]);
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32 / 31.0, (y % 31) as f32 / 31.0);
            c.put(
                x,
                y,
                shade([99, 138, 63], 0.72 + noise(u, v, 5, 720) * 0.42),
            );
        }
    }
    // Touffes de 3 à 7 pixels : les brins ont une ombre et une face éclairée.
    for i in 0..48 {
        let h = hash(i, 0, 725);
        let (cx, cy) = ((h % 31) as i32, ((h >> 8) % 31) as i32);
        let len = 2 + ((h >> 16) % 4) as i32;
        let color = if i % 3 == 0 {
            [133, 160, 75]
        } else {
            [105, 145, 63]
        };
        for t in 0..len {
            let (x, y) = ((cx + t / 3).rem_euclid(31), (cy - t).rem_euclid(31));
            c.put(x, y, shade(color, 1.05 + t as f32 * 0.035));
            c.put((x + 1) % 31, y, shade(color, 0.72));
            if t == 1 {
                c.put((x + 30) % 31, y, shade(color, 0.92));
            }
        }
    }
    c.periodic(true);
    c
}

/// Distance sur le tore : un éclat traversant le bord se poursuit sur le bloc voisin.
fn delta(a: f32, b: f32) -> f32 {
    (a - b + 15.5).rem_euclid(31.0) - 15.5
}

pub(super) fn rock(name: &str) -> Canvas {
    let base = match name {
        "deepstone" | "bedrock" => [56, 64, 70],
        "volcanic_rock" => [65, 60, 65],
        "sandstone" | "sandstone_top" => [187, 163, 114],
        "crypt_brick" => [92, 103, 104],
        _ => [119, 132, 135],
    };
    let seed = seed(name);
    let mut c = Canvas::new(base);
    let cobble = name.contains("cobble") || name == "crypt_brick";
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32, (y % 31) as f32);
            let mut nearest = (f32::MAX, 0);
            let mut second = f32::MAX;
            for i in 0..if cobble { 12 } else { 8 } {
                let h = hash(i, 0, seed);
                let (cx, cy) = ((h % 310) as f32 / 10.0, ((h >> 12) % 310) as f32 / 10.0);
                let (dx, dy) = (delta(u, cx), delta(v, cy));
                let dist = dx.abs() * 0.82 + dy.abs() * 1.18 + (dx + dy * 0.4).abs() * 0.25;
                if dist < nearest.0 {
                    second = nearest.0;
                    nearest = (dist, i);
                } else {
                    second = second.min(dist);
                }
            }
            let ridge = second - nearest.0;
            let value = (hash(nearest.1, 1, seed) % 100) as f32 / 100.0;
            let grain = noise(u / 31.0, v / 31.0, 12, seed + 2);
            let factor = if ridge < 0.65 {
                if cobble { 0.48 } else { 0.70 }
            } else if ridge < 1.35 {
                1.17
            } else {
                0.82 + value * 0.26 + grain * 0.12
            };
            let mut color = shade(base, factor);
            // Petites strates interrompues, sans encadrement de chaque tuile.
            if !cobble && (v + u * 0.35 + grain * 1.5).rem_euclid(10.0) < 0.6 {
                color = shade(color, 0.87);
            }
            c.put(x, y, color);
        }
    }
    c.periodic(true);
    c
}

fn ore(name: &str) -> Canvas {
    let mut c = rock("stone");
    let metal = match name {
        "coal_ore" => [47, 51, 54],
        "copper_ore" => [185, 116, 70],
        "iron_ore" => [181, 161, 135],
        "gold_ore" => [212, 174, 76],
        "sapphire_ore" => [74, 138, 189],
        "jade_ore" | "malachite_ore" => [75, 160, 110],
        "cinnabar_ore" => [186, 74, 64],
        _ => [136, 109, 160],
    };
    for i in 0..7 {
        let h = hash(i, 0, seed(name));
        let (cx, cy) = ((h % 29) as i32, ((h >> 10) % 29) as i32);
        for dy in -3i32..=3 {
            for dx in -4i32..=4 {
                if dx.abs() + dy.abs() * 2 > 5 + (i % 2) as i32 {
                    continue;
                }
                let (x, y) = ((cx + dx).rem_euclid(31), (cy + dy + dx / 3).rem_euclid(31));
                let edge = dx.abs() + dy.abs() * 2;
                let factor = if edge >= 5 {
                    0.46
                } else if dy < 0 {
                    1.28
                } else if dx > 0 {
                    0.76
                } else {
                    1.0
                };
                c.put(x, y, shade(metal, factor));
                if name == "copper_ore" && i % 3 == 0 && dy > 0 {
                    c.put(x, y, [67, 119, 103]);
                }
            }
        }
    }
    c.periodic(true);
    c
}

fn granular(name: &str) -> Canvas {
    let base = match name {
        "sand" => [207, 186, 139],
        "red_sand" => [180, 112, 72],
        "snow" => [224, 236, 237],
        "ice" => [138, 184, 199],
        "gravel" => [126, 123, 119],
        "clay" => [149, 155, 155],
        "terracotta_pale" => [191, 164, 132],
        "terracotta_dark" => [111, 72, 64],
        _ => [168, 108, 79],
    };
    let mut c = Canvas::new(base);
    let seed = seed(name);
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32 / 31.0, (y % 31) as f32 / 31.0);
            let n = noise(u, v, 5, seed);
            let ripple =
                (v * 4.0 * std::f32::consts::TAU + n * 3.0 + u * std::f32::consts::TAU).sin();
            let factor = if name == "ice" {
                let line = (u * 3.0 + v * 2.0 + n * 0.18).fract();
                if line < 0.035 {
                    1.30
                } else if line < 0.07 {
                    0.82
                } else {
                    0.91 + n * 0.18
                }
            } else if name == "snow" {
                0.96 + ripple * 0.025 + n * 0.05
            } else {
                0.92 + ripple * 0.045 + noise(u, v, 15, seed + 2) * 0.13
            };
            c.put(x, y, shade(base, factor));
        }
    }
    if name == "sand" || name == "red_sand" || name == "gravel" {
        for i in 0..if name == "gravel" { 65 } else { 24 } {
            let h = hash(i, 2, seed);
            let (x, y) = ((h % 30) as i32, ((h >> 9) % 30) as i32);
            c.put(x, y, shade(base, 0.75));
            c.put((x + 1) % 31, y, shade(base, 1.18));
            if name == "gravel" {
                c.put(x, (y + 1) % 31, shade(base, 0.9));
            }
        }
    }
    c.periodic(true);
    c
}

fn leaves(source: &[Pixel], name: &str) -> Canvas {
    let base = source.iter().find(|p| p[3] > 0).map(|p| rgb(*p)).unwrap();
    let mut c = Canvas::new(shade(base, 0.65));
    for i in 0..38 {
        let h = hash(i, 0, seed(name));
        let (cx, cy) = ((h % 31) as i32, ((h >> 10) % 31) as i32);
        for dy in -3i32..=3 {
            for dx in -4i32..=4 {
                if dx.abs() + (dy + dx / 2).abs() > 4 {
                    continue;
                }
                let x = (cx + dx).rem_euclid(31);
                let y = (cy + dy).rem_euclid(31);
                let f = if dy == -dx / 2 {
                    1.3
                } else if dy < 0 {
                    1.12
                } else {
                    0.85
                };
                c.put(x, y, shade(base, f + (i % 4) as f32 * 0.06));
            }
        }
    }
    // Petites découpes entre les bouquets, conservant le rendu découpé du feuillage.
    for i in 0..10 {
        let h = hash(i, 5, seed(name));
        let (x, y) = ((h % 31) as usize, ((h >> 8) % 31) as usize);
        c.0[x + y * 32] = [0; 4];
        c.0[(x + 1) % 31 + y * 32] = [0; 4];
    }
    c.periodic(true);
    c
}

pub(super) fn draw(source: &[Pixel], name: &str) -> Option<Canvas> {
    Some(match name {
        "grass_top" => turf(),
        "water" => {
            let mut canvas = Canvas::transparent();
            for y in 0..32 {
                for x in 0..32 {
                    let color = crate::textures::water_tone(x as usize, y as usize, 32);
                    canvas.put(x, y, color.map(|c| (c * 255.0).round() as u8));
                }
            }
            canvas
        }
        "dirt" | "coarse_dirt" | "farmland_dry" | "farmland_wet" => earth(name),
        "grass_side" | "podzol_side" => {
            let mut c = earth("dirt");
            let top = turf();
            for x in 0..32 {
                let h = hash((x % 31) as usize / 3, 0, 728);
                let bottom = 5 + (h % 5) as i32;
                for y in 0..bottom {
                    c.put(x, y, rgb(top.at(x, y)));
                }
                c.put(x, bottom, [75, 98, 43]);
            }
            c.periodic(false);
            c
        }
        "podzol_top" => {
            let mut c = earth("dirt");
            for i in 0..25 {
                let h = hash(i, 2, 737);
                let (x, y) = ((h % 27) as i32, ((h >> 10) % 27) as i32);
                c.line((x, y), (x + 3, y + 2), [157, 104, 50]);
                c.line((x + 1, y), (x + 4, y + 2), [71, 67, 34]);
            }
            c.periodic(true);
            c
        }
        "stone" | "cobblestone" | "bedrock" | "deepstone" | "volcanic_rock" | "sandstone"
        | "sandstone_top" | "crypt_brick" => rock(name),
        "sand" | "red_sand" | "snow" | "ice" | "gravel" | "clay" | "terracotta"
        | "terracotta_dark" | "terracotta_pale" => granular(name),
        n if n.ends_with("_ore") => ore(name),
        n if n.ends_with("_leaves") => leaves(source, name),
        "cactus_side" => {
            let mut c = Canvas::new([83, 121, 67]);
            for x in 0..32 {
                c.rect(
                    [x, 0, 1, 32],
                    shade(
                        [89, 126, 70],
                        [0.65, 0.8, 1.1, 1.25, 1.0, 0.9, 0.8, 0.65][x as usize % 8],
                    ),
                );
            }
            for (x, y) in [(4, 6), (12, 16), (20, 8), (28, 25)] {
                c.line((x, y), (x + 1, y - 2), [210, 208, 157]);
                c.put(x, y + 1, INK);
            }
            c
        }
        _ => return None,
    })
}
