//! Direction « terre, ardoise et cuivre » : motifs originaux 16 × 16, aplats groupés,
//! ombres froides et accents chauds. Aucune ressource graphique externe.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Rng;

mod items;
use items::item_icons;

const SIZE: usize = 16;

/// Une image RGBA de 16 × 16 pixels.
struct Image {
    pixels: [[u8; 4]; SIZE * SIZE],
}

impl Image {
    fn filled(color: [f32; 3]) -> Self {
        Self {
            pixels: [rgba(color); SIZE * SIZE],
        }
    }

    fn set(&mut self, x: usize, y: usize, color: [f32; 3]) {
        self.pixels[x + y * SIZE] = rgba(color);
    }

    /// Une image entièrement transparente : pour les plantes, dessinées pixel par pixel.
    fn transparent() -> Self {
        Self {
            pixels: [[0; 4]; SIZE * SIZE],
        }
    }

    /// Rend un pixel transparent (un trou dans les feuilles).
    fn clear(&mut self, x: usize, y: usize) {
        self.pixels[x + y * SIZE] = [0; 4];
    }

    /// Pose un pixel s'il est dans l'image (les dessins peuvent déborder).
    fn plot(&mut self, x: i32, y: i32, color: [f32; 3]) {
        if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
            self.set(x as usize, y as usize, color);
        }
    }

    /// Des groupes de pixels, plutôt qu'un bruit blanc qui brouille les matières.
    fn noise(mut self, base: [f32; 3], amount: f32, seed: u64) -> Self {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let broad = (texel_hash(x / 4, y / 4, seed) % 5) as f32 - 2.0;
                let cluster = (texel_hash(x / 2, y / 2, seed + 17) % 5) as f32 - 2.0;
                let grain = (texel_hash(x, y, seed + 41) % 3) as f32 - 1.0;
                let shade = 1.0 + (broad * 0.25 + cluster * 0.35 + grain * 0.12) * amount;
                self.set(x, y, scale(base, shade));
            }
        }
        self
    }

    /// Quelques taches d'une autre couleur.
    fn speckles(mut self, color: [f32; 3], density: f32, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        for y in 0..SIZE {
            for x in 0..SIZE {
                if rng.next_f32() < density {
                    let shade = 1.0 + rng.signed() * 0.1;
                    self.set(x, y, scale(color, shade));
                }
            }
        }
        self
    }

    fn save(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let mut encoder = png::Encoder::new(
            BufWriter::new(File::create(path)?),
            SIZE as u32,
            SIZE as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let bytes: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|&pixel| finish_pixel(pixel))
            .collect();
        encoder.write_header()?.write_image_data(&bytes)?;
        Ok(())
    }
}

fn texel_hash(x: usize, y: usize, seed: u64) -> u64 {
    let mut n = seed ^ (x as u64).wrapping_mul(0x9e37_79b9) ^ (y as u64).wrapping_mul(0x85eb_ca6b);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n ^ (n >> 15)
}

/// Même palette pour le terrain, les objets et les machines. L'alpha est conservé.
fn finish_pixel([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    if a == 0 {
        return [0; 4];
    }
    let rgb = [r, g, b].map(|c| c as f32 / 255.0);
    let luma = rgb[0] * 0.25 + rgb[1] * 0.6 + rgb[2] * 0.15;
    let shadow = (0.35 - luma).max(0.0) * 0.18;
    let highlight = (luma - 0.65).max(0.0) * 0.12;
    let shifts = [
        -shadow * 0.25 + highlight,
        shadow * 0.25 + highlight * 0.65,
        shadow - highlight * 0.4,
    ];
    let graded: [u8; 3] = std::array::from_fn(|i| {
        let c = (rgb[i] * 0.94 + luma * 0.06 + shifts[i]).clamp(0.0, 1.0);
        ((c * 31.0).round() / 31.0 * 255.0).round() as u8
    });
    [graded[0], graded[1], graded[2], a]
}

fn rgba(color: [f32; 3]) -> [u8; 4] {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [c(color[0]), c(color[1]), c(color[2]), 255]
}

fn scale(color: [f32; 3], factor: f32) -> [f32; 3] {
    color.map(|c| c * factor)
}

const GRASS: [f32; 3] = [0.46, 0.64, 0.32];
const DIRT: [f32; 3] = [0.48, 0.34, 0.24];
const STONE: [f32; 3] = [0.52, 0.55, 0.55];
const COPPER: [f32; 3] = [0.76, 0.48, 0.29];
const IRON: [f32; 3] = [0.64, 0.72, 0.73];
const INK: [f32; 3] = [0.12, 0.16, 0.18];

/// Bruit périodique interpolé : les deux bords ont les mêmes valeurs, sans cadre
/// ni gros îlot contrasté qui dessine une case à chaque répétition.
fn periodic_noise(x: f32, y: f32, cells: i32, seed: u64) -> f32 {
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let smooth = |v: f32| v * v * (3.0 - 2.0 * v);
    let u = smooth(x.fract());
    let v = smooth(y.fract());
    let sample = |dx: i32, dy: i32| {
        (texel_hash(
            (ix + dx).rem_euclid(cells) as usize,
            (iy + dy).rem_euclid(cells) as usize,
            seed,
        ) % 1024) as f32
            / 1023.0
    };
    let a = sample(0, 0) * (1.0 - u) + sample(1, 0) * u;
    let b = sample(0, 1) * (1.0 - u) + sample(1, 1) * u;
    a * (1.0 - v) + b * v
}

fn seamless_ground(base: [f32; 3], seed: u64, contrast: f32) -> Image {
    let mut image = Image::filled(base);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = x as f32 / (SIZE - 1) as f32;
            let v = y as f32 / (SIZE - 1) as f32;
            let grain = periodic_noise(u * 8.0, v * 8.0, 8, seed);
            let fine = periodic_noise(u * 12.0, v * 12.0, 12, seed + 89);
            image.set(
                x,
                y,
                scale(
                    base,
                    1.0 + ((grain - 0.5) * 1.4 + (fine - 0.5) * 0.6) * contrast,
                ),
            );
        }
    }
    image
}

fn grass_top() -> Image {
    seamless_ground(GRASS, 1, 0.075)
}

fn dirt() -> Image {
    seamless_ground(DIRT, 4, 0.14)
}

/// Le dessus de la terre labourée : des sillons, plus sombres quand elle est humide.
fn farmland(wet: bool) -> Image {
    let base = if wet {
        [0.27, 0.18, 0.12]
    } else {
        [0.45, 0.32, 0.21]
    };
    let mut image = Image::filled(base).noise(base, 0.08, if wet { 41 } else { 40 });
    for y in (1..SIZE).step_by(4) {
        for x in 0..SIZE {
            image.set(x, y, scale(base, 0.7));
        }
    }
    image
}

/// Le dessus de la table d'enchantement : un tapis violet, des runes et un saphir au centre.
fn enchanting_top() -> Image {
    let base = [0.35, 0.16, 0.5];
    let mut image = Image::filled(base).noise(base, 0.1, 60);
    for k in 0..SIZE {
        for edge in [0, 1, SIZE - 2, SIZE - 1] {
            image.set(k, edge, [0.75, 0.6, 0.2]);
            image.set(edge, k, [0.75, 0.6, 0.2]);
        }
    }
    for (x, y) in [
        (4, 4),
        (11, 4),
        (4, 11),
        (11, 11),
        (7, 3),
        (7, 12),
        (3, 7),
        (12, 7),
    ] {
        image.set(x, y, [0.9, 0.75, 1.0]);
    }
    for y in 6..10 {
        for x in 6..10 {
            image.set(x, y, [0.3, 0.5, 1.0]);
        }
    }
    image.set(6, 6, [0.7, 0.85, 1.0]);
    image
}

/// Le côté : du bois sombre avec un liseré violet.
fn enchanting_side() -> Image {
    let wood = [0.3, 0.2, 0.14];
    let mut image = Image::filled(wood).noise(wood, 0.08, 61);
    for x in 0..SIZE {
        image.set(x, 0, [0.5, 0.25, 0.7]);
        image.set(x, 1, [0.5, 0.25, 0.7]);
        image.set(x, 14, [0.5, 0.25, 0.7]);
        image.set(x, 15, [0.5, 0.25, 0.7]);
    }
    image
}

/// Le dessus de l'alambic : une plaque de fer, trois cuves et un brûleur.
fn brewing_top() -> Image {
    let iron = [0.42, 0.42, 0.46];
    let mut image = Image::filled(iron).noise(iron, 0.07, 62);
    for (cx, cy) in [(4, 4), (11, 4), (7, 11)] {
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                if dx * dx + dy * dy <= 5 {
                    let edge = dx * dx + dy * dy > 3;
                    image.plot(
                        cx + dx,
                        cy + dy,
                        if edge {
                            [0.25, 0.25, 0.28]
                        } else {
                            [0.35, 0.6, 0.9]
                        },
                    );
                }
            }
        }
    }
    image
}

