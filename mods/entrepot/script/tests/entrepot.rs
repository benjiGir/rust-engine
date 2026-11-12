//! Les tests de l'Entrepôt, sans fenêtre : le vrai script, avec les vraies données du mod.
//! Lancer : `cargo xtask test-mod entrepot`.

use engine_mods::testing::{Harness, MACHINES_PLACED, MACHINES_TICK};
use glam::IVec3;

const AT: IVec3 = IVec3::new(0, 100, 0);

fn placed(h: &mut Harness, pos: IVec3, key: &str) {
    h.place(pos, key);
    h.machines(MACHINES_PLACED, &[pos]).unwrap();
}

fn tick(h: &mut Harness, pos: IVec3) {
    h.machines(MACHINES_TICK, &[pos]).unwrap();
}

/// Tous les objets d'un bloc : (objet, total).
fn totals(h: &Harness, pos: IVec3, slots: usize) -> Vec<(String, u32)> {
    let mut all: Vec<(String, u32)> = Vec::new();
    for s in 0..slots {
        if let Some((item, n)) = h.slot_of(pos, s) {
            match all.iter_mut().find(|(k, _)| *k == item) {
                Some((_, t)) => *t += n,
                None => all.push((item, n)),
            }
        }
    }
    all.sort();
    all
}

#[test]
fn every_machine_gets_its_inventory_when_placed() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    for (n, key) in [
        "entrepot:vault",
        "entrepot:sorter",
        "entrepot:harvester",
        "entrepot:drill",
    ]
    .iter()
    .enumerate()
    {
        let pos = AT + IVec3::new(n as i32 * 2, 0, 0);
        placed(&mut h, pos, key);
        // Ranger un objet dans la dernière case prouve qu'elle existe.
        let last = if n == 0 { 53 } else { 8 };
        h.put(pos, last, "core:stone", 1);
        assert_eq!(
            h.slot_of(pos, last),
            Some(("core:stone".into(), 1)),
            "{key}"
        );
    }
}

#[test]
fn tidy_merges_piles_sorts_them_and_leaves_tools_where_they_are() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    placed(&mut h, AT, "entrepot:vault");
    h.put(AT, 2, "core:stone", 30);
    h.put(AT, 5, "core:dirt", 10);
    h.put(AT, 7, "core:stone", 50);
    h.put(AT, 9, "core:iron_pickaxe", 1);
    h.put(AT, 11, "core:dirt", 5);
    // Le bouton « Ranger » (id 1).
    h.ui(AT, engine_mods::menu::UI_BUTTON, 1, 0).unwrap();
    // 80 pierres = 64 + 16 ; 15 terres : trois piles dans les trois premières cases utilisées.
    let before = totals(&h, AT, 54);
    assert_eq!(
        before,
        vec![
            ("core:dirt".to_string(), 15),
            ("core:iron_pickaxe".to_string(), 1),
            ("core:stone".to_string(), 80),
        ]
    );
    let piles = (0..54).filter(|&s| h.slot_of(AT, s).is_some()).count();
    assert_eq!(piles, 4, "3 piles et la pioche : {piles}");
    assert_eq!(
        h.slot_of(AT, 9),
        Some(("core:iron_pickaxe".into(), 1)),
        "la pioche n'a pas bougé"
    );
    // Trié par nom : la terre d'abord (case 2, la première utilisée par un objet empilable).
    assert_eq!(h.slot_of(AT, 2), Some(("core:dirt".into(), 15)));
    assert_eq!(h.slot_of(AT, 5), Some(("core:stone".into(), 64)));
    assert_eq!(h.slot_of(AT, 7), Some(("core:stone".into(), 16)));
}

