//! Matières périodiques, cernes et fibres sans cadre systématique au bord des blocs.
use super::{Image, SIZE, STONE, periodic_noise, scale, seamless_ground, texel_hash};

pub(super) fn mineral(kind: &str) -> Image {
    let (base, seed, contrast) = match kind {
        "stone" => ([0.48, 0.53, 0.54], 608, 0.13),
        "sand" => ([0.82, 0.74, 0.55], 610, 0.07),
        "snow" => ([0.87, 0.93, 0.93], 612, 0.035),
        "ice" => ([0.54, 0.72, 0.78], 614, 0.07),
        _ => unreachable!(),
    };
    let mut image = seamless_ground(base, seed, contrast);
    for y in 0..16 {
        for x in 0..16 {
            let u = x as f32 / 15.0;
            let v = y as f32 / 15.0;
            let grain = periodic_noise(u * 4.0, v * 4.0, 4, seed + 1);
            let shade = match kind {
                "stone" => {
                    if grain > 0.66 {
                        1.07
                    } else if grain < 0.28 {
                        0.91
                    } else {
                        1.0
                    }
                }
                "sand" => {
                    1.0 + (std::f32::consts::TAU * (v * 2.0 + u + (grain - 0.5) * 0.2)).sin()
                        * 0.025
                }
                "ice" => {
                    if (u * 2.0 + v + grain * 0.12).fract() < 0.11 {
                        1.15
                    } else {
                        0.98
                    }
                }
                _ => 1.0 + (grain - 0.5) * 0.025,
            };
            // Éviter une seconde quantification : le grain final est gradué une fois à save().
            let pixel = image.pixels[x + y * 16];
            image.set(
                x,
                y,
                scale(
                    [
                        pixel[0] as f32 / 255.0,
                        pixel[1] as f32 / 255.0,
                        pixel[2] as f32 / 255.0,
                    ],
                    shade,
                ),
            );
        }
    }
    image
}

pub(super) fn ore(color: [f32; 3], seed: u64) -> Image {
    let mut image = mineral("stone");
    // Des veines fragmentées, avec une disposition différente selon le gisement.
    for (i, (cx, cy)) in [(2, 3), (9, 2), (5, 9), (11, 11)].into_iter().enumerate() {
        let x = cx + (texel_hash(i, 0, seed) % 2) as usize;
        let y = cy + (texel_hash(i, 1, seed) % 2) as usize;
        let vertical = texel_hash(i, 2, seed).is_multiple_of(2);
        for (dx, dy, shade) in [
            (0, 0, 1.2),
            (1, 0, 1.02),
            (1, 1, 0.9),
            (2, 1, 0.78),
            (3, 1, 0.85),
        ] {
            let (dx, dy) = if vertical { (dy, dx) } else { (dx, dy) };
            image.set(x + dx, y + dy, scale(color, shade));
        }
        image.set(x, y + 2, scale(STONE, 0.78));
    }
    image
}

pub(super) fn bark(base: [f32; 3], seed: u64) -> Image {
    let mut image = seamless_ground(base, seed, 0.09);
    for y in 0..16 {
        for x in 0..16 {
            let bend = periodic_noise(0.0, y as f32 / 15.0 * 4.0, 4, seed + 1);
            let lane = ((x as f32 / 15.0 * 4.0) + (bend - 0.5) * 0.55).rem_euclid(1.0);
            let factor = if lane < 0.18 {
                0.72
            } else if lane < 0.32 {
                1.16
            } else {
                0.98
            };
            image.set(x, y, scale(base, factor));
        }
    }
    // Un nœud, à l'intérieur de la tuile ; les bords restent périodiques.
    let cx = 4 + (seed % 6) as usize;
    for (dx, dy, f) in [
        (0, 0, 0.6),
        (1, 0, 0.74),
        (0, 1, 0.68),
        (1, 1, 0.8),
        (0, 2, 0.75),
    ] {
        image.set(cx + dx, 6 + dy, scale(base, f));
    }
    image
}

pub(super) fn birch() -> Image {
    let base = [0.79, 0.79, 0.70];
    let mut image = seamless_ground(base, 660, 0.035);
    for (x, y, n) in [(2, 3, 4), (10, 7, 3), (4, 12, 2), (11, 14, 3)] {
        for dx in 0..n {
            image.set(x + dx, y, [0.30, 0.32, 0.30]);
            if dx > 0 {
                image.set(x + dx, y + 1, scale(base, 0.80));
            }
        }
    }
    image
}

