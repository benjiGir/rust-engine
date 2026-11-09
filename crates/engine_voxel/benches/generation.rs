//! Benchmark de la génération : `cargo bench -p engine_voxel --bench generation`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{LightRegion, TerrainGenerator};
use glam::{IVec2, IVec3};

fn bench_generation(c: &mut Criterion) {
    let generator = TerrainGenerator::new(42);
    let mut group = c.benchmark_group("generation");
    // Quatre cas typiques : le fond (socle, lave), sous terre (grottes, minerais), à la
    // surface (y = 64 à 95, autour du niveau de la mer), dans le ciel.
    let cases = [
        ("fond", IVec3::new(3, 0, 5)),
        ("souterrain", IVec3::new(3, 1, 5)),
        ("surface", IVec3::new(3, 2, 5)),
        ("ciel", IVec3::new(3, 5, 5)),
    ];
    for (name, pos) in cases {
        group.bench_function(name, |b| b.iter(|| generator.generate(black_box(pos))));
    }
    // La moyenne sur une grande zone : 8 × 8 colonnes de 6 chunks, divisée par 384 chunks.
    group.sample_size(10);
    group.bench_function("zone 8 × 8 colonnes (384 chunks)", |b| {
        b.iter(|| generator.generate_area(black_box(IVec3::new(-40, 0, 17)), 8, 8))
    });
    group.finish();

    // La lumière d'une colonne entière (6 chunks), avec ses 8 voisines pour la marge.
    let world = generator.generate_region(3, 3);
    let region = LightRegion::from_world(&world, IVec2::new(1, 1));
    c.bench_function("lumière d'une colonne (6 chunks)", |b| {
        b.iter(|| black_box(&region).compute())
    });
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
