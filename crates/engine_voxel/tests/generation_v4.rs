//! La génération v4 (`docs/plan-biomes.md`) : le climat suit la latitude (l'axe z). Plus on va vers le
//! nord (z négatif), plus il fait froid ; plus on va vers le sud, plus il fait chaud ; le monde ne
//! change pas pour les anciennes versions.

use engine_voxel::{GenerationVersion, TerrainGenerator};

fn generator(version: GenerationVersion, seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(version)
        .expect("la génération s'initialise")
}

/// La température moyenne d'une bande de latitude (autour de `z`), sur plusieurs x.
fn band_temperature(g: &TerrainGenerator, z: i32) -> f64 {
    let samples: Vec<f64> = (0..32)
        .map(|i| {
            g.v3_parameters(i * 700 - 11_000, z)
                .expect("génération à densité")
                .temperature
        })
        .collect();
    samples.iter().sum::<f64>() / samples.len() as f64
}

#[test]
fn v4_temperature_rises_from_the_north_to_the_south() {
    let g = generator(GenerationVersion::V4, 7);
    let bands: Vec<f64> = [-5_500, -3_500, -1_500, 0, 1_500, 3_500, 5_500]
        .into_iter()
        .map(|z| band_temperature(&g, z))
        .collect();
    for pair in bands.windows(2) {
        assert!(
            pair[0] < pair[1],
            "la température doit monter vers le sud : {bands:?}"
        );
    }
    // Un vrai contraste : glacial au nord, brûlant au sud, tempéré au milieu.
    assert!(bands[0] < -0.5, "{bands:?}");
    assert!(bands[6] > 0.5, "{bands:?}");
    assert!(bands[3].abs() < 0.3, "{bands:?}");
}

#[test]
fn v4_climate_reflects_instead_of_climbing_forever() {
    let g = generator(GenerationVersion::V4, 7);
    // Après le pôle chaud (z = 6000), l'onde redescend : 12 000 est de nouveau tempéré.
    assert!(band_temperature(&g, 12_000).abs() < 0.3);
}

#[test]
fn v3_worlds_keep_their_own_climate() {
    // La v3 n'a pas de gradient : le nord et le sud lointains ne sont pas systématiquement opposés.
    let g = generator(GenerationVersion::V3, 7);
    let north = band_temperature(&g, -5_500);
    let south = band_temperature(&g, 5_500);
    assert!((south - north).abs() < 0.8, "{north} {south}");
}

/// Les biomes de terre d'une bande de latitude, avec leur part (en %), du plus fréquent au plus rare.
fn band_biomes(g: &TerrainGenerator, z0: i32, z1: i32) -> Vec<(String, u32)> {
    let mut counts = std::collections::HashMap::<String, u32>::new();
    let mut total = 0;
    for z in (z0..z1).step_by(250) {
        for x in (-12_000..12_000).step_by(250) {
            let biome = g.biome(g.biome_at(x, 100, z));
            if matches!(
                biome.kind,
                engine_voxel::BiomeKind::Land | engine_voxel::BiomeKind::Mountains
            ) {
                *counts.entry(biome.key.clone()).or_default() += 1;
                total += 1;
            }
        }
    }
    let mut list: Vec<_> = counts
        .into_iter()
        .map(|(k, n)| (k, n * 100 / total.max(1)))
        .collect();
    list.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    list
}

/// La part (en %) d'un groupe de biomes dans une bande de latitude.
fn share(list: &[(String, u32)], keys: &[&str]) -> u32 {
    list.iter()
        .filter(|(k, _)| keys.contains(&k.as_str()))
        .map(|(_, n)| n)
        .sum()
}

#[test]
fn v4_biomes_follow_the_latitude() {
    let g = generator(GenerationVersion::V4, 7);
    let snowy = [
        "core:snowy_plains",
        "core:snowy_taiga",
        "core:snowy_fir_forest",
        "core:snowy_tundra",
    ];
    let wet_south = ["core:jungle", "core:rainforest", "core:sparse_jungle"];
    let deserts = [
        "core:desert",
        "core:arid_desert",
        "core:badlands",
        "core:flowering_desert",
    ];
    // Le grand nord : presque tout est enneigé.
    let far_north = band_biomes(&g, -6000, -5000);
    assert!(share(&far_north, &snowy) >= 90, "{far_north:?}");
    // Le milieu : ni neige ni désert, des prairies.
    let middle = band_biomes(&g, -500, 500);
    assert_eq!(
        share(&middle, &snowy) + share(&middle, &deserts),
        0,
        "{middle:?}"
    );
    assert!(share(&middle, &["core:great_prairie"]) >= 10, "{middle:?}");
    // Le sud humide : de la jungle et de la forêt humide.
    let south = band_biomes(&g, 3000, 4000);
    assert!(share(&south, &wet_south) >= 50, "{south:?}");
    // L'extrême sud : des déserts, rien de glacé.
    let far_south = band_biomes(&g, 5000, 6000);
    assert!(share(&far_south, &deserts) >= 40, "{far_south:?}");
    assert_eq!(share(&far_south, &snowy), 0, "{far_south:?}");
    assert!(
        share(&far_south, &["core:arid_desert"]) >= 10,
        "{far_south:?}"
    );
    // Les érables vivent dans la bande fraîche, vers le nord.
    let cool = band_biomes(&g, -2200, -1000);
    assert!(share(&cool, &["core:autumn_maples"]) >= 10, "{cool:?}");
}

