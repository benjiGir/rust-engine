//! Silhouettes 32 × 32 des accessoires aux repères trop fins dans le traitement générique.

use super::draw::{COPPER, Canvas, Color, INK, IRON, SLATE, WOOD, shade};
use super::items::outline;

pub(super) fn item(name: &str) -> Option<Canvas> {
    let mut image = match name {
        "shears" => shears(),
        "axe_head" | "pickaxe_head" | "sword_blade" => forged_part(name),
        "backpack_leather" => backpack([138, 98, 63], COPPER),
        "backpack_copper" => backpack(COPPER, IRON),
        "backpack_iron" => backpack(IRON, [192, 151, 85]),
        "backpack_sapphire" => backpack([76, 139, 185], [191, 160, 96]),
        "seal_socket" | "seal_unsocket" => seal(name == "seal_unsocket"),
        "charm_fire_resistance"
        | "charm_healing"
        | "charm_regeneration"
        | "charm_speed"
        | "charm_strength" => charm(name),
        _ => return None,
    };
    outline(&mut image);
    Some(image)
}

fn shears() -> Canvas {
    let mut image = Canvas::transparent();
    for y in 3..29 {
        for x in 3..29 {
            let (u, v) = (x as f32, y as f32);
            let arm = (u - 15.5).abs();
            if y <= 16 {
                let edge = (17.0 - v) * 0.55;
                let width = (v - 2.0).min(3.0);
                if (arm - edge).abs() <= width {
                    let bevel = if arm > edge + 1.0 {
                        1.40
                    } else if arm < edge - 0.7 {
                        0.64
                    } else {
                        1.0
                    };
                    image.put(x, y, shade(IRON, bevel));
                }
            } else {
                let loop_shape = ((arm - 5.6) / 4.25).powi(2) + ((v - 23.0) / 5.5).powi(2);
                if (0.40..=1.0).contains(&loop_shape) {
                    image.put(x, y, shade(IRON, if y < 23 { 1.14 } else { 0.70 }));
                }
                if y < 21 && (arm - (v - 15.0) * 0.66).abs() < 1.6 {
                    image.put(x, y, shade(IRON, 0.80));
                }
            }
        }
    }
    image.ellipse((15.5, 16.0), (2.5, 2.5), COPPER);
    image.put(15, 15, shade(COPPER, 1.55));
    image
}

fn forged_part(name: &str) -> Canvas {
    let mut image = Canvas::transparent();
    let steel = [144, 158, 166];
    for y in 2..30 {
        for x in 2..30 {
            let (u, v) = (x as f32, y as f32);
            let center = u - 15.5;
            let light = match name {
                "pickaxe_head" => {
                    let arc = 6.0 + center * center * 0.071;
                    if (v - arc).abs() < (2.85 - center.abs() * 0.17) && center.abs() <= 12.5 {
                        Some(if v < arc - 0.4 {
                            1.36
                        } else if v > arc + 0.85 {
                            0.59
                        } else {
                            1.0
                        })
                    } else if center.abs() < 3.0 && (7..14).contains(&y) {
                        Some(if center < -0.5 { 0.92 } else { 0.59 })
                    } else {
                        None
                    }
                }
                "axe_head" => {
                    let edge = 4.0 + (v - 15.0).powi(2) * 0.024;
                    let blade =
                        (6..25).contains(&y) && u >= edge && u < 23.0 - (v - 14.0).abs() * 0.70;
                    if blade {
                        Some(if u < edge + 1.7 {
                            1.52
                        } else if u < 11.0 {
                            1.17
                        } else if y > 20 {
                            0.76
                        } else {
                            1.0
                        })
                    } else if (19..27).contains(&x) && (12..18).contains(&y) {
                        Some(if y == 12 { 1.24 } else { 0.67 })
                    } else {
                        None
                    }
                }
                _ => {
                    let axis = (u - v + 25.0) * std::f32::consts::FRAC_1_SQRT_2;
                    let across = (u + v - 31.0) * std::f32::consts::FRAC_1_SQRT_2;
                    let tip = ((32.0 - axis) * 0.48).min(3.5);
                    if axis >= 6.0 && across.abs() < tip {
                        Some(if across < -1.05 {
                            1.50
                        } else if across > 1.4 {
                            0.60
                        } else {
                            1.05
                        })
                    } else if (1.5..6.0).contains(&axis) && across.abs() < 1.20 {
                        Some(0.62)
                    } else {
                        None
                    }
                }
            };
            if let Some(factor) = light {
                image.put(x, y, shade(steel, factor));
            }
        }
    }
    if name == "axe_head" {
        image.rect([21, 13, 2, 4], INK);
    } else if name == "pickaxe_head" {
        image.rect([15, 9, 2, 3], INK);
    }
    image
}

