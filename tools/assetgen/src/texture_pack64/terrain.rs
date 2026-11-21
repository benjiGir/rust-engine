//! Sols et bois : hauteur locale, éclairage diffus précalculé et raccords périodiques.

use super::{Pixel, SIDE, color, shade};
use crate::textures::{periodic_noise, texel_hash};

const PERIOD: usize = SIDE - 1;

pub(super) fn noise(u: f32, v: f32, frequency: i32, seed: u64) -> f32 {
    periodic_noise(
        u.rem_euclid(1.0) * frequency as f32,
        v.rem_euclid(1.0) * frequency as f32,
        frequency,
        seed,
    )
}

pub(super) fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| (f32::from(a[i]) * (1.0 - t) + f32::from(b[i]) * t).round() as u8)
}

#[derive(Clone, Copy)]
struct Surface {
    pigment: [u8; 3],
    height: f32,
}

impl Surface {
    fn new(pigment: [u8; 3], height: f32) -> Self {
        Self { pigment, height }
    }
}

/// La pente de la matière change sa lumière locale. L'éclairage du monde reste celui du jeu.
fn material(draw: impl Fn(f32, f32) -> Surface, relief: f32) -> Vec<Pixel> {
    let points: Vec<_> = (0..PERIOD * PERIOD)
        .map(|i| {
            draw(
                (i % PERIOD) as f32 / PERIOD as f32,
                (i / PERIOD) as f32 / PERIOD as f32,
            )
        })
        .collect();
    let at = |x: usize, y: usize| points[x % PERIOD + y % PERIOD * PERIOD];
    (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = (i % SIDE % PERIOD, i / SIDE % PERIOD);
            let p = at(x, y);
            let left = at(x + PERIOD - 1, y).height;
            let top = at(x, y + PERIOD - 1).height;
            let right = at(x + 1, y).height;
            let bottom = at(x, y + 1).height;
            let slope = (right - left) * 0.52 + (bottom - top) * 0.66;
            let crevice = ((left + top + right + bottom) * 0.25 - p.height).max(0.0);
            let light = (1.0 + 0.64 * relief * (slope - crevice * 0.54)).clamp(0.58, 1.40);
            color(shade(p.pigment, light))
        })
        .collect()
}

pub(super) fn rock(name: &str) -> Vec<Pixel> {
    let (base, oxide, columns, rows, seed) = match name {
        "deepstone" => ([76, 82, 87], [101, 91, 82], 6, 14, 8411),
        "cobblestone" => ([123, 121, 111], [142, 135, 104], 5, 5, 8413),
        _ => ([131, 131, 125], [148, 127, 106], 3, 5, 8417),
    };
    material(
        |u, v| {
            let coarse = noise(u, v, 4, seed);
            let grit = noise(u, v, 31, seed + 3);
            let warp = noise(u, v, 8, seed + 1);
            let x = (u + (warp - 0.5) * 0.14) * columns as f32;
            let y = (v + (coarse - 0.5) * 0.13 + (warp - 0.5) * 0.06) * rows as f32;
            let (ix, iy) = (x.floor() as i32, y.floor() as i32);
            let mut nearest = (f32::MAX, 0);
            let mut second = f32::MAX;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (cx, cy) = (ix + dx, iy + dy);
                    let h = texel_hash(
                        cx.rem_euclid(columns) as usize,
                        cy.rem_euclid(rows) as usize,
                        seed,
                    );
                    let px = x - cx as f32 - 0.14 - (h % 71) as f32 * 0.01;
                    let py = y - cy as f32 - 0.14 - ((h >> 21) % 71) as f32 * 0.01;
                    let distance =
                        px.abs() * 0.72 + py.abs() + (px * 0.64 + py * 0.42).abs() * 0.52;
                    if distance < nearest.0 {
                        second = nearest.0;
                        nearest = (distance, h);
                    } else {
                        second = second.min(distance);
                    }
                }
            }
            let edge = second - nearest.0;
            let mineral = ((nearest.1 >> 28) % 100) as f32 * 0.01;
            let nodule = noise(u, v, 17, seed + 11);
            let crossbedding = (v * 23.0 + warp * 0.94 + coarse * 0.6).fract();
            let cobble = name == "cobblestone";
            let fracture = if cobble { 0.19 } else { 0.075 + coarse * 0.11 };
            let opening = (edge / fracture).clamp(0.0, 1.0);
            // Un joint de maçonnerie se prolonge sur tout le pavé. La roche en
            // place se ressoude par endroits entre deux lèvres d'une fissure.
            let bridge = if cobble {
                0.0
            } else {
                ((coarse * 0.41 + warp * 0.32 + nodule * 0.27 - 0.46) * 3.9).clamp(0.0, 0.75)
            };
            let ledge = opening + bridge * (1.0 - opening);
            let vein = if crossbedding < 0.18 && nodule > 0.61 {
                0.06
            } else {
                0.0
            };
            let pigment = mix(
                base,
                oxide,
                (coarse - 0.47).max(0.0) * 1.2 + (mineral - 0.44).max(0.0) * 0.32,
            );
            let face = 0.83 + mineral * 0.24 + grit * 0.14 + vein;
            Surface::new(
                shade(pigment, face * (0.83 + ledge * 0.17)),
                ledge * (0.44 + mineral * 0.17) + grit * 0.08 + nodule * 0.16 + vein,
            )
        },
        0.90,
    )
}

