//! Direction « terre, ardoise et cuivre » : motifs originaux 16 × 16, aplats groupés,
//! ombres froides et accents chauds. Aucune ressource graphique externe.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Rng;

mod adventure;
mod enchanting;
mod items;
mod machines;
mod mod_assets;
mod nature;
mod rails;
mod shaping;
mod storage;
mod workstations;
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
        let mut pixels = self.pixels.map(finish_pixel);
        if path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == "textures")
            && let Some(name) = path.file_stem().and_then(|stem| stem.to_str())
        {
            crate::machine_faces::join_trim(name, &mut pixels, SIZE);
        }
        let bytes: Vec<u8> = pixels.into_iter().flatten().collect();
        encoder.write_header()?.write_image_data(&bytes)?;
        Ok(())
    }
}

pub(crate) fn texel_hash(x: usize, y: usize, seed: u64) -> u64 {
    let mut n = seed ^ (x as u64).wrapping_mul(0x9e37_79b9) ^ (y as u64).wrapping_mul(0x85eb_ca6b);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n ^ (n >> 15)
}

/// Même palette pour le terrain, les objets et les machines. L'alpha est conservé.
pub(crate) fn finish_pixel([r, g, b, a]: [u8; 4]) -> [u8; 4] {
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
pub(crate) fn periodic_noise(x: f32, y: f32, cells: i32, seed: u64) -> f32 {
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
    let mut image = seamless_ground(GRASS, 1, 0.075);
    ground_top_edges(&mut image.pixels, SIZE);
    image
}

/// La même bordure sur les quatre arêtes du dessus, avec une seule texture latérale.
pub(crate) fn ground_top_edges(pixels: &mut [[u8; 4]], size: usize) {
    for i in 0..size {
        let edge = pixels[i];
        pixels[i * size] = edge;
        pixels[i * size + size - 1] = edge;
        pixels[(size - 1) * size + i] = edge;
    }
}

pub(crate) fn ground_cover_depth(x: usize, size: usize, podzol: bool) -> usize {
    let phase = (x % (size - 1)) as f32 / (size - 1) as f32 * std::f32::consts::TAU;
    let depth = if podzol { 2.5 } else { 3.0 };
    ((depth + phase.cos() * 0.6 + (phase * 3.0).cos() * 0.55) * size as f32 / 16.0).round() as usize
}

pub(crate) fn ground_side(base: &mut [[u8; 4]], top: &[[u8; 4]], size: usize, podzol: bool) {
    for x in 0..size {
        for y in 0..ground_cover_depth(x, size, podzol) {
            base[x + y * size] = top[x + y * size];
        }
    }
}

pub(crate) fn grass_side_tint(pixels: &mut [[u8; 4]], size: usize) {
    for x in 0..size {
        for y in 0..ground_cover_depth(x, size, false) {
            // Seul l'alpha est lu par le shader.
            pixels[x + y * size] = [132, 132, 132, 255];
        }
    }
}

fn dirt() -> Image {
    seamless_ground(DIRT, 4, 0.14)
}

/// Le dessus de la terre labourée : des sillons, plus sombres quand elle est humide.
fn farmland(wet: bool) -> Image {
    nature::farmland(wet)
}

/// Le dessus de la table : un livre de lin sur un plateau d’ardoise gravé.
fn enchanting_top() -> Image {
    workstations::draw("enchanting_top")
}

/// Le côté : pierre bleutée, pages de lin et liserés de cuivre.
fn enchanting_side() -> Image {
    workstations::draw("enchanting_side")
}

/// Le dessus de l’alambic : une cuve de cuivre et un condenseur sur socle.
fn brewing_top() -> Image {
    workstations::draw("brewing_top")
}

/// Le côté : cuivre et verre patiné, aux hauteurs des pièces du modèle.
fn brewing_side() -> Image {
    workstations::draw("brewing_side")
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
    machines::draw(kind, on)
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

/// Un levier : manche en bois, pivot et contact cuivré, clair à l’activation.
fn lever(on: bool) -> Image {
    machines::draw("lever", on)
}

/// Un bouton : contact cuivré plus clair quand il est pressé.
fn button(on: bool) -> Image {
    machines::draw("button", on)
}

/// Une herse : acier et cuivre ; les ouvertures viennent des boîtes du bloc.
fn gate(open: bool) -> Image {
    machines::draw("gate", open)
}

/// L'inverseur (un point) et le répéteur (deux points), allumés ou non.
fn logic(dots: usize, on: bool) -> Image {
    machines::draw(if dots == 2 { "repeater" } else { "inverter" }, on)
}

/// Un piston : une planche bordée de fer ; `centre` colore le cœur (vert : collant).
fn piston(extended: bool, centre: [f32; 3]) -> Image {
    machines::draw(
        if centre[1] > centre[0] {
            "sticky_piston"
        } else {
            "piston"
        },
        extended,
    )
}

/// La tête d'un piston sorti : une planche pleine.
fn piston_head() -> Image {
    machines::draw("piston_head", false)
}

/// Le côté de l'herbe : de la terre, avec une bordure d'herbe irrégulière en haut.
fn grass_side() -> Image {
    let mut image = dirt();
    ground_side(&mut image.pixels, &grass_top().pixels, SIZE, false);
    image
}

fn stone() -> Image {
    nature::stone()
}

pub(crate) fn stone_pixels() -> [[u8; 4]; SIZE * SIZE] {
    stone().pixels.map(finish_pixel)
}

const PODZOL: [f32; 3] = [0.47, 0.31, 0.17];

/// Le sol des vieilles forêts de conifères : un tapis brun-roux d'aiguilles tombées.
fn podzol_top() -> Image {
    let mut image = seamless_ground(PODZOL, 1701, 0.15);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Des aiguilles : de courts traits clairs ou sombres, en diagonale.
            let h = texel_hash(x, y, 1702) % 23;
            if h == 0 {
                image.set(x, y, scale(PODZOL, 1.35));
                image.set((x + 1) % SIZE, (y + 1) % SIZE, scale(PODZOL, 1.25));
            } else if h == 1 {
                image.set(x, y, scale(PODZOL, 0.7));
            }
        }
    }
    ground_top_edges(&mut image.pixels, SIZE);
    image
}

