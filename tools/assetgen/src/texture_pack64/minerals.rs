//! Pépites et cristaux enchâssés. Les contours, les faces et les ombres sont
//! calculés séparément, dans le même repère que la roche.

use super::terrain::mix;
use super::{Pixel, SIDE, color, shade};
use crate::textures::texel_hash;

type Rgb = [u8; 3];

#[derive(Clone, Copy)]
enum Shape {
    Nugget,
    Shard,
    Chunk,
    Nodule,
}

use Shape::{Chunk, Nodule, Nugget, Shard};

struct Mineral {
    shape: Shape,
    body: Rgb,
    gleam: Rgb,
    crust: Rgb,
}

impl Mineral {
    const fn new(shape: Shape, body: Rgb, gleam: Rgb, crust: Rgb) -> Self {
        Self {
            shape,
            body,
            gleam,
            crust,
        }
    }

    fn of(name: &str) -> Self {
        match name {
            "iron_ore" => Self::new(Nugget, [161, 116, 87], [239, 201, 171], [127, 72, 44]),
            "copper_ore" => Self::new(Nugget, [195, 103, 57], [255, 183, 108], [62, 120, 103]),
            "gold_ore" => Self::new(Nugget, [227, 157, 26], [255, 229, 110], [126, 92, 44]),
            "tin_ore" => Self::new(Nugget, [135, 155, 181], [218, 235, 251], [84, 102, 119]),
            "lead_ore" => Self::new(Nugget, [95, 95, 134], [169, 178, 214], [76, 73, 100]),
            "silver_ore" => Self::new(Nugget, [183, 193, 199], [252, 251, 238], [87, 108, 113]),
            "nickel_ore" => Self::new(Nugget, [155, 157, 86], [223, 225, 151], [96, 105, 64]),
            "coal_ore" => Self::new(Chunk, [37, 43, 50], [100, 109, 114], [42, 45, 47]),
            "cinnabar_ore" => Self::new(Chunk, [168, 51, 34], [239, 115, 66], [125, 59, 39]),
            "malachite_ore" => Self::new(Nodule, [43, 125, 88], [126, 217, 142], [52, 96, 79]),
            "jade_ore" => Self::new(Chunk, [103, 167, 146], [200, 230, 184], [83, 118, 100]),
            "sapphire_ore" => Self::new(Shard, [51, 84, 174], [142, 186, 255], [65, 80, 106]),
            "diamond_ore" => Self::new(Shard, [69, 174, 186], [209, 252, 248], [58, 115, 128]),
            "ruby_ore" => Self::new(Shard, [157, 43, 71], [255, 142, 155], [104, 64, 68]),
            "emerald_ore" => Self::new(Shard, [46, 147, 85], [159, 249, 172], [59, 109, 89]),
            "quartz_ore" => Self::new(Shard, [191, 165, 128], [253, 243, 209], [122, 104, 87]),
            _ => unreachable!("minerai sans dessin 64 × 64 : {name}"),
        }
    }

    fn face(&self, u: f32, v: f32, contour: f32, grain: f32, up_light: f32) -> Rgb {
        if contour > 0.81 {
            // Le bord éclairé est mince. Le côté dans la roche garde une lèvre sombre.
            return if up_light > 0.38 {
                mix(
                    self.body,
                    self.gleam,
                    if matches!(self.shape, Chunk) {
                        0.34
                    } else {
                        0.65
                    },
                )
            } else {
                shade(self.body, 0.52 + grain * 0.15)
            };
        }

        let polished = match self.shape {
            Shard => {
                let ridge = u + 0.19 * v;
                if v < -0.37 {
                    mix(
                        self.body,
                        self.gleam,
                        if ridge < 0.06 { 0.76 } else { 0.36 },
                    )
                } else if ridge < -0.45 {
                    mix(self.body, self.gleam, 0.34)
                } else if ridge < 0.03 {
                    mix(self.body, self.gleam, 0.62)
                } else if ridge < 0.48 {
                    self.body
                } else {
                    shade(self.body, 0.65)
                }
            }
            Nugget => {
                if v + u * 0.31 < -0.25 {
                    mix(self.body, self.gleam, 0.62 + grain * 0.19)
                } else if u - v * 0.49 > 0.34 {
                    shade(self.body, 0.62)
                } else if v - u * 0.65 > 0.32 {
                    shade(self.body, 0.79)
                } else {
                    mix(self.body, self.gleam, 0.20)
                }
            }
            Nodule => {
                let ring = (u * 0.88 + 0.26).hypot(v + 0.18);
                mix(
                    self.body,
                    self.gleam,
                    if (ring * 4.9).fract() < 0.27 {
                        0.76
                    } else {
                        0.19
                    },
                )
            }
            Chunk => {
                if v + 0.36 * u < -0.23 {
                    mix(self.body, self.gleam, 0.52 + grain * 0.11)
                } else if u - 0.48 * v > 0.30 {
                    shade(self.body, 0.72)
                } else {
                    self.body
                }
            }
        };
        shade(polished, 0.89 + 0.18 * grain)
    }
}