pub(super) fn dirt() -> Vec<Pixel> {
    material(
        |u, v| {
            let clay = noise(u, v, 6, 8491);
            let grains = noise(u, v, 26, 8497);
            let clumps = noise(u, v, 13, 8501);
            let grit = noise(u, v, 37, 8513);
            let pebble = (grains * 0.51 + clumps * 0.30 + grit * 0.19 - 0.58).max(0.0);
            let fibre = (24.0 * u - 11.0 * v + clay * 2.3).rem_euclid(1.0);
            let root = fibre < 0.052 && clumps > 0.63 && clay < 0.55;
            let color = if root {
                mix([110, 76, 49], [165, 122, 80], grit)
            } else {
                let soil = mix([72, 53, 39], [151, 107, 73], clay * 0.80 + grit * 0.17);
                mix(soil, [146, 134, 113], pebble * 3.7)
            };
            Surface::new(
                color,
                clay * 0.19
                    + clumps * 0.29
                    + grit * 0.10
                    + pebble * 1.25
                    + if root { 0.07 } else { 0.0 },
            )
        },
        1.25,
    )
}

/// L'herbe pousse en poches irrégulières séparées par du feutre sec. Le podzol
/// reprend la même composition au sol, avec des aiguilles couchées sur l'humus.
pub(super) fn turf(podzol: bool) -> Vec<Pixel> {
    let seed = if podzol { 8573 } else { 8597 };
    if !podzol {
        // Grains courts de contraste régulier : aucun gros îlot répété au centre.
        return (0..SIDE * SIDE)
            .map(|i| {
                let x = (i % SIDE % PERIOD) as f32;
                let y = (i / SIDE % PERIOD) as f32;
                let grain = |x: f32, y: f32| {
                    let u = x / PERIOD as f32;
                    let v = y / PERIOD as f32;
                    noise(u, v, 17, seed) * 0.37
                        + noise(u + v, v, 29, seed + 2) * 0.48
                        + noise(u, v, 8, seed + 3) * 0.15
                };
                // La bordure latérale partage le profil du bord supérieur, avec un fondu.
                let distance = x.min(PERIOD as f32 - x);
                let other = y.min(PERIOD as f32 - y);
                let w = (1. - distance / 4.).clamp(0., 1.) * other / (distance + other).max(1.);
                let t = grain(x, y) * (1. - w) + grain(y, x) * w;
                color(mix([79, 112, 60], [129, 154, 88], t))
            })
            .collect::<Vec<_>>();
    }
    let mut pixels = material(
        |u, v| {
            let moisture = noise(u, v, 5, seed);
            let bed = noise(u, v, 11, seed + 7);
            let detail = noise(u, v, 30, seed + 11);
            let bunch = noise(u, v, 18, seed + 13);
            let litter = (moisture * 0.58 + bed * 0.42 - 0.40).clamp(0.0, 0.31) / 0.31;
            let palette = [[68, 48, 33], [110, 76, 45], [145, 110, 70]];
            let dense = mix(palette[0], palette[1], litter);
            let tips = (bunch * 0.64 + detail * 0.36 - 0.43) * 1.75;
            Surface::new(
                mix(dense, palette[2], tips),
                bed * 0.31 + detail * 0.21 + bunch * 0.15 + litter * 0.10,
            )
        },
        1.19,
    );
    for i in 0..215 {
        let h = texel_hash(i, 3, seed);
        let x = (h % PERIOD as u64) as usize;
        let y = ((h >> 12) % PERIOD as u64) as usize;
        let local = noise(
            x as f32 / PERIOD as f32,
            y as f32 / PERIOD as f32,
            10,
            seed + 7,
        );
        if local < 0.39 {
            continue;
        }
        let heading = ((h >> 24) % 360) as f32 * std::f32::consts::PI / 180.0;
        let (dy, dx) = heading.sin_cos();
        let length = 2 + ((h >> 39) % 4) as i32;
        let base = pixels[x + y * SIDE];
        let [r, g, b, _] = base;
        let accent = [159, 115, 65];
        for t in 0..length {
            let xx = (x as i32 + (dx * t as f32).round() as i32).rem_euclid(PERIOD as i32) as usize;
            let yy = (y as i32 + (dy * t as f32).round() as i32).rem_euclid(PERIOD as i32) as usize;
            let p = xx + yy * SIDE;
            let strength = 0.18 + local * 0.25 + t as f32 / length as f32 * 0.22;
            pixels[p] = color(mix([r, g, b], accent, strength));
        }
    }
    for y in 0..PERIOD {
        pixels[PERIOD + y * SIDE] = pixels[y * SIDE];
    }
    for x in 0..SIDE {
        pixels[x + PERIOD * SIDE] = pixels[x];
    }
    crate::textures::ground_top_edges(&mut pixels, SIDE);
    pixels
}

