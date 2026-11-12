//! Les tests du mod d'exemple, sans fenêtre : le vrai script compilé, dans un petit monde.
//! Lancer : `cargo xtask test-mod example` (compile le script puis lance ces tests).

use engine_mods::ModEvent;
use engine_mods::testing::Harness;
use engine_voxel::{GridPos, MachineAction};
use glam::IVec3;

#[test]
fn breaking_stone_sometimes_gives_a_ruby() {
    let mut h = Harness::for_mod("example").unwrap();
    let (stone, ruby) = (h.block_id("core:stone"), h.block_id("example:ruby"));
    let broken = ModEvent::BlockBroken {
        at: GridPos::world(IVec3::new(0, 50, 0)),
        block: stone,
    };
    let rubies = (0..400)
        .map(|_| h.event(&broken).unwrap())
        .flat_map(|r| r.actions)
        .filter(|a| matches!(a, MachineAction::GiveItem { block, .. } if *block == ruby))
        .count();
    // 1 chance sur 8 par défaut : environ 50 sur 400.
    assert!((20..100).contains(&rubies), "{rubies} rubis sur 400");
}

#[test]
fn the_breaker_opens_its_menu_on_right_click() {
    let mut h = Harness::for_mod("example").unwrap();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "example:breaker");
    let (taken, requests) = h.use_block(at).unwrap();
    assert!(taken, "le casseur prend le clic droit");
    assert!(
        requests
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::ShowMenu { mod_id, .. } if mod_id == "example"))
    );
}

#[test]
fn the_toggle_disables_the_breaker_and_refreshes_the_menu() {
    use engine_mods::menu::UI_TOGGLE;
    let mut h = Harness::for_mod("example").unwrap();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "example:breaker");
    let requests = h.ui(at, UI_TOGGLE, 1, 0).unwrap();
    // Le menu est renvoyé à jour, et le casseur éteint ne s'arme plus.
    assert!(
        requests
            .actions
            .iter()
            .any(|a| matches!(a, MachineAction::ShowMenu { .. }))
    );
}

#[test]
fn new_chunks_get_a_surface_ruby() {
    use engine_mods::ModEvent;
    let mut h = Harness::for_mod("example").unwrap();
    // Un rubis dans chaque chunk neuf qui a une surface (réglage forcé à 1 sur 1).
    h.settings
        .borrow_mut()
        .set("example", "surface_ruby_chance", "1");
    let ruby = h.block_id("example:ruby");
    // Les filons de rubis du terrain existent déjà : on compte ce que le script ajoute.
    let count = |h: &Harness| {
        let streamer = h.streamer.borrow();
        let mut n = 0;
        for x in -32..64 {
            for z in -32..64 {
                for y in 0..128 {
                    n += usize::from(streamer.world().block(IVec3::new(x, y, z)) == ruby);
                }
            }
        }
        n
    };
    let before = count(&h);
    for cx in -1..=1 {
        for cz in -1..=1 {
            for cy in 0..3 {
                h.event(&ModEvent::ChunkGenerated {
                    chunk: IVec3::new(cx, cy, cz),
                })
                .unwrap();
                h.next_tick();
            }
        }
    }
    let placed = count(&h) - before;
    assert!(placed >= 3, "{placed} rubis de surface ajoutés");
    // Désactivé par le réglage : rien n'est posé (les filons de rubis du terrain, eux, existent déjà).
    let mut off = Harness::for_mod("example").unwrap();
    off.settings
        .borrow_mut()
        .set("example", "surface_ruby_chance", "0");
    let count_chunk = |h: &Harness| {
        let streamer = h.streamer.borrow();
        let mut n = 0;
        for x in 0..32 {
            for y in 0..96 {
                for z in 0..32 {
                    n += usize::from(streamer.world().block(IVec3::new(x, y, z)) == ruby);
                }
            }
        }
        n
    };
    let before = count_chunk(&off);
    for cy in 0..3 {
        off.event(&ModEvent::ChunkGenerated {
            chunk: IVec3::new(0, cy, 0),
        })
        .unwrap();
    }
    assert_eq!(count_chunk(&off), before);
}

