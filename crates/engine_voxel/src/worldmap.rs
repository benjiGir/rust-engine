//! Des cartes du monde sans fenêtre : altitude, pente, humidité, température, continentalité et
//! biomes d'une zone, pour regarder la géographie d'une graine sans lancer le jeu.
//!
//! On échantillonne d'abord la zone une fois ([`MapSamples::compute`]), puis on tire autant de
//! cartes qu'on veut ([`MapSamples::render`]). Chaque pixel est un carré de `step` blocs de côté,
//! lu en son coin (x, z) : une carte de 512 pixels à `step` 4 couvre 2 048 blocs.
//!
//! Le moteur ne sait pas écrire d'image : une [`Raster`] est un tampon RVB que l'appelant encode
//! en PNG ou en PPM ([`Raster::to_ppm`]). La commande est `cargo xtask map` (voir
//! `crates/engine_voxel/examples/worldmap.rs`).

use crate::biome::BiomeKind;
use crate::generator::{Climate, Column, SEA_LEVEL, TerrainGenerator, WORLD_HEIGHT_CHUNKS};

/// Ce que montre une carte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapKind {
    /// Hauteur de la colonne : bleu sous le niveau de la mer, vert, brun puis blanc en montagne,
    /// avec un léger ombrage.
    Altitude,
    /// Inclinaison du relief, de 0 (blanc, plat) à 60 degrés et plus (noir).
    Slope,
    /// Humidité du climat : brun (aride) vers bleu (détrempé).
    Humidity,
    /// Température du climat : bleu (glacial) vers rouge (brûlant).
    Temperature,
    /// La continentalité : l'organisation des océans et des continents.
    Continentalness,
    /// Un biome par couleur ; la légende est donnée par [`MapSamples::biome_legend`].
    Biomes,
}

impl MapKind {
    pub const ALL: [MapKind; 6] = [
        MapKind::Altitude,
        MapKind::Slope,
        MapKind::Humidity,
        MapKind::Temperature,
        MapKind::Continentalness,
        MapKind::Biomes,
    ];

    /// Le nom court, utilisé en ligne de commande et dans le nom des fichiers.
    pub fn name(self) -> &'static str {
        match self {
            MapKind::Altitude => "altitude",
            MapKind::Slope => "pente",
            MapKind::Humidity => "humidite",
            MapKind::Temperature => "temperature",
            MapKind::Continentalness => "continentalite",
            MapKind::Biomes => "biomes",
        }
    }

    /// Retrouve une carte par son nom (accents et variantes courantes tolérés).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|k| {
            k.name() == name
                || matches!(
                    (k, name.as_str()),
                    (MapKind::Slope, "slope")
                        | (MapKind::Humidity, "humidité" | "humidity")
                        | (MapKind::Temperature, "température")
                        | (
                            MapKind::Continentalness,
                            "continentalité" | "continentalness"
                        )
                )
        })
    }
}

/// La zone d'une carte : coin (x, z) en blocs, taille en pixels, blocs par pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapArea {
    pub x: i32,
    pub z: i32,
    pub width: u32,
    pub height: u32,
    pub step: u32,
}