/// Le côté : du bois cerclé de fer.
fn brewing_side() -> Image {
    let wood = [0.5, 0.38, 0.24];
    let mut image = Image::filled(wood).noise(wood, 0.07, 63);
    for x in 0..SIZE {
        for y in [2, 3, 12, 13] {
            image.set(x, y, [0.4, 0.4, 0.44]);
        }
    }
    image
}

/// Une fiole : un verre rond à col étroit ; `liquid` colore son contenu.
fn flask(liquid: Option<[f32; 3]>) -> Image {
    let glass = [0.64, 0.80, 0.81];
    let mut image = Image::transparent();
    for y in 1..6 {
        for x in 6..10 {
            image.set(
                x,
                y,
                if y < 3 {
                    [0.56, 0.38, 0.23]
                } else if y == 3 {
                    COPPER
                } else {
                    glass
                },
            );
        }
    }
    for y in 5..15 {
        for x in 2..14 {
            let dx = x as f32 - 7.5;
            let dy = y as f32 - 9.5;
            let r = dx * dx + dy * dy;
            if r > 25.0 {
                continue;
            }
            let color = if r > 17.5 {
                scale(glass, if x < 8 { 1.12 } else { 0.76 })
            } else if y >= 9 {
                liquid.map_or(scale(glass, 0.86), |l| {
                    scale(l, if x < 7 { 1.14 } else { 0.88 })
                })
            } else {
                glass
            };
            image.set(x, y, color);
        }
    }
    for (x, y) in [(5, 7), (4, 8), (4, 9)] {
        image.set(x, y, [0.94, 0.98, 0.93]);
    }
    items::outlined(image)
}

/// Les blocs des réseaux et leurs machines (phase 9c) : un fond de métal et un motif.
fn tech(kind: &str, on: bool) -> Image {
    let steel = [0.5, 0.52, 0.56];
    let mut image = Image::filled(steel).noise(steel, 0.06, 70 + kind.len() as u64);
    let edge = |image: &mut Image, c: [f32; 3]| {
        for k in 0..SIZE {
            for e in [0, SIZE - 1] {
                image.set(k, e, c);
                image.set(e, k, c);
            }
        }
    };
    match kind {
        "cable" => {
            image = Image::filled([0.25, 0.25, 0.28]).noise([0.25, 0.25, 0.28], 0.05, 71);
            for y in 6..10 {
                for x in 0..SIZE {
                    image.set(x, y, [0.8, 0.5, 0.25]);
                }
            }
            for x in (0..SIZE).step_by(4) {
                for y in 5..11 {
                    image.set(x, y, [0.15, 0.15, 0.17]);
                }
            }
        }
        "pipe" => {
            image = Image::filled([0.45, 0.5, 0.55]).noise([0.45, 0.5, 0.55], 0.05, 72);
            for x in (2..SIZE).step_by(5) {
                for y in 0..SIZE {
                    image.set(x, y, [0.25, 0.45, 0.8]);
                }
            }
        }
        "shaft" => {
            image = Image::filled([0.55, 0.4, 0.25]).noise([0.55, 0.4, 0.25], 0.08, 73);
            for x in (1..SIZE).step_by(5) {
                for y in 0..SIZE {
                    image.set(x, y, [0.45, 0.45, 0.5]);
                }
            }
        }
        "solar_panel" => {
            let blue = [0.12, 0.2, 0.45];
            image = Image::filled(blue).noise(blue, 0.05, 74);
            for k in (0..SIZE).step_by(4) {
                for j in 0..SIZE {
                    image.set(k, j, [0.55, 0.6, 0.7]);
                    image.set(j, k, [0.55, 0.6, 0.7]);
                }
            }
        }
        "battery" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            for (i, y) in (3..13).step_by(3).enumerate() {
                let lit = (i as u32) < 3;
                for x in 3..13 {
                    image.set(
                        x,
                        y,
                        if lit {
                            [0.5, 0.9, 0.3]
                        } else {
                            [0.25, 0.3, 0.2]
                        },
                    );
                    image.set(
                        x,
                        y + 1,
                        if lit {
                            [0.5, 0.9, 0.3]
                        } else {
                            [0.25, 0.3, 0.2]
                        },
                    );
                }
            }
        }
        "power_lamp" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            let glow = if on {
                [1.0, 0.95, 0.6]
            } else {
                [0.35, 0.3, 0.2]
            };
            for y in 3..13 {
                for x in 3..13 {
                    image.set(x, y, glow);
                }
            }
            if on {
                image.set(6, 6, [1.0, 1.0, 0.95]);
            }
        }
        "pump" | "filler" => {
            edge(&mut image, [0.3, 0.32, 0.36]);
            for y in 4..12 {
                for x in 4..12 {
                    image.set(x, y, [0.2, 0.4, 0.75]);
                }
            }
            // La buse : en bas pour la pompe, en haut pour le remplisseur.
            let rows: [usize; 3] = if kind == "pump" {
                [12, 13, 14]
            } else {
                [1, 2, 3]
            };
            for y in rows {
                for x in 6..10 {
                    image.set(x, y, [0.2, 0.2, 0.22]);
                }
            }
        }
        "tank" => {
            edge(&mut image, [0.3, 0.32, 0.36]);
            for y in 3..13 {
                for x in 3..13 {
                    let water = y >= 7;
                    image.set(
                        x,
                        y,
                        if water {
                            [0.2, 0.45, 0.85]
                        } else {
                            [0.7, 0.85, 0.95]
                        },
                    );
                }
            }
        }
        "motor" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            for y in 3..13 {
                for x in 3..13 {
                    let coil = (x + y) % 3 == 0;
                    image.set(
                        x,
                        y,
                        if coil {
                            [0.85, 0.5, 0.25]
                        } else {
                            [0.5, 0.3, 0.2]
                        },
                    );
                }
            }
            image.set(8, 8, [0.9, 0.9, 0.95]);
        }
        "assembler" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            // Un engrenage au centre : un anneau, des dents, un trou.
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
                    let r = (dx * dx + dy * dy).sqrt();
                    let teeth = (dy.atan2(dx) * 4.0).cos() > 0.3;
                    if (3.0..5.0).contains(&r) || (5.0..6.5).contains(&r) && teeth {
                        image.set(x, y, [0.85, 0.7, 0.3]);
                    }
                }
            }
        }
        "balloon" => {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let stripe = (x / 4) % 2 == 0;
                    let c = if stripe {
                        [0.9, 0.25, 0.2]
                    } else {
                        [0.95, 0.9, 0.85]
                    };
                    image.set(x, y, scale(c, 0.9 + 0.08 * ((x + y) % 3) as f32));
                }
            }
        }
        "propeller" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            for k in 2..14 {
                image.set(k, 5, [0.8, 0.82, 0.88]);
                image.set(k, 6, [0.8, 0.82, 0.88]);
                image.set(k, 9, [0.65, 0.68, 0.75]);
                image.set(k, 10, [0.65, 0.68, 0.75]);
            }
            for y in 7..9 {
                for x in 7..9 {
                    image.set(x, y, [0.9, 0.8, 0.3]);
                }
            }
            // Une pointe : le sens de la poussée, vers la droite.
            for k in 0..4 {
                image.set(12 + k.min(3), 7, [0.9, 0.3, 0.2]);
                image.set(12 + k.min(3), 8, [0.9, 0.3, 0.2]);
            }
        }
        "helm" => {
            let wood = [0.5, 0.36, 0.22];
            image = Image::filled(wood).noise(wood, 0.07, 75);
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
                    let r = (dx * dx + dy * dy).sqrt();
                    let spoke = dx.abs() < 0.8
                        || dy.abs() < 0.8
                        || (dx - dy).abs() < 0.9
                        || (dx + dy).abs() < 0.9;
                    if (5.5..7.0).contains(&r) || (r < 5.5 && spoke) {
                        image.set(x, y, [0.25, 0.16, 0.08]);
                    }
                }
            }
            image.set(7, 7, [0.9, 0.75, 0.3]);
            image.set(8, 8, [0.9, 0.75, 0.3]);
        }
        "fan" => {
            edge(&mut image, [0.3, 0.3, 0.33]);
            let (a, b): ([f32; 3], [f32; 3]) = if on {
                ([0.85, 0.9, 0.95], [0.65, 0.7, 0.8])
            } else {
                ([0.4, 0.42, 0.46], [0.3, 0.32, 0.36])
            };
            for k in 2..14 {
                // Quatre pales en croix ; en marche, elles sont floues (deux tons).
                image.set(k, 8, if k % 2 == 0 { a } else { b });
                image.set(8, k, if k % 2 == 0 { a } else { b });
                image.set(k, k, b);
                image.set(k, 15 - k, b);
            }
            image.set(8, 8, [0.9, 0.9, 0.2]);
        }
        _ => {}
    }
    image
}

