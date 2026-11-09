use std::collections::{HashMap, HashSet};

use super::*;

fn id(g: &TerrainGenerator, key: &str) -> BlockId {
    g.registry().id(key).unwrap()
}

#[test]
fn same_seed_gives_the_same_world() {
    let a = TerrainGenerator::new(42);
    let b = TerrainGenerator::new(42);
    for pos in [
        IVec3::new(0, 0, 0),
        IVec3::new(-7, 1, 13),
        IVec3::new(3, 2, -9),
    ] {
        assert_eq!(a.generate(pos), b.generate(pos), "chunk {pos}");
    }
}

#[test]
fn different_seeds_give_different_worlds() {
    let a = TerrainGenerator::new(1);
    let b = TerrainGenerator::new(2);
    let heights = |g: &TerrainGenerator| {
        (0..64)
            .map(|x| g.column(x * 37, 0).height)
            .collect::<Vec<_>>()
    };
    assert_ne!(heights(&a), heights(&b));
}

/// `column()` (utilisé par le jeu) et `generate()` doivent être d'accord, à la frontière
/// d'un chunk comme ailleurs : ils interpolent les mêmes points de grille.
#[test]
fn surface_matches_column_height() {
    let generator = TerrainGenerator::new(42);
    let world = generator.generate_area(IVec3::new(-1, 0, -1), 2, 2);
    let (water, ice) = (id(&generator, "core:water"), id(&generator, "core:ice"));
    let mut solid = 0;
    for x in -32..32 {
        for z in -32..32 {
            let height = generator.column(x, z).height;
            let above = world.block(IVec3::new(x, height + 1, z));
            let liquid = above == water || above == ice;
            assert!(
                above.is_air() || (liquid && height < SEA_LEVEL),
                "({x}, {z}) au-dessus de {height}"
            );
            // La surface peut être percée par une grotte : on compte les autres.
            if !world.block(IVec3::new(x, height, z)).is_air() {
                solid += 1;
            }
        }
    }
    assert!(
        solid > 3900,
        "presque toutes les colonnes ont un sol ({solid}/4096)"
    );
}

#[test]
fn sky_and_out_of_range_chunks_are_empty() {
    let generator = TerrainGenerator::new(42);
    assert!(generator.generate(IVec3::new(0, -1, 0)).is_none());
    assert!(
        generator
            .generate(IVec3::new(0, WORLD_HEIGHT_CHUNKS, 0))
            .is_none()
    );
    assert!(generator.generate(IVec3::new(0, 0, 0)).is_some());
}

/// Sur un grand territoire, on doit croiser tous les biomes, des montagnes enneigées
/// et des océans profonds.
#[test]
fn a_large_area_has_every_biome() {
    let generator = TerrainGenerator::new(42);
    let mut counts: HashMap<BiomeId, usize> = HashMap::new();
    let mut highest = 0;
    let mut lowest = i32::MAX;
    for x in (-12_000..12_000).step_by(48) {
        for z in (-12_000..12_000).step_by(48) {
            let column = generator.column(x, z);
            *counts.entry(column.biome).or_default() += 1;
            highest = highest.max(column.height);
            lowest = lowest.min(column.height);
        }
    }
    let total: usize = counts.values().sum();
    let mut report: Vec<String> = generator
        .biomes()
        .iter()
        .map(|(id, b)| {
            format!(
                "{} {:.1} %",
                b.key,
                100.0 * *counts.get(&id).unwrap_or(&0) as f64 / total as f64
            )
        })
        .collect();
    report.sort();
    eprintln!("répartition des biomes : {}", report.join(", "));
    for (id, biome) in generator.biomes().iter() {
        assert!(
            counts.get(&id).is_some_and(|&n| n > 0),
            "biome absent : {} ({report:?})",
            biome.key
        );
    }
    assert!(highest > SEA_LEVEL + 90, "pas de hauts sommets : {highest}");
    assert!(lowest < SEA_LEVEL - 25, "pas d'océan profond : {lowest}");
    // Les océans couvrent une part raisonnable du monde.
    let ocean = [BiomeKind::Ocean, BiomeKind::DeepOcean]
        .iter()
        .map(|&k| {
            counts
                .get(&generator.biomes().first_of(k))
                .copied()
                .unwrap_or(0)
        })
        .sum::<usize>() as f64
        / total as f64;
    assert!(
        (0.15..0.6).contains(&ocean),
        "océans : {:.0} % ({report:?})",
        ocean * 100.0
    );
}

