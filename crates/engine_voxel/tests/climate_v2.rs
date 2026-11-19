//! Le climat, les biomes et la végétation de la génération v2 (lot 5).
//!
//! Le climat vient de la géographie (voir `generator/climate.rs`) ; ces tests vérifient ses lois
//! (altitude, ombre pluviométrique, continuité), la cohérence des biomes avec le paysage (la mer,
//! le rivage, les rivières, les marais, les montagnes), les règles de la végétation, et que rien
//! de tout cela ne dépend de l'ordre ou de l'état des caches.
//!
//! `cargo test -p engine_voxel --test climate_v2 print_climate_stats --release -- --ignored
//! --nocapture` imprime les mesures citées dans `docs/plan-generation-rendu.md`.

#[allow(dead_code)]
mod support {
    pub mod area;
}

use engine_voxel::{
    BiomeKind, GenerationVersion, MacroWorld, SEA_LEVEL, SurfaceInfo, TerrainGenerator,
    WorldClimate,
};
use std::collections::HashMap;
use support::area::{Area, around_by, mountain_spots};

const SEEDS: [u32; 4] = [7, 42, 1337, 20260];

fn v2(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V2)
        .expect("la génération v2 s'initialise")
        .relief_only()
}

/// Une grille de colonnes de la génération v2 : `reach` points de chaque côté de l'origine, tous les
/// `step` blocs.
fn grid(generator: &TerrainGenerator, reach: i32, step: i32) -> Vec<(i32, i32, SurfaceInfo)> {
    let mut out = Vec::new();
    for j in -reach..=reach {
        for i in -reach..=reach {
            let (x, z) = (i * step, j * step);
            out.push((x, z, generator.surface_info(x, z).expect("v2")));
        }
    }
    out
}

fn kind_of(generator: &TerrainGenerator, info: &SurfaceInfo) -> BiomeKind {
    generator.biome(info.biome).kind
}

/// Les points du plan où le climat régional est lu : une grille large, sans lire de colonne.
fn macro_points(reach: i32, step: i32) -> impl Iterator<Item = (f64, f64)> {
    (-reach..=reach)
        .flat_map(move |j| (-reach..=reach).map(move |i| ((i * step) as f64, (j * step) as f64)))
}

// ---------- Les lois du climat ----------

#[test]
fn climate_is_pure_and_continuous() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let climate = WorldClimate::new(seed);
        let at = |x: f64, z: f64| climate.sample(world.geography(), x, z);
        // Même graine, mêmes points : mêmes valeurs, dans n'importe quel ordre.
        let points: Vec<_> = macro_points(12, 530).collect();
        let forward: Vec<_> = points.iter().map(|&(x, z)| at(x, z)).collect();
        let backward: Vec<_> = points.iter().rev().map(|&(x, z)| at(x, z)).collect();
        assert!(
            forward.iter().eq(backward.iter().rev()),
            "graine {seed} : le climat dépend de l'ordre"
        );
        // Une autre graine donne un autre climat.
        let other = WorldClimate::new(seed ^ 0x5a5a);
        let differs = points
            .iter()
            .take(64)
            .any(|&(x, z)| other.sample(world.geography(), x, z) != at(x, z));
        assert!(differs, "graine {seed} : la graine ne change rien");

        // Continuité : pas de saut au pas de 16 blocs (la grille de lecture des colonnes).
        let (mut max_t, mut max_h): (f64, f64) = (0.0, 0.0);
        for j in 0..24 {
            let z = -3000.0 + 337.0 * j as f64;
            let mut previous = at(-6000.0, z);
            for i in 1..=750 {
                let current = at(-6000.0 + 16.0 * i as f64, z);
                max_t = max_t.max((current.temperature - previous.temperature).abs());
                max_h = max_h.max((current.humidity - previous.humidity).abs());
                previous = current;
            }
        }
        assert!(max_t < 0.12, "graine {seed} : saut de température {max_t}");
        assert!(max_h < 0.14, "graine {seed} : saut d'humidité {max_h}");
    }
}

