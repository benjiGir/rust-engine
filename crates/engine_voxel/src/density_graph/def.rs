//! La description d'un graphe de fonctions de densité, telle qu'on l'écrit en RON.
//!
//! ```ron
//! (
//!     noises: {
//!         "relief": (first_octave: -7, amplitudes: [1.0, 1.0, 0.5]),
//!     },
//!     functions: {
//!         // Plein en bas, vide en haut, avec des bosses.
//!         "terrain": Add(
//!             YClampedGradient(from_y: 0, to_y: 128, from_value: 1.0, to_value: -1.0),
//!             Noise(noise: "relief", xz_scale: 1.0, y_scale: 0.5),
//!         ),
//!         "final": Interpolated(Ref("terrain")),
//!     },
//! )
//! ```
//!
//! Une densité positive est un bloc plein, négative du vide. `Ref("nom")` désigne une autre fonction
//! du fichier : elle n'est compilée qu'une fois, et partagée.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::noise::NoiseDef;

fn one() -> f64 {
    1.0
}

/// Une fonction de densité : une valeur en chaque position de bloc (x, y, z).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub enum DensityDef {
    Constant(f64),
    /// Une autre fonction du fichier, par son nom.
    Ref(String),
    /// Un bruit du fichier, lu en `(x × xz_scale, y × y_scale, z × xz_scale)`.
    Noise {
        noise: String,
        #[serde(default = "one")]
        xz_scale: f64,
        #[serde(default = "one")]
        y_scale: f64,
    },
    /// Un bruit lu à une position décalée par trois autres fonctions (les « déformations » du relief).
    ShiftedNoise {
        noise: String,
        #[serde(default = "one")]
        xz_scale: f64,
        #[serde(default = "one")]
        y_scale: f64,
        shift_x: Box<DensityDef>,
        shift_y: Box<DensityDef>,
        shift_z: Box<DensityDef>,
    },
    Add(Box<DensityDef>, Box<DensityDef>),
    Mul(Box<DensityDef>, Box<DensityDef>),
    Min(Box<DensityDef>, Box<DensityDef>),
    Max(Box<DensityDef>, Box<DensityDef>),
    Clamp {
        input: Box<DensityDef>,
        min: f64,
        max: f64,
    },
    Abs(Box<DensityDef>),
    Square(Box<DensityDef>),
    Cube(Box<DensityDef>),
    /// La moitié de la valeur quand elle est négative.
    HalfNegative(Box<DensityDef>),
    /// Le quart de la valeur quand elle est négative.
    QuarterNegative(Box<DensityDef>),
    /// Bornée à [-1, 1] puis adoucie : `c / 2 - c³ / 24`.
    Squeeze(Box<DensityDef>),
    /// De `from_value` à `y = from_y` jusqu'à `to_value` à `y = to_y`, constante au-delà.
    YClampedGradient {
        from_y: i32,
        to_y: i32,
        from_value: f64,
        to_value: f64,
    },
    Spline(SplineDef),
    /// Calculée aux coins des cellules (4 × 8 × 4 blocs) et interpolée entre eux lors du
    /// remplissage d'un chunk : la part chère du relief ne coûte qu'aux coins.
    Interpolated(Box<DensityDef>),
    /// Calculée une fois par carré de 4 × 4 blocs (au coin, à y = 0) et commune à toute la hauteur.
    FlatCache(Box<DensityDef>),
    /// Gardée tant que (x, z) ne change pas (une colonne de blocs).
    Cache2d(Box<DensityDef>),
    /// Gardée tant que la position ne change pas (utile si elle sert plusieurs fois).
    CacheOnce(Box<DensityDef>),
    /// `when_in_range` si `input` est dans `[min_inclusive, max_exclusive)`, sinon `when_out_of_range`.
    RangeChoice {
        input: Box<DensityDef>,
        min_inclusive: f64,
        max_exclusive: f64,
        when_in_range: Box<DensityDef>,
        when_out_of_range: Box<DensityDef>,
    },
}

/// Une spline cubique : la valeur suit des points (abscisse, valeur, pente) le long d'une
/// coordonnée, elle-même une fonction de densité. Une valeur peut être une autre spline.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SplineDef {
    pub coordinate: Box<DensityDef>,
    pub points: Vec<SplinePoint>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SplinePoint {
    pub location: f64,
    pub value: SplineValue,
    #[serde(default)]
    pub derivative: f64,
}

/// Une valeur de spline : un nombre, ou une spline imbriquée (écrite comme une `SplineDef`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum SplineValue {
    Constant(f64),
    Spline(Box<SplineDef>),
}

/// Un fichier de densité : des bruits et des fonctions nommés.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct DensityFile {
    #[serde(default)]
    pub noises: BTreeMap<String, NoiseDef>,
    #[serde(default)]
    pub functions: BTreeMap<String, DensityDef>,
}
