//! Test de charge des machines : `cargo bench -p engine_voxel --bench machines`.
//!
//! N horloges qui travaillent **à chaque tick** (période 1) : le pire cas, toutes les
//! machines actives en même temps. Chacune bascule sa puissance et prévient ses voisins.
//! Le budget visé : moins de 10 ms par tick pour 50 000 machines.
//!
//! Deuxième scénario, « câbles et tuyaux » : 50 000 blocs passifs avec des capacités
//! (25 000 stocks d'énergie, 25 000 réservoirs d'eau). On mesure ce que coûte leur
//! **stockage** dans les chunks, en mémoire et au moment de la sauvegarde.
//!
//! Troisième scénario, « réseaux » (phase 9c) : un réseau de N câbles, des batteries et des
//! lampes ; le coût d'un tick de réseau en régime établi, puis la reconstruction après avoir
//! cassé un câble.

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

/// Un réseau de `n` câbles (des rangées reliées par une colonne), une batterie chargée et une
/// lampe électrique tous les 20 câbles.
fn grid_network(n: usize) -> (RefCell<ChunkStreamer>, Machines, Vec<IVec3>) {
    let streamer = loaded_world();
    let registry = streamer.world().registry().clone();
    let (cable, battery, lamp) = (
        registry.id("core:cable").unwrap(),
        registry.id("core:battery").unwrap(),
        registry.id("core:power_lamp").unwrap(),
    );
    let mut machines = Machines::with_natives(&registry);
    let mut actions = Vec::new();
    let streamer = RefCell::new(streamer);
    let mut cables = Vec::new();
    // Des étages de 40 rangées de 100 câbles, tous les 2 blocs, reliés par une colonne.
    let per_layer = 100 * 40;
    let y = 100;
    for i in 0..n {
        let (layer, row, x) = (i / per_layer, (i % per_layer) / 100, (i % 100) as i32);
        let pos = IVec3::new(x - 50, y + layer as i32 * 2, row as i32 * 2 - 40);
        assert!(
            streamer.borrow_mut().set_block_raw(pos, cable),
            "hors du monde chargé"
        );
        machines.block_changed(&streamer, pos, &mut actions);
        cables.push(pos);
        // Une colonne à x = -50 relie les rangées, puis les étages.
        let link = if x == 0 && row > 0 {
            Some(pos - IVec3::Z)
        } else if x == 0 && row == 0 && layer > 0 {
            Some(pos - IVec3::Y)
        } else {
            None
        };
        if let Some(link) = link {
            streamer.borrow_mut().set_block_raw(link, cable);
            machines.block_changed(&streamer, link, &mut actions);
        }
        if i % 250 == 5 {
            let up = pos + IVec3::Y;
            streamer.borrow_mut().set_block_raw(up, lamp);
            machines.block_changed(&streamer, up, &mut actions);
        }
    }
    let source = IVec3::new(-51, y, -40);
    streamer.borrow_mut().set_block_raw(source, battery);
    machines.block_changed(&streamer, source, &mut actions);
    streamer.borrow_mut().set_block_entity(
        source,
        Some(BlockEntity {
            caps: Some(Box::new(Capabilities {
                energy: Some(EnergyStorage {
                    amount: 90_000,
                    ..EnergyStorage::new(100_000, 2_000)
                }),
                ..Default::default()
            })),
            ..Default::default()
        }),
    );
    (streamer, machines, cables)
}

fn networks(c: &mut Criterion) {
    let mut group = c.benchmark_group("réseaux");
    group.sample_size(20);
    for n in [1_000, 10_000, 50_000] {
        let (streamer, mut machines, cables) = grid_network(n);
        let mut actions = Vec::new();
        // Le premier tick construit le réseau.
        let start = std::time::Instant::now();
        machines.run_tick(&streamer, &mut actions);
        eprintln!(
            "{n} câbles : construction du réseau en {:.1} ms ({:?})",
            start.elapsed().as_secs_f64() * 1000.0,
            machines.networks().stats()
        );
        group.bench_function(BenchmarkId::new("tick", n), |b| {
            b.iter(|| machines.run_tick(&streamer, &mut actions))
        });
        // Casser un câble au milieu : reconstruction des deux morceaux, puis le remettre.
        let middle = cables[n / 2];
        let cable = streamer
            .borrow()
            .world()
            .registry()
            .id("core:cable")
            .unwrap();
        group.bench_function(BenchmarkId::new("casser et remettre un câble", n), |b| {
            b.iter(|| {
                streamer
                    .borrow_mut()
                    .set_block_raw(middle, engine_voxel::BlockId::AIR);
                machines.block_changed(&streamer, middle, &mut actions);
                machines.run_tick(&streamer, &mut actions);
                streamer.borrow_mut().set_block_raw(middle, cable);
                machines.block_changed(&streamer, middle, &mut actions);
                machines.run_tick(&streamer, &mut actions);
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench, conduits, networks);
criterion_main!(benches);
