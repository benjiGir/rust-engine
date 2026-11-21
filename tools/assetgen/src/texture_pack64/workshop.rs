//! Matières de l'atelier, évaluées dans le repère des faces du modèle.

use super::terrain::{mix, noise, plank_pixel};
use super::{Pixel, SIDE, color, shade};

type Rgb = [u8; 3];

const WROUGHT: Rgb = [83, 92, 89];
const COPPER: Rgb = [171, 113, 69];
const OAK: Rgb = [119, 87, 60];
const IRON: Rgb = [116, 128, 128];

fn metal(x: f32, y: f32, base: Rgb, seed: u64) -> Rgb {
    let pits = noise(x, y, 28, seed);
    let oxide = noise(x, y, 8, seed + 11);
    let scrape = noise(x, y * 5.0, 12, seed + 17);
    let scale = if oxide > 0.61 {
        (oxide - 0.61) * 0.99
    } else {
        0.0
    };
    let metal = mix(base, [143, 103, 73], scale);
    let glare = if scrape > 0.72 && pits > 0.44 {
        0.16
    } else {
        0.0
    };
    shade(
        metal,
        0.81 + pits * 0.29 + glare - (oxide - 0.48).max(0.0) * 0.30,
    )
}

fn plate_edge(x: usize, y: usize, w: usize, h: usize, base: Rgb) -> Rgb {
    let paint = metal(x as f32 / 64.0, y as f32 / 64.0, base, 9239);
    if x < 2 || y < 2 {
        shade(paint, 1.35)
    } else if x + 2 >= w || y + 2 >= h {
        shade(paint, 0.58)
    } else {
        paint
    }
}

fn stud(x: usize, y: usize, cx: usize, cy: usize) -> Option<Rgb> {
    let x = x as i32 - cx as i32;
    let y = y as i32 - cy as i32;
    (x * x + y * y < 8).then(|| shade(COPPER, (0.89 - (x + y) as f32 * 0.13).clamp(0.48, 1.38)))
}

fn chest(name: &str) -> Vec<Pixel> {
    let paired = name.split_once("_double_");
    let (family, face, shift, width) = if let Some((family, suffix)) = paired {
        let (half, face) = suffix.split_once('_').expect("moitié et face du coffre");
        (family, face, usize::from(half == "left") * SIDE, SIDE * 2)
    } else if let Some(suffix) = name.strip_prefix("vault_") {
        let face = if suffix == "side" { "front" } else { suffix };
        ("vault", face, 0, SIDE)
    } else {
        let (family, suffix) = name.split_once("chest_").expect("famille du coffre");
        let face = if suffix == "side" { "front" } else { suffix };
        (family, face, 0, SIDE)
    };

    let (body, iron, wood) = match family {
        "copper_" | "copper_chest" => ([142, 87, 53], [88, 125, 120], false),
        "iron_" | "iron_chest" => ([109, 123, 123], [64, 74, 75], false),
        "gold_" | "gold_chest" => ([167, 127, 58], [87, 77, 53], false),
        "sapphire_" | "sapphire_chest" => ([60, 89, 132], [95, 116, 127], false),
        "vault" => ([91, 98, 97], [61, 71, 76], false),
        _ => (OAK, WROUGHT, true),
    };
    // Dans le repère du modèle la droite précède la gauche. Une colonne commune
    // élimine les différences de grain pile au contact, de la caisse aux ferrures.
    let wrap_x = |x: usize| {
        if paired.is_some() && x == SIDE {
            SIDE - 1
        } else {
            x
        }
    };
    (0..SIDE * SIDE)
        .map(|i| {
            let x = wrap_x(shift + i % SIDE);
            let y = i / SIDE;
            let uv = (x as f32 / width as f32, y as f32 / 64.0);
            let edge = x.min(width - 1 - x);
            let center = x.abs_diff(width / 2);
            let inner = face == "inside";
            let top = face == "top";
            let seam = if family == "vault" { 8 } else { 24 };
            let mut paint = if wood || inner {
                let base = if inner { [68, 56, 43] } else { body };
                plank_pixel(uv.0, uv.1, base, 9203)
            } else {
                metal(uv.0, uv.1, body, 9209)
            };
            if inner {
                return color(if y % 17 == 5 {
                    shade(paint, 0.73)
                } else {
                    paint
                });
            }
            if top {
                if y <= 9 || y >= 54 || edge <= 9 {
                    paint = metal(uv.0, uv.1, iron, 9217);
                    if y == 8 || edge == 8 {
                        paint = shade(paint, 1.30);
                    }
                } else if y == 23 || y == 41 {
                    paint = shade(paint, 0.77);
                }
            } else if edge <= 9 || y <= 9 || y >= 54 {
                paint = metal(uv.0, uv.1, iron, 9217);
                if edge == 8 || y == 9 {
                    paint = shade(paint, 1.36);
                }
            } else if y == 35 || y == 47 {
                paint = shade(paint, if wood { 0.69 } else { 0.87 });
            }

            let band = edge.abs_diff(if paired.is_some() { 22 } else { 16 });
            if band < 4 && face != "side_panel" {
                paint = metal(uv.0, uv.1, iron, 9221);
                if band == 3 {
                    paint = shade(paint, 0.62);
                }
                let band_centre = if x < width / 2 {
                    x - edge + if paired.is_some() { 22 } else { 16 }
                } else {
                    width - 1 - if paired.is_some() { 22 } else { 16 }
                };
                if let Some(rivet) = [14, 51]
                    .into_iter()
                    .find_map(|cy| stud(x, y, band_centre, cy))
                {
                    paint = rivet;
                }
            }
            if !top && (seam - 2..=seam + 2).contains(&y) {
                paint = shade(paint, if y < seam { 1.17 } else { 0.56 });
            }
            if face == "front" && (16..32).contains(&y) && center <= 8 {
                let keyhole = center <= 2 && (22..28).contains(&y);
                paint = if keyhole {
                    [25, 24, 24]
                } else {
                    plate_edge(center + 1, y - 16, 11, 16, if wood { COPPER } else { iron })
                };
            } else if matches!(face, "back_panel" | "back") && band < 7 && (18..32).contains(&y) {
                paint = plate_edge(band, y - 18, 8, 14, iron);
            }
            if family == "vault" && face == "front" && (19..48).contains(&y) && center <= 15 {
                let radius = (center as f32).hypot(y as f32 - 33.0);
                if (10.0..15.0).contains(&radius) || center <= 2 || y.abs_diff(33) <= 2 {
                    paint = metal(uv.0, uv.1, IRON, 9241);
                }
            }
            color(paint)
        })
        .collect()
}

