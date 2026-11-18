//! Les cartes de climat, de biomes et de végétation de la génération v2 (lot 5).
//!
//! Les cartes du climat régional (`ombre-pluie`, `maritime`, `vent`) lisent seulement la
//! géographie macro : elles sont peu coûteuses, même sur plusieurs milliers de blocs. Les cartes
//! locales (`biomes-v2`, `temperature-v2`, `humidite-v2`, `canopee`) lisent le contexte de colonne
//! du générateur v2 (une colonne de chunks de 32 × 32 blocs par appel, mis en cache) : elles
//! disent ce que le jeu fera réellement, eau, marais et pente compris.

use rayon::prelude::*;

use crate::generator::{MacroWorld, SEA_LEVEL, SurfaceInfo, TerrainGenerator, WorldClimate};
use crate::worldmap::{MapArea, Raster, altitude_color, biome_color, ramp};

/// Ce que montre une carte de climat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClimateMapKind {
    /// La part de l'humidité retenue par un relief au vent : de blanc (aucune) à rouge (ombre
    /// forte), sur un fond d'altitude estompé. Les zones d'ombre se lisent sous les chaînes.
    RainShadow,
    /// La part du chemin d'air amont qui est de la mer, de brun (aucune) à bleu (toute).
    Maritime,
    /// L'humidité régionale du climat (sans l'eau locale), de brun à bleu.
    RegionalHumidity,
    /// Les biomes réellement choisis, une couleur par biome.
    Biomes,
    /// La température du climat (altitude comprise), de bleu à rouge.
    Temperature,
    /// L'humidité locale (eau proche et zones humides comprises), de brun à bleu.
    Humidity,
    /// Le coefficient de végétation : de noir (aucun arbre) à vert vif (le plus dense).
    Canopy,
}

impl ClimateMapKind {
    pub const ALL: [ClimateMapKind; 7] = [
        ClimateMapKind::RainShadow,
        ClimateMapKind::Maritime,
        ClimateMapKind::RegionalHumidity,
        ClimateMapKind::Biomes,
        ClimateMapKind::Temperature,
        ClimateMapKind::Humidity,
        ClimateMapKind::Canopy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::RainShadow => "ombre-pluie",
            Self::Maritime => "maritime",
            Self::RegionalHumidity => "humidite-regionale",
            Self::Biomes => "biomes-v2",
            Self::Temperature => "temperature-v2",
            Self::Humidity => "humidite-v2",
            Self::Canopy => "canopee",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.to_lowercase();
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Cette carte lit les colonnes du générateur (plus cher) et non la géographie seule.
    pub fn needs_columns(self) -> bool {
        matches!(
            self,
            Self::Biomes | Self::Temperature | Self::Humidity | Self::Canopy
        )
    }
}

/// Le climat régional d'un pixel : ce que la géographie macro en dit.
#[derive(Debug, Clone, Copy)]
struct Regional {
    macro_elevation: f32,
    shadow: f32,
    maritime: f32,
    humidity: f32,
}

/// Les échantillons d'une zone.
pub struct ClimateMapSamples<'a> {
    generator: Option<&'a TerrainGenerator>,
    area: MapArea,
    regional: Vec<Regional>,
    /// Vide quand aucune carte locale n'est demandée.
    columns: Vec<Option<SurfaceInfo>>,
}

impl<'a> ClimateMapSamples<'a> {
    /// Lit le climat régional de chaque pixel et, si `generator` est donné, ses colonnes.
    pub fn compute(
        world: &MacroWorld,
        generator: Option<&'a TerrainGenerator>,
        area: MapArea,
    ) -> Self {
        let step = area.step.max(1) as i32;
        let (w, h) = (area.width as i32, area.height as i32);
        let climate = WorldClimate::new(world.seed());
        let at = |n: i32| (area.x + (n % w) * step, area.z + (n / w) * step);
        let regional: Vec<Regional> = (0..w * h)
            .into_par_iter()
            .map(|n| {
                let (x, z) = at(n);
                let c = climate.sample(world.geography(), x as f64, z as f64);
                Regional {
                    macro_elevation: world.geography().macro_elevation(x as f64, z as f64),
                    shadow: c.rain_shadow as f32,
                    maritime: c.maritime as f32,
                    humidity: c.humidity as f32,
                }
            })
            .collect();
        let columns = match generator {
            Some(generator) => (0..w * h)
                .into_par_iter()
                .map(|n| {
                    let (x, z) = at(n);
                    generator.surface_info(x, z)
                })
                .collect(),
            None => Vec::new(),
        };
        Self {
            generator,
            area,
            regional,
            columns,
        }
    }

