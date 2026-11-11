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
    let mut image = Image::filled(scale(STONE, 0.9)).noise(scale(STONE, 0.9), 0.1, 110);
    // Des pierres arrondies séparées par des joints sombres.
    let mut rng = Rng::new(111);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let cell = ((x / 4) + (y / 4) * 7 + (rng.next_u64() % 2) as usize) % 3;
            if x % 4 == 0 || y % 4 == ((x / 4) % 2) * 2 {
                image.set(x, y, scale(STONE, 0.6));
            } else if cell == 0 {
                image.set(x, y, scale(STONE, 1.05));
            }
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
    let wood = [0.62, 0.45, 0.27];
    let mut image = planks(wood, 112);
    for i in 0..SIZE {
        for (x, y) in [(i, 0), (i, 15), (0, i), (15, i)] {
            image.set(x, y, scale(wood, 0.6));
        }
    }
    // Une grille de 3 × 3 cases gravée sur le plateau.
    for i in 2..14 {
        for k in [5, 10] {
            image.set(k, i, scale(wood, 0.55));
            image.set(i, k, scale(wood, 0.55));
        }
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
    // La bouche du four : un trou sombre avec des braises.
    for y in 8..14 {
        for x in 4..12 {
            image.set(x, y, [0.12, 0.1, 0.1]);
        }
    }
    for x in [5, 7, 9, 10] {
        image.set(x, 13, [0.95, 0.5, 0.1]);
    }
    for x in 3..13 {
        image.set(x, 7, scale(STONE, 0.5));
    }
    image
}

fn furnace_top() -> Image {
    let base = scale(STONE, 0.85);
    Image::filled(base).noise(base, 0.06, 114)
}

fn chest_side() -> Image {
    let wood = [0.62, 0.43, 0.22];
    let mut image = planks(wood, 115);
    for i in 0..SIZE {
        for (x, y) in [(i, 0), (i, 15), (0, i), (15, i), (i, 5)] {
            image.set(x, y, scale(wood, 0.55));
        }
    }
    // La serrure.
    for (x, y) in [(7, 5), (8, 5), (7, 6), (8, 6), (7, 7), (8, 7)] {
        image.set(x, y, [0.8, 0.8, 0.82]);
    }
    image
}

fn chest_top() -> Image {
    let wood = [0.62, 0.43, 0.22];
    let mut image = planks(wood, 116);
    for i in 0..SIZE {
        for (x, y) in [(i, 0), (i, 15), (0, i), (15, i)] {
            image.set(x, y, scale(wood, 0.55));
        }
    }
    image
}

