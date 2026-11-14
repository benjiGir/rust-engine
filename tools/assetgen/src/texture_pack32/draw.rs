//! Primitives de dessin à la résolution native du pack, sans filtrage d'une image 16.
use super::{Pixel, SIDE, hash, quantize};

pub(super) type Color = [u8; 3];
pub(super) const INK: Color = [30, 37, 39];
pub(super) const COPPER: Color = [178, 115, 72];
pub(super) const IRON: Color = [139, 161, 163];
pub(super) const SLATE: Color = [74, 90, 96];
pub(super) const WOOD: Color = [123, 86, 50];

pub(super) fn shade(c: Color, factor: f32) -> Color {
    c.map(|v| (v as f32 * factor).clamp(0.0, 255.0) as u8)
}
pub(super) fn mix(a: Color, b: Color, t: f32) -> Color {
    std::array::from_fn(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8)
}
pub(super) fn rgb(p: Pixel) -> Color {
    [p[0], p[1], p[2]]
}
pub(super) fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}
/// Bruit de valeur périodique, pour les volumes de matière plutôt qu'un bruit blanc.
pub(super) fn noise(x: f32, y: f32, cells: i32, seed: u64) -> f32 {
    let (x, y) = (x * cells as f32, y * cells as f32);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let ease = |v: f32| v * v * (3.0 - 2.0 * v);
    let (u, v) = (ease(x.fract()), ease(y.fract()));
    let at = |dx: i32, dy: i32| {
        (hash(
            (ix + dx).rem_euclid(cells) as usize,
            (iy + dy).rem_euclid(cells) as usize,
            seed,
        ) % 1000) as f32
            / 999.0
    };
    let a = at(0, 0) * (1.0 - u) + at(1, 0) * u;
    let b = at(0, 1) * (1.0 - u) + at(1, 1) * u;
    a * (1.0 - v) + b * v
}

pub(super) struct Canvas(pub Vec<Pixel>);
impl Canvas {
    pub fn new(c: Color) -> Self {
        Self(vec![[c[0], c[1], c[2], 255]; SIDE * SIDE])
    }
    pub fn transparent() -> Self {
        Self(vec![[0; 4]; SIDE * SIDE])
    }
    pub fn at(&self, x: i32, y: i32) -> Pixel {
        if (0..32).contains(&x) && (0..32).contains(&y) {
            self.0[x as usize + y as usize * SIDE]
        } else {
            [0; 4]
        }
    }
    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        if (0..32).contains(&x) && (0..32).contains(&y) {
            self.0[x as usize + y as usize * SIDE] = [c[0], c[1], c[2], 255];
        }
    }
    pub fn rect(&mut self, [x, y, w, h]: [i32; 4], c: Color) {
        for v in y..y + h {
            for u in x..x + w {
                self.put(u, v, c);
            }
        }
    }
    pub fn line(&mut self, a: (i32, i32), b: (i32, i32), c: Color) {
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
        for i in 0..=steps {
            self.put(
                a.0 + (b.0 - a.0) * i / steps,
                a.1 + (b.1 - a.1) * i / steps,
                c,
            );
        }
    }
    pub fn panel(&mut self, r: [i32; 4], c: Color) {
        let [x, y, w, h] = r;
        self.rect(r, shade(c, 0.52));
        self.rect([x + 1, y + 1, w - 2, h - 2], c);
        self.line((x + 1, y + 1), (x + w - 2, y + 1), shade(c, 1.35));
        self.line((x + 1, y + 1), (x + 1, y + h - 2), shade(c, 1.18));
        self.line((x + 1, y + h - 2), (x + w - 2, y + h - 2), shade(c, 0.75));
    }
    pub fn bolt(&mut self, x: i32, y: i32, c: Color) {
        self.rect([x - 1, y, 3, 2], shade(c, 0.45));
        self.rect([x, y - 1, 2, 3], c);
        self.put(x, y - 1, shade(c, 1.6));
        self.put(x + 1, y + 1, shade(c, 0.65));
    }
    pub fn ellipse(&mut self, center: (f32, f32), radii: (f32, f32), c: Color) {
        for y in 0..32 {
            for x in 0..32 {
                if ((x as f32 - center.0) / radii.0).powi(2)
                    + ((y as f32 - center.1) / radii.1).powi(2)
                    <= 1.0
                {
                    self.put(x, y, c);
                }
            }
        }
    }
    pub fn periodic(&mut self, vertical: bool) {
        for y in 0..32 {
            self.0[31 + y * SIDE] = self.0[y * SIDE];
        }
        if vertical {
            for x in 0..32 {
                self.0[x + 31 * SIDE] = self.0[x];
            }
        }
    }
    pub fn finish(mut self) -> Vec<Pixel> {
        for p in &mut self.0 {
            if p[3] == 0 {
                *p = [0; 4];
            } else {
                for v in &mut p[..3] {
                    *v = quantize(*v as i32);
                }
            }
        }
        self.0
    }
}
