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
fn v3_is_the_new_worlds_version_numbered_3_with_the_tall_range() {
    assert_eq!(GenerationVersion::V3.number(), 3);
    assert_eq!(
        GenerationVersion::from_number(3).unwrap(),
        GenerationVersion::V3
    );
    assert_eq!(
        GenerationVersion::for_new_world_with(Some("v3")),
        GenerationVersion::V3
    );
    assert_eq!(GenerationVersion::FOR_NEW_WORLDS, GenerationVersion::V3);
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
                        // Seuls les bassins des sources chaudes (deux blocs au plus) dépassent la mer.
                        if b == water && g.biome(g.column(x, z).biome).key != "core:hot_springs" {
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
            let biome = g.biome(g.column(x, z).biome);
            // Un volcan recouvre ce qu'il y avait (vallée, pic) : son biome suit le cône.
            if biome.key == "core:volcano" {
                continue;
            }
            let kind = biome.kind;
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
fn the_new_biomes_appear_in_v3_and_never_in_v1() {
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
        // Vague 3.
        "core:volcano",
        "core:volcanic_plains",
        "core:hot_springs",
        "core:maquis",
        "core:flowering_desert",
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
    // Les mondes v1 ne changent pas : ils ne choisissent jamais ces biomes.
    for g in &[TerrainGenerator::new(42)] {
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

/// Un liquide bien tenu : ni vide dessous, ni vide à côté (dans la zone générée) ; sinon il coulerait.
fn assert_contained(
    chunks: &std::collections::HashMap<(i32, i32), Vec<Option<engine_voxel::Chunk>>>,
    liquid: BlockId,
    what: &str,
) -> std::collections::BTreeMap<(i32, i32), i32> {
    let at = |x: i32, y: i32, z: i32| {
        chunks
            .get(&(x.div_euclid(32), z.div_euclid(32)))
            .map(|c| block(c, x.rem_euclid(32) as usize, y, z.rem_euclid(32) as usize))
    };
    let mut tops = std::collections::BTreeMap::new();
    for (&(cx, cz), column) in chunks {
        for lz in 0..32 {
            for lx in 0..32 {
                let (x, z) = (cx * 32 + lx, cz * 32 + lz);
                for y in SEA_LEVEL + 1..MAX_CHUNK_Y * 32 - 1 {
                    if block(column, lx as usize, y, lz as usize) != liquid {
                        continue;
                    }
                    tops.insert((x, z), y);
                    assert!(
                        !at(x, y - 1, z).unwrap().is_air(),
                        "{what} au-dessus du vide en ({x}, {y}, {z})"
                    );
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        if let Some(b) = at(x + dx, y, z + dz) {
                            assert!(!b.is_air(), "{what} ouvert sur le côté en ({x}, {y}, {z})");
                        }
                    }
                }
            }
        }
    }
    tops
}

#[test]
fn volcano_craters_hold_a_flat_lava_lake_and_spring_pools_hold_their_water() {
    let g = v3(42);
    let registry = g.registry().clone();
    let (water, lava) = (
        registry.id("core:water").unwrap(),
        registry.id("core:lava").unwrap(),
    );
    // Le cratère d'un volcan : les colonnes de chunks qu'il touche.
    let sites = g.volcano_sites((-4000, -4000), (4000, 4000));
    assert!(sites.len() >= 2, "trop peu de volcans : {sites:?}");
    // Chaque cratère tient un lac plat ; sa taille dépend du bord (un cône sur une pente a un bord bas
    // d'un côté), mais au moins l'un des premiers est bien rempli.
    let mut biggest = 0;
    for &(vx, vz) in sites.iter().take(3) {
        let mut chunks = std::collections::HashMap::new();
        for cz in (vz - 12).div_euclid(32)..=(vz + 12).div_euclid(32) {
            for cx in (vx - 12).div_euclid(32)..=(vx + 12).div_euclid(32) {
                chunks.insert((cx, cz), column_chunks(&g, cx, cz));
            }
        }
        let tops = assert_contained(&chunks, lava, "de la lave");
        let levels: std::collections::BTreeSet<i32> = tops.values().copied().collect();
        assert!(
            levels.len() <= 1,
            "un lac de lave qui n'est pas plat en ({vx}, {vz}) : {levels:?}"
        );
        biggest = biggest.max(tops.len());
        assert_eq!(
            g.biome(g.column(vx, vz).biome).key,
            "core:volcano",
            "le cône porte le biome du volcan"
        );
    }
    assert!(
        biggest > 30,
        "des cratères presque vides : {biggest} colonnes de lave au plus"
    );

    // Les bassins d'une source chaude.
    let springs = g.biomes().id("core:hot_springs").unwrap();
    let (sx, sz) = g
        .find_column(0, 0, 32, 40_000, |c| c.biome == springs)
        .expect("des sources chaudes");
    let mut chunks = std::collections::HashMap::new();
    for cz in sz.div_euclid(32) - 1..=sz.div_euclid(32) + 1 {
        for cx in sx.div_euclid(32) - 1..=sx.div_euclid(32) + 1 {
            chunks.insert((cx, cz), column_chunks(&g, cx, cz));
        }
    }
    let pools = assert_contained(&chunks, water, "l'eau d'un bassin");
    assert!(!pools.is_empty(), "aucun bassin autour de ({sx}, {sz})");
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

#[test]
fn underground_biomes_decorate_deep_caves_only() {
    let g = v3(42);
    let registry = g.registry().clone();
    let id = |k: &str| registry.id(k).unwrap();
    let cases = [
        (
            "core:crystal_cavern",
            vec![
                id("core:crystal_block"),
                id("core:crystal_cluster"),
                id("core:calcite"),
            ],
        ),
        (
            "core:glowing_grotto",
            vec![id("core:glow_moss"), id("core:glowcap")],
        ),
        (
            "core:fungal_cave",
            vec![id("core:mycelium"), id("core:mushroom_stem")],
        ),
    ];
    let all: Vec<BlockId> = cases.iter().flat_map(|(_, b)| b.clone()).collect();
    for (key, blocks) in &cases {
        let biome = g.biomes().id(key).unwrap();
        // Un point de ce biome, cherché sur une grille.
        let spot = (-30..30)
            .flat_map(|j| (-30..30).map(move |i| (i * 48, j * 48)))
            .flat_map(|(x, z)| [-100, -60, -20, 10].map(move |y| IVec3::new(x, y, z)))
            .find(|p| g.biome_at(p.x, p.y, p.z) == biome)
            .unwrap_or_else(|| panic!("{key} introuvable"));
        let chunk_pos = spot.div_euclid(IVec3::splat(32));
        // Sa grotte est décorée dans le chunk ou ses voisins (le point peut être dans la roche).
        let found = (-1..=1)
            .flat_map(|dy| (-1..=1).map(move |dx| chunk_pos + IVec3::new(dx, dy, 0)))
            .filter_map(|p| g.generate(p))
            .any(|chunk| {
                (0..32 * 32 * 32)
                    .any(|i| blocks.contains(&chunk.get(i % 32, i / 1024, i / 32 % 32)))
            });
        assert!(found, "{key} : aucune décoration autour de {spot}");
        // Le biome n'existe pas près de la surface.
        let column = g.column(spot.x, spot.z);
        assert_eq!(
            g.biome_at(spot.x, column.height - 3, spot.z),
            column.biome,
            "{key} : un biome souterrain à la surface"
        );
    }
    // Aucune décoration de grotte au-dessus de y = 30.
    for (cx, cz) in [(0, 0), (12, -7)] {
        for cy in 1..8 {
            if let Some(chunk) = g.generate(IVec3::new(cx, cy, cz)) {
                assert!(
                    !(0..32 * 32 * 32).any(|i| all.contains(&chunk.get(
                        i % 32,
                        i / 1024,
                        i / 32 % 32
                    ))),
                    "décoration de grotte dans le chunk ({cx}, {cy}, {cz})"
                );
            }
        }
    }
}

/// La part des grottes profondes (sous y = 30) dans chaque biome souterrain.
/// `cargo test -p engine_voxel --test generation_v3 print_v3_cave_biome_shares --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_v3_cave_biome_shares() {
    let g = v3(42);
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for j in -20..20 {
        for i in -20..20 {
            for y in (-120..30).step_by(10) {
                let b = g.biome_at(i * 40, y, j * 40);
                *counts.entry(g.biome(b).key.clone()).or_default() += 1;
            }
        }
    }
    let total: u64 = counts.values().sum();
    for (k, n) in counts
        .iter()
        .filter(|(k, _)| k.contains("cave") || k.contains("grotto"))
    {
        println!("{k:24} {:5.2} %", *n as f64 * 100.0 / total as f64);
    }
}

#[test]
fn structures_are_built_in_v3_where_their_sites_are() {
    let g = v3(42);
    assert!(g.has_structures());
    let sites = g.structure_sites_near(0, 0, 2500);
    assert!(!sites.is_empty(), "aucune structure près de l'origine");
    // Chaque site a son plan, et son centre est bâti : des blocs posés là où le terrain v3 n'en met pas.
    for site in sites.iter().take(3) {
        let plan = g.blueprint(site).expect("un plan pour chaque site");
        let blocks = plan.blocks();
        assert!(!blocks.is_empty(), "{:?} : plan vide", site.kind);
        let (bx, by, bz) = {
            let (p, _) = blocks[blocks.len() / 2];
            (p.x, p.y, p.z)
        };
        let chunk = g
            .generate(IVec3::new(bx, by, bz).div_euclid(IVec3::splat(32)))
            .expect("le chunk du plan existe");
        let found = blocks
            .iter()
            .filter(|(p, _)| {
                p.div_euclid(IVec3::splat(32))
                    == IVec3::new(bx, by, bz).div_euclid(IVec3::splat(32))
            })
            .filter(|(p, b)| {
                let l = p.rem_euclid(IVec3::splat(32));
                chunk.get(l.x as usize, l.y as usize, l.z as usize) == *b
            })
            .count();
        assert!(
            found > 0,
            "{:?} en ({}, {}) : rien de posé",
            site.kind,
            site.x,
            site.z
        );
    }
}

#[test]
fn cave_water_stays_well_below_the_ground_of_its_column() {
    // La nappe reste sous le sol le plus bas de sa région : jamais au ras d'une ouverture.
    let g = v3(42);
    let water = g.registry().id("core:water").unwrap();
    let mut closest = i32::MAX;
    for (cx, cz) in [
        (30, -20),
        (-12, 40),
        (5, 5),
        (70, -70),
        (-55, -9),
        (12, 33),
        (-80, 15),
    ] {
        let chunks = column_chunks(&g, cx, cz);
        for lz in 0..32 {
            for lx in 0..32 {
                let h = g.column(cx * 32 + lx as i32, cz * 32 + lz as i32).height;
                for y in MIN_CHUNK_Y * 32..h {
                    if block(&chunks, lx, y, lz) == water {
                        closest = closest.min(h - y);
                    }
                }
            }
        }
    }
    assert!(
        closest >= 10,
        "de l'eau de grotte à {closest} blocs sous le sol"
    );
}

#[test]
fn v3_dungeons_rest_on_rock_not_in_cave_voids() {
    use engine_voxel::StructureKind;
    let g = v3(42);
    let mut checked = 0;
    for site in g
        .structure_sites_near(0, 0, 4000)
        .into_iter()
        .filter(|s| matches!(s.kind, StructureKind::Dungeon | StructureKind::Graph))
        .take(4)
    {
        let plan = g.blueprint(&site).unwrap();
        let blocks = plan.blocks();
        let Some(bottom) = blocks.iter().map(|(p, _)| p.y).min() else {
            continue;
        };
        // Sous le plancher de la structure : de la roche, pas le vide d'une grotte.
        let mut chunks = std::collections::HashMap::new();
        let (mut under, mut open) = (0, 0);
        for (p, _) in blocks.iter().filter(|(p, _)| p.y == bottom).step_by(5) {
            let below = *p - IVec3::Y;
            let key = below.div_euclid(IVec3::splat(32));
            let chunk = chunks.entry(key).or_insert_with(|| g.generate(key));
            let l = below.rem_euclid(IVec3::splat(32));
            let b = chunk.as_ref().map_or(BlockId::AIR, |c| {
                c.get(l.x as usize, l.y as usize, l.z as usize)
            });
            under += 1;
            open += usize::from(b.is_air() || b == g.registry().id("core:water").unwrap());
        }
        assert!(
            open * 10 <= under,
            "{:?} en ({}, {}) : {open} blocs vides sous {under} du plancher",
            site.kind,
            site.x,
            site.z
        );
        checked += 1;
    }
    assert!(checked > 0, "aucun donjon près de l'origine");
}

#[test]
fn liquids_never_stand_against_air_on_their_sides() {
    // Ni colonne d'eau de mer dans un puits ou une grotte ouverte, ni nappe sans barrage : de l'eau
    // (ou de la lave) à côté de l'air coulerait. La graine et le lieu d'une partie où on l'a vu.
    for (seed, chunks) in [
        (
            1404532460u32,
            vec![(-22, -42), (-21, -41), (-23, -41), (-20, -42)],
        ),
        (42, vec![(30, -20), (-12, 40), (5, 5)]),
    ] {
        let g = v3(seed);
        let registry = g.registry().clone();
        let liquids = [
            registry.id("core:water").unwrap(),
            registry.id("core:lava").unwrap(),
        ];
        let mut bad = Vec::new();
        for (cx, cz) in chunks {
            let column = column_chunks(&g, cx, cz);
            for lz in 1..31 {
                for lx in 1..31 {
                    for y in MIN_CHUNK_Y * 32 + 1..MAX_CHUNK_Y * 32 {
                        if !liquids.contains(&block(&column, lx, y, lz)) {
                            continue;
                        }
                        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                            let (nx, nz) = ((lx as i32 + dx) as usize, (lz as i32 + dz) as usize);
                            if block(&column, nx, y, nz).is_air() {
                                bad.push((cx * 32 + lx as i32, y, cz * 32 + lz as i32));
                            }
                        }
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "graine {seed} : {} liquides ouverts sur le côté, dont {:?}",
            bad.len(),
            &bad[..bad.len().min(5)]
        );
    }
}

#[test]
fn v3_ores_reach_the_deep_world_and_new_ores_stay_out_of_v1() {
    let g = v3(42);
    let registry = g.registry().clone();
    let id = |k: &str| registry.id(k).unwrap();
    let count = |g: &TerrainGenerator, block: BlockId, cy: std::ops::Range<i32>| {
        let mut n = 0;
        for (cx, cz) in [(0, 0), (3, -2), (-4, 5), (7, 7), (-6, -3), (10, -9)] {
            for y in cy.clone() {
                if let Some(chunk) = g.generate(IVec3::new(cx, y, cz)) {
                    n += (0..32 * 32 * 32)
                        .filter(|&i| chunk.get(i % 32, i / 1024, i / 32 % 32) == block)
                        .count();
                }
            }
        }
        n
    };
    // Sous 0 (pierre profonde) : les minerais profonds, anciens et nouveaux.
    for key in [
        "core:iron_ore",
        "core:gold_ore",
        "core:sapphire_ore",
        "core:diamond_ore",
        "core:silver_ore",
        "core:nickel_ore",
        "core:lead_ore",
        "core:quartz_ore",
    ] {
        assert!(count(&g, id(key), -4..0) > 0, "{key} absent sous 0 en v3");
    }
    assert!(
        count(&g, id("core:tin_ore"), 0..4) > 0,
        "étain absent en v3"
    );
    // Le diamant reste tout au fond.
    assert_eq!(
        count(&g, id("core:diamond_ore"), 0..8),
        0,
        "du diamant au-dessus de 0"
    );
    // Les mondes v1 n'ont aucun des nouveaux minerais.
    let v1 = TerrainGenerator::new(42);
    for key in [
        "core:tin_ore",
        "core:lead_ore",
        "core:silver_ore",
        "core:nickel_ore",
        "core:quartz_ore",
        "core:diamond_ore",
    ] {
        assert_eq!(count(&v1, id(key), 0..4), 0, "{key} dans un monde v1");
    }
}
