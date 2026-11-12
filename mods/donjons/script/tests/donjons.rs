//! Les tests de Donjons, sans fenêtre : le vrai script, et le vrai générateur avec les données du mod.
//! Lancer : `cargo xtask test-mod donjons`.

use engine_mods::ModEvent;
use engine_mods::mobs::MobSnapshot;
use engine_mods::player::PlayerSnapshot;
use engine_mods::testing::{Harness, MACHINES_PLACED, MACHINES_TICK};
use engine_voxel::MachineAction::{self, MobAttack, MobMove, MobState};
use glam::{IVec3, Vec3};

const AT: IVec3 = IVec3::new(0, 100, 0);

/// Un joueur dont les yeux sont en `eye`.
fn player(h: &mut Harness, eye: Vec3) {
    h.set_player(PlayerSnapshot {
        eye,
        ..PlayerSnapshot::default()
    });
}

fn placed(h: &mut Harness, key: &str) {
    h.place(AT, key);
    h.machines(MACHINES_PLACED, &[AT]).unwrap();
}

#[test]
fn spikes_hurt_the_player_who_stands_on_them_and_only_then() {
    let mut h = Harness::for_mod("donjons").unwrap();
    placed(&mut h, "donjons:spike_trap");
    // Debout sur la case (les yeux 1,62 au-dessus des pieds, qui sont à y = 100).
    player(&mut h, Vec3::new(0.5, 101.62, 0.5));
    let hit = h.machines(MACHINES_TICK, &[AT]).unwrap();
    assert!(
        hit.actions
            .contains(&MachineAction::PlayerHurt { amount: 3.0 })
    );
    assert!(!hit.schedules.is_empty(), "le piège se réarme");
    // À côté, ou en l'air : rien.
    player(&mut h, Vec3::new(4.5, 101.62, 0.5));
    assert!(h.machines(MACHINES_TICK, &[AT]).unwrap().actions.is_empty());
    player(&mut h, Vec3::new(0.5, 106.0, 0.5));
    assert!(h.machines(MACHINES_TICK, &[AT]).unwrap().actions.is_empty());
}

