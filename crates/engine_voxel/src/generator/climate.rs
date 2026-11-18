//! Le climat de la génération v2 (lot 5) : température et humidité déduites de la géographie.
//!
//! Le climat vient **après** la géographie et l'hydrologie : il les lit, il ne les modifie jamais
//! (le calcul est acyclique : relief -> climat -> biomes, jamais l'inverse). Chaque valeur est une
//! fonction pure de `(graine, x, z)` : aucun état, aucun cache, donc aucune couture entre chunks.
//!
//! # Température
//!
//! Un champ régional lent (8 000 blocs : de vastes régions climatiques), tempéré près des côtes
//! (la mer modère les extrêmes de 30 % au rivage), puis refroidi par l'altitude du relief macro
//! (0,0065 par bloc au-dessus de `SEA + 25` : un sommet de 100 blocs perd 0,5 à 0,65). La
//! température « au niveau de la mer » est conservée à part : elle règle la ligne des arbres.
//!
//! # Humidité et ombre pluviométrique
//!
//! Le vent dominant a une direction par graine, qui dérive lentement (12 000 blocs). L'air qui
//! arrive en un point a traversé les 1 280 blocs amont : on lit des altitudes macro
//! régulièrement espacées sur ce chemin, sur **trois chemins de front** (150 blocs d'écart) pour ne pas
//! tracer de sillages étroits derrière une île ou un sommet isolé : 12 pas de 107 blocs × 3, soit
//! 36 appels à la géographie macro (coût borné).
//!
//! - la part du chemin amont qui est de la mer donne l'**apport maritime** (+0,55 au plus) ;
//! - la distance au rivage donne la **continentalité** (jusqu'à -0,55 loin des côtes) ;
//! - un relief amont plus haut que le point d'au moins 20 blocs retient la pluie (pleinement à 90) : c'est l'**ombre
//!   pluviométrique** (-0,9 au plus). Elle pèse moins quand l'obstacle est loin (de 1 à 0,4 sur le chemin) ;
//! - un relief qui monte face au vent (le point est plus haut que la terre 224 blocs en amont)
//!   reçoit un surcroît de pluie (+0,45) : les versants au vent.
//!
//! Ces termes s'ajoutent à un champ régional lent (5 000 blocs) d'amplitude 0,6. Tout reste entre
//! -1 et 1, comme les climats de `biomes.ron`.
//!
//! # Ce que le climat ne fait pas
//!
//! Il ne lit pas l'eau réelle (cours, lacs) : l'humidité locale près de l'eau, les marais et la
//! végétation riveraine sont ajoutés par la construction des colonnes (`v2_context.rs`), qui
//! connaît les cours. Aucune rétroaction : la pluie n'agrandit pas un bassin, ne remplit pas un
//! lac. Un effet qui changerait l'hydrologie exigerait une nouvelle étape bornée et testée.

use super::SEA_LEVEL;
use super::geography::Geography;
use super::macro_noise::{fbm, smooth, unit};

const SALT_TEMPERATURE: u64 = 0x51;
const SALT_HUMIDITY: u64 = 0x52;
const SALT_VARIETY: u64 = 0x53;
const SALT_WIND: u64 = 0x54;
const SALT_WETLAND: u64 = 0x55;

/// Longueurs d'onde des champs régionaux, en blocs.
const TEMPERATURE_WAVELENGTH: f64 = 8000.0;
const HUMIDITY_WAVELENGTH: f64 = 5000.0;
const VARIETY_WAVELENGTH: f64 = 1100.0;
const WIND_WAVELENGTH: f64 = 12_000.0;
const WETLAND_WAVELENGTH: f64 = 650.0;

/// Pas et nombre de lectures amont du vent : le chemin d'air regardé fait `STEP * STEPS` blocs.
pub const WIND_STEP: f64 = 107.0;
pub const WIND_STEPS: usize = 12;
/// Écart latéral (blocs) des trois chemins d'air lus de front : une île ou un sommet isolé ne
/// trace plus un sillage étroit dans l'humidité.
pub const WIND_SPREAD: f64 = 150.0;

/// Refroidissement par bloc au-dessus de [`LAPSE_START`] (au-dessus du niveau de la mer).
const LAPSE_RATE: f64 = 0.0065;
const LAPSE_START: f64 = 25.0;

/// Le climat d'un point (voir la documentation du module).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClimateSample {
    /// La température du climat, de -1 à 1, altitude comprise : elle choisit le biome.
    pub temperature: f64,
    /// La même au niveau de la mer, sans le refroidissement de l'altitude.
    pub sea_temperature: f64,
    /// L'humidité, de -1 à 1, avant les effets locaux de l'eau (voir `v2_context.rs`).
    pub humidity: f64,
    /// La variété, de -1 à 1 : elle départage les variantes d'un même climat.
    pub variety: f64,
    /// De 0 à 1 : la part de l'humidité retenue par un relief au vent (ombre pluviométrique).
    pub rain_shadow: f64,
    /// De 0 à 1 : la part du chemin d'air amont qui est de la mer.
    pub maritime: f64,
    /// La direction du vent (le sens où il souffle), vecteur unitaire.
    pub wind: (f64, f64),
}

