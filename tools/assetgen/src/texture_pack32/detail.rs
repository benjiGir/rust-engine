//! Dessins de plantes et finition des petits accessoires qui gardent leur silhouette.
use super::draw::{Canvas, INK, mix, noise, rgb, seed, shade};
use super::{Pixel, hash};

pub(super) fn plant(source: &[Pixel], name: &str) -> Option<Canvas> {
    let plant = [
        "tall_grass",
        "fern",
        "dead_bush",
        "seaweed",
        "vine",
        "poppy",
        "dandelion",
        "daisy",
        "cornflower",
        "lavender",
        "brown_mushroom",
        "red_mushroom",
    ]
    .contains(&name)
        || name.ends_with("_sapling")
        || ["wheat_", "carrot_", "potato_", "cabbage_", "melon_"]
            .iter()
            .any(|p| name.starts_with(p));
    if !plant {
        return None;
    }
    let mut c = Canvas::transparent();
    let green = [83, 131, 65];
    let stage = name
        .chars()
        .last()
        .and_then(|c| c.to_digit(10))
        .unwrap_or(3) as i32;
    // Ces cultures sont des rosettes ou des buissons bas, pas des tiges de céréale.
    if ["carrot_", "potato_", "cabbage_", "melon_"]
        .iter()
        .any(|p| name.starts_with(p))
    {
        let radius = 3.0 + stage as f32 * 1.5;
        if name.starts_with("melon_") && stage == 3 {
            c.ellipse((18.0, 25.0), (7.0, 6.0), [102, 141, 58]);
            for x in [14, 18, 22] {
                c.line((x, 21), (x + 1, 28), [63, 103, 43]);
            }
        }
        for i in 0..7 {
            let a = i as f32 * 2.399;
            let center = (
                16.0 + a.cos() * radius * 0.65,
                27.0 + a.sin() * radius * 0.35,
            );
            c.ellipse(
                center,
                (radius * 0.7, radius * 0.45),
                shade(green, 0.77 + i as f32 * 0.085),
            );
            c.line(
                (center.0 as i32 - 2, center.1 as i32),
                (center.0 as i32 + 2, center.1 as i32 - 1),
                shade(green, 1.35),
            );
        }
        if name.starts_with("carrot_") && stage == 3 {
            c.rect([15, 28, 3, 3], [195, 116, 43]);
            for i in 0..6 {
                c.line(
                    (16, 28),
                    (7 + i * 3, 14 + i % 3),
                    shade(green, 0.8 + i as f32 * 0.09),
                );
            }
        }
        return Some(c);
    }
    let height = 9 + stage * 5;
    let flower = ["poppy", "dandelion", "daisy", "cornflower", "lavender"].contains(&name);
    if name.contains("mushroom") {
        c.panel([13, 17, 7, 13], [186, 164, 118]);
        let base = if name.starts_with("red") {
            [172, 69, 54]
        } else {
            [135, 97, 59]
        };
        for y in 7..21 {
            for x in 4..28 {
                let r = ((x - 16) as f32 / 12.0).powi(2) + ((y - 19) as f32 / 12.0).powi(2);
                if r < 1.0 {
                    c.put(x, y, shade(base, if x < 16 { 1.2 } else { 0.8 }));
                }
            }
        }
        for (x, y) in [(10, 13), (19, 10), (22, 16), (14, 17)] {
            c.rect([x, y, 2, 2], [214, 194, 150]);
        }
        return Some(c);
    }
    let stems = if flower || name.ends_with("_sapling") {
        1
    } else {
        6
    };
    for i in 0..stems {
        let x = if stems == 1 { 16 } else { 4 + i * 4 };
        let top = 30 - height + (i * 3 % 5);
        c.line((x, 30), (x + 1, top), shade(green, 0.7));
        c.line((x + 1, 30), (x + 2, top), shade(green, 1.22));
        for y in (top + 3..28).step_by(5) {
            let direction = if (y + i) % 2 == 0 { 1 } else { -1 };
            for n in 0..6 {
                c.line(
                    (x + direction * n, y - n / 2),
                    (x + direction * n, y - n / 2 - 2),
                    shade(green, if n < 3 { 1.2 } else { 0.85 }),
                );
            }
        }
        if name.starts_with("wheat") && stage >= 2 {
            for n in 0..5 {
                let side = if n % 2 == 0 { -1 } else { 1 };
                c.rect(
                    [x + side, top + n, 2, 2],
                    shade([191, 161, 76], 0.8 + n as f32 * 0.08),
                );
            }
        }
        if flower || name.ends_with("_sapling") {
            let color = if name.ends_with("_sapling") {
                green
            } else {
                source
                    .iter()
                    .filter(|p| p[3] > 0 && (p[0] > p[1] || p[2] > p[1]))
                    .max_by_key(|p| p[0].max(p[2]))
                    .map(|p| rgb(*p))
                    .unwrap_or([214, 180, 69])
            };
            let branches = if flower { 5 } else { 14 };
            for j in 0..branches {
                let a = j as f32 * 2.399;
                let r = if flower { 3.0 } else { (j as f32).sqrt() * 2.0 };
                let (cx, cy) = (17.0 + a.cos() * r, top as f32 + a.sin() * r * 0.7);
                c.ellipse((cx, cy), (2.6, 2.0), shade(color, 0.83 + j as f32 * 0.025));
            }
            if flower {
                c.rect([16, top - 1, 2, 2], [224, 190, 84]);
            }
        }
    }
    Some(c)
}

