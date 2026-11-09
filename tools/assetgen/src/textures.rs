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

// ----- Le terrain (phase O1) -----

const DEEPSTONE: [f32; 3] = [0.3, 0.3, 0.34];

/// La pierre profonde : plus sombre, bleutée, avec des stries horizontales.
fn deepstone() -> Image {
    let mut image = Image::filled(DEEPSTONE).noise(DEEPSTONE, 0.07, 30);
    let mut rng = Rng::new(31);
    for y in [2, 6, 10, 14] {
        let start = (rng.next_u64() % 8) as usize;
        for x in start..(start + 6 + (rng.next_u64() % 6) as usize).min(SIZE) {
            image.set(x, y, scale(DEEPSTONE, 0.75));
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
    let base = [0.5, 0.48, 0.47];
    let mut image = Image::filled(base).noise(base, 0.1, 35);
    let mut rng = Rng::new(36);
    for _ in 0..14 {
        let (x, y) = (
            (rng.next_u64() % 15) as usize,
            (rng.next_u64() % 15) as usize,
        );
        let shade = [0.35, 0.62, 0.45, 0.55][(rng.next_u64() % 4) as usize];
        let color = [shade, shade * 0.97, shade * 0.95];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            image.set(x + dx, y + dy, color);
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
    let base = [0.95, 0.4, 0.05];
    Image::filled(base)
        .noise(base, 0.12, 43)
        .speckles([1.0, 0.75, 0.2], 0.2, 44)
        .speckles([0.7, 0.15, 0.02], 0.12, 45)
}

/// Un minerai : de la pierre (ou de la pierre profonde) et des grains colorés en amas.
fn ore(color: [f32; 3], seed: u64) -> Image {
    let mut image = stone();
    let mut rng = Rng::new(seed);
    for _ in 0..5 {
        let (cx, cy) = (
            2 + (rng.next_u64() % 12) as i32,
            2 + (rng.next_u64() % 12) as i32,
        );
        for _ in 0..4 {
            let x = (cx + (rng.next_u64() % 3) as i32 - 1).clamp(0, 15) as usize;
            let y = (cy + (rng.next_u64() % 3) as i32 - 1).clamp(0, 15) as usize;
            let shade = 0.85 + rng.next_f32() * 0.3;
            image.set(x, y, scale(color, shade));
        }
    }
    image
}

// ----- La végétation (phase O2) -----

/// L'écorce : des stries verticales irrégulières.
fn log_side(base: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(base).noise(base, 0.08, seed);
    let mut rng = Rng::new(seed + 1);
    for x in 0..SIZE {
        if rng.next_f32() < 0.45 {
            let start = (rng.next_u64() % 16) as usize;
            let length = 3 + (rng.next_u64() % 8) as usize;
            for y in start..(start + length).min(SIZE) {
                image.set(x, y, scale(base, 0.72));
            }
        }
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
    let mut image = Image::filled(bark);
    for y in 1..SIZE - 1 {
        for x in 1..SIZE - 1 {
            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
            let ring = if (d as i32) % 3 == 0 { 0.82 } else { 1.0 };
            image.set(x, y, scale(wood, ring));
        }
    }
    image
}

/// Des planches : quatre lattes horizontales.
fn planks(wood: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(wood).noise(wood, 0.06, seed);
    for x in 0..SIZE {
        for y in [3, 7, 11, 15] {
            image.set(x, y, scale(wood, 0.7));
        }
    }
    for (y0, x) in [(0, 5), (4, 12), (8, 3), (12, 9)] {
        for y in y0..y0 + 3 {
            image.set(x, y, scale(wood, 0.75));
        }
    }
    image
}

/// Des feuilles : la couleur avec du bruit, et des trous transparents.
fn leaves(base: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(base).noise(base, 0.18, seed);
    let mut rng = Rng::new(seed + 1);
    for y in 0..SIZE {
        for x in 0..SIZE {
            if rng.next_f32() < 0.22 {
                image.clear(x, y);
            } else if rng.next_f32() < 0.15 {
                image.set(x, y, scale(base, 0.7));
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
        (root.join("mods/example/textures/breaker.png"), breaker()),
        (root.join("mods/example/textures/ruby.png"), ruby()),
    ];
    for (path, image) in &textures {
        image.save(path)?;
    }
    Ok(textures.into_iter().map(|(path, _)| path).collect())
}