/// Le climat d'une graine.
#[derive(Debug, Clone, Copy)]
pub struct WorldClimate {
    seed: u32,
}

impl WorldClimate {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    /// La direction du vent en (x, z) : l'angle de base de la graine, qui dérive lentement.
    pub fn wind(&self, x: f64, z: f64) -> (f64, f64) {
        let base = unit(self.seed, SALT_WIND, 0, 0) * std::f64::consts::TAU;
        let drift = 0.8
            * fbm(
                self.seed,
                SALT_WIND,
                x / WIND_WAVELENGTH,
                z / WIND_WAVELENGTH,
                2,
                0.5,
            );
        let angle = base + drift;
        (angle.cos(), angle.sin())
    }

    /// Le bruit des zones basses humides (marais), de -1 à 1 environ : des taches de 650 blocs.
    pub fn wetland_noise(&self, x: f64, z: f64) -> f64 {
        fbm(
            self.seed,
            SALT_WETLAND,
            x / WETLAND_WAVELENGTH,
            z / WETLAND_WAVELENGTH,
            2,
            0.5,
        )
    }

    /// Le climat du point (x, z), lu sur la géographie macro.
    pub fn sample(&self, geography: &Geography, x: f64, z: f64) -> ClimateSample {
        let sea = SEA_LEVEL as f64;
        let coast = geography.coast_distance_at(x, z);
        let elevation = geography.macro_elevation(x, z) as f64;
        let wind = self.wind(x, z);

        // La température : région, modération maritime, altitude.
        let regional = (2.0
            * fbm(
                self.seed,
                SALT_TEMPERATURE,
                x / TEMPERATURE_WAVELENGTH,
                z / TEMPERATURE_WAVELENGTH,
                2,
                0.5,
            ))
        .clamp(-1.0, 1.0);
        let inland = smooth(0.0, 800.0, coast);
        let sea_temperature = regional * (1.0 - 0.3 * (1.0 - inland));
        let lapse = LAPSE_RATE * (elevation - sea - LAPSE_START).max(0.0);
        let temperature = (sea_temperature - lapse).clamp(-1.0, 1.0);

        // Le chemin d'air amont : la mer qu'il a traversée, l'obstacle le plus gênant. Trois chemins
        // de front (centre, gauche, droite) : leurs apports se moyennent, leurs ombres aussi.
        let mut sea_weight = 0.0;
        let mut total_weight = 0.0;
        let mut shadow_sum = 0.0;
        let reference = elevation.max(sea);
        let side = (-wind.1, wind.0);
        for lane in [-1.0, 0.0, 1.0] {
            let (ox, oz) = (side.0 * lane * WIND_SPREAD, side.1 * lane * WIND_SPREAD);
            let mut shadow: f64 = 0.0;
            for i in 1..=WIND_STEPS {
                let distance = i as f64 * WIND_STEP;
                // Les points proches pèsent plus : leur air est celui qui arrive.
                let weight =
                    1.0 - 0.6 * (distance - WIND_STEP) / (WIND_STEP * (WIND_STEPS as f64 - 1.0));
                let upwind = geography
                    .macro_elevation(x - wind.0 * distance + ox, z - wind.1 * distance + oz);
                total_weight += weight;
                // Mer ou terre sans saut : l'indicateur passe de 1 à 0 entre SEA - 8 et SEA + 8, pour
                // qu'un chemin qui effleure une côte ne fasse pas varier l'humidité par à-coups.
                let ocean = smooth(sea + 8.0, sea - 8.0, upwind as f64);
                sea_weight += weight * ocean;
                let obstacle = upwind as f64 - reference;
                shadow = shadow.max(smooth(20.0, 90.0, obstacle) * weight * (1.0 - ocean));
            }
            shadow_sum += shadow;
        }
        let maritime = sea_weight / total_weight;
        let rain_shadow = (shadow_sum / 3.0).clamp(0.0, 1.0);

        // Le versant au vent : le terrain monte face au vent.
        let near = geography.macro_elevation(x - wind.0 * 224.0, z - wind.1 * 224.0) as f64;
        let uplift = smooth(15.0, 90.0, elevation - near.max(sea))
            * (0.4 + 0.6 * maritime)
            * smooth(sea + 6.0, sea + 20.0, elevation);

        let continental = smooth(300.0, 2400.0, coast);
        let regional_humidity = 0.6
            * (1.6
                * fbm(
                    self.seed,
                    SALT_HUMIDITY,
                    x / HUMIDITY_WAVELENGTH,
                    z / HUMIDITY_WAVELENGTH,
                    2,
                    0.5,
                ))
            .clamp(-1.0, 1.0);
        let humidity = (regional_humidity + 0.55 * maritime - 0.55 * continental + 0.45 * uplift
            - 0.9 * rain_shadow)
            .clamp(-1.0, 1.0);

        let variety = (1.1
            * fbm(
                self.seed,
                SALT_VARIETY,
                x / VARIETY_WAVELENGTH,
                z / VARIETY_WAVELENGTH,
                2,
                0.5,
            ))
        .clamp(-1.0, 1.0);

        ClimateSample {
            temperature,
            sea_temperature,
            humidity,
            variety,
            rain_shadow,
            maritime,
            wind,
        }
    }
}
