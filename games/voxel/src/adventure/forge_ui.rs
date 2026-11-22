//! Les écrans des deux tables du butin (voir `adventure/forge_tables.rs` pour les règles).
//!
//! Dans les deux, on **prend l'objet à la souris** (un clic sur sa case dans l'inventaire, en
//! dessous), puis on agit avec les boutons de l'écran. Les matériaux sont pris dans l'inventaire,
//! ce qu'on récupère y retourne (ou tombe à terre s'il est plein).

use engine_render::egui::{self, Color32};
#[allow(unused_imports)]
use voxel_sim::adventure::forge_ui::*;

use crate::VoxelGame;
use crate::ui::skin;
use voxel_sim::adventure::affixes;
use voxel_sim::adventure::data::AdventureData;
use voxel_sim::adventure::forge_tables::{self, Purse};
use voxel_sim::net::protocol::{ClientMsg, ScreenAction};
use voxel_sim::player::item_data::ItemData;

/// Une rareté se lit mal sur le cadre clair de l'inventaire : son nom est posé sur un fond sombre.
pub fn rarity_text(text: impl Into<String>, color: Color32) -> egui::RichText {
    egui::RichText::new(text)
        .strong()
        .color(color)
        .background_color(Color32::from_rgb(19, 17, 28))
}

/// La couleur d'une rareté.
pub fn rarity_color(data: &AdventureData, rarity: u8) -> Color32 {
    data.rarity(rarity).map_or(skin::TEXT, |r| {
        Color32::from_rgb(r.color.0, r.color.1, r.color.2)
    })
}

/// Les lignes d'une fiche tirée : ses affixes, ses emplacements.
fn sheet_lines(sheet: &ItemData) -> Vec<String> {
    let mut lines: Vec<String> = sheet.affixes.iter().filter_map(affixes::describe).collect();
    if !sheet.sockets.is_empty() {
        lines.push(format!("{} emplacement(s) de gemme", sheet.sockets.len()));
    }
    lines
}

impl VoxelGame {
    /// La table de récupération : l'objet tenu à la souris (d'après la vue du joueur), ce qu'il
    /// rendra, et le bouton qui envoie `ScreenAction::Salvage`.
    pub(crate) fn salvage_panel(&mut self, ui: &mut egui::Ui) {
        let data = voxel_sim::adventure::data::current();
        let items = self.client.content.items.clone();
        let Some(held) = self.client.view.me.inventory.cursor else {
            ui.label("Prends à la souris un objet à affixes (clic sur sa case, dans l'inventaire), puis reviens ici.");
            ui.label(
                egui::RichText::new(
                    "Il sera détruit et rendra des matériaux de sa rareté ; ses gemmes te reviennent.",
                )
                .weak(),
            );
            return;
        };
        let def = items.get(held.item);
        let name = voxel_sim::player::item_data::sheet(held.damage)
            .and_then(|s| s.name)
            .unwrap_or_else(|| def.name.clone());
        match affixes::rarity_of(held.damage) {
            Some(r) => ui.label(rarity_text(name, rarity_color(&data, r)).size(18.0)),
            None => ui.heading(name),
        };
        let preview = forge_tables::salvage_preview(&data, held.damage)
            .filter(|_| forge_tables::can_salvage(held.damage));
        let Some((shard, shards, dust)) = preview else {
            ui.label("Cet objet n'a pas d'affixe : il n'y a rien à récupérer.");
            return;
        };
        let shard_name = items
            .id(&shard)
            .map_or(shard.clone(), |id| items.get(id).name.clone());
        let range = |(lo, hi): (u32, u32)| {
            if lo == hi {
                lo.to_string()
            } else {
                format!("{lo} à {hi}")
            }
        };
        ui.label(format!("Rend : {} × {shard_name}", range(shards)));
        if dust.1 > 0 {
            ui.label(format!(
                "Et peut-être : {} poussière(s) de gemme",
                range(dust)
            ));
        }
        let gems = voxel_sim::adventure::gems::gems_of(held.damage);
        if !gems.is_empty() {
            ui.label(format!(
                "Les {} gemme(s) serties te sont rendues.",
                gems.len()
            ));
        }
        if ui.button("Détruire et récupérer").clicked() {
            self.send_now(ClientMsg::Screen(ScreenAction::Salvage));
        }
    }