/// Une image RVB, huit bits par canal, ligne par ligne du haut (z croissant) vers le bas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Raster {
    /// L'image au format PPM binaire (P6), lisible par la plupart des visionneuses.
    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend_from_slice(&self.rgb);
        out
    }

    /// La couleur du pixel (px, pz).
    pub fn pixel(&self, px: u32, pz: u32) -> [u8; 3] {
        let i = ((pz * self.width + px) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// Les colonnes et climats d'une zone, avec une bordure d'un pixel pour calculer les pentes.
pub struct MapSamples<'a> {
    generator: &'a TerrainGenerator,
    area: MapArea,
    /// `(width + 2) × (height + 2)` échantillons, bordure comprise.
    samples: Vec<(Column, Climate)>,
}

impl<'a> MapSamples<'a> {
    /// Échantillonne la zone : un appel à [`TerrainGenerator::sample`] par pixel (bordure
    /// comprise), le même résultat que la génération des chunks.
    pub fn compute(generator: &'a TerrainGenerator, area: MapArea) -> Self {
        let step = area.step.max(1) as i32;
        let (w, h) = (area.width as usize + 2, area.height as usize + 2);
        let mut samples = Vec::with_capacity(w * h);
        for pz in 0..h as i32 {
            for px in 0..w as i32 {
                samples.push(generator.sample(area.x + (px - 1) * step, area.z + (pz - 1) * step));
            }
        }
        Self {
            generator,
            area,
            samples,
        }
    }

    /// L'échantillon du pixel (px, pz) ; -1 et `width` atteignent la bordure.
    fn at(&self, px: i32, pz: i32) -> &(Column, Climate) {
        let w = self.area.width as usize + 2;
        &self.samples[(pz + 1) as usize * w + (px + 1) as usize]
    }

    /// Dessine une carte.
    pub fn render(&self, kind: MapKind) -> Raster {
        let (width, height) = (self.area.width, self.area.height);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for pz in 0..height as i32 {
            for px in 0..width as i32 {
                rgb.extend_from_slice(&self.color(kind, px, pz));
            }
        }
        Raster { width, height, rgb }
    }

    /// Les biomes présents dans la zone, avec leur couleur sur la carte des biomes.
    pub fn biome_legend(&self) -> Vec<(String, [u8; 3])> {
        let mut seen = vec![false; 256];
        for (column, _) in &self.samples {
            seen[column.biome.0 as usize] = true;
        }
        self.generator
            .biomes()
            .iter()
            .filter(|(id, _)| seen[id.0 as usize])
            .map(|(id, def)| (def.key.clone(), biome_color(def.kind, id.0)))
            .collect()
    }

    fn color(&self, kind: MapKind, px: i32, pz: i32) -> [u8; 3] {
        let (column, climate) = self.at(px, pz);
        match kind {
            MapKind::Altitude => {
                let shade = self.shade(px, pz);
                altitude_color(column.height, shade)
            }
            MapKind::Slope => {
                let degrees = self.slope_degrees(px, pz);
                let t = 1.0 - (degrees / 60.0).clamp(0.0, 1.0);
                gray(t)
            }
            MapKind::Humidity => ramp(
                (climate.humidity + 1.0) / 2.0,
                &[
                    [150, 110, 60],
                    [225, 215, 150],
                    [70, 150, 90],
                    [30, 80, 190],
                ],
            ),
            MapKind::Temperature => ramp(
                (climate.temperature + 1.0) / 2.0,
                &[
                    [40, 70, 200],
                    [150, 210, 230],
                    [240, 230, 130],
                    [210, 50, 30],
                ],
            ),
            MapKind::Continentalness => ramp(
                (climate.continentalness + 1.0) / 2.2,
                &[
                    [10, 20, 90],
                    [40, 120, 200],
                    [230, 215, 160],
                    [90, 160, 80],
                    [240, 240, 240],
                ],
            ),
            MapKind::Biomes => biome_color(self.generator.biome(column.biome).kind, column.biome.0),
        }
    }

    /// Les différences de hauteur avec les voisins (est et sud, ouest et nord), en blocs par bloc.
    fn gradient(&self, px: i32, pz: i32) -> (f32, f32) {
        let step = self.area.step.max(1) as f32;
        let h = |x, z| self.at(x, z).0.height as f32;
        (
            (h(px + 1, pz) - h(px - 1, pz)) / (2.0 * step),
            (h(px, pz + 1) - h(px, pz - 1)) / (2.0 * step),
        )
    }

    fn slope_degrees(&self, px: i32, pz: i32) -> f32 {
        let (dx, dz) = self.gradient(px, pz);
        slope_degrees(dx, dz)
    }

    /// Un ombrage simple : la lumière vient du nord-ouest. Entre -1 et 1.
    fn shade(&self, px: i32, pz: i32) -> f32 {
        let (dx, dz) = self.gradient(px, pz);
        ((dx + dz) * -0.8).clamp(-1.0, 1.0)
    }
}

/// L'inclinaison, en degrés, d'un relief de gradient (dx, dz) blocs par bloc.
pub fn slope_degrees(dx: f32, dz: f32) -> f32 {
    dx.hypot(dz).atan().to_degrees()
}

pub(crate) fn gray(t: f32) -> [u8; 3] {
    let v = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
    [v, v, v]
}

/// Interpolation linéaire entre des couleurs régulièrement espacées, `t` de 0 à 1.
pub(crate) fn ramp(t: f32, colors: &[[u8; 3]]) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (t.floor() as usize).min(colors.len() - 2);
    let f = t - i as f32;
    std::array::from_fn(|c| {
        (colors[i][c] as f32 + (colors[i + 1][c] as f32 - colors[i][c] as f32) * f).round() as u8
    })
}

pub(crate) fn altitude_color(height: i32, shade: f32) -> [u8; 3] {
    let sea = SEA_LEVEL;
    let top = WORLD_HEIGHT_CHUNKS * 32 - 2;
    let base = if height < sea {
        // La mer : plus profonde, plus sombre.
        let depth = ((sea - height) as f32 / 40.0).clamp(0.0, 1.0);
        ramp(depth, &[[70, 150, 220], [20, 50, 140]])
    } else {
        let t = (height - sea) as f32 / (top - sea) as f32;
        ramp(
            t,
            &[
                [225, 215, 150],
                [90, 160, 70],
                [60, 120, 50],
                [140, 110, 70],
                [150, 150, 150],
                [250, 250, 250],
            ],
        )
    };
    let k = 1.0 + 0.25 * shade;
    std::array::from_fn(|c| (base[c] as f32 * k).clamp(0.0, 255.0) as u8)
}

