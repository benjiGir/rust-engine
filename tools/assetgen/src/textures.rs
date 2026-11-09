//! Les textures 16 × 16 : une couleur de base, du bruit, et un motif par bloc.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Rng;

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

    /// Chaque pixel varie un peu en luminosité : l'aspect « naturel ».
    fn noise(mut self, base: [f32; 3], amount: f32, seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let shade = 1.0 + rng.signed() * amount;
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
        let bytes: Vec<u8> = self.pixels.iter().flatten().copied().collect();
        encoder.write_header()?.write_image_data(&bytes)?;
        Ok(())
    }
}

fn rgba(color: [f32; 3]) -> [u8; 4] {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [c(color[0]), c(color[1]), c(color[2]), 255]
}

fn scale(color: [f32; 3], factor: f32) -> [f32; 3] {
    color.map(|c| c * factor)
}

const GRASS: [f32; 3] = [0.38, 0.62, 0.26];
const DIRT: [f32; 3] = [0.47, 0.33, 0.21];
const STONE: [f32; 3] = [0.52, 0.52, 0.54];

fn grass_top() -> Image {
    Image::filled(GRASS)
        .noise(GRASS, 0.12, 1)
        .speckles([0.3, 0.52, 0.2], 0.12, 2)
        .speckles([0.48, 0.7, 0.32], 0.08, 3)
}

fn dirt() -> Image {
    Image::filled(DIRT)
        .noise(DIRT, 0.12, 4)
        .speckles([0.36, 0.25, 0.16], 0.15, 5)
        .speckles([0.58, 0.45, 0.32], 0.05, 6)
}

/// Le côté de l'herbe : de la terre, avec une bordure d'herbe irrégulière en haut.
fn grass_side() -> Image {
    let mut image = dirt();
    let top = grass_top();
    let mut rng = Rng::new(7);
    for x in 0..SIZE {
        // Entre 2 et 4 pixels d'herbe qui « débordent » sur la terre.
        let depth = 2 + (rng.next_u64() % 3) as usize;
        for y in 0..depth {
            image.pixels[x + y * SIZE] = top.pixels[x + y * SIZE];
        }
    }
    image
}

fn stone() -> Image {
    let mut image = Image::filled(STONE).noise(STONE, 0.08, 8);
    // Des fissures sombres : de courtes marches aléatoires.
    let mut rng = Rng::new(9);
    for _ in 0..5 {
        let (mut x, mut y) = ((rng.next_u64() % 16) as i32, (rng.next_u64() % 16) as i32);
        for _ in 0..4 {
            image.set(x as usize, y as usize, scale(STONE, 0.72));
            x = (x + (rng.next_u64() % 3) as i32 - 1).rem_euclid(16);
            y = (y + 1).rem_euclid(16);
        }
    }
    image
}

fn sand() -> Image {
    let base = [0.87, 0.8, 0.56];
    Image::filled(base)
        .noise(base, 0.05, 10)
        .speckles([0.78, 0.7, 0.48], 0.12, 11)
}

fn snow() -> Image {
    let base = [0.95, 0.96, 1.0];
    Image::filled(base)
        .noise(base, 0.025, 12)
        .speckles([0.85, 0.9, 1.0], 0.08, 13)
}

/// L'eau : un bleu uni avec quelques vaguelettes plus claires.
fn water() -> Image {
    let base = [0.2, 0.4, 0.8];
    let mut image = Image::filled(base).noise(base, 0.05, 14);
    for (y, offset) in [(3, 2), (8, 9), (13, 4)] {
        for x in 0..5 {
            image.set((offset + x) % SIZE, y, [0.45, 0.65, 0.95]);
        }
    }
    image
}

/// Le verre : presque blanc, avec un cadre et deux reflets en diagonale.
fn glass() -> Image {
    let mut image = Image::filled([0.85, 0.93, 0.98]);
    for i in 0..SIZE {
        for (x, y) in [(i, 0), (i, SIZE - 1), (0, i), (SIZE - 1, i)] {
            image.set(x, y, [0.6, 0.75, 0.85]);
        }
    }
    for i in 3..7 {
        image.set(i, 9 - i, [1.0, 1.0, 1.0]);
        image.set(i + 4, 13 - i, [1.0, 1.0, 1.0]);
    }
    image
}

/// La torche : un bois sombre avec un cœur lumineux.
fn torch() -> Image {
    let mut image = Image::filled([0.42, 0.28, 0.15]).noise([0.42, 0.28, 0.15], 0.1, 15);
    for y in 3..13 {
        for x in 5..11 {
            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
            let glow = (1.0 - d / 6.0).clamp(0.0, 1.0);
            image.set(x, y, [1.0, 0.65 + 0.3 * glow, 0.2 + 0.5 * glow]);
        }
    }
    image
}

fn glowstone() -> Image {
    let base = [0.95, 0.8, 0.45];
    Image::filled(base)
        .noise(base, 0.1, 16)
        .speckles([1.0, 0.95, 0.75], 0.25, 17)
        .speckles([0.75, 0.55, 0.25], 0.1, 18)
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
        (root.join("mods/example/textures/breaker.png"), breaker()),
        (root.join("mods/example/textures/ruby.png"), ruby()),
    ];
    for (path, image) in &textures {
        image.save(path)?;
    }
    Ok(textures.into_iter().map(|(path, _)| path).collect())
}
