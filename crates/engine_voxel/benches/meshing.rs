//! Benchmark du maillage : `cargo bench -p engine_voxel`.
//!
//! criterion lance chaque fonction des milliers de fois et donne un temps moyen
//! avec un intervalle de confiance. Les rapports HTML sont dans `target/criterion/`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{
    BlockRegistry, CHUNK_SIZE, FLAG_CUTOUT, TerrainGenerator, World, mesh_binary, mesh_culled,
    mesh_greedy,
};
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

/// Dans une région générée, le chunk dont le maillage a le plus de sommets qui satisfont `wanted`.
fn richest(world: &World, wanted: impl Fn(&engine_voxel::MeshVertex) -> bool) -> IVec3 {
    world
        .chunk_positions()
        .max_by_key(|&pos| {
            mesh_greedy(world, pos)
                .vertices
                .iter()
                .filter(|v| wanted(v))
                .count()
        })
        .expect("une région non vide")
}

fn bench_meshing(c: &mut Criterion) {
    // Un chunk de terrain entouré de ses voisins, pour tester aussi les bords.
    let terrain = TerrainGenerator::new(42).generate_region(3, 3);
    // Des chunks qui ont de l'eau (maillage translucide) et du feuillage (voie complète), dans une
    // région plus large ; la forêt et la mer comptent plus que la plaine pour le coût réel.
    let wide = TerrainGenerator::new(7).generate_region(5, 5);
    let forest = richest(&wide, |v| v.texture & FLAG_CUTOUT != 0);
    let sea = wide
        .chunk_positions()
        .max_by_key(|&pos| mesh_greedy(&wide, pos).translucent_indices.len())
        .expect("une région non vide");
    let scenes = [
        // y = 1 : la couche qui contient la surface (collines entre 26 et 54 blocs).
        ("terrain", terrain, IVec3::new(1, 1, 1)),
        ("foret", wide.clone(), forest),
        ("mer", wide, sea),
        ("damier", checkerboard(), IVec3::ZERO),
    ];

    for (name, world, chunk) in &scenes {
        // criterion ne mesure que le temps : on affiche les triangles à part.
        let culled = mesh_culled(world, *chunk).triangle_count();
        let greedy = mesh_greedy(world, *chunk).triangle_count();
        let binary = mesh_binary(world, *chunk).triangle_count();
        println!("{name} : {culled} triangles en culling, {greedy} en greedy, {binary} en binaire");

        let mut group = c.benchmark_group(*name);
        // `black_box` empêche le compilateur d'optimiser un calcul dont on ignore le résultat.
        group.bench_function("culling", |b| {
            b.iter(|| mesh_culled(black_box(world), black_box(*chunk)))
        });
        group.bench_function("greedy", |b| {
            b.iter(|| mesh_greedy(black_box(world), black_box(*chunk)))
        });
        group.bench_function("binaire", |b| {
            b.iter(|| mesh_binary(black_box(world), black_box(*chunk)))
        });
        group.finish();
    }
}

criterion_group!(benches, bench_meshing);
criterion_main!(benches);
