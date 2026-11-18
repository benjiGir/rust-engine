//! Benchmark de la géographie et de l'hydrologie macro (générateur v2, prototype) :
//! `cargo bench -p engine_voxel --bench hydrology`.
//!
//! Tout est mesuré **à froid** : un contexte neuf (caches vides) avant chaque mesure, comme la
//! première requête d'une zone neuve. Le coût à chaud (caches pleins) est celui d'une recherche
//! en table et n'a pas d'intérêt ici.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use engine_voxel::{MacroWorld, SEA_LEVEL};

const SEED: u32 = 42;

/// Le premier point terrestre d'une grille qui satisfait `accept` (déterministe).
fn find(accept: impl Fn(&engine_voxel::GeoSample) -> bool) -> (f64, f64) {
    let world = MacroWorld::new(SEED);
    for i in 0..4000 {
        let (x, z) = (
            ((i * 37) % 200 - 100) as f64 * 160.0,
            ((i * 91) % 200 - 100) as f64 * 160.0,
        );
        let s = world.geography().sample(x, z);
        if s.macro_elevation >= SEA_LEVEL as f32 && accept(&s) {
            return (x, z);
        }
    }
    panic!("aucun point trouvé");
}

fn bench_hydrology(c: &mut Criterion) {
    let mut group = c.benchmark_group("hydrologie");
    let cases = [
        (
            "plaine côtière",
            find(|s| (200.0..500.0).contains(&s.coast_distance) && s.chain_height < 5.0),
        ),
        ("massif", find(|s| s.chain_height > 40.0)),
        ("intérieur", find(|s| s.coast_distance > 1500.0)),
    ];
    for (name, (x, z)) in cases {
        group.bench_function(format!("requête de point à froid, {name}"), |b| {
            b.iter_batched(
                || MacroWorld::new(SEED),
                |world| black_box(world.sample(black_box(x), black_box(z))),
                BatchSize::SmallInput,
            )
        });
        group.bench_function(format!("géographie seule, {name}"), |b| {
            let world = MacroWorld::new(SEED);
            b.iter(|| black_box(world.geography().sample(black_box(x), black_box(z))))
        });
    }
    let (x, z) = cases[1].1;
    group.bench_function("résolution d'un bassin à froid (massif)", |b| {
        b.iter_batched(
            || MacroWorld::new(SEED),
            |world| {
                let cell = engine_voxel::fine_cell(x, z);
                black_box(world.hydrology().chain(engine_voxel::basin_of(cell)))
            },
            BatchSize::SmallInput,
        )
    });
    // Une zone de 1 024 × 1 024 blocs au pas de 16 : 4 096 points, un contexte neuf.
    group.sample_size(10);
    group.bench_function("zone 1 024 × 1 024 blocs à froid (4 096 points)", |b| {
        b.iter_batched(
            || MacroWorld::new(SEED),
            |world| {
                for j in 0..64 {
                    for i in 0..64 {
                        black_box(world.sample(i as f64 * 16.0 + x, j as f64 * 16.0 + z));
                    }
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench_hydrology);
criterion_main!(benches);
