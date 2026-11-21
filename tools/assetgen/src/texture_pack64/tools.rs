//! Outils forgés à la grille 64 : chanfrein, emmanchement, cuir et bois.

use super::terrain::{mix, noise};
use super::{Pixel, SIDE, color, shade};
use std::f32::consts::FRAC_1_SQRT_2;

pub(super) const MATERIALS: [&str; 6] = ["wooden", "stone", "copper", "iron", "golden", "sapphire"];
pub(super) const TOOLS: [&str; 4] = ["pickaxe", "axe", "shovel", "sword"];

fn material(tier: &str) -> [u8; 3] {
    match tier {
        "wooden" => [151, 109, 63],
        "stone" => [112, 128, 134],
        "copper" => [172, 97, 60],
        "golden" => [188, 147, 74],
        "sapphire" => [89, 137, 176],
        _ => [115, 123, 119],
    }
}

// Positif à l'intérieur. Le débord d'un pixel sert de trait de contour.
fn blade(a: f32, b: f32, kind: &str) -> f32 {
    match kind {
        "pickaxe" => {
            let sweep = a * a * 0.018 - 4.8;
            (23.5 - a.abs()).min(3.0 - (b - sweep).abs() - a.abs() * 0.07)
        }
        "axe" => {
            // La joue s'élargit jusqu'au tranchant et se prolonge en barbe,
            // tandis que le talon reste court du côté du manche.
            let reach = -a;
            let edge = 20.0 - (b - 1.1).powi(2) * 0.057;
            let shoulder = b + 3.4 + reach.max(0.0) * 0.29;
            let beard = 3.1 + (reach - 5.5).max(0.0) * 0.84 - b;
            let cheek = (edge - reach).min(shoulder).min(beard).min(reach + 2.2);
            let poll = (4.5 - (a - 2.8).abs()).min(3.4 - (b + 0.5).abs());
            cheek.max(poll)
        }
        "shovel" => {
            let width = if b < -13.0 {
                (b + 20.0) * 1.21
            } else {
                8.5 - (b + 13.0) * 0.21
            };
            (width - a.abs()).min(b + 20.0).min(3.2 - b)
        }
        _ => (4.4 - a.abs())
            .min((b + 27.0) * 0.63 - a.abs())
            .min(7.0 - b),
    }
}

pub(super) fn draw(tier: &str, kind: &str) -> Vec<Pixel> {
    let steel = material(tier);
    let bronze = [118, 90, 58];
    let timber = [107, 77, 52];
    let ink = [39, 41, 36];
    (0..SIDE * SIDE)
        .map(|i| {
            let (x, y) = ((i % SIDE) as f32 + 0.5, (i / SIDE) as f32 + 0.5);
            let (dx, dy) = (x - 38.0, y - 26.0);
            let (a, b) = ((dx + dy) * FRAC_1_SQRT_2, (dy - dx) * FRAC_1_SQRT_2);
            let oxides = noise(x / 64.0, y / 64.0, 17, 9631);
            let cold = noise(a * 0.052, b * 0.018, 8, 9637);
            let sword = kind == "sword";
            let grip_start = if sword { 12.5 } else { 2.0 };
            let grip_end = match kind {
                "sword" => 33.5,
                "shovel" => 30.0,
                _ => 38.0,
            };
            let grip = (3.0 - a.abs()).min(b - grip_start).min(grip_end - b);
            let guard = if sword {
                (11.0 - a.abs()).min(2.8 - (b - 9.1 + a.abs() * 0.12).abs())
            } else {
                (3.9 - a.abs()).min(5.7 - (b - 2.9).abs())
            };
            let pommel = if sword {
                (4.0 - a.abs()).min(3.4 - (b - 34.7).abs())
            } else if kind == "shovel" {
                let ring = (a.powi(2) * 0.78 + (b - 35.0).powi(2)).sqrt();
                (6.1 - ring).min(ring - 3.25)
            } else {
                (3.4 - a.abs()).min(2.5 - (b - 37.3).abs())
            };
            let head = blade(a, b, kind);
            let surface = if guard > 0.0 || pommel > 0.0 {
                shade(
                    bronze,
                    if guard.max(pommel) < 1.3 {
                        0.70
                    } else if a < -0.6 {
                        1.30
                    } else {
                        0.88 + cold * 0.21
                    },
                )
            } else if head > 0.0 {
                let bevel = if head < 0.75 {
                    1.58
                } else if head < 1.85 {
                    1.36
                } else if a > 1.5 && kind != "axe" {
                    0.76
                } else {
                    0.94 + cold * 0.19
                };
                let corrosion = (oxides - 0.56).max(0.0) * (head * 0.2).min(0.85);
                let worn = mix(steel, [118, 86, 61], corrosion);
                shade(worn, bevel + (oxides - 0.5) * 0.23)
            } else if grip > 0.0 {
                let wrap = if (b - a * 0.42).rem_euclid(4.2) < 0.9 {
                    0.64
                } else {
                    1.0
                };
                let fibres = 0.91 + cold * 0.19 + oxides * 0.07;
                shade(timber, wrap * fibres * if a < -0.6 { 1.27 } else { 0.88 })
            } else if head.max(grip).max(guard).max(pommel) > -1.0 {
                ink
            } else {
                return [0; 4];
            };
            color(surface)
        })
        .collect()
}