/// Le côté du podzol : de la terre, avec une bordure d'aiguilles en haut.
fn podzol_side() -> Image {
    let mut image = dirt();
    ground_side(&mut image.pixels, &podzol_top().pixels, SIZE, true);
    image
}

/// La terre grossière des pays secs : plus pâle que la terre, semée de cailloux.
fn coarse_dirt() -> Image {
    let base = [0.53, 0.39, 0.27];
    let mut image = seamless_ground(base, 1711, 0.12);
    for y in 0..SIZE {
        for x in 0..SIZE {
            match texel_hash(x, y, 1712) % 13 {
                0 => image.set(x, y, scale(STONE, 0.95)),
                1 => image.set(x, y, scale(STONE, 0.75)),
                2 => image.set(x, y, scale(base, 0.75)),
                _ => {}
            }
        }
    }
    image
}

/// Le sable rouge des plateaux arides, avec les mêmes rides que le sable.
fn red_sand() -> Image {
    let base = [0.76, 0.44, 0.24];
    let mut image = seamless_ground(base, 1721, 0.07);
    for y in 0..SIZE {
        let v = y as f32 / (SIZE - 1) as f32;
        for x in 0..SIZE {
            let u = x as f32 / (SIZE - 1) as f32;
            let ripple = 1.0 + (std::f32::consts::TAU * (v * 2.0 + u)).sin() * 0.03;
            let pixel = image.pixels[x + y * SIZE];
            image.set(
                x,
                y,
                scale(
                    [
                        pixel[0] as f32 / 255.0,
                        pixel[1] as f32 / 255.0,
                        pixel[2] as f32 / 255.0,
                    ],
                    ripple,
                ),
            );
        }
    }
    image
}

/// La terre cuite des falaises arides : une argile durcie, presque unie. Trois teintes, empilées
/// en strates par le générateur.
fn terracotta(base: [f32; 3], seed: u64) -> Image {
    seamless_ground(base, seed, 0.045)
}

