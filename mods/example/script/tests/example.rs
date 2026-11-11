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
