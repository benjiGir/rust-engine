//! La référence de la génération historique (v1) : des empreintes de blocs figées dans le dépôt.
//!
//! Tant que la génération v1 n'est pas volontairement changée, **rien** ne doit modifier ces
//! empreintes : ni un cache, ni l'ordre des tâches, ni une optimisation. Ne jamais les mettre à
//! jour pour faire passer un test ; si un changement de monde est voulu, il appartient à une
//! nouvelle version de génération (voir `docs/plan-generation-rendu.md`).
//!
//! Pour régénérer la table (seulement si v1 change réellement, ce qui est interdit pour les
//! mondes existants) : `cargo test -p engine_voxel --test generation_reference
//! print_reference -- --ignored --nocapture`.

mod support {
    pub mod fingerprint;
    pub mod reference_v1;
}

use engine_voxel::TerrainGenerator;
use support::fingerprint::*;
use support::reference_v1::{COLUMN_MAPS, COLUMNS};

/// Les empreintes de toutes les colonnes suivies d'une graine, comme figées dans la table.
fn current_columns(generator: &TerrainGenerator, keys: &Keys, seed: u32) -> Vec<[u64; 6]> {
    columns_of(seed)
        .iter()
        .map(|&c| column_hashes(generator, keys, c))
        .collect()
}

#[test]
#[ignore = "affiche la table à figer"]
fn print_reference() {
    println!("pub const COLUMNS: &[(u32, i32, i32, [u64; 6])] = &[");
    for seed in seeds() {
        let generator = TerrainGenerator::new(seed);
        let keys = Keys::of(&generator);
        for (&(cx, cz), hashes) in columns_of(seed)
            .iter()
            .zip(current_columns(&generator, &keys, seed))
        {
            println!("    ({seed}, {cx}, {cz}, {hashes:?}),");
        }
    }
    println!("];");
    println!("pub const COLUMN_MAPS: &[(u32, u64)] = &[");
    for seed in seeds() {
        let generator = TerrainGenerator::new(seed);
        println!(
            "    ({seed}, {}),",
            columns_hash(&generator, &Keys::of(&generator))
        );
    }
    println!("];");
}

