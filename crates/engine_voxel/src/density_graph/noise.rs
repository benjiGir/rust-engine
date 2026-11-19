//! Les bruits du graphe de densité : Perlin « amélioré » (Ken Perlin, 2002), somme d'octaves et
//! « normal noise » (deux sommes d'octaves décalées), sur le modèle de la génération de Minecraft
//! moderne. Tout est dérivé d'une graine et d'un **nom** : l'ordre de création ne change rien.

use serde::Deserialize;

/// Un générateur pseudo-aléatoire minimal (SplitMix64) : rapide, sans dépendance, déterministe.
#[derive(Debug, Clone)]
pub(crate) struct SplitMix(u64);

impl SplitMix {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Un nombre dans `[0, 1)`.
    pub(crate) fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Un entier dans `0..n`.
    fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }
}

/// La graine d'un élément nommé : la même pour un même nom, quel que soit l'ordre de création.
pub(crate) fn derive(seed: u64, name: &str) -> u64 {
    // FNV-1a sur le nom, mêlé à la graine par SplitMix.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    SplitMix::new(seed ^ hash.rotate_left(17)).next_u64()
}

/// Le bruit de Perlin amélioré : gradients sur les 12 arêtes du cube, lissage quintique, table de
/// permutation mélangée par la graine et origine décalée au hasard.
#[derive(Debug, Clone)]
pub struct ImprovedPerlin {
    perm: [u8; 256],
    offset: [f64; 3],
}

impl ImprovedPerlin {
    pub(crate) fn new(rng: &mut SplitMix) -> Self {
        let offset = [
            rng.next_f64() * 256.0,
            rng.next_f64() * 256.0,
            rng.next_f64() * 256.0,
        ];
        let mut perm: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in (1..256).rev() {
            let j = rng.below(i as u32 + 1) as usize;
            perm.swap(i, j);
        }
        Self { perm, offset }
    }

    #[inline]
    fn p(&self, i: i32) -> i32 {
        i32::from(self.perm[(i & 255) as usize])
    }

    /// Environ de -1 à 1.
    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        let (x, y, z) = (x + self.offset[0], y + self.offset[1], z + self.offset[2]);
        let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = (fx as i32, fy as i32, fz as i32);
        let (x, y, z) = (x - fx, y - fy, z - fz);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let a = self.p(xi) + yi;
        let (aa, ab) = (self.p(a) + zi, self.p(a + 1) + zi);
        let b = self.p(xi + 1) + yi;
        let (ba, bb) = (self.p(b) + zi, self.p(b + 1) + zi);
        lerp(
            w,
            lerp(
                v,
                lerp(
                    u,
                    grad(self.p(aa), x, y, z),
                    grad(self.p(ba), x - 1.0, y, z),
                ),
                lerp(
                    u,
                    grad(self.p(ab), x, y - 1.0, z),
                    grad(self.p(bb), x - 1.0, y - 1.0, z),
                ),
            ),
            lerp(
                v,
                lerp(
                    u,
                    grad(self.p(aa + 1), x, y, z - 1.0),
                    grad(self.p(ba + 1), x - 1.0, y, z - 1.0),
                ),
                lerp(
                    u,
                    grad(self.p(ab + 1), x, y - 1.0, z - 1.0),
                    grad(self.p(bb + 1), x - 1.0, y - 1.0, z - 1.0),
                ),
            ),
        )
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

#[inline]
fn grad(hash: i32, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// Ramène une coordonnée près de 0 sans changer le bruit à l'échelle d'un monde : au-delà de
/// quelques millions, les flottants perdent leurs décimales.
#[inline]
fn wrap(v: f64) -> f64 {
    const PERIOD: f64 = 33_554_432.0;
    v - (v / PERIOD + 0.5).floor() * PERIOD
}

/// La description d'un bruit (en RON) : la plus basse octave (`-7` : une période de 128 blocs) et
/// l'amplitude de chaque octave, de la plus basse à la plus haute.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NoiseDef {
    pub first_octave: i32,
    pub amplitudes: Vec<f64>,
}

/// Une somme d'octaves : chaque octave double la fréquence et divise le poids par deux ; la somme
/// des poids vaut 1 (à amplitudes unitaires), donc la valeur reste de l'ordre de -1 à 1.
#[derive(Debug, Clone)]
pub struct OctaveNoise {
    octaves: Vec<(ImprovedPerlin, f64)>,
    lowest_input: f64,
    lowest_value: f64,
}

impl OctaveNoise {
    fn new(rng: &mut SplitMix, def: &NoiseDef) -> Self {
        let n = def.amplitudes.len().max(1) as i32;
        Self {
            octaves: def
                .amplitudes
                .iter()
                .map(|&a| (ImprovedPerlin::new(rng), a))
                .collect(),
            lowest_input: 2f64.powi(def.first_octave),
            lowest_value: 2f64.powi(n - 1) / (2f64.powi(n) - 1.0),
        }
    }

    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut input = self.lowest_input;
        let mut weight = self.lowest_value;
        let mut sum = 0.0;
        for (octave, amplitude) in &self.octaves {
            if *amplitude != 0.0 {
                sum += amplitude
                    * weight
                    * octave.sample(wrap(x * input), wrap(y * input), wrap(z * input));
            }
            input *= 2.0;
            weight /= 2.0;
        }
        sum
    }

