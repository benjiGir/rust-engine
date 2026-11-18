//! La densité 3D du générateur v2 : solide ou vide, bloc par bloc.
//!
//! # La formule
//!
//! Une colonne a un relief `top` (le dernier bloc plein du relief continu, quantifié : voir
//! `v2_context.rs`) et, si elle est **éligible**, une amplitude `A` de blocs (au plus
//! [`AMPLITUDE`]). La densité du bloc d'altitude `y` est
//!
//! ```text
//! D = (top + 0,5 - y)  -  A × s(prof) × max(0, 0,15 - n(x, y, z))  +  0,9 × falaise × dureté(y)
//! ```
//!
//! et le bloc est solide quand `D > 0`. Sans amplitude, `D > 0` revient exactement à `y <= top` :
//! le relief continu est la **seule** règle des colonnes réservées (lits, berges, lacs), de la mer
//! et de toutes les plaines : aucun bruit ne peut y ouvrir ni fermer quoi que ce soit.
//!
//! - `n` est un bruit 3D (deux octaves de Perlin, étiré horizontalement) évalué sur une grille de
//!   4 blocs, une fois pour la verticale, puis **interpolé** (trilinéaire) ;
//! - `s(prof)` croît de 0 à 1 entre 0 et 8 blocs sous la surface : la peau du relief reste intacte,
//!   les poches se creusent dessous. Là où le bruit est très négatif (`n < -0,4` environ), la poche
//!   dépasse la profondeur (`A × s × 1,15 > prof`) et le vide apparaît ; exposé par une pente
//!   raide, il devient un **surplomb** (la peau reste suspendue) ou, traversant une crête, une
//!   **arche** ;
//! - l'**entaille** : au pied d'une paroi (une colonne voisine à 1 ou 2 blocs plus basse d'au moins
//!   3 blocs), la moitié basse de la partie visible est creusée où `n < 0,1` : la partie haute
//!   surplombe. C'est ce qui fait de vrais surplombs sur les parois des massifs terrassés ;
//! - le terme `dureté` (+1 pour la pierre, -1 pour les grès, +0,4 pour la roche volcanique) fait
//!   saillir les couches dures et reculer les couches tendres sur une falaise : la *résistance
//!   apparente* des strates (voir `surface.rs`). Il ne vaut que sur les colonnes à falaise.
//!
//! Il n'y a jamais de bloc au-dessus du relief : rien ne flotte au-dessus du sol. Une poche est
//! plus profonde que 5 blocs dès que le bruit la creuse, et le creusement sous le niveau de la mer
//! est interdit (pas d'eau qui coule dans une falaise côtière).
//!
//! # Éligibilité
//!
//! L'amplitude est `AMPLITUDE × w`, avec `w` = pente raide (de 0,6 à 1,4) × famille de relief
//! (massifs et piémonts, ou pente extrême) × altitude (hors des côtes et des fonds) × `(1 - réservation)`
//! × `(1 - 0,7 × vallée)`. Les plaines, plateaux, côtes basses et mers n'ont jamais de surplomb.

use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use super::v2::ColumnContext;
use crate::chunk::CHUNK_SIZE;

/// Pas de la grille du bruit de densité, en blocs.
pub(super) const NOISE_STEP: i32 = 4;
/// Points de la grille par côté de la colonne de chunks.
pub(super) const GRID: usize = CHUNK_SIZE / NOISE_STEP as usize + 1;
/// Amplitude maximale des poches, en blocs.
pub(super) const AMPLITUDE: f32 = 16.0;
/// Profondeur au-delà de laquelle une poche ne peut plus apparaître (blocs).
pub(super) const MAX_CARVE: i32 = 18;
/// Lignes lues sous et au-dessus du chunk : le sable ne doit pas tomber (bloc dessous) et la
/// profondeur de sol se compte depuis l'air (jusqu'à 8 blocs au-dessus).
pub(super) const BELOW: i32 = 1;
pub(super) const ABOVE: i32 = 8;
pub(super) const SPAN: usize = CHUNK_SIZE + (BELOW + ABOVE) as usize;

/// Le bruit 3D des poches : deux octaves de Perlin, longueurs d'onde de 18 blocs en x et z et de
/// 11 blocs en y.
pub(super) struct DensityNoise {
    field: Fbm<Perlin>,
}

impl DensityNoise {
    pub fn new(seed: u32) -> Self {
        Self {
            field: Fbm::<Perlin>::new(seed.wrapping_add(31))
                .set_octaves(2)
                .set_frequency(1.0)
                .set_lacunarity(2.0)
                .set_persistence(0.5),
        }
    }

    fn at(&self, x: i32, y: i32, z: i32) -> f32 {
        let p = [x as f64 / 18.0, y as f64 / 11.0, z as f64 / 18.0];
        self.field.get(p).clamp(-1.0, 1.0) as f32
    }
}

/// Les valeurs du bruit aux points d'une grille de 4 blocs, sur toute la hauteur utile d'une
/// colonne de chunks (une seule fois par verticale, partagée par ses six chunks).
#[derive(Debug)]
pub(super) struct NoiseGrid {
    /// y du premier plan, multiple de 4.
    y0: i32,
    layers: usize,
    /// `(k × GRID + j) × GRID + i` : plan `k` en y, `j` en z, `i` en x.
    values: Vec<f32>,
}

