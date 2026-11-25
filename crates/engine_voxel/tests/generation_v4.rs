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