#[test]
fn altitude_cools_the_climate_and_the_sea_does_not_follow() {
    let (mut high, mut low) = (0, 0);
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let climate = WorldClimate::new(seed);
        for (x, z) in macro_points(40, 190) {
            let elevation = world.geography().macro_elevation(x, z) as f64;
            let c = climate.sample(world.geography(), x, z);
            assert!(c.temperature <= c.sea_temperature + 1e-9);
            assert!((-1.0..=1.0).contains(&c.temperature));
            assert!((-1.0..=1.0).contains(&c.humidity));
            assert!((-1.0..=1.0).contains(&c.variety));
            if elevation <= SEA_LEVEL as f64 + 25.0 {
                low += 1;
                assert!(
                    (c.temperature - c.sea_temperature).abs() < 1e-9,
                    "graine {seed} : refroidi sans altitude en ({x}, {z})"
                );
            } else if elevation > SEA_LEVEL as f64 + 90.0 && c.sea_temperature > -0.2 {
                high += 1;
                assert!(
                    c.sea_temperature - c.temperature > 0.3,
                    "graine {seed} : un sommet ({elevation}) à peine plus froid en ({x}, {z})"
                );
            }
        }
    }
    assert!(low > 1000 && high > 30, "{low} bas, {high} hauts");
}

#[test]
fn the_lee_of_a_range_is_drier_than_open_ground() {
    // Les basses terres sous l'ombre d'un relief au vent, face aux basses terres dégagées dont l'air
    // vient autant de la mer : leur humidité moyenne doit être nettement plus basse.
    let (mut shadowed, mut open) = (Vec::new(), Vec::new());
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let climate = WorldClimate::new(seed);
        for (x, z) in macro_points(60, 160) {
            let elevation = world.geography().macro_elevation(x, z) as f64;
            let lowland = (SEA_LEVEL as f64 + 4.0..SEA_LEVEL as f64 + 45.0).contains(&elevation);
            if !lowland || world.geography().coast_distance_at(x, z) < 400.0 {
                continue;
            }
            let c = climate.sample(world.geography(), x, z);
            if c.rain_shadow > 0.5 {
                shadowed.push((c.humidity, c.maritime));
            } else if c.rain_shadow < 0.03 {
                open.push((c.humidity, c.maritime));
            }
        }
    }
    assert!(
        shadowed.len() >= 30 && open.len() >= 300,
        "{} à l'ombre, {} dégagés",
        shadowed.len(),
        open.len()
    );
    let mean =
        |v: &[(f64, f64)], f: fn(&(f64, f64)) -> f64| v.iter().map(f).sum::<f64>() / v.len() as f64;
    let (dry, wet) = (mean(&shadowed, |p| p.0), mean(&open, |p| p.0));
    assert!(
        dry < wet - 0.3,
        "ombre : {dry:.2} ; dégagé : {wet:.2} (l'ombre doit assécher d'au moins 0,3)"
    );
}

#[test]
fn the_wind_blows_from_a_fixed_direction_per_seed_and_drifts_slowly() {
    for seed in SEEDS {
        let climate = WorldClimate::new(seed);
        let (wx, wz) = climate.wind(0.0, 0.0);
        assert!((wx.hypot(wz) - 1.0).abs() < 1e-9);
        // À 500 blocs, la direction n'a presque pas bougé ; à 40 000, elle a pu changer.
        let (nx, nz) = climate.wind(500.0, 500.0);
        assert!(wx * nx + wz * nz > 0.99, "graine {seed}");
    }
    let angles: Vec<f64> = SEEDS
        .iter()
        .map(|&s| {
            let (x, z) = WorldClimate::new(s).wind(0.0, 0.0);
            z.atan2(x)
        })
        .collect();
    assert!(
        angles.windows(2).any(|w| (w[0] - w[1]).abs() > 0.2),
        "les graines ont toutes le même vent"
    );
}

// ---------- Les biomes suivent le paysage ----------

/// Les colonnes lues d'une grille : (x, z, colonne).
type Columns = Vec<(i32, i32, SurfaceInfo)>;

/// Les colonnes d'une grille pour chacune des 4 graines, avec leur générateur.
fn sampled() -> Vec<(u32, TerrainGenerator, Columns)> {
    SEEDS
        .iter()
        .map(|&seed| {
            let generator = v2(seed);
            let columns = grid(&generator, 22, 150);
            (seed, generator, columns)
        })
        .collect()
}