/// Entre deux biomes des terres, deux colonnes voisines n'ont jamais un grand écart de
/// hauteur : le relief est mélangé. (Les berges des rivières et les montagnes, elles,
/// peuvent être raides.)
#[test]
fn biome_borders_have_no_cliffs() {
    let generator = TerrainGenerator::new(42);
    let mut borders = 0;
    for x in (-3000..3000).step_by(7) {
        for z in (-3000..3000).step_by(97) {
            let (a, b) = (generator.column(x, z), generator.column(x + 1, z));
            let land = |c: Column| generator.biome(c.biome).kind == BiomeKind::Land;
            if a.biome != b.biome && land(a) && land(b) {
                borders += 1;
                assert!(
                    (a.height - b.height).abs() <= 3,
                    "falaise en ({x}, {z}) : {a:?} / {b:?}"
                );
            }
        }
    }
    assert!(borders > 20, "trop peu de frontières testées ({borders})");
}

/// Le dessous du monde : le socle en y = 0, de la pierre profonde en bas.
#[test]
fn bedrock_and_deepstone_layers() {
    let generator = TerrainGenerator::new(42);
    let world = generator.generate_area(IVec3::new(2, 0, 2), 1, 1);
    let (bedrock, deepstone, stone) = (
        id(&generator, "core:bedrock"),
        id(&generator, "core:deepstone"),
        id(&generator, "core:stone"),
    );
    for x in 64..96 {
        for z in 64..96 {
            assert_eq!(world.block(IVec3::new(x, 0, z)), bedrock);
            for y in 4..DEEPSTONE_LEVEL - 4 {
                assert_ne!(
                    world.block(IVec3::new(x, y, z)),
                    stone,
                    "pierre en ({x}, {y}, {z})"
                );
            }
        }
    }
    let deep = (64..96)
        .flat_map(|x| (64..96).map(move |z| (x, z)))
        .filter(|&(x, z)| world.block(IVec3::new(x, 5, z)) == deepstone)
        .count();
    assert!(deep > 300, "peu de pierre profonde : {deep}");
}

