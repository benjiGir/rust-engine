//! Benchmark de la sauvegarde : `cargo bench -p engine_voxel --bench save`.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use engine_voxel::{ChunkToSave, SaveStore, TerrainGenerator, UnknownBlocks};
use glam::IVec3;

fn bench_save(c: &mut Criterion) {
    let generator = TerrainGenerator::new(42);
    let registry = generator.registry().clone();
    let dir = std::env::temp_dir().join(format!("voxel-bench-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = SaveStore::open(&dir).unwrap();

    // Trois chunks typiques : sous terre (pierre et grottes), surface (herbe, terre, eau), ciel.
    for (name, pos) in [
        ("souterrain", IVec3::new(3, 0, 5)),
        ("surface", IVec3::new(3, 1, 5)),
    ] {
        let chunk = Arc::new(generator.generate(pos).unwrap());
        let save = [ChunkToSave {
            pos,
            chunk,
            unknown: UnknownBlocks::new(),
        }];
        // La taille sur disque : une région qui ne contient que ce chunk.
        let one = SaveStore::open(dir.join(name)).unwrap();
        let report = one.save_chunks(&save, &registry).unwrap();
        println!(
            "{name} : {} octets sur disque (64 Ko de blocs en mémoire, 32 Ko de lumière non sauvegardée)",
            report.bytes
        );
        c.bench_function(&format!("sauvegarde d'un chunk ({name})"), |b| {
            b.iter(|| store.save_chunks(black_box(&save), &registry).unwrap())
        });
        c.bench_function(&format!("chargement d'un chunk ({name})"), |b| {
            b.iter(|| store.load_chunk(black_box(pos), &registry).unwrap())
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, bench_save);
criterion_main!(benches);
