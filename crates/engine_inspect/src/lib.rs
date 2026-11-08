//! Le trait [`Inspect`] : afficher une valeur dans l'interface egui et la modifier.
//!
//! Les types de base (nombres, booléens, vecteurs, texte) l'implémentent ici ; une
//! struct l'obtient avec `#[derive(Inspect)]`, qui appelle `inspect` sur chaque champ.
//! C'est ce qui permet à l'inspecteur (F3) de modifier les composants en direct.

// On réexporte egui : le code généré par la macro écrit `::engine_inspect::egui::Ui`,
// il fonctionne donc dans une crate qui ne dépend pas elle-même d'egui.
pub use egui;
pub use engine_inspect_derive::Inspect;

/// Une valeur modifiable dans l'interface.
pub trait Inspect {
    /// Dessine la valeur ; renvoie `true` si l'utilisateur l'a modifiée.
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool;

    /// Les noms des champs (vide pour les types simples).
    fn field_names() -> &'static [&'static str]
    where
        Self: Sized,
    {
        &[]
    }
}

/// Une ligne « nom : éditeur ».
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        add(ui).changed()
    })
    .inner
}

/// Les nombres : un champ qu'on fait glisser à la souris. Une macro pour éviter de
/// répéter le même code pour chaque type de nombre.
macro_rules! impl_number {
    ($($t:ty),*) => {$(
        impl Inspect for $t {
            fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
                row(ui, label, |ui| ui.add(egui::DragValue::new(self).speed(0.1)))
            }
        }
    )*};
}

impl_number!(f32, f64, i32, u32, i64, u64, u8, u16, usize);

impl Inspect for bool {
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
        ui.checkbox(self, label).changed()
    }
}

impl Inspect for String {
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
        row(ui, label, |ui| ui.text_edit_singleline(self))
    }
}

impl Inspect for glam::Vec3 {
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            let mut changed = false;
            for (axis, value) in ["x", "y", "z"]
                .iter()
                .zip([&mut self.x, &mut self.y, &mut self.z])
            {
                changed |= ui
                    .add(
                        egui::DragValue::new(value)
                            .speed(0.1)
                            .prefix(format!("{axis} ")),
                    )
                    .changed();
            }
            changed
        })
        .inner
    }
}

impl Inspect for glam::IVec3 {
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
        ui.horizontal(|ui| {
            ui.label(label);
            let mut changed = false;
            for value in [&mut self.x, &mut self.y, &mut self.z] {
                changed |= ui.add(egui::DragValue::new(value)).changed();
            }
            changed
        })
        .inner
    }
}

impl<T: Inspect> Inspect for Option<T> {
    fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
        match self {
            Some(value) => value.inspect(ui, label),
            None => {
                ui.label(format!("{label} : aucun"));
                false
            }
        }
    }
}
