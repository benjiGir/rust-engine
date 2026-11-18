//! Le contexte macro du générateur v2 (prototype) : la géographie et l'hydrologie d'une graine
//! réunies en une requête de point, avec les vallées et les lacs modelés dans l'altitude.
//!
//! [`MacroWorld`] est partageable entre threads (`&self`, caches internes bornés). Une valeur ne
//! dépend que de `(graine, x, z)` : le contexte n'est pas une région, il n'a pas de frontière.
//! Rien n'est encore branché sur les blocs (lot 4) : seuls les outils de cartes et les tests
//! l'interrogent.
//!
//! # Façonner le terrain autour de l'eau
//!
//! L'altitude macro est modifiée au voisinage d'une rivière (distance `d` à son axe, demi-largeur
//! `w`, niveau d'eau `L`) :
//!
//! - dans le lit (`d <= w`), le sol descend sous le niveau d'eau ;
//! - au-delà, une cible `clamp(altitude, L + 1, L + 1 + pente × (d - w))` : le terrain trop haut est
//!   abaissé selon une pente de berge (douce en plaine alluviale, raide en vallée encaissée), le
//!   terrain trop bas est relevé jusqu'à la berge (remblai) ;
//! - cette cible est fondue dans l'altitude d'origine sur [`VALLEY_REACH`] blocs.
//!
//! Un lac abaisse le sol sous son niveau, en cuvette, et le relève à son rivage s'il est trop bas.

use super::geography::{GeoSample, Geography};
use super::hydrology::{Hydrology, LakePoint, RiverPoint};
use super::macro_noise::smooth;

/// Largeur (blocs) sur laquelle la vallée se fond dans le terrain : inférieure à une cellule fine,
/// ce qui garantit que les arêtes lues (5 × 5 cellules) suffisent.
pub const VALLEY_REACH: f32 = 56.0;
/// Relèvement maximal du terrain (blocs) sous un cours dont le niveau d'eau le dépasse.
pub const MAX_FILL: f32 = 2.0;

/// Tout ce que le contexte macro sait d'un point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacroSample {
    pub geo: GeoSample,
    /// L'identité du bassin versant (le puits où l'eau du point finit).
    pub basin: u64,
    /// La rivière la plus proche (même hors de son lit : voir `distance` et `width`).
    pub river: Option<RiverPoint>,
    pub lake: Option<LakePoint>,
    /// Vrai si le point est dans le lit d'une rivière ou sous un lac.
    pub in_water: bool,
    /// L'altitude avec vallées et lacs, en blocs.
    pub elevation: f32,
}

/// Le contexte macro d'une graine.
pub struct MacroWorld {
    hydrology: Hydrology,
}

impl MacroWorld {
    pub fn new(seed: u32) -> Self {
        Self {
            hydrology: Hydrology::new(seed),
        }
    }

    /// Avec une borne de cache donnée (voir [`Hydrology::with_cache_limit`]).
    pub fn with_cache_limit(seed: u32, limit: usize) -> Self {
        Self {
            hydrology: Hydrology::with_cache_limit(seed, limit),
        }
    }

    pub fn seed(&self) -> u32 {
        self.hydrology.seed()
    }

    pub fn geography(&self) -> &Geography {
        self.hydrology.geography()
    }

    pub fn hydrology(&self) -> &Hydrology {
        &self.hydrology
    }

    pub fn clear_caches(&self) {
        self.hydrology.clear_caches();
    }

    /// Le point (x, z) : géographie, drainage, lacs et altitude modelée.
    pub fn sample(&self, x: f64, z: f64) -> MacroSample {
        let geo = self.geography().sample(x, z);
        let lake = self.hydrology.lake_at(x, z);
        // Aucune rivière au large : l'eau y est la mer.
        let river = (geo.macro_elevation >= super::SEA_LEVEL as f32 - 1.0)
            .then(|| self.hydrology.river_near(x, z))
            .flatten();
        let mut elevation = geo.elevation;
        let mut in_water = false;
        if let Some(r) = river {
            let half = r.width * 0.5;
            let over = (r.distance - half).max(0.0);
            // Vallée encaissée si le terrain domine l'eau, plaine alluviale s'il est proche.
            let relief = (geo.elevation - r.level).max(0.0);
            let slope = 0.12 + 0.68 * smooth(8.0, 60.0, relief as f64) as f32;
            // Le remblai est plafonné : le niveau d'eau reste exact, mais le terrain n'est relevé que
            // de `MAX_FILL` blocs (le relèvement complet des dépressions est l'affaire du lot 4).
            let floor = (r.level + 1.0).min(geo.elevation + MAX_FILL);
            let target = geo.elevation.clamp(floor, floor + slope * over);
            let k = (1.0 - smooth(0.0, VALLEY_REACH as f64, over as f64) as f32) * r.taper;
            elevation += (target - elevation) * k;
            if r.distance <= half {
                in_water = true;
                let depth = 1.0 + 0.1 * r.width;
                elevation = elevation.min(r.level - depth * (1.0 - r.distance / half.max(0.5)));
            }
        }
        if let Some(l) = lake {
            // Une cuvette qui plonge sous le niveau, fondue dans le terrain au rivage.
            let bottom = l.level - 1.0 - 5.0 * l.depth_fraction.sqrt();
            let rim = smooth(0.0, 0.25, l.depth_fraction as f64) as f32;
            elevation += (bottom - elevation) * rim;
            in_water = true;
        }
        MacroSample {
            geo,
            basin: self.hydrology.basin_id_at(x, z),
            river,
            lake,
            in_water,
            elevation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_is_the_same_in_any_order_and_cache_state() {
        let world = MacroWorld::new(42);
        let points: Vec<(f64, f64)> = (0..60)
            .map(|i| (-3000.0 + i as f64 * 101.5, 2000.0 - i as f64 * 77.25))
            .collect();
        let forward: Vec<_> = points.iter().map(|&(x, z)| world.sample(x, z)).collect();
        world.clear_caches();
        let backward: Vec<_> = points
            .iter()
            .rev()
            .map(|&(x, z)| world.sample(x, z))
            .collect();
        for (a, b) in forward.iter().zip(backward.iter().rev()) {
            assert_eq!(a, b);
        }
    }
}
