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
use crate::menu as abi_ui;

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

/// Le catalogue `mod_sdk/hooks.ron`, réduit à ce que le test compare.
#[derive(serde::Deserialize)]
struct Catalog {
    module: String,
    functions: Vec<CatalogFunction>,
    events: Vec<CatalogEntry>,
    machine_calls: Vec<CatalogEntry>,
}

#[derive(serde::Deserialize)]
struct CatalogFunction {
    name: String,
    params: Vec<(String, CatalogType)>,
    #[serde(default)]
    ret: Option<CatalogType>,
}

#[derive(serde::Deserialize, Debug, Clone, Copy)]
enum CatalogType {
    I32,
    I64,
    F32,
    F64,
}

#[derive(serde::Deserialize)]
struct CatalogEntry {
    id: i32,
    name: String,
}

fn catalog() -> Catalog {
    let text = include_str!("../../../mod_sdk/hooks.ron");
    ron::from_str(text).expect("catalogue illisible")
}

fn wasm_type(ty: &wasmtime::ValType) -> String {
    use wasmtime::ValType;
    match ty {
        ValType::I32 => "I32",
        ValType::I64 => "I64",
        ValType::F32 => "F32",
        ValType::F64 => "F64",
        _ => "autre",
    }
    .to_string()
}

/// Le jeu définit exactement les fonctions du catalogue, avec les mêmes types : un mod
/// compilé avec le SDK (généré depuis le catalogue) se lie toujours.
#[test]
fn host_matches_the_catalog() {
    let catalog = catalog();
    let host = ScriptHost::new().unwrap();
    let streamer = Rc::new(RefCell::new(ChunkStreamer::new(
        TerrainGenerator::new(1),
        StreamingConfig::default(),
    )));
    let state = HostState {
        mod_id: "test".into(),
        streamer,
        registry: BlockRegistry::builtin(),
        content: Default::default(),
        settings: Default::default(),
        requests: Requests::default(),
        subscriptions: Vec::new(),
        limits: wasmtime::StoreLimitsBuilder::new().build(),
        rng: 1,
    };
    let mut store = wasmtime::Store::new(&host.engine, state);
    let mut defined = std::collections::BTreeMap::new();
    let funcs: Vec<(String, wasmtime::Func)> = host
        .linker
        .iter(&mut store)
        .filter(|(module, _, _)| *module == catalog.module)
        .filter_map(|(_, name, ext)| Some((name.to_string(), ext.into_func()?)))
        .collect();
    for (name, func) in funcs {
        let ty = func.ty(&store);
        let params: Vec<String> = ty.params().map(|p| wasm_type(&p)).collect();
        let results: Vec<String> = ty.results().map(|r| wasm_type(&r)).collect();
        defined.insert(name, (params, results));
    }

    let mut errors = Vec::new();
    for f in &catalog.functions {
        let expected_params: Vec<String> = f.params.iter().map(|(_, t)| format!("{t:?}")).collect();
        let expected_results: Vec<String> = f.ret.iter().map(|t| format!("{t:?}")).collect();
        match defined.remove(&f.name) {
            None => errors.push(format!("« {} » est au catalogue, pas dans le jeu", f.name)),
            Some((params, results)) => {
                if params != expected_params || results != expected_results {
                    errors.push(format!(
                        "« {} » : le jeu prend {params:?} → {results:?}, le catalogue dit {expected_params:?} → {expected_results:?}",
                        f.name
                    ));
                }
            }
        }
    }
    for name in defined.keys() {
        errors.push(format!(
            "« {name} » est définie par le jeu mais absente du catalogue"
        ));
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Les numéros envoyés par le jeu sont ceux du catalogue (abi.rs en est généré, mais
/// `ModEvent::kind` choisit encore la constante à la main).
#[test]
fn event_numbers_match_the_catalog() {
    let catalog = catalog();
    let id = |name: &str| {
        catalog
            .events
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.id)
            .unwrap_or_else(|| panic!("{name} absent du catalogue"))
    };
    let at = GridPos::world(IVec3::ZERO);
    let events = [
        (
            "BlockBroken",
            ModEvent::BlockBroken {
                at,
                block: BlockId(1),
            },
        ),
        (
            "BlockPlaced",
            ModEvent::BlockPlaced {
                at,
                block: BlockId(1),
                facing: IVec3::X,
            },
        ),
        (
            "EntityDied",
            ModEvent::EntityDied {
                kind: "core:pig".into(),
                position: Vec3::ZERO,
            },
        ),
        ("Tick", ModEvent::Tick(1)),
        (
            "PlayerDamaged",
            ModEvent::PlayerDamaged {
                amount: 1.0,
                health_left: 19.0,
            },
        ),
    ];
    assert_eq!(events.len(), catalog.events.len(), "un événement sans test");
    for (name, event) in events {
        assert_eq!(event.kind(), id(name), "{name}");
    }
    let calls: Vec<(String, i32)> = catalog
        .machine_calls
        .iter()
        .map(|m| (m.name.clone(), m.id))
        .collect();
    assert_eq!(calls.len(), 4);
}

/// Un hôte qui connaît les objets du jeu et une recette `core:press` : 2 charbon → 1 charbon de bois.
fn content_host(streamer: &Rc<RefCell<ChunkStreamer>>) -> ScriptHost {
    let registry = streamer.borrow().world().registry().clone();
    let items = engine_voxel::ItemRegistry::builtin(&registry).unwrap();
    let mut recipes = engine_voxel::MachineRecipes::default();
    recipes
        .add_source(
            "core",
            r#"(machines: {"press": [(inputs: [("coal", 2)], outputs: [("charcoal", 1)], time: 1.0, energy: 40)]})"#,
            "test",
            &items,
        )
        .unwrap();
    let mut host = ScriptHost::new().unwrap();
    host.set_content(ModContent {
        items: Arc::new(items),
        machine_recipes: Arc::new(recipes),
    });
    host
}

#[test]
fn a_script_finds_items_and_machine_recipes() {
    let s = loaded_streamer();
    // Les textes sont rangés en mémoire : « core:press » (0), « core:coal*2 » (16),
    // « core:coal*1 » (32), « core:charcoal » (48). À chaque événement, le mod donne un
    // charbon de bois, puis autant que la taille du texte de la recette trouvée (-1 si
    // l'entrée est insuffisante : rien n'est donné).
    let wat = r#"(module
        (import "voxel" "subscribe" (func $subscribe (param i32)))
        (import "voxel" "item_id" (func $item_id (param i32 i32) (result i32)))
        (import "voxel" "give_stack" (func $give (param i32 i32)))
        (import "voxel" "recipe_find" (func $find (param i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "core:press")
        (data (i32.const 16) "core:coal*2")
        (data (i32.const 32) "core:coal*1")
        (data (i32.const 48) "core:charcoal")
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param i32 i32 i32)
            (call $give (call $item_id (i32.const 48) (i32.const 13)) (i32.const 1))
            (call $give (call $item_id (i32.const 48) (i32.const 13))
                (call $find (i32.const 0) (i32.const 10) (i32.const 16) (i32.const 11) (i32.const 512) (i32.const 256)))
            (call $give (call $item_id (i32.const 48) (i32.const 13))
                (call $find (i32.const 0) (i32.const 10) (i32.const 32) (i32.const 11) (i32.const 512) (i32.const 256))))
    )"#;
    let registry = s.borrow().world().registry().clone();
    let mut script = content_host(&s)
        .load("recipes", wat.as_bytes(), s.clone(), registry)
        .unwrap();
    let requests = script.send_event(&broken()).unwrap();
    let charcoal = engine_voxel::ItemRegistry::builtin(&BlockRegistry::builtin())
        .unwrap()
        .id("core:charcoal")
        .unwrap();
    // « 20;40;core:coal*2;core:charcoal*1 » : 33 octets.
    assert_eq!(
        requests.actions,
        vec![
            MachineAction::GiveStack {
                item: charcoal,
                count: 1
            },
            MachineAction::GiveStack {
                item: charcoal,
                count: 33
            },
        ]
    );
}

#[test]
fn a_right_click_reaches_the_script_which_may_take_it() {
    let s = loaded_streamer();
    // `on_use` prend le clic (renvoie 1) seulement si x vaut 3, et donne alors un objet.
    let wat = module(
        r#"
        (func (export "on_use") (param $g i32) (param $x i32) (param $y i32) (param $z i32) (result i32)
            (if (result i32) (i32.eq (local.get $x) (i32.const 3))
                (then (call $give_item (i32.const 1) (i32.const 5)) (i32.const 1))
                (else (i32.const 0))))
        "#,
    );
    let mut script = load(&wat, &s).unwrap();
    let (handled, requests) = script.used(GridPos::world(IVec3::new(3, 0, 0))).unwrap();
    assert!(handled);
    assert_eq!(requests.actions.len(), 1);
    let (handled, requests) = script.used(GridPos::world(IVec3::new(4, 0, 0))).unwrap();
    assert!(!handled && requests.actions.is_empty());
    // Un mod sans `on_use` ne prend rien.
    let mut plain = load(&module(""), &s).unwrap();
    assert!(!plain.used(GridPos::world(IVec3::ZERO)).unwrap().0);
}

#[test]
fn a_script_shows_a_menu_and_hears_the_player() {
    let s = loaded_streamer();
    // À l'événement, le mod montre le menu « label hi » de la machine en (1,2,3) ; quand le
    // joueur presse un bouton (kind 1, id 7), il donne 7 objets.
    let wat = r#"(module
        (import "voxel" "subscribe" (func $subscribe (param i32)))
        (import "voxel" "give_item" (func $give (param i32 i32)))
        (import "voxel" "ui_show" (func $show (param i32 i32 i32 i32 i32 i32)))
        (import "voxel" "ui_close" (func $close))
        (memory (export "memory") 1)
        (data (i32.const 0) "\01\02\00hi")
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param i32 i32 i32)
            (call $show (i32.const 0) (i32.const 1) (i32.const 2) (i32.const 3) (i32.const 0) (i32.const 5))
            (call $close))
        (func (export "on_ui") (param $kind i32) (param $g i32) (param $x i32) (param $y i32) (param $z i32) (param $id i32) (param $v i32)
            (if (i32.eq (local.get $kind) (i32.const 1))
                (then (call $give (i32.const 1) (local.get $id)))))
    )"#;
    let mut script = load(wat, &s).unwrap();
    let requests = script.send_event(&broken()).unwrap();
    let at = GridPos::world(IVec3::new(1, 2, 3));
    assert_eq!(
        requests.actions,
        vec![
            MachineAction::ShowMenu {
                mod_id: "test".into(),
                at,
                data: vec![1, 2, 0, b'h', b'i']
            },
            MachineAction::CloseMenu {
                mod_id: "test".into()
            },
        ]
    );
    let heard = script.ui_event(abi_ui::UI_BUTTON, at, 7, 0).unwrap();
    assert_eq!(
        heard.actions,
        vec![MachineAction::GiveItem {
            block: BlockId(1),
            count: 7
        }]
    );
    // Un clic de menu pour un mod sans `on_ui` : rien.
    let mut plain = load(&module(""), &s).unwrap();
    assert_eq!(
        plain.ui_event(abi_ui::UI_BUTTON, at, 1, 0).unwrap(),
        Requests::default()
    );
}