/// Le rang d'un biome de terre sur l'échelle des climats, du pôle à l'équateur : deux biomes qui se
/// touchent ne doivent pas être à plus d'un rang l'un de l'autre (pas de jungle collée à une forêt
/// de chênes). `None` : un biome de relief, d'eau ou souterrain, qui ne compte pas.
fn climate_rank(key: &str) -> Option<(f32, &'static str)> {
    let rank = match key {
        "core:snowy_plains"
        | "core:snowy_taiga"
        | "core:snowy_fir_forest"
        | "core:snowy_tundra"
        | "core:fjord" => (0.0, "polaire"),
        "core:taiga" | "core:old_taiga" | "core:birch_forest" | "core:steppe" | "core:tundra"
        | "core:moor" | "core:maple_woods" | "core:autumn_maples" | "core:highland"
        | "core:crag" => (1.0, "froid"),
        "core:forest"
        | "core:dark_forest"
        | "core:plains"
        | "core:flower_meadow"
        | "core:great_prairie"
        | "core:prairie"
        | "core:orchard"
        | "core:cherry_grove"
        | "core:lavender_field"
        | "core:sunflower_plains"
        | "core:redwood_forest"
        | "core:mixed_forest"
        | "core:swamp"
        | "core:wooded_hills" => (2.0, "tempéré"),
        "core:savanna" | "core:shrubland" | "core:sparse_jungle" | "core:maquis"
        | "core:hot_springs" | "core:rainforest" | "core:bayou" => (3.0, "chaud"),
        "core:jungle" => (4.0, "tropical"),
        "core:desert"
        | "core:badlands"
        | "core:arid_desert"
        | "core:flowering_desert"
        | "core:volcanic_plains" => (4.0, "aride"),
        _ => return None,
    };
    Some(rank)
}

/// La part (en ‰) des frontières de terre entre deux biomes que l'échelle des climats interdit, et la
/// liste des paires fautives les plus fréquentes.
fn incoherent_borders(g: &TerrainGenerator, z0: i32, z1: i32) -> (u32, Vec<(String, u32)>) {
    let step = 128;
    let xs: Vec<i32> = (-4_000..4_000).step_by(step as usize).collect();
    let zs: Vec<i32> = (z0..z1).step_by(step as usize).collect();
    // Un seul calcul par point de la grille.
    let grid: Vec<Vec<String>> = zs
        .iter()
        .map(|&z| {
            xs.iter()
                .map(|&x| g.biome(g.biome_at(x, 100, z)).key.clone())
                .collect()
        })
        .collect();
    let mut pairs = std::collections::HashMap::<String, u32>::new();
    let (mut borders, mut bad) = (0u32, 0u32);
    for (zi, row) in grid.iter().enumerate() {
        for (xi, a) in row.iter().enumerate() {
            let right = row.get(xi + 1);
            let down = grid.get(zi + 1).map(|r| &r[xi]);
            for b in [right, down].into_iter().flatten() {
                if a == b {
                    continue;
                }
                let (Some((ra, fa)), Some((rb, fb))) = (climate_rank(a), climate_rank(b)) else {
                    continue;
                };
                borders += 1;
                let tropical_arid =
                    (fa == "tropical" && fb == "aride") || (fb == "tropical" && fa == "aride");
                if (ra - rb).abs() > 1.0 || tropical_arid {
                    bad += 1;
                    let mut names = [a.clone(), b.clone()];
                    names.sort();
                    *pairs.entry(names.join(" | ")).or_default() += 1;
                }
            }
        }
    }
    let mut list: Vec<_> = pairs.into_iter().collect();
    list.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    (bad * 1000 / borders.max(1), list)
}

/// Deux biomes de terre qui se touchent restent voisins sur l'échelle des climats : pas de jungle collée
/// à une forêt de chênes. Une marge (1,5 %) pour les frontières de relief que la grille grossière voit mal.
#[test]
fn v4_land_borders_follow_the_climate_ladder() {
    for seed in [7, 99] {
        let g = generator(GenerationVersion::V4, seed);
        let (permille, worst) = incoherent_borders(&g, -6000, 6000);
        assert!(
            permille <= 15,
            "graine {seed} : {permille} ‰, pires paires {worst:?}"
        );
    }
}

/// L'humidité vient de la mer : à latitude égale, les côtes sont plus humides que l'intérieur des
/// continents, et le fond des vallées un peu plus que les crêtes.
#[test]
fn v4_humidity_falls_with_the_distance_from_the_sea() {
    let g = generator(GenerationVersion::V4, 7);
    let (mut coast, mut inland, mut valley, mut ridge) = (vec![], vec![], vec![], vec![]);
    for z in (-1500..1500).step_by(50) {
        for x in (-12_000..12_000).step_by(50) {
            let p = g.v3_parameters(x, z).unwrap();
            if (-0.1..0.15).contains(&p.continentalness) {
                coast.push(p.humidity);
            } else if p.continentalness > 0.7 {
                inland.push(p.humidity);
            }
            if p.continentalness > 0.0 {
                if p.pv < -0.8 {
                    valley.push(p.humidity);
                } else if p.pv > 0.5 {
                    ridge.push(p.humidity);
                }
            }
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    println!(
        "côte {:.3} ({}) intérieur {:.3} ({}) vallée {:.3} crête {:.3}",
        mean(&coast),
        coast.len(),
        mean(&inland),
        inland.len(),
        mean(&valley),
        mean(&ridge)
    );
    assert!(coast.len() > 50 && inland.len() > 50);
    assert!(
        mean(&coast) > mean(&inland) + 0.25,
        "côte {} intérieur {}",
        mean(&coast),
        mean(&inland)
    );
    assert!(
        mean(&valley) > mean(&ridge),
        "vallée {} crête {}",
        mean(&valley),
        mean(&ridge)
    );
}
