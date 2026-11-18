//! Les cartes de la géographie macro du générateur v2 (prototype des lots 2 et 3) : terre et mer,
//! distance au rivage, altitude, axes de chaînes, familles de relief, bassins, réseau de drainage
//! et lacs. Même principe que [`crate::worldmap`] : on échantillonne la zone une fois
//! ([`MacroMapSamples::compute`], en parallèle), puis on tire autant de cartes qu'on veut.
//!
//! La commande est `cargo xtask map --generator v2` (voir `crates/engine_voxel/examples/worldmap.rs`).

use rayon::prelude::*;

use crate::generator::{MacroSample, MacroWorld, ReliefFamily, SEA_LEVEL};
use crate::worldmap::{MapArea, Raster, altitude_color, gray, ramp};

/// Ce que montre une carte macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroMapKind {
    /// Terre (par bandes de distance au rivage) et mer (par profondeur).
    LandSea,
    /// Distance signée au rivage, en dégradé, avec une courbe tous les 500 blocs.
    CoastDistance,
    /// Altitude de la géographie seule (avant l'hydrologie), ombrée.
    MacroAltitude,
    /// Altitude après vallées et lacs, ombrée.
    Altitude,
    /// Hauteur des chaînes en gris, axes en rouge (principal) : crêtes, cols et contreforts.
    Ridges,
    /// Familles de relief, une couleur chacune.
    Families,
    /// Bassins versants : une couleur par identité de bassin.
    Basins,
    /// Le réseau de drainage sur le relief : largeur selon l'accumulation.
    Drainage,
    /// Les lacs : terminaux (fermés) en orange, de passage en cyan.
    Lakes,
}

impl MacroMapKind {
    pub const ALL: [MacroMapKind; 9] = [
        MacroMapKind::LandSea,
        MacroMapKind::CoastDistance,
        MacroMapKind::MacroAltitude,
        MacroMapKind::Altitude,
        MacroMapKind::Ridges,
        MacroMapKind::Families,
        MacroMapKind::Basins,
        MacroMapKind::Drainage,
        MacroMapKind::Lakes,
    ];

