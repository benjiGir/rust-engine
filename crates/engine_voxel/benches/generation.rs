//! Benchmark de la génération : `cargo bench -p engine_voxel --bench generation`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{LightRegion, SEA_LEVEL, TerrainGenerator, World, mesh_greedy};
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
    // Les chunks de surface les plus chargés : une forêt dense et une jungle (arbres des
    // 9 colonnes voisines recalculés à chaque chunk, voir `generator/features.rs`).
    let mut dense = Vec::new();
    for biome in ["core:forest", "core:jungle"] {
        let id = generator.biomes().id(biome).unwrap();
        let (x, z) = generator
            .find_column(0, 0, 32, 20_000, |c| c.biome == id && c.height > SEA_LEVEL)
            .unwrap();
        let column = generator.column(x, z);
        let pos = World::split(IVec3::new(x, column.height + 4, z)).0;
        group.bench_function(format!("surface {biome}"), |b| {
            b.iter(|| generator.generate(black_box(pos)))
        });
        dense.push((biome, pos));
    }
    group.finish();

    // Le maillage de ces chunks (feuilles découpées, plantes en croix).
    let mut group = c.benchmark_group("maillage dense");
    for (biome, pos) in dense {
        let world = generator.generate_area(IVec3::new(pos.x - 1, 0, pos.z - 1), 3, 3);
        group.bench_function(format!("greedy {biome}"), |b| {
            b.iter(|| mesh_greedy(black_box(&world), pos))
        });
        let mesh = mesh_greedy(&world, pos);
        eprintln!(
            "{biome} : {} triangles dans le chunk {pos}",
            mesh.triangle_count()
        );
    }
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