/// Le matelas du lit de feuillage : un vert tacheté.
fn mattress_pixel(x: usize, y: usize) -> [f32; 3] {
    let green = [0.34, 0.56, 0.24];
    if (x * 7 + y * 13 + x * y).is_multiple_of(5) {
        scale(green, 0.78)
    } else if (x * 3 + y * 11).is_multiple_of(7) {
        scale(green, 1.15)
    } else {
        green
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

/// Une icône dessinée à partir d'un masque de 16 lignes de 16 caractères. Chaque
/// caractère choisit une couleur dans `palette` ; « . » est transparent. Un contour
/// sombre est ajouté autour, pour que l'icône se détache du fond.
fn icon(mask: &[&str; 16], palette: &[(char, [f32; 3])]) -> Image {
    let mut image = Image::transparent();
    for (y, row) in mask.iter().enumerate() {
        for (x, c) in row.chars().enumerate().take(SIZE) {
            if let Some((_, color)) = palette.iter().find(|(k, _)| *k == c) {
                image.set(x, y, *color);
            }
        }
    }
    let opaque = |img: &Image, x: i32, y: i32| {
        (0..16).contains(&x)
            && (0..16).contains(&y)
            && img.pixels[x as usize + y as usize * SIZE][3] > 0
    };
    let copy = Image {
        pixels: image.pixels,
    };
    for y in 0..16i32 {
        for x in 0..16i32 {
            if !opaque(&copy, x, y)
                && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| opaque(&copy, x + dx, y + dy))
            {
                image.set(x as usize, y as usize, [0.08, 0.06, 0.05]);
            }
        }
    }
    image
}

const HANDLE: [f32; 3] = [0.5, 0.35, 0.18];

/// Les cinq outils, dessinés en diagonale : la tête (H, h plus sombre) et le manche (S).
fn tool(kind: &str, head: [f32; 3]) -> Image {
    let mask: [&str; 16] = match kind {
        "pickaxe" => [
            "................",
            "....hHHHHHh.....",
            "...HHHHHHHHH....",
            "..hHH....SHHh...",
            "..HH....SS.HH...",
            "..H....SS...H...",
            "......SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "SS..............",
            "S...............",
            "................",
            "................",
        ],
        "axe" => [
            "................",
            ".......hHH......",
            "......hHHHH.....",
            ".....hHHHHH.....",
            ".....HHHSHH.....",
            "......HSSH......",
            "......SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "SS..............",
            "S...............",
            "................",
            "................",
        ],
        "shovel" => [
            "................",
            "..........hHh...",
            ".........hHHHh..",
            "........hHHHHH..",
            ".........HHHHh..",
            "........SHHHh...",
            ".......SS.......",
            "......SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "SS..............",
            "S...............",
            "................",
        ],
        "hoe" => [
            "................",
            "......hHHHHh....",
            "......HHHHHH....",
            "...........SH...",
            "..........SS....",
            ".........SS.....",
            "........SS......",
            ".......SS.......",
            "......SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "SS..............",
            "................",
        ],
        _ => [
            "...............h",
            "..............hH",
            ".............hHh",
            "............hHh.",
            "...........hHh..",
            "..........hHh...",
            ".........hHh....",
            "........hHh.....",
            "...SS..hHh......",
            "....SShHh.......",
            ".....SSh........",
            ".....SSS........",
            "....SS.SS.......",
            "...SS...........",
            "..S.............",
            "................",
        ],
    };
    icon(
        &mask,
        &[('H', head), ('h', scale(head, 0.7)), ('S', HANDLE)],
    )
}

fn shears() -> Image {
    let metal = [0.78, 0.78, 0.82];
    let mask = [
        "................",
        ".........h.....h",
        "........hH....hH",
        ".......hH....hH.",
        "......hH....hH..",
        ".....hH....hH...",
        "......hH..hH....",
        ".......hHhH.....",
        "........hH......",
        ".......RR.RR....",
        "......R..R..R...",
        ".....R..R....R..",
        ".....R..R....R..",
        "......RR......R.",
        "...............R",
        "................",
    ];
    icon(
        &mask,
        &[
            ('H', metal),
            ('h', scale(metal, 0.7)),
            ('R', [0.7, 0.15, 0.12]),
        ],
    )
}

/// Un seau, vide ou plein d'un fluide (la couleur du contenu).
fn bucket(content: Option<[f32; 3]>) -> Image {
    let metal = [0.72, 0.72, 0.76];
    let mask = [
        "................",
        "................",
        "................",
        "...h........h...",
        "....h......h....",
        "...HHHHHHHHHH...",
        "...HFFFFFFFFH...",
        "...HFFFFFFFFH...",
        "....HHHHHHHH....",
        "....HHHHHHHH....",
        "....hHHHHHHh....",
        ".....HHHHHH.....",
        ".....hHHHHh.....",
        "................",
        "................",
        "................",
    ];
    let inside = content.unwrap_or([0.35, 0.35, 0.38]);
    icon(
        &mask,
        &[('H', metal), ('h', scale(metal, 0.7)), ('F', inside)],
    )
}

/// Une côtelette : la viande, un bord de gras et l'os. `meat` et `fat` : ses couleurs
/// (rose cru, brun grillé ; plus rouge pour le bœuf).
fn chop(meat: [f32; 3], fat: [f32; 3]) -> Image {
    let mask = [
        "................",
        "................",
        "................",
        ".....MMMMM......",
        "...MMMMMMMMM....",
        "..MMMmMMMMMMF...",
        "..MMMMMMmMMMF...",
        ".MMMMMMMMMMMF...",
        ".MMmMMMMMMMF....",
        ".MMMMMMMMMFB....",
        "..MMMMMMMF..BB..",
        "...FFFFFF....BBb",
        ".............bb.",
        "................",
        "................",
        "................",
    ];
    let bone = [0.94, 0.92, 0.85];
    icon(
        &mask,
        &[
            ('M', meat),
            ('m', scale(meat, 0.78)),
            ('F', fat),
            ('B', bone),
            ('b', scale(bone, 0.8)),
        ],
    )
}

/// Une peau tannée : un cuir aux bords irréguliers, avec des coutures.
fn leather() -> Image {
    let mask = [
        "................",
        "................",
        "...HH......HH...",
        "...HHHHHHHHHH...",
        "....HHhHHhHHH...",
        "....HHHHHHHH....",
        "...HHHhHHHhHH...",
        "...HHHHHHHHHH...",
        "....HhHHHHhH....",
        "....HHHHHHHH....",
        "...HHHHhHHHHH...",
        "...HH......HH...",
        "................",
        "................",
        "................",
        "................",
    ];
    let hide = [0.58, 0.36, 0.2];
    icon(&mask, &[('H', hide), ('h', scale(hide, 0.65))])
}

/// Une pomme rouge, sa queue et une feuille.
fn apple() -> Image {
    let mask = [
        "................",
        "........s.......",
        "........sLL.....",
        ".......s.LL.....",
        "....RRRsRRR.....",
        "...RRRRRRRRR....",
        "..RRRRRRRRRRR...",
        "..RrRRRRRRRRR...",
        "..RrRRRRRRRRR...",
        "..RRRRRRRRRRR...",
        "..RRRRRRRRRRd...",
        "...RRRRRRRRd....",
        "....RRRdRRd.....",
        ".....dd.dd......",
        "................",
        "................",
    ];
    let red = [0.82, 0.12, 0.1];
    icon(
        &mask,
        &[
            ('R', red),
            ('r', [0.98, 0.55, 0.5]),
            ('d', scale(red, 0.65)),
            ('s', [0.4, 0.28, 0.15]),
            ('L', [0.3, 0.62, 0.2]),
        ],
    )
}

/// Les pièces d'armure.
fn armor(piece: &str, color: [f32; 3]) -> Image {
    let mask: [&str; 16] = match piece {
        "helmet" => [
            "................",
            "................",
            "................",
            "....HHHHHHHH....",
            "...HHHHHHHHHH...",
            "..HHhhhhhhhhHH..",
            "..HHh......hHH..",
            "..HHh......hHH..",
            "..HH........HH..",
            "..HH........HH..",
            "................",
            "................",
            "................",
            "................",
            "................",
            "................",
        ],
        "chestplate" => [
            "................",
            "..HHH......HHH..",
            ".HHHHH....HHHHH.",
            ".HHHHHHHHHHHHHH.",
            ".HHHHHHHHHHHHHH.",
            "..hHHHHHHHHHHh..",
            "....HHHHHHHH....",
            "....HHHHHHHH....",
            "....hHHHHHHh....",
            "....HHHHHHHH....",
            "....HHHHHHHH....",
            "....hHHHHHHh....",
            "................",
            "................",
            "................",
            "................",
        ],
        "leggings" => [
            "................",
            "....HHHHHHHH....",
            "....hhhhhhhh....",
            "....HHHHHHHH....",
            "....HHH..HHH....",
            "....HHH..HHH....",
            "....HHH..HHH....",
            "....HHH..HHH....",
            "....HHH..HHH....",
            "....hHH..HHh....",
            "....HHH..HHH....",
            "................",
            "................",
            "................",
            "................",
            "................",
        ],
        _ => [
            "................",
            "................",
            "................",
            "................",
            "................",
            "................",
            "...HHH....HHH...",
            "...HHH....HHH...",
            "...HHH....HHH...",
            "..HHHH....HHHH..",
            ".HHHHH....HHHHH.",
            ".hhhhh....hhhhh.",
            "................",
            "................",
            "................",
            "................",
        ],
    };
    icon(&mask, &[('H', color), ('h', scale(color, 0.7))])
}

/// Un objet « matière » : lingot, minerai brut, gemme, poudre, bâton...
fn material(kind: &str, color: [f32; 3]) -> Image {
    let mask: [&str; 16] = match kind {
        "ingot" => [
            "................",
            "................",
            "................",
            "................",
            "................",
            "......LLLLLLL...",
            ".....LLLLLLLLL..",
            "....LHHHHHHHL...",
            "...LHHHHHHHH....",
            "..hHHHHHHHHh....",
            "..hhhhhhhhh.....",
            "................",
            "................",
            "................",
            "................",
            "................",
        ],
        "raw" => [
            "................",
            "................",
            "................",
            "......HHH.......",
            ".....HLHHH......",
            "....HHHHhHH.....",
            "...HHLHHHHHH....",
            "...HHHHHhHHHH...",
            "...HHHhHHHLHH...",
            "....HHHHHHHh....",
            ".....hHHHhH.....",
            ".......hh.......",
            "................",
            "................",
            "................",
            "................",
        ],
        "gem" => [
            "................",
            "................",
            "................",
            "......LLLL......",
            ".....LLHHHH.....",
            "....LHHHHHHH....",
            "...HHHHHHHHHh...",
            "...hHHHHHHHhh...",
            "....hHHHHHhh....",
            ".....hHHHhh.....",
            "......hHhh......",
            ".......hh.......",
            "................",
            "................",
            "................",
            "................",
        ],
        "dust" => [
            "................",
            "................",
            "................",
            "................",
            "................",
            "................",
            ".......H........",
            "......HHL.......",
            ".....HHHHH......",
            "....HHLHHHH.....",
            "...HHHHHHHHH....",
            "..hhhhhhhhhhh...",
            "................",
            "................",
            "................",
            "................",
        ],
        "seeds" => [
            "................",
            "................",
            "................",
            "................",
            ".....H....H.....",
            "....HH...HH.....",
            "..........h.....",
            "...H...H........",
            "..HH..HH...H....",
            "......h...HH....",
            "....H.......H...",
            "...HH......HH...",
            "................",
            "................",
            "................",
            "................",
        ],
        _ => [
            "................",
            "..............hS",
            ".............hS.",
            "............hS..",
            "...........hS...",
            "..........hS....",
            ".........hS.....",
            "........hS......",
            ".......hS.......",
            "......hS........",
            ".....hS.........",
            "....hS..........",
            "...hS...........",
            "..hS............",
            "................",
            "................",
        ],
    };
    icon(
        &mask,
        &[
            ('H', color),
            ('h', scale(color, 0.7)),
            ('L', scale(color, 1.3)),
            ('S', color),
        ],
    )
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
    for (path, image) in &textures {
        image.save(path)?;
    }
    Ok(textures.into_iter().map(|(path, _)| path).collect())
}

/// Les icônes des objets (`assets/core/items/`).
fn item_icons(dir: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    let mut add = |name: &str, image: Image| out.push((dir.join(format!("{name}.png")), image));
    for (name, kind, color) in [
        ("stick", "stick", HANDLE),
        ("coal", "raw", [0.18, 0.18, 0.2]),
        ("charcoal", "raw", [0.25, 0.2, 0.17]),
        ("raw_iron", "raw", [0.75, 0.62, 0.52]),
        ("raw_copper", "raw", [0.82, 0.5, 0.3]),
        ("raw_gold", "raw", [0.95, 0.78, 0.3]),
        ("iron_ingot", "ingot", [0.8, 0.8, 0.84]),
        ("copper_ingot", "ingot", [0.85, 0.5, 0.3]),
        ("gold_ingot", "ingot", [0.98, 0.82, 0.25]),
        ("sapphire", "gem", [0.3, 0.5, 1.0]),
        ("malachite", "gem", [0.2, 0.7, 0.45]),
        ("jade", "gem", [0.4, 0.9, 0.6]),
        ("cinnabar_dust", "dust", [0.85, 0.15, 0.12]),
        ("flint", "gem", [0.3, 0.3, 0.33]),
        ("seeds", "seeds", [0.6, 0.65, 0.3]),
    ] {
        add(name, material(kind, color));
    }
    for (material, color) in [
        ("wooden", [0.62, 0.46, 0.28]),
        ("stone", [0.55, 0.55, 0.57]),
        ("copper", [0.85, 0.52, 0.32]),
        ("iron", [0.82, 0.82, 0.86]),
        ("golden", [0.98, 0.82, 0.25]),
        ("sapphire", [0.3, 0.52, 1.0]),
    ] {
        for kind in ["pickaxe", "axe", "shovel", "hoe", "sword"] {
            add(&format!("{material}_{kind}"), tool(kind, color));
        }
        if !matches!(material, "wooden" | "stone") {
            for piece in ["helmet", "chestplate", "leggings", "boots"] {
                add(&format!("{material}_{piece}"), armor(piece, color));
            }
        }
    }
    add("shears", shears());
    add("bucket", bucket(None));
    add("water_bucket", bucket(Some([0.25, 0.45, 0.95])));
    add("lava_bucket", bucket(Some([1.0, 0.5, 0.1])));
    add("raw_pork", chop([0.9, 0.5, 0.5], [0.98, 0.88, 0.85]));
    add("cooked_pork", chop([0.55, 0.32, 0.16], [0.82, 0.62, 0.38]));
    add("raw_beef", chop([0.75, 0.22, 0.2], [0.95, 0.85, 0.8]));
    add("cooked_beef", chop([0.42, 0.24, 0.13], [0.7, 0.5, 0.3]));
    add("leather", leather());
    add("shadow_dust", material("dust", [0.3, 0.22, 0.42]));
    add("spores", material("seeds", [0.78, 0.82, 0.35]));
    add("apple", apple());
    add("wheat", material("seeds", [0.88, 0.74, 0.3]));
    add("bread", chop([0.75, 0.5, 0.22], [0.92, 0.75, 0.45]));
    add("carrot", material("dust", [0.95, 0.5, 0.1]));
    add("potato", material("raw", [0.72, 0.56, 0.32]));
    add("baked_potato", material("raw", [0.85, 0.62, 0.25]));
    add("cabbage", material("gem", [0.45, 0.75, 0.38]));
    add("melon_slice", material("gem", [0.9, 0.25, 0.3]));
    add("melon_seeds", material("seeds", [0.2, 0.22, 0.12]));
    add("pear", material("gem", [0.72, 0.82, 0.25]));
    add("nut", material("raw", [0.5, 0.36, 0.2]));
    add("fertilizer", material("dust", [0.5, 0.4, 0.25]));
    out
}