fn backpack(body: Color, buckle: Color) -> Canvas {
    let mut image = Canvas::transparent();
    image.line((12, 5), (12, 3), shade(body, 0.69));
    image.line((12, 3), (19, 3), shade(body, 1.22));
    image.line((19, 3), (19, 6), shade(body, 0.69));
    image.panel([5, 9, 5, 17], shade(body, 0.66));
    image.panel([23, 10, 4, 16], shade(body, 0.58));
    image.panel([8, 6, 16, 22], body);
    image.panel([10, 17, 12, 10], shade(body, 0.81));
    image.panel([8, 7, 16, 9], shade(body, 1.09));
    for x in [11, 20] {
        image.rect([x, 10, 2, 12], shade(WOOD, 0.65));
        image.rect([x, 14, 2, 3], buckle);
        image.put(x, 14, shade(buckle, 1.40));
    }
    for x in [11, 13, 15, 17, 19] {
        image.put(x, 25, shade(body, 1.35));
    }
    image
}

fn seal(extract: bool) -> Canvas {
    let mut image = Canvas::transparent();
    let stone = if extract {
        [171, 110, 177]
    } else {
        [80, 175, 174]
    };
    for y in 4..28 {
        for x in 4..28 {
            let (dx, dy) = ((x - 16i32).abs(), (y - 16i32).abs());
            let edge = dx.max(dy) + (dx.min(dy) * 3) / 5;
            if edge <= 12 {
                let color = if edge >= 10 {
                    shade(COPPER, if x + y < 32 { 1.43 } else { 0.80 })
                } else {
                    SLATE
                };
                image.put(x, y, color);
            }
            if dx + dy < 6 {
                image.put(x, y, shade(stone, if x + y < 30 { 1.35 } else { 0.72 }));
            }
        }
    }
    let (corner, wing) = if extract { (8, 12) } else { (12, 8) };
    let pale = [232, 221, 168];
    for u in [0, 1] {
        image.line((16, corner + u), (12, wing + u), pale);
        image.line((16, corner + u), (20, wing + u), pale);
        image.line((16, 32 - corner - u), (12, 32 - wing - u), pale);
        image.line((16, 32 - corner - u), (20, 32 - wing - u), pale);
    }
    image
}

fn charm(name: &str) -> Canvas {
    type Segment = [(i32, i32); 2];
    let (gem, strokes): (Color, &[Segment]) = match name {
        "charm_fire_resistance" => (
            [214, 126, 91],
            &[
                [(15, 11), (19, 16)],
                [(19, 16), (18, 21)],
                [(18, 21), (12, 21)],
                [(12, 21), (12, 16)],
                [(12, 16), (15, 11)],
            ],
        ),
        "charm_healing" => (
            [153, 214, 163],
            &[[(15, 12), (15, 21)], [(11, 16), (19, 16)]],
        ),
        "charm_regeneration" => (
            [197, 144, 195],
            &[
                [(18, 13), (12, 13)],
                [(12, 13), (11, 19)],
                [(11, 19), (17, 22)],
                [(17, 22), (19, 17)],
            ],
        ),
        "charm_speed" => (
            [125, 200, 219],
            &[
                [(12, 13), (17, 16)],
                [(17, 16), (12, 20)],
                [(16, 13), (21, 16)],
                [(21, 16), (16, 20)],
            ],
        ),
        _ => (
            [222, 184, 115],
            &[
                [(11, 19), (16, 12)],
                [(16, 12), (20, 19)],
                [(20, 19), (11, 19)],
            ],
        ),
    };
    let mut image = Canvas::transparent();
    image.line((8, 9), (12, 5), shade(WOOD, 0.74));
    image.line((12, 5), (19, 5), shade(WOOD, 1.22));
    image.line((19, 5), (23, 9), shade(WOOD, 0.74));
    image.ellipse((15.5, 17.2), (10.0, 10.25), shade(COPPER, 0.71));
    image.ellipse((15.4, 16.3), (9.0, 9.0), COPPER);
    image.ellipse((15.5, 17.0), (7.3, 7.5), shade(gem, 0.48));
    for &[a, b] in strokes {
        image.line(a, b, shade(gem, 1.18));
        image.line((a.0 + 1, a.1), (b.0 + 1, b.1), gem);
    }
    image
}

pub(super) fn rune_table() -> Canvas {
    let mut image = Canvas::new(shade(SLATE, 0.65));
    image.panel([1, 1, 30, 30], shade(SLATE, 1.02));
    image.panel([5, 5, 22, 22], shade(SLATE, 0.76));
    for k in [3, 27] {
        image.line((3, k), (28, k), COPPER);
        image.line((k, 3), (k, 28), COPPER);
    }
    let inlay = [109, 192, 182];
    for (a, b) in [
        ((16, 8), (24, 16)),
        ((24, 16), (16, 24)),
        ((16, 24), (8, 16)),
        ((8, 16), (16, 8)),
    ] {
        image.line(a, b, inlay);
    }
    image.ellipse((16.0, 16.0), (3.2, 3.2), shade(INK, 1.33));
    image.line((14, 18), (18, 14), inlay);
    for x in [5, 25] {
        for y in [5, 25] {
            image.bolt(x, y, shade(COPPER, 1.22));
        }
    }
    image
}
