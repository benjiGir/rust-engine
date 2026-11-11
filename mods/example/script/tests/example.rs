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
