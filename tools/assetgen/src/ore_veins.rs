//! Inclusions minérales peintes sur la pierre du pack, avec une bordure de roche intacte.

use crate::textures::texel_hash;

#[derive(Clone, Copy)]
enum Shape {
    Seam,
    Nodule,
    Crystal,
}

pub(crate) struct Ore {
    pub(crate) name: &'static str,
    color: [u8; 3],
    shape: Shape,
}

use Shape::{Crystal, Nodule, Seam};

pub(crate) const ORES: [Ore; 16] = [
    Ore::new("coal_ore", [39, 42, 46], Nodule),
    Ore::new("iron_ore", [191, 156, 127], Seam),
    Ore::new("copper_ore", [195, 107, 63], Seam),
    Ore::new("gold_ore", [230, 182, 66], Seam),
    Ore::new("cinnabar_ore", [177, 67, 58], Nodule),
    Ore::new("malachite_ore", [60, 151, 113], Seam),
    Ore::new("sapphire_ore", [70, 113, 199], Crystal),
    Ore::new("jade_ore", [115, 177, 133], Nodule),
    Ore::new("tin_ore", [149, 170, 188], Seam),
    Ore::new("lead_ore", [89, 93, 117], Nodule),
    Ore::new("silver_ore", [219, 223, 230], Seam),
    Ore::new("nickel_ore", [167, 156, 98], Nodule),
    Ore::new("diamond_ore", [96, 212, 219], Crystal),
    Ore::new("ruby_ore", [187, 51, 77], Crystal),
    Ore::new("emerald_ore", [60, 178, 103], Crystal),
    Ore::new("quartz_ore", [221, 201, 173], Crystal),
];

impl Ore {
    const fn new(name: &'static str, color: [u8; 3], shape: Shape) -> Self {
        Self { name, color, shape }
    }

    pub(crate) fn paint(&self, stone: &mut [[u8; 4]], size: usize) {
        let seed = self.name.bytes().fold(317u64, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
        let pixels_per_unit = size as f32 / 16.0;
        let (width, height) = match self.shape {
            Seam => (2.45, 1.08),
            Nodule => (1.62, 1.43),
            Crystal => (1.46, 1.83),
        };
        for (deposit, (cx, cy)) in [(3.8, 3.7), (11.2, 4.5), (5.9, 9.2), (11.5, 12.0)]
            .into_iter()
            .enumerate()
        {
            let h = texel_hash(deposit, 0, seed);
            let center_x = cx + ((h % 19) as f32 - 9.0) * 0.13;
            let center_y = cy + (((h >> 8) % 19) as f32 - 9.0) * 0.13;
            let scale = 0.79 + ((h >> 16) % 11) as f32 * 0.046;
            let slope = 0.32 + (((h >> 24) % 13) as f32 - 6.0) * 0.055;

            // Les arêtes de la tuile conservent toujours les pixels de la pierre nue.
            for y in 1..size - 1 {
                for x in 1..size - 1 {
                    let dx = (x as f32 + 0.5) / pixels_per_unit - center_x;
                    let dy = (y as f32 + 0.5) / pixels_per_unit - center_y;
                    let u = dx / (width * scale);
                    let v = (dy - dx * slope) / (height * scale);
                    let grain = (texel_hash(x, y, seed) % 13) as f32 / 12.0 - 0.5;
                    let edge = match self.shape {
                        Seam => u.abs() * 0.76 + (v + 0.3 * u * u).abs(),
                        Nodule => u.abs().max(v.abs()) + 0.24 * (u + v).abs(),
                        Crystal => u.abs() * 0.65 + v.abs() * 0.62 + (u - v).abs() * 0.26,
                    } + grain * 0.22;
                    if edge >= 1.06 {
                        continue;
                    }

                    let weight = if edge >= 0.82 { 0.30 } else { 0.84 };
                    let facet = if v < -0.21 + u * 0.26 {
                        1.15
                    } else if u > 0.28 || v > 0.48 {
                        0.79
                    } else {
                        1.0
                    };
                    let pixel = &mut stone[x + y * size];
                    let stone_light = 0.80 + f32::from(pixel[1]) * 0.0015;
                    for (channel, mineral) in pixel[..3].iter_mut().zip(self.color) {
                        let value = f32::from(mineral) * (facet + grain * 0.10) * stone_light;
                        *channel = (f32::from(*channel) * (1.0 - weight) + value * weight)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}
