//! La géographie macro du générateur v2 (prototype, lot 2) : continents, côtes, chaînes de
//! montagnes et profils de relief, en coordonnées monde.
//!
//! Rien ici ne dépend d'un chunk ni d'une région : [`Geography::sample`] est une fonction pure de
//! `(graine, x, z)`. Deux régions qui se touchent lisent donc la même valeur à leur frontière, et
//! l'ordre des appels, les threads ou un cache n'y changent rien.
//!
//! # Échelles indépendantes (en blocs, calées sur un monde de 192 blocs de haut)
//!
//! | Échelle | Longueur d'onde | Rôle |
//! | --- | --- | --- |
//! | continents | environ 6 400 | champ terre/mer [`Geography::continent`] |
//! | côtes | environ 500 | golfes, caps, archipels : module la ligne de rivage |
//! | chaînes | 2 400 en travers (allongées ×2,2), 1 700 (secondaires) | axes de crêtes, cols, contreforts |
//! | vallées | 64 à 512 | l'hydrologie ([`super::hydrology`]) |
//! | détail | environ 150 | bosses fines, qui modulent sans casser la hiérarchie |
//!
//! # Le champ terre/mer et la distance au rivage
//!
//! Le champ continental `C` est un bruit à grande longueur d'onde, déformé par un « warp » lent,
//! plus un étage de détail pour les côtes. La terre est où `C > 0`. La distance au rivage est
//! **estimée** par `C / |∇C|` (différences finies sur 48 blocs) : exacte pour un champ linéaire,
//! approximative près des extrema. Elle est bornée à ±[`COAST_DISTANCE_MAX`]. Les courbes de
//! profondeur et de plaine côtière en dépendent : plateau continental, plage, plaine, plateau
//! intérieur.
//!
//! # Les chaînes
//!
//! Les **axes** sont les lignes où un bruit lent `m` s'annule ; la distance à l'axe est estimée
//! par `|m| / |∇m|`. Une chaîne n'existe que là où un second champ lent « intensité » est positif :
//! ses extrémités sont donc des fins de chaîne, pas des coupures. Le long d'un axe, la hauteur
//! varie avec un troisième champ : ses creux sont les **cols**. Un second axe de plus courte
//! portée, limité au voisinage du premier, donne des **crêtes secondaires** et des contreforts.
//! Le profil perpendiculaire est une courbe cubique monotone plaine → piémont → massif
//! ([`PIEDMONT_PROFILE`]), sans palier.
//!
//! La hauteur au-dessus de la mer subit un écrêtage **doux** (exponentiel à partir de
//! [`SOFT_CAP_KNEE`]) : aucun sommet n'est tronqué à plat, mais la somme reste sous la hauteur du
//! monde. Pas de simulation tectonique.

use super::macro_noise::{fbm, profile, smooth};
use super::{MAX_HEIGHT, SEA_LEVEL};

/// La distance au rivage est bornée à cette valeur (en blocs), de chaque côté.
pub const COAST_DISTANCE_MAX: f64 = 2600.0;

const SALT_WARP_X: u64 = 0x11;
const SALT_WARP_Z: u64 = 0x12;
const SALT_CONTINENT: u64 = 0x13;
const SALT_COAST: u64 = 0x14;
const SALT_AXIS_WARP_X: u64 = 0x21;
const SALT_AXIS_WARP_Z: u64 = 0x22;
const SALT_AXIS: u64 = 0x23;
const SALT_AXIS2_WARP_X: u64 = 0x24;
const SALT_AXIS2_WARP_Z: u64 = 0x25;
const SALT_AXIS2: u64 = 0x26;
const SALT_STRENGTH: u64 = 0x27;
const SALT_COL: u64 = 0x28;
const SALT_WIDTH: u64 = 0x29;
const SALT_ANGLE: u64 = 0x2a;
const SALT_DETAIL: u64 = 0x31;

