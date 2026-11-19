//! La génération v3, lot 2 du plan : le relief en densité 3D. Version et plage, cohérence des hauteurs
//! avec les blocs, eau jamais perchée (les rivières sont au niveau de la mer), sous-sol plein,
//! déterminisme, variété du relief, chargement par le vrai streamer, et coût.

use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, ChunkStreamer, GenerationVersion, MAX_CHUNK_Y, MIN_CHUNK_Y, SEA_LEVEL, Shape,
    StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

fn v3(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V3)
        .expect("la génération v3 s'initialise")
}

/// Le bloc (x, y, z) d'une colonne de chunks générée (`chunks[cy - MIN_CHUNK_Y]`).
fn column_chunks(g: &TerrainGenerator, cx: i32, cz: i32) -> Vec<Option<engine_voxel::Chunk>> {
    (MIN_CHUNK_Y..MAX_CHUNK_Y)
        .map(|cy| g.generate(IVec3::new(cx, cy, cz)))
        .collect()
}

fn block(chunks: &[Option<engine_voxel::Chunk>], lx: usize, y: i32, lz: usize) -> BlockId {
    let cy = (y.div_euclid(32) - MIN_CHUNK_Y) as usize;
    chunks[cy]
        .as_ref()
        .map_or(BlockId::AIR, |c| c.get(lx, y.rem_euclid(32) as usize, lz))
}

#[test]
fn v3_is_a_test_version_numbered_3_with_the_tall_range() {
    assert_eq!(GenerationVersion::V3.number(), 3);
    assert_eq!(
        GenerationVersion::from_number(3).unwrap(),
        GenerationVersion::V3
    );
    assert_eq!(
        GenerationVersion::for_new_world_with(Some("v3")),
        GenerationVersion::V3
    );
    assert_eq!(GenerationVersion::FOR_NEW_WORLDS, GenerationVersion::V1);
    assert_eq!(v3(1).chunk_range(), MIN_CHUNK_Y..MAX_CHUNK_Y);
    assert!(v3(1).v3_parameters(0, 0).is_some());
    assert!(TerrainGenerator::new(1).v3_parameters(0, 0).is_none());
}

