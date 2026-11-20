//! Icônes natives des métaux bruts et des gemmes.

use super::draw::{Canvas, Color, shade};
use super::items::outline;

#[derive(Clone, Copy)]
enum Mineral {
    Tin,
    Lead,
    Silver,
    Nickel,
    Diamond,
    Emerald,
    Quartz,
}

impl Mineral {
    fn color(self) -> Color {
        match self {
            Self::Tin => [150, 173, 184],
            Self::Lead => [100, 111, 140],
            Self::Silver => [215, 224, 226],
            Self::Nickel => [160, 153, 107],
            Self::Diamond => [95, 218, 225],
            Self::Emerald => [50, 167, 113],
            Self::Quartz => [226, 211, 187],
        }
    }

    fn extent(self, x: f32, y: f32) -> f32 {
        match self {
            Self::Tin => x.abs() + y.abs() * 0.8,
            Self::Lead => x.abs().max(y.abs()) + (x + y).abs() * 0.16,
            Self::Silver => x.abs() + y.abs() * 0.8,
            Self::Nickel => (x * x + y * y).sqrt() + (x - y).abs() * 0.12,
            Self::Diamond => x.abs() * 0.84 + y.abs() * 0.82,
            Self::Emerald => x.abs().max(y.abs()) + (x.abs() + y.abs()) * 0.13,
            Self::Quartz => x.abs() + (y.abs() - 0.49).max(0.0) * 1.68,
        }
    }
}

pub(super) fn item(name: &str) -> Option<Canvas> {
    use Mineral::*;
    let (mineral, size, lean) = match name {
        "raw_tin" => (Tin, (12.3, 8.4), -0.2),
        "raw_lead" => (Lead, (10.5, 9.1), 0.20),
        "raw_silver" => (Silver, (12.5, 8.9), 0.53),
        "raw_nickel" => (Nickel, (11.8, 9.6), -0.16),
        "diamond" => (Diamond, (10.4, 10.8), 0.04),
        "emerald" => (Emerald, (9.5, 12.4), 0.23),
        "quartz" => (Quartz, (7.3, 12.1), 0.62),
        _ => return None,
    };
    let mut canvas = Canvas::transparent();
    match mineral {
        Silver => fragment(&mut canvas, Silver, (11, 20), (8.1, 6.0), 0.35),
        Quartz => fragment(&mut canvas, Quartz, (21, 17), (4.0, 9.1), -0.2),
        Tin | Lead | Nickel => fragment(&mut canvas, mineral, (9, 20), (5.2, 6.6), 0.12),
        _ => {}
    }
    fragment(&mut canvas, mineral, (16, 15), size, lean);
    outline(&mut canvas);
    Some(canvas)
}

fn fragment(
    canvas: &mut Canvas,
    mineral: Mineral,
    (cx, cy): (i32, i32),
    (width, height): (f32, f32),
    lean: f32,
) {
    let base = mineral.color();
    for y in 0..32 {
        for x in 0..32 {
            let u = ((x - cx) as f32 - (y - cy) as f32 * lean) / width;
            let v = (y - cy) as f32 / height;
            let edge = mineral.extent(u, v);
            if edge >= 1.0 {
                continue;
            }
            let facet = if edge > 0.86 && v > -0.25 {
                0.59
            } else if u < -0.27 {
                1.21
            } else if v < -0.22 + 0.31 * u {
                1.37
            } else if u > 0.32 {
                0.72
            } else {
                1.01
            };
            let fine = ((x * 17 + y * 31 + x * y) % 7) as f32 * 0.017;
            canvas.put(x, y, shade(base, facet + fine));
        }
    }
}