#[test]
fn biomes_agree_with_the_relief_and_the_water() {
    for (seed, generator, columns) in sampled() {
        let geography = MacroWorld::new(seed);
        let mut seen: HashMap<BiomeKind, usize> = HashMap::new();
        for &(x, z, info) in &columns {
            let kind = kind_of(&generator, &info);
            *seen.entry(kind).or_default() += 1;
            let at = format!("graine {seed}, ({x}, {z}), {:?} {info:?}", kind);
            let bed = info.river_bed || info.lake_bed;
            assert_eq!(kind == BiomeKind::River, bed, "{at} : rivière et lit");
            match kind {
                BiomeKind::Ocean | BiomeKind::DeepOcean => {
                    assert!(info.top < SEA_LEVEL, "{at} : océan hors de la mer")
                }
                BiomeKind::Peaks => assert!(info.top > SEA_LEVEL + 85, "{at} : pic trop bas"),
                BiomeKind::Mountains => {
                    assert!(info.top > SEA_LEVEL + 10, "{at} : montagne trop basse")
                }
                BiomeKind::Beach => {
                    assert!(info.top <= SEA_LEVEL + 4, "{at} : plage en altitude");
                    let coast = geography.geography().coast_distance_at(x as f64, z as f64);
                    assert!(coast < 90.0, "{at} : plage à {coast} blocs du rivage");
                }
                _ => {}
            }
            if generator.biome(info.biome).key == "core:swamp" {
                assert!(info.top <= SEA_LEVEL + 14, "{at} : marais en hauteur");
                assert!(info.slope < 0.5, "{at} : marais sur une pente");
                assert!(info.humidity >= 0.2, "{at} : marais sec");
                assert!(info.climate > -0.3, "{at} : marais gelé");
            }
        }
        for needed in [BiomeKind::Ocean, BiomeKind::Land, BiomeKind::River] {
            assert!(
                seen.contains_key(&needed),
                "graine {seed} : pas de {needed:?}"
            );
        }
    }
}

#[test]
fn land_biomes_match_the_local_climate() {
    // Le biome d'une terre est celui dont le climat est proche (température, humidité) : aucun tirage
    // au hasard, aucune règle cachée. La variété départage seulement des variantes voisines.
    let mut checked = 0;
    for (seed, generator, columns) in sampled() {
        for &(x, z, info) in &columns {
            if kind_of(&generator, &info) != BiomeKind::Land {
                continue;
            }
            let def = generator.biome(info.biome);
            let distance = (info.climate as f64 - def.climate.0).powi(2)
                + (info.humidity as f64 - def.climate.1).powi(2);
            assert!(
                distance < 1.0,
                "graine {seed}, ({x}, {z}) : {} loin de son climat ({:.2}, {:.2}) pour ({:.2}, {:.2})",
                def.key,
                def.climate.0,
                def.climate.1,
                info.climate,
                info.humidity
            );
            checked += 1;
        }
    }
    assert!(checked > 1500, "{checked}");
}

#[test]
fn every_low_gentle_shore_is_a_beach() {
    // Tout rivage bas et doux est une plage ; aucune autre famille ne s'intercale entre la mer et la
    // terre, sauf un cours qui se jette dans la mer.
    let mut shores = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        let world = MacroWorld::new(seed);
        // Les passages mer -> terre, sur des lignes est-ouest.
        for line in 0..30 {
            let z = -4000 + 270 * line;
            let mut previous = world.geography().coast_distance_at(-6000.0, z as f64);
            for step in 1..=600 {
                let x = -6000 + 20 * step;
                let current = world.geography().coast_distance_at(x as f64, z as f64);
                if previous < 0.0 && current >= 0.0 {
                    // Quelques colonnes à l'intérieur des terres : plage ou rivage rocheux.
                    for inland in [10, 25, 40] {
                        let info = generator.surface_info(x + inland, z).expect("v2");
                        let coast = world
                            .geography()
                            .coast_distance_at((x + inland) as f64, z as f64);
                        if coast < 55.0
                            && coast > 5.0
                            && info.top <= SEA_LEVEL + 4
                            && info.top >= SEA_LEVEL
                            && info.slope < 0.9
                            && !info.river_bed
                            && !info.lake_bed
                        {
                            shores += 1;
                            assert_eq!(
                                kind_of(&generator, &info),
                                BiomeKind::Beach,
                                "graine {seed}, ({}, {z}) : un rivage qui n'est pas une plage ({info:?})",
                                x + inland
                            );
                        }
                    }
                }
                previous = current;
            }
        }
    }
    assert!(shores > 40, "{shores} colonnes de rivage vérifiées");
}

