//! Les tests de Magie, sans fenêtre : le vrai script dans un petit monde.
//! Lancer : `cargo xtask test-mod magie`.

use engine_mods::ModEvent;
use engine_mods::menu::UI_BUTTON;
use engine_mods::player::PlayerSnapshot;
use engine_mods::testing::{Harness, MACHINES_PLACED};
use engine_voxel::MachineAction;
use glam::{IVec3, Vec3};

/// Un joueur debout au-dessus du sol du monde de test, qui regarde vers le bas.
fn harness() -> Harness {
    let mut h = Harness::for_mod("magie").unwrap();
    h.set_player(PlayerSnapshot {
        eye: Vec3::new(5.5, 110.0, 5.5),
        look: Vec3::NEG_Y,
        health: 10.0,
        max_health: 20.0,
        level: 0.0,
    });
    h
}

fn spell(h: &Harness, text: &str) {
    h.world_data.borrow_mut().set("magie", "spell", text);
}

fn say_contains(actions: &[MachineAction], text: &str) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, MachineAction::Say { text: t } if t.contains(text)))
}

#[test]
fn runes_on_the_table_compose_and_inscribe_a_spell() {
    let mut h = harness();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "magie:rune_table");
    h.machines(MACHINES_PLACED, &[at]).unwrap();
    // Flamme + puissance + portée.
    h.put(at, 0, "magie:rune_flame", 1);
    h.put(at, 1, "magie:rune_power", 1);
    h.put(at, 2, "magie:rune_reach", 1);
    let (taken, opened) = h.use_block(at).unwrap();
    assert!(
        taken
            && opened
                .actions
                .iter()
                .any(|a| matches!(a, MachineAction::ShowMenu { .. }))
    );
    let done = h.ui(at, UI_BUTTON, 1, 0).unwrap();
    assert_eq!(
        h.world_data.borrow().get("magie", "spell"),
        Some("flame+power+reach")
    );
    assert!(say_contains(&done.actions, "Sort inscrit"));
    // Les runes ont servi.
    assert!((0..3).all(|i| h.slot_of(at, i).is_none()));
}

#[test]
fn a_bad_composition_is_refused_and_keeps_the_runes() {
    let mut h = harness();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "magie:rune_table");
    h.machines(MACHINES_PLACED, &[at]).unwrap();
    // Deux runes d'effet : refusé. Seulement un modificateur : refusé.
    h.put(at, 0, "magie:rune_flame", 1);
    h.put(at, 1, "magie:rune_heal", 1);
    let done = h.ui(at, UI_BUTTON, 1, 0).unwrap();
    assert!(say_contains(&done.actions, "rune d'effet"));
    assert_eq!(h.world_data.borrow().get("magie", "spell"), None);
    assert!(h.slot_of(at, 0).is_some(), "les runes restent");
    h.put(at, 0, "magie:rune_power", 1);
    h.put(at, 1, "magie:rune_power", 1);
    assert!(say_contains(
        &h.ui(at, UI_BUTTON, 1, 0).unwrap().actions,
        "rune d'effet"
    ));
}

#[test]
fn the_wand_throws_a_flame_where_the_player_looks_and_costs_mana() {
    let mut h = harness();
    spell(&h, "flame+power");
    let requests = h.use_item("magie:wand").unwrap();
    let hurt = requests
        .actions
        .iter()
        .find_map(|a| match a {
            MachineAction::HurtArea {
                center,
                radius,
                damage,
            } => Some((*center, *radius, *damage)),
            _ => None,
        })
        .expect("une explosion");
    // Le regard descend vers le sol : le point d'impact est en dessous des yeux, dans la portée.
    assert!(
        (hurt.0.x - 5.5).abs() < 0.01 && hurt.0.y < 110.0 && hurt.0.y > 90.0,
        "{:?}",
        hurt.0
    );
    // Puissance 1 : +60 % de dégâts et un rayon plus grand.
    assert_eq!((hurt.1, hurt.2), (3.0, 8.0));
    // Le livre de quêtes est prévenu, dans l'espace de noms du mod.
    assert!(requests.actions.contains(&MachineAction::QuestSignal {
        id: "magie:spell_cast".into()
    }));
    // 12 de base + 6 pour la puissance : 50 - 18 = 32, et c'est sauvegardé.
    assert_eq!(h.world_data.borrow().get("magie", "mana"), Some("32.0"));
}

#[test]
fn without_enough_mana_or_without_a_spell_nothing_is_cast() {
    let mut h = harness();
    let none = h.use_item("magie:wand").unwrap();
    assert!(say_contains(&none.actions, "Aucun sort"));
    spell(&h, "heal");
    h.world_data.borrow_mut().set("magie", "mana", "3");
    let poor = h.use_item("magie:wand").unwrap();
    assert!(say_contains(&poor.actions, "Pas assez de mana"));
    assert!(
        !poor
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::PlayerHeal { .. }))
    );
    // Un sort qui ne part pas n'envoie pas de signal.
    assert!(
        !poor
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::QuestSignal { .. }))
    );
    assert_eq!(
        h.world_data.borrow().get("magie", "mana"),
        Some("3"),
        "rien n'est dépensé"
    );
}

