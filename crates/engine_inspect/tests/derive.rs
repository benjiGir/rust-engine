//! Test de `#[derive(Inspect)]`, utilisé depuis une autre crate, comme le fera le jeu.

use engine_inspect::{Inspect, Inspector};

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

/// Un inspecteur qui note les noms qu'on lui décrit et met chaque réel à 2.
#[derive(Default)]
struct Names(Vec<String>);

impl Inspector for Names {
    fn float(&mut self, label: &str, value: &mut f64) -> bool {
        self.0.push(label.into());
        *value = 2.0;
        true
    }
    fn int(&mut self, label: &str, _: &mut i64) -> bool {
        self.0.push(label.into());
        false
    }
    fn boolean(&mut self, label: &str, _: &mut bool) -> bool {
        self.0.push(label.into());
        false
    }
    fn text(&mut self, label: &str, _: &mut String) -> bool {
        self.0.push(label.into());
        false
    }
    fn floats(&mut self, label: &str, _: &mut [f32]) -> bool {
        self.0.push(label.into());
        false
    }
    fn ints(&mut self, label: &str, _: &mut [i32]) -> bool {
        self.0.push(label.into());
        false
    }
    fn absent(&mut self, label: &str) {
        self.0.push(label.into());
    }
    fn group(&mut self, label: &str, body: &mut dyn FnMut(&mut dyn Inspector) -> bool) -> bool {
        self.0.push(format!("[{label}]"));
        body(self)
    }
}

#[test]
fn derive_lists_fields_and_skips_marked_ones() {
    assert_eq!(
        Mob::field_names(),
        ["health", "speed", "hostile", "position"]
    );
    assert_eq!(Tagged::field_names(), ["0", "1"]);
    let mut names = Names::default();
    let mut mob = Mob::default();
    assert!(mob.inspect(&mut names, "mob"), "un réel a changé");
    assert_eq!(names.0, ["[mob]", "health", "speed", "hostile", "position"]);
    assert_eq!((mob.health, mob.speed), (2.0, 2.0));
    let mut names = Names::default();
    Tagged::default().inspect(&mut names, "t");
    assert_eq!(names.0, ["[t]", "0", "1"]);
}