pub(super) fn loose(name: &str) -> Vec<Pixel> {
    let (base, secondary, seed) = match name {
        "sand" => ([193, 174, 139], [227, 212, 170], 8633),
        "red_sand" => ([158, 94, 62], [202, 133, 83], 8641),
        "snow" => ([198, 214, 222], [240, 241, 233], 8647),
        "ice" => ([89, 143, 156], [191, 225, 228], 8651),
        _ => ([102, 99, 94], [166, 157, 144], 8663),
    };
    material(
        |u, v| {
            let undulation = noise(u, v, 5, seed);
            let grain = noise(u, v, 34, seed + 2);
            let clusters = noise(u, v, 19, seed + 4);
            let (light, height) = match name {
                "sand" | "red_sand" => {
                    let drift = (7.0 * v - 2.0 * u + undulation * 1.96).rem_euclid(1.0);
                    (
                        grain * 0.52 + undulation * 0.48,
                        drift * 0.12 + grain * 0.09,
                    )
                }
                "ice" => {
                    let split = (6.0 * u - 4.0 * v + 1.7 * undulation).rem_euclid(1.0);
                    let secondary = (5.0 * v + 3.0 * u + clusters * 0.9).fract();
                    let crack = if undulation > 0.45 && split < 0.14
                        || clusters > 0.62 && secondary < 0.12
                    {
                        0.42
                    } else {
                        0.0
                    };
                    (
                        0.16 + clusters * 0.42 + crack,
                        undulation * 0.20 + crack * 0.26,
                    )
                }
                "snow" => (
                    0.49 + undulation * 0.3 + grain * 0.13,
                    clusters * 0.16 + grain * 0.11,
                ),
                _ => (
                    grain * 0.25 + clusters * 0.60,
                    grain * 0.18 + clusters * 0.59,
                ),
            };
            Surface::new(mix(base, secondary, light), height)
        },
        if name == "gravel" { 1.24 } else { 0.72 },
    )
}

pub(super) const WOODS: [&str; 6] = ["oak", "birch", "spruce", "acacia", "jungle", "redwood"];

