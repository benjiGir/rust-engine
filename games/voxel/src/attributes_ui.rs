//! La fiche des attributs du joueur, ouverte par un bouton de l'écran d'inventaire.
//!
//! Elle liste les quatorze attributs avec leur valeur ; au survol d'une ligne, une bulle donne le
//! détail (la base, puis ce que chaque source apporte : objet en main, pièces d'armure, potions).
//! Une case à cocher ne garde que les attributs **modifiés** (différents de leur base). Les
//! valeurs viennent de [`voxel_sim::player::gear::Gear`] : rien n'est recalculé ici.

use engine_render::egui;

use crate::VoxelGame;
use crate::skin;
use voxel_sim::player::attributes::{Attribute, Attributes};

/// L'état de la fiche : ouverte ou non, et son filtre.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttributesPanel {
    pub open: bool,
    /// Ne montrer que les attributs modifiés.
    pub modified_only: bool,
}

/// Les attributs à montrer, dans l'ordre de la fiche.
pub fn rows(attrs: &Attributes, modified_only: bool) -> Vec<Attribute> {
    Attribute::ALL
        .into_iter()
        .filter(|a| !modified_only || attrs.is_modified(*a))
        .collect()
}

/// Le détail d'un attribut (la bulle) : la base, puis chaque source.
pub fn detail(attrs: &Attributes, attribute: Attribute) -> Vec<String> {
    let mut lines = vec![format!(
        "Base : {}",
        attribute.format(attrs.base(attribute))
    )];
    for c in attrs.contributions(attribute) {
        let mut parts = Vec::new();
        if c.flat != 0.0 {
            parts.push(format!("{}{}", sign(c.flat), trim(c.flat.abs())));
        }
        if c.percent != 0.0 {
            parts.push(format!(
                "{}{} %",
                sign(c.percent),
                trim(c.percent.abs() * 100.0)
            ));
        }
        lines.push(format!("{} : {}", c.label, parts.join(" et ")));
    }
    lines
}

fn sign(value: f32) -> &'static str {
    if value < 0.0 { "-" } else { "+" }
}

/// Un nombre sans zéros inutiles (« 1,5 », « 4 »).
fn trim(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0')
        .trim_end_matches('.')
        .replace('.', ",")
}

impl VoxelGame {
    /// Le bouton de l'écran d'inventaire.
    pub(crate) fn attributes_button(&mut self, ui: &mut egui::Ui) {
        let label = if self.client.ui.attributes.open {
            "Attributs ◂"
        } else {
            "Attributs"
        };
        if ui.small_button(label).clicked() {
            self.client.ui.attributes.open = !self.client.ui.attributes.open;
        }
    }

