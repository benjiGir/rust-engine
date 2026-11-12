//! Les tests de Mécanique, sans fenêtre : le vrai script dans un petit monde.
//! Lancer : `cargo xtask test-mod mecanique`.

use engine_mods::menu::UI_REFRESH;
use engine_mods::testing::{Harness, MACHINES_PLACED, MACHINES_TICK};
use engine_voxel::MachineAction;
use glam::IVec3;

const AT: IVec3 = IVec3::new(0, 100, 0);

fn press() -> Harness {
    let mut h = Harness::for_mod("mecanique").unwrap();
    h.place(AT, "mecanique:press");
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
    h
}

/// Fait tourner la presse `ticks` ticks (en la réveillant tant qu'elle demande à l'être).
fn run(h: &mut Harness, ticks: u32) {
    for _ in 0..ticks {
        let requests = h.machines(MACHINES_TICK, &[AT]).unwrap();
        if requests.schedules.is_empty() {
            break;
        }
    }
}

#[test]
fn the_press_turns_an_ingot_into_a_plate_when_the_shaft_spins() {
    let mut h = press();
    h.put(AT, 0, "core:iron_ingot", 3);
    // À 32 tr/min la presse avance de 2 ticks de recette par tick de jeu : la recette (2 s =
    // 40 ticks de recette) dure donc 20 ticks.
    h.set_speed(AT, 32.0);
    run(&mut h, 20);
    assert_eq!(h.slot_of(AT, 0), Some(("core:iron_ingot".into(), 2)));
    assert_eq!(h.slot_of(AT, 1), Some(("mecanique:iron_plate".into(), 1)));
    run(&mut h, 20);
    assert_eq!(h.slot_of(AT, 1), Some(("mecanique:iron_plate".into(), 2)));
}

#[test]
fn without_rotation_nothing_happens_and_the_press_sleeps() {
    let mut h = press();
    h.put(AT, 0, "core:iron_ingot", 1);
    let requests = h.machines(MACHINES_TICK, &[AT]).unwrap();
    assert!(requests.schedules.is_empty(), "la presse s'endort");
    assert_eq!(h.slot_of(AT, 1), None);
}

#[test]
fn a_full_output_blocks_the_press_and_a_different_item_too() {
    let mut h = press();
    h.set_speed(AT, 64.0);
    h.put(AT, 0, "core:iron_ingot", 1);
    h.put(AT, 1, "mecanique:copper_plate", 3);
    run(&mut h, 100);
    assert_eq!(
        h.slot_of(AT, 0),
        Some(("core:iron_ingot".into(), 1)),
        "sortie occupée par autre chose"
    );
    h.put(AT, 1, "mecanique:iron_plate", 64);
    run(&mut h, 100);
    assert_eq!(
        h.slot_of(AT, 0),
        Some(("core:iron_ingot".into(), 1)),
        "sortie pleine"
    );
}

#[test]
fn an_item_without_a_recipe_is_left_alone() {
    let mut h = press();
    h.set_speed(AT, 64.0);
    h.put(AT, 0, "core:stick", 5);
    run(&mut h, 50);
    assert_eq!(h.slot_of(AT, 0), Some(("core:stick".into(), 5)));
    assert_eq!(h.slot_of(AT, 1), None);
}

#[test]
fn the_crank_spins_on_right_click_then_stops() {
    let mut h = Harness::for_mod("mecanique").unwrap();
    h.place(AT, "mecanique:crank");
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
    assert_eq!(h.speed_of(AT), 0.0);
    let (taken, requests) = h.use_block(AT).unwrap();
    assert!(taken);
    assert_eq!(h.speed_of(AT), 32.0, "vitesse par défaut");
    // Le tour dure 5 s = 100 ticks : un tick programmé le termine.
    assert_eq!(
        requests.schedules,
        vec![(engine_voxel::GridPos::world(AT), 100)]
    );
    h.machines(MACHINES_TICK, &[AT]).unwrap();
    assert_eq!(h.speed_of(AT), 0.0);
}

