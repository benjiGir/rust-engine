//! Silhouettes natives 32 et matériaux des équipements. Aucun sprite externe.
use super::Pixel;
use super::draw::{COPPER, Canvas, Color, INK, IRON, WOOD, mix, rgb, shade};

fn material(name: &str) -> Color {
    if name.contains("wooden") {
        [153, 112, 65]
    } else if name.contains("stone") {
        [125, 135, 132]
    } else if name.contains("gold") {
        [214, 176, 65]
    } else if name.contains("sapphire") {
        [75, 153, 198]
    } else if name.contains("copper") {
        COPPER
    } else if name.contains("bronze") {
        [171, 128, 62]
    } else if name.contains("steel") {
        [136, 154, 167]
    } else {
        IRON
    }
}

fn tool(name: &str) -> Option<Canvas> {
    let kind = ["pickaxe", "shovel", "sword", "axe", "hoe", "hammer"]
        .into_iter()
        .find(|kind| name.ends_with(kind))?;
    let metal = material(name);
    let mut c = Canvas::transparent();
    for y in 0..32 {
        for x in 0..32 {
            let (dx, dy) = (x as f32 - 20.0, y as f32 - 11.0);
            let a = (dx + dy) * std::f32::consts::FRAC_1_SQRT_2;
            let b = (-dx + dy) * std::f32::consts::FRAC_1_SQRT_2;
            let handle = a.abs() <= 1.7 && (0.0..=21.0).contains(&b);
            let head = match kind {
                "pickaxe" => {
                    let curve = a.abs().powi(2) * 0.035 - 2.0;
                    a.abs() < 12.5 && b > curve - 1.8 && b < curve + 1.6 - a.abs() * 0.06
                }
                "axe" => {
                    // La grande lame est du côté opposé au joueur dans le profil
                    // en main : a négatif dans l'image, comme le dessin classique.
                    let a = -a;
                    (a.abs() < 2.2 && (-3.5..3.0).contains(&b))
                        || ((2.0..9.5).contains(&a)
                            && (a - 1.5).powi(2) * 0.5 + b.powi(2) < 53.0
                            && !(b > 2.5 && a < 5.0))
                }
                "shovel" => {
                    a.abs() < 4.7
                        && (-7.0..2.0).contains(&b)
                        && a.abs() * 0.8 - b < 8.0
                        && a.abs() + b * 0.3 < 5.0
                }
                "sword" => {
                    a.abs() < 2.4 && (-13.0..5.0).contains(&b) && a.abs() < (b + 13.0) * 0.55
                }
                "hoe" => {
                    (-2.5..8.5).contains(&a) && (-3.5..0.8).contains(&b)
                        || (6.0..8.5).contains(&a) && (-1.0..5.5).contains(&b)
                }
                _ => a.abs() < 7.5 && (-4.5..3.5).contains(&b),
            };
            let guard = kind == "sword" && a.abs() < 6.0 && (5.0..7.0).contains(&b);
            let handle = handle && (kind != "sword" || (7.0..18.0).contains(&b));
            if handle {
                let band = (b * 0.6).fract() < 0.28 && b > 10.0;
                let base = if band { [79, 53, 37] } else { WOOD };
                c.put(
                    x,
                    y,
                    shade(
                        base,
                        if a < -0.45 {
                            1.35
                        } else if a > 0.6 {
                            0.65
                        } else {
                            1.0
                        },
                    ),
                );
            }
            if head {
                let a = if kind == "axe" { -a } else { a };
                let factor = if kind == "pickaxe" {
                    let curve = a.abs().powi(2) * 0.035 - 2.0;
                    if b < curve - 0.7 {
                        1.45
                    } else if b > curve + 0.4 {
                        0.6
                    } else {
                        1.0
                    }
                } else if kind == "sword" {
                    if a < -1.2 {
                        1.5
                    } else if a < 0.2 {
                        1.15
                    } else {
                        0.68
                    }
                } else if a > 5.0 || (kind == "shovel" && (a.abs() > 3.0 || b < -5.0)) {
                    1.4
                } else if a < -1.5 || b > 2.0 {
                    0.58
                } else {
                    1.0
                };
                c.put(x, y, shade(metal, factor));
                if b.abs() < 0.8 && a.abs() < 0.8 && kind != "sword" {
                    c.put(x, y, INK);
                }
            }
            if guard {
                c.put(x, y, shade(COPPER, if b < 6.0 { 1.35 } else { 0.65 }));
            }
            if kind == "sword" && (17.0..20.0).contains(&b) && a.abs() < 2.5 {
                c.put(x, y, shade(metal, if a < 0.0 { 1.25 } else { 0.7 }));
            }
            if kind == "shovel" && (16.0..23.0).contains(&b) {
                let ring = a.powi(2) / 13.0 + (b - 19.5).powi(2) / 10.0;
                if (0.42..=1.0).contains(&ring) {
                    c.put(x, y, shade(WOOD, if a < 0.0 { 1.4 } else { 0.8 }));
                } else if ring < 0.42 {
                    c.0[x as usize + y as usize * 32] = [0; 4];
                }
            }
        }
    }
    // Fin liseré sombre extérieur, qui reste d'un pixel à la résolution native.
    outline(&mut c);
    Some(c)
}