fn timber(u: f32, v: f32, base: [u8; 3], seed: u64) -> Surface {
    let bend = noise(u, v, 3, seed) - 0.5;
    let (x, y) = (
        (u - 0.36).abs().min(1. - (u - 0.36).abs()),
        (v - 0.54).abs(),
    );
    let knot = (-24. * x * x - 175. * y * y).exp();
    let phase = v * 11. + 0.42 * bend + knot * 1.45;
    let latewood = (0.5 + 0.5 * (phase * std::f32::consts::TAU).cos()).powi(4);
    let fibre = noise(u, v * 4., 9, seed + 3);
    let tone = 1.02 + 0.17 * (fibre - 0.5) - 0.15 * latewood - 0.16 * knot.powi(9);
    Surface::new(shade(base, tone), 0.07 * fibre - 0.08 * latewood)
}

pub(super) fn plank_pixel(u: f32, v: f32, base: [u8; 3], seed: u64) -> [u8; 3] {
    timber(u, v, base, seed).pigment
}

pub(super) fn wood(wood: &str, surface: &str) -> Vec<Pixel> {
    let (base, bark, seed) = match wood {
        "redwood" => ([156, 91, 66], [104, 64, 53], 8759),
        "birch" => ([182, 162, 129], [174, 166, 144], 8731),
        "spruce" => ([119, 91, 63], [90, 70, 55], 8737),
        "acacia" => ([158, 100, 72], [106, 92, 81], 8741),
        "jungle" => ([144, 105, 76], [109, 84, 63], 8747),
        _ => ([141, 110, 76], [101, 86, 67], 8753),
    };
    material(
        |u, v| {
            let wave = noise(u, v, 7, seed);
            let fine = noise(u, v, 28, seed + 2);
            match surface {
                "log_top" => {
                    let (x, y) = (u - 0.48, (v - 0.53) * 1.14);
                    let angle = y.atan2(x);
                    let radius = x.hypot(y);
                    let ring =
                        (14.0 * radius + (angle * 7.0).cos() * 0.28 + wave * 0.74).rem_euclid(1.0);
                    let checked = (angle * 1.95 + fine * 0.056).rem_euclid(1.0) < 0.06
                        && radius > 0.24 + wave * 0.16;
                    let outside = u <= 0.08 || u >= 0.92 || v <= 0.08 || v >= 0.92;
                    let early = if ring < 0.17 {
                        0.72
                    } else if ring < 0.32 {
                        1.09
                    } else {
                        1.0
                    };
                    let pigment = if outside { bark } else { base };
                    let split = if checked { 0.13 } else { 0.0 };
                    if outside {
                        return Surface::new(
                            shade(bark, 0.86 + wave * 0.19 + fine * 0.12),
                            0.1 + fine * 0.06,
                        );
                    }
                    Surface::new(
                        shade(pigment, early * (0.86 + fine * 0.25) - split),
                        ring * 0.10 + fine * 0.12 - split,
                    )
                }
                "log" => {
                    let long = noise(u * 3.0, v, 9, seed + 8);
                    let split =
                        (u * 9.0 + noise(u, v, 3, seed) * 1.37 + wave * 0.54 + long * 0.18).fract();
                    let ridge = (split * (1.0 - split) * 4.0).powi(2);
                    let scar = if wood == "birch" && noise(u, v * 5.0, 8, seed + 6) > 0.76 {
                        0.55
                    } else {
                        1.0
                    };
                    Surface::new(
                        shade(
                            bark,
                            scar * (0.74 + ridge * 0.20 + wave * 0.17 + fine * 0.09),
                        ),
                        ridge * 0.32 + fine * 0.16 - (1.0 - scar) * 0.13,
                    )
                }
                _ => {
                    let plank = (v * 3.0).floor() as usize;
                    let row = (v * 3.0).fract();
                    let mut timber = timber(
                        u,
                        (v + wave * 0.015).rem_euclid(1.0),
                        shade(base, [1.05, 0.93, 1.01][plank]),
                        seed,
                    );
                    let gap = (u + [0.13, 0.79, 0.47][plank]).fract() < 0.022;
                    if row < 0.055 || gap {
                        timber.pigment = shade(timber.pigment, 0.66);
                        timber.height -= 0.24;
                    } else if row < 0.13 {
                        timber.height += 0.09;
                    }
                    timber
                }
            }
        },
        1.1,
    )
}
