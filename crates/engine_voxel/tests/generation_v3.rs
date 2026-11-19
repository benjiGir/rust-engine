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

/// Ce que contient le sous-sol de quelques colonnes de chunks : vide, eau, lave, eau suspendue au-dessus
/// du vide, et vide juste sous un fond de mer.
#[derive(Debug, Default)]
struct Underground {
    total: usize,
    air: usize,
    water: usize,
    lava: usize,
    floating: usize,
    leaks: usize,
    ocean_columns: usize,
}

fn underground(seed: u32) -> Underground {
    let g = v3(seed);
    let registry = g.registry().clone();
    let (water, lava) = (
        registry.id("core:water").unwrap(),
        registry.id("core:lava").unwrap(),
    );
    let mut u = Underground::default();
    for (cx, cz) in [
        (30, -20),
        (31, -20),
        (-12, 40),
        (5, 5),
        (70, -70),
        (-55, -9),
    ] {
        let chunks = column_chunks(&g, cx, cz);
        for lz in 0..32 {
            for lx in 0..32 {
                let (x, z) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                let h = g.column(x, z).height;
                for y in MIN_CHUNK_Y * 32 + 1..h - 12 {
                    u.total += 1;
                    let b = block(&chunks, lx, y, lz);
                    let below = block(&chunks, lx, y - 1, lz);
                    if b.is_air() {
                        u.air += 1;
                    } else if b == water || b == lava {
                        if b == water {
                            u.water += 1
                        } else {
                            u.lava += 1
                        }
                        u.floating += usize::from(below.is_air());
                    }
                }
                if h < SEA_LEVEL - 4 {
                    u.ocean_columns += 1;
                    // Sous un fond de mer, la croûte tient : pas de vide dans les 6 blocs.
                    u.leaks += (h - 6..=h)
                        .filter(|&y| block(&chunks, lx, y, lz).is_air())
                        .count();
                }
            }
        }
    }
    u
}

#[test]
fn caves_hollow_the_underground_without_hanging_water_or_leaking_seas() {
    for seed in [7, 42] {
        let u = underground(seed);
        let share = |n: usize| n as f64 / u.total as f64;
        assert!(
            (0.03..0.25).contains(&(share(u.air) + share(u.water) + share(u.lava))),
            "graine {seed} : {u:?}"
        );
        assert!(
            u.air > 0 && u.water > 0,
            "graine {seed} : des grottes sèches et des nappes : {u:?}"
        );
        assert_eq!(
            u.floating, 0,
            "graine {seed} : de l'eau ou de la lave au-dessus du vide"
        );
        assert_eq!(
            u.leaks, 0,
            "graine {seed} : un trou sous un fond de mer ({u:?})"
        );
    }
}

