//! Benchmark de la génération : `cargo bench -p engine_voxel --bench generation`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{LightRegion, TerrainGenerator};
use glam::{IVec2, IVec3};

fn bench_generation(c: &mut Criterion) {
    let generator = TerrainGenerator::new(42);
    let mut group = c.benchmark_group("generation");
    // Trois cas typiques : sous terre (plein, avec grottes), à la surface, dans le ciel.
    let cases = [
        ("souterrain", IVec3::new(3, 0, 5)),
        ("surface", IVec3::new(3, 1, 5)),
        ("ciel", IVec3::new(3, 4, 5)),
    ];
    for (name, pos) in cases {
        group.bench_function(name, |b| b.iter(|| generator.generate(black_box(pos))));
    }
    group.finish();

    // La lumière d'une colonne entière (5 chunks), avec ses 8 voisines pour la marge.
    let world = generator.generate_region(3, 3);
    let region = LightRegion::from_world(&world, IVec2::new(1, 1));
    c.bench_function("lumière d'une colonne (5 chunks)", |b| {
        b.iter(|| black_box(&region).compute())
    });
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
