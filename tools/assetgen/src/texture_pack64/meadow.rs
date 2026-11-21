//! Silhouettes de prairie dessinées au centre des pixels de la tuile.
use super::{Pixel, SIDE, color, terrain::mix};
use std::f32::consts::{PI, TAU};

type Point = (f32, f32);

struct Plant(Vec<Pixel>);

impl Plant {
    // Une feuille effilée suit une tige courbe. Sa nervure sépare les deux pans.
    fn blade(&mut self, base: Point, tip: Point, width: f32, bend: f32, tint: [u8; 3]) {
        for y in tip.1.floor().max(0.) as usize..SIDE {
            let t = (base.1 - y as f32) / (base.1 - tip.1);
            if !(0.0..1.0).contains(&t) {
                continue;
            }
            let center = base.0 + (tip.0 - base.0) * t + bend * (PI * t).sin();
            let radius = width * (0.32 * (1. - t) + (PI * t).sin()).max(0.);
            for x in 0..SIDE {
                let dx = x as f32 - center;
                if dx.abs() <= radius {
                    let light = if dx < -0.3 { 0.30 } else { 0.06 };
                    self.0[y * SIDE + x] = color(mix(tint, [190, 202, 119], light + t * 0.20));
                }
            }
        }
    }

    fn petal(&mut self, center: Point, angle: f32, length: f32, width: f32, tint: [u8; 3]) {
        let (sin, cos) = angle.sin_cos();
        for y in 0..SIDE {
            for x in 0..SIDE {
                let (dx, dy) = (x as f32 - center.0, y as f32 - center.1);
                let u = (dx * cos + dy * sin) / length;
                let v = (dy * cos - dx * sin) / width;
                if u * u + v * v <= 1. {
                    self.0[y * SIDE + x] = color(super::shade(tint, 0.92 + 0.11 * u - 0.09 * v));
                }
            }
        }
    }
}

pub(super) fn draw(name: &str) -> Option<Vec<Pixel>> {
    let mut plant = Plant(vec![[0; 4]; SIDE * SIDE]);
    let green = [49, 99, 61];
    if name == "tall_grass" || name == "fern" {
        let count = if name == "fern" { 5 } else { 15 };
        for i in 0..count {
            let x = if name == "fern" {
                (6 + 13 * i) as f32
            } else {
                (i * 37 % 54 + 5) as f32
            };
            let tip = (
                x,
                if name == "fern" {
                    9. + (x - 32.).abs() * 0.85
                } else {
                    (9 + i * 13 % 30) as f32
                },
            );
            let base = (31. + (x - 31.) * 0.35, 65.);
            let bend = (31. - x) * 0.24;
            plant.blade(
                base,
                tip,
                if name == "fern" { 1.15 } else { 2.3 },
                bend,
                green,
            );
            if name == "fern" {
                for j in 1..8 {
                    let t = j as f32 / 9.;
                    let px = base.0 + (x - base.0) * t + bend * (PI * t).sin();
                    let py = 65. + (tip.1 - 65.) * t;
                    for sign in [-1., 1.] {
                        plant.blade(
                            (px, py),
                            (px + sign * (8. - 5. * t), py - 4.),
                            1.35,
                            0.,
                            green,
                        );
                    }
                }
            }
        }
    } else {
        let (petals, tint, width) = match name {
            "poppy" => (5, [220, 66, 53], 3.8),
            "cornflower" => (8, [91, 137, 219], 2.3),
            "daisy" => (11, [240, 234, 209], 1.65),
            "dandelion" => (17, [242, 197, 58], 1.45),
            "marigold" => (10, [234, 143, 40], 2.6),
            "lavender" => (0, [149, 126, 198], 2.6),
            "heather" => (0, [197, 124, 164], 2.1),
            _ => return None,
        };
        let stems = match name {
            "heather" => vec![
                (12., 36., 0.66),
                (19., 22., 0.75),
                (29., 31., 0.81),
                (38., 15., 0.83),
                (49., 28., 0.7),
            ],
            "poppy" => vec![(18., 32., 0.65), (39., 16., 1.18)],
            "dandelion" => vec![(20., 34., 0.82), (42., 25., 0.92)],
            _ => vec![(18., 30., 0.73), (45., 21., 0.83), (30., 13., 1.)],
        };
        for (x, y, scale) in stems {
            plant.blade((31., 65.), (x, y), 1.2, 2., green);
            for (dx, h) in [(-9., 49.), (9., 40.)] {
                let sx = x + (31. - x) * (h - y) / (65. - y) + 1.7;
                plant.blade((sx, h), (sx + dx, h - 9.), 2.4, dx * 0.15, green);
            }
            if petals == 0 {
                for i in 0..9 {
                    let s = if i % 2 == 0 { -1. } else { 1. };
                    plant.petal(
                        (x + s * (2.8 - i as f32 * 0.24), y + 8. - i as f32 * 1.65),
                        s * 0.8,
                        3. * scale,
                        width * scale,
                        tint,
                    );
                }
            } else {
                for i in 0..petals {
                    let a = i as f32 * TAU / petals as f32 + 0.28;
                    plant.petal(
                        (x + 4. * scale * a.cos(), y + 3.3 * scale * a.sin()),
                        a,
                        4.4 * scale,
                        width * scale,
                        tint,
                    );
                }
                let heart = if name == "poppy" {
                    [65, 50, 49]
                } else {
                    [216, 151, 51]
                };
                plant.petal((x, y), 0., 2.8 * scale, 2.1 * scale, heart);
            }
        }
    }
    Some(plant.0)
}
