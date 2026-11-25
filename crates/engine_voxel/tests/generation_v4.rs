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
