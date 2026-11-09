//! Tests du bac à sable, avec de petits modules écrits à la main en WAT (le format texte
//! de WebAssembly) : pas besoin de compiler un mod Rust pour tester l'hôte.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, ChunkStreamer, GridPos, MachineAction, Machines, StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

use std::sync::Arc;

use engine_voxel::BlockRegistry;

use super::*;

fn loaded_streamer() -> Rc<RefCell<ChunkStreamer>> {
    streamer_with(TerrainGenerator::new(7))
}

fn streamer_with(generator: TerrainGenerator) -> Rc<RefCell<ChunkStreamer>> {
    let config = StreamingConfig {
        view_radius: 2,
        ..StreamingConfig::default()
    };
    let mut s = ChunkStreamer::new(generator, config);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        s.update(Vec3::new(5.0, 40.0, 5.0));
        while s.next_event().is_some() {}
        if s.is_idle() {
            break;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
    Rc::new(RefCell::new(s))
}

/// Un module minimal : mémoire, `sdk_alloc` (toujours l'adresse 1024), et le corps donné.
fn module(body: &str) -> String {
    format!(
        r#"(module
            (import "voxel" "subscribe" (func $subscribe (param i32)))
            (import "voxel" "give_item" (func $give_item (param i32 i32)))
            (import "voxel" "schedule_tick" (func $schedule_tick (param i32 i32 i32 i32 i32)))
            (import "voxel" "set_block" (func $set_block (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
            {body}
        )"#
    )
}

fn load(wat: &str, streamer: &Rc<RefCell<ChunkStreamer>>) -> Result<ScriptMod> {
    let registry = streamer.borrow().world().registry().clone();
    ScriptHost::new()?.load("test", wat.as_bytes(), streamer.clone(), registry)
}

fn broken() -> ModEvent {
    ModEvent::BlockBroken {
        at: GridPos::world(IVec3::new(1, 2, 3)),
        block: BlockId(1),
    }
}

#[test]
fn a_subscribed_handler_is_called_with_the_event() {
    let s = loaded_streamer();
    // À l'initialisation, le mod s'abonne à BlockBroken ; à chaque bloc cassé, il donne
    // `y` exemplaires de l'objet numéro `x` (lus dans les données de l'événement :
    // grille, x, y, z, bloc).
    let wat = module(
        r#"
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param $kind i32) (param $ptr i32) (param $len i32)
            (call $give_item (i32.load offset=4 (local.get $ptr)) (i32.load offset=8 (local.get $ptr))))
        "#,
    );
    let mut script = load(&wat, &s).unwrap();
    let requests = script.send_event(&broken()).unwrap();
    assert_eq!(
        requests.actions,
        vec![MachineAction::GiveItem {
            block: BlockId(1),
            count: 2
        }]
    );
    // Pas abonné aux ticks : rien.
    assert_eq!(
        script.send_event(&ModEvent::Tick(3)).unwrap(),
        Requests::default()
    );
}

#[test]
fn an_endless_loop_is_cut_off_and_the_mod_disabled() {
    let s = loaded_streamer();
    let wat = module(
        r#"
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param i32 i32 i32) (loop $forever (br $forever)))
        "#,
    );
    let mut script = load(&wat, &s).unwrap();
    let start = Instant::now();
    let err = script.send_event(&broken()).unwrap_err();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "l'appel aurait dû être interrompu"
    );
    assert!(err.to_string().contains("trop long"), "{err}");
    assert!(script.disabled().is_some());
    // Les appels suivants ne font plus rien : le jeu continue sans lui.
    assert_eq!(script.send_event(&broken()).unwrap(), Requests::default());
}

#[test]
fn a_crashing_mod_is_disabled() {
    let s = loaded_streamer();
    let wat = module(
        r#"
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param i32 i32 i32) unreachable)
        "#,
    );
    let mut script = load(&wat, &s).unwrap();
    let err = script.send_event(&broken()).unwrap_err();
    assert!(err.to_string().contains("plantage"), "{err}");
    let mut runtime = ModRuntime::default();
    runtime.add(script);
    assert_eq!(runtime.disabled().len(), 1);
    // Dispatcher à un mod désactivé ne plante pas le jeu.
    assert_eq!(runtime.dispatch(&broken()), Requests::default());
}