#[test]
fn v1_blocks_match_the_frozen_fingerprints() {
    let mut checked = 0;
    for seed in seeds() {
        let generator = TerrainGenerator::new(seed);
        let keys = Keys::of(&generator);
        for &(s, cx, cz, expected) in COLUMNS.iter().filter(|c| c.0 == seed) {
            let got = column_hashes(&generator, &keys, (cx, cz));
            assert_eq!(
                got, expected,
                "graine {s}, colonne ({cx}, {cz}) : la génération v1 a changé"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        COLUMNS.len(),
        "toutes les entrées de la table ont servi"
    );
}

#[test]
fn v1_column_maps_match_the_frozen_fingerprints() {
    for &(seed, expected) in COLUMN_MAPS {
        let generator = TerrainGenerator::new(seed);
        let got = columns_hash(&generator, &Keys::of(&generator));
        assert_eq!(got, expected, "graine {seed} : hauteurs ou biomes changés");
    }
}

#[test]
fn the_corpus_covers_surface_underground_negatives_and_region_edges() {
    // Garde-fou sur la table elle-même : elle ne doit pas se réduire sans qu'on le voie.
    assert_eq!(COLUMNS.len(), 2 * 8 + 5 * 4);
    assert!(COLUMNS.iter().any(|c| c.1 < 0 && c.2 < 0), "négatifs");
    assert!(
        COLUMNS.iter().any(|c| c.1 == 15 || c.2 == 16),
        "bords de région"
    );
    for &(seed, cx, cz, hashes) in COLUMNS {
        // Le fond (bedrock) existe toujours ; le haut du monde est souvent vide.
        assert_ne!(hashes[0], 0, "graine {seed}, ({cx}, {cz}) : le fond manque");
        assert_ne!(
            hashes[1], 0,
            "graine {seed}, ({cx}, {cz}) : sous-sol et mer"
        );
    }
    assert!(
        COLUMNS.iter().any(|c| c.3[2] != 0),
        "au moins une colonne atteint la surface des reliefs (y >= 64)"
    );
    assert!(
        COLUMNS.iter().any(|c| c.3[4] != 0),
        "au moins une colonne de haute montagne"
    );
}

// ----- Lot 1 : les caches ne changent aucun bloc -----

use std::sync::Arc;

use glam::IVec3;

/// Compare toutes les colonnes de la table pour `seed`, en les parcourant dans l'ordre donné par
/// `order` (une permutation des indices de `columns_of(seed)`), du haut vers le bas si `descending`.
fn assert_matches_table(
    generator: &TerrainGenerator,
    keys: &Keys,
    seed: u32,
    order: &[usize],
    descending: bool,
    context: &str,
) {
    let columns = columns_of(seed);
    for &i in order {
        let (cx, cz) = columns[i];
        let expected = COLUMNS
            .iter()
            .find(|c| (c.0, c.1, c.2) == (seed, cx, cz))
            .expect("colonne de la table")
            .3;
        let mut got = [0; 6];
        let ys: Vec<usize> = if descending {
            (0..6).rev().collect()
        } else {
            (0..6).collect()
        };
        for y in ys {
            got[y] = chunk_hash(generator, keys, IVec3::new(cx, y as i32, cz));
        }
        assert_eq!(
            got, expected,
            "{context} : graine {seed}, colonne ({cx}, {cz})"
        );
    }
}

#[test]
fn reverse_order_gives_the_same_blocks() {
    for seed in MAIN_SEEDS {
        let generator = TerrainGenerator::new(seed);
        let keys = Keys::of(&generator);
        let order: Vec<usize> = (0..columns_of(seed).len()).rev().collect();
        assert_matches_table(&generator, &keys, seed, &order, true, "ordre inverse");
    }
}

#[test]
fn parallel_generation_in_different_orders_gives_the_same_blocks() {
    for seed in MAIN_SEEDS {
        let generator = Arc::new(TerrainGenerator::new(seed));
        let keys = Keys::of(&generator);
        let count = columns_of(seed).len();
        // Chaque thread parcourt toutes les colonnes, dans son propre ordre : ils se disputent les
        // mêmes colonnes du cache (la première arrivée la calcule, les autres la lisent).
        std::thread::scope(|scope| {
            for thread in 0..4 {
                let (generator, keys) = (&generator, &keys);
                scope.spawn(move || {
                    let order: Vec<usize> =
                        (0..count).map(|i| (i * 3 + thread * 5) % count).collect();
                    let mut sorted = order.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    assert_eq!(sorted.len(), count, "une permutation");
                    assert_matches_table(
                        generator,
                        keys,
                        seed,
                        &order,
                        thread % 2 == 1,
                        "parallèle",
                    );
                });
            }
        });
    }
}

#[test]
fn empty_full_cleared_and_evicted_caches_give_the_same_blocks() {
    let seed = 42;
    let generator = TerrainGenerator::new(seed);
    let keys = Keys::of(&generator);
    let order: Vec<usize> = (0..columns_of(seed).len()).collect();
    assert_eq!(generator.cached_columns(), 0);
    assert_matches_table(&generator, &keys, seed, &order, false, "cache vide");
    assert!(generator.cached_columns() > 0, "le cache a servi");
    assert_matches_table(&generator, &keys, seed, &order, false, "cache plein");

    generator.clear_caches();
    assert_eq!(generator.cached_columns(), 0);
    assert_matches_table(&generator, &keys, seed, &order, true, "cache vidé");

    // Éviction : on remplit bien au-delà de la borne avec d'autres colonnes.
    for cx in 0..TerrainGenerator::COLUMN_CACHE_LIMIT as i32 + 200 {
        generator.biome_columns(IVec3::new(1000 + cx, 0, 1000));
    }
    assert!(generator.cached_columns() <= TerrainGenerator::COLUMN_CACHE_LIMIT);
    assert_matches_table(&generator, &keys, seed, &order, false, "cache évincé");
}

#[test]
fn clones_and_other_seeds_never_mix_their_columns() {
    let (a, b) = (TerrainGenerator::new(7), TerrainGenerator::new(42));
    let clone = a.clone();
    let pos = IVec3::new(2, 0, -3);
    // `a` et son clone partagent leur cache (même graine, même configuration) ; `b` a le sien.
    let (ha, hb) = (a.biome_columns(pos), b.biome_columns(pos));
    assert_eq!(clone.biome_columns(pos), ha);
    assert_ne!(ha, hb, "deux graines, deux cartes de biomes");
    assert_eq!(a.columns(pos)[5][9], clone.column(2 * 32 + 5, -3 * 32 + 9));
    assert_eq!(b.columns(pos)[5][9], b.column(2 * 32 + 5, -3 * 32 + 9));
}

#[test]
fn column_snapshots_equal_a_direct_evaluation() {
    // Chaque colonne du cache est celle que donne `column(x, z)` : mêmes hauteur et biome, aux
    // bords d'un chunk comme au milieu, en coordonnées négatives comme positives.
    let generator = TerrainGenerator::new(7);
    for (cx, cz) in [(0, 0), (-1, -1), (-5, 3)] {
        let grid = generator.columns(IVec3::new(cx, 3, cz));
        for (lx, lz) in [(0, 0), (31, 0), (0, 31), (31, 31), (13, 21)] {
            let (x, z) = (cx * 32 + lx as i32, cz * 32 + lz as i32);
            assert_eq!(grid[lx][lz], generator.column(x, z), "({x}, {z})");
        }
    }
}