/// Une couleur stable par biome : l'eau en bleus (océan profond, océan, rivière) et le rivage en
/// sable, puis, pour les autres, une teinte qui avance de l'angle d'or à chaque numéro.
pub(crate) fn biome_color(kind: BiomeKind, id: u8) -> [u8; 3] {
    match kind {
        BiomeKind::DeepOcean => return [15, 30, 120],
        BiomeKind::Ocean => return [40, 100, 200],
        BiomeKind::River => return [90, 190, 235],
        BiomeKind::Beach => return [235, 220, 160],
        _ => {}
    }
    let hue = (id as f32 * 0.618_034).fract();
    let (s, v) = if id.is_multiple_of(2) {
        (0.65, 0.95)
    } else {
        (0.85, 0.7)
    };
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

    fn area(x: i32, z: i32) -> MapArea {
        MapArea {
            x,
            z,
            width: 24,
            height: 16,
            step: 8,
        }
    }

    #[test]
    fn every_map_has_the_requested_size_and_is_deterministic() {
        let generator = TerrainGenerator::new(42);
        let a = MapSamples::compute(&generator, area(-100, -60));
        let other = TerrainGenerator::new(42);
        let b = MapSamples::compute(&other, area(-100, -60));
        for kind in MapKind::ALL {
            let raster = a.render(kind);
            assert_eq!((raster.width, raster.height), (24, 16));
            assert_eq!(raster.rgb.len(), 24 * 16 * 3, "{}", kind.name());
            assert_eq!(
                raster,
                b.render(kind),
                "{} : même graine, même carte",
                kind.name()
            );
        }
    }

    #[test]
    fn pixels_read_the_same_columns_as_the_generator_including_negative_coordinates() {
        let generator = TerrainGenerator::new(7);
        let samples = MapSamples::compute(&generator, area(-100, -60));
        for (px, pz) in [(0, 0), (5, 3), (23, 15)] {
            let (x, z) = (-100 + px * 8, -60 + pz * 8);
            assert_eq!(samples.at(px, pz).0, generator.column(x, z));
        }
    }

    #[test]
    fn the_altitude_map_shows_sea_as_blue_and_high_land_as_light() {
        let generator = TerrainGenerator::new(42);
        let ocean = generator
            .find_column(0, 0, 16, 20_000, |c| {
                generator.biome(c.biome).kind == BiomeKind::DeepOcean
            })
            .expect("un océan profond existe");
        let one = MapArea {
            x: ocean.0,
            z: ocean.1,
            width: 1,
            height: 1,
            step: 1,
        };
        let [r, _, b] = MapSamples::compute(&generator, one)
            .render(MapKind::Altitude)
            .pixel(0, 0);
        assert!(b > r, "l'eau est bleue : rouge {r}, bleu {b}");
        assert!(
            altitude_color(190, 0.0).iter().all(|&c| c >= 240),
            "les sommets sont blancs"
        );
    }

    #[test]
    fn slope_is_zero_when_flat_and_grows_with_the_gradient() {
        assert_eq!(slope_degrees(0.0, 0.0), 0.0);
        assert!((slope_degrees(1.0, 0.0) - 45.0).abs() < 1e-4);
        assert!(slope_degrees(2.0, 0.0) > slope_degrees(1.0, 0.0));
        assert!((slope_degrees(3.0, 4.0) - slope_degrees(5.0, 0.0)).abs() < 1e-4);
    }

    #[test]
    fn names_round_trip_and_the_legend_lists_only_present_biomes() {
        for kind in MapKind::ALL {
            assert_eq!(MapKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(MapKind::from_name("slope"), Some(MapKind::Slope));
        assert_eq!(MapKind::from_name("inconnue"), None);
        let generator = TerrainGenerator::new(42);
        let samples = MapSamples::compute(&generator, area(0, 0));
        let legend = samples.biome_legend();
        assert!(!legend.is_empty() && legend.len() <= generator.biomes().len());
    }

    #[test]
    fn ppm_has_a_header_and_three_bytes_per_pixel() {
        let raster = Raster {
            width: 2,
            height: 1,
            rgb: vec![1, 2, 3, 4, 5, 6],
        };
        let ppm = raster.to_ppm();
        assert!(ppm.starts_with(b"P6\n2 1\n255\n"));
        assert_eq!(&ppm[ppm.len() - 6..], &[1, 2, 3, 4, 5, 6]);
    }
}