    /// Le nom court, utilisé en ligne de commande et dans le nom des fichiers.
    pub fn name(self) -> &'static str {
        match self {
            Self::LandSea => "terre-mer",
            Self::CoastDistance => "rivage",
            Self::MacroAltitude => "altitude-macro",
            Self::Altitude => "altitude",
            Self::Ridges => "chaines",
            Self::Families => "familles",
            Self::Basins => "bassins",
            Self::Drainage => "drainage",
            Self::Lakes => "lacs",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Les échantillons macro d'une zone, avec une bordure d'un pixel pour les ombrages.
pub struct MacroMapSamples<'a> {
    world: &'a MacroWorld,
    area: MapArea,
    samples: Vec<MacroSample>,
}

impl<'a> MacroMapSamples<'a> {
    /// Échantillonne la zone (un [`MacroWorld::sample`] par pixel, bordure comprise), en parallèle.
    pub fn compute(world: &'a MacroWorld, area: MapArea) -> Self {
        let step = area.step.max(1) as i32;
        let (w, h) = (area.width as usize + 2, area.height as usize + 2);
        let samples = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (px, pz) = ((i % w) as i32 - 1, (i / w) as i32 - 1);
                world.sample((area.x + px * step) as f64, (area.z + pz * step) as f64)
            })
            .collect();
        Self {
            world,
            area,
            samples,
        }
    }

    pub fn world(&self) -> &MacroWorld {
        self.world
    }

    /// L'échantillon du pixel (px, pz) ; -1 et `width`/`height` atteignent la bordure.
    pub fn at(&self, px: i32, pz: i32) -> &MacroSample {
        let w = self.area.width as usize + 2;
        &self.samples[(pz + 1) as usize * w + (px + 1) as usize]
    }

    pub fn render(&self, kind: MacroMapKind) -> Raster {
        let (width, height) = (self.area.width, self.area.height);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pz in 0..height as i32 {
            for px in 0..width as i32 {
                rgb.extend_from_slice(&self.color(kind, px, pz));
            }
        }
        Raster { width, height, rgb }
    }

    /// Les différences de `value` avec les voisins, en blocs par bloc.
    fn shade(&self, px: i32, pz: i32, value: impl Fn(&MacroSample) -> f32) -> f32 {
        let step = self.area.step.max(1) as f32;
        let dx = (value(self.at(px + 1, pz)) - value(self.at(px - 1, pz))) / (2.0 * step);
        let dz = (value(self.at(px, pz + 1)) - value(self.at(px, pz - 1))) / (2.0 * step);
        ((dx + dz) * -0.9).clamp(-1.0, 1.0)
    }

    fn is_sea(s: &MacroSample) -> bool {
        s.geo.macro_elevation < SEA_LEVEL as f32
    }

    /// Le pixel est dans le trait d'une rivière : le lit, ou au moins un pixel de large.
    fn river_pixel(&self, s: &MacroSample) -> bool {
        s.river
            .is_some_and(|r| r.distance <= (r.width * 0.5).max(self.area.step as f32 * 0.75))
    }

    fn color(&self, kind: MacroMapKind, px: i32, pz: i32) -> [u8; 3] {
        let s = self.at(px, pz);
        let sea = Self::is_sea(s);
        match kind {
            MacroMapKind::LandSea => {
                if sea {
                    let depth = ((-s.geo.coast_distance) / 1500.0).clamp(0.0, 1.0);
                    ramp(depth, &[[110, 175, 230], [20, 55, 140]])
                } else {
                    let band = (s.geo.coast_distance / 400.0).floor().min(6.0) as usize;
                    [
                        [235, 220, 160],
                        [190, 210, 120],
                        [140, 190, 100],
                        [100, 170, 90],
                        [80, 150, 85],
                        [65, 130, 80],
                        [50, 110, 75],
                    ][band]
                }
            }
            MacroMapKind::CoastDistance => {
                let d = s.geo.coast_distance;
                let base = if d < 0.0 {
                    ramp((-d / 2600.0).min(1.0), &[[150, 200, 240], [15, 40, 120]])
                } else {
                    ramp(
                        (d / 2600.0).min(1.0),
                        &[[245, 235, 170], [120, 190, 90], [40, 100, 60]],
                    )
                };
                let contour = (d / 500.0 - (d / 500.0).round()).abs() * 500.0;
                if contour < self.area.step as f32 * 0.6 {
                    [0, 0, 0]
                } else {
                    base
                }
            }
            MacroMapKind::MacroAltitude => altitude_color(
                s.geo.macro_elevation as i32,
                self.shade(px, pz, |s| s.geo.macro_elevation),
            ),
            MacroMapKind::Altitude => {
                let shade = self.shade(px, pz, |s| s.elevation);
                if s.in_water && !sea && self.water_visible(s) {
                    water_color(s)
                } else {
                    altitude_color(s.elevation as i32, shade)
                }
            }
            MacroMapKind::Ridges => {
                if sea && s.geo.chain_height < 1.0 {
                    return [25, 45, 100];
                }
                if s.geo.chain_strength > 0.12 && s.geo.axis_distance < self.area.step as f32 * 0.9
                {
                    return [230, 40, 30];
                }
                let t = (s.geo.chain_height / 70.0).clamp(0.0, 1.0);
                let g = gray(1.0 - t);
                if sea {
                    [g[0] / 2, g[1] / 2, g[2] / 2 + 60]
                } else {
                    g
                }
            }
            MacroMapKind::Families => match s.geo.family {
                ReliefFamily::DeepSea => [15, 30, 110],
                ReliefFamily::Shelf => [70, 140, 215],
                ReliefFamily::Coast => [236, 222, 160],
                ReliefFamily::Plain => [140, 200, 100],
                ReliefFamily::Plateau => [200, 190, 100],
                ReliefFamily::Foothills => [150, 115, 70],
                ReliefFamily::Massif => [235, 235, 240],
            },
            MacroMapKind::Basins => {
                if sea {
                    return [20, 40, 100];
                }
                let c = basin_color(s.basin);
                let shade = self.shade(px, pz, |s| s.geo.macro_elevation);
                let lit = |v: u8| (v as f32 * (1.0 + 0.18 * shade)).clamp(0.0, 255.0) as u8;
                if self.river_pixel(s) {
                    [255, 255, 255]
                } else {
                    [lit(c[0]), lit(c[1]), lit(c[2])]
                }
            }
            MacroMapKind::Drainage => {
                if sea {
                    return [25, 55, 125];
                }
                let shade = self.shade(px, pz, |s| s.elevation);
                let base = altitude_gray(s.elevation, shade);
                if let Some(l) = s.lake.filter(|l| l.depth_fraction > 0.0) {
                    return if l.terminal {
                        [60, 30, 140]
                    } else {
                        [20, 90, 200]
                    };
                }
                if self.river_pixel(s) {
                    let r = s.river.expect("river_pixel suppose une rivière");
                    let t = (r.accumulation as f32 / crate::generator::ACCUMULATION_CAP as f32)
                        .sqrt()
                        .clamp(0.0, 1.0);
                    return ramp(t, &[[120, 200, 255], [20, 90, 230], [10, 30, 170]]);
                }
                base
            }
            MacroMapKind::Lakes => {
                let shade = self.shade(px, pz, |s| s.elevation);
                if let Some(l) = s.lake.filter(|l| l.depth_fraction > 0.0) {
                    return if l.terminal {
                        [240, 130, 20]
                    } else {
                        [30, 220, 230]
                    };
                }
                if sea {
                    return [25, 55, 125];
                }
                altitude_gray(s.elevation, shade)
            }
        }
    }

    fn water_visible(&self, s: &MacroSample) -> bool {
        s.lake.is_some_and(|l| l.depth_fraction > 0.0) || self.river_pixel(s)
    }
}