#[test]
fn heights_match_the_blocks_and_water_never_sits_above_the_sea() {
    for seed in [7, 42, 1337] {
        let g = v3(seed);
        let registry = g.registry().clone();
        let water = registry.id("core:water").unwrap();
        // Un relief de terrain : un cube plein qui n'est ni un tronc ni des feuilles.
        let terrain = |b: BlockId| {
            let def = registry.def(b);
            def.solid
                && def.shape == Shape::Cube
                && !def.key.ends_with("_log")
                && !def.key.ends_with("_leaves")
        };
        for (cx, cz) in [(0, 0), (17, -5), (-40, 33), (90, 12)] {
            let chunks = column_chunks(&g, cx, cz);
            for lz in (0..32).step_by(5) {
                for lx in (0..32).step_by(5) {
                    let (x, z) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                    let height = g.column(x, z).height;
                    assert!(
                        terrain(block(&chunks, lx, height, lz)),
                        "graine {seed} ({x}, {z}) : rien de plein à la hauteur {height}"
                    );
                    for y in height + 1..(MAX_CHUNK_Y * 32) {
                        let b = block(&chunks, lx, y, lz);
                        assert!(
                            !terrain(b),
                            "graine {seed} ({x}, {z}) : du terrain à {y} au-dessus de {height}"
                        );
                        if b == water {
                            assert!(
                                y <= SEA_LEVEL,
                                "graine {seed} ({x}, {z}) : de l'eau perchée à {y}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn valley_floors_inland_are_below_the_sea_so_rivers_cannot_be_perched() {
    // Le bruit 3D peut relever un fond de vallée de quelques blocs (un gué à sec) ; presque tous sont
    // sous la mer, aucun n'est haut.
    let g = v3(42);
    let (mut checked, mut wet) = (0, 0);
    for j in -60..60 {
        for i in -60..60 {
            let (x, z) = (i * 37, j * 41);
            let p = g.v3_parameters(x, z).unwrap();
            if p.pv < -0.97 && p.continentalness > 0.05 {
                let h = g.column(x, z).height;
                assert!(
                    h <= SEA_LEVEL + 3,
                    "({x}, {z}) : fond de vallée à {h} ({p:?})"
                );
                checked += 1;
                wet += usize::from(h < SEA_LEVEL);
            }
        }
    }
    assert!(checked > 20, "trop peu de fonds de vallée vus : {checked}");
    assert!(
        wet * 100 >= checked * 95,
        "{wet} fonds sous la mer sur {checked}"
    );
}

#[test]
fn the_underground_is_solid_rock_until_caves_arrive() {
    let g = v3(42);
    let deepstone = g.registry().id("core:deepstone").unwrap();
    for (cx, cz) in [(0, 0), (12, -30)] {
        for cy in [MIN_CHUNK_Y, -2] {
            let chunk = g.generate(IVec3::new(cx, cy, cz)).expect("plein");
            assert!(
                chunk.blocks().iter().all(|b| !b.is_air()),
                "trou dans le chunk {cx} {cy} {cz}"
            );
            assert!(chunk.blocks().contains(&deepstone));
        }
    }
}

#[test]
fn the_relief_has_seas_lowlands_and_high_mountains() {
    let g = v3(42);
    let (mut lowest, mut highest, mut sea, mut land) = (i32::MAX, i32::MIN, 0, 0);
    for j in -40..40 {
        for i in -40..40 {
            let h = g.column(i * 100, j * 100).height;
            lowest = lowest.min(h);
            highest = highest.max(h);
            if h < SEA_LEVEL { sea += 1 } else { land += 1 }
        }
    }
    assert!(lowest < SEA_LEVEL - 20, "pas d'océan profond : {lowest}");
    assert!(
        highest > SEA_LEVEL + 100,
        "pas de haute montagne : {highest}"
    );
    assert!(sea > 600 && land > 1500, "mer {sea}, terre {land}");
}

#[test]
fn v3_blocks_do_not_depend_on_order_threads_or_caches() {
    let positions: Vec<IVec3> = (0..6)
        .map(|i| IVec3::new(i * 3 - 7, i % 5 - 1, 11 - i * 2))
        .collect();
    let g = v3(2024);
    let forward: Vec<_> = positions.iter().map(|&p| g.generate(p)).collect();
    let fresh = v3(2024);
    let backward: Vec<_> = positions.iter().rev().map(|&p| fresh.generate(p)).collect();
    assert_eq!(forward, backward.into_iter().rev().collect::<Vec<_>>());
    let shared = v3(2024);
    let threaded: Vec<_> = std::thread::scope(|scope| {
        let g = &shared;
        let handles: Vec<_> = positions
            .iter()
            .map(|&p| scope.spawn(move || g.generate(p)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(forward, threaded);
    assert_ne!(v3(2025).generate(positions[2]), forward[2]);
}

#[test]
fn a_v3_world_loads_lights_and_meshes_with_the_real_streamer() {
    let config = StreamingConfig {
        view_radius: 2,
        unload_margin: 1,
        ..StreamingConfig::default()
    };
    let g = v3(7);
    let (x, z) = g.find_spawn();
    let height = g.column(x, z).height;
    let mut s = ChunkStreamer::new(g, config);
    assert_eq!(s.world().vertical(), MIN_CHUNK_Y..MAX_CHUNK_Y);
    let player = Vec3::new(x as f32, height as f32 + 2.0, z as f32);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut meshes = 0;
    loop {
        s.update(player);
        while let Some(event) = s.next_event() {
            meshes += usize::from(matches!(event, engine_voxel::StreamEvent::Mesh(..)));
        }
        if s.is_idle() {
            break;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(meshes > 10);
    // Le joueur apparaît sur la terre ferme, au-dessus de la mer.
    assert!(height > SEA_LEVEL, "apparition à {height}");
    assert!(s.world().block(IVec3::new(x, height, z)) != BlockId::AIR);
}

/// `cargo test -p engine_voxel --test generation_v3 print_v3_cost --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_v3_cost() {
    for (name, g) in [("v1", TerrainGenerator::new(42)), ("v3", v3(42))] {
        let started = Instant::now();
        let mut chunks = 0;
        for cx in 0..6 {
            for cz in 0..6 {
                for cy in g.chunk_range() {
                    if g.generate(IVec3::new(cx + 100, cy, cz - 50)).is_some() {
                        chunks += 1;
                    }
                }
            }
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{name} : 36 colonnes en {elapsed:.0} ms ({:.2} ms par colonne, {chunks} chunks non vides)",
            elapsed / 36.0
        );
    }
}