/// Longueur d'onde des continents, en blocs.
const CONTINENT_WAVELENGTH: f64 = 6400.0;
/// Le « warp » des continents : longueur d'onde et déplacement maximal.
const CONTINENT_WARP: (f64, f64) = (3000.0, 700.0);
/// Longueur d'onde et amplitude de l'étage de détail des côtes.
const COAST_DETAIL: (f64, f64) = (520.0, 0.11);
/// Écart du champ continental : la terre est où `C > 0`. Ajusté pour environ 40 % de terres.
const LAND_BIAS: f64 = 0.04;
const CONTINENT_GAIN: f64 = 1.45;

/// Planchers doux des gradients des estimations de distance (par bloc).
const AXIS_GRADIENT_FLOOR: f64 = 2.5e-4;
const COAST_GRADIENT_FLOOR: f64 = 1.5e-4;
/// Gradient de référence du champ continental au large (par bloc), pour des fonds marins lisses.
const SEA_GRADIENT_REFERENCE: f64 = 4.0e-4;

/// Hauteur maximale d'une chaîne au-dessus de son piémont (blocs), avant écrêtage doux.
const CHAIN_AMPLITUDE: f64 = 100.0;
/// Les chaînes : longueur d'onde de l'axe principal et des crêtes secondaires.
const AXIS_WAVELENGTH: f64 = 2400.0;
const AXIS2_WAVELENGTH: f64 = 1700.0;
/// Demi-largeur moyenne d'une chaîne (blocs), de l'axe à la fin du piémont.
const CHAIN_HALF_WIDTH: f64 = 1100.0;
/// Écrêtage doux : au-delà du genou, la hauteur se rapproche asymptotiquement du plafond.
pub const SOFT_CAP_KNEE: f64 = 85.0;
pub const SOFT_CAP_TOP: f64 = 120.0;

/// Profil perpendiculaire d'une chaîne, de l'axe (`t = 0`) à la fin du piémont (`t = 1`), en
/// fraction de la hauteur : crête, flancs du massif, piémont doux, puis plaine.
const PIEDMONT_PROFILE: [(f64, f64); 7] = [
    (0.0, 1.0),
    (0.08, 0.80),
    (0.22, 0.52),
    (0.40, 0.26),
    (0.60, 0.10),
    (0.80, 0.025),
    (1.0, 0.0),
];

/// Altitude de base (blocs au-dessus de la mer) selon la distance signée au rivage : fonds
/// marins (négatifs : plateau continental puis talus), plage, plaine côtière, plateau intérieur.
const BASE_PROFILE: [(f64, f64); 11] = [
    (-2600.0, -42.0),
    (-1500.0, -38.0),
    (-800.0, -30.0),
    (-350.0, -18.0),
    (-120.0, -7.0),
    (0.0, 0.0),
    (70.0, 2.5),
    (300.0, 8.0),
    (800.0, 15.0),
    (1600.0, 25.0),
    (2600.0, 36.0),
];

/// La famille de relief d'un point : ce que l'on doit lire sur une carte et que les lots suivants
/// (surface, biomes) pourront exploiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReliefFamily {
    /// Fond marin au-delà du plateau continental.
    DeepSea,
    /// Plateau continental : mer peu profonde.
    Shelf,
    /// Bande côtière : plage, estuaires, côtes basses.
    Coast,
    /// Plaine : peu de relief, zone constructible.
    Plain,
    /// Plateau intérieur surélevé, sans chaîne.
    Plateau,
    /// Piémont : contreforts et collines au pied d'une chaîne.
    Foothills,
    /// Massif : flancs et crêtes.
    Massif,
}

impl ReliefFamily {
    pub const ALL: [ReliefFamily; 7] = [
        ReliefFamily::DeepSea,
        ReliefFamily::Shelf,
        ReliefFamily::Coast,
        ReliefFamily::Plain,
        ReliefFamily::Plateau,
        ReliefFamily::Foothills,
        ReliefFamily::Massif,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::DeepSea => "mer profonde",
            Self::Shelf => "plateau continental",
            Self::Coast => "côte",
            Self::Plain => "plaine",
            Self::Plateau => "plateau",
            Self::Foothills => "piémont",
            Self::Massif => "massif",
        }
    }
}