    /// La table de réforge : la rareté voulue, le coût, les trois tirages (calculés comme le
    /// serveur, d'après la graine de la vue du joueur) et le bouton qui envoie `ReforgeChoose`.
    pub(crate) fn reforge_panel(&mut self, ui: &mut egui::Ui) {
        let data = voxel_sim::adventure::data::current();
        let items = self.client.content.items.clone();
        let Some(vitals) = self.client.view.me.vitals.clone() else {
            return;
        };
        let inventory = self.client.view.me.inventory.as_inventory();
        let Some(held) = inventory.cursor else {
            ui.label("Prends à la souris un objet de base (outil ou armure sans affixe), puis reviens ici.");
            ui.label(
                egui::RichText::new(
                    "Choisis une rareté : la table propose trois tirages, tu en gardes un.",
                )
                .weak(),
            );
            return;
        };
        let def = items.get(held.item);
        ui.heading(&def.name);
        if !forge_tables::can_reforge(def, held.damage) {
            ui.label(if affixes::can_carry(def) {
                "Cet objet a déjà des affixes : récupère-le d'abord, ou garde-le tel quel."
            } else {
                "Cet objet ne peut pas porter d'affixe."
            });
            return;
        }
        // La rareté voulue.
        let current = vitals
            .forge_rarity
            .min(data.rarities.len().saturating_sub(1) as u8);
        let mut rarity = current;
        ui.horizontal_wrapped(|ui| {
            for (i, r) in data.rarities.iter().enumerate() {
                let color = Color32::from_rgb(r.color.0, r.color.1, r.color.2);
                if ui
                    .selectable_label(rarity as usize == i, rarity_text(&r.name, color))
                    .clicked()
                {
                    rarity = i as u8;
                }
            }
        });
        if rarity != current {
            self.send_now(ClientMsg::Screen(ScreenAction::ReforgeRarity(rarity)));
        }
        let Some(cost) = forge_tables::reforge_cost(&data, rarity) else {
            return;
        };
        let shard_id = items.id(&cost.shard);
        let purse = Purse {
            shards: shard_id.map_or(0, |id| inventory.count(id)),
            dust: forge_tables::dust_item(&items).map_or(0, |id| inventory.count(id)),
            levels: vitals.level,
        };
        let shard_name = shard_id.map_or(cost.shard.clone(), |id| items.get(id).name.clone());
        let affordable = cost.affordable(purse);
        ui.label(
            egui::RichText::new(format!(
                "Coût d'un tirage : {} × {shard_name} (tu en as {}), {} poussière(s) de gemme ({}), {} niveau(x) (tu es au niveau {})",
                cost.shards, purse.shards, cost.dust, purse.dust, cost.levels, purse.levels
            ))
            .color(if affordable { Color32::BLACK } else { Color32::from_rgb(150, 85, 15) }),
        );
        let draws = forge_tables::draws(
            &data,
            def,
            rarity,
            vitals.forge_seed ^ u64::from(held.item.0),
        );
        let color = rarity_color(&data, rarity);
        let mut chosen = None;
        for (i, draw) in draws.iter().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(rarity_text(draw.name.clone().unwrap_or_default(), color));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(affordable, egui::Button::new("Choisir"))
                            .clicked()
                        {
                            chosen = Some(i);
                        }
                    });
                });
                for line in sheet_lines(draw) {
                    ui.label(egui::RichText::new(line).size(12.0));
                }
            });
        }
        if let Some(i) = chosen {
            self.send_now(ClientMsg::Screen(ScreenAction::ReforgeChoose(i as u8)));
        }
    }
}