/// Un câble de signal : un trait en croix qui relie les bords de la case.
fn wire(on: bool) -> Image {
    let color = if on {
        [1.0, 0.18, 0.1]
    } else {
        [0.42, 0.08, 0.06]
    };
    let mut image = Image::transparent();
    for k in 0..SIZE {
        for w in 7..9 {
            image.set(k, w, scale(color, 0.9 + 0.1 * (k % 2) as f32));
            image.set(w, k, scale(color, 0.9 + 0.1 * (k % 2) as f32));
        }
    }
    image
}

/// Une plaque de pierre grise sur laquelle on dessine les mécanismes.
fn plate(seed: u64) -> Image {
    let base = [0.5, 0.5, 0.53];
    let mut image = Image::filled(base).noise(base, 0.06, seed);
    for k in 0..SIZE {
        for edge in [0, SIZE - 1] {
            image.set(k, edge, scale(base, 0.65));
            image.set(edge, k, scale(base, 0.65));
        }
    }
    image
}

/// Un levier : un manche penché à gauche (relevé) ou à droite (actif, bout rouge).
fn lever(on: bool) -> Image {
    let mut image = plate(50);
    for k in 0..8i32 {
        let x = if on { 5 + k / 2 } else { 11 - k / 2 };
        image.plot(x, 12 - k, [0.45, 0.32, 0.2]);
        image.plot(x + 1, 12 - k, [0.55, 0.4, 0.25]);
    }
    let tip = if on { 9 } else { 6 };
    let color = if on {
        [1.0, 0.2, 0.12]
    } else {
        [0.5, 0.15, 0.1]
    };
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        image.plot(tip + dx, 3 + dy, color);
    }
    image.plot(8, 13, [0.3, 0.3, 0.32]);
    image
}

/// Un bouton : un carré en relief, enfoncé et rouge quand il est pressé.
fn button(on: bool) -> Image {
    let mut image = plate(51);
    let (inner, rim) = if on {
        ([0.9, 0.22, 0.15], [0.55, 0.12, 0.1])
    } else {
        ([0.72, 0.72, 0.76], [0.4, 0.4, 0.44])
    };
    for y in 4..12 {
        for x in 4..12 {
            let edge = x == 4 || y == 4 || x == 11 || y == 11;
            image.set(x, y, if edge { rim } else { inner });
        }
    }
    image
}

/// Une herse : des barreaux de fer ; ouverte, ils sont découpés sur fond transparent.
fn gate(open: bool) -> Image {
    let iron = [0.55, 0.56, 0.6];
    let mut image = if open {
        Image::transparent()
    } else {
        Image::filled([0.22, 0.22, 0.25]).noise([0.22, 0.22, 0.25], 0.08, 52)
    };
    for x in (1..SIZE).step_by(4) {
        for y in 0..SIZE {
            for w in 0..2 {
                image.set(x + w, y, scale(iron, 0.85 + 0.1 * (y % 3) as f32));
            }
        }
    }
    for y in [1, 8, 14] {
        for x in 0..SIZE {
            image.set(x, y, scale(iron, 0.7));
        }
    }
    image
}

/// L'inverseur (un point) et le répéteur (deux points), allumés ou non.
fn logic(dots: usize, on: bool) -> Image {
    let mut image = plate(53 + dots as u64);
    let color = if on {
        [1.0, 0.2, 0.1]
    } else {
        [0.35, 0.1, 0.08]
    };
    let xs: &[usize] = if dots == 1 { &[7] } else { &[4, 10] };
    for &x in xs {
        for dy in 0..3 {
            for dx in 0..3 {
                image.set(x + dx, 6 + dy, color);
            }
        }
    }
    // Une rainure d'entrée à gauche, de sortie à droite : le sens de la lecture.
    for x in 0..3 {
        image.set(x, 7, scale([0.5, 0.5, 0.53], 0.5));
    }
    for x in 13..SIZE {
        image.set(x, 7, color);
    }
    image
}

/// Un piston : une planche bordée de fer ; `centre` colore le cœur (vert : collant).
fn piston(extended: bool, centre: [f32; 3]) -> Image {
    let wood = [0.62, 0.5, 0.3];
    let mut image = Image::filled(wood).noise(wood, 0.06, 56);
    let iron = [0.5, 0.5, 0.55];
    for k in 0..SIZE {
        for edge in [0, 1, SIZE - 2, SIZE - 1] {
            image.set(k, edge, iron);
            image.set(edge, k, iron);
        }
    }
    for y in 5..11 {
        for x in 5..11 {
            let shade = if extended { 0.55 } else { 1.0 };
            image.set(x, y, scale(centre, shade));
        }
    }
    image
}

/// La tête d'un piston sorti : une planche pleine.
fn piston_head() -> Image {
    let wood = [0.7, 0.57, 0.34];
    let mut image = Image::filled(wood).noise(wood, 0.07, 57);
    for k in 0..SIZE {
        for edge in [0, SIZE - 1] {
            image.set(k, edge, scale(wood, 0.7));
            image.set(edge, k, scale(wood, 0.7));
        }
    }
    image
}

/// Le côté de l'herbe : de la terre, avec une bordure d'herbe irrégulière en haut.
fn grass_side() -> Image {
    let mut image = dirt();
    let top = grass_top();
    for x in 0..SIZE {
        let t = x as f32 / (SIZE - 1) as f32 * std::f32::consts::TAU;
        let depth = (3.0 + t.cos() * 0.6 + (t * 3.0).cos() * 0.55).round() as usize;
        for y in 0..depth {
            image.pixels[x + y * SIZE] = top.pixels[x + y * SIZE];
        }
        image.set(x, depth, scale(DIRT, 0.94));
    }
    image
}

fn stone() -> Image {
    let mut image = Image::filled(STONE).noise(STONE, 0.13, 8);
    // Éclats plats : l'arête du haut claire et une ombre courte au-dessous.
    for (x, y, length) in [(1, 3, 4), (9, 1, 3), (6, 7, 5), (0, 12, 3), (11, 12, 4)] {
        for k in 0..length {
            image.set((x + k) % SIZE, y, scale(STONE, 1.12));
            image.set((x + k) % SIZE, (y + 2) % SIZE, scale(STONE, 0.86));
        }
        image.set((x + length) % SIZE, (y + 1) % SIZE, scale(STONE, 0.90));
    }
    image
}

fn sand() -> Image {
    let base = [0.84, 0.75, 0.54];
    let mut image = Image::filled(base).noise(base, 0.06, 10);
    for (x, y, length) in [(0, 3, 6), (9, 6, 5), (3, 11, 7), (13, 14, 4)] {
        for k in 0..length {
            image.set((x + k) % SIZE, y, [0.90, 0.82, 0.62]);
            image.set((x + k + 1) % SIZE, (y + 1) % SIZE, [0.77, 0.68, 0.49]);
        }
    }
    image
}

fn snow() -> Image {
    let base = [0.88, 0.94, 0.94];
    let mut image = Image::filled(base).noise(base, 0.035, 12);
    for (x, y) in [(2, 4), (9, 1), (6, 12), (13, 9)] {
        image.set(x, y, [0.98, 0.99, 0.95]);
        image.set((x + 1) % SIZE, y, [0.98, 0.99, 0.95]);
        image.set(x, (y + 1) % SIZE, [0.78, 0.87, 0.91]);
    }
    image
}

/// L'eau : un bleu uni avec quelques vaguelettes plus claires.
fn water() -> Image {
    let base = [0.24, 0.49, 0.62];
    let mut image = Image::filled(base).noise(base, 0.07, 14);
    for (start, y, length) in [(1, 2, 6), (10, 5, 5), (4, 9, 7), (13, 13, 6)] {
        for k in 0..length {
            image.set((start + k) % SIZE, y, [0.36, 0.62, 0.69]);
            if k > 1 && k < length - 1 {
                image.set((start + k) % SIZE, (y + 1) % SIZE, [0.27, 0.54, 0.65]);
            }
        }
    }
    image
}

/// Le verre : presque blanc, avec un cadre et deux reflets en diagonale.
fn glass() -> Image {
    let mut image = Image::filled([0.76, 0.88, 0.86]);
    for k in 0..SIZE {
        image.set(k, 0, [0.59, 0.72, 0.70]);
        image.set(0, k, [0.59, 0.72, 0.70]);
        image.set(k, 15, [0.44, 0.60, 0.60]);
        image.set(15, k, [0.44, 0.60, 0.60]);
    }
    for (x, y) in [(2, 2), (13, 2), (2, 13), (13, 13)] {
        image.set(x, y, COPPER);
    }
    for k in 0..5 {
        image.set(3 + k, 7 - k, [0.93, 0.97, 0.93]);
    }
    for k in 0..3 {
        image.set(9 + k, 13 - k, [0.86, 0.94, 0.90]);
    }
    image
}