/// Ce que la géographie sait d'un point (x, z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoSample {
    /// Le champ continental : positif sur la terre, négatif en mer.
    pub continent: f32,
    /// Distance estimée au rivage, en blocs : positive sur la terre, négative en mer.
    pub coast_distance: f32,
    /// Distance estimée à l'axe de chaîne le plus proche, en blocs (bornée).
    pub axis_distance: f32,
    /// Distance estimée à l'axe de la crête secondaire la plus proche, en blocs (bornée ; la borne
    /// hors du voisinage de la chaîne). Les escarpements du relief s'alignent dessus.
    pub ridge_distance: f32,
    /// L'intensité locale de la chaîne, de 0 (aucune) à 1 : cols et fins de chaîne comprises.
    pub chain_strength: f32,
    /// La hauteur ajoutée par les chaînes (axe principal et crêtes secondaires), en blocs.
    pub chain_height: f32,
    /// L'altitude de la géographie macro, sans détail fin, en blocs (y du sol, flottant).
    pub macro_elevation: f32,
    /// L'altitude avec le détail fin, en blocs.
    pub elevation: f32,
    pub family: ReliefFamily,
}

/// La géographie d'une graine.
#[derive(Debug, Clone, Copy)]
pub struct Geography {
    seed: u32,
}

/// Les paramètres d'un champ d'axes (voir [`Geography::axis_distance`]).
struct AxisField {
    salt: u64,
    wavelength: f64,
    aspect: f64,
    /// Deux orientations (radians) fondues par `blend` (0 : la première, 1 : la seconde).
    angles: (f64, f64),
    blend: f64,
    octaves: u32,
    warp: (u64, u64, f64, f64),
}

/// Les résultats intermédiaires communs à l'altitude macro et à l'échantillon complet.
struct Core {
    continent: f64,
    coast_distance: f64,
    axis_distance: f64,
    ridge_distance: f64,
    strength: f64,
    chain: f64,
    macro_height: f64,
}

impl Geography {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    /// Le point (x, z) déformé par un bruit lent de longueur d'onde `wavelength` et de
    /// déplacement maximal `amplitude` (sels `sx`, `sz`).
    fn warped(
        &self,
        x: f64,
        z: f64,
        sx: u64,
        sz: u64,
        wavelength: f64,
        amplitude: f64,
    ) -> (f64, f64) {
        (
            x + fbm(self.seed, sx, x / wavelength, z / wavelength, 2, 0.5) * amplitude,
            z + fbm(self.seed, sz, x / wavelength, z / wavelength, 2, 0.5) * amplitude,
        )
    }

    /// La part lisse du champ continental (sans le détail des côtes), centrée sur le rivage.
    fn continent_smooth(&self, x: f64, z: f64) -> f64 {
        let (px, pz) = self.warped(
            x,
            z,
            SALT_WARP_X,
            SALT_WARP_Z,
            CONTINENT_WARP.0,
            CONTINENT_WARP.1,
        );
        let big = fbm(
            self.seed,
            SALT_CONTINENT,
            px / CONTINENT_WAVELENGTH,
            pz / CONTINENT_WAVELENGTH,
            4,
            0.5,
        );
        big * CONTINENT_GAIN - LAND_BIAS
    }

    /// Le détail des côtes, entre -1 et 1 environ.
    fn coast_detail(&self, x: f64, z: f64) -> f64 {
        let (px, pz) = self.warped(
            x,
            z,
            SALT_WARP_X,
            SALT_WARP_Z,
            CONTINENT_WARP.0,
            CONTINENT_WARP.1,
        );
        fbm(
            self.seed,
            SALT_COAST,
            px / COAST_DETAIL.0,
            pz / COAST_DETAIL.0,
            3,
            0.5,
        )
    }

