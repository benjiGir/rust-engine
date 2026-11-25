//! Textures natives des animaux. Le relief reste lisible à l'échelle d'une patte ou d'une tête.

use super::{hash, noise, quantize, shade};
use std::f32::consts::TAU;

type Rgb = [u8; 3];

#[derive(Clone, Copy)]
enum Material {
    Fleece,
    Fur(Rgb),
    AlpacaWool,
    AlpacaFur,
    Horse(bool),
    HorseMark(HorseMark),
    Feather,
    Scale,
    Fin,
    PinkSkin,
    CowPatch(bool),
    Highland(bool),
    Zebu(bool),
    Hoof,
    Horn,
}

#[derive(Clone, Copy)]
enum HorseMark {
    Blaze,
    Sock,
    Patch,
}

impl HorseMark {
    fn visible(self, u: f32, v: f32, broad: f32, fine: f32) -> bool {
        let edge = (broad - 0.5) * 0.12 + (fine - 0.5) * 0.05;
        match self {
            Self::Blaze => {
                let center = 0.5 + 0.105 * (wave(u * 1.4) - 0.5) + edge * 0.5;
                let width = 0.255 - 0.56 * (u - 0.46).powi(2) + edge;
                u > 0.04 + edge && u < 0.97 + edge && (v - center).abs() < width
            }
            Self::Sock => v > 0.12 + 0.13 * wave(2.0 * u + 0.12) + edge,
            Self::Patch => {
                let upper = ((u - 0.39) * 2.8).hypot((v - 0.43) * 2.7);
                let lower = ((u - 0.65) * 3.8).hypot((v - 0.63) * 3.8);
                upper.min(lower) < 0.92 + edge * 3.0
            }
        }
    }
}

impl Material {
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "mob_sheep_wool" => Self::Fleece,
            "mob_alpaca_wool" => Self::AlpacaWool,
            "mob_sheep_face" => Self::Fur([128, 113, 91]),
            "mob_alpaca_fur" => Self::AlpacaFur,
            "mob_cow_leg" => Self::Fur([195, 180, 143]),
            "mob_horse_coat" => Self::Horse(false),
            "mob_horse_mane" => Self::Horse(true),
            "mob_horse_blaze" => Self::HorseMark(HorseMark::Blaze),
            "mob_horse_sock" => Self::HorseMark(HorseMark::Sock),
            "mob_horse_patch" => Self::HorseMark(HorseMark::Patch),
            "mob_chicken_feathers" => Self::Feather,
            "mob_fish_scales" => Self::Scale,
            "mob_fish_fins" => Self::Fin,
            "mob_pig_skin" => Self::PinkSkin,
            "mob_cow_hide" => Self::CowPatch(false),
            "mob_cow_head" => Self::CowPatch(true),
            "mob_cow_highland_hide" => Self::Highland(false),
            "mob_cow_highland_head" => Self::Highland(true),
            "mob_cow_zebu_hide" => Self::Zebu(false),
            "mob_cow_zebu_head" => Self::Zebu(true),
            "mob_hoof" => Self::Hoof,
            "mob_horn" => Self::Horn,
            _ => return None,
        })
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t) as u8)
}

fn tile(u: f32, v: f32, size: i32, seed: u64) -> f32 {
    noise(u.rem_euclid(1.0), v.rem_euclid(1.0), size, seed)
}

fn wave(t: f32) -> f32 {
    0.5 + 0.5 * (t * TAU).sin()
}

// Une boucle éclairée par son bord, dans une toison où chaque mèche a sa propre orientation.
fn fleece(u: f32, v: f32) -> f32 {
    let (columns, rows) = (8, 8);
    let x_warp = (tile(u, v, 5, 9127) - 0.5) * 0.15;
    let y_warp = (tile(u, v, 4, 9131) - 0.5) * 0.12;
    let y = (v + y_warp).rem_euclid(1.0) * rows as f32;
    let x = (u + x_warp).rem_euclid(1.0) * columns as f32
        + 0.5 * (y.floor() as i32).rem_euclid(2) as f32;
    let random = hash(
        (x.floor() as i32).rem_euclid(columns),
        y.floor() as i32,
        9113,
    );
    let jitter = (random & 31) as f32 / 31.0;
    let (s, c) = (jitter * 1.8 - 0.9).sin_cos();
    let px = x.fract() - 0.5;
    let py = y.fract() - 0.54;
    let (dx, dy) = (c * px - s * py, s * px + c * py);
    let radius = (dx * 1.16).hypot(dy);
    let curl = wave(radius * (2.15 + 0.90 * jitter) - dy.atan2(dx) / TAU + 0.16 * jitter);
    let volume = 1.0 - (radius * 1.7).min(1.0);
    0.79 + volume * 0.19 + curl * 0.10 - dy * 0.12 + jitter * 0.08
}