#[test]
fn lava_lies_only_at_the_bottom_and_underground_water_is_flat_per_region() {
    let g = v3(42);
    let registry = g.registry().clone();
    let (water, lava) = (
        registry.id("core:water").unwrap(),
        registry.id("core:lava").unwrap(),
    );
    let chunks = column_chunks(&g, 30, -20);
    // Pour chaque colonne, le plus haut bloc d'eau souterraine (sous la croûte).
    let mut tops = std::collections::HashMap::new();
    for lz in 0..32 {
        for lx in 0..32 {
            let h = g.column(30 * 32 + lx as i32, -20 * 32 + lz as i32).height;
            for y in MIN_CHUNK_Y * 32..h - 15 {
                let b = block(&chunks, lx, y, lz);
                if b == lava {
                    assert!(y < MIN_CHUNK_Y * 32 + 20, "lave à {y}");
                }
                if b == water && !block(&chunks, lx, y + 1, lz).is_air() {
                    continue;
                }
                if b == water {
                    tops.insert((lx, lz), y);
                }
            }
        }
    }
    // Une même région de nappe a un seul niveau : peu de niveaux différents sur un chunk.
    let levels: std::collections::BTreeSet<i32> = tops.values().copied().collect();
    assert!(
        levels.len() <= 3,
        "trop de niveaux d'eau sur un chunk : {levels:?}"
    );
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
fn the_far_terrain_estimate_follows_the_real_surface() {
    // `column_at_grid_point` (le terrain lointain) cherche la surface sur une seule verticale, sans
    // grottes ni interpolation de cellule : il doit rester près de la vraie hauteur.
    let g = v3(42);
    let (mut close, mut total) = (0, 0);
    for i in 0..400 {
        let (gx, gz) = (i * 37 % 2000 - 1000, i * 53 % 2000 - 1000);
        let far = g.column_at_grid_point(gx, gz).height;
        let real = g.column(gx * 4, gz * 4).height;
        total += 1;
        close += usize::from((far - real).abs() <= 3);
        assert!(
            (far - real).abs() <= 24,
            "({gx}, {gz}) : lointain {far}, réel {real}"
        );
    }
    assert!(
        close * 100 >= total * 90,
        "{close} sur {total} à 3 blocs près"
    );
}

#[test]
fn biomes_follow_the_parameters() {
    use engine_voxel::BiomeKind;
    let g = v3(42);
    let (mut rivers, mut oceans, mut peaks) = ((0, 0), (0, 0), (0, 0));
    for j in -50..50 {
        for i in -50..50 {
            let (x, z) = (i * 61, j * 67);
            let p = g.v3_parameters(x, z).unwrap();
            let kind = g.biome(g.column(x, z).biome).kind;
            if p.pv < -0.95 && p.continentalness > 0.05 && p.erosion > -0.4 {
                rivers.0 += 1;
                rivers.1 += usize::from(kind == BiomeKind::River);
            }
            if p.continentalness < -0.5 {
                oceans.0 += 1;
                oceans.1 += usize::from(matches!(kind, BiomeKind::Ocean | BiomeKind::DeepOcean));
            }
            if p.pv > 0.75 && p.erosion < -0.6 && p.continentalness > 0.1 {
                peaks.0 += 1;
                peaks.1 += usize::from(kind == BiomeKind::Peaks);
            }
        }
    }
    for (name, (seen, right)) in [("rivières", rivers), ("mers", oceans), ("pics", peaks)] {
        assert!(seen > 5, "{name} : trop peu de points ({seen})");
        assert_eq!(right, seen, "{name} : {right} bons sur {seen}");
    }
}

#[test]
fn surface_rules_keep_grass_off_cold_tops_steep_slopes_and_cave_walls() {
    let g = v3(42);
    let registry = g.registry().clone();
    let grass = registry.id("core:grass").unwrap();
    for (cx, cz) in [(30, -20), (-12, 40), (70, -70), (5, 5)] {
        let chunks = column_chunks(&g, cx, cz);
        for lz in 0..32 {
            for lx in 0..32 {
                let (x, z) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
                let column = g.column(x, z);
                let top = column.height;
                let biome = g.biome(column.biome);
                let altitude = (top - (SEA_LEVEL + 40)).max(0) as f32;
                let cold = biome.temperature - altitude * 0.012 < engine_voxel::FREEZING;
                if block(&chunks, lx, top, lz) == grass {
                    assert!(!cold, "({x}, {z}) : de l'herbe là où il gèle (à {top})");
                }
                // Sous le sol du dessus (plancher ou plafond de grotte), jamais d'herbe.
                for y in MIN_CHUNK_Y * 32..top - 6 {
                    assert_ne!(
                        block(&chunks, lx, y, lz),
                        grass,
                        "({x}, {y}, {z}) : herbe sous terre"
                    );
                }
            }
        }
    }
}

#[test]
fn the_new_biomes_appear_in_v3_and_never_in_v1_or_v2() {
    let wave = [
        // Vague 1.
        "core:redwood_forest",
        "core:bayou",
        "core:lavender_field",
        "core:prairie",
        "core:orchard",
        "core:maple_woods",
        // Vague 2.
        "core:tundra",
        "core:snowy_fir_forest",
        "core:highland",
        "core:moor",
        "core:crag",
    ];
    let g = v3(42);
    let ids: Vec<_> = wave.iter().map(|k| g.biomes().id(k).unwrap()).collect();
    let mut seen = vec![false; ids.len()];
    for j in -60..60 {
        for i in -60..60 {
            // Le point de grille (terrain lointain) : les mêmes paramètres et la même table, sans
            // contexte de colonne (bien plus rapide).
            let b = g.column_at_grid_point(i * 25, j * 25).biome;
            if let Some(k) = ids.iter().position(|&id| id == b) {
                seen[k] = true;
            }
        }
    }
    for (key, found) in wave.iter().zip(&seen) {
        assert!(found, "{key} absent d'un monde v3 de 12 km de côté");
    }
    // Les mondes v1 et v2 ne changent pas : ils ne choisissent jamais ces biomes.
    let older = [
        TerrainGenerator::new(42),
        TerrainGenerator::new(42)
            .with_version(GenerationVersion::V2)
            .unwrap(),
    ];
    for g in &older {
        for j in -15..15 {
            for i in -15..15 {
                let b = g.column(i * 400, j * 400).biome;
                assert!(
                    !g.biome(b).v3_only,
                    "{} dans un monde ancien",
                    g.biome(b).key
                );
            }
        }
    }
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

/// La part de terre de chaque biome v3, sur plusieurs graines (points de grille du terrain lointain).
/// `cargo test -p engine_voxel --test generation_v3 print_v3_biome_shares --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_v3_biome_shares() {
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    let mut land = 0u64;
    for seed in [1, 42, 2024, 7, 99] {
        let g = v3(seed);
        for j in -80..80 {
            for i in -80..80 {
                let c = g.column_at_grid_point(i * 24, j * 24);
                if c.height < SEA_LEVEL {
                    continue;
                }
                land += 1;
                *counts.entry(g.biome(c.biome).key.clone()).or_default() += 1;
            }
        }
    }
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (key, n) in rows {
        println!("{key:28} {:5.2} %", n as f64 * 100.0 / land as f64);
    }
}