    /// Le champ continental : positif sur la terre, négatif en mer. Le détail des côtes ne
    /// s'ajoute qu'au voisinage du rivage (il s'éteint au large et à l'intérieur) : les fonds
    /// marins et les plateaux restent lisses.
    pub fn continent(&self, x: f64, z: f64) -> f64 {
        let smooth_part = self.continent_smooth(x, z);
        let weight = (-smooth_part.abs() / 0.10).exp();
        smooth_part + weight * COAST_DETAIL.1 * self.coast_detail(x, z)
    }

    /// La distance estimée au rivage (blocs, positive sur la terre) et le champ continental.
    /// Le gradient vient de la part lisse : l'estimation n'est pas bruitée par le détail.
    fn coast(&self, x: f64, z: f64) -> (f64, f64) {
        const H: f64 = 48.0;
        let s0 = self.continent_smooth(x, z);
        let gx = (self.continent_smooth(x + H, z) - s0) / H;
        let gz = (self.continent_smooth(x, z + H) - s0) / H;
        let mut gradient = (gx * gx + gz * gz + COAST_GRADIENT_FLOOR * COAST_GRADIENT_FLOOR).sqrt();
        if s0 < 0.0 {
            // Au large, le gradient du « warp » tord l'estimation en tourbillons visibles sur les
            // fonds marins : il s'efface au profit d'une valeur de référence (le rivage, lui,
            // garde son estimation exacte).
            let far = smooth(0.05, 0.30, -s0);
            gradient = gradient * (1.0 - far) + SEA_GRADIENT_REFERENCE * far;
        }
        let c = s0 + (-s0.abs() / 0.10).exp() * COAST_DETAIL.1 * self.coast_detail(x, z);
        (
            (c / gradient).clamp(-COAST_DISTANCE_MAX, COAST_DISTANCE_MAX),
            c,
        )
    }

    /// La distance estimée au rivage en (x, z), en blocs (positive sur la terre) : le même nombre que
    /// [`GeoSample::coast_distance`], pour un coût de quelques centaines de nanosecondes.
    pub fn coast_distance_at(&self, x: f64, z: f64) -> f64 {
        self.coast(x, z).0
    }

    /// L'intensité des chaînes en (x, z) : où elles existent, et leurs cols.
    fn strength(&self, x: f64, z: f64, coast_distance: f64) -> f64 {
        let belt = smooth(
            -0.28,
            0.22,
            fbm(self.seed, SALT_STRENGTH, x / 7000.0, z / 7000.0, 2, 0.5),
        );
        let landward = smooth(-700.0, 500.0, coast_distance);
        // Les cols : de longues sections de l'axe où la crête s'abaisse de moitié.
        let col = 0.5
            + 0.5
                * smooth(
                    -0.55,
                    0.35,
                    fbm(self.seed, SALT_COL, x / 950.0, z / 950.0, 2, 0.5),
                );
        belt * landward * col
    }

    /// Distance estimée au zéro d'un bruit d'axe, étiré le long de la direction `angle` (rapport
    /// `aspect` entre la longueur d'onde le long de l'axe et celle en travers) : les zéros sont des
    /// lignes allongées, des chaînes, pas des anneaux.
    fn axis_distance(&self, x: f64, z: f64, axis: &AxisField) -> f64 {
        const H: f64 = 20.0;
        let (qx, qz) = self.warped(x, z, axis.warp.0, axis.warp.1, axis.warp.2, axis.warp.3);
        // Rotations de l'espace par des angles **constants** : une rotation dont l'angle dépend de
        // la position tournerait autour de l'origine et tordrait tout le monde loin d'elle. La
        // lente variation d'orientation vient du fondu entre deux champs d'orientations fixes.
        let rotated = |angle: f64, a: f64, b: f64| {
            let (sin, cos) = angle.sin_cos();
            fbm(
                self.seed,
                axis.salt,
                (cos * a + sin * b) / (axis.wavelength * axis.aspect),
                (-sin * a + cos * b) / axis.wavelength,
                axis.octaves,
                0.5,
            )
        };
        let m = |a: f64, b: f64| {
            (1.0 - axis.blend) * rotated(axis.angles.0, a, b)
                + axis.blend * rotated(axis.angles.1, a, b)
        };
        let m0 = m(qx, qz);
        let gx = (m(qx + H, qz) - m0) / H;
        let gz = (m(qx, qz + H) - m0) / H;
        // Plancher doux du gradient : près d'un col du bruit d'axe, la distance estimée ne
        // s'envole pas (elle varierait de plusieurs centaines de blocs sur quelques blocs).
        (m0.abs() / (gx * gx + gz * gz + AXIS_GRADIENT_FLOOR * AXIS_GRADIENT_FLOOR).sqrt())
            .min(CHAIN_HALF_WIDTH * 1.6)
    }