fn outline(c: &mut Canvas) {
    let before = c.0.clone();
    for y in 1..31 {
        for x in 1..31 {
            if before[x + y * 32][3] == 0
                && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                    .iter()
                    .any(|&(x, y)| before[x + y * 32][3] > 0)
            {
                c.put(x as i32, y as i32, shade(INK, 0.8));
            }
        }
    }
}

fn armor(name: &str) -> Canvas {
    let mut c = Canvas::transparent();
    let base = material(name);
    for y in 2..30 {
        for x in 3..29 {
            let dx = (x as f32 - 15.5).abs();
            let inside = if name.ends_with("helmet") {
                ((x as f32 - 15.5) / 11.0).powi(2) + ((y as f32 - 16.0) / 12.0).powi(2) < 1.0
                    && !(y > 17 && dx < 6.5)
            } else if name.ends_with("chestplate") {
                (5..27).contains(&y)
                    && dx < if y < 12 { 12.5 } else { 8.5 }
                    && !(y < 10 && dx < 4.5)
            } else if name.ends_with("leggings") {
                (4..28).contains(&y) && dx < 10.5 && !(y > 11 && dx < 2.0)
            } else {
                (8..27).contains(&y) && (4.0..11.0).contains(&dx)
                    || (22..28).contains(&y) && (2.0..12.0).contains(&dx)
            };
            if !inside {
                continue;
            }
            let seam = y == 11 || y == 23 || dx > 8.0 || (x == 15 && y > 12);
            let f = if seam {
                0.58
            } else if x < 12 {
                1.22
            } else if x > 20 {
                0.76
            } else {
                1.0
            };
            c.put(x, y, shade(base, f));
        }
    }
    if name.ends_with("chestplate") {
        c.panel([11, 13, 10, 8], shade(base, 1.06));
        c.bolt(15, 17, COPPER);
    }
    for (x, y) in [(7, 12), (24, 12), (9, 24), (22, 24)] {
        if c.at(x, y)[3] > 0 {
            c.put(x, y, shade(base, 1.5));
        }
    }
    outline(&mut c);
    c
}