struct Inclusion {
    x: f32,
    y: f32,
    scale: f32,
    angle: f32,
    profile: u64,
}

fn draw(stone: &mut [Pixel], material: &Mineral, lens: Inclusion) {
    let (sin, cos) = lens.angle.sin_cos();
    let (w, h) = match material.shape {
        Nugget => (lens.scale, lens.scale * 0.79),
        Shard => (lens.scale * 0.61, lens.scale * 1.22),
        Chunk => (lens.scale * 0.96, lens.scale * 0.85),
        Nodule => (lens.scale * 1.05, lens.scale * 0.90),
    };
    // Le relief s'arrête avant le raccord de la texture avec la pierre nue.
    for y in 2..SIDE - 2 {
        for x in 2..SIDE - 2 {
            let (dx, dy) = (x as f32 + 0.5 - lens.x, y as f32 + 0.5 - lens.y);
            let (u, v) = ((dx * cos + dy * sin) / w, (-dx * sin + dy * cos) / h);
            let grain = (texel_hash(x, y, lens.profile) % 100) as f32 * 0.01;
            let contour = match material.shape {
                Nugget => {
                    // Pépites trapues, avec de petites cassures dans leur profil.
                    0.64 * u.abs().max(v.abs())
                        + 0.40 * u.hypot(v)
                        + (u * 7.0 - v * 5.0).sin() * 0.065
                }
                Shard => v.abs().max(
                    (u - v * 0.10).abs() + (-v - 0.37).max(0.0) * 1.45 + (v + 0.26).max(0.0) * 0.15,
                ),
                Chunk => u.abs().max(v.abs()) + (u * 0.45 + v * 0.31).abs(),
                Nodule => (u + 0.15 * v).hypot(v) + (u * 9.0 + v * 5.0).sin() * 0.071,
            } + grain * 0.053;
            if contour >= 1.22 {
                continue;
            }

            let up_light = (-dx / lens.scale - dy / lens.scale) * 0.73;
            let at = x + y * SIDE;
            if contour < 1.0 {
                stone[at] = color(material.face(u, v, contour, grain, up_light));
            } else {
                let [r, g, b, _] = stone[at];
                let strength = ((1.22 - contour) / 0.22).clamp(0.0, 1.0);
                let recess = shade(material.crust, if up_light < -0.1 { 0.52 } else { 0.91 });
                stone[at] = color(mix([r, g, b], recess, strength * 0.80));
            }
        }
    }
}

pub(super) fn paint(name: &str, stone: &mut [Pixel]) {
    let material = Mineral::of(name);
    let seed = name.bytes().fold(2677u64, |a, b| {
        a.wrapping_mul(41).wrapping_add(u64::from(b))
    });

    // Centres décalés et tournés selon le minerai pour distinguer les gisements.
    for (index, (x, y, size)) in [
        (18.0, 18.0, 9.1),
        (46.0, 17.0, 6.6),
        (26.0, 46.0, 8.0),
        (48.0, 44.0, 5.4),
    ]
    .into_iter()
    .enumerate()
    {
        let h = texel_hash(index, 2, seed);
        let angle = ((h >> 26) % 115) as f32 * 0.015 - 0.83;
        let x = x + (h % 105) as f32 * 0.086 - 4.6;
        let y = y + ((h >> 11) % 95) as f32 * 0.090 - 4.2;
        let (x, y) = match seed % 4 {
            1 => (64.0 - y, x),
            2 => (64.0 - x, 64.0 - y),
            3 => (y, 64.0 - x),
            _ => (x, y),
        };
        let scale = size * (0.82 + ((h >> 40) % 39) as f32 * 0.010);
        if index != 3 {
            let direction = if (h >> 33) & 1 == 0 { -1.0 } else { 1.0 };
            draw(
                stone,
                &material,
                Inclusion {
                    x: x + direction * (scale * 0.88 + 4.0),
                    y: y + 4.8 - direction * 2.4,
                    scale: scale
                        * if matches!(material.shape, Shard) {
                            0.55
                        } else {
                            0.41
                        },
                    angle: angle - direction * 0.61,
                    profile: h ^ 0x24ab,
                },
            );
        }
        draw(
            stone,
            &material,
            Inclusion {
                x,
                y,
                scale,
                angle,
                profile: h,
            },
        );
    }
}
