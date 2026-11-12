//! Les tests de la Forge, sans fenêtre : le vrai script, avec les vraies données du mod.
//! Lancer : `cargo xtask test-mod forge`.

use engine_mods::menu::UI_BUTTON;
use engine_mods::testing::{Harness, MACHINES_PLACED, MACHINES_TICK};
use glam::IVec3;

const AT: IVec3 = IVec3::new(0, 100, 0);

fn placed(h: &mut Harness, key: &str) {
    h.place(AT, key);
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
}

fn tick(h: &mut Harness, n: usize) {
    for _ in 0..n {
        h.machines(MACHINES_TICK, &[AT]).unwrap();
    }
}

#[test]
fn the_furnace_burns_coal_to_make_steel() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:alloy_furnace");
    h.put(AT, 0, "core:iron_ingot", 4);
    h.put(AT, 1, "core:coal", 2);
    h.put(AT, 3, "core:coal", 1);
    // Le temps de la recette : 10 s = 200 ticks, plus l'allumage.
    tick(&mut h, 150);
    assert_eq!(h.slot_of(AT, 4), None, "pas encore fini");
    tick(&mut h, 80);
    assert_eq!(h.slot_of(AT, 4), Some(("forge:steel_ingot".into(), 1)));
    assert_eq!(h.slot_of(AT, 0), Some(("core:iron_ingot".into(), 2)));
    assert_eq!(h.slot_of(AT, 1), Some(("core:coal".into(), 1)));
    assert_eq!(h.slot_of(AT, 3), None, "le charbon de chauffe est consommé");
    // Le feu tient encore : la seconde fusion ne demande pas de combustible.
    tick(&mut h, 220);
    assert_eq!(h.slot_of(AT, 4), Some(("forge:steel_ingot".into(), 2)));
}

#[test]
fn without_fuel_nothing_melts() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:alloy_furnace");
    h.put(AT, 0, "core:iron_ingot", 2);
    h.put(AT, 1, "core:coal", 1);
    tick(&mut h, 400);
    assert_eq!(h.slot_of(AT, 4), None);
    assert_eq!(h.slot_of(AT, 0), Some(("core:iron_ingot".into(), 2)));
}

#[test]
fn copper_and_malachite_make_three_bronze_ingots() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:alloy_furnace");
    h.put(AT, 0, "core:copper_ingot", 3);
    h.put(AT, 2, "core:malachite", 1);
    h.put(AT, 3, "core:charcoal", 1);
    tick(&mut h, 200);
    assert_eq!(h.slot_of(AT, 4), Some(("forge:bronze_ingot".into(), 3)));
    assert_eq!(h.slot_of(AT, 0), None);
}

#[test]
fn a_full_output_stops_the_furnace() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:alloy_furnace");
    h.put(AT, 0, "core:iron_ingot", 2);
    h.put(AT, 1, "core:coal", 1);
    h.put(AT, 3, "core:coal", 1);
    h.put(AT, 4, "forge:steel_ingot", 64);
    tick(&mut h, 300);
    assert_eq!(h.slot_of(AT, 4), Some(("forge:steel_ingot".into(), 64)));
    assert_eq!(
        h.slot_of(AT, 3),
        Some(("core:coal".into(), 1)),
        "pas de gaspillage"
    );
}

fn forge_button(h: &mut Harness, button: i32) {
    h.ui(AT, UI_BUTTON, button, 0).unwrap();
}

#[test]
fn the_anvil_needs_its_hammer_then_forges_each_piece() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:anvil");
    h.put(AT, 0, "forge:steel_ingot", 8);
    // Sans marteau : rien.
    forge_button(&mut h, 1);
    assert_eq!(h.slot_of(AT, 3), None);
    h.put(AT, 2, "forge:bronze_hammer", 1);
    forge_button(&mut h, 1);
    assert_eq!(h.slot_of(AT, 3), Some(("forge:steel_plate".into(), 1)));
    assert_eq!(h.slot_of(AT, 0), Some(("forge:steel_ingot".into(), 6)));
    assert_eq!(
        h.slot_of(AT, 2),
        Some(("forge:bronze_hammer".into(), 1)),
        "le marteau n'est pas consommé"
    );
    // Une autre pièce, sortie occupée par une plaque : refusé.
    forge_button(&mut h, 2);
    assert_eq!(h.slot_of(AT, 3), Some(("forge:steel_plate".into(), 1)));
    h.clear_slot(AT, 3);
    forge_button(&mut h, 2);
    assert_eq!(h.slot_of(AT, 3), Some(("forge:pickaxe_head".into(), 1)));
    assert_eq!(h.slot_of(AT, 0), Some(("forge:steel_ingot".into(), 3)));
}

#[test]
fn the_anvil_refuses_when_ingots_are_missing() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:anvil");
    h.put(AT, 0, "forge:steel_ingot", 2);
    h.put(AT, 2, "forge:bronze_hammer", 1);
    forge_button(&mut h, 3);
    assert_eq!(h.slot_of(AT, 3), None, "une tête de hache en demande 3");
    forge_button(&mut h, 4);
    assert_eq!(h.slot_of(AT, 3), Some(("forge:sword_blade".into(), 1)));
}

#[test]
fn the_blocks_load() {
    let h = Harness::for_mod("forge").unwrap();
    for key in ["forge:alloy_furnace", "forge:anvil"] {
        assert!(h.block_id(key).0 > 0, "{key}");
    }
}

#[test]
fn every_forging_wears_the_hammer_until_it_breaks() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:anvil");
    h.put(AT, 0, "forge:steel_ingot", 6);
    h.put(AT, 2, "forge:bronze_hammer", 1);
    forge_button(&mut h, 1);
    assert_eq!(h.damage_of(AT, 2), Some(1));
    // Presque cassé : 199 usures sur 200, le prochain coup le brise.
    h.set_damage(AT, 2, 199);
    h.clear_slot(AT, 3);
    forge_button(&mut h, 1);
    assert_eq!(h.slot_of(AT, 2), None, "le marteau s'est brisé");
    assert_eq!(
        h.slot_of(AT, 3),
        Some(("forge:steel_plate".into(), 1)),
        "la pièce est faite"
    );
}

#[test]
fn the_anvil_repairs_a_quarter_of_the_durability_per_ingot() {
    let mut h = Harness::for_mod("forge").unwrap();
    placed(&mut h, "forge:anvil");
    h.put(AT, 0, "forge:steel_pickaxe", 1);
    h.set_damage(AT, 0, 500);
    h.put(AT, 1, "forge:steel_ingot", 2);
    h.put(AT, 2, "forge:bronze_hammer", 1);
    forge_button(&mut h, 5);
    // La pioche d'acier tient 700 usages : un quart = 175.
    assert_eq!(h.damage_of(AT, 0), Some(325));
    assert_eq!(h.slot_of(AT, 1), Some(("forge:steel_ingot".into(), 1)));
    assert_eq!(h.damage_of(AT, 2), Some(1), "la réparation use le marteau");
    // Sans usure, ou sans lingot : rien n'est consommé.
    h.set_damage(AT, 0, 0);
    forge_button(&mut h, 5);
    assert_eq!(h.slot_of(AT, 1), Some(("forge:steel_ingot".into(), 1)));
    h.set_damage(AT, 0, 100);
    h.clear_slot(AT, 1);
    forge_button(&mut h, 5);
    assert_eq!(h.damage_of(AT, 0), Some(100));
}