    fn core(&self, x: f64, z: f64) -> Core {
        let (coast_distance, continent) = self.coast(x, z);
        let strength = self.strength(x, z, coast_distance);
        let half_width = CHAIN_HALF_WIDTH
            * (0.8
                + 0.4 * (0.5 + 0.5 * fbm(self.seed, SALT_WIDTH, x / 3500.0, z / 3500.0, 1, 0.5)));
        let mut chain = 0.0;
        let mut axis = CHAIN_HALF_WIDTH * 1.6;
        let mut ridge = CHAIN_HALF_WIDTH * 1.6;
        if strength > 0.002 {
            // L'orientation des chaînes passe lentement d'une direction à l'autre : des arcs.
            let blend = smooth(
                -0.4,
                0.4,
                fbm(self.seed, SALT_ANGLE, x / 8000.0, z / 8000.0, 2, 0.5),
            );
            axis = self.axis_distance(
                x,
                z,
                &AxisField {
                    salt: SALT_AXIS,
                    wavelength: AXIS_WAVELENGTH,
                    aspect: 2.2,
                    angles: (0.45, 1.6),
                    blend,
                    octaves: 2,
                    warp: (SALT_AXIS_WARP_X, SALT_AXIS_WARP_Z, 2200.0, 380.0),
                },
            );
            let main = profile(axis / half_width, &PIEDMONT_PROFILE);
            // Crêtes secondaires : un second axe, plus court, limité au voisinage du principal.
            let near = smooth(half_width, 0.3 * half_width, axis);
            let mut secondary = 0.0;
            if near > 0.0 {
                let d2 = self.axis_distance(
                    x,
                    z,
                    &AxisField {
                        salt: SALT_AXIS2,
                        wavelength: AXIS2_WAVELENGTH,
                        aspect: 1.3,
                        angles: (1.5, 0.2),
                        blend,
                        octaves: 1,
                        warp: (SALT_AXIS2_WARP_X, SALT_AXIS2_WARP_Z, 1800.0, 300.0),
                    },
                );
                ridge = d2;
                let u = (d2 / (0.26 * half_width)).min(1.0);
                secondary = 0.32 * near * (1.0 - u) * (1.0 - u);
            }
            chain = CHAIN_AMPLITUDE * strength * (main + secondary);
        }
        let base = profile(coast_distance, &BASE_PROFILE);
        let macro_height = soft_cap(base + chain);
        Core {
            continent,
            coast_distance,
            axis_distance: axis,
            ridge_distance: ridge,
            strength,
            chain,
            macro_height,
        }
    }

    /// L'altitude de la géographie macro (sans détail fin), en blocs. C'est ce que lit
    /// l'hydrologie : lisse sur plus de 100 blocs.
    pub fn macro_elevation(&self, x: f64, z: f64) -> f32 {
        (SEA_LEVEL as f64 + self.core(x, z).macro_height) as f32
    }

