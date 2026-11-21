//! Ce que les scripts des mods peuvent faire du joueur : le lire (`player`), lui rendre de la vie,
//! le téléporter, lui donner un effet, blesser les créatures autour d'un point, afficher des
//! jauges à l'écran, et être prévenus d'un clic droit avec un objet `usable` en main.
//!
//! Le script lit un **instantané** du joueur (rafraîchi à chaque pas de simulation) ; ses ordres
//! reviennent au jeu comme les autres `MachineAction`, appliqués ici.

#[allow(unused_imports)]
use crate::player_api_server::*;
use std::collections::BTreeMap;

use crate::VoxelGame;

/// Au plus ce nombre de jauges par mod.
const MAX_BARS_PER_MOD: usize = 4;

/// Les jauges des mods, par (mod, numéro) : dans cet ordre à l'écran.
pub type HudBars = BTreeMap<(String, u32), HudBar>;

impl VoxelGame {
    /// La fenêtre range une jauge de mod (quatre par mod au plus).
    pub(crate) fn receive_mod_hud(&mut self, mod_id: String, id: u32, bar: Option<HudBar>) {
        let bars = &mut self.client.ui.mod_hud;
        let Some(bar) = bar else {
            bars.remove(&(mod_id, id));
            return;
        };
        let known = bars.contains_key(&(mod_id.clone(), id));
        let count = bars.keys().filter(|(m, _)| *m == mod_id).count();
        if known || count < MAX_BARS_PER_MOD {
            bars.insert((mod_id, id), bar);
        }
    }

    /// Les jauges des mods, au-dessus de la barre d'objets, l'une sous l'autre.
    pub(crate) fn draw_mod_hud(&mut self, ctx: &engine_render::egui::Context) {
        use engine_render::egui::{self, Color32};
        if self.client.ui.mod_hud.is_empty()
            || self.client.ui.inventory_open
            || self.mode != crate::Mode::Playing
        {
            return;
        }
        egui::Area::new(egui::Id::unique("mod_hud"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -120.0))
            .interactable(false)
            .show(ctx, |ui| {
                for bar in self.client.ui.mod_hud.values() {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2((ctx.content_rect().width() * 0.4 - 12.0).min(220.0), 14.0),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter();
                    painter.rect_filled(rect, 2.0, Color32::from_black_alpha(160));
                    let mut fill = rect.shrink(1.5);
                    fill.set_width(fill.width() * bar.fraction.clamp(0.0, 1.0));
                    let [r, g, b] = bar.color;
                    painter.rect_filled(
                        fill,
                        2.0,
                        Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8),
                    );
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        bar.label.clone(),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );
                    ui.add_space(2.0);
                }
            });
    }
}