#[test]
fn coasts_have_a_short_and_ordered_sequence_of_biomes() {
    // De 120 blocs en mer à 250 blocs dans les terres : au plus 6 changements de biome (océan, plage ou
    // rivage rocheux, puis une ou deux terres). Pas de bandes qui alternent.
    let mut transects = 0;
    let mut worst = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        let world = MacroWorld::new(seed);
        for line in 0..40 {
            let z = -5000 + 250 * line;
            let mut previous = world.geography().coast_distance_at(-6000.0, z as f64);
            for step in 1..=600 {
                let x = -6000 + 20 * step;
                let current = world.geography().coast_distance_at(x as f64, z as f64);
                if previous < 0.0 && current >= 0.0 {
                    let mut changes = 0;
                    let mut last = None;
                    for dx in (-120..=250).step_by(10) {
                        let info = generator.surface_info(x + dx, z).expect("v2");
                        // Un cours qui se jette dans la mer n'est pas une alternance de bandes.
                        if kind_of(&generator, &info) == BiomeKind::River {
                            continue;
                        }
                        let id = info.biome;
                        if last.is_some_and(|l| l != id) {
                            changes += 1;
                        }
                        last = Some(id);
                    }
                    worst = worst.max(changes);
                    transects += 1;
                    if changes > 6 {
                        let names: Vec<String> = (-120..=250)
                            .step_by(10)
                            .map(|dx| {
                                let i = generator.surface_info(x + dx, z).expect("v2");
                                format!("{}:{}", dx, generator.biome(i.biome).key)
                            })
                            .collect();
                        panic!(
                            "graine {seed}, côte en ({x}, {z}) : {changes} changements de biome : {names:?}"
                        );
                    }
                }
                previous = current;
            }
        }
    }
    assert!(
        transects > 25,
        "{transects} côtes mesurées (pire : {worst})"
    );
}

#[test]
fn biomes_are_diverse_and_regions_are_large() {
    let mut land_total = 0usize;
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (seed, generator, columns) in sampled() {
        let mut keys_here: HashMap<String, usize> = HashMap::new();
        for &(_, _, info) in &columns {
            if kind_of(&generator, &info) == BiomeKind::Land {
                let key = generator.biome(info.biome).key.clone();
                *keys_here.entry(key.clone()).or_default() += 1;
                *by_key.entry(key).or_default() += 1;
                land_total += 1;
            }
        }
        assert!(keys_here.len() >= 3, "graine {seed} : {keys_here:?}");
    }
    assert!(
        by_key.len() >= 8,
        "trop peu de biomes de terre : {by_key:?}"
    );
    let biggest = by_key.values().copied().max().unwrap_or(0) as f64 / land_total as f64;
    assert!(
        biggest < 0.45,
        "un seul biome couvre {biggest:.2} des terres : {by_key:?}"
    );

    // Des régions étendues : le long d'une ligne de 6 000 blocs, un biome de terre dure en moyenne
    // plus de 250 blocs (pas de mosaïque fine).
    let (mut runs, mut length) = (0usize, 0usize);
    for seed in SEEDS {
        let generator = v2(seed);
        for line in 0..6 {
            let z = -1500 + 600 * line;
            let mut current = None;
            for i in 0..300 {
                let x = -3000 + 20 * i;
                let info = generator.surface_info(x, z).expect("v2");
                if kind_of(&generator, &info) == BiomeKind::River {
                    continue; // un cours coupe une région, il ne la change pas
                }
                let land = kind_of(&generator, &info) == BiomeKind::Land;
                let id = land.then_some(info.biome);
                match (current, id) {
                    (Some(a), Some(b)) if a == b => length += 20,
                    (_, Some(_)) => {
                        runs += 1;
                        length += 20;
                    }
                    _ => {}
                }
                current = id;
            }
        }
    }
    let mean = length as f64 / runs.max(1) as f64;
    assert!(
        mean > 250.0,
        "régions trop fines : {mean:.0} blocs en moyenne ({runs} tronçons)"
    );
}