#[test]
fn a_script_has_an_options_page_and_persistent_settings() {
    let s = loaded_streamer();
    // À l'ouverture (kind 6) le mod montre sa page ; sur un curseur (kind 3) il enregistre le
    // réglage « k » = « v1 », puis relit sa taille (2) pour la donner en objets.
    let wat = r#"(module
        (import "voxel" "give_item" (func $give (param i32 i32)))
        (import "voxel" "options_show" (func $show (param i32 i32)))
        (import "voxel" "setting_set" (func $set (param i32 i32 i32 i32) (result i32)))
        (import "voxel" "setting_get" (func $get (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "\01\02\00hi")
        (data (i32.const 16) "kv1")
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "on_options") (param $kind i32) (param $id i32) (param $v i32)
            (if (i32.eq (local.get $kind) (i32.const 6))
                (then (call $show (i32.const 0) (i32.const 5))))
            (if (i32.eq (local.get $kind) (i32.const 3))
                (then
                    (drop (call $set (i32.const 16) (i32.const 1) (i32.const 17) (i32.const 2)))
                    (call $give (i32.const 1)
                        (call $get (i32.const 16) (i32.const 1) (i32.const 512) (i32.const 8))))))
    )"#;
    let settings = Rc::new(RefCell::new(crate::ModSettings::default()));
    let mut host = ScriptHost::new().unwrap();
    host.set_settings(settings.clone());
    let registry = s.borrow().world().registry().clone();
    let mut script = host
        .load("opt", wat.as_bytes(), s.clone(), registry)
        .unwrap();
    let opened = script.options_event(abi_ui::UI_OPEN, 0, 0).unwrap();
    assert_eq!(
        opened.actions,
        vec![MachineAction::ShowOptions {
            mod_id: "opt".into(),
            data: vec![1, 2, 0, b'h', b'i']
        }]
    );
    let moved = script.options_event(abi_ui::UI_SLIDER, 1, 0).unwrap();
    assert_eq!(settings.borrow().get("opt", "k"), Some("v1"));
    assert!(settings.borrow().is_dirty());
    assert_eq!(
        moved.actions,
        vec![MachineAction::GiveItem {
            block: BlockId(1),
            count: 2
        }]
    );
    // Les réglages sont par mod : un autre mod ne voit pas « k ».
    assert_eq!(settings.borrow().get("autre", "k"), None);
}