// Chaque rang recouvre le suivant. La bordure courbe, la crête et la nervure
// utilisent la même géométrie de plume ou d'écaille.
fn overlap(u: f32, v: f32, columns: i32, rows: i32, seed: u64) -> (f32, f32, f32, f32) {
    let y = v * rows as f32;
    let x = u * columns as f32 + 0.5 * (y.floor() as i32).rem_euclid(2) as f32;
    let across = x.fract() - 0.5;
    let length = y.fract();
    let rounding = length + 0.40 * (across * 2.0).powi(2);
    let rim = (1.0 - (rounding - 0.89).abs() * 8.0).max(0.0);
    let grain = (hash(
        (x.floor() as i32).rem_euclid(columns),
        y.floor() as i32,
        seed,
    ) & 31) as f32
        / 31.0;
    (across, length, rim, grain)
}

pub(super) fn draw(name: &str, side: usize) -> Option<Vec<[u8; 4]>> {
    let material = Material::of(name)?;
    // Les marques partagent le même modèle détouré dans les trois résolutions.
    if side < 32 && !matches!(material, Material::HorseMark(_)) {
        return None;
    }
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let u = (x as f32 + 0.5) / side as f32;
            let v = (y as f32 + 0.5) / side as f32;
            let broad = noise(u, v, 5, 3301);
            let fine = noise(u, v, (side * 9 / 16) as i32, 4237);
            let ripple = tile(u * 4.0 + 0.020 * (v * TAU).sin(), v, 8, 4243);
            let staple = wave(23.0 * u + 0.45 * (v * TAU).sin() + broad * 0.55);
            let light = 0.84 + 0.13 * broad + 0.11 * fine;
            let color = match material {
                Material::Fleece => {
                    shade([220, 211, 189], fleece(u, v) + 0.05 * fine + 0.06 * broad)
                }
                Material::Fur(tint) => {
                    shade(tint, light + 0.10 * ripple + 0.05 * staple - 0.045 * v)
                }
                Material::AlpacaWool => {
                    let bend = 0.039 * (v * TAU).sin() + (broad - 0.5) * 0.10;
                    let locks = tile(u + bend, v * 0.46, 13, 9137);
                    let crimp = wave(13.0 * v + 1.55 * wave(10.0 * (u + bend)));
                    let fibers = wave(29.0 * (u + 0.5 * bend) + 1.2 * locks);
                    let tuft = (locks * 1.65 - 0.21).clamp(0.0, 1.0);
                    shade(
                        [222, 216, 197],
                        0.82 + 0.25 * tuft + 0.09 * crimp * tuft + 0.05 * fibers + 0.07 * fine,
                    )
                }
                Material::AlpacaFur => {
                    let grain = tile(u + 0.19 * v, v * 0.62, 17, 9157);
                    let short_hairs = wave(21.0 * u + 11.0 * v + 0.9 * broad);
                    shade(
                        [224, 218, 201],
                        0.79 + 0.14 * grain + 0.10 * broad + 0.065 * short_hairs + 0.075 * fine,
                    )
                }
                Material::Horse(mane) => {
                    let flow = u + 0.045 * (v * TAU).sin() + 0.038 * wave(v * 1.5);
                    let sheen = if mane {
                        let locks = wave(8.0 * flow + 0.48 * broad).powi(2);
                        let hairs = wave(25.0 * flow + 0.9 * locks);
                        0.73 + 0.19 * locks + 0.14 * hairs + 0.12 * tile(flow, v * 0.47, 11, 5129)
                    } else {
                        let down = tile(u + 0.30 * v, v * 0.54, 18, 5137);
                        let glow = (1.0 - (v - 0.34 - 0.14 * wave(u)).abs() * 1.5).max(0.0);
                        0.82 + 0.10 * glow + 0.08 * down + 0.06 * fine
                    };
                    shade([237; 3], sheen)
                }
                Material::HorseMark(_) => shade([237; 3], 0.87 + 0.12 * broad + 0.08 * fine),
                Material::Feather => {
                    let (across, length, rim, grain) = overlap(u, v, 7, 6, 6169);
                    let shaft = (1.0 - across.abs() * 16.0).max(0.0) * (1.0 - length);
                    let barb = wave(5.0 * length - 4.0 * across.abs());
                    let tint = mix([198, 186, 158], [242, 234, 205], 0.35 + grain * 0.58);
                    shade(
                        tint,
                        0.91 + 0.06 * fine + 0.065 * shaft + 0.025 * barb - 0.17 * rim,
                    )
                }
                Material::Scale => {
                    let (across, length, rim, grain) = overlap(u, v, 10, 8, 6197);
                    let tint = mix([75, 114, 99], [168, 183, 149], v * 0.85 + 0.10 * broad);
                    let flash = (1.0 - (across + 0.19).abs() * 3.6).max(0.0) * rim;
                    shade(
                        tint,
                        0.88 + 0.13 * grain + 0.09 * fine + 0.29 * flash - 0.20 * rim * length,
                    )
                }
                Material::Fin => {
                    let ray = wave(17.0 * u + 0.65 * (v * TAU).sin() * (u - 0.5));
                    shade(
                        [159, 135, 88],
                        0.68 + 0.23 * ray + 0.13 * broad + 0.13 * (1.0 - v),
                    )
                }
                Material::PinkSkin => {
                    let tint = mix([191, 124, 109], [220, 164, 139], broad);
                    let pore = hash(x as i32, y as i32, 6257).is_multiple_of(47);
                    shade(
                        tint,
                        0.94 + 0.09 * fine + 0.035 * ripple - if pore { 0.055 } else { 0.0 },
                    )
                }
                Material::CowPatch(head) => {
                    let islands = noise(u, v, 4, if head { 3319 } else { 3313 });
                    let boundary = islands + 0.08 * (fine - 0.5) + 0.035 * (ripple - 0.5);
                    let dark = ((boundary - if head { 0.58 } else { 0.50 }) * 17.0).clamp(0.0, 1.0);
                    shade(
                        mix([208, 193, 163], [73, 48, 35], dark),
                        light + 0.11 * ripple + 0.05 * staple,
                    )
                }
                Material::Highland(head) => {
                    let taper = wave(11.0 * u + 1.35 * v + 0.38 * broad);
                    let tint = if head { [184, 105, 59] } else { [176, 88, 43] };
                    shade(
                        tint,
                        0.67 + 0.23 * taper + 0.14 * ripple + 0.08 * staple + 0.13 * fine,
                    )
                }
                Material::Zebu(head) => {
                    let tint = if head {
                        [173, 166, 156]
                    } else {
                        [215, 210, 197]
                    };
                    let freckles = noise(u, v, 11, 6373);
                    shade(
                        tint,
                        light + 0.07 * ripple + 0.055 * staple - 0.08 * freckles,
                    )
                }
                Material::Hoof => {
                    let cleft = (u - 0.5 + (v * TAU).sin() * 0.02).abs();
                    let groove = ((0.062 - cleft) * 16.0).clamp(0.0, 1.0);
                    shade(
                        [70, 58, 49],
                        light + 0.14 * staple + 0.12 * (1.0 - v) - 0.37 * groove,
                    )
                }
                Material::Horn => {
                    let rings = wave(7.0 * v + 0.23 * wave(u));
                    shade(
                        [218, 196, 151],
                        0.77 + 0.17 * rings + 0.09 * ripple + 0.09 * broad,
                    )
                }
            };
            let [r, g, b] = color.map(quantize);
            let alpha = match material {
                Material::HorseMark(mark) if !mark.visible(u, v, broad, fine) => 0,
                _ => 255,
            };
            pixels.push([r, g, b, alpha]);
        }
    }
    Some(pixels)
}