    /// Tout ce que la géographie sait du point (x, z).
    pub fn sample(&self, x: f64, z: f64) -> GeoSample {
        let c = self.core(x, z);
        // Le détail fin module : plus fort en montagne, absent au large.
        let relief = 5.5 + 0.12 * c.chain;
        let landward = smooth(-300.0, 50.0, c.coast_distance);
        let detail = relief * landward * fbm(self.seed, SALT_DETAIL, x / 150.0, z / 150.0, 3, 0.5);
        let sea = SEA_LEVEL as f64;
        let elevation = (sea + c.macro_height + detail).clamp(3.0, MAX_HEIGHT as f64 - 1.0);
        let above = c.macro_height;
        let family = if above < -28.0 {
            ReliefFamily::DeepSea
        } else if above < 0.0 {
            ReliefFamily::Shelf
        } else if c.coast_distance < 90.0 {
            ReliefFamily::Coast
        } else if c.chain >= 48.0 {
            ReliefFamily::Massif
        } else if c.chain >= 14.0 {
            ReliefFamily::Foothills
        } else if c.coast_distance > 1500.0 && above > 20.0 {
            ReliefFamily::Plateau
        } else {
            ReliefFamily::Plain
        };
        GeoSample {
            continent: c.continent as f32,
            coast_distance: c.coast_distance as f32,
            axis_distance: c.axis_distance as f32,
            ridge_distance: c.ridge_distance as f32,
            chain_strength: c.strength as f32,
            chain_height: c.chain as f32,
            macro_elevation: (sea + c.macro_height) as f32,
            elevation: elevation as f32,
            family,
        }
    }
}

/// Écrêtage doux : identité jusqu'au genou, puis approche exponentielle du plafond. Continu et
/// strictement croissant : un sommet plus haut reste plus haut, aucun plateau n'est créé.
pub(super) fn soft_cap(h: f64) -> f64 {
    if h <= SOFT_CAP_KNEE {
        h
    } else {
        let span = SOFT_CAP_TOP - SOFT_CAP_KNEE;
        SOFT_CAP_KNEE + span * (1.0 - (-(h - SOFT_CAP_KNEE) / span).exp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_cap_is_monotone_continuous_and_bounded() {
        let mut last = f64::MIN;
        for i in -500..4000 {
            let v = soft_cap(i as f64 * 0.1);
            assert!(v > last, "strictement croissant à {i}");
            assert!(v < SOFT_CAP_TOP);
            last = v;
        }
        assert_eq!(soft_cap(40.0), 40.0);
        assert!((soft_cap(SOFT_CAP_KNEE + 1e-9) - SOFT_CAP_KNEE).abs() < 1e-6);
    }

    #[test]
    fn sample_is_pure_and_ordered_consistently() {
        let g = Geography::new(42);
        let points = [(-3000.0, 1200.0), (0.0, 0.0), (5000.5, -4100.25)];
        let forward: Vec<_> = points.iter().map(|&(x, z)| g.sample(x, z)).collect();
        let backward: Vec<_> = points.iter().rev().map(|&(x, z)| g.sample(x, z)).collect();
        for (a, b) in forward.iter().zip(backward.iter().rev()) {
            assert_eq!(a, b);
        }
        assert_eq!(g.sample(0.0, 0.0), Geography::new(42).sample(0.0, 0.0));
        assert_ne!(
            g.sample(10.0, 10.0).continent,
            Geography::new(43).sample(10.0, 10.0).continent
        );
    }

    #[test]
    fn land_sea_split_follows_the_coast_distance_sign() {
        let g = Geography::new(7);
        let sea = SEA_LEVEL as f32;
        for i in 0..400 {
            let (x, z) = (
                i as f64 * 97.0 - 20_000.0,
                (i * i % 311) as f64 * 131.0 - 20_000.0,
            );
            let s = g.sample(x, z);
            if s.coast_distance < -1.0 && s.chain_height < 0.5 {
                assert!(s.macro_elevation < sea, "en mer, sous le niveau marin");
            }
            if s.coast_distance > 1.0 {
                assert!(
                    s.macro_elevation >= sea - 1e-3,
                    "sur terre, au-dessus de la mer"
                );
            }
            assert!(s.elevation >= 3.0 && s.elevation <= MAX_HEIGHT as f32);
        }
    }
}