/// La torche : un bois sombre avec un cœur lumineux.
fn torch() -> Image {
    let wood = [0.38, 0.27, 0.20];
    let mut image = Image::filled(wood).noise(wood, 0.14, 15);
    for y in 0..SIZE {
        image.set(5, y, scale(wood, 0.72));
        image.set(10, y, scale(wood, 1.22));
    }
    for y in 2..12 {
        for x in 4..12 {
            let edge = x == 4 || x == 11 || y == 2 || y == 11;
            image.set(
                x,
                y,
                if edge {
                    COPPER
                } else if y < 5 {
                    [0.98, 0.90, 0.57]
                } else {
                    [0.94, 0.65, 0.26]
                },
            );
        }
    }
    for x in 4..12 {
        image.set(x, 7, scale(COPPER, 0.65));
    }
    image
}

fn glowstone() -> Image {
    let mut image = Image::filled([0.54, 0.36, 0.22]);
    for (cx, cy, radius) in [
        (3i32, 3i32, 4i32),
        (12, 1, 3),
        (9, 8, 5),
        (1, 12, 4),
        (13, 14, 4),
    ] {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() + dy.abs() > radius {
                    continue;
                }
                let color = if dy < -1 {
                    [0.98, 0.91, 0.66]
                } else if dx + dy < 2 {
                    [0.92, 0.75, 0.39]
                } else {
                    [0.74, 0.52, 0.24]
                };
                image.set(
                    (cx + dx).rem_euclid(16) as usize,
                    (cy + dy).rem_euclid(16) as usize,
                    color,
                );
            }
        }
    }
    image
}

/// L'horloge : un cadran rouge avec deux aiguilles.
fn clock() -> Image {
    let mut image = Image::filled([0.45, 0.12, 0.1]).noise([0.45, 0.12, 0.1], 0.08, 19);
    for y in 2..14 {
        for x in 2..14 {
            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
            if d < 5.8 {
                image.set(x, y, [0.92, 0.9, 0.82]);
            }
        }
    }
    for i in 0..4 {
        image.set(7, 7 - i, [0.1, 0.1, 0.1]);
        image.set(7 + i, 7, [0.1, 0.1, 0.1]);
    }
    image
}

/// La lampe : un verre ambré dans un cadre ; allumée, le verre est lumineux.
fn lamp(lit: bool) -> Image {
    let frame = [0.3, 0.22, 0.14];
    let glass = if lit {
        [1.0, 0.85, 0.45]
    } else {
        [0.42, 0.34, 0.22]
    };
    let mut image = Image::filled(frame).noise(frame, 0.1, 20);
    let mut rng = Rng::new(21);
    for y in 2..14 {
        for x in 2..14 {
            let shade = 1.0 + rng.signed() * if lit { 0.08 } else { 0.12 };
            image.set(x, y, scale(glass, shade));
        }
    }
    image
}

/// Le casseur du mod d'exemple : un bloc de métal avec une « mâchoire ».
fn breaker() -> Image {
    let base = [0.42, 0.45, 0.5];
    let mut image = Image::filled(base).noise(base, 0.08, 22);
    for x in 3..13 {
        for y in [4, 11] {
            image.set(x, y, [0.18, 0.18, 0.2]);
        }
    }
    for y in 5..11 {
        for x in (4..13).step_by(2) {
            image.set(x, y, [0.75, 0.75, 0.78]);
        }
    }
    image
}

/// Le rubis du mod d'exemple : des facettes en losange.
fn ruby() -> Image {
    let base = [0.7, 0.06, 0.14];
    let mut image = Image::filled([0.35, 0.03, 0.08]);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = (x as i32 - 8).abs() + (y as i32 - 8).abs();
            if d < 8 {
                let shade = if (x + y) % 4 < 2 { 1.0 } else { 0.8 };
                image.set(x, y, scale(base, shade + (7 - d) as f32 * 0.05));
            }
        }
    }
    image.set(6, 5, [1.0, 0.7, 0.75]);
    image
}

/// Les textures du mod de référence partagent les matériaux du jeu.
fn mechanism(kind: &str) -> Image {
    let mut image = Image::filled(INK).noise(INK, 0.15, 150);
    match kind {
        "shaft" => {
            image = log_side([0.48, 0.35, 0.24], 151);
            for x in [0, 7, 15] {
                for y in 0..SIZE {
                    image.set(x, y, IRON);
                }
            }
        }
        "gear" => {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let dx = x as f32 - 7.5;
                    let dy = y as f32 - 7.5;
                    let r = dx.hypot(dy);
                    let tooth = (dy.atan2(dx) * 6.0).cos() > 0.0;
                    if (3.0..5.6).contains(&r) || ((5.6..7.8).contains(&r) && tooth) {
                        image.set(x, y, scale(COPPER, if x + y < 15 { 1.20 } else { 0.80 }));
                    } else if r < 2.2 {
                        image.set(x, y, IRON);
                    }
                }
            }
        }
        "belt" => {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let factor = if y % 4 == 0 { 1.7 } else { 1.0 };
                    image.set(
                        x,
                        y,
                        if !(2..=13).contains(&x) {
                            COPPER
                        } else {
                            scale(INK, factor)
                        },
                    );
                }
            }
        }
        "crank" | "press" => {
            image = plate(152);
            for y in 2..14 {
                for x in 2..14 {
                    image.set(x, y, INK);
                }
            }
            if kind == "press" {
                for y in 3..10 {
                    for x in 6..10 {
                        image.set(x, y, COPPER);
                    }
                }
                for y in [9, 10, 13] {
                    for x in 3..13 {
                        image.set(x, y, IRON);
                    }
                }
            } else {
                for y in 5..9 {
                    for x in 6..10 {
                        image.set(x, y, IRON);
                    }
                }
                for x in 8..14 {
                    image.set(x, 7, COPPER);
                    image.set(x, 8, COPPER);
                }
                for y in 3..8 {
                    image.set(13, y, [0.61, 0.41, 0.26]);
                }
            }
            for (x, y) in [(1, 1), (14, 1), (1, 14), (14, 14)] {
                image.set(x, y, COPPER);
            }
        }
        _ => {}
    }
    image
}

fn metal_plate(color: [f32; 3]) -> Image {
    let mut image = Image::transparent();
    for y in 4..13 {
        for x in 2..14 {
            let edge = x == 2 || y == 12;
            image.set(
                x,
                y,
                scale(
                    color,
                    if edge {
                        0.65
                    } else if y == 4 {
                        1.2
                    } else {
                        1.0
                    },
                ),
            );
        }
    }
    for (x, y) in [(4, 6), (11, 10)] {
        image.set(x, y, scale(color, 0.5));
    }
    image
}

// ----- Le terrain (phase O1) -----

const DEEPSTONE: [f32; 3] = [0.30, 0.35, 0.39];

/// La pierre profonde : plus sombre, bleutée, avec des stries horizontales.
fn deepstone() -> Image {
    let base = DEEPSTONE;
    let mut image = Image::filled(base).noise(base, 0.18, 30);
    for (x, y, len) in [(0, 2, 8), (10, 5, 6), (3, 9, 10), (12, 14, 7)] {
        for k in 0..len {
            image.set((x + k) % SIZE, y, scale(base, 0.75));
            image.set((x + k) % SIZE, (y + 1) % SIZE, scale(base, 1.12));
        }
    }
    image
}

/// Le socle : des taches très sombres et très claires mélangées.
fn bedrock() -> Image {
    let base = [0.25, 0.25, 0.25];
    Image::filled(base)
        .noise(base, 0.2, 32)
        .speckles([0.08, 0.08, 0.08], 0.3, 33)
        .speckles([0.5, 0.5, 0.5], 0.12, 34)
}

/// Le gravier : des cailloux de plusieurs gris.
fn gravel() -> Image {
    let base = [0.49, 0.49, 0.45];
    let mut image = Image::filled(scale(base, 0.68));
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = (y / 3 % 2) * 2;
            let gx = (x + offset) % SIZE;
            let shade = 0.80 + (texel_hash(gx / 3, y / 3, 36) % 5) as f32 * 0.08;
            if !gx.is_multiple_of(3) && y % 3 != 0 {
                image.set(
                    x,
                    y,
                    scale(base, shade + if y % 3 == 1 { 0.12 } else { 0.0 }),
                );
            }
        }
    }
    image
}

fn clay() -> Image {
    let base = [0.62, 0.65, 0.7];
    Image::filled(base)
        .noise(base, 0.04, 37)
        .speckles([0.56, 0.6, 0.66], 0.1, 38)
}

/// Le grès : du sable tassé en bandes horizontales.
fn sandstone() -> Image {
    let base = [0.85, 0.77, 0.55];
    let mut image = Image::filled(base).noise(base, 0.04, 39);
    for x in 0..SIZE {
        for y in [0, 4, 11] {
            image.set(x, y, scale(base, 0.88));
        }
    }
    image
}

