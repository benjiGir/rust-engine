//! Test de charge des machines : `cargo bench -p engine_voxel --bench machines`.
//!
//! N horloges qui travaillent **à chaque tick** (période 1) : le pire cas, toutes les
//! machines actives en même temps. Chacune bascule sa puissance et prévient ses voisins.
//! Le budget visé : moins de 10 ms par tick pour 50 000 machines.
//!
//! Deuxième scénario, « câbles et tuyaux » : 50 000 blocs passifs avec des capacités
//! (25 000 stocks d'énergie, 25 000 réservoirs d'eau). Ils ne travaillent pas (ce sera le
//! rôle des réseaux, phase 9c) : on mesure ce que coûte leur **stockage** dans les chunks,
//! en mémoire et au moment de la sauvegarde.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::cell::RefCell;

use engine_voxel::{
    BlockEntity, Capabilities, ChunkStreamer, ChunkToSave, EnergyStorage, FluidTank, Machines,
    SaveStore, StreamingConfig, TerrainGenerator, UnknownBlocks, World,
};
use glam::{IVec3, Vec3};

/// Un monde chargé autour de l'origine, avec `n` horloges dans le ciel (une case sur deux).
fn loaded_world() -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: 3,
        ..StreamingConfig::default()
    };
    let mut streamer = ChunkStreamer::new(TerrainGenerator::new(1), config);
    loop {
        streamer.update(Vec3::new(5.0, 60.0, 5.0));
        while streamer.next_event().is_some() {}
        if streamer.is_idle() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    streamer
}

/// Les positions d'une grille dans le ciel : une case sur deux, 48 × 48 par étage.
fn grid_positions(n: usize) -> Vec<IVec3> {
    let mut out = Vec::with_capacity(n);
    'fill: for y in (64..160).step_by(2) {
        for x in (-48..48).step_by(2) {
            for z in (-48..48).step_by(2) {
                if out.len() == n {
                    break 'fill;
                }
                out.push(IVec3::new(x, y, z));
            }
        }
    }
    assert_eq!(out.len(), n, "pas assez de place pour {n} blocs");
    out
}

fn factory(n: usize) -> (RefCell<ChunkStreamer>, Machines) {
    let streamer = loaded_world();
    let registry = streamer.world().registry().clone();
    let clock = registry.id("core:clock").unwrap();
    let mut machines = Machines::with_natives(&registry);
    let mut actions = Vec::new();
    let streamer = RefCell::new(streamer);
    for pos in grid_positions(n) {
        streamer.borrow_mut().set_block_raw(pos, clock);
        // Période 1 : l'horloge travaille à chaque tick.
        streamer.borrow_mut().set_block_entity(
            pos,
            Some(BlockEntity {
                data: vec![1],
                ..Default::default()
            }),
        );
        machines.block_changed(&streamer, pos, &mut actions);
    }
    (streamer, machines)
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("tick de machines");
    group.sample_size(20);
    for n in [10_000, 50_000, 100_000] {
        let (streamer, mut machines) = factory(n);
        let mut actions = Vec::new();
        // Un premier tick : toutes les horloges sont programmées pour le suivant.
        machines.run_tick(&streamer, &mut actions);
        group.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter(|| {
                let report = machines.run_tick(&streamer, &mut actions);
                assert_eq!(report.scheduled, n);
            })
        });
    }
    group.finish();
}

/// Un câble (un stock d'énergie) ou un tuyau (un réservoir d'eau à moitié plein).
fn conduit(i: usize) -> BlockEntity {
    let mut caps = Capabilities::default();
    if i.is_multiple_of(2) {
        let mut energy = EnergyStorage::new(1000, 100);
        energy.amount = 500;
        caps.energy = Some(energy);
    } else {
        let mut tank = FluidTank::new(1000);
        tank.fill(None, "core:water", 500, false);
        caps.tanks.push(tank);
    }
    BlockEntity {
        caps: Some(Box::new(caps)),
        ..Default::default()
    }
}

fn conduits(c: &mut Criterion) {
    const N: usize = 50_000;
    let mut streamer = loaded_world();
    let glass = streamer.world().registry().id("core:glass").unwrap();
    let positions = grid_positions(N);
    for (i, &pos) in positions.iter().enumerate() {
        streamer.set_block_raw(pos, glass);
        streamer.set_block_entity(pos, Some(conduit(i)));
    }

    // La mémoire, estimée : l'entrée de la table du chunk (clé + état, plus ~1 octet de
    // contrôle), la boîte des capacités, et ce qu'elles allouent (réservoir, nom du fluide).
    let entry = size_of::<(u16, BlockEntity)>() + 1;
    let caps = size_of::<Capabilities>();
    let tank = size_of::<FluidTank>() + "core:water".len();
    let total = N * (entry + caps) + N / 2 * tank;
    eprintln!(
        "{N} câbles et tuyaux : ~{} Kio en mémoire ({} octets par bloc en moyenne : entrée {entry}, capacités {caps}, réservoir {tank})",
        total / 1024,
        total / N
    );

    let chunks: Vec<IVec3> = {
        let mut set: Vec<IVec3> = positions.iter().map(|&p| World::split(p).0).collect();
        set.sort_by_key(|p| p.to_array());
        set.dedup();
        set
    };
    let to_save: Vec<ChunkToSave> = chunks
        .iter()
        .map(|&pos| ChunkToSave {
            pos,
            chunk: streamer.world().chunk_arc(pos).unwrap(),
            unknown: UnknownBlocks::new(),
        })
        .collect();
    let registry = streamer.world().registry().clone();
    let dir = std::env::temp_dir().join(format!("voxel-bench-conduits-{}", std::process::id()));
    let store = SaveStore::open(&dir).unwrap();
    let report = store.save_chunks(&to_save, &registry).unwrap();
    eprintln!(
        "sauvegarde : {} chunks, {} régions, {} Kio sur disque",
        report.chunks,
        report.regions,
        report.bytes / 1024
    );

    let mut group = c.benchmark_group("câbles et tuyaux");
    group.sample_size(10);
    group.bench_function(BenchmarkId::new("sauvegarde", N), |b| {
        b.iter(|| store.save_chunks(&to_save, &registry).unwrap())
    });
    group.finish();
    let _ = std::fs::remove_dir_all(dir);
}

criterion_group!(benches, bench, conduits);
criterion_main!(benches);