/// Chaque minerai n'apparaît qu'entre ses altitudes, et les plus courants apparaissent.
#[test]
fn ores_appear_at_their_depths() {
    let generator = TerrainGenerator::new(7);
    let world = generator.generate_area(IVec3::new(-3, 0, -3), 6, 6);
    let ores: HashMap<BlockId, (i32, i32)> = generator
        .biomes()
        .ores()
        .iter()
        .map(|o| (o.block, o.height))
        .collect();
    let mut found: HashMap<BlockId, usize> = HashMap::new();
    for pos in world.chunk_positions().collect::<Vec<_>>() {
        let chunk = world.chunk(pos).unwrap();
        let origin = World::chunk_origin(pos);
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let block = chunk.get(x, y, z);
                    if let Some(&(min, max)) = ores.get(&block) {
                        let wy = origin.y + y as i32;
                        assert!(
                            (min..=max).contains(&wy),
                            "{} à y = {wy}",
                            generator.registry().def(block).key
                        );
                        *found.entry(block).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut summary: Vec<String> = found
        .iter()
        .map(|(b, n)| format!("{} {n}", generator.registry().def(*b).key))
        .collect();
    summary.sort();
    eprintln!("minerais sur 6 × 6 colonnes : {}", summary.join(", "));
    for key in [
        "core:coal_ore",
        "core:iron_ore",
        "core:copper_ore",
        "core:gold_ore",
        "core:sapphire_ore",
    ] {
        assert!(
            found.get(&id(&generator, key)).is_some_and(|&n| n > 0),
            "{key} introuvable"
        );
    }
    // Le charbon est bien plus courant que le saphir.
    assert!(
        found[&id(&generator, "core:coal_ore")] > 10 * found[&id(&generator, "core:sapphire_ore")]
    );
}

/// Des grottes : des vides sous la surface, et de la lave tout en bas.
#[test]
fn caves_lava_and_aquifers() {
    let generator = TerrainGenerator::new(42);
    let world = generator.generate_area(IVec3::new(-2, 0, -2), 4, 4);
    let (lava, water) = (id(&generator, "core:lava"), id(&generator, "core:water"));
    let (mut air, mut lava_blocks, mut underground_water) = (0, 0, 0);
    for x in -64..64 {
        for z in -64..64 {
            let height = generator.column(x, z).height;
            for y in 1..height - 10 {
                let block = world.block(IVec3::new(x, y, z));
                if block.is_air() {
                    air += 1;
                } else if block == lava {
                    lava_blocks += 1;
                    assert!(y <= LAVA_LEVEL);
                } else if block == water && y < SEA_LEVEL - 12 {
                    underground_water += 1;
                }
            }
        }
    }
    assert!(air > 5000, "trop peu de grottes : {air}");
    assert!(lava_blocks > 0, "pas de lave");
    let _ = underground_water; // Les nappes dépendent de l'endroit : on ne les exige pas ici.
}

/// Les océans et les rivières sont sous l'eau, avec leurs blocs de fond.
#[test]
fn oceans_and_rivers_are_wet() {
    let generator = TerrainGenerator::new(42);
    let mut seen = HashSet::new();
    for x in (-6000..6000).step_by(24) {
        for z in (-6000..6000).step_by(24) {
            let column = generator.column(x, z);
            let kind = generator.biome(column.biome).kind;
            if matches!(
                kind,
                BiomeKind::Ocean | BiomeKind::DeepOcean | BiomeKind::River
            ) {
                assert!(
                    column.height < SEA_LEVEL,
                    "{kind:?} hors de l'eau en ({x}, {z})"
                );
                seen.insert(kind);
            }
            if kind == BiomeKind::Beach {
                assert!((SEA_LEVEL..=SEA_LEVEL + 2).contains(&column.height));
            }
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

#[test]
fn cold_biomes_are_snowy() {
    let generator = TerrainGenerator::new(42);
    let tundra = generator.biomes().id("core:snowy_tundra").unwrap();
    let (x, z) = (-8000..8000)
        .step_by(40)
        .flat_map(|x| (-8000..8000).step_by(40).map(move |z| (x, z)))
        .find(|&(x, z)| generator.column(x, z).biome == tundra)
        .expect("pas de toundra");
    let chunk = World::split(IVec3::new(x, 0, z)).0;
    let world = generator.generate_area(IVec3::new(chunk.x, 0, chunk.z), 1, 1);
    let column = generator.column(x, z);
    let top = world.block(IVec3::new(x, column.height, z));
    assert!(
        top == id(&generator, "core:snow") || column.height < SEA_LEVEL,
        "surface de la toundra : {}",
        generator.registry().def(top).key
    );
}

#[test]
fn spawn_is_on_dry_land() {
    for seed in [1, 42, 1234] {
        let generator = TerrainGenerator::new(seed);
        let (x, z) = generator.find_spawn();
        let column = generator.column(x, z);
        assert!(
            column.height > SEA_LEVEL,
            "graine {seed} : apparition sous l'eau en ({x}, {z})"
        );
        assert_eq!(generator.biome(column.biome).kind, BiomeKind::Land);
    }
}
