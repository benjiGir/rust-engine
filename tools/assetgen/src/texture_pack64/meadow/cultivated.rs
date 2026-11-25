//! Les céréales utilisent quatre tiges indépendantes par tuile. La géométrie du
//! champ prélève un quart de texture pour chaque tige enracinée.

use super::{LEAF, Plant, Rgb, SHOOT, SIDE, STEM, mix, shade};

#[derive(Clone, Copy)]
enum Grain {
    Wheat,
    Barley,
    Oats,
}

impl Grain {
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "wheat" => Self::Wheat,
            "barley" => Self::Barley,
            "oats" => Self::Oats,
            _ => return None,
        })
    }
}

fn palette(kind: Grain, stage: usize) -> (Rgb, Rgb, Rgb) {
    if stage < 3 {
        return (STEM, [89, 130, 61], SHOOT);
    }
    match kind {
        Grain::Wheat => ([155, 117, 53], [166, 146, 67], [219, 171, 79]),
        Grain::Barley => ([174, 148, 77], [165, 152, 90], [204, 175, 103]),
        Grain::Oats => ([153, 135, 80], [165, 159, 99], [214, 189, 122]),
    }
}

fn cereal(p: &mut Plant, kind: Grain, stage: usize, wild: bool) {
    let (stem, leaf, grain) = palette(kind, stage);
    for i in 0..4 {
        let left = i as f32 * 16.0;
        let center = left + 8.0;
        let tilt = [-2.1, 0.65, 2.0, -0.9][i] * if wild { 1.4 } else { 1.0 };
        let tip_y = [8.3, 10.7, 6.3, 13.0][i];
        let head = match (kind, stage) {
            (_, 0 | 1) => tip_y,
            (Grain::Wheat, _) => tip_y + 4.0,
            (Grain::Barley | Grain::Oats, _) => tip_y + 11.0,
        };
        let bend = tilt * 0.9;
        let stipe = p.stem((center, 65.0), (center + tilt, head), 1.46, bend, stem);
        let leaves: &[f32] = if stage == 0 {
            &[0.12, 0.56]
        } else {
            &[0.14, 0.40, 0.62]
        };
        for (j, t) in leaves.iter().copied().enumerate() {
            let at = stipe(t);
            let direction = if (j + i).is_multiple_of(2) { -1.0 } else { 1.0 };
            let tip = (
                (at.0 + direction * (5.0 + t * 3.0)).clamp(left + 1.2, left + 14.8),
                at.1 - 16.0 + t * 6.0,
            );
            let color = shade(leaf, if j == 0 { 0.90 } else { 1.08 });
            p.blade(
                at,
                tip,
                if stage == 0 { 2.0 } else { 2.25 },
                direction * 1.85,
                color,
                0.0,
            );
        }
        if stage < 2 {
            continue;
        }
        if matches!(kind, Grain::Oats) {
            // L'avoine porte une panicule ouverte, chaque grain pend à une ramification.
            for j in 0..4 {
                let a = stipe(0.68 + j as f32 * 0.094);
                for side in [-1.0, 1.0] {
                    let reach = 6.4 - j as f32 * 0.75;
                    let tip = (
                        (a.0 + side * reach).clamp(left + 2.5, left + 13.5),
                        a.1 - 8.2 + j as f32 * 0.86,
                    );
                    let _ = p.stem(a, tip, 0.90, side * 0.76, shade(stem, 1.18));
                    p.ellipse(
                        (tip.0 + side * 0.35, tip.1 + 1.25),
                        (1.50, 2.55),
                        side * -0.35,
                        shade(grain, 0.9 + 0.045 * j as f32),
                    );
                }
            }
            continue;
        }

        let barley = matches!(kind, Grain::Barley);
        for j in 0..8 {
            let a = stipe(0.70 + 0.0355 * j as f32);
            let taper = 0.55 + 0.45 * (std::f32::consts::PI * (j as f32 + 1.0) / 9.0).sin();
            for side in [-1.0, 1.0] {
                let width = taper * if barley { 1.57 } else { 2.22 };
                let offset = taper * if barley { 1.40 } else { 2.62 };
                let knot = (a.0 + side * offset, a.1 - 1.0 + side * 0.60);
                let tint = shade(grain, 0.88 + 0.13 * (1.0 - j as f32 / 8.0) + side * 0.07);
                p.ellipse(
                    knot,
                    (width, 2.2 + 0.55 * taper),
                    side * if barley { 0.21 } else { 0.53 },
                    tint,
                );
                if barley {
                    let bristle = (
                        (knot.0 + side * (3.2 + 0.24 * j as f32)).clamp(left + 1.4, left + 14.6),
                        knot.1 - (12.0 + j as f32 * 0.60),
                    );
                    let _ = p.stem(knot, bristle, 0.78, side * 0.40, shade(grain, 1.12));
                } else {
                    let glint = (knot.0 - side * width * 0.31, knot.1 - 0.76);
                    p.ellipse(glint, (width * 0.42, 1.05), side * 0.42, shade(tint, 1.27));
                }
            }
        }
    }
}

fn lavender(p: &mut Plant) {
    let stems = [
        (8., 24.),
        (15., 13.),
        (24., 17.),
        (30., 7.),
        (39., 10.),
        (49., 15.),
        (56., 25.),
    ];
    for (i, (x, y)) in stems.into_iter().enumerate() {
        let at = p.stem(
            (24.0 + i as f32 * 2.75, 65.0),
            (x, y + 3.0),
            1.35,
            (32.0 - x) * 0.11,
            [78, 111, 73],
        );
        for (j, phase) in [0.15, 0.26, 0.37, 0.49].into_iter().enumerate() {
            let a = at(phase);
            for side in [-1.0, 1.0] {
                let b = (a.0 + side * (9.5 - j as f32 * 1.25), a.1 - 8.0);
                p.blade(
                    a,
                    b,
                    1.65,
                    side * 1.2,
                    mix(LEAF, [146, 158, 143], 0.47),
                    0.0,
                );
            }
        }
        for j in 0..7 {
            let (bx, by) = at(0.64 + j as f32 * 0.059);
            let width = 2.4 * (1.0 - j as f32 * 0.064);
            for side in [-1.0, 1.0] {
                let center = (bx + width * side, by - 1.15);
                p.ellipse(
                    center,
                    (width, 2.85),
                    side * 0.55,
                    shade([117, 89, 170], 0.93 + 0.065 * (i % 4) as f32),
                );
                p.ellipse(
                    (center.0 + 0.28, center.1 - 0.95),
                    (width * 0.53, 1.24),
                    side * 0.5,
                    [181, 149, 220],
                );
            }
        }
    }
}

pub(in crate::texture_pack64) fn draw(name: &str, resolution: usize) -> Option<Vec<[u8; 4]>> {
    if name == "lavender" {
        let mut plant = Plant::with_resolution(SIDE, resolution);
        lavender(&mut plant);
        return Some(plant.pixels);
    }
    let (kind, stage, wild) = if let Some(species) = name.strip_prefix("wild_") {
        (Grain::of(species)?, 3, true)
    } else {
        let (species, stage) = name.rsplit_once('_')?;
        let stage: usize = stage.parse().ok()?;
        if stage > 3 {
            return None;
        }
        (Grain::of(species)?, stage, false)
    };
    let mut plant = Plant::with_resolution(SIDE, resolution);
    cereal(&mut plant, kind, stage, wild);
    Some(plant.pixels)
}