#[test]
fn building_land_is_common() {
    // Des zones constructibles : de la terre plate hors zone réservée, partout.
    for (seed, generator, columns) in sampled() {
        let land: Vec<_> = columns
            .iter()
            .filter(|(_, _, i)| kind_of(&generator, i) == BiomeKind::Land && !i.reserved)
            .collect();
        let flat = land.iter().filter(|(_, _, i)| i.slope < 0.35).count();
        assert!(
            flat as f64 > 0.4 * land.len() as f64,
            "graine {seed} : {flat} colonnes plates sur {}",
            land.len()
        );
    }
}

// ---------- La végétation ----------

#[test]
fn vegetation_avoids_cliffs_the_tree_line_and_stays_bounded() {
    let mut zero = 0;
    for (seed, _, columns) in sampled() {
        for &(x, z, info) in &columns {
            let at = format!("graine {seed}, ({x}, {z}), {info:?}");
            assert!(
                (0.0..=3.8).contains(&info.canopy),
                "{at} : couvert hors des bornes"
            );
            if info.slope >= 1.12 {
                assert!(info.canopy == 0.0, "{at} : des arbres sur une falaise");
                zero += 1;
            }
            // La ligne des arbres ne dépasse jamais SEA + 100 + 28 + 8.
            if info.top > SEA_LEVEL + 140 {
                assert!(
                    info.canopy == 0.0,
                    "{at} : des arbres au-dessus de la ligne"
                );
            }
        }
    }
    assert!(zero > 0, "aucune falaise rencontrée");
}

/// Les troncs (bloc de bois dont le dessous est le sol) d'une zone : (x, z, y).
fn trunks(area: &Area) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for x in area.min.0..=area.max.0 {
        for z in area.min.1..=area.max.1 {
            let Some((ground, _)) = area.ground(x, z) else {
                continue;
            };
            if area.key(area.block(x, ground + 1, z)).ends_with("_log") {
                out.push((x, z, ground + 1));
            }
        }
    }
    out
}

#[test]
fn every_tree_grows_where_vegetation_is_allowed() {
    // Au pied de chaque tronc, le coefficient de végétation est positif : aucun arbre sur une falaise
    // ni au-dessus de la ligne des arbres, dans les blocs réellement générés.
    let mut counted = 0;
    for seed in SEEDS {
        let generator = TerrainGenerator::new(seed)
            .with_version(GenerationVersion::V2)
            .expect("v2");
        let mut spots = mountain_spots(seed, 2);
        spots.push(generator.find_spawn());
        for spot in spots {
            let area = around_by(&generator, spot, 3);
            for (x, z, y) in trunks(&area) {
                let info = generator.surface_info(x, z).expect("v2");
                counted += 1;
                assert!(
                    info.canopy > 0.0 && info.slope < 1.12 && y < SEA_LEVEL + 142,
                    "graine {seed}, tronc en ({x}, {y}, {z}) : {info:?}"
                );
            }
        }
    }
    assert!(counted > 200, "{counted} troncs vérifiés");
}

#[test]
fn wood_is_within_reach_of_the_spawn() {
    // Du bois à moins de 200 blocs du point d'apparition : une partie commence sans marche forcée.
    for seed in SEEDS {
        let generator = TerrainGenerator::new(seed)
            .with_version(GenerationVersion::V2)
            .expect("v2");
        let spawn = generator.find_spawn();
        let area = around_by(&generator, spawn, 6);
        let near = trunks(&area)
            .into_iter()
            .filter(|&(x, z, _)| (x - spawn.0).abs().max((z - spawn.1).abs()) <= 200)
            .count();
        assert!(
            near > 0,
            "graine {seed} : aucun arbre à 200 blocs de {spawn:?}"
        );
        // Et le point d'apparition est une terre habitable : ni mer, ni montagne, ni zone réservée.
        let info = generator.surface_info(spawn.0, spawn.1).expect("v2");
        assert_eq!(
            generator.biome(info.biome).kind,
            BiomeKind::Land,
            "graine {seed} : {info:?}"
        );
    }
}

// ---------- Déterminisme ----------