#[test]
fn heal_and_blink_act_on_the_player() {
    let mut h = harness();
    spell(&h, "heal+reach");
    let healed = h.use_item("magie:wand").unwrap();
    assert!(
        healed
            .actions
            .contains(&MachineAction::PlayerHeal { amount: 5.0 })
    );
    assert!(healed.actions.contains(&MachineAction::PlayerEffect {
        name: "regeneration".into(),
        seconds: 4.0,
        power: 1.0
    }));
    // Le saut : vers le point visé, un peu avant le sol, les pieds sous les yeux.
    spell(&h, "blink+power");
    let blinked = h.use_item("magie:wand").unwrap();
    let to = blinked
        .actions
        .iter()
        .find_map(|a| match a {
            MachineAction::PlayerTeleport { position } => Some(*position),
            _ => None,
        })
        .expect("une téléportation");
    assert!((to.x - 5.5).abs() < 0.01 && to.y < 110.0 - 1.62);
    assert!(
        blinked
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::PlayerEffect { name, .. } if name == "speed"))
    );
}

#[test]
fn the_crystal_restores_mana_and_is_used_up() {
    let mut h = harness();
    h.world_data.borrow_mut().set("magie", "mana", "10");
    let used = h.use_item("magie:mana_crystal").unwrap();
    assert!(
        used.actions
            .contains(&MachineAction::ConsumeHeld { count: 1 })
    );
    assert_eq!(h.world_data.borrow().get("magie", "mana"), Some("50.0"));
    // Au maximum : le surplus est perdu (50 à l'expérience 0).
    h.use_item("magie:mana_crystal").unwrap();
    assert_eq!(h.world_data.borrow().get("magie", "mana"), Some("50.0"));
}

#[test]
fn mana_regenerates_with_ticks_and_the_hud_bar_follows() {
    let mut h = harness();
    h.world_data.borrow_mut().set("magie", "mana", "10");
    let first = h.event(&ModEvent::Tick(1)).unwrap();
    // Dès le premier tick la jauge est dessinée.
    let bar = |actions: &[MachineAction]| {
        actions.iter().find_map(|a| match a {
            MachineAction::HudBar {
                label,
                fraction,
                mod_id,
                id,
                ..
            } => Some((mod_id.clone(), *id, label.clone(), *fraction)),
            _ => None,
        })
    };
    let (mod_id, id, label, fraction) = bar(&first.actions).expect("la jauge de mana");
    assert_eq!((mod_id.as_str(), id), ("magie", 0));
    assert_eq!(label, "Mana 10/50");
    assert!((fraction - 0.2).abs() < 1e-4);
    // 1 mana par seconde, 20 ticks par seconde : 40 ticks rendent 2 points.
    let mut last = None;
    for t in 2..=40 {
        let r = h.event(&ModEvent::Tick(t)).unwrap();
        last = bar(&r.actions).or(last);
        h.next_tick();
    }
    assert_eq!(last.unwrap().2, "Mana 12/50");
    // Le niveau d'expérience monte le maximum : +10 par niveau.
    h.set_player(PlayerSnapshot {
        level: 3.0,
        ..PlayerSnapshot::default()
    });
    let mut shown = None;
    for t in 41..=50 {
        shown = bar(&h.event(&ModEvent::Tick(t)).unwrap().actions).or(shown);
    }
    assert!(shown.unwrap().2.ends_with("/80"));
}

#[test]
fn the_commands_report_mana_and_spell() {
    let mut h = harness();
    spell(&h, "blink");
    let (mana, _) = h.command("mana", "").unwrap();
    assert_eq!(mana.as_deref(), Some("Mana : 50/50"));
    let (sort, _) = h.command("sort", "").unwrap();
    assert!(sort.unwrap().contains("Saut"));
    // Le mana sauvegardé du monde est repris.
    h.world_data.borrow_mut().set("magie", "mana", "7");
    let mut fresh = Harness::for_mod("magie").unwrap();
    fresh.world_data.borrow_mut().set("magie", "mana", "7");
    assert_eq!(
        fresh.command("mana", "").unwrap().0.as_deref(),
        Some("Mana : 7/50")
    );
}

#[test]
fn the_sort_command_can_inscribe_a_spell_without_runes() {
    let mut h = harness();
    assert!(
        h.command("sort", "flame+power")
            .unwrap()
            .0
            .unwrap()
            .contains("Flamme")
    );
    assert_eq!(
        h.world_data.borrow().get("magie", "spell"),
        Some("flame+power")
    );
    assert!(
        h.command("sort", "nonsense")
            .unwrap()
            .0
            .unwrap()
            .contains("inconnu")
    );
}