    /// Une borne de la valeur absolue (le Perlin amélioré reste sous 1,04 environ).
    fn max_value(&self) -> f64 {
        let mut weight = self.lowest_value;
        let mut sum = 0.0;
        for (_, amplitude) in &self.octaves {
            sum += amplitude.abs() * weight;
            weight /= 2.0;
        }
        sum * 1.05
    }
}

/// Le « normal noise » : deux sommes d'octaves, la seconde lue à une échelle légèrement différente,
/// additionnées puis normalisées : la répartition est plus régulière qu'une seule somme (moins de
/// motifs alignés sur la grille), de l'ordre de -1 à 1.
#[derive(Debug, Clone)]
pub struct NormalNoise {
    first: OctaveNoise,
    second: OctaveNoise,
    factor: f64,
}

/// L'écart d'échelle du second bruit : un nombre « irrationnel » proche de 1.
const SECOND_INPUT: f64 = 1.018_126_888_217_522_7;

impl NormalNoise {
    /// Le bruit `name` de la graine de monde `seed`.
    pub fn new(seed: u64, name: &str, def: &NoiseDef) -> Self {
        let mut rng = SplitMix::new(derive(seed, name));
        let first = OctaveNoise::new(&mut rng, def);
        let second = OctaveNoise::new(&mut rng, def);
        // L'étendue des octaves utilisées : plus il y en a, plus la somme se resserre autour de 0.
        let used: Vec<usize> = def
            .amplitudes
            .iter()
            .enumerate()
            .filter(|(_, a)| **a != 0.0)
            .map(|(i, _)| i)
            .collect();
        let span = match (used.first(), used.last()) {
            (Some(a), Some(b)) => (b - a) as f64,
            _ => 0.0,
        };
        let deviation = 0.1 * (1.0 + 1.0 / (span + 1.0));
        Self {
            first,
            second,
            factor: (1.0 / 6.0) / deviation,
        }
    }

    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        (self.first.sample(x, y, z)
            + self
                .second
                .sample(x * SECOND_INPUT, y * SECOND_INPUT, z * SECOND_INPUT))
            * self.factor
    }

    /// Une borne de la valeur absolue.
    pub fn max_value(&self) -> f64 {
        (self.first.max_value() + self.second.max_value()) * self.factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(first_octave: i32, amplitudes: &[f64]) -> NoiseDef {
        NoiseDef {
            first_octave,
            amplitudes: amplitudes.to_vec(),
        }
    }

    #[test]
    fn perlin_is_zero_on_lattice_points_and_stays_bounded() {
        let mut rng = SplitMix::new(1);
        let p = ImprovedPerlin::new(&mut rng);
        let mut most: f64 = 0.0;
        for i in 0..20_000 {
            let f = i as f64;
            let v = p.sample(f * 0.137, f * 0.071 - 30.0, f * 0.193);
            most = most.max(v.abs());
        }
        assert!(most < 1.05 && most > 0.5, "{most}");
        // Aux nœuds de la grille (après décalage), le bruit est nul.
        let at = p.sample(-p.offset[0] + 3.0, -p.offset[1] + 7.0, -p.offset[2] - 2.0);
        assert!(at.abs() < 1e-9, "{at}");
    }

    #[test]
    fn noises_depend_on_seed_and_name_but_not_on_creation_order() {
        let d = def(-6, &[1.0, 1.0, 0.5]);
        let a = NormalNoise::new(42, "relief", &d);
        let _other = NormalNoise::new(42, "humidité", &d);
        let a2 = NormalNoise::new(42, "relief", &d);
        let b = NormalNoise::new(43, "relief", &d);
        let c = NormalNoise::new(42, "relief2", &d);
        let at = (123.4, 56.0, -789.1);
        assert_eq!(a.sample(at.0, at.1, at.2), a2.sample(at.0, at.1, at.2));
        assert_ne!(a.sample(at.0, at.1, at.2), b.sample(at.0, at.1, at.2));
        assert_ne!(a.sample(at.0, at.1, at.2), c.sample(at.0, at.1, at.2));
    }

    #[test]
    fn normal_noise_is_spread_around_zero_and_within_its_bound() {
        for d in [
            def(-7, &[1.0, 1.0]),
            def(-9, &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
        ] {
            let n = NormalNoise::new(7, "essai", &d);
            let bound = n.max_value();
            let (mut sum, mut sq, mut most) = (0.0, 0.0, 0.0f64);
            let count = 40_000;
            for i in 0..count {
                let v = n.sample(i as f64 * 37.3, 0.0, (i * 7919 % 10_007) as f64 * 41.1);
                assert!(v.abs() <= bound, "{v} > {bound}");
                sum += v;
                sq += v * v;
                most = most.max(v.abs());
            }
            let mean = sum / count as f64;
            let deviation = (sq / count as f64 - mean * mean).sqrt();
            assert!(mean.abs() < 0.08, "moyenne {mean}");
            assert!((0.15..0.6).contains(&deviation), "écart {deviation}");
            assert!(most > 0.5, "amplitude {most}");
        }
    }

    #[test]
    fn the_first_octave_sets_the_scale() {
        // Une octave à -4 (période 16) varie entre deux points à 8 blocs ; une octave à -10 (période
        // 1 024) presque pas.
        let fine = NormalNoise::new(1, "a", &def(-4, &[1.0]));
        let coarse = NormalNoise::new(1, "a", &def(-10, &[1.0]));
        let mut fine_change = 0.0;
        let mut coarse_change = 0.0;
        for i in 0..500 {
            let x = i as f64 * 97.0;
            fine_change += (fine.sample(x + 8.0, 0.0, 0.0) - fine.sample(x, 0.0, 0.0)).abs();
            coarse_change += (coarse.sample(x + 8.0, 0.0, 0.0) - coarse.sample(x, 0.0, 0.0)).abs();
        }
        assert!(
            fine_change > 10.0 * coarse_change,
            "{fine_change} {coarse_change}"
        );
    }

    #[test]
    fn huge_coordinates_do_not_lose_precision() {
        let n = NormalNoise::new(3, "loin", &def(-5, &[1.0, 1.0, 1.0]));
        let far = 30_000_000.0;
        let a = n.sample(far, 64.0, far);
        let b = n.sample(far + 0.5, 64.0, far);
        assert!(a.is_finite() && b.is_finite());
        assert_ne!(a, b, "le bruit varie encore au demi-bloc");
    }
}
