//! Rameaux superposés. Chaque feuille se prolonge sur la face voisine.
use super::{Pixel, SIDE, color, terrain::mix};
use crate::textures::texel_hash;
use std::f32::consts::TAU;

pub(super) fn leaves(wood: &str) -> Vec<Pixel> {
    let (dark, light, length, width) = match wood {
        "birch" => ([62, 91, 42], [164, 182, 83], 5., 2.8),
        "olive" => ([60, 80, 55], [156, 167, 115], 5.4, 1.65),
        "maple" => ([123, 55, 34], [222, 138, 58], 6.4, 3.9),
        "acacia" => ([70, 88, 41], [159, 166, 83], 4.1, 1.8),
        "jungle" => ([37, 74, 48], [119, 160, 81], 8.4, 3.6),
        "blossom" => ([115, 67, 83], [237, 178, 190], 4.8, 3.3),
        "cherry" => ([168, 84, 118], [252, 196, 216], 4.6, 3.4),
        "spruce" | "fir" | "redwood" => ([35, 65, 51], [115, 143, 91], 6.8, 1.15),
        _ => ([49, 77, 45], [143, 166, 84], 6.3, 3.4),
    };
    let seed = wood
        .bytes()
        .fold(751u64, |a, b| a.wrapping_mul(31) + u64::from(b));
    let mut pixels = vec![[0; 4]; SIDE * SIDE];
    let mut stamp = |x: i32, y: i32, c| {
        pixels[x.rem_euclid(64) as usize + SIDE * y.rem_euclid(64) as usize] = color(c)
    };
    for layer in 0..2 {
        for n in 0..20 {
            let h = texel_hash(n, layer, seed);
            let a = (h % 1024) as f32 * TAU / 1024.;
            let (dy, dx) = a.sin_cos();
            let x = (n % 5) as f32 * 12.8 + ((h >> 12) % 13) as f32;
            let y = (n / 5) as f32 * 16. + ((h >> 20) % 16) as f32;
            for t in 0..16 {
                stamp(
                    (x + dx * t as f32) as i32,
                    (y + dy * t as f32) as i32,
                    [76, 74, 47],
                );
            }
            for j in 0..6 {
                let s = if j % 2 == 0 { -1. } else { 1. };
                let angle = a + s * (0.48 + ((h >> (j + 8)) % 9) as f32 * 0.085);
                let (v, u) = angle.sin_cos();
                let cx = x + dx * (j as f32 * 2.6) + u * length * 0.85;
                let cy = y + dy * (j as f32 * 2.6) + v * length * 0.85;
                let tone = layer as f32 * 0.24 + ((h >> (j + 22)) % 15) as f32 * 0.015;
                for py in cy as i32 - 10..=cy as i32 + 10 {
                    for px in cx as i32 - 10..=cx as i32 + 10 {
                        let (xx, yy) = (px as f32 - cx, py as f32 - cy);
                        let along = (xx * u + yy * v) / length;
                        let across = (-xx * v + yy * u) / width;
                        let mut outline = (1. - along.abs().powf(1.45)).max(0.);
                        if wood == "oak" || wood == "maple" {
                            outline *= 0.87 + 0.13 * (along * 12.).cos();
                        }
                        if across.abs() < outline {
                            let vein = if across.abs() * width < 0.42 {
                                0.22
                            } else {
                                0.
                            };
                            let face = if across < 0. { 0.2 } else { 0. };
                            stamp(
                                px,
                                py,
                                mix(dark, light, tone + face + vein + 0.13 * (along + 1.)),
                            );
                        }
                    }
                }
            }
        }
    }
    pixels
}