#[test]
fn the_ruby_command_gives_rubies() {
    use engine_voxel::MachineAction;
    let mut h = Harness::for_mod("example").unwrap();
    assert_eq!(h.commands().len(), 1);
    let ruby = h.block_id("example:ruby");
    let (reply, requests) = h.command("rubis", "5").unwrap();
    assert!(reply.unwrap().contains('5'));
    assert_eq!(
        requests.actions,
        vec![MachineAction::GiveItem {
            block: ruby,
            count: 5
        }]
    );
    // Sans argument : un seul ; trop grand : 64 au plus.
    let (_, one) = h.command("rubis", "").unwrap();
    assert_eq!(
        one.actions,
        vec![MachineAction::GiveItem {
            block: ruby,
            count: 1
        }]
    );
    let (_, many) = h.command("rubis", "1000").unwrap();
    assert_eq!(
        many.actions,
        vec![MachineAction::GiveItem {
            block: ruby,
            count: 64
        }]
    );
}

#[test]
fn the_guard_idles_chases_attacks_and_flees() {
    use engine_mods::mobs::MobSnapshot;
    use engine_voxel::MachineAction::{MobAttack, MobMove, MobState};
    let mut h = Harness::for_mod("example").unwrap();
    let guard = h.mob_kind("example:guard");
    let snap = |to: [f32; 3], health: f32, state: u32| MobSnapshot {
        id: 5,
        kind: guard,
        health,
        max_health: 20.0,
        to_player: to,
        state,
        ..Default::default()
    };
    // Loin du joueur : il rôde, rien n'est dit sur son état (déjà 0).
    let far = h.mobs(&[snap([60.0, 0.0, 0.0], 20.0, 0)]).unwrap();
    assert!(
        !far.actions
            .iter()
            .any(|a| matches!(a, MobState { .. } | MobAttack { .. }))
    );
    // Le joueur à 10 blocs : il le poursuit (état 1) en marchant vers lui à 3 blocs/s.
    let chase = h.mobs(&[snap([10.0, 0.0, 0.0], 20.0, 0)]).unwrap();
    assert!(chase.actions.contains(&MobState { mob: 5, value: 1 }));
    assert!(chase.actions.contains(&MobMove {
        mob: 5,
        dx: 10.0,
        dz: 0.0,
        speed: 3.0
    }));
    // Tout près : il s'arrête et frappe.
    let close = h.mobs(&[snap([1.0, 0.0, 0.0], 20.0, 1)]).unwrap();
    assert!(close.actions.contains(&MobAttack {
        mob: 5,
        damage: 2.0
    }));
    assert!(close.actions.contains(&MobMove {
        mob: 5,
        dx: 0.0,
        dz: 0.0,
        speed: 0.0
    }));
    // Blessé : il fuit, à l'opposé du joueur, et change d'état.
    let flee = h.mobs(&[snap([10.0, 0.0, 2.0], 4.0, 1)]).unwrap();
    assert!(flee.actions.contains(&MobState { mob: 5, value: 2 }));
    assert!(flee.actions.contains(&MobMove {
        mob: 5,
        dx: -10.0,
        dz: -2.0,
        speed: 4.0
    }));
    // Les créatures d'un autre type sont laissées tranquilles.
    let other = MobSnapshot {
        kind: guard + 1,
        ..snap([1.0, 0.0, 0.0], 20.0, 0)
    };
    assert!(h.mobs(&[other]).unwrap().actions.is_empty());
}

#[test]
fn a_chest_broken_by_a_script_spills_its_contents() {
    use engine_mods::menu::UI_BUTTON;
    let mut h = Harness::for_mod("example").unwrap();
    let at = IVec3::new(0, 100, 0);
    h.place(at, "example:breaker");
    let chest = at + IVec3::X;
    h.place(chest, "core:chest");
    h.put(chest, 0, "core:iron_ingot", 5);
    h.put(chest, 3, "core:stone", 12);
    // « Casser maintenant » : le casseur regarde vers +X par défaut.
    let requests = h.ui(at, UI_BUTTON, 2, 0).unwrap();
    assert_eq!(h.block(chest), "core:air");
    let mut spilled: Vec<(String, u32)> = requests
        .actions
        .iter()
        .filter_map(|a| match a {
            MachineAction::DropStack {
                pos, item, count, ..
            } if *pos == chest => Some((item.clone(), *count)),
            _ => None,
        })
        .collect();
    spilled.sort();
    assert_eq!(
        spilled,
        vec![("core:iron_ingot".into(), 5), ("core:stone".into(), 12)]
    );
}
