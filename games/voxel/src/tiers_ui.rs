//! L'écran des paliers du monde : on y voit les cinq paliers, ce qui débloque chacun et où on en
//! est, et on choisit celui qu'on veut parmi ceux qu'on a débloqués. On l'ouvre avec la touche
//! « Paliers du monde » des options (J par défaut) ; la même touche le referme.
//!
//! Le palier choisi change les créatures qui **apparaissent ensuite** (vie, dégâts, élites,
//! envahisseurs) et la rareté du butin ; celles qui sont déjà là ne changent pas. Les règles sont dans
//! `tiers.rs`, les données dans `tiers.ron`.

use engine_render::egui;

use crate::VoxelGame;
use crate::adventure_data::{self, TierDef};
use crate::skin;
use crate::tiers;
use crate::ui::Screen;

/// Les couleurs de l'écran : le cadre de l'inventaire est clair, il lui faut du texte sombre.
const DONE: egui::Color32 = egui::Color32::from_rgb(40, 110, 50);
const PENDING: egui::Color32 = egui::Color32::from_rgb(150, 85, 15);

/// Les effets d'un palier, écrits pour l'écran.
pub fn effects_line(tier: &TierDef) -> String {
    let mut parts = vec![
        format!(
            "Créatures : vie ×{:.1}, dégâts ×{:.1}",
            tier.health, tier.damage
        ),
        format!("Élites : {:.0} %", tier.elite_chance * 100.0),
        format!("Rareté : +{:.0} % par cran", tier.rarity_boost * 100.0),
        format!("Butin des hostiles : {:.0} %", tier.drop_chance * 100.0),
    ];
    if tier.invader.1 > 0 {
        parts.push(format!(
            "Envahisseurs : toutes les {} à {} s",
            tier.invader.0, tier.invader.1
        ));
    }
    parts.join(" · ").replace('.', ",")
}

impl VoxelGame {
    /// La touche des paliers : ouvre l'écran, ou le referme.
    pub(crate) fn toggle_tiers_screen(&mut self) {
        if self.client.ui.inventory_open && self.client.ui.screen == Screen::Tiers {
            self.close_screen();
        } else if self.client.mouse_captured && !self.client.ui.inventory_open {
            self.client.ui.screen = Screen::Tiers;
            self.client.ui.inventory_open = true;
        }
    }

    /// Le corps de l'écran : une carte par palier.
    pub(crate) fn tiers_panel(&mut self, ui: &mut egui::Ui) {
        let data = adventure_data::current();
        // L'aventure telle que la vue du joueur la décrit.
        let Some(progress) = self.client.view.me.vitals.as_ref().map(|v| v.tiers) else {
            return;
        };
        let mut chosen = None;
        ui.label(
            egui::RichText::new(
                "Un palier plus haut rend les créatures plus rudes et le butin plus riche. \
                 Il s'applique aux créatures qui apparaissent ensuite.",
            )
            .weak(),
        );
        ui.add_space(4.0);
        for (i, tier) in data.tiers.iter().enumerate() {
            let index = i as u8;
            let unlocked = progress.is_unlocked(index);
            let current = progress.tier == index;
            egui::Frame::group(ui.style())
                .stroke(egui::Stroke::new(
                    if current { 2.0 } else { 1.0 },
                    if current { skin::GOLD } else { skin::MUTED },
                ))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}. {}", i + 1, tier.name))
                                .strong()
                                .size(16.0),
                        );
                        if current {
                            ui.label(egui::RichText::new("palier actif").color(PENDING));
                        } else if !unlocked {
                            ui.label(egui::RichText::new("verrouillé").weak());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if unlocked && !current && ui.button("Choisir").clicked() {
                                chosen = Some(index);
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&tier.description).weak());
                    ui.label(egui::RichText::new(effects_line(tier)).size(12.0));
                    if !unlocked {
                        for criterion in &tier.unlock {
                            let done = progress.meets(*criterion);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    if done { "[x]" } else { "[ ]" },
                                    tiers::describe(*criterion, &progress, &data.rarities)
                                ))
                                .size(12.0)
                                .color(if done {
                                    DONE
                                } else {
                                    PENDING
                                }),
                            );
                        }
                    }
                });
        }
        if let Some(index) = chosen {
            self.send_now(crate::protocol::ClientMsg::Screen(
                crate::protocol::ScreenAction::Tier(index),
            ));
        }
    }

    /// `Tier` : le serveur change le palier du joueur courant, s'il est débloqué.
    pub(crate) fn choose_tier(&mut self, index: u8) {
        let data = adventure_data::current();
        if self
            .players
            .me_mut()
            .adventure
            .progress
            .select(index, &data.tiers)
        {
            let name = data
                .tier(index)
                .map_or("?", |t| t.name.as_str())
                .to_string();
            self.toast(&format!("Palier : {name}"));
            self.sound("core:enchant");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tier_is_described_with_its_effects() {
        let data = adventure_data::AdventureData::builtin();
        let haven = effects_line(&data.tiers[0]);
        assert!(haven.contains("vie ×1,0"), "{haven}");
        assert!(!haven.contains("Envahisseurs"));
        let ascent = effects_line(&data.tiers[2]);
        assert!(ascent.contains("Envahisseurs"), "{ascent}");
        assert!(ascent.contains("vie ×1,7"));
    }
}