/// Un mod qui, à chaque événement, brûle environ 2 millions d'instructions.
fn burner() -> String {
    r#"(module
        (import "voxel" "subscribe" (func $subscribe (param i32)))
        (memory (export "memory") 1)
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "mod_init") (call $subscribe (i32.const 1)))
        (func (export "on_event") (param i32 i32 i32) (local $i i32)
            (loop $l
                (br_if $l (i32.lt_u (local.tee $i (i32.add (local.get $i) (i32.const 1))) (i32.const 400000))))))
    "#
    .to_string()
}

#[test]
fn a_mod_over_its_tick_budget_waits_for_the_next_tick() {
    let s = loaded_streamer();
    let mut script = load(&burner(), &s).unwrap();
    script.set_tick_budget(1_000_000);
    script.send_event(&broken()).unwrap();
    assert!(
        script.over_budget(),
        "~2 M d'instructions pour 1 M de budget"
    );
    // Dans le même tick, l'événement suivant n'est pas traité (et compté).
    let before = script.stats().calls;
    script.send_event(&broken()).unwrap();
    assert_eq!(script.stats().calls, before);
    assert_eq!(script.stats().skipped_calls, 1);
    // Au tick suivant, le budget est rendu.
    script.begin_tick();
    assert!(!script.over_budget());
    assert!(script.stats().fuel_last_tick >= 1_000_000);
    assert_eq!(script.stats().overrun_ticks, 1);
    script.send_event(&broken()).unwrap();
    assert!(script.stats().calls > before, "le script est rappelé");
}