#[test]
fn the_crank_follows_the_options_page() {
    let mut h = Harness::for_mod("mecanique").unwrap();
    h.place(AT, "mecanique:crank");
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
    h.settings
        .borrow_mut()
        .set("mecanique", "crank_speed", "100");
    h.settings
        .borrow_mut()
        .set("mecanique", "crank_seconds", "2");
    let (_, requests) = h.use_block(AT).unwrap();
    assert_eq!(h.speed_of(AT), 100.0);
    assert_eq!(requests.schedules[0].1, 40);
}

#[test]
fn the_press_menu_shows_slots_and_refreshes() {
    let mut h = press();
    let (taken, requests) = h.use_block(AT).unwrap();
    assert!(taken);
    assert!(
        requests
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::ShowMenu { mod_id, .. } if mod_id == "mecanique"))
    );
    let again = h.ui(AT, UI_REFRESH, 0, 0).unwrap();
    assert!(
        again
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::ShowMenu { .. }))
    );
}

#[test]
fn shafts_and_gears_are_variant_conduits_in_boxes() {
    let h = Harness::for_mod("mecanique").unwrap();
    for key in ["mecanique:shaft", "mecanique:gear"] {
        let def = h.registry.def(h.block_id(key));
        assert_eq!(def.conduit, Some(engine_voxel::NetworkKind::Kinetic));
        assert_eq!(def.variants.len(), 3, "axes Y, X, Z");
        assert!(!def.boxes.is_empty());
        let x = h.registry.def(h.block_id(&format!("{key}[axis=x]")));
        assert_eq!(x.conduit, Some(engine_voxel::NetworkKind::Kinetic));
    }
}

#[test]
fn the_world_remembers_crank_turns_and_other_mods_can_ask() {
    let mut h = Harness::for_mod("mecanique").unwrap();
    h.place(AT, "mecanique:crank");
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
    for _ in 0..3 {
        h.use_block(AT).unwrap();
    }
    // La donnée est dans le monde de ce mod (sauvegardée avec lui par le jeu).
    assert_eq!(
        h.world_data.borrow().get("mecanique", "crank_turns"),
        Some("3")
    );
    // Un autre mod peut la demander, ainsi que la vitesse réglée.
    let turns = h.call("crank_turns", b"").unwrap().unwrap();
    assert_eq!(u32::from_le_bytes(turns.try_into().unwrap()), 3);
    let speed = h.call("crank_speed", b"").unwrap().unwrap();
    assert_eq!(f32::from_le_bytes(speed.try_into().unwrap()), 32.0);
    assert_eq!(h.call("inconnu", b"").unwrap(), None);
}

/// Le monde généré avec les mods contient les ateliers de Mécanique, tels que décrits par
/// `structures.ron`.
#[test]
fn the_generator_places_workshops_with_their_machines_and_loot() {
    use engine_voxel::{StructureKind, TerrainGenerator};
    let h = Harness::for_mod("mecanique").unwrap();
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs: Vec<(String, std::path::PathBuf)> = ["example", "mecanique"]
        .iter()
        .map(|m| (m.to_string(), mods.join(m)))
        .collect();
    let generator = TerrainGenerator::with_mods(7, h.registry.clone(), &dirs).unwrap();
    let site = generator
        .structure_sites_in((-3000, -3000), (3000, 3000))
        .into_iter()
        .find(|s| s.kind == StructureKind::Template)
        .expect("un atelier quelque part");
    let plan = generator.blueprint(&site).unwrap();
    let press = h.block_id("mecanique:press");
    let crank = h.block_id("mecanique:crank");
    assert!(plan.blocks().iter().any(|&(_, b)| b == press), "la presse");
    assert!(
        plan.blocks().iter().any(|&(_, b)| b == crank),
        "la manivelle"
    );
    // Le plan n'est pas tourné : la chaîne reste alignée sur X, et le coffre a son butin.
    let [(chest, inventory)] = plan.chests() else {
        panic!("un coffre");
    };
    assert!(inventory.slots.iter().flatten().count() >= 2);
    assert!(chest.y > 0);
    // Les rubis de l'exemple sont dans le générateur (un filon par défaut ne suffit pas à le
    // prouver : on regarde la liste des minerais).
    let ruby = h.block_id("example:ruby");
    assert!(generator.biomes().ores().iter().any(|o| o.block == ruby));
}