fn sandstone_top() -> Image {
    let base = [0.86, 0.79, 0.58];
    Image::filled(base)
        .noise(base, 0.035, 40)
        .speckles([0.8, 0.72, 0.5], 0.06, 41)
}

fn ice() -> Image {
    let base = [0.62, 0.78, 0.98];
    let mut image = Image::filled(base).noise(base, 0.04, 42);
    // Quelques fissures blanches.
    for i in 2..9 {
        image.set(i, i + 2, [0.9, 0.95, 1.0]);
        image.set(14 - i, i + 5, [0.85, 0.92, 1.0]);
    }
    image
}

fn lava() -> Image {
    let mut image = Image::filled([0.70, 0.23, 0.10]);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let bend = [0, 1, 2, 2, 1, 0, -1, -1][y % 8];
            let channel = (x as i32 + bend).rem_euclid(8);
            let color = match channel {
                0 | 1 => [0.98, 0.77, 0.31],
                2 | 3 => [0.94, 0.49, 0.16],
                4 | 5 => [0.82, 0.30, 0.10],
                _ => [0.60, 0.19, 0.12],
            };
            image.set(x, y, color);
        }
    }
    image
}

/// Un minerai : de la pierre (ou de la pierre profonde) et des grains colorés en amas.
fn ore(color: [f32; 3], seed: u64) -> Image {
    let mut image = stone();
    // Chaque gisement possède sa disposition ; les amas sont de petits cristaux biseautés.
    let mut rng = Rng::new(seed);
    for (cx, cy) in [(3, 3), (10, 2), (6, 9), (12, 12)] {
        let x = cx + (rng.next_u64() % 2) as usize;
        let y = cy + (rng.next_u64() % 2) as usize;
        for (dx, dy, shade) in [
            (0, 0, 1.2),
            (1, 0, 1.1),
            (0, 1, 1.0),
            (1, 1, 0.92),
            (2, 1, 0.8),
            (1, 2, 0.7),
        ] {
            image.set(x + dx, y + dy, scale(color, shade));
        }
        image.set(x, y + 2, scale(STONE, 0.63));
        image.set(x + 2, y + 2, scale(STONE, 0.72));
    }
    image
}

// ----- La végétation (phase O2) -----

/// L'écorce : des stries verticales irrégulières.
fn log_side(base: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(base).noise(base, 0.17, seed);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let bend = ((y / 4 + x / 5) % 3) as i32 - 1;
            let lane = (x as i32 + bend).rem_euclid(5);
            if lane == 0 {
                image.set(x, y, scale(base, 0.66));
            } else if lane == 1 {
                image.set(x, y, scale(base, 1.18));
            }
        }
    }
    for (x, y) in [(3, 5), (11, 11)] {
        for dy in 0..3 {
            image.set(x, y + dy, scale(base, 0.55));
        }
        image.set(x + 1, y + 1, scale(base, 0.82));
    }
    image
}

/// Le bouleau : une écorce blanche avec des taches noires horizontales.
fn birch_side() -> Image {
    let base = [0.86, 0.85, 0.8];
    let mut image = Image::filled(base).noise(base, 0.04, 60);
    let mut rng = Rng::new(61);
    for _ in 0..7 {
        let (x, y) = (
            (rng.next_u64() % 14) as usize,
            (rng.next_u64() % 16) as usize,
        );
        for dx in 0..2 + (rng.next_u64() % 3) as usize {
            image.set((x + dx).min(15), y, [0.15, 0.15, 0.14]);
        }
    }
    image
}

/// Le dessus d'une bûche : des cernes autour du cœur.
fn log_top(bark: [f32; 3], wood: [f32; 3]) -> Image {
    let mut image = Image::filled(bark).noise(bark, 0.10, 89);
    for y in 1..15 {
        for x in 1..15 {
            let dx = (x as i32 - 7).abs();
            let dy = (y as i32 - 8).abs();
            let radius = dx.max(dy) + dx.min(dy) / 2;
            let factor = if radius % 3 == 0 {
                0.78
            } else if radius % 3 == 1 {
                1.08
            } else {
                0.97
            };
            if radius < 8 {
                image.set(x, y, scale(wood, factor));
            }
        }
    }
    for (x, y) in [(7, 8), (8, 8), (8, 9)] {
        image.set(x, y, scale(wood, 0.62));
    }
    image
}

/// Des planches : quatre lattes horizontales.
fn planks(wood: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(wood).noise(wood, 0.08, seed);
    for y in 0..SIZE {
        let (course, yy, height) = if y < 5 {
            (0, y, 5)
        } else if y < 11 {
            (1, y - 5, 6)
        } else {
            (2, y - 11, 5)
        };
        let joint = [6, 12, 3][course];
        for x in 0..SIZE {
            let factor = if yy == height - 1 {
                0.67
            } else if yy == 0 {
                1.15
            } else if x == joint {
                0.78
            } else {
                1.0
            };
            if factor != 1.0 {
                image.set(x, y, scale(wood, factor));
            }
        }
        if yy == 2 {
            let start = (texel_hash(course, 0, seed) % 12) as usize;
            for dx in 0..3 {
                image.set((start + dx) % SIZE, y, scale(wood, 0.90));
            }
        }
    }
    image
}

/// Des feuilles : la couleur avec du bruit, et des trous transparents.
fn leaves(base: [f32; 3], seed: u64) -> Image {
    let base = base.map(|c| (c * 1.08 + 0.035).min(1.0));
    let mut image = Image::filled(scale(base, 0.64));
    // Des bouquets décalés et imbriqués, sans grille visible entre les feuilles.
    for (cx, cy) in [
        (1, 2),
        (6, 1),
        (12, 3),
        (4, 6),
        (10, 7),
        (15, 8),
        (0, 11),
        (6, 12),
        (12, 13),
        (3, 15),
    ] {
        let light = 0.94 + (texel_hash(cx, cy, seed) % 4) as f32 * 0.09;
        for dy in -2i32..=2 {
            for dx in -3i32..=3 {
                if dx.abs() + dy.abs() * 2 > 5 {
                    continue;
                }
                let x = (cx as i32 + dx).rem_euclid(16) as usize;
                let y = (cy as i32 + dy).rem_euclid(16) as usize;
                let bevel = if dy < 0 {
                    0.10
                } else if dy > 1 {
                    -0.18
                } else {
                    0.0
                };
                image.set(x, y, scale(base, light + bevel));
            }
        }
    }
    for y in 0..SIZE {
        for x in 0..SIZE {
            if texel_hash(x, y, seed + 71).is_multiple_of(11) {
                image.clear(x, y);
            }
        }
    }
    image
}

/// Une petite pousse : une tige et quelques feuilles.
fn sapling(stem: [f32; 3], leaf: [f32; 3]) -> Image {
    let mut image = Image::transparent();
    for y in 8..16 {
        image.set(7, y, stem);
    }
    for (x, y) in [
        (5, 7),
        (6, 6),
        (7, 5),
        (8, 6),
        (9, 7),
        (6, 8),
        (8, 8),
        (7, 4),
        (4, 9),
        (10, 9),
        (7, 7),
    ] {
        image.plot(x, y, leaf);
    }
    for (x, y) in [(6, 3), (8, 3), (5, 5), (9, 5), (3, 8), (11, 8)] {
        image.plot(x, y, scale(leaf, 0.8));
    }
    image
}

/// Une culture à un stade (0..=3) : des tiges qui montent, puis l'épi ou le fane de carotte.
fn crop(kind: &str, stage: usize) -> Image {
    let mut image = Image::transparent();
    let height = 4 + stage as i32 * 3;
    let ripe = stage == 3;
    for (i, x) in (2..15).step_by(3).enumerate() {
        let h = height + (i as i32 % 2);
        for k in 0..h {
            let green = if kind == "wheat" && ripe {
                [0.82, 0.68, 0.25]
            } else {
                [0.3 + 0.03 * stage as f32, 0.62, 0.2]
            };
            image.plot(x, 15 - k, scale(green, 0.85 + 0.05 * (k % 3) as f32));
            if kind == "carrot" && stage > 0 && k % 3 == 2 {
                image.plot(x - 1, 15 - k, green);
                image.plot(x + 1, 15 - k, green);
            }
        }
        if kind == "wheat" && stage >= 2 {
            let ear = if ripe {
                [0.92, 0.78, 0.3]
            } else {
                [0.6, 0.7, 0.25]
            };
            image.plot(x, 15 - h, ear);
            image.plot(x, 14 - h, ear);
            image.plot(x - 1, 14 - h, scale(ear, 0.85));
        }
        if kind == "carrot" && ripe {
            image.plot(x, 15, [0.95, 0.5, 0.1]);
        }
    }
    image
}