fn sand() -> Image {
    nature::mineral("sand")
}

fn snow() -> Image {
    nature::mineral("snow")
}

/// Fond de couleur périodique. Les vagues et les reflets sont calculés par le shader.
pub(crate) fn water_tone(x: usize, y: usize, side: usize) -> [f32; 3] {
    let period = side - 1;
    let angle = |i: usize| std::f32::consts::TAU * (i % period) as f32 / period as f32;
    let (x, y) = (angle(x), angle(y));
    let variation = 0.012 * (x + y + 0.65 * y.sin()).sin() + 0.005 * (2.0 * x - y).cos();
    [
        0.274 + variation,
        0.509 + 1.2 * variation,
        0.626 + 0.83 * variation,
    ]
}

fn water() -> Image {
    let mut image = Image::transparent();
    for y in 0..SIZE {
        for x in 0..SIZE {
            image.set(x, y, water_tone(x, y, SIZE));
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

/// Bois fendu dans la longueur, avec une tête brûlée et des braises dans les fissures.
pub(crate) fn torch_wood(side: usize) -> Vec<[u8; 4]> {
    (0..side * side)
        .map(|i| {
            let (x, y) = (i % side, i / side);
            let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
            let line = 0.53 + (v * 6.0 + 0.8).sin() * 0.062;
            let split = (u - line).abs() < 0.035;
            let grain = (u * 63.0 + (v * 9.0).sin() * 1.45).sin();
            let grit = (texel_hash(x, y, 17257) & 15) as f32 / 15.0;
            let mut rgb = if v < 0.48 {
                let ash = 0.84 + 0.18 * grit + grain * 0.08;
                if split && (texel_hash(x / 2, y, 17261) & 3) != 0 {
                    [214.0, 91.0, 31.0]
                } else {
                    [51.0 * ash, 37.0 * ash, 31.0 * ash]
                }
            } else {
                let tone =
                    0.95 + 0.16 * grain + 0.14 * grit - 0.28 * (if split { 1.0 } else { 0.0 });
                [133.0 * tone, 83.0 * tone, 44.0 * tone]
            };
            // Le haut de la poignée est noirci par la chaleur.
            let soot = if (0.48..0.62).contains(&v) {
                0.64 + (v - 0.48) * (0.36 / 0.14)
            } else {
                1.0
            };
            rgb = rgb.map(|channel| channel * soot);
            [rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255]
        })
        .collect()
}

/// La torche : un bois sombre avec un cœur lumineux.
fn torch() -> Image {
    nature::torch()
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
    machines::draw("lamp", lit)
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
    nature::mineral("ice")
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

// ----- La végétation (phase O2) -----

/// L'écorce : des stries verticales irrégulières.
fn log_side(base: [f32; 3], seed: u64) -> Image {
    nature::bark(base, seed)
}

/// Le bouleau : une écorce blanche avec des taches noires horizontales.
fn birch_side() -> Image {
    nature::birch()
}

/// Le dessus d'une bûche : des cernes autour du cœur.
fn log_top(bark: [f32; 3], wood: [f32; 3]) -> Image {
    nature::endgrain(bark, wood)
}

/// Des planches : trois assises de lattes horizontales.
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
    nature::foliage(base, seed)
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

/// Des pétales roses tombés : des petites taches de deux à quatre pixels, de trois roses, éparses sur un
/// fond transparent (le bloc est posé à plat sur l'herbe).
fn pink_petals() -> Image {
    let mut image = Image::transparent();
    let shades = [[0.96, 0.7, 0.8], [0.9, 0.56, 0.7], [0.99, 0.82, 0.88]];
    for n in 0..26u64 {
        let h = texel_hash(n as usize, 7, 4441);
        let (x, y) = ((h % 16) as usize, ((h >> 8) % 16) as usize);
        let color = shades[((h >> 16) % 3) as usize];
        image.set(x, y, color);
        // Un pétale fait parfois deux ou trois pixels, dans un sens tiré au hasard.
        let (dx, dy) = [(1, 0), (0, 1), (1, 1)][((h >> 20) % 3) as usize];
        if !(h >> 24).is_multiple_of(3) {
            image.set((x + dx) % SIZE, (y + dy) % SIZE, scale(color, 0.92));
        }
        if (h >> 28).is_multiple_of(4) {
            image.set((x + 2 * dx) % SIZE, (y + 2 * dy) % SIZE, scale(color, 0.85));
        }
    }
    image
}

fn cactus_side() -> Image {
    nature::cactus(false)
}

fn cactus_top() -> Image {
    nature::cactus(true)
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

/// Les moellons gagnés par la mousse : des plaques vertes, par groupes, sur les pierres et les joints.
fn mossy_cobblestone() -> Image {
    let mut image = cobblestone();
    let moss = [0.3, 0.46, 0.2];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let patch = texel_hash(x / 3, y / 3, 146) % 5 < 2;
            let edge = texel_hash(x, y, 147) % 3;
            if patch && edge != 0
                || (!patch && edge == 0 && texel_hash(x / 2, y / 2, 148).is_multiple_of(7))
            {
                let shade = 0.88 + (texel_hash(x, y, 149) % 4) as f32 * 0.06;
                image.set(x, y, scale(moss, shade));
            }
        }
    }
    image
}

/// Les cendres volcaniques : une poudre gris sombre, à grain fin, avec des éclats plus clairs.
fn ash() -> Image {
    let base = [0.27, 0.27, 0.28];
    Image::filled(base)
        .noise(base, 0.12, 153)
        .speckles([0.42, 0.41, 0.4], 0.08, 154)
        .speckles([0.15, 0.15, 0.16], 0.1, 155)
}

/// Le bloc de cristal : des facettes violettes, claires sur les arêtes, sombres au cœur.
fn crystal_block() -> Image {
    let base = [0.5, 0.3, 0.76];
    let mut image = Image::filled(base);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Des bandes en diagonale (les faces des cristaux) et quelques éclats.
            let band = ((x + y) / 4 + (x + SIZE - y) / 5) % 3;
            let shade = [0.82, 1.0, 1.18][band];
            let spark = texel_hash(x, y, 160).is_multiple_of(19);
            let color = if spark {
                [0.86, 0.74, 1.0]
            } else {
                scale(base, shade)
            };
            image.set(x, y, color);
        }
    }
    image
}

/// Un amas de cristaux : trois pointes qui sortent de la roche.
fn crystal_cluster() -> Image {
    let mut image = Image::transparent();
    let light = [0.82, 0.66, 1.0];
    let dark = [0.48, 0.28, 0.74];
    for (x0, height, lean) in [(3i32, 9i32, -1i32), (7, 13, 0), (11, 8, 1)] {
        for k in 0..height {
            let x = x0 + lean * k / 4;
            let y = 15 - k;
            let width = if k > height - 3 { 1 } else { 2 };
            for dx in 0..width {
                image.plot(x + dx, y, if dx == 0 { light } else { dark });
            }
        }
    }
    image
}

/// La calcite : un blanc crème, des veines grises.
fn calcite() -> Image {
    let base = [0.86, 0.85, 0.82];
    let mut image = Image::filled(base).noise(base, 0.06, 161);
    for x in 0..SIZE {
        let y = (x * 3 / 5 + 4 + (texel_hash(x, 0, 162) % 2) as usize) % SIZE;
        image.set(x, y, [0.62, 0.62, 0.62]);
    }
    image
}

/// La mousse luminescente : un tapis vert sombre piqué de points turquoise.
fn glow_moss() -> Image {
    let base = [0.14, 0.36, 0.28];
    Image::filled(base)
        .noise(base, 0.18, 163)
        .speckles([0.4, 0.95, 0.82], 0.12, 164)
}

/// Le dessus du mycélium : un feutre gris violacé, des filaments clairs.
fn mycelium_top() -> Image {
    let base = [0.42, 0.36, 0.42];
    Image::filled(base)
        .noise(base, 0.12, 165)
        .speckles([0.62, 0.56, 0.6], 0.1, 166)
}

/// Le côté du mycélium : de la roche sous un liseré de feutre (il pousse sur la pierre des grottes).
fn mycelium_side() -> Image {
    let mut image = nature::mineral("stone");
    let top = mycelium_top();
    for x in 0..SIZE {
        let depth = 2 + (texel_hash(x, 0, 167) % 3) as usize;
        for y in 0..depth {
            image.pixels[x + y * SIZE] = top.pixels[x + y * SIZE];
        }
    }
    image
}

/// Le pied d'un champignon géant : des fibres crème verticales.
fn mushroom_stem() -> Image {
    let base = [0.86, 0.82, 0.74];
    let mut image = Image::filled(base);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let fiber = 0.9 + (texel_hash(x, 0, 168) % 4) as f32 * 0.05;
            let grain = (texel_hash(x, y, 169) % 5) as f32 * 0.01;
            image.set(x, y, scale(base, fiber + grain));
        }
    }
    image
}

