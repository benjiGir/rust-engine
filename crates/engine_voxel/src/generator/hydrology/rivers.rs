//! Le tracé des cours d'eau à l'échelle des blocs : les arêtes cellule → cellule sont lissées par
//! des courbes, et la requête « rivière la plus proche » lit ces courbes.
//!
//! # Une courbe par arête, sans couture
//!
//! L'arête `g → n` est une courbe cubique d'Hermite entre les nœuds de `g` et de `n`. Elle part de
//! `g` dans la direction de sa propre arête (la corde) et arrive en `n` dans la direction de
//! l'arête **suivante** de `n`. Un cours continue donc sans cassure à chaque nœud (la tangente
//! d'arrivée de l'arête amont est la tangente de départ de l'arête aval), et un affluent rejoint le
//! cours principal tangentiellement, comme une vraie confluence. La courbe ne dépend que de `g`, `n`
//! et de l'arête suivante : une fonction pure, la même pour toute requête.
//!
//! Un décalage perpendiculaire borné (une dixième de la longueur de l'arête au plus, nul aux deux
//! nœuds) y ajoute des méandres. Aux nœuds, jitterés entre 0,3 et 0,7 d'une cellule, deux cours voisins
//! restent à plus de 50 blocs l'un de l'autre, alors que courbure et méandres s'écartent d'une vingtaine
//! de blocs au plus de la corde : les cours ne se croisent pas.
//!
//! # Largeur
//!
//! La largeur croît avec l'accumulation ([`river_width`]) ; près de la mer (distance au rivage
//! faible, niveau d'eau proche de la mer), une embouchure **s'évase** jusqu'au double.

use super::{
    FineCell, Hydrology, RiverPoint, SALT_MEANDER, STREAM_MIN_ACCUMULATION, fine_cell, river_width,
};
use crate::generator::SEA_LEVEL;
use crate::generator::macro_noise::{smooth, unit};

/// Segments par courbe pour la distance à l'axe.
const SEGMENTS: usize = super::RIVER_SEGMENTS;
/// Distance maximale (blocs) à laquelle un cours peut influencer un point : demi-largeur maximale
/// d'une embouchure évasée plus la portée des vallées, avec une marge.
const MAX_INFLUENCE: f64 = 140.0;
/// Fraction de la longueur de la corde, tangentes de la courbe.
const TANGENT: f64 = 0.9;
/// Amplitude des méandres, en fraction de la longueur de la corde.
const MEANDER_AMPLITUDE: f64 = 0.10;
/// Évasement d'une embouchure : la largeur est multipliée par `1 + MOUTH_FLARE` au rivage.
const MOUTH_FLARE: f64 = 1.0;
/// Distance au rivage (blocs) sur laquelle l'embouchure s'évase.
const MOUTH_REACH: f64 = 700.0;

/// Une courbe de cours d'eau : `SEGMENTS + 1` points.
pub(super) type Line = [[f64; 2]; SEGMENTS + 1];

/// L'arête sortante d'une cellule de terre : sa cellule aval et sa courbe.
#[derive(Debug, Clone, Copy)]
pub(super) struct Edge {
    next: FineCell,
    line: Line,
}

impl Hydrology {
    /// L'arête sortante de `g` (mémoïsée), si c'est une cellule de terre qui s'écoule.
    fn river_edge(&self, g: FineCell) -> Option<Edge> {
        if let Some(e) = self.edges.get(&g) {
            return e;
        }
        let edge = if self.is_sea(g) {
            None
        } else {
            self.next(g).map(|n| Edge {
                next: n,
                line: self.river_polyline(g, n),
            })
        };
        self.edges.put(g, edge);
        edge
    }

    /// La courbe de l'arête `g → n`, en `SEGMENTS + 1` points de `g` à `n`.
    pub fn river_polyline(&self, g: FineCell, n: FineCell) -> [[f64; 2]; SEGMENTS + 1] {
        let (p0, p1) = (self.node(g), self.node(n));
        let chord = [p1[0] - p0[0], p1[1] - p0[1]];
        let len = chord[0].hypot(chord[1]).max(1e-9);
        let m0 = [chord[0] * TANGENT, chord[1] * TANGENT];
        // Tangente d'arrivée : la direction de l'arête suivante de `n` (confluence tangentielle).
        // Sur un virage serré (arête suivante presque opposée), elle se rapproche de la corde pour
        // éviter un crochet.
        let m1 = match self.next(n) {
            Some(nn) => {
                let p2 = self.node(nn);
                let d = [p2[0] - p1[0], p2[1] - p1[1]];
                let dl = d[0].hypot(d[1]).max(1e-9);
                let (u, c) = ([d[0] / dl, d[1] / dl], [chord[0] / len, chord[1] / len]);
                let w = smooth(-0.2, 0.5, u[0] * c[0] + u[1] * c[1]);
                let v = [c[0] * (1.0 - w) + u[0] * w, c[1] * (1.0 - w) + u[1] * w];
                let vl = v[0].hypot(v[1]).max(1e-9);
                [v[0] / vl * len * TANGENT, v[1] / vl * len * TANGENT]
            }
            None => m0,
        };
        let perp = [-chord[1] / len, chord[0] / len];
        let a = 2.0 * unit(self.seed, SALT_MEANDER, g.0, g.1) - 1.0;
        let b = 2.0 * unit(self.seed, SALT_MEANDER ^ 0x100, g.0, g.1) - 1.0;
        let amplitude = MEANDER_AMPLITUDE * len;
        let mut line = [[0.0; 2]; SEGMENTS + 1];
        for (i, point) in line.iter_mut().enumerate() {
            let t = i as f64 / SEGMENTS as f64;
            let (t2, t3) = (t * t, t * t * t);
            let (h00, h10, h01, h11) = (
                2.0 * t3 - 3.0 * t2 + 1.0,
                t3 - 2.0 * t2 + t,
                -2.0 * t3 + 3.0 * t2,
                t3 - t2,
            );
            let wobble = amplitude
                * (a * (std::f64::consts::PI * t).sin()
                    + 0.5 * b * (2.0 * std::f64::consts::PI * t).sin());
            for k in 0..2 {
                point[k] = h00 * p0[k] + h10 * m0[k] + h01 * p1[k] + h11 * m1[k] + perp[k] * wobble;
            }
        }
        line
    }