#[test]
fn the_commands_are_declared_and_answer() {
    let mut h = Harness::for_mod("mecanique").unwrap();
    let names: Vec<String> = h.commands().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["fonderie", "retour", "tours", "manivelle"]);
    let (reply, _) = h.command("tours", "").unwrap();
    assert!(reply.unwrap().contains("0 fois"));
    // `/manivelle 100` règle la vitesse de la manivelle, comme la page d'options.
    let (reply, _) = h.command("manivelle", "100").unwrap();
    assert!(reply.unwrap().contains("100"));
    assert_eq!(
        h.settings.borrow().get("mecanique", "crank_speed"),
        Some("100")
    );
    assert!(
        h.command("manivelle", "9999")
            .unwrap()
            .0
            .unwrap()
            .contains("1 à 256")
    );
    assert!(
        h.command("manivelle", "abc")
            .unwrap()
            .0
            .unwrap()
            .contains("Usage")
    );
    assert_eq!(h.command("inconnue", "").unwrap().0, None);
}

#[test]
fn belts_are_item_conduits_and_the_press_declares_its_ports() {
    let h = Harness::for_mod("mecanique").unwrap();
    let belt = h.registry.def(h.block_id("mecanique:belt"));
    assert_eq!(belt.conduit, Some(engine_voxel::NetworkKind::Items));
    assert_eq!(belt.channel.as_deref(), Some("mecanique:belt"));
    let ports = h
        .registry
        .def(h.block_id("mecanique:press"))
        .ports
        .clone()
        .unwrap();
    assert_eq!((ports.inputs, ports.outputs), (vec![0], vec![1]));
    // Seuls les objets que ses recettes consomment entrent : le type de recette est le sien.
    assert_eq!(ports.accepts.as_deref(), Some("mecanique:press"));
}

#[test]
fn the_foundry_dimension_and_its_portals_are_declared() {
    use engine_voxel::{BiomeRegistry, DimensionRegistry, WorldKind};
    let h = Harness::for_mod("mecanique").unwrap();
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs: Vec<(String, std::path::PathBuf)> = ["example", "mecanique"]
        .iter()
        .map(|m| (m.to_string(), mods.join(m)))
        .collect();
    let biomes = BiomeRegistry::load_mods(&h.registry, &dirs).unwrap();
    let dimensions = DimensionRegistry::load_mods(&h.registry, &biomes, &dirs).unwrap();
    let foundry = dimensions.get("mecanique:foundry").expect("la Fonderie");
    assert_eq!(foundry.gravity, 0.6);
    assert_eq!(foundry.seed_offset, 777);
    assert!(matches!(&foundry.kind, WorldKind::Terrain { biomes } if biomes.len() == 1));
    // Le portail de départ mène à la Fonderie, celui d'arrivée ramène ; les deux se traversent.
    let gate = h.registry.def(h.block_id("mecanique:gate"));
    assert_eq!(gate.portal.as_deref(), Some("mecanique:foundry"));
    assert!(!gate.solid);
    assert_eq!(foundry.arrival, Some(h.block_id("mecanique:return_gate")));
    let back = h.registry.def(h.block_id("mecanique:return_gate"));
    assert_eq!(back.portal.as_deref(), Some("core:overworld"));
}

#[test]
fn the_travel_commands_ask_the_game_to_change_dimension() {
    use engine_voxel::MachineAction;
    let mut h = Harness::for_mod("mecanique").unwrap();
    let (reply, requests) = h.command("fonderie", "").unwrap();
    assert_eq!(reply.as_deref(), Some("En route…"));
    assert_eq!(
        requests.actions,
        vec![MachineAction::Travel {
            dimension: "mecanique:foundry".into()
        }]
    );
    // Dans le monde de départ, `/retour` n'a rien à faire.
    let (reply, requests) = h.command("retour", "").unwrap();
    assert_eq!(reply.as_deref(), Some("Vous y êtes déjà."));
    assert!(requests.actions.is_empty());
}