    /// Les biomes présents dans la zone avec leur couleur (carte `biomes-v2`).
    pub fn biome_legend(&self) -> Vec<(String, [u8; 3])> {
        let Some(generator) = self.generator else {
            return Vec::new();
        };
        let mut seen = vec![false; 256];
        for info in self.columns.iter().flatten() {
            seen[info.biome.0 as usize] = true;
        }
        generator
            .biomes()
            .iter()
            .filter(|(id, _)| seen[id.0 as usize])
            .map(|(id, def)| (def.key.clone(), biome_color(def.kind, id.0)))
            .collect()
    }

    pub fn render(&self, kind: ClimateMapKind) -> Raster {
        let (width, height) = (self.area.width, self.area.height);
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for n in 0..(width * height) as usize {
            rgb.extend_from_slice(&self.color(kind, n));
        }
        Raster { width, height, rgb }
    }

    fn color(&self, kind: ClimateMapKind, n: usize) -> [u8; 3] {
        let r = &self.regional[n];
        // Un fond d'altitude estompé pour les cartes régionales : la mer en bleu sombre.
        let ground = |tint: [u8; 3], amount: f32| {
            let base = altitude_color(r.macro_elevation as i32, 1.0);
            let a = amount.clamp(0.0, 1.0);
            let mix = |b: u8, t: u8| (b as f32 * (1.0 - a) + t as f32 * a) as u8;
            [
                mix(base[0] / 2 + 60, tint[0]),
                mix(base[1] / 2 + 60, tint[1]),
                mix(base[2] / 2 + 60, tint[2]),
            ]
        };
        match kind {
            ClimateMapKind::RainShadow => ground([200, 30, 20], r.shadow * 0.9),
            ClimateMapKind::Maritime => {
                if r.macro_elevation < SEA_LEVEL as f32 {
                    [20, 40, 120]
                } else {
                    ramp(
                        r.maritime,
                        &[[150, 110, 60], [225, 215, 150], [70, 150, 190]],
                    )
                }
            }
            ClimateMapKind::RegionalHumidity => ramp(
                (r.humidity + 1.0) / 2.0,
                &[
                    [150, 110, 60],
                    [225, 215, 150],
                    [70, 150, 90],
                    [30, 80, 190],
                ],
            ),
            local => {
                let Some(info) = self.columns.get(n).copied().flatten() else {
                    return [0, 0, 0];
                };
                match local {
                    ClimateMapKind::Biomes => {
                        let generator = self.generator.expect("cartes locales : un générateur");
                        let def = generator.biome(info.biome);
                        biome_color(def.kind, info.biome.0)
                    }
                    ClimateMapKind::Temperature => ramp(
                        (info.climate + 1.0) / 2.0,
                        &[
                            [40, 70, 200],
                            [150, 210, 230],
                            [240, 230, 130],
                            [210, 50, 30],
                        ],
                    ),
                    ClimateMapKind::Humidity => ramp(
                        (info.humidity + 1.0) / 2.0,
                        &[
                            [150, 110, 60],
                            [225, 215, 150],
                            [70, 150, 90],
                            [30, 80, 190],
                        ],
                    ),
                    ClimateMapKind::Canopy => {
                        if info.water.is_some() {
                            [20, 40, 120]
                        } else {
                            ramp(
                                info.canopy / 2.0,
                                &[[10, 10, 10], [60, 90, 40], [90, 200, 60]],
                            )
                        }
                    }
                    _ => unreachable!("cartes régionales traitées plus haut"),
                }
            }
        }
    }
}
