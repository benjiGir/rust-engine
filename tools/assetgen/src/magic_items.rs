//! Baguette, runes et sceau. Les tracés sont évalués sur la grille de chaque pack.

type Pixel = [u8; 4];
type Color = [u8; 3];
type Point = (f32, f32);

const OUTLINE: Color = [31, 33, 37];
const SLATE: Color = [92, 107, 113];
const COPPER: Color = [176, 110, 66];

pub fn draw(name: &str, side: usize) -> Option<Vec<Pixel>> {
    let color_at: &dyn Fn(f32, f32) -> Option<Color> = match name {
        "wand" => &|x, y| wand(x, y, side),
        "warden_sigil" => &|x, y| warden_sigil(x, y, side),
        _ => &|x, y| rune(name, x, y, side),
    };
    if !matches!(name, "wand" | "warden_sigil") && glyph(name).is_none() {
        return None;
    }
    let step = 16.0 / side as f32;
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let (u, v) = ((x as f32 + 0.5) * step, (y as f32 + 0.5) * step);
            pixels.push(color_at(u, v).map_or([0; 4], |[r, g, b]| [r, g, b, 255]));
        }
    }
    let body = pixels.clone();
    for y in 1..side - 1 {
        for x in 1..side - 1 {
            let at = x + y * side;
            if body[at][3] == 0
                && [at - 1, at + 1, at - side, at + side]
                    .into_iter()
                    .any(|i| body[i][3] != 0)
            {
                let [r, g, b] = OUTLINE;
                pixels[at] = [r, g, b, 255];
            }
        }
    }
    Some(pixels)
}

fn shade(color: Color, factor: f32) -> Color {
    color.map(|c| (f32::from(c) * factor).round().clamp(0.0, 255.0) as u8)
}

fn wand(x: f32, y: f32, side: usize) -> Option<Color> {
    let (dx, dy) = (x - 2.4, y - 13.3);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let (along, across) = ((dx - dy) * k, (dx + dy) * k);
    let detailed = side == 32;
    let jewel = if detailed { 12.4 } else { 12.1 };
    let shoulder = ((along - jewel) / 3.10).abs();
    let crystal_width = 1.90 - (shoulder - 0.18).max(0.0) * 1.84;

    // Facette claire côté haut, cœur cyan et arête froide dans la monture.
    if shoulder <= 1.0 && across.abs() < crystal_width {
        let crystal = if across < -0.55 && along > jewel - 1.0 {
            [184, 234, 229]
        } else if across > 0.50 {
            [46, 118, 130]
        } else if along > jewel + 1.0 {
            [121, 206, 210]
        } else {
            [68, 165, 175]
        };
        return Some(if detailed && (across + along * 0.11 - 1.25).abs() < 0.18 {
            [213, 249, 237]
        } else {
            crystal
        });
    }

    let prong = (9.0..=jewel + 0.55).contains(&along)
        && (across.abs() - (0.66 + (along - 9.0) * 0.35)).abs() < 0.38;
    if prong {
        return Some(shade(COPPER, if across < 0.0 { 1.40 } else { 0.82 }));
    }
    let wrapped = along < 4.45;
    let collar = (8.8..=10.35).contains(&along);
    let width = if wrapped || collar {
        1.12
    } else if detailed {
        0.72
    } else {
        0.84
    };
    if !(0.0..=11.15).contains(&along) || across.abs() > width {
        return None;
    }
    let base = if collar || along < 0.91 {
        COPPER
    } else if wrapped && (along - across * 0.36).rem_euclid(1.55) < 0.58 {
        [99, 72, 54]
    } else if wrapped {
        [65, 53, 50]
    } else {
        [116, 76, 49]
    };
    Some(shade(base, if across < 0.0 { 1.21 } else { 0.77 }))
}

fn glyph(name: &str) -> Option<(Color, &'static [Point])> {
    Some(match name {
        "rune_flame" => (
            [242, 136, 67],
            &[
                (8.0, 4.0),
                (9.1, 6.5),
                (10.9, 8.2),
                (10.5, 10.4),
                (8.0, 12.0),
                (5.5, 10.4),
                (5.6, 8.1),
                (7.0, 9.0),
                (8.0, 4.0),
            ],
        ),
        "rune_heal" => (
            [155, 213, 129],
            &[(8.0, 4.6), (8.0, 11.4), (8.0, 8.0), (5.0, 8.0), (11.0, 8.0)],
        ),
        "rune_blink" => (
            [185, 151, 231],
            &[
                (6.0, 11.5),
                (10.3, 8.0),
                (6.0, 4.5),
                (8.0, 8.0),
                (6.0, 11.5),
            ],
        ),
        "rune_power" => (
            [235, 191, 90],
            &[(9.2, 4.1), (5.7, 8.3), (9.6, 7.8), (7.0, 11.9)],
        ),
        "rune_reach" => (
            [115, 204, 218],
            &[
                (5.6, 10.6),
                (10.4, 5.7),
                (7.3, 5.7),
                (10.4, 5.7),
                (10.4, 8.8),
            ],
        ),
        _ => return None,
    })
}

fn dist_to_stroke(p: Point, points: &[Point]) -> f32 {
    points.windows(2).fold(f32::INFINITY, |distance, segment| {
        let [a, b] = [segment[0], segment[1]];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (px, py) = (p.0 - a.0, p.1 - a.1);
        let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        distance.min((px - dx * t).hypot(py - dy * t))
    })
}

fn rune(name: &str, x: f32, y: f32, side: usize) -> Option<Color> {
    let (ink, stroke) = glyph(name)?;
    let (dx, dy) = ((x - 8.0).abs(), (y - 8.0).abs());
    if dx > 5.0 || dy > 6.0 || dx + dy > 9.6 {
        return None;
    }
    let d = dist_to_stroke((x, y), stroke);
    if d < if side == 16 { 0.77 } else { 0.59 } {
        return Some(shade(ink, if side == 32 && d < 0.20 { 1.21 } else { 1.0 }));
    }
    if d < 1.15 {
        return Some(shade(ink, 0.37));
    }
    let border = dx > 4.1 || dy > 5.15 || dx + dy > 8.85;
    Some(if border {
        shade(SLATE, if x + y < 16.0 { 1.35 } else { 0.63 })
    } else if side == 32 && (x * 0.7 + y * 0.45).rem_euclid(3.5) < 0.45 {
        shade(SLATE, 0.83)
    } else {
        SLATE
    })
}

fn warden_sigil(x: f32, y: f32, side: usize) -> Option<Color> {
    let (dx, dy) = ((x - 8.0).abs(), (y - 8.0).abs());
    let rim = dx.max(dy) + dx.min(dy) * 0.42;
    if rim > 6.25 {
        return None;
    }
    if rim > 4.65 {
        return Some(shade(COPPER, if x + y < 16.0 { 1.40 } else { 0.78 }));
    }
    let shield = [
        (5.5, 6.0),
        (8.0, 5.0),
        (10.5, 6.0),
        (10.5, 8.2),
        (8.0, 10.9),
        (5.5, 8.2),
        (5.5, 6.0),
    ];
    if dist_to_stroke((x, y), &shield) < if side == 16 { 0.72 } else { 0.50 } {
        return Some([218, 173, 98]);
    }
    Some(if dy < 0.85 && dx + dy < 1.4 {
        [116, 163, 151]
    } else {
        [63, 80, 82]
    })
}