fn furnace(name: &str, rock: &[Pixel]) -> Vec<Pixel> {
    let cap = [71, 79, 78];
    let is_top = name.ends_with("top");
    let front = name == "furnace_front" || name == "alloy_furnace_side";
    let back = name.ends_with("back_panel");
    (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = (i % SIDE, i / SIDE);
            let (u, v) = (x as f32 / 64.0, y as f32 / 64.0);
            let distance = x.abs_diff(32);
            let edge = x.min(63 - x);
            let rivet = [9, 54]
                .into_iter()
                .find_map(|rx| [9, 55].into_iter().find_map(|ry| stud(x, y, rx, ry)));
            if is_top {
                let radius = (x as f32 - 31.5).hypot(y as f32 - 31.5);
                let rim = (19.5..22.0).contains(&radius);
                let slit = radius < 17.5 && y % 8 >= 5;
                let color_at = if slit {
                    [31, 36, 36]
                } else if rim {
                    metal(u, v, IRON, 9293)
                } else {
                    plate_edge(x, y, SIDE, SIDE, cap)
                };
                return color(rivet.unwrap_or(color_at));
            }
            // La bouche garde exactement la zone UV du shader (4..12, 8..14 en 16 px).
            let arch =
                (25..32).contains(&y) && distance as f32 * 0.7 + (32 - y) as f32 * 1.1 < 11.7;
            if front && ((16..48).contains(&x) && (32..56).contains(&y) || arch) {
                let bottom = name.starts_with("alloy") && (48..55).contains(&y);
                return color(if bottom {
                    shade([147, 76, 42], 0.75 + noise(u, v, 18, 9269) * 0.5)
                } else {
                    shade([27, 29, 29], 0.80 + noise(u, v, 14, 9277) * 0.30)
                });
            }
            let course = (y + 4) / 12;
            let horizontal = (y + 4) % 12;
            let vertical = (x + course % 2 * 11) % 23;
            let [r, g, b, _] = rock[i];
            let base = mix([r, g, b], [133, 114, 94], noise(u, v, 6, 9277) * 0.57);
            let masonry = if horizontal == 0 || vertical == 0 {
                shade(base, 0.62)
            } else if horizontal == 1 || vertical == 1 {
                shade(base, 1.12)
            } else {
                shade(base, 0.86)
            };
            let soot = if front {
                (0.28 - distance as f32 * 0.009 - y.abs_diff(34) as f32 * 0.009).max(0.0)
            } else {
                0.0
            };
            let mut paint = shade(masonry, 1.0 - soot);
            if !(11..56).contains(&y) || edge < 6 {
                paint = plate_edge(x, y, SIDE, SIDE, cap);
            }
            if front && (14..24).contains(&y) && (13..51).contains(&x) {
                paint = if (17..22).contains(&y) && (x + 3) % 10 < 4 {
                    [32, 39, 37]
                } else {
                    plate_edge(x - 13, y - 14, 38, 10, cap)
                };
            }
            if back && (19..47).contains(&y) && (17..47).contains(&x) {
                paint = if (22..44).contains(&y) && (y - 22) % 7 > 3 && (20..44).contains(&x) {
                    [31, 38, 37]
                } else {
                    plate_edge(x - 17, y - 19, 30, 28, cap)
                };
            }
            color(rivet.unwrap_or(paint))
        })
        .collect()
}

