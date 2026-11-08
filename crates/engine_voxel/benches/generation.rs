//! Benchmark de la génération : `cargo bench -p engine_voxel --bench generation`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::TerrainGenerator;
use glam::IVec3;

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
}

criterion_group!(benches, bench_generation);
criterion_main!(benches);