fn bottle(name: &str, source: &[Pixel]) -> Canvas {
    let mut c = Canvas::transparent();
    let liquid = if name.contains("water") {
        [54, 123, 178]
    } else {
        accent(source)
    };
    c.ellipse((16.0, 20.0), (9.0, 10.0), [68, 110, 116]);
    c.ellipse((16.0, 20.0), (7.5, 8.5), [151, 192, 183]);
    c.ellipse((16.0, 21.0), (6.5, 7.5), liquid);
    c.rect([12, 5, 8, 10], [124, 165, 159]);
    c.panel([11, 3, 10, 5], WOOD);
    c.line((12, 16), (11, 22), [203, 229, 215]);
    c.line((12, 23), (13, 25), [155, 210, 201]);
    c.line((14, 17), (20, 17), shade(liquid, 1.35));
    c.panel([14, 20, 7, 5], [207, 191, 146]);
    c.rect([16, 21, 2, 3], shade(liquid, 0.65));
    outline(&mut c);
    c
}

fn accent(source: &[Pixel]) -> Color {
    source
        .iter()
        .filter(|p| p[3] > 0)
        .max_by_key(|p| {
            let [r, g, b, _] = **p;
            r.max(g).max(b) as i32 - r.min(g).min(b) as i32
        })
        .map(|p| rgb(*p))
        .unwrap_or(COPPER)
}

pub(super) fn draw(source: &[Pixel], name: &str) -> Option<Canvas> {
    if let Some(c) = tool(name) {
        return Some(c);
    }
    Some(
        if ["helmet", "chestplate", "leggings", "boots"]
            .iter()
            .any(|s| name.ends_with(s))
        {
            armor(name)
        } else if name.contains("flask") || name.starts_with("potion_") {
            bottle(name, source)
        } else if name.ends_with("_ingot") || name.ends_with("_plate") {
            let base = material(name);
            let mut c = Canvas::transparent();
            for y in 9..24 {
                for x in 3..29 {
                    let inside = x > 5 - (y - 9) / 3 && x < 25 + (y - 9) / 3;
                    if inside {
                        c.put(
                            x,
                            y,
                            shade(
                                base,
                                if y < 13 {
                                    1.4
                                } else if y > 20 {
                                    0.64
                                } else if x > 24 {
                                    0.8
                                } else {
                                    1.0
                                },
                            ),
                        );
                    }
                }
            }
            c.line((9, 12), (23, 12), shade(base, 1.6));
            c.line((6, 20), (24, 20), shade(base, 0.76));
            c.bolt(15, 17, shade(base, 0.85));
            outline(&mut c);
            c
        } else if name.starts_with("gem_")
            || name.starts_with("shard_")
            || ["jade", "mana_crystal", "ruby", "sapphire"].contains(&name)
        {
            let base = accent(source);
            let mut c = Canvas::transparent();
            let rank = name
                .chars()
                .last()
                .and_then(|c| c.to_digit(10))
                .unwrap_or(3) as f32;
            for y in 3..29 {
                for x in 4..28 {
                    let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.0);
                    if dx.abs() * 0.8 + dy.abs() < 13.0 {
                        let f = if dy < -5.0 {
                            1.4
                        } else if dx < -2.0 {
                            1.15
                        } else if dx > 3.0 {
                            0.56
                        } else {
                            0.85
                        };
                        c.put(x, y, shade(base, f));
                        if (dx * 0.7 + dy).abs() < 0.65 {
                            c.put(x, y, shade(base, 1.65));
                        }
                    }
                }
            }
            for i in 0..rank as i32 {
                c.put(12 + i * 2, 23, mix(base, [237, 220, 158], 0.6));
            }
            outline(&mut c);
            c
        } else if name == "book" || name == "guide" || name.starts_with("tome_") {
            let mut c = Canvas::transparent();
            let cover = mix(accent(source), [78, 79, 65], 0.35);
            c.panel([6, 4, 21, 25], cover);
            c.panel([9, 6, 16, 20], [214, 201, 163]);
            for y in (9..24).step_by(3) {
                c.line((11, y), (22, y), [150, 140, 115]);
            }
            c.rect([6, 4, 4, 25], shade(cover, 0.65));
            c.panel([11, 10, 10, 10], cover);
            c.bolt(15, 14, COPPER);
            for y in [7, 25] {
                c.line((6, y), (9, y), COPPER);
            }
            c
        } else {
            return None;
        },
    )
}