impl NoiseGrid {
    /// La grille de la colonne de chunks d'origine `(x0, z0)`, de `y_min` à `y_max` compris.
    pub fn new(noise: &DensityNoise, x0: i32, z0: i32, y_min: i32, y_max: i32) -> Self {
        let y0 = y_min.div_euclid(NOISE_STEP) * NOISE_STEP;
        let layers = ((y_max - y0).div_euclid(NOISE_STEP) + 2).max(2) as usize;
        let mut values = Vec::with_capacity(layers * GRID * GRID);
        for k in 0..layers {
            for j in 0..GRID {
                for i in 0..GRID {
                    values.push(noise.at(
                        x0 + i as i32 * NOISE_STEP,
                        y0 + k as i32 * NOISE_STEP,
                        z0 + j as i32 * NOISE_STEP,
                    ));
                }
            }
        }
        Self { y0, layers, values }
    }

    /// Le bruit interpolé au bloc `(lx, y, lz)` (local en x et z, absolu en y).
    pub fn sample(&self, lx: usize, y: i32, lz: usize) -> f32 {
        let s = NOISE_STEP as usize;
        let ry = (y - self.y0).max(0) as usize;
        let k = (ry / s).min(self.layers - 2);
        let fy = (ry - k * s) as f32 / s as f32;
        let (i, j) = (lx / s, lz / s);
        let (fx, fz) = ((lx % s) as f32 / s as f32, (lz % s) as f32 / s as f32);
        let at = |di: usize, dk: usize, dj: usize| {
            self.values[((k + dk) * GRID + (j + dj)) * GRID + (i + di)]
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let x00 = lerp(at(0, 0, 0), at(1, 0, 0), fx);
        let x10 = lerp(at(0, 1, 0), at(1, 1, 0), fx);
        let x01 = lerp(at(0, 0, 1), at(1, 0, 1), fx);
        let x11 = lerp(at(0, 1, 1), at(1, 1, 1), fx);
        lerp(lerp(x00, x10, fy), lerp(x01, x11, fy), fz)
    }
}

/// Les bornes en y de la grille de bruit qu'exige une colonne d'amplitude non nulle et de relief
/// `top` : de la plus profonde poche possible à la surface.
pub(super) fn noise_span(top: i32) -> (i32, i32) {
    (top - MAX_CARVE - 2, top + 1)
}

/// 0 à `edge0`, 1 à `edge1`, courbe en S entre les deux.
fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// La densité d'un bloc d'une colonne éligible (voir la documentation du module).
pub(super) fn density(
    top: i32,
    y: i32,
    amplitude: f32,
    cliff: f32,
    n: f32,
    hardness: f32,
    drop: u8,
) -> f32 {
    let depth = (top - y) as f32 + 0.5;
    // L'entaille d'une falaise : là où une colonne voisine (à 1 ou 2 blocs) est plus basse de
    // `drop` blocs, le pied de la paroi (la moitié basse de la partie visible) est creusé où le
    // bruit est bas : la partie haute surplombe l'entaille.
    if drop >= 3 && depth > 0.5 * drop as f32 + 0.5 && depth < drop as f32 && n < 0.1 {
        return -1.0;
    }
    let pocket = amplitude * smooth(0.0, 3.0, depth) * (0.15 - n).max(0.0);
    depth - pocket + 0.9 * cliff * hardness
}

/// Solide ou vide, pour `SPAN` blocs de la colonne `(lx, lz)` à partir de `origin_y - BELOW`.
pub(super) fn solid_column(
    ctx: &ColumnContext,
    strata: &super::surface::StrataColumn,
    lx: usize,
    lz: usize,
    origin_y: i32,
) -> [bool; SPAN] {
    let core = ctx.core_at(lx, lz);
    let top = ctx.wide_at(lx as i32, lz as i32).top as i32;
    let mut out = [false; SPAN];
    let base = origin_y - BELOW;
    if core.amplitude <= 0.0 {
        for (i, cell) in out.iter_mut().enumerate() {
            *cell = base + i as i32 <= top;
        }
        return out;
    }
    let grid = ctx
        .noise
        .as_ref()
        .expect("une colonne à surplombs a une grille de bruit");
    for (i, cell) in out.iter_mut().enumerate() {
        let y = base + i as i32;
        *cell = if y > top {
            false
        } else if y <= super::SEA_LEVEL + 1 || y <= top - MAX_CARVE {
            // La peau profonde et le niveau de la mer ne se creusent pas.
            true
        } else {
            let n = grid.sample(lx, y, lz);
            density(
                top,
                y,
                core.amplitude,
                core.cliff,
                n,
                strata.hardness(y),
                core.drop,
            ) > 0.0
        };
    }
    out
}

/// Le nombre de blocs solides consécutifs au-dessus de l'indice `idx` : la profondeur sous l'air.
pub(super) fn depth_below_air(solid: &[bool; SPAN], idx: usize) -> i32 {
    solid[idx + 1..].iter().take_while(|&&s| s).count() as i32
}

/// Le bloc solide le plus haut d'une colonne éligible : on descend du relief jusqu'au premier
/// solide (une poche creusée sous la peau ne s'ouvre jamais au sommet : au plus quelques blocs).
pub(super) fn real_top(
    grid: &NoiseGrid,
    strata: &super::surface::StrataColumn,
    lx: usize,
    lz: usize,
    cell: &super::v2::CoreCell,
) -> i32 {
    let top = cell.top as i32;
    for y in (top - MAX_CARVE..=top).rev() {
        if y <= super::SEA_LEVEL + 1 || y <= top - MAX_CARVE {
            return y;
        }
        let n = grid.sample(lx, y, lz);
        let d = density(
            top,
            y,
            cell.amplitude,
            cell.cliff,
            n,
            strata.hardness(y),
            cell.drop,
        );
        if d > 0.0 {
            return y;
        }
    }
    top - MAX_CARVE
}