/// Le chapeau d'un champignon géant : rouge brun, taché de crème.
fn mushroom_cap() -> Image {
    let base = [0.64, 0.2, 0.16];
    let mut image = Image::filled(base).noise(base, 0.1, 170);
    for (cx, cy) in [(3usize, 4usize), (11, 3), (7, 10), (13, 12), (2, 13)] {
        for dy in 0..2 {
            for dx in 0..2 {
                image.set((cx + dx) % SIZE, (cy + dy) % SIZE, [0.92, 0.88, 0.8]);
            }
        }
    }
    image
}

/// La bruyère : une touffe basse de tiges brunes et vertes, couverte de petites fleurs mauves.
fn heather() -> Image {
    let stem = [0.32, 0.36, 0.2];
    let bloom = [0.66, 0.36, 0.62];
    let mut image = Image::transparent();
    for (i, x) in [1, 3, 4, 6, 8, 9, 11, 13, 14].into_iter().enumerate() {
        let top = 7 + (texel_hash(i, 0, 150) % 4) as i32;
        let lean = (texel_hash(i, 1, 150) % 3) as i32 - 1;
        for y in top..16 {
            let dx = if y < top + 3 { lean } else { 0 };
            image.plot(x + dx, y, stem);
        }
        for k in 0..3 {
            let shade = 0.85 + (texel_hash(i, k, 151) % 4) as f32 * 0.08;
            let dx = lean + if k == 1 { 1 } else { 0 };
            image.plot(x + dx, top - 1 + k as i32, scale(bloom, shade));
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
    workstations::draw("workbench_top")
}

fn workbench_side() -> Image {
    workstations::draw("workbench_side")
}

/// Le dessus du lit : le matelas, un oreiller pâle en haut, le cadre en bois.
fn bed_top() -> Image {
    workstations::draw("bed_top")
}

/// Le côté du lit : le matelas en haut, le cadre et les pieds en bas.
fn bed_side() -> Image {
    workstations::draw("bed_side")
}

/// Écrit toutes les textures ; renvoie les chemins.
pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let core = root.join("assets/core/textures");
    let textures: Vec<(PathBuf, Image)> = vec![
        (core.join("grass_top.png"), grass_top()),
        (core.join("grass_side.png"), grass_side()),
        (core.join("grass_side_tint.png"), {
            let mut mask = Image::transparent();
            grass_side_tint(&mut mask.pixels, SIZE);
            mask
        }),
        (core.join("dirt.png"), dirt()),
        (core.join("podzol_top.png"), podzol_top()),
        (core.join("podzol_side.png"), podzol_side()),
        (core.join("coarse_dirt.png"), coarse_dirt()),
        (core.join("red_sand.png"), red_sand()),
        (
            core.join("terracotta.png"),
            terracotta([0.66, 0.38, 0.26], 1731),
        ),
        (
            core.join("terracotta_pale.png"),
            terracotta([0.82, 0.66, 0.52], 1732),
        ),
        (
            core.join("terracotta_dark.png"),
            terracotta([0.44, 0.27, 0.21], 1733),
        ),
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
        // Les essences de la génération v3 (vague 1) : séquoia, érable, arbres en fleurs.
        (
            core.join("redwood_log.png"),
            log_side([0.42, 0.2, 0.12], 140),
        ),
        (
            core.join("redwood_log_top.png"),
            log_top([0.42, 0.2, 0.12], [0.62, 0.32, 0.2]),
        ),
        (
            core.join("redwood_leaves.png"),
            leaves([0.13, 0.3, 0.15], 141),
        ),
        (
            core.join("redwood_planks.png"),
            planks([0.58, 0.29, 0.18], 142),
        ),
        (
            core.join("redwood_sapling.png"),
            sapling([0.42, 0.2, 0.12], [0.15, 0.32, 0.16]),
        ),
        (
            core.join("maple_leaves.png"),
            leaves([0.78, 0.3, 0.09], 143),
        ),
        (
            core.join("maple_sapling.png"),
            sapling([0.3, 0.19, 0.08], [0.78, 0.3, 0.09]),
        ),
        (
            core.join("blossom_leaves.png"),
            leaves([0.95, 0.76, 0.84], 144),
        ),
        // Le cerisier (lot N4 du plan nature) : écorce brun-pourpre, bois rose pâle, feuillage rose.
        (
            core.join("cherry_log.png"),
            log_side([0.3, 0.17, 0.18], 146),
        ),
        (
            core.join("cherry_log_top.png"),
            log_top([0.3, 0.17, 0.18], [0.86, 0.62, 0.58]),
        ),
        (
            core.join("cherry_leaves.png"),
            leaves([0.96, 0.62, 0.76], 147),
        ),
        (
            core.join("cherry_planks.png"),
            planks([0.86, 0.6, 0.56], 148),
        ),
        (
            core.join("cherry_sapling.png"),
            sapling([0.3, 0.17, 0.18], [0.96, 0.62, 0.76]),
        ),
        (core.join("pink_petals.png"), pink_petals()),
        // La vague 2 : sapin aux aiguilles bleutées.
        (core.join("fir_leaves.png"), leaves([0.16, 0.34, 0.32], 145)),
        (
            core.join("fir_sapling.png"),
            sapling([0.3, 0.22, 0.14], [0.16, 0.36, 0.33]),
        ),
        // La vague 3 : olivier au feuillage argenté.
        (
            core.join("olive_leaves.png"),
            leaves([0.42, 0.5, 0.36], 152),
        ),
        (
            core.join("olive_sapling.png"),
            sapling([0.36, 0.3, 0.22], [0.44, 0.52, 0.38]),
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
        (core.join("heather.png"), heather()),
        (
            core.join("marigold.png"),
            flower([0.96, 0.58, 0.1], [0.62, 0.22, 0.05]),
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
        (core.join("mossy_cobblestone.png"), mossy_cobblestone()),
        (core.join("ash.png"), ash()),
        // La vague 4 : les grottes.
        (core.join("crystal_block.png"), crystal_block()),
        (core.join("crystal_cluster.png"), crystal_cluster()),
        (core.join("calcite.png"), calcite()),
        (core.join("glow_moss.png"), glow_moss()),
        (core.join("glowcap.png"), mushroom([0.35, 0.82, 0.95], true)),
        (core.join("mycelium_top.png"), mycelium_top()),
        (core.join("mycelium_side.png"), mycelium_side()),
        (core.join("mushroom_stem.png"), mushroom_stem()),
        (core.join("mushroom_cap.png"), mushroom_cap()),
        (core.join("volcanic_rock.png"), volcanic_rock()),
        (core.join("workbench_top.png"), workbench_top()),
        (core.join("workbench_side.png"), workbench_side()),
        (core.join("bed_top.png"), bed_top()),
        (core.join("bed_side.png"), bed_side()),
        (
            core.join("bed_head_top.png"),
            workstations::draw("bed_head_top"),
        ),
        (
            core.join("bed_foot_top.png"),
            workstations::draw("bed_foot_top"),
        ),
        (
            core.join("bed_long_side.png"),
            workstations::draw("bed_long_side"),
        ),
        (root.join("mods/example/textures/breaker.png"), breaker()),
        (root.join("mods/example/textures/ruby.png"), ruby()),
    ];
    let mut textures = textures;
    textures.extend(
        crate::machine_faces::NAMES
            .iter()
            .map(|&name| name.to_owned())
            .chain(crate::machine_faces::paired_chest_names())
            .map(|name| {
                let pixels = crate::machine_faces::draw(&name, SIZE)
                    .unwrap()
                    .try_into()
                    .unwrap();
                (core.join(format!("{name}.png")), Image { pixels })
            }),
    );
    textures.extend(crate::ore_veins::ORES.iter().map(|ore| {
        let mut image = stone();
        ore.paint(&mut image.pixels, SIZE);
        (core.join(format!("{}.png", ore.name)), image)
    }));
    textures.extend(crate::doors::TEXTURES.into_iter().map(|name| {
        let pixels = crate::doors::draw(name, SIZE).unwrap().try_into().unwrap();
        (core.join(format!("{name}.png")), Image { pixels })
    }));
    textures.extend(workstations::mod_textures(root));
    textures.extend(mod_assets::generate(root));
    textures.push((
        core.join("machine_frame.png"),
        machines::draw("machine_frame", false),
    ));
    textures.extend(item_icons(&root.join("assets/core/items")));
    textures.extend(rails::generate(&core, &root.join("assets/core/items")));
    textures.extend(storage::generate(&core, &root.join("assets/core/items")));
    textures.extend(adventure::generate(&core, &root.join("assets/core/items")));
    textures.extend(shaping::generate(&core, &root.join("assets/core/items")));
    textures.extend(enchanting::generate(&core, &root.join("assets/core/items")));
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
    let mut written: Vec<_> = textures.into_iter().map(|(path, _)| path).collect();
    written.extend(crate::creature_skins::generate(root)?);
    written.extend(crate::fauna_items::generate(root)?);
    Ok(written)
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    #[test]
    fn all_versioned_core_and_mod_pngs_are_reproducible() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let temporary =
            std::env::temp_dir().join(format!("assetgen-review-{}", std::process::id()));
        let generated = generate(&temporary).unwrap();
        let mut checked = 0;
        for namespace in [
            "core",
            "example",
            "mecanique",
            "donjons",
            "entrepot",
            "magie",
            "forge",
        ] {
            let base = if namespace == "core" {
                root.join("assets/core")
            } else {
                root.join("mods").join(namespace)
            };
            for kind in ["textures", "items"] {
                let directory = base.join(kind);
                if !directory.exists() {
                    continue;
                }
                for entry in std::fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension().is_none_or(|ext| ext != "png") {
                        continue;
                    }
                    let relative = path.strip_prefix(&root).unwrap();
                    let produced = temporary.join(relative);
                    assert!(
                        generated.contains(&produced),
                        "PNG hors générateur : {}",
                        relative.display()
                    );
                    assert_eq!(
                        std::fs::read(&path).unwrap(),
                        std::fs::read(&produced).unwrap(),
                        "PNG non régénéré : {}",
                        relative.display()
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 250);
        std::fs::remove_dir_all(temporary).unwrap();
    }

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

    #[test]
    fn natural_materials_join_after_palette_quantization() {
        for image in [
            stone(),
            sand(),
            snow(),
            ice(),
            log_side([0.42, 0.31, 0.18], 90),
            log_side([0.30, 0.21, 0.12], 95),
            leaves([0.3, 0.55, 0.2], 91),
        ] {
            for i in 0..SIZE {
                assert_eq!(
                    finish_pixel(image.pixels[i * SIZE]),
                    finish_pixel(image.pixels[i * SIZE + SIZE - 1]),
                    "raccord horizontal à la ligne {i}"
                );
                assert_eq!(
                    finish_pixel(image.pixels[i]),
                    finish_pixel(image.pixels[i + SIZE * (SIZE - 1)]),
                    "raccord vertical à la colonne {i}"
                );
            }
        }
    }
}