#[test]
fn a_spawner_waits_for_the_player_then_spawns_three_prowlers_and_vanishes() {
    let mut h = Harness::for_mod("donjons").unwrap();
    placed(&mut h, "donjons:spawner");
    player(&mut h, Vec3::new(40.5, 101.0, 0.5));
    let far = h.machines(MACHINES_TICK, &[AT]).unwrap();
    assert!(
        far.actions.is_empty() && !far.schedules.is_empty(),
        "il attend"
    );
    assert_eq!(h.block(AT), "donjons:spawner");
    player(&mut h, Vec3::new(6.5, 101.0, 0.5));
    let near = h.machines(MACHINES_TICK, &[AT]).unwrap();
    let spawned: Vec<&str> = near
        .actions
        .iter()
        .filter_map(|a| match a {
            MachineAction::SpawnEntity { kind, .. } => Some(kind.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(spawned, ["core:prowler"; 3]);
    assert_eq!(h.block(AT), "core:air", "la cage s'efface");
    assert!(near.schedules.is_empty());
}

#[test]
fn the_boss_seal_summons_the_warden_once() {
    let mut h = Harness::for_mod("donjons").unwrap();
    placed(&mut h, "donjons:boss_spawner");
    player(&mut h, Vec3::new(5.5, 101.0, 0.5));
    let r = h.machines(MACHINES_TICK, &[AT]).unwrap();
    assert!(
        r.actions.iter().any(
            |a| matches!(a, MachineAction::SpawnEntity { kind, .. } if kind == "donjons:warden")
        )
    );
    assert_eq!(h.block(AT), "core:air");
}

fn warden(h: &Harness, to: [f32; 3], health: f32, state: u32) -> MobSnapshot {
    MobSnapshot {
        id: 9,
        kind: h.mob_kind("donjons:warden"),
        health,
        max_health: 80.0,
        to_player: to,
        position: [10.0, 70.0, 10.0],
        state,
        ..Default::default()
    }
}

#[test]
fn the_warden_chases_charges_strikes_and_calls_reinforcements_at_half_health() {
    let mut h = Harness::for_mod("donjons").unwrap();
    // Loin : il ne bouge pas.
    let far = h.mobs(&[warden(&h, [60.0, 0.0, 0.0], 80.0, 0)]).unwrap();
    assert!(far.actions.contains(&MobMove {
        mob: 9,
        dx: 0.0,
        dz: 0.0,
        speed: 0.0
    }));
    // À 10 blocs : il poursuit, puis passe en charge (phase 2, compteur de 5 dans les bits hauts).
    let chase = h.mobs(&[warden(&h, [10.0, 0.0, 0.0], 80.0, 1)]).unwrap();
    assert!(chase.actions.contains(&MobMove {
        mob: 9,
        dx: 10.0,
        dz: 0.0,
        speed: 2.6
    }));
    assert!(chase.actions.contains(&MobState {
        mob: 9,
        value: 2 | (5 << 4)
    }));
    // En charge, il fonce plus vite et décompte.
    let charge = h
        .mobs(&[warden(&h, [10.0, 0.0, 0.0], 80.0, 2 | (5 << 4))])
        .unwrap();
    assert!(charge.actions.contains(&MobMove {
        mob: 9,
        dx: 10.0,
        dz: 0.0,
        speed: 7.0
    }));
    assert!(charge.actions.contains(&MobState {
        mob: 9,
        value: 2 | (4 << 4)
    }));
    // Au contact : un coup, et retour à la poursuite avec un délai.
    let hit = h
        .mobs(&[warden(&h, [2.0, 0.0, 0.0], 80.0, 2 | (3 << 4))])
        .unwrap();
    assert!(hit.actions.contains(&MobAttack {
        mob: 9,
        damage: 8.0
    }));
    assert!(hit.actions.contains(&MobState {
        mob: 9,
        value: 1 | (6 << 4)
    }));
    // Poursuivant de près, sans charge : un coup normal.
    let close = h.mobs(&[warden(&h, [2.0, 0.0, 0.0], 80.0, 1)]).unwrap();
    assert!(close.actions.contains(&MobAttack {
        mob: 9,
        damage: 6.0
    }));
    // Sous la moitié de sa vie, une seule fois : trois renforts.
    let low = h.mobs(&[warden(&h, [10.0, 0.0, 0.0], 30.0, 1)]).unwrap();
    let reinforcements = low
        .actions
        .iter()
        .filter(|a| matches!(a, MachineAction::SpawnEntity { kind, .. } if kind == "core:prowler"))
        .count();
    assert_eq!(reinforcements, 3);
    assert!(low.actions.contains(&MobState {
        mob: 9,
        value: 1 | (1 << 12)
    }));
    let again = h
        .mobs(&[warden(&h, [10.0, 0.0, 0.0], 30.0, 1 | (1 << 12))])
        .unwrap();
    assert!(
        !again
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::SpawnEntity { .. }))
    );
}

#[test]
fn the_boss_bar_follows_its_health_and_goes_when_it_is_gone() {
    let mut h = Harness::for_mod("donjons").unwrap();
    let bar = |actions: &[MachineAction]| {
        actions.iter().find_map(|a| match a {
            MachineAction::HudBar {
                label, fraction, ..
            } => Some((label.clone(), *fraction)),
            _ => None,
        })
    };
    let shown = h.mobs(&[warden(&h, [10.0, 0.0, 0.0], 40.0, 1)]).unwrap();
    let (label, fraction) = bar(&shown.actions).expect("la jauge du boss");
    assert!(label.contains("40/80") && (fraction - 0.5).abs() < 1e-4);
    // Plus aucun gardien dans le lot : la jauge est retirée, une seule fois.
    let gone = h.mobs(&[]).unwrap();
    assert!(
        gone.actions
            .iter()
            .any(|a| matches!(a, MachineAction::HudClear { .. }))
    );
    assert!(h.mobs(&[]).unwrap().actions.is_empty());
}

#[test]
fn a_dead_warden_leaves_a_treasure_chest() {
    let mut h = Harness::for_mod("donjons").unwrap();
    let at = IVec3::new(3, 100, 3);
    h.event(&ModEvent::EntityDied {
        kind: "donjons:warden".into(),
        position: at.as_vec3(),
    })
    .unwrap();
    assert_eq!(h.block(at), "core:chest");
    assert_eq!(h.slot_of(at, 0), Some(("core:gold_ingot".into(), 12)));
    assert_eq!(h.slot_of(at, 1), Some(("core:sapphire".into(), 4)));
    // Un autre mort ne laisse rien.
    let other = IVec3::new(8, 100, 8);
    h.event(&ModEvent::EntityDied {
        kind: "core:pig".into(),
        position: other.as_vec3(),
    })
    .unwrap();
    assert_eq!(h.block(other), "core:air");
}

/// Le vrai générateur, avec les données du mod : une crypte a son escalier, ses salles et ses
/// pièges, ses sceaux et ses coffres.
#[test]
fn the_generator_builds_crypts_with_traps_spawners_a_boss_seal_and_chests() {
    use engine_voxel::{StructureKind, TerrainGenerator};
    let h = Harness::for_mod("donjons").unwrap();
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs: Vec<(String, std::path::PathBuf)> = ["donjons", "example", "magie", "mecanique"]
        .iter()
        .map(|m| (m.to_string(), mods.join(m)))
        .collect();
    let generator = TerrainGenerator::with_mods(7, h.registry.clone(), &dirs).unwrap();
    let site = generator
        .structure_sites_in((-6000, -6000), (6000, 6000))
        .into_iter()
        .find(|s| s.kind == StructureKind::Graph)
        .expect("une crypte quelque part");
    let plan = generator.blueprint(&site).expect("son plan");
    let count = |key: &str| {
        let id = h.block_id(key);
        plan.blocks().iter().filter(|&&(_, b)| b == id).count()
    };
    assert!(count("donjons:spike_trap") >= 4, "des pièges");
    assert_eq!(count("donjons:boss_spawner"), 1, "un sceau de boss");
    assert!(count("donjons:crypt_brick") > 500, "des murs");
    // Les salles ordinaires ont une cage ou des pièges ; au moins une entrée et un boss : ≥ 4 salles
    // avec coffre.
    assert!(plan.chests().len() >= 3, "{} coffres", plan.chests().len());
    // Le même site donne toujours le même donjon.
    assert_eq!(generator.blueprint(&site).unwrap().blocks(), plan.blocks());
}

/// Les machines posées par le monde (une cage, un piège) reçoivent un état : sans lui, le jeu ne les
/// réveillerait jamais au chargement du chunk.
#[test]
fn machines_placed_by_the_generator_have_a_state_so_the_game_wakes_them() {
    use engine_voxel::{StructureKind, TerrainGenerator, World};
    let h = Harness::for_mod("donjons").unwrap();
    let mods = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dirs: Vec<(String, std::path::PathBuf)> = ["donjons", "example", "magie", "mecanique"]
        .iter()
        .map(|m| (m.to_string(), mods.join(m)))
        .collect();
    let generator = TerrainGenerator::with_mods(7, h.registry.clone(), &dirs).unwrap();
    let site = generator
        .structure_sites_in((-6000, -6000), (6000, 6000))
        .into_iter()
        .find(|s| s.kind == StructureKind::Graph)
        .unwrap();
    let plan = generator.blueprint(&site).unwrap();
    let seal = h.block_id("donjons:boss_spawner");
    let (pos, _) = *plan
        .blocks()
        .iter()
        .rev()
        .find(|&&(_, b)| b == seal)
        .unwrap();
    let chunk_pos = World::chunk_containing(pos.as_vec3());
    let chunk = generator.generate(chunk_pos).expect("le chunk du sceau");
    let local = pos - World::chunk_origin(chunk_pos);
    let (x, y, z) = (local.x as usize, local.y as usize, local.z as usize);
    assert_eq!(chunk.get(x, y, z), seal);
    assert!(
        chunk.block_entity(x, y, z).is_some(),
        "un état pour réveiller le sceau"
    );
}