pub(super) fn accessory(source: &[Pixel], name: &str, item: bool) -> Canvas {
    let mut c = Canvas::transparent();
    let seed = seed(name);
    let at = |x: i32, y: i32| {
        if (0..16).contains(&x) && (0..16).contains(&y) {
            source[x as usize + y as usize * 16]
        } else {
            [0; 4]
        }
    };
    for y in 0..32 {
        for x in 0..32 {
            let (sx, sy) = (x / 2, y / 2);
            let e = at(sx, sy);
            let (n, w, s, r) = (
                at(sx, sy - 1),
                at(sx - 1, sy),
                at(sx, sy + 1),
                at(sx + 1, sy),
            );
            // Les masques et repères des accessoires restent lisibles, avec des diagonales
            // affinées. La matière est reconstruite, sans ajouter du bruit blanc uniforme.
            let p = if n != s && w != r {
                match (x % 2, y % 2) {
                    (0, 0) if w == n => w,
                    (1, 0) if n == r => r,
                    (0, 1) if w == s => w,
                    (1, 1) if s == r => r,
                    _ => e,
                }
            } else {
                e
            };
            if p[3] == 0 {
                continue;
            }
            let base = rgb(p);
            let metal = base[2] >= base[0].saturating_sub(24);
            let cloth = name.contains("backpack") || name == "balloon";
            let glass = name == "glass" || name == "water";
            let (u, v) = ((x % 31) as f32 / 31.0, (y % 31) as f32 / 31.0);
            let f = if glass {
                0.96 + noise(u, v, 4, seed) * 0.08
            } else if cloth {
                0.9 + noise(u, v, 7, seed) * 0.18 + if (x + y) % 4 == 0 { 0.04 } else { 0.0 }
            } else if metal {
                0.88 + noise(u, v, 5, seed) * 0.22
            } else {
                0.85 + noise(u, v, 9, seed) * 0.26
            };
            let mut color = shade(base, f);
            if item && n[3] == 0 && y % 2 == 0 {
                color = mix(color, [228, 224, 193], 0.26);
            }
            if item && (r[3] == 0 || s[3] == 0) {
                color = shade(color, 0.75);
            }
            c.put(x, y, color);
            c.0[x as usize + y as usize * 32][3] = p[3];
        }
    }
    if item {
        // Marques de fabrication et usure groupée dans les régions assez épaisses.
        for i in 0..8 {
            let h = hash(i, 0, seed);
            let (x, y) = (3 + (h % 25) as i32, 3 + ((h >> 8) % 25) as i32);
            if [
                c.at(x - 2, y),
                c.at(x + 2, y),
                c.at(x, y - 2),
                c.at(x, y + 2),
            ]
            .iter()
            .all(|p| p[3] > 0)
            {
                let color = rgb(c.at(x, y));
                c.line((x, y), (x + 2, y - 1), shade(color, 1.3));
                c.put(x + 1, y + 1, mix(color, INK, 0.3));
            }
        }
    }
    // Seules les directions déjà raccordées de l'accessoire sont recollées.
    if (0..16).all(|y| source[y * 16] == source[y * 16 + 15]) {
        c.periodic((0..16).all(|x| source[x] == source[x + 15 * 16]));
    }
    c
}
