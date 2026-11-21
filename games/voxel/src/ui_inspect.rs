//! L'inspecteur egui (F3) : il dessine ce que décrit un `engine_inspect::Inspect` et rapporte ce que
//! l'utilisateur change.

use engine_inspect::Inspector;
use engine_render::egui;

/// Un inspecteur qui dessine dans une zone egui.
pub struct EguiInspector<'a>(pub &'a mut egui::Ui);

/// Une ligne « nom : éditeur ».
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        add(ui).changed()
    })
    .inner
}

impl Inspector for EguiInspector<'_> {
    fn float(&mut self, label: &str, value: &mut f64) -> bool {
        row(self.0, label, |ui| {
            ui.add(egui::DragValue::new(value).speed(0.1))
        })
    }

    fn int(&mut self, label: &str, value: &mut i64) -> bool {
        row(self.0, label, |ui| {
            ui.add(egui::DragValue::new(value).speed(0.1))
        })
    }

    fn boolean(&mut self, label: &str, value: &mut bool) -> bool {
        self.0.checkbox(value, label).changed()
    }

    fn text(&mut self, label: &str, value: &mut String) -> bool {
        row(self.0, label, |ui| ui.text_edit_singleline(value))
    }

    fn floats(&mut self, label: &str, values: &mut [f32]) -> bool {
        self.0
            .horizontal(|ui| {
                ui.label(label);
                let mut changed = false;
                for (axis, value) in ["x", "y", "z", "w"].iter().zip(values.iter_mut()) {
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

    fn ints(&mut self, label: &str, values: &mut [i32]) -> bool {
        self.0
            .horizontal(|ui| {
                ui.label(label);
                let mut changed = false;
                for value in values.iter_mut() {
                    changed |= ui.add(egui::DragValue::new(value)).changed();
                }
                changed
            })
            .inner
    }

    fn absent(&mut self, label: &str) {
        self.0.label(format!("{label} : aucun"));
    }

    fn group(&mut self, label: &str, body: &mut dyn FnMut(&mut dyn Inspector) -> bool) -> bool {
        let mut changed = false;
        self.0.collapsing(label, |ui| {
            changed = body(&mut EguiInspector(ui));
        });
        changed
    }
}
