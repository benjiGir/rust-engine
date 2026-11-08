//! Test de `#[derive(Inspect)]`, utilisé depuis une autre crate, comme le fera le jeu.

use engine_inspect::{Inspect, egui};

#[derive(Inspect, Default)]
struct Mob {
    health: f32,
    speed: f32,
    hostile: bool,
    position: glam::Vec3,
    #[inspect(skip)]
    _secret: u32,
}

#[derive(Inspect, Default)]
struct Tagged(u32, f32);

/// Dessine `value` dans une interface egui sans fenêtre.
fn draw(value: &mut impl Inspect) -> bool {
    let ctx = egui::Context::default();
    let mut changed = false;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        changed = value.inspect(ui, "test");
    });
    // Sans GPU, on n'envoie pas les textures d'egui : on les jette explicitement.
    output.textures_delta.clear();
    changed
}

#[test]
fn derive_lists_fields_and_skips_marked_ones() {
    assert_eq!(
        Mob::field_names(),
        ["health", "speed", "hostile", "position"]
    );
    assert_eq!(Tagged::field_names(), ["0", "1"]);
    // L'affichage marche hors fenêtre ; sans clic, rien ne change.
    assert!(!draw(&mut Mob::default()));
    assert!(!draw(&mut Tagged::default()));
}