fn panel(name: &str) -> Vec<Pixel> {
    let wood = name.starts_with("bench") || name.starts_with("workbench");
    let worktop = name == "workbench_top";
    let has_plate = name.ends_with("back_panel") || name == "workbench_side";
    (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = (i % SIDE, i / SIDE);
            let (u, v) = (x as f32 / 64.0, y as f32 / 64.0);
            let mut paint = if wood {
                plank_pixel(u, v, OAK, 9311)
            } else {
                metal(u, v, IRON, 9319)
            };
            let frame = x.min(63 - x) < 6 || y.min(63 - y) < 7;
            if frame {
                paint = plate_edge(x, y, SIDE, SIDE, WROUGHT);
            } else if worktop && (x % 14 == 0 || y % 14 == 0) {
                paint = shade(paint, 0.80);
            }
            if has_plate && (13..51).contains(&x) && (17..46).contains(&y) {
                paint = if !wood && (18..46).contains(&x) && (23..42).contains(&y) && y % 7 > 4 {
                    [37, 48, 45]
                } else {
                    plate_edge(x - 13, y - 17, 38, 29, WROUGHT)
                };
            }
            if let Some(nail) = [10, 53]
                .into_iter()
                .find_map(|rx| [11, 52].into_iter().find_map(|ry| stud(x, y, rx, ry)))
            {
                paint = nail;
            }
            color(paint)
        })
        .collect()
}

pub(super) fn draw(name: &str, rock: &[Pixel]) -> Vec<Pixel> {
    if name.contains("chest_") || name.starts_with("vault_") {
        chest(name)
    } else if name.contains("furnace_") {
        furnace(name, rock)
    } else {
        panel(name)
    }
}

pub(super) fn door(name: &str) -> Vec<Pixel> {
    let wood = name.starts_with("wooden");
    let lower = name.ends_with("lower");
    let window = name.ends_with("window");
    (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = (i % SIDE, i / SIDE);
            let (u, v) = (x as f32 / 64.0, y as f32 / 64.0);
            let side_edge = x.min(63 - x);
            let rail = if lower { y >= 53 } else { y < 11 };
            let mut paint = if wood {
                let grain = plank_pixel(v, u, shade(OAK, 1.20), 9403);
                if x % 17 < 2 {
                    shade(grain, 0.62)
                } else {
                    grain
                }
            } else {
                metal(u, v, IRON, 9413)
            };
            if rail || side_edge < 5 {
                paint = plate_edge(x, y, SIDE, SIDE, WROUGHT);
            }
            if window && ((8..13).contains(&x) || (51..56).contains(&x)) && (8..52).contains(&y)
                || window && ((8..13).contains(&y) || (47..52).contains(&y)) && (8..56).contains(&x)
            {
                paint = metal(u, v, if wood { COPPER } else { WROUGHT }, 9419);
            }
            let hinge = if lower { 43 } else { 23 };
            if x < 14 && y.abs_diff(hinge) < 5 {
                paint = plate_edge(x, y + 5 - hinge, 14, 10, WROUGHT);
            }
            if lower && (44..59).contains(&x) && (11..25).contains(&y) {
                paint = plate_edge(x - 44, y - 11, 15, 14, COPPER);
                if (49..55).contains(&x) && (16..20).contains(&y) {
                    paint = [36, 43, 42];
                }
            }
            color(paint)
        })
        .collect()
}