pub(super) fn endgrain(bark: [f32; 3], wood: [f32; 3]) -> Image {
    let mut image = Image::filled(bark);
    for y in 1..15 {
        for x in 1..15 {
            let dx = x as f32 - 7.0;
            let dy = y as f32 - 8.0;
            let radius = (dx * dx + dy * dy * 0.84).sqrt();
            if radius < 7.1 {
                let ring = (radius + dx * 0.06).rem_euclid(2.6);
                let shade = if ring < 0.6 {
                    0.80
                } else if ring < 1.2 {
                    1.08
                } else {
                    0.98
                };
                image.set(x, y, scale(wood, shade));
            }
        }
    }
    for (x, y) in [(7, 8), (8, 8), (12, 5), (13, 4)] {
        image.set(x, y, scale(wood, 0.65));
    }
    image
}

pub(super) fn foliage(base: [f32; 3], seed: u64) -> Image {
    // Garder les différences d'essences, avec moins de vert électrique.
    let sage = [0.39, 0.52, 0.32];
    let base = std::array::from_fn(|c| base[c] * 0.70 + sage[c] * 0.30);
    let mut image = Image::filled(scale(base, 0.78));
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = x as f32 / 15.0;
            let v = y as f32 / 15.0;
            let n = periodic_noise(u * 8.0, v * 8.0, 8, seed);
            let lobes = periodic_noise(u * 4.0, v * 4.0, 4, seed + 81);
            if n < 0.21 {
                image.clear(x, y);
            } else {
                image.set(
                    x,
                    y,
                    scale(base, 0.8 + lobes * 0.34 + if n > 0.7 { 0.12 } else { 0.0 }),
                );
            }
        }
    }
    image
}

pub(super) fn farmland(wet: bool) -> Image {
    let base = if wet {
        [0.30, 0.22, 0.16]
    } else {
        [0.48, 0.35, 0.23]
    };
    let mut image = seamless_ground(base, 640, 0.07);
    for y in 0..16 {
        for x in 0..16 {
            let shade = match x % 4 {
                0 => 0.74,
                1 => 0.86,
                2 => 1.12,
                _ => 1.0,
            };
            let variation = 1.0 + (periodic_noise(0.0, y as f32 / 15.0 * 8.0, 8, 641) - 0.5) * 0.08;
            image.set(x, y, scale(base, shade * variation));
        }
    }
    image
}

pub(super) fn cactus(top: bool) -> Image {
    let base = [0.36, 0.51, 0.30];
    let mut image = Image::filled(base);
    for y in 0..16 {
        for x in 0..16 {
            let rib = if top {
                ((x as i32 - 8).abs().max((y as i32 - 8).abs())) % 3
            } else {
                x as i32 % 4
            };
            image.set(
                x,
                y,
                scale(
                    base,
                    if rib == 0 {
                        0.78
                    } else if rib == 1 {
                        1.14
                    } else {
                        0.98
                    },
                ),
            );
        }
    }
    if top {
        image.set(7, 7, [0.69, 0.70, 0.43]);
        image.set(8, 8, [0.69, 0.70, 0.43]);
    } else {
        for (x, y) in [(3, 2), (7, 6), (11, 11), (3, 14)] {
            image.set(x, y, [0.73, 0.72, 0.55]);
            image.set(x, y + 1, [0.22, 0.30, 0.20]);
        }
    }
    image
}

pub(super) fn torch() -> Image {
    let wood = [0.45, 0.32, 0.22];
    let mut image = Image::filled(wood);
    for y in 0..16 {
        for x in 0..16 {
            image.set(
                x,
                y,
                if (2..8).contains(&y) {
                    [0.90, 0.62, 0.25]
                } else if y == 8 || y == 11 {
                    super::COPPER
                } else {
                    scale(wood, if x % 3 == 0 { 0.8 } else { 1.1 })
                },
            );
        }
    }
    super::workstations::rect(&mut image, [6, 5, 10, 7], [0.98, 0.83, 0.46]);
    image
}