#[test]
fn skipped_machines_are_woken_again_next_tick() {
    let s = loaded_streamer();
    // Un mod qui exporte `on_machines` et brûle ~2 M d'instructions à chaque appel.
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "sdk_alloc") (param i32) (result i32) i32.const 1024)
        (func (export "on_machines") (param i32 i32 i32) (local $i i32)
            (loop $l
                (br_if $l (i32.lt_u (local.tee $i (i32.add (local.get $i) (i32.const 1))) (i32.const 400000))))))"#;
    let mut machines = load(wat, &s).unwrap();
    machines.set_tick_budget(1_000_000);
    let at = [GridPos::world(IVec3::new(1, 2, 3))];
    machines.machines(3, &at).unwrap();
    assert!(machines.over_budget());
    let late = machines.machines(3, &at).unwrap();
    assert_eq!(late.schedules, vec![(at[0], 1)]);
}

#[test]
fn a_mod_over_budget_for_five_seconds_is_disabled() {
    let s = loaded_streamer();
    let mut script = load(&burner(), &s).unwrap();
    script.set_tick_budget(1_000_000);
    for _ in 0..crate::MAX_OVERRUN_TICKS {
        script.send_event(&broken()).unwrap();
        script.begin_tick();
    }
    let reason = script.disabled().expect("désactivé");
    assert!(reason.contains("budget"), "{reason}");
    // Un mod qui reste dans son budget n'est jamais inquiété.
    let mut calm = load(&burner(), &s).unwrap();
    for _ in 0..crate::MAX_OVERRUN_TICKS * 2 {
        calm.send_event(&broken()).unwrap();
        calm.begin_tick();
    }
    assert!(calm.disabled().is_none());
    assert_eq!(calm.stats().overrun_ticks, 0);
}