/// Une pomme de terre, un chou ou une pastèque à un stade (0..=3).
fn ground_crop(kind: &str, stage: usize) -> Image {
    let mut image = Image::transparent();
    let leaf = |k: f32| match kind {
        "cabbage" => [0.4 * k, 0.68 * k, 0.32 * k],
        "melon" => [0.22 * k, 0.55 * k, 0.2 * k],
        _ => [0.28 * k, 0.55 * k, 0.2 * k],
    };
    match kind {
        // Un chou : une rosette de feuilles qui s'élargit et se referme en pomme.
        "cabbage" => {
            let r = 2 + stage as i32 * 2;
            for dy in 0..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy * 2 <= r * r + 2 {
                        let shade = 0.8 + 0.1 * ((dx + dy).rem_euclid(3)) as f32;
                        let c = if stage == 3 && dy > r / 2 {
                            [0.6 * shade, 0.82 * shade, 0.5 * shade]
                        } else {
                            leaf(shade)
                        };
                        image.plot(8 + dx, 15 - dy, c);
                    }
                }
            }
        }
        // Une pastèque : une liane qui rampe, puis un gros fruit rayé.
        "melon" => {
            let len = 4 + stage as i32 * 3;
            for x in 0..len {
                image.plot(2 + x, 15 - (x % 3 == 1) as i32, leaf(0.9));
                if x % 3 == 0 {
                    image.plot(2 + x, 13, leaf(1.1));
                    image.plot(3 + x, 12, leaf(1.0));
                }
            }
            if stage == 3 {
                for dy in 0..6 {
                    for dx in 0..9 {
                        let stripe = dx % 3 == 1;
                        image.plot(
                            4 + dx,
                            15 - dy,
                            if stripe {
                                [0.12, 0.38, 0.14]
                            } else {
                                [0.25, 0.62, 0.25]
                            },
                        );
                    }
                }
            }
        }
        // Une pomme de terre : un buisson de feuilles, avec des fleurs blanches à maturité.
        _ => {
            let h = 3 + stage as i32 * 2;
            for x in (3..14).step_by(2) {
                for k in 0..h + x % 3 {
                    image.plot(x, 15 - k, leaf(0.8 + 0.08 * (k % 3) as f32));
                    if k > 1 && k % 2 == 0 {
                        image.plot(x - 1, 15 - k, leaf(1.0));
                    }
                }
                if stage == 3 {
                    image.plot(x, 15 - h - 1, [0.95, 0.95, 0.9]);
                }
            }
        }
    }
    image
}

/// Des herbes : des brins de hauteurs variées.
fn tall_grass(color: [f32; 3], seed: u64) -> Image {
    let mut image = Image::transparent();
    let mut rng = Rng::new(seed);
    for x in (1..15).step_by(2) {
        let height = 6 + (rng.next_u64() % 9) as i32;
        let lean = rng.signed();
        for k in 0..height {
            let px = x + (lean * k as f32 / 4.0) as i32;
            image.plot(px, 15 - k, scale(color, 0.85 + 0.25 * rng.next_f32()));
        }
    }
    image
}

fn fern() -> Image {
    let color = [0.38, 0.62, 0.3];
    let mut image = Image::transparent();
    for (cx, lean) in [(4, -1), (8, 0), (12, 1)] {
        for k in 0..12 {
            let x = cx + lean * k / 4;
            image.plot(x, 15 - k, color);
            if k % 2 == 0 && k > 2 {
                image.plot(x - 1, 15 - k, scale(color, 0.85));
                image.plot(x + 1, 15 - k, scale(color, 0.85));
            }
        }
    }
    image
}

fn dead_bush() -> Image {
    let color = [0.5, 0.36, 0.2];
    let mut image = Image::transparent();
    for (dx, dy) in [(-1, -1), (1, -1), (0, -1), (-1, -2), (1, -2)] {
        let (mut x, mut y) = (7, 15);
        for _ in 0..7 {
            image.plot(x, y, color);
            x += dx;
            y += dy;
        }
    }
    image
}

/// Une fleur : une tige verte et une corolle de la couleur donnée, cœur jaune.
fn flower(petals: [f32; 3], heart: [f32; 3]) -> Image {
    let stem = [0.25, 0.55, 0.2];
    let mut image = Image::transparent();
    for y in 7..16 {
        image.set(7, y, stem);
    }
    image.set(6, 11, stem);
    image.set(5, 10, stem);
    image.set(8, 12, stem);
    image.set(9, 11, stem);
    for (x, y) in [
        (7, 3),
        (6, 4),
        (8, 4),
        (5, 5),
        (9, 5),
        (6, 6),
        (8, 6),
        (7, 7),
        (5, 4),
        (9, 4),
    ] {
        image.set(x, y, petals);
    }
    image.set(7, 5, heart);
    image
}

fn mushroom(cap: [f32; 3], dots: bool) -> Image {
    let stem = [0.88, 0.85, 0.78];
    let mut image = Image::transparent();
    for y in 9..16 {
        image.set(7, y, stem);
        image.set(8, y, stem);
    }
    for y in 5..9 {
        let half = (y - 3).min(4);
        for x in 8 - half..8 + half {
            image.plot(x, y, cap);
        }
    }
    if dots {
        for (x, y) in [(5, 7), (8, 6), (10, 7), (7, 8)] {
            image.set(x, y, [0.95, 0.95, 0.92]);
        }
    }
    image
}

/// Des lianes : des brins qui pendent.
fn vine() -> Image {
    let color = [0.3, 0.55, 0.2];
    let mut image = Image::transparent();
    let mut rng = Rng::new(70);
    for x in [1, 5, 9, 13] {
        let length = 8 + (rng.next_u64() % 8) as usize;
        for y in 0..length {
            image.plot(x + (y as i32 / 5) % 2, y as i32, color);
            if rng.next_f32() < 0.3 {
                image.plot(x + 1, y as i32, scale(color, 0.8));
            }
        }
    }
    image
}

fn seaweed() -> Image {
    let color = [0.25, 0.55, 0.3];
    let mut image = Image::transparent();
    for x0 in [3, 8, 12] {
        for y in 0..16 {
            let x = x0 + ((y as f32 * 0.6).sin() * 1.5) as i32;
            image.plot(x, y, scale(color, 0.85 + 0.01 * y as f32));
            image.plot(x + 1, y, color);
        }
    }
    image
}

/// Un nénuphar : un disque vert avec une encoche, vu de dessus.
fn lily_pad() -> Image {
    let color = [0.22, 0.55, 0.22];
    let mut image = Image::transparent();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let notch = dx > 0.0 && dy.abs() < dx * 0.35;
            if dx * dx + dy * dy < 49.0 && !notch {
                let vein = (dx.abs() < 0.6 || dy.abs() < 0.6) as u8 as f32;
                image.set(x, y, scale(color, 1.0 - 0.15 * vein));
            }
        }
    }
    image
}

fn cactus_side() -> Image {
    let base = [0.3, 0.55, 0.25];
    let mut image = Image::filled(base).noise(base, 0.06, 80);
    for y in 0..SIZE {
        for x in [0, 15] {
            image.set(x, y, scale(base, 0.75));
        }
        for x in [4, 11] {
            image.set(x, y, scale(base, 0.85));
        }
    }
    for (x, y) in [(2, 3), (6, 7), (9, 2), (13, 10), (2, 12), (7, 14)] {
        image.set(x, y, [0.15, 0.15, 0.12]);
    }
    image
}

fn cactus_top() -> Image {
    let base = [0.38, 0.62, 0.3];
    let mut image = Image::filled(base).noise(base, 0.05, 81);
    for i in 0..SIZE {
        for (x, y) in [(i, 0), (i, 15), (0, i), (15, i)] {
            image.set(x, y, scale(base, 0.75));
        }
    }
    image
}

// ----- Objets et artisanat (phase O3) -----

fn cobblestone() -> Image {
    let mortar = [0.29, 0.34, 0.35];
    let mut image = Image::filled(mortar);
    for y in 0..SIZE {
        let (row, yy, height) = if y < 5 {
            (0, y, 5)
        } else if y < 11 {
            (1, y - 5, 6)
        } else {
            (2, y - 11, 5)
        };
        for x in 0..SIZE {
            let xx = (x + [0, 3, 8][row]) % SIZE;
            let (col, dx, width) = if xx < 6 {
                (0, xx, 6)
            } else if xx < 11 {
                (1, xx - 6, 5)
            } else {
                (2, xx - 11, 5)
            };
            if dx == width - 1 || yy == height - 1 || (dx == 0 && yy == 0) {
                continue;
            }
            let factor = 0.89 + (texel_hash(col, row, 110) % 3) as f32 * 0.07;
            let bevel = if yy == 0 || dx == 0 {
                0.12
            } else if yy == height - 2 {
                -0.10
            } else {
                0.0
            };
            image.set(x, y, scale(STONE, factor + bevel));
        }
    }
    image
}

/// La roche volcanique : très sombre, avec des bulles et quelques reflets rougeâtres.
fn volcanic_rock() -> Image {
    let base = [0.17, 0.15, 0.16];
    Image::filled(base)
        .noise(base, 0.15, 117)
        .speckles([0.08, 0.07, 0.08], 0.2, 118)
        .speckles([0.35, 0.18, 0.14], 0.05, 119)
}

