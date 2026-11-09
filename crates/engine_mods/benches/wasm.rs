//! Le coût du WebAssembly : `cargo bench -p engine_mods --bench wasm`.
//!
//! - **jeu → mod** : un appel `on_event` qui ne fait rien (le coût fixe d'un appel, avec
//!   le plein de carburant et l'écriture des données dans la mémoire du mod) ;
//! - **mod → jeu** : un appel qui fait 10 000 `get_block` (le coût d'une fonction hôte) ;
//! - **10 000 machines scriptées** : une horloge écrite en WebAssembly (lire sa puissance,
//!   la basculer, reprogrammer un tick), comparée à l'horloge native, et en un seul appel
//!   (par lot) contre un appel par machine.

use std::cell::RefCell;
use std::rc::Rc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use engine_mods::{ModEvent, ScriptBehavior, ScriptHost, ScriptMod, abi};
use engine_voxel::{
    BlockEntity, BlockId, ChunkStreamer, GridPos, Machines, StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

const MACHINES: usize = 10_000;

fn loaded_world() -> Rc<RefCell<ChunkStreamer>> {
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
    Rc::new(RefCell::new(streamer))
}

/// Un mod de mesure : un `on_event` vide, un `on_event` qui appelle 10 000 fois le jeu
/// (type d'événement 4, « tick »), et une horloge scriptée dans `on_machines`.
const BENCH_MOD: &str = r#"(module
    (import "voxel" "subscribe" (func $subscribe (param i32)))
    (import "voxel" "get_block" (func $get_block (param i32 i32 i32 i32) (result i32)))
    (import "voxel" "get_power" (func $get_power (param i32 i32 i32 i32) (result i32)))
    (import "voxel" "set_power" (func $set_power (param i32 i32 i32 i32 i32) (result i32)))
    (import "voxel" "schedule_tick" (func $schedule_tick (param i32 i32 i32 i32 i32)))
    (memory (export "memory") 4)
    (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
    (func (export "mod_init") (call $subscribe (i32.const 1)) (call $subscribe (i32.const 4)))
    (func (export "on_event") (param $kind i32) (param $ptr i32) (param $len i32) (local $i i32)
        (if (i32.eq (local.get $kind) (i32.const 4))
            (then (loop $calls
                (drop (call $get_block (i32.const 0) (local.get $i) (i32.const 64) (i32.const 0)))
                (local.set $i (i32.add (local.get $i) (i32.const 1)))
                (br_if $calls (i32.lt_u (local.get $i) (i32.const 10000)))))))
    ;; L'horloge : pour chaque position (grille, x, y, z = 16 octets), basculer la
    ;; puissance entre 0 et 15 et se reprogrammer au tick suivant.
    (func (export "on_machines") (param $kind i32) (param $ptr i32) (param $count i32)
        (local $i i32) (local $p i32) (local $g i32) (local $x i32) (local $y i32) (local $z i32)
        (if (i32.eqz (local.get $count)) (then (return)))
        (loop $each
            (local.set $p (i32.add (local.get $ptr) (i32.shl (local.get $i) (i32.const 4))))
            (local.set $g (i32.load (local.get $p)))
            (local.set $x (i32.load offset=4 (local.get $p)))
            (local.set $y (i32.load offset=8 (local.get $p)))
            (local.set $z (i32.load offset=12 (local.get $p)))
            (drop (call $set_power (local.get $g) (local.get $x) (local.get $y) (local.get $z)
                (select (i32.const 0) (i32.const 15)
                    (call $get_power (local.get $g) (local.get $x) (local.get $y) (local.get $z)))))
            (call $schedule_tick (local.get $g) (local.get $x) (local.get $y) (local.get $z) (i32.const 1))
            (local.set $i (i32.add (local.get $i) (i32.const 1)))
            (br_if $each (i32.lt_u (local.get $i) (local.get $count)))))
)"#;

fn load(world: &Rc<RefCell<ChunkStreamer>>) -> ScriptMod {
    let registry = world.borrow().world().registry().clone();
    ScriptHost::new()
        .unwrap()
        .load("bench", BENCH_MOD.as_bytes(), world.clone(), registry)
        .unwrap()
}

fn positions() -> Vec<IVec3> {
    let mut out = Vec::new();
    'fill: for y in (64..160).step_by(2) {
        for x in (-48..48).step_by(2) {
            for z in (-48..48).step_by(2) {
                if out.len() == MACHINES {
                    break 'fill;
                }
                out.push(IVec3::new(x, y, z));
            }
        }
    }
    out
}