    /// La rivière la plus proche du point (x, z) parmi les arêtes des 5 × 5 cellules voisines
    /// (distance à l'axe, à comparer à la largeur), pour les arêtes de cours d'eau seulement.
    pub fn river_near(&self, x: f64, z: f64) -> Option<RiverPoint> {
        let c = fine_cell(x, z);
        let mut best: Option<(f64, RiverPoint)> = None;
        for dz in -2..=2 {
            for dx in -2..=2 {
                let g = (c.0 + dx, c.1 + dz);
                let Some(Edge { next: n, line }) = self.river_edge(g) else {
                    continue;
                };
                let (distance, t, at) = nearest_on(&line, x, z);
                if distance > MAX_INFLUENCE {
                    continue;
                }
                let acc = self.accumulation(g);
                if acc < STREAM_MIN_ACCUMULATION {
                    continue;
                }
                let (w0, w1) = (river_width(acc), river_width(self.accumulation(n)));
                let level = self.level(g) as f64 + (self.level(n) - self.level(g)) as f64 * t;
                let mut width = w0 as f64 + (w1 - w0) as f64 * t;
                // L'embouchure s'évase : près du rivage, à un niveau voisin de la mer.
                let shore = self.geography.coast_distance_at(at[0], at[1]);
                let flare = (1.0 - smooth(0.0, MOUTH_REACH, shore))
                    * (1.0 - smooth(2.0, 10.0, level - SEA_LEVEL as f64))
                    * (acc as f64 / 40.0).min(1.0);
                width *= 1.0 + MOUTH_FLARE * flare;
                // Un ruisseau naît progressivement : étroit et sans vallée à sa source.
                let taper = smooth(
                    STREAM_MIN_ACCUMULATION as f64 * 0.8,
                    STREAM_MIN_ACCUMULATION as f64 * 4.0,
                    acc as f64,
                );
                width *= 0.45 + 0.55 * taper;
                let score = distance - width * 0.5;
                if best.is_none_or(|(s, _)| score < s) {
                    best = Some((
                        score,
                        RiverPoint {
                            distance: distance as f32,
                            width: width as f32,
                            level: level as f32,
                            accumulation: acc,
                            taper: taper as f32,
                            basin: self.basin_id(self.basin_of(g)),
                        },
                    ));
                }
            }
        }
        best.map(|(_, r)| r)
    }
}

/// Le point de la courbe le plus proche de (x, z) : distance, abscisse `t` dans `[0, 1]` de la
/// courbe, et le point lui-même.
fn nearest_on(line: &[[f64; 2]; SEGMENTS + 1], x: f64, z: f64) -> (f64, f64, [f64; 2]) {
    let mut best = (f64::MAX, 0.0, line[0]);
    for (i, w) in line.windows(2).enumerate() {
        let (sx, sz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let len2 = (sx * sx + sz * sz).max(1e-9);
        let u = (((x - w[0][0]) * sx + (z - w[0][1]) * sz) / len2).clamp(0.0, 1.0);
        let q = [w[0][0] + sx * u, w[0][1] + sz * u];
        let d = (x - q[0]).hypot(z - q[1]);
        if d < best.0 {
            best = (d, (i as f64 + u) / SEGMENTS as f64, q);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_starts_and_ends_at_the_nodes_and_is_the_same_for_every_call() {
        let h = Hydrology::new(42);
        let mut checked = 0;
        for i in 0..200 {
            let g = (i % 20 - 10, i / 20 - 5);
            let Some(n) = h.next(g) else { continue };
            let line = h.river_polyline(g, n);
            assert_eq!(line, Hydrology::new(42).river_polyline(g, n));
            let (p0, p1) = (h.node(g), h.node(n));
            assert!((line[0][0] - p0[0]).hypot(line[0][1] - p0[1]) < 1e-6);
            let end = line[SEGMENTS];
            assert!((end[0] - p1[0]).hypot(end[1] - p1[1]) < 1e-6);
            // La courbe reste près de sa corde (courbure et méandres bornés).
            let len = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
            for p in &line {
                let off = ((p[0] - p0[0]) * (p1[1] - p0[1]) - (p[1] - p0[1]) * (p1[0] - p0[0]))
                    .abs()
                    / len.max(1e-9);
                assert!(off < 0.5 * len + 1.0, "écart {off} pour une corde de {len}");
            }
            assert!(len < 3.0 * crate::generator::FINE_SIZE as f64);
            checked += 1;
        }
        assert!(checked > 20);
    }
}