fn workbench_top() -> Image {
    let wood = [0.61, 0.44, 0.29];
    let mut image = planks(wood, 112);
    // Une planche de travail marquée, une règle et deux fixations en cuivre.
    for y in 2..14 {
        for x in 3..13 {
            image.set(x, y, [0.74, 0.60, 0.41]);
        }
    }
    for y in 2..14 {
        image.set(3, y, scale(wood, 0.65));
    }
    for x in 3..13 {
        image.set(x, 13, scale(wood, 0.65));
    }
    for x in 5..12 {
        image.set(x, 4, INK);
        if x % 2 == 0 {
            image.set(x, 5, INK);
        }
    }
    for k in 0..5 {
        image.set(6 + k, 11 - k, scale(wood, 0.8));
    }
    for (x, y) in [(1, 1), (14, 14)] {
        image.set(x, y, COPPER);
    }
    image
}

fn workbench_side() -> Image {
    let wood = [0.55, 0.4, 0.24];
    let mut image = planks(wood, 113);
    // Des outils accrochés : une scie et un marteau.
    for x in 3..8 {
        image.set(x, 5, [0.75, 0.75, 0.78]);
    }
    for y in 3..9 {
        image.set(11, y, [0.4, 0.28, 0.15]);
    }
    for x in 9..14 {
        image.set(x, 3, [0.6, 0.6, 0.62]);
    }
    image
}

fn furnace_front() -> Image {
    let mut image = cobblestone();
    for y in 1..6 {
        for x in 4..12 {
            image.set(x, y, INK);
        }
    }
    for x in [5, 8, 11] {
        for y in 2..5 {
            image.set(x, y, scale(IRON, 0.65));
        }
    }
    for x in 3..13 {
        image.set(x, 7, COPPER);
        image.set(x, 14, scale(STONE, 0.64));
    }
    for y in 8..14 {
        for x in 4..12 {
            image.set(x, y, [0.10, 0.12, 0.13]);
        }
    }
    // La bouche garde exactement ses coordonnées : le shader anime les flammes ici.
    for (x, y) in [(2, 7), (13, 7)] {
        image.set(x, y, scale(IRON, 1.05));
    }
    image
}

fn furnace_top() -> Image {
    let base = scale(STONE, 0.85);
    Image::filled(base).noise(base, 0.06, 114)
}

fn chest_side() -> Image {
    let wood = [0.51, 0.36, 0.23];
    let mut image = planks(wood, 115);
    for x in 0..SIZE {
        image.set(x, 5, INK);
        image.set(x, 6, scale(wood, 1.20));
    }
    for x in [2, 12] {
        for y in 0..SIZE {
            image.set(x, y, scale(COPPER, 0.72));
            image.set(x + 1, y, COPPER);
        }
    }
    // Fermoir en losange, décentré des sangles.
    for y in 5..10 {
        for x in 6..10 {
            if (x as i32 - 7).abs() + (y as i32 - 7).abs() <= 2 {
                image.set(x, y, [0.87, 0.68, 0.36]);
            }
        }
    }
    image.set(7, 7, INK);
    image
}

fn chest_top() -> Image {
    let wood = [0.56, 0.40, 0.26];
    let mut image = planks(wood, 116);
    for x in [2, 12] {
        for y in 0..SIZE {
            image.set(x, y, scale(COPPER, 0.72));
            image.set(x + 1, y, COPPER);
        }
    }
    for (x, y) in [(3, 2), (13, 2), (3, 13), (13, 13)] {
        image.set(x, y, scale(IRON, 1.15));
    }
    image
}

/// Le matelas du lit de feuillage : un vert tacheté.
fn mattress_pixel(x: usize, y: usize) -> [f32; 3] {
    let base = [0.30, 0.49, 0.47];
    if y.is_multiple_of(6) {
        [0.75, 0.70, 0.49]
    } else if (x / 2 + y / 2).is_multiple_of(2) {
        scale(base, 1.08)
    } else {
        base
    }
}

/// Le dessus du lit : le matelas, un oreiller pâle en haut, le cadre en bois.
fn bed_top() -> Image {
    let wood = [0.6, 0.42, 0.22];
    let mut image = Image::filled(wood);
    for y in 1..SIZE - 1 {
        for x in 1..SIZE - 1 {
            let pillow = (2..=4).contains(&y) && (3..=12).contains(&x);
            let color = if pillow {
                if y == 4 {
                    [0.82, 0.8, 0.72]
                } else {
                    [0.93, 0.91, 0.84]
                }
            } else {
                mattress_pixel(x, y)
            };
            image.set(x, y, color);
        }
    }
    image
}

/// Le côté du lit : le matelas en haut, le cadre et les pieds en bas.
fn bed_side() -> Image {
    let wood = [0.6, 0.42, 0.22];
    let mut image = planks(wood, 117);
    for y in 0..7 {
        for x in 0..SIZE {
            image.set(x, y, mattress_pixel(x, y));
        }
    }
    for x in 0..SIZE {
        image.set(x, 7, scale(wood, 0.6));
    }
    // L'ombre sous le lit, entre les deux pieds.
    for y in 12..SIZE {
        for x in 2..SIZE - 2 {
            image.set(x, y, scale(wood, 0.3));
        }
    }
    image
}