    /// La fiche, à gauche de l'inventaire (`frame`), sur la place qu'il laisse.
    pub(crate) fn attributes_window(&mut self, ctx: &egui::Context, frame: Option<egui::Rect>) {
        if !self.client.ui.attributes.open {
            return;
        }
        let safe = crate::ui_layout::safe_rect(ctx);
        let margin = skin::panel().total_margin().sum().x;
        let room = frame.map_or(safe.width(), |f| f.left() - safe.left() - 10.0);
        let width = 300.0_f32.min(room.max(180.0)).min(safe.width());
        // La largeur réelle de l'image précédente (le cadre ajoute des marges) place la fiche
        // contre l'inventaire sans le recouvrir.
        let id = egui::Id::unique("attributes_window");
        let actual = ctx.memory(|m| m.area_rect(id)).map_or(width, |r| r.width());
        let x = frame.map_or(safe.left(), |f| (f.left() - actual - 10.0).max(safe.left()));
        let mut close = false;
        let mut modified_only = self.client.ui.attributes.modified_only;
        let Some(attrs) = self
            .client
            .view
            .me
            .vitals
            .as_ref()
            .map(|v| v.attributes.clone())
        else {
            return;
        };
        egui::Window::new("Attributs")
            .id(id)
            .frame(skin::panel())
            .title_bar(false)
            .fixed_pos(egui::pos2(x, safe.top()))
            .constrain_to(safe)
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_width(width - margin);
                ui.horizontal(|ui| {
                    ui.heading("Attributs");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.small_button("×").clicked();
                    });
                });
                ui.checkbox(&mut modified_only, "Modifiés seulement");
                ui.add_space(4.0);
                let shown = rows(&attrs, modified_only);
                if shown.is_empty() {
                    ui.label(
                        egui::RichText::new("Aucun attribut modifié : rien en main ni sur soi.")
                            .color(skin::MUTED),
                    );
                }
                egui::Grid::new("attributes_grid")
                    .num_columns(2)
                    .min_col_width((width - margin) * 0.5 - 4.0)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for attribute in shown {
                            let modified = attrs.is_modified(attribute);
                            let name = ui.label(egui::RichText::new(attribute.name()).size(13.0));
                            let color = if modified { skin::SAGE } else { skin::TEXT };
                            let value = ui.label(
                                egui::RichText::new(attribute.format(attrs.value(attribute)))
                                    .size(13.0)
                                    .color(color),
                            );
                            let tip = detail(&attrs, attribute);
                            for response in [name, value] {
                                response.on_hover_ui(|ui| {
                                    for (i, line) in tip.iter().enumerate() {
                                        let color = if i == 0 { skin::MUTED } else { skin::TEXT };
                                        ui.label(egui::RichText::new(line).color(color));
                                    }
                                });
                            }
                            ui.end_row();
                        }
                    });
            });
        if close {
            self.client.ui.attributes.open = false;
        }
        self.client.ui.attributes.modified_only = modified_only;
    }

    /// Les attributs en JSON, pour `ctl attributes` : valeur, base, et le détail par source.
    pub(crate) fn attributes_json(&self) -> String {
        let attrs = self.server.players.me().gear.attrs();
        let rows: Vec<String> = Attribute::ALL
            .into_iter()
            .map(|a| {
                let sources: Vec<String> = detail(attrs, a)
                    .iter()
                    .skip(1)
                    .map(|l| voxel_sim::server::json_text::json_str(l))
                    .collect();
                format!(
                    "{{\"id\":{},\"name\":{},\"value\":{:.4},\"base\":{:.4},\"modified\":{},\"text\":{},\"sources\":[{}]}}",
                    a.id(),
                    voxel_sim::server::json_text::json_str(a.name()),
                    attrs.value(a),
                    attrs.base(a),
                    attrs.is_modified(a),
                    voxel_sim::server::json_text::json_str(&a.format(attrs.value(a))),
                    sources.join(",")
                )
            })
            .collect();
        format!(
            "{{\"ok\":true,\"open\":{},\"modified_only\":{},\"attributes\":[{}]}}",
            self.client.ui.attributes.open,
            self.client.ui.attributes.modified_only,
            rows.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_sim::player::attributes::{BaseAttributes, Modifier, Source};

    fn attrs() -> Attributes {
        Attributes::compute(
            BaseAttributes::default(),
            vec![Source {
                label: "Épée de fer".into(),
                modifiers: vec![
                    Modifier::flat(Attribute::Damage, 2.0),
                    Modifier::percent(Attribute::MiningSpeed, 0.3),
                ],
            }],
        )
    }

    #[test]
    fn the_filter_keeps_only_modified_attributes() {
        let attrs = attrs();
        assert_eq!(
            rows(&attrs, false).len(),
            voxel_sim::player::attributes::COUNT
        );
        assert_eq!(
            rows(&attrs, true),
            vec![Attribute::Damage, Attribute::MiningSpeed]
        );
        assert!(rows(&Attributes::plain(BaseAttributes::default()), true).is_empty());
    }

    #[test]
    fn the_detail_lists_the_base_then_each_source() {
        let attrs = attrs();
        assert_eq!(
            detail(&attrs, Attribute::Damage),
            vec!["Base : 4,0".to_string(), "Épée de fer : +2".to_string()]
        );
        assert_eq!(
            detail(&attrs, Attribute::MiningSpeed)[1],
            "Épée de fer : +30 %"
        );
        assert_eq!(detail(&attrs, Attribute::Armor), vec!["Base : 0,0"]);
    }
}