fn water_color(s: &MacroSample) -> [u8; 3] {
    if s.lake.is_some() {
        [30, 110, 210]
    } else {
        [70, 160, 235]
    }
}

/// Un relief en gris ombré, pour que le réseau d'eau ressorte.
fn altitude_gray(elevation: f32, shade: f32) -> [u8; 3] {
    let t = ((elevation - SEA_LEVEL as f32) / 128.0).clamp(0.0, 1.0);
    let v = (0.78 - 0.34 * t) * (1.0 + 0.30 * shade);
    let g = (v.clamp(0.0, 1.0) * 255.0) as u8;
    [g, (g as f32 * 0.97) as u8, (g as f32 * 0.9) as u8]
}

/// Une couleur stable par identité de bassin (teinte par l'angle d'or du hachage).
fn basin_color(id: u64) -> [u8; 3] {
    let hue = ((id >> 11) as f64 / (1u64 << 53) as f64) as f32;
    let (s, v) = (0.45 + 0.25 * (((id >> 3) & 7) as f32 / 7.0), 0.78);
    let h = hue * 6.0;
    let (i, f) = (h.floor() as i32 % 6, h.fract());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> MapArea {
        MapArea {
            x: -3000,
            z: -2000,
            width: 40,
            height: 30,
            step: 64,
        }
    }

    #[test]
    fn every_macro_map_has_the_requested_size_and_is_deterministic() {
        let a = MacroWorld::new(42);
        let b = MacroWorld::new(42);
        let (sa, sb) = (
            MacroMapSamples::compute(&a, area()),
            MacroMapSamples::compute(&b, area()),
        );
        for kind in MacroMapKind::ALL {
            let raster = sa.render(kind);
            assert_eq!((raster.width, raster.height), (40, 30));
            assert_eq!(raster.rgb.len(), 40 * 30 * 3, "{}", kind.name());
            assert_eq!(raster, sb.render(kind), "{}", kind.name());
        }
    }

    #[test]
    fn names_round_trip() {
        for kind in MacroMapKind::ALL {
            assert_eq!(MacroMapKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(MacroMapKind::from_name("inconnue"), None);
    }
}
