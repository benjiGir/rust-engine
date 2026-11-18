//! Le terrain lointain (lot 8) lit le même monde que les chunks : l'eau des lacs et des cours
//! d'un monde v2 y apparaît à la hauteur que donne le générateur, pas seulement la mer.

use engine_voxel::{
    FAR_CELLS, FarMesh, FarTile, GenerationVersion, SEA_LEVEL, TerrainGenerator, build_tile,
};

fn v2(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V2)
        .expect("la génération v2 s'initialise")
}

/// Les hauteurs (monde) des sommets d'eau d'un maillage.
fn water_heights(mesh: &FarMesh) -> Vec<f32> {
    let mut seen = std::collections::BTreeSet::new();
    for &i in &mesh.translucent_indices {
        seen.insert(i);
    }
    seen.into_iter()
        .map(|i| mesh.origin.y + mesh.vertices[i as usize].local_position().y * mesh.scale)
        .collect()
}

/// La tuile fine (niveau 0) qui contient le point (x, z).
fn fine_tile(x: i32, z: i32) -> FarTile {
    let size = FarTile {
        level: 0,
        x: 0,
        z: 0,
    }
    .size_blocks();
    FarTile {
        level: 0,
        x: x.div_euclid(size),
        z: z.div_euclid(size),
    }
}

/// Un point d'eau douce au-dessus de la mer (lac ou cours), s'il y en a dans la zone balayée.
fn inland_water(generator: &TerrainGenerator) -> Option<(i32, i32, i32)> {
    for j in -150..150 {
        for i in -150..150 {
            let (x, z) = (i * 24, j * 24);
            let info = generator.surface_info(x, z).expect("v2");
            if let Some(w) = info.water
                && w > SEA_LEVEL + 2
            {
                return Some((x, z, w));
            }
        }
    }
    None
}

#[test]
fn v2_far_tiles_put_lake_and_river_water_at_the_generator_level() {
    let mut checked = 0;
    for seed in [7, 42, 1337] {
        let generator = v2(seed);
        let Some((x, z, level)) = inland_water(&generator) else {
            continue;
        };
        let tile = fine_tile(x, z);
        let mesh = build_tile(&generator, tile);
        assert!(
            !mesh.translucent_indices.is_empty(),
            "graine {seed} : pas d'eau au lointain"
        );
        // Chaque sommet d'eau est à la surface d'un point d'eau échantillonné de la tuile.
        let (ox, oz) = tile.origin_blocks();
        let step = tile.step();
        let mut surfaces = Vec::new();
        for j in 0..=FAR_CELLS {
            for i in 0..=FAR_CELLS {
                let info = generator
                    .surface_info(ox + i * step, oz + j * step)
                    .expect("v2");
                if let Some(w) = info.water {
                    surfaces.push(w as f32 + 0.875);
                }
            }
        }
        for y in water_heights(&mesh) {
            assert!(
                surfaces.iter().any(|s| (s - y).abs() < 0.02),
                "graine {seed} : eau à y = {y}, absente des niveaux du générateur"
            );
        }
        assert!(
            water_heights(&mesh)
                .iter()
                .any(|&y| y > SEA_LEVEL as f32 + 2.0),
            "graine {seed} : l'eau de l'intérieur des terres (niveau {level}) est ramenée à la mer"
        );
        assert!(
            mesh.bounds.1.y >= level as f32 + 0.875,
            "la boîte englobante coupe l'eau"
        );
        checked += 1;
    }
    assert!(
        checked >= 2,
        "trop peu de graines avec de l'eau intérieure : {checked}"
    );
}

#[test]
fn v1_far_water_is_still_only_the_sea() {
    let generator = TerrainGenerator::new(42);
    let mut seen_water = false;
    for z in -6..6 {
        for x in -6..6 {
            let mesh = build_tile(&generator, FarTile { level: 0, x, z });
            for y in water_heights(&mesh) {
                seen_water = true;
                assert!(
                    (y - (SEA_LEVEL as f32 + 0.875)).abs() < 0.02,
                    "eau v1 à y = {y}"
                );
            }
        }
    }
    assert!(seen_water, "aucune tuile d'eau trouvée autour de l'origine");
}

/// `cargo test -p engine_voxel --test far_v2 print_tile_costs --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_tile_costs() {
    for (name, generator) in [("v1", TerrainGenerator::new(42)), ("v2", v2(42))] {
        for level in 0..5u8 {
            let started = std::time::Instant::now();
            let n = 24;
            let mut verts = 0;
            for i in 0..n {
                let mesh = build_tile(
                    &generator,
                    FarTile {
                        level,
                        x: 40 + i % 6,
                        z: 40 + i / 6,
                    },
                );
                verts += mesh.vertices.len();
            }
            println!(
                "{name} niveau {level} : {:.2} ms par tuile, {} sommets en moyenne",
                started.elapsed().as_secs_f64() * 1000.0 / n as f64,
                verts / n as usize
            );
        }
    }
}