/// `MACHINES` blocs `block` avec un état, enregistrés auprès de `machines`.
fn place(
    world: &Rc<RefCell<ChunkStreamer>>,
    machines: &mut Machines,
    block: BlockId,
    data: Vec<u8>,
) {
    let mut actions = Vec::new();
    for pos in positions() {
        world.borrow_mut().set_block_raw(pos, block);
        world.borrow_mut().set_block_entity(
            pos,
            Some(BlockEntity {
                data: data.clone(),
                ..Default::default()
            }),
        );
        machines.block_changed(world, pos, &mut actions);
    }
}

fn calls(c: &mut Criterion) {
    let world = loaded_world();
    let mut script = load(&world);
    let mut group = c.benchmark_group("appels wasm");
    let broken = ModEvent::BlockBroken {
        at: GridPos::world(IVec3::ZERO),
        block: BlockId(1),
    };
    group.bench_function("jeu → mod (un appel vide)", |b| {
        b.iter(|| script.send_event(&broken).unwrap())
    });
    group.bench_function("mod → jeu (10 000 get_block)", |b| {
        b.iter(|| script.send_event(&ModEvent::Tick(1)).unwrap())
    });
    group.finish();
}

fn scripted_machines(c: &mut Criterion) {
    let mut group = c.benchmark_group("10 000 horloges");
    group.sample_size(20);

    // La référence : l'horloge native (période 1).
    {
        let world = loaded_world();
        let registry = world.borrow().world().registry().clone();
        let mut machines = Machines::with_natives(&registry);
        place(
            &world,
            &mut machines,
            registry.id("core:clock").unwrap(),
            vec![1],
        );
        let mut actions = Vec::new();
        machines.run_tick(&world, &mut actions);
        group.bench_function(BenchmarkId::new("tick", "native"), |b| {
            b.iter(|| machines.run_tick(&world, &mut actions))
        });
    }

    // La même en WebAssembly, réveillée par lot (un seul appel par tick).
    {
        let world = loaded_world();
        let registry = world.borrow().world().registry().clone();
        let glass = registry.id("core:glass").unwrap();
        let script = Rc::new(RefCell::new(load(&world)));
        let mut machines = Machines::new();
        machines.register(glass, Rc::new(ScriptBehavior::new(script.clone())));
        place(&world, &mut machines, glass, Vec::new());
        let mut actions = Vec::new();
        machines.run_tick(&world, &mut actions);
        group.bench_function(BenchmarkId::new("tick", "wasm par lot"), |b| {
            b.iter(|| {
                let report = machines.run_tick(&world, &mut actions);
                assert_eq!(report.scheduled, MACHINES);
            })
        });

        // Sans lot : un appel par machine (ce que le moteur évite).
        let all: Vec<GridPos> = positions().into_iter().map(GridPos::world).collect();
        group.bench_function(BenchmarkId::new("appels", "wasm un par machine"), |b| {
            b.iter(|| {
                let mut script = script.borrow_mut();
                for &at in &all {
                    script.machines(abi::MACHINES_TICK, &[at]).unwrap();
                }
            })
        });
        group.bench_function(BenchmarkId::new("appels", "wasm en un appel"), |b| {
            b.iter(|| {
                script
                    .borrow_mut()
                    .machines(abi::MACHINES_TICK, &all)
                    .unwrap()
            })
        });
    }
    group.finish();
}

criterion_group!(benches, calls, scripted_machines);
criterion_main!(benches);