#[test]
fn the_sandbox_refuses_too_much_memory_and_unknown_imports() {
    let s = loaded_streamer();
    // 1000 pages de 64 Kio = 64 Mio : au-delà de la limite.
    let greedy = r#"(module (memory (export "memory") 1000) (func (export "sdk_alloc") (param i32) (result i32) i32.const 0))"#;
    assert!(load(greedy, &s).is_err());
    // Demander un accès aux fichiers (WASI) : refusé, le jeu ne le fournit pas.
    let wasi = r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1) (func (export "sdk_alloc") (param i32) (result i32) i32.const 0))"#;
    let err = load(wasi, &s).err().unwrap();
    assert!(format!("{err:#}").contains("ne fournit pas"), "{err:#}");
}

#[test]
fn a_scripted_machine_receives_its_scheduled_tick() {
    let s = loaded_streamer();
    let glass = s.borrow().world().registry().id("core:glass").unwrap();
    let machine = s.borrow().world().registry().id("core:sand").unwrap();
    // Posée : programme un tick dans 3 ticks. Au tick : pose du verre au-dessus.
    let wat = module(&format!(
        r#"
        (func (export "on_machines") (param $kind i32) (param $ptr i32) (param $count i32)
            (if (i32.eq (local.get $kind) (i32.const 0))
                (then (call $schedule_tick (i32.load (local.get $ptr))
                    (i32.load offset=4 (local.get $ptr)) (i32.load offset=8 (local.get $ptr))
                    (i32.load offset=12 (local.get $ptr)) (i32.const 3))))
            (if (i32.eq (local.get $kind) (i32.const 3))
                (then (drop (call $set_block (i32.load (local.get $ptr))
                    (i32.load offset=4 (local.get $ptr))
                    (i32.add (i32.load offset=8 (local.get $ptr)) (i32.const 1))
                    (i32.load offset=12 (local.get $ptr)) (i32.const {})))))
        )
        "#,
        glass.0
    ));
    let script = Rc::new(RefCell::new(load(&wat, &s).unwrap()));
    let mut machines = Machines::new();
    machines.register(machine, Rc::new(ScriptBehavior::new(script)));
    let at = IVec3::new(8, 100, 8);
    assert!(s.borrow_mut().set_block(at, machine));
    machines.block_changed(&s, at, &mut Vec::new());
    let mut placed_at = None;
    for _ in 0..6 {
        let tick = machines.scheduler().tick();
        machines.run_tick(&s, &mut Vec::new());
        if placed_at.is_none() && s.borrow().world().block(at + IVec3::Y) == glass {
            placed_at = Some(tick);
        }
    }
    assert!(placed_at.is_some(), "le tick programmé n'est jamais arrivé");
}

#[test]
fn hot_reload_swaps_the_script_behind_existing_behaviors() {
    let s = loaded_streamer();
    let v1 = module(
        r#"(func (export "mod_init") (call $subscribe (i32.const 1)))
           (func (export "on_event") (param i32 i32 i32) (call $give_item (i32.const 1) (i32.const 1)))"#,
    );
    let v2 = v1.replace(
        "(i32.const 1) (i32.const 1))",
        "(i32.const 1) (i32.const 5))",
    );
    let mut runtime = ModRuntime::default();
    runtime.add(load(&v1, &s).unwrap());
    assert!(runtime.replace(load(&v2, &s).unwrap()));
    assert_eq!(
        runtime.dispatch(&broken()).actions,
        vec![MachineAction::GiveItem {
            block: BlockId(1),
            count: 5
        }]
    );
}