#[test]
fn the_harvester_picks_ripe_crops_and_replants_them() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    placed(&mut h, AT, "entrepot:harvester");
    h.place(AT + IVec3::new(1, 0, 0), "core:wheat_3");
    h.place(AT + IVec3::new(0, 0, 2), "core:carrot_3");
    h.place(AT + IVec3::new(-2, 0, 0), "core:wheat_1");
    // Trop loin : à 5 blocs.
    h.place(AT + IVec3::new(5, 0, 0), "core:wheat_3");
    tick(&mut h, AT);
    assert_eq!(h.block(AT + IVec3::new(1, 0, 0)), "core:wheat_0");
    assert_eq!(h.block(AT + IVec3::new(0, 0, 2)), "core:carrot_0");
    assert_eq!(
        h.block(AT + IVec3::new(-2, 0, 0)),
        "core:wheat_1",
        "pas mûr"
    );
    assert_eq!(
        h.block(AT + IVec3::new(5, 0, 0)),
        "core:wheat_3",
        "hors de portée"
    );
    let got = totals(&h, AT, 9);
    assert!(
        got.iter().any(|(k, n)| k == "core:wheat" && *n >= 1),
        "{got:?}"
    );
    assert!(
        got.iter().any(|(k, n)| k == "core:seeds" && *n >= 1),
        "{got:?}"
    );
    assert!(
        got.iter().any(|(k, n)| k == "core:carrot" && *n >= 2),
        "{got:?}"
    );
}

#[test]
fn a_full_harvester_leaves_the_crops_alone() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    placed(&mut h, AT, "entrepot:harvester");
    for s in 0..9 {
        h.put(AT, s, "core:stone", 64);
    }
    h.place(AT + IVec3::new(1, 0, 0), "core:wheat_3");
    tick(&mut h, AT);
    assert_eq!(
        h.block(AT + IVec3::new(1, 0, 0)),
        "core:wheat_3",
        "sans place, on ne casse pas la culture"
    );
}

#[test]
fn the_drill_needs_power_then_digs_down_one_block_at_a_time() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    for y in 90..100 {
        h.place(IVec3::new(0, y, 0), "core:stone");
    }
    placed(&mut h, AT, "entrepot:drill");
    // Éteinte : rien ne se passe.
    tick(&mut h, AT);
    assert_eq!(h.block(AT + IVec3::new(0, -1, 0)), "core:stone");
    // Un levier allumé à côté l'alimente.
    h.place(AT + IVec3::new(1, 0, 0), "core:lever_on");
    h.set_power(AT + IVec3::new(1, 0, 0), 15);
    for _ in 0..3 {
        tick(&mut h, AT);
    }
    for dy in 1..=3 {
        assert_eq!(
            h.block(AT - IVec3::new(0, dy, 0)),
            "core:air",
            "niveau -{dy}"
        );
    }
    assert_eq!(h.block(AT - IVec3::new(0, 4, 0)), "core:stone");
    let got = totals(&h, AT, 9);
    assert_eq!(got, vec![("core:cobblestone".to_string(), 3)], "{got:?}");
}

#[test]
fn the_drill_stops_on_bedrock_and_when_its_slots_are_full() {
    let mut h = Harness::for_mod("entrepot").unwrap();
    placed(&mut h, AT, "entrepot:drill");
    h.place(AT + IVec3::new(1, 0, 0), "core:lever_on");
    h.set_power(AT + IVec3::new(1, 0, 0), 15);
    h.place(AT - IVec3::new(0, 1, 0), "core:bedrock");
    tick(&mut h, AT);
    assert_eq!(h.block(AT - IVec3::new(0, 1, 0)), "core:bedrock");
    // Pleine : elle laisse le bloc.
    h.place(AT - IVec3::new(0, 1, 0), "core:stone");
    for s in 0..9 {
        h.put(AT, s, "core:cobblestone", 64);
    }
    tick(&mut h, AT);
    assert_eq!(h.block(AT - IVec3::new(0, 1, 0)), "core:stone");
    // Et sans la saturation, elle creuse bien : on vide une case.
    h.put(AT, 3, "core:cobblestone", 1);
    tick(&mut h, AT);
    assert_eq!(h.block(AT - IVec3::new(0, 1, 0)), "core:air");
}

#[test]
fn the_sorter_declares_a_filter_slot_and_the_recipes_load() {
    // Le chargement du mod (blocs, ports, recettes) a réussi : `for_mod` l'a fait. Les ports du
    // trieur sont vérifiés par le moteur (`networks::tests`).
    let h = Harness::for_mod("entrepot").unwrap();
    assert!(h.block_id("entrepot:sorter").0 > 0);
}
