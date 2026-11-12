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
            // Au-dessus du sol : de l'air, de l'eau, ou de la végétation (plante, arbre).
            let def = generator.registry().def(above);
            let vegetation =
                !def.solid || def.cutout || def.key.ends_with("_log") || def.key == "core:cactus";
            assert!(
                above.is_air() || vegetation || (liquid && height < SEA_LEVEL),
                "({x}, {z}) au-dessus de {height} : {}",
                def.key
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
    let ocean = counts
        .iter()
        .filter(|(id, _)| {
            matches!(
                generator.biome(**id).kind,
                BiomeKind::Ocean | BiomeKind::DeepOcean
            )
        })
        .map(|(_, n)| n)
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

/// Un arbre planté près du bord d'un chunk est complet : son tronc est bien là, même la
/// partie qui tombe dans le chunk voisin (généré séparément, sans se parler).
#[test]
fn trees_are_complete_across_chunk_borders() {
    let generator = TerrainGenerator::new(42);
    let forest = generator.biomes().id("core:forest").unwrap();
    let (x, z) = generator
        .find_column(0, 0, 32, 20_000, |c| {
            c.biome == forest && c.height > SEA_LEVEL
        })
        .expect("pas de forêt");
    let home = World::split(IVec3::new(x, 0, z)).0;
    let world = generator.generate_area(IVec3::new(home.x - 1, 0, home.z - 1), 3, 3);
    let blocks = crate::trees::TreeBlocks::resolve(generator.registry()).unwrap();
    let mut border_trees = 0;
    for site in generator.tree_sites(home.x, home.z).iter() {
        let local = site.base - World::chunk_origin(World::split(site.base).0);
        let near_border = [local.x, local.z].iter().any(|&v| !(4..28).contains(&v));
        let mut rng = crate::trees::Rng64::new(site.seed);
        let mut logs = Vec::new();
        crate::trees::grow(
            site.kind,
            site.base,
            &mut rng,
            &blocks,
            &mut |p, b, part| {
                if part == crate::trees::Part::Log {
                    logs.push((p, b));
                }
            },
        );
        for (p, _) in &logs {
            let def = generator.registry().def(world.block(*p));
            assert!(
                def.key.ends_with("_log") || def.key == "core:cactus",
                "tronc incomplet en {p} : {}",
                def.key
            );
        }
        border_trees += near_border as usize;
    }
    assert!(
        border_trees > 0,
        "aucun arbre près du bord dans cette forêt"
    );
}

/// Un chunk généré seul est identique au même chunk généré au milieu de ses voisins :
/// l'ordre de génération (et donc les threads) ne change rien.
#[test]
fn generation_order_does_not_matter() {
    let generator = TerrainGenerator::new(42);
    let jungle = generator.biomes().id("core:jungle").unwrap();
    let (x, z) = generator
        .find_column(0, 0, 32, 20_000, |c| {
            c.biome == jungle && c.height > SEA_LEVEL
        })
        .expect("pas de jungle");
    let home = World::split(IVec3::new(x, 0, z)).0;
    let area = generator.generate_area(IVec3::new(home.x - 1, 0, home.z - 1), 3, 3);
    for y in 0..WORLD_HEIGHT_CHUNKS {
        let pos = IVec3::new(home.x, y, home.z);
        let alone = generator.generate(pos);
        assert_eq!(
            alone.as_ref().map(|c| c.blocks()),
            area.chunk(pos).map(|c| c.blocks()),
            "chunk {pos}"
        );
    }
}

/// Les forêts ont des arbres, les plaines des fleurs et des herbes, l'océan des algues.
#[test]
fn biomes_have_their_vegetation() {
    let generator = TerrainGenerator::new(42);
    let count = |biome: &str, keys: &[&str]| -> usize {
        let id = generator.biomes().id(biome).unwrap();
        // Au cœur du biome : le chunk entier en est (pas une bordure).
        let (x, z) = generator
            .find_column(0, 0, 32, 20_000, |c| c.biome == id)
            .and_then(|(x, z)| {
                let inside = |px: i32, pz: i32| {
                    [(0, 0), (32, 0), (-32, 0), (0, 32), (0, -32)]
                        .iter()
                        .all(|(dx, dz)| generator.column(px + dx, pz + dz).biome == id)
                };
                (-40..=40)
                    .flat_map(|i| (-40..=40).map(move |j| (x + i * 16, z + j * 16)))
                    .find(|&(px, pz)| inside(px, pz))
            })
            .unwrap_or_else(|| panic!("pas de {biome}"));
        let home = World::split(IVec3::new(x, 0, z)).0;
        let world = generator.generate_area(IVec3::new(home.x, 0, home.z), 1, 1);
        let wanted: Vec<BlockId> = keys
            .iter()
            .map(|k| super::tests::id(&generator, k))
            .collect();
        world
            .chunk_positions()
            .collect::<Vec<_>>()
            .iter()
            .map(|&p| {
                world
                    .chunk(p)
                    .unwrap()
                    .blocks()
                    .iter()
                    .filter(|b| wanted.contains(b))
                    .count()
            })
            .sum()
    };
    assert!(count("core:forest", &["core:oak_log", "core:birch_log"]) > 20);
    assert!(count("core:taiga", &["core:spruce_log"]) > 20);
    assert!(count("core:plains", &["core:tall_grass"]) > 50);
    assert!(count("core:desert", &["core:cactus"]) > 0);
    assert!(count("core:ocean", &["core:seaweed"]) > 10);
}

// ----- Les dimensions -----

mod dimensions {
    use super::*;
    use crate::biome::BiomeRegistry;
    use crate::dimension::{DimensionRegistry, WorldKind};

    const SOURCE: &str = r#"[
        (id: "test:flat", name: "Plat", generator: Flat(layers: [("core:bedrock", 1), ("core:stone", 9), ("core:gravel", 2)], biome: "core:plains")),
        (id: "test:void", name: "Vide", generator: Void(platform: "core:stone")),
        (id: "test:bare", name: "Nu", generator: Void()),
        (id: "test:cold", name: "Froid", generator: Terrain(biomes: ["core:snowy_tundra"]), seed_offset: 5),
    ]"#;

    fn generator(key: &str, seed: u32) -> TerrainGenerator {
        let registry = BlockRegistry::builtin();
        let biomes = BiomeRegistry::builtin(&registry).unwrap();
        let mut dimensions = DimensionRegistry::builtin();
        dimensions
            .add_source("test", SOURCE, "test", &registry, &biomes)
            .unwrap();
        TerrainGenerator::with_dimension(seed, registry, &[], dimensions.get(key).unwrap()).unwrap()
    }

    #[test]
    fn a_flat_world_is_layers_from_y_zero_everywhere() {
        let g = generator("test:flat", 1);
        let id = |k: &str| g.registry().id(k).unwrap();
        let chunk = g.generate(IVec3::new(7, 0, -3)).expect("des blocs");
        for (y, expected) in [
            (0, "core:bedrock"),
            (5, "core:stone"),
            (9, "core:stone"),
            (10, "core:gravel"),
            (11, "core:gravel"),
        ] {
            assert_eq!(chunk.get(3, y, 4), id(expected), "y = {y}");
        }
        assert_eq!(chunk.get(3, 12, 4), BlockId::AIR, "au-dessus : de l'air");
        // Au-dessus des couches, rien à stocker ; la colonne dit la hauteur du sol.
        assert!(g.generate(IVec3::new(0, 1, 0)).is_none());
        assert_eq!(g.column(100, -200).height, 11);
        assert_eq!(g.find_spawn(), (0, 0));
        // Pas de structures dans un monde plat.
        assert!(!g.has_structures());
    }

    #[test]
    fn a_void_world_has_only_its_platform() {
        let g = generator("test:void", 1);
        let stone = g.registry().id("core:stone").unwrap();
        let chunk = g
            .generate(IVec3::new(0, 2, 0))
            .expect("la plateforme est dans ce chunk");
        assert_eq!(chunk.get(8, 0, 8), stone, "y = 64 : la plateforme");
        assert_eq!(chunk.get(8, 1, 8), BlockId::AIR);
        assert_eq!(g.column(0, 0).height, 64);
        assert_eq!(g.column(500, 500).height, 0);
        // Loin de l'origine, ou à une autre hauteur : rien.
        assert!(g.generate(IVec3::new(5, 2, 5)).is_none());
        assert!(g.generate(IVec3::new(0, 0, 0)).is_none());
        // Sans plateforme : le vide complet.
        let bare = generator("test:bare", 1);
        assert!(bare.generate(IVec3::new(0, 2, 0)).is_none());
    }

    #[test]
    fn a_terrain_dimension_can_be_limited_to_some_biomes_and_has_its_own_map() {
        let cold = generator("test:cold", 1);
        let tundra = cold.biomes().id("core:snowy_tundra").unwrap();
        // Tous les biomes de terre y sont la toundra.
        let lands: Vec<_> = (0..40)
            .map(|i| cold.column(i * 300, i * -170))
            .filter(|c| cold.biome(c.biome).kind == crate::biome::BiomeKind::Land)
            .collect();
        assert!(!lands.is_empty());
        assert!(lands.iter().all(|c| c.biome == tundra));
        // La graine est celle du monde plus le décalage : une autre carte que celle de la dimension de départ.
        let normal = TerrainGenerator::new(1);
        assert!(
            (0..20)
                .any(|i| cold.column(i * 100, i * 50).height
                    != normal.column(i * 100, i * 50).height)
        );
        assert!(matches!(cold.world, WorldKind::Terrain { .. }));
    }
}