#[test]
fn capabilities_are_reachable_from_a_script() {
    let s = loaded_streamer();
    // Le mod donne un stock d'énergie au bloc (0, 100, 0) du monde, puis y insère 70
    // (débit maximal 50) ; dans une grille inconnue (7), rien ne se passe.
    let wat = r#"(module
        (import "voxel" "add_energy" (func $add_energy (param i32 i32 i32 i32 i64 i64 i32) (result i32)))
        (import "voxel" "energy_insert" (func $insert (param i32 i32 i32 i32 i32 i64 i32) (result i64)))
        (import "voxel" "give_item" (func $give_item (param i32 i32)))
        (memory (export "memory") 1)
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "mod_init")
            (drop (call $add_energy (i32.const 0) (i32.const 0) (i32.const 100) (i32.const 0) (i64.const 1000) (i64.const 50) (i32.const 63)))
            (drop (call $add_energy (i32.const 7) (i32.const 0) (i32.const 100) (i32.const 0) (i64.const 1000) (i64.const 50) (i32.const 63)))
            (call $give_item (i32.const 1)
                (i32.wrap_i64 (call $insert (i32.const 0) (i32.const 0) (i32.const 100) (i32.const 0) (i32.const -1) (i64.const 70) (i32.const 0)))))
    )"#;
    let registry = s.borrow().world().registry().clone();
    let mut script = ScriptHost::new()
        .unwrap()
        .load("caps", wat.as_bytes(), s.clone(), registry)
        .unwrap();
    assert!(script.disabled().is_none());
    let entity = s
        .borrow()
        .block_entity(IVec3::new(0, 100, 0))
        .cloned()
        .unwrap();
    assert_eq!(
        entity.caps.unwrap().energy.unwrap().amount,
        50,
        "débit limité à 50"
    );
    let _ = script.send_event(&broken());
}

/// Le vrai mod d'exemple, compilé depuis Rust (`cargo xtask build-mods`) : une horloge
/// alimente le casseur, qui casse la pierre devant lui et la donne au joueur.
#[test]
fn the_example_mod_breaks_blocks_when_powered() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wasm = root.join("mods/example/example.wasm");
    let Ok(bytes) = std::fs::read(&wasm) else {
        panic!(
            "{} manquant : lancer `cargo xtask build-mods`",
            wasm.display()
        );
    };
    let registry =
        Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap());
    let s = streamer_with(TerrainGenerator::with_registry(7, registry.clone()).unwrap());
    let id = |key: &str| registry.id(key).unwrap();
    let (breaker, clock, stone) = (id("example:breaker"), id("core:clock"), id("core:stone"));

    let mut runtime = ModRuntime::default();
    runtime.add(
        ScriptHost::new()
            .unwrap()
            .load("example", &bytes, s.clone(), registry.clone())
            .unwrap(),
    );
    let mut machines = Machines::with_natives(&registry);
    runtime.register_behaviors(&mut machines, &registry);
    assert!(machines.has_behavior(breaker));

    // Casseur en (8, 100, 8) tourné vers +Z, pierre devant, horloge à côté.
    let at = IVec3::new(8, 100, 8);
    let mut actions = Vec::new();
    let mut place = |pos: IVec3, block: BlockId, machines: &mut Machines| {
        assert!(s.borrow_mut().set_block(pos, block));
        machines.block_changed(&s, pos, &mut actions);
    };
    place(at, breaker, &mut machines);
    let requests = runtime.dispatch(&ModEvent::BlockPlaced {
        at: GridPos::world(at),
        block: breaker,
        facing: IVec3::Z,
    });
    assert!(requests.actions.is_empty());
    place(at + IVec3::Z, stone, &mut machines);
    place(at + IVec3::NEG_X, clock, &mut machines);

    let mut actions = Vec::new();
    for _ in 0..120 {
        machines.run_tick(&s, &mut actions);
    }
    assert_eq!(
        s.borrow().world().block(at + IVec3::Z),
        BlockId::AIR,
        "la pierre aurait dû être cassée"
    );
    assert!(
        actions.contains(&MachineAction::GiveItem {
            block: stone,
            count: 1
        }),
        "{actions:?}"
    );
    assert!(runtime.disabled().is_empty());
}
