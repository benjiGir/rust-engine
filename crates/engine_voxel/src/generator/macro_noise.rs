//! Le bruit de la géographie macro (générateur v2, prototype) : déterministe, sans état, défini
//! en coordonnées monde.
//!
//! Chaque valeur ne dépend que de la graine, d'un sel et de la position : aucun tableau, aucun
//! cache, aucun ordre d'évaluation. Un nœud de la grille de bruit reçoit sa direction d'un hachage
//! de ses indices (deux mélanges `mix64`), ce qui coûte quelques nanosecondes de plus qu'une
//! table mais évite toute dépendance au bruit externe : les fonctions sont à nous, leurs sorties
//! aussi stables que le hachage.

use super::mix64;

/// Seize directions unitaires équiréparties (multiples de 22,5 degrés) pour les gradients.
const H: f64 = std::f64::consts::FRAC_1_SQRT_2;
const DIRECTIONS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_4),
    (H, H),
    (0.382_683_4, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_4, 0.923_879_5),
    (-H, H),
    (-0.923_879_5, 0.382_683_4),
    (-1.0, 0.0),
    (-0.923_879_5, -0.382_683_4),
    (-H, -H),
    (-0.382_683_4, -0.923_879_5),
    (0.0, -1.0),
    (0.382_683_4, -0.923_879_5),
    (H, -H),
    (0.923_879_5, -0.382_683_4),
];

/// Le hachage d'un nœud de la grille : deux mélanges, graine et sel compris.
pub(super) fn lattice(seed: u32, salt: u64, ix: i32, iz: i32) -> u64 {
    let key = ((ix as u32 as u64) << 32) | iz as u32 as u64;
    let k = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    mix64(mix64(key ^ k) ^ salt)
}

/// Un nombre uniforme dans `[0, 1)` pour un couple d'entiers (jitter des cellules, tirages).
pub(super) fn unit(seed: u32, salt: u64, a: i32, b: i32) -> f64 {
    (lattice(seed, salt, a, b) >> 11) as f64 / (1u64 << 53) as f64
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Un bruit de gradients en 2D, d'amplitude voisine de -1 à 1, continu et lisse.
pub(super) fn gradient_noise(seed: u32, salt: u64, x: f64, z: f64) -> f64 {
    let (x0, z0) = (x.floor(), z.floor());
    let (ix, iz) = (x0 as i32, z0 as i32);
    let (fx, fz) = (x - x0, z - z0);
    let dot = |cx: i32, cz: i32| {
        let h = lattice(seed, salt, ix.wrapping_add(cx), iz.wrapping_add(cz));
        let (gx, gz) = DIRECTIONS[(h >> 60) as usize];
        gx * (fx - cx as f64) + gz * (fz - cz as f64)
    };
    let (u, v) = (fade(fx), fade(fz));
    let top = dot(0, 0) + (dot(1, 0) - dot(0, 0)) * u;
    let bottom = dot(0, 1) + (dot(1, 1) - dot(0, 1)) * u;
    (top + (bottom - top) * v) * std::f64::consts::SQRT_2
}

/// Une somme d'octaves (chacune deux fois plus fine, `gain` fois moins forte), normalisée pour
/// rester entre -1 et 1 environ. Chaque octave a son sel et sa translation : pas d'alignement.
pub(super) fn fbm(seed: u32, salt: u64, x: f64, z: f64, octaves: u32, gain: f64) -> f64 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut fx, mut fz) = (x, z);
    for o in 0..octaves {
        sum += amp * gradient_noise(seed, salt ^ ((o as u64 + 1) << 40), fx, fz);
        norm += amp;
        amp *= gain;
        fx = fx * 2.03 + 17.3;
        fz = fz * 2.03 - 9.1;
    }
    sum / norm
}

/// 0 avant `edge0`, 1 après `edge1`, une courbe en S entre les deux (marche à l'envers aussi).
pub(super) fn smooth(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Une courbe cubique monotone (Fritsch-Carlson) par les points `pts` (x croissants) : continue,
/// sans dépassement entre deux points, sans palier aux nœuds. Constante hors des extrémités.
pub(super) fn profile(x: f64, pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    if x <= pts[0].0 {
        return pts[0].1;
    }
    if x >= pts[n - 1].0 {
        return pts[n - 1].1;
    }
    let k = pts.windows(2).position(|w| x <= w[1].0).unwrap_or(n - 2);
    let secant = |i: usize| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0);
    let tangent = |i: usize| {
        if i == 0 {
            return secant(0);
        }
        if i == n - 1 {
            return secant(n - 2);
        }
        let (d0, d1) = (secant(i - 1), secant(i));
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (h0, h1) = (pts[i].0 - pts[i - 1].0, pts[i + 1].0 - pts[i].0);
        let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        (w0 + w1) / (w0 / d0 + w1 / d1)
    };
    let ((x0, y0), (x1, y1)) = (pts[k], pts[k + 1]);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0
        + (t3 - 2.0 * t2 + t) * h * tangent(k)
        + (-2.0 * t3 + 3.0 * t2) * y1
        + (t3 - t2) * h * tangent(k + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_bounded_and_continuous() {
        let mut max = 0.0f64;
        let mut last = gradient_noise(7, 1, -50.0, 3.0);
        for i in 0..4000 {
            let x = -50.0 + i as f64 * 0.05;
            let v = gradient_noise(7, 1, x, 3.0);
            assert_eq!(v, gradient_noise(7, 1, x, 3.0));
            max = max.max(v.abs());
            assert!((v - last).abs() < 0.2, "saut à x = {x}");
            last = v;
        }
        assert!(max <= 1.2 && max > 0.4, "amplitude {max}");
        assert_ne!(
            gradient_noise(7, 1, 0.3, 0.3),
            gradient_noise(8, 1, 0.3, 0.3)
        );
    }

    #[test]
    fn profile_is_monotone_between_monotone_points() {
        let pts = [(0.0, 1.0), (0.1, 0.8), (0.4, 0.3), (1.0, 0.0)];
        let mut last = f64::MAX;
        for i in 0..=1000 {
            let v = profile(i as f64 / 1000.0, &pts);
            assert!(v <= last + 1e-12 && (0.0..=1.0).contains(&v));
            last = v;
        }
        assert_eq!(profile(-1.0, &pts), 1.0);
        assert_eq!(profile(2.0, &pts), 0.0);
    }

    #[test]
    fn unit_is_in_range_and_spread() {
        let mean: f64 = (0..2000).map(|i| unit(3, 9, i, -i)).sum::<f64>() / 2000.0;
        assert!((0.45..0.55).contains(&mean), "moyenne {mean}");
        assert!((0..2000).all(|i| (0.0..1.0).contains(&unit(3, 9, i, 5))));
    }
}