/// Écrit toutes les textures ; renvoie les chemins.
pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let core = root.join("assets/core/textures");
    let textures: Vec<(PathBuf, Image)> = vec![
        (core.join("grass_top.png"), grass_top()),
        (core.join("grass_side.png"), grass_side()),
        (core.join("dirt.png"), dirt()),
        (core.join("stone.png"), stone()),
        (core.join("sand.png"), sand()),
        (core.join("snow.png"), snow()),
        (core.join("water.png"), water()),
        (core.join("glass.png"), glass()),
        (core.join("torch.png"), torch()),
        (core.join("glowstone.png"), glowstone()),
        (core.join("clock.png"), clock()),
        (core.join("lamp.png"), lamp(false)),
        (core.join("lamp_lit.png"), lamp(true)),
        (core.join("deepstone.png"), deepstone()),
        (core.join("bedrock.png"), bedrock()),
        (core.join("gravel.png"), gravel()),
        (core.join("clay.png"), clay()),
        (core.join("sandstone.png"), sandstone()),
        (core.join("sandstone_top.png"), sandstone_top()),
        (core.join("ice.png"), ice()),
        (core.join("lava.png"), lava()),
        (core.join("coal_ore.png"), ore([0.12, 0.12, 0.12], 50)),
        (core.join("iron_ore.png"), ore([0.82, 0.68, 0.55], 51)),
        (core.join("copper_ore.png"), ore([0.85, 0.5, 0.3], 52)),
        (core.join("gold_ore.png"), ore([0.98, 0.85, 0.25], 53)),
        (core.join("cinnabar_ore.png"), ore([0.85, 0.12, 0.12], 54)),
        (core.join("malachite_ore.png"), ore([0.2, 0.7, 0.45], 55)),
        (core.join("sapphire_ore.png"), ore([0.3, 0.5, 1.0], 56)),
        (core.join("jade_ore.png"), ore([0.4, 0.9, 0.6], 57)),
        (core.join("oak_log.png"), log_side([0.42, 0.31, 0.18], 90)),
        (
            core.join("oak_log_top.png"),
            log_top([0.42, 0.31, 0.18], [0.7, 0.56, 0.35]),
        ),
        (core.join("oak_leaves.png"), leaves([0.3, 0.55, 0.2], 91)),
        (core.join("oak_planks.png"), planks([0.68, 0.53, 0.32], 92)),
        (
            core.join("oak_sapling.png"),
            sapling([0.42, 0.31, 0.18], [0.3, 0.6, 0.2]),
        ),
        (core.join("birch_log.png"), birch_side()),
        (
            core.join("birch_log_top.png"),
            log_top([0.86, 0.85, 0.8], [0.82, 0.72, 0.5]),
        ),
        (core.join("birch_leaves.png"), leaves([0.5, 0.66, 0.3], 93)),
        (
            core.join("birch_planks.png"),
            planks([0.82, 0.74, 0.52], 94),
        ),
        (
            core.join("birch_sapling.png"),
            sapling([0.85, 0.85, 0.8], [0.5, 0.7, 0.3]),
        ),
        (core.join("spruce_log.png"), log_side([0.3, 0.21, 0.12], 95)),
        (
            core.join("spruce_log_top.png"),
            log_top([0.3, 0.21, 0.12], [0.52, 0.4, 0.25]),
        ),
        (
            core.join("spruce_leaves.png"),
            leaves([0.2, 0.38, 0.25], 96),
        ),
        (
            core.join("spruce_planks.png"),
            planks([0.45, 0.33, 0.2], 97),
        ),
        (
            core.join("spruce_sapling.png"),
            sapling([0.3, 0.21, 0.12], [0.2, 0.42, 0.25]),
        ),
        (
            core.join("acacia_log.png"),
            log_side([0.45, 0.42, 0.38], 98),
        ),
        (
            core.join("acacia_log_top.png"),
            log_top([0.45, 0.42, 0.38], [0.75, 0.42, 0.22]),
        ),
        (
            core.join("acacia_leaves.png"),
            leaves([0.42, 0.58, 0.2], 99),
        ),
        (
            core.join("acacia_planks.png"),
            planks([0.75, 0.42, 0.24], 100),
        ),
        (
            core.join("acacia_sapling.png"),
            sapling([0.45, 0.42, 0.38], [0.45, 0.6, 0.2]),
        ),
        (
            core.join("jungle_log.png"),
            log_side([0.38, 0.3, 0.16], 101),
        ),
        (
            core.join("jungle_log_top.png"),
            log_top([0.38, 0.3, 0.16], [0.65, 0.48, 0.3]),
        ),
        (
            core.join("jungle_leaves.png"),
            leaves([0.2, 0.6, 0.15], 102),
        ),
        (
            core.join("jungle_planks.png"),
            planks([0.62, 0.45, 0.3], 103),
        ),
        (
            core.join("jungle_sapling.png"),
            sapling([0.38, 0.3, 0.16], [0.2, 0.62, 0.15]),
        ),
        (
            core.join("tall_grass.png"),
            tall_grass([0.4, 0.66, 0.26], 104),
        ),
        (core.join("fern.png"), fern()),
        (core.join("dead_bush.png"), dead_bush()),
        (
            core.join("poppy.png"),
            flower([0.85, 0.12, 0.1], [0.2, 0.1, 0.05]),
        ),
        (
            core.join("dandelion.png"),
            flower([0.98, 0.85, 0.15], [0.9, 0.6, 0.1]),
        ),
        (
            core.join("cornflower.png"),
            flower([0.3, 0.45, 0.95], [0.95, 0.9, 0.5]),
        ),
        (
            core.join("daisy.png"),
            flower([0.97, 0.97, 0.94], [0.95, 0.8, 0.2]),
        ),
        (
            core.join("lavender.png"),
            flower([0.65, 0.42, 0.88], [0.8, 0.6, 0.95]),
        ),
        (
            core.join("red_mushroom.png"),
            mushroom([0.8, 0.12, 0.1], true),
        ),
        (
            core.join("brown_mushroom.png"),
            mushroom([0.55, 0.38, 0.25], false),
        ),
        (core.join("vine.png"), vine()),
        (core.join("seaweed.png"), seaweed()),
        (core.join("lily_pad.png"), lily_pad()),
        (core.join("cactus_side.png"), cactus_side()),
        (core.join("cactus_top.png"), cactus_top()),
        (core.join("wheat_0.png"), crop("wheat", 0)),
        (core.join("wheat_1.png"), crop("wheat", 1)),
        (core.join("wheat_2.png"), crop("wheat", 2)),
        (core.join("wheat_3.png"), crop("wheat", 3)),
        (core.join("carrot_0.png"), crop("carrot", 0)),
        (core.join("carrot_1.png"), crop("carrot", 1)),
        (core.join("carrot_2.png"), crop("carrot", 2)),
        (core.join("carrot_3.png"), crop("carrot", 3)),
        (core.join("potato_0.png"), ground_crop("potato", 0)),
        (core.join("potato_1.png"), ground_crop("potato", 1)),
        (core.join("potato_2.png"), ground_crop("potato", 2)),
        (core.join("potato_3.png"), ground_crop("potato", 3)),
        (core.join("cabbage_0.png"), ground_crop("cabbage", 0)),
        (core.join("cabbage_1.png"), ground_crop("cabbage", 1)),
        (core.join("cabbage_2.png"), ground_crop("cabbage", 2)),
        (core.join("cabbage_3.png"), ground_crop("cabbage", 3)),
        (core.join("melon_0.png"), ground_crop("melon", 0)),
        (core.join("melon_1.png"), ground_crop("melon", 1)),
        (core.join("melon_2.png"), ground_crop("melon", 2)),
        (core.join("melon_3.png"), ground_crop("melon", 3)),
        (core.join("enchanting_table_top.png"), enchanting_top()),
        (core.join("enchanting_table_side.png"), enchanting_side()),
        (core.join("brewing_stand_top.png"), brewing_top()),
        (core.join("brewing_stand_side.png"), brewing_side()),
        (core.join("cable.png"), tech("cable", false)),
        (core.join("pipe.png"), tech("pipe", false)),
        (core.join("shaft.png"), tech("shaft", false)),
        (core.join("solar_panel.png"), tech("solar_panel", false)),
        (core.join("battery.png"), tech("battery", false)),
        (core.join("power_lamp.png"), tech("power_lamp", false)),
        (core.join("power_lamp_on.png"), tech("power_lamp", true)),
        (core.join("pump.png"), tech("pump", false)),
        (core.join("tank.png"), tech("tank", false)),
        (core.join("filler.png"), tech("filler", false)),
        (core.join("motor.png"), tech("motor", false)),
        (core.join("fan.png"), tech("fan", false)),
        (core.join("fan_on.png"), tech("fan", true)),
        (core.join("assembler.png"), tech("assembler", false)),
        (core.join("balloon.png"), tech("balloon", false)),
        (core.join("propeller.png"), tech("propeller", false)),
        (core.join("helm.png"), tech("helm", false)),
        (core.join("wire.png"), wire(false)),
        (core.join("wire_on.png"), wire(true)),
        (core.join("lever.png"), lever(false)),
        (core.join("lever_on.png"), lever(true)),
        (core.join("button.png"), button(false)),
        (core.join("button_on.png"), button(true)),
        (core.join("gate.png"), gate(false)),
        (core.join("gate_open.png"), gate(true)),
        (core.join("inverter.png"), logic(1, false)),
        (core.join("inverter_on.png"), logic(1, true)),
        (core.join("repeater.png"), logic(2, false)),
        (core.join("repeater_on.png"), logic(2, true)),
        (core.join("piston.png"), piston(false, [0.78, 0.74, 0.7])),
        (core.join("piston_on.png"), piston(true, [0.78, 0.74, 0.7])),
        (
            core.join("sticky_piston.png"),
            piston(false, [0.45, 0.75, 0.3]),
        ),
        (
            core.join("sticky_piston_on.png"),
            piston(true, [0.45, 0.75, 0.3]),
        ),
        (core.join("piston_head.png"), piston_head()),
        (core.join("farmland_dry.png"), farmland(false)),
        (core.join("farmland_wet.png"), farmland(true)),
        (core.join("cobblestone.png"), cobblestone()),
        (core.join("volcanic_rock.png"), volcanic_rock()),
        (core.join("workbench_top.png"), workbench_top()),
        (core.join("workbench_side.png"), workbench_side()),
        (core.join("furnace_front.png"), furnace_front()),
        (core.join("furnace_top.png"), furnace_top()),
        (core.join("chest_side.png"), chest_side()),
        (core.join("chest_top.png"), chest_top()),
        (core.join("bed_top.png"), bed_top()),
        (core.join("bed_side.png"), bed_side()),
        (root.join("mods/example/textures/breaker.png"), breaker()),
        (root.join("mods/example/textures/ruby.png"), ruby()),
    ];
    let mut textures = textures;
    textures.extend(item_icons(&root.join("assets/core/items")));
    for kind in ["shaft", "gear", "crank", "press", "belt"] {
        textures.push((
            root.join(format!("mods/mecanique/textures/{kind}.png")),
            mechanism(kind),
        ));
    }
    for (name, color) in [("iron_plate", IRON), ("copper_plate", COPPER)] {
        textures.push((
            root.join(format!("mods/mecanique/items/{name}.png")),
            metal_plate(color),
        ));
    }
    for (path, image) in &textures {
        image.save(path)?;
    }
    Ok(textures.into_iter().map(|(path, _)| path).collect())
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    #[test]
    fn ground_edges_match_after_palette_quantization() {
        for image in [grass_top(), dirt()] {
            for i in 0..SIZE {
                assert_eq!(
                    finish_pixel(image.pixels[i * SIZE]),
                    finish_pixel(image.pixels[i * SIZE + SIZE - 1])
                );
                assert_eq!(
                    finish_pixel(image.pixels[i]),
                    finish_pixel(image.pixels[i + SIZE * (SIZE - 1)])
                );
            }
            assert_ne!(
                image.pixels[0],
                image.pixels[5 + 5 * SIZE],
                "la matière ne doit pas être un aplat"
            );
        }
        let side = grass_side();
        for y in 0..SIZE {
            assert_eq!(
                finish_pixel(side.pixels[y * SIZE]),
                finish_pixel(side.pixels[y * SIZE + SIZE - 1])
            );
        }
    }
}