#[test]
fn local_climate_does_not_depend_on_order_or_caches() {
    for seed in SEEDS {
        let generator = v2(seed);
        let points: Vec<(i32, i32)> = (-20..20)
            .flat_map(|j| (-20..20).map(move |i| (i * 71 - 5, j * 53 + 11)))
            .collect();
        let forward: Vec<_> = points
            .iter()
            .map(|&(x, z)| generator.surface_info(x, z).unwrap())
            .collect();
        // Caches vidés, ordre inverse, générateur neuf.
        generator.clear_caches();
        let fresh = v2(seed);
        let backward: Vec<_> = points
            .iter()
            .rev()
            .map(|&(x, z)| fresh.surface_info(x, z).unwrap())
            .collect();
        assert!(
            forward.iter().eq(backward.iter().rev()),
            "graine {seed} : le climat local dépend de l'ordre"
        );
        // Caches pleins puis évincés : les mêmes valeurs encore.
        let again: Vec<_> = points
            .iter()
            .map(|&(x, z)| generator.surface_info(x, z).unwrap())
            .collect();
        assert_eq!(forward, again, "graine {seed}");
    }
}

#[test]
fn local_climate_is_continuous_across_chunk_column_borders() {
    // Aux frontières des colonnes de chunks (tous les 32 blocs), l'humidité et la température
    // se suivent : la grille de lecture du climat est la même de part et d'autre.
    for seed in SEEDS {
        let generator = v2(seed);
        let (mut max_h, mut max_t): (f32, f32) = (0.0, 0.0);
        for j in -15..15 {
            for i in -15..15 {
                let z = j * 213 + 7;
                let x = i * 32; // la colonne `x - 1` est dans la colonne de chunks voisine
                let left = generator.surface_info(x - 1, z).unwrap();
                let right = generator.surface_info(x, z).unwrap();
                max_h = max_h.max((left.humidity - right.humidity).abs());
                max_t = max_t.max((left.climate - right.climate).abs());
            }
        }
        assert!(max_h < 0.25, "graine {seed} : saut d'humidité {max_h}");
        assert!(max_t < 0.1, "graine {seed} : saut de température {max_t}");
    }
}

// ---------- Mesures du plan ----------

#[test]
#[ignore = "imprime les mesures du plan"]
fn print_climate_stats() {
    // L'ombre pluviométrique : humidité moyenne des basses terres à l'ombre et dégagées.
    let (mut shadowed, mut open) = (Vec::new(), Vec::new());
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let climate = WorldClimate::new(seed);
        for (x, z) in macro_points(60, 160) {
            let elevation = world.geography().macro_elevation(x, z) as f64;
            if !(SEA_LEVEL as f64 + 4.0..SEA_LEVEL as f64 + 45.0).contains(&elevation)
                || world.geography().coast_distance_at(x, z) < 400.0
            {
                continue;
            }
            let c = climate.sample(world.geography(), x, z);
            if c.rain_shadow > 0.5 {
                shadowed.push(c.humidity);
            } else if c.rain_shadow < 0.03 {
                open.push(c.humidity);
            }
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    println!(
        "ombre pluviométrique : {} points à l'ombre (humidité moyenne {:.2}), {} dégagés ({:.2})",
        shadowed.len(),
        mean(&shadowed),
        open.len(),
        mean(&open)
    );
    for (seed, generator, columns) in sampled() {
        let mut kinds: HashMap<String, usize> = HashMap::new();
        let mut land: HashMap<String, usize> = HashMap::new();
        for &(_, _, info) in &columns {
            let def = generator.biome(info.biome);
            *kinds.entry(format!("{:?}", def.kind)).or_default() += 1;
            if def.kind == BiomeKind::Land {
                *land.entry(def.key.clone()).or_default() += 1;
            }
        }
        let mut kinds: Vec<_> = kinds.into_iter().collect();
        kinds.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let mut land: Vec<_> = land.into_iter().collect();
        land.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let swamps: Vec<_> = columns
            .iter()
            .filter(|(_, _, i)| generator.biome(i.biome).key == "core:swamp")
            .map(|(x, z, _)| (*x, *z))
            .take(6)
            .collect();
        let land_columns: usize = land.iter().map(|(_, n)| n).sum();
        let swamp_columns = columns
            .iter()
            .filter(|(_, _, i)| generator.biome(i.biome).key == "core:swamp")
            .count();
        println!("graine {seed} : marais {swamp_columns}/{land_columns} terres, en {swamps:?}");
        println!("graine {seed} : familles {kinds:?}");
        println!("graine {seed} : terres {land:?}");
    }
}
