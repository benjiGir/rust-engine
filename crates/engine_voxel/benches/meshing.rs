//! Benchmark du maillage : `cargo bench -p engine_voxel`.
//!
//! criterion lance chaque fonction des milliers de fois et donne un temps moyen
//! avec un intervalle de confiance. Les rapports HTML sont dans `target/criterion/`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{BlockRegistry, CHUNK_SIZE, TerrainGenerator, World, mesh_culled, mesh_greedy};
use glam::IVec3;

/// Le pire cas pour le greedy : un damier 3D, où aucune face ne peut être fusionnée.
fn checkerboard() -> World {
    let stone = BlockRegistry::builtin().id("core:stone").unwrap();
    let mut world = World::new();
    let chunk = world.chunk_mut(IVec3::ZERO);
    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    chunk.set(x, y, z, stone);
                }
            }
        }
    }
    world
}

fn bench_meshing(c: &mut Criterion) {
    // Un chunk de terrain entouré de ses voisins, pour tester aussi les bords.
    let terrain = TerrainGenerator::new(42).generate_region(3, 3);
    let scenes = [
        // y = 1 : la couche qui contient la surface (collines entre 26 et 54 blocs).
        ("terrain", terrain, IVec3::new(1, 1, 1)),
        ("damier", checkerboard(), IVec3::ZERO),
    ];

    for (name, world, chunk) in &scenes {
        // criterion ne mesure que le temps : on affiche les triangles à part.
        let culled = mesh_culled(world, *chunk).triangle_count();
        let greedy = mesh_greedy(world, *chunk).triangle_count();
        println!("{name} : {culled} triangles en culling, {greedy} en greedy");

        let mut group = c.benchmark_group(*name);
        // `black_box` empêche le compilateur d'optimiser un calcul dont on ignore le résultat.
        group.bench_function("culling", |b| {
            b.iter(|| mesh_culled(black_box(world), black_box(*chunk)))
        });
        group.bench_function("greedy", |b| {
            b.iter(|| mesh_greedy(black_box(world), black_box(*chunk)))
        });
        group.finish();
    }
}

criterion_group!(benches, bench_meshing);
criterion_main!(benches);
