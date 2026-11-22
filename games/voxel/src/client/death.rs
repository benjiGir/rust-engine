//! La mort et la réapparition (phase O4), et le lit qui fixe le point de réapparition.
//!
//! - Vie à 0 : l'inventaire tombe par terre là où le joueur est mort, et l'écran de mort
//!   dit pourquoi (la cause du dernier coup, voir `player/survival.rs`). Le monde continue.
//! - « Réapparaître » : au lit, s'il existe encore, sinon au point de départ du monde ;
//!   vie, faim, air pleins.
//! - Le lit (clic droit) : fixe le point de réapparition, et la nuit fait dormir jusqu'au
//!   matin, sauf si une créature hostile rôde.

use engine_render::egui::{self, Color32, RichText};
#[allow(unused_imports)]
use voxel_sim::server::death::*;

use crate::ui::menus::button;
use crate::{Request, VoxelGame};

/// Combien de temps un message reste affiché.
const TOAST_SECONDS: f32 = 3.0;

impl VoxelGame {
    /// Le message en cours, s'il n'a pas expiré.
    pub(crate) fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = &self.client.ui.toast else {
            return;
        };
        if since.elapsed().as_secs_f32() > TOAST_SECONDS {
            self.client.ui.toast = None;
            return;
        }
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(
                    0.0,
                    if self.mode == crate::Mode::Playing && !self.client.ui.inventory_open {
                        self.client.ui.target_bottom + 8.0
                    } else {
                        12.0
                    },
                ),
            )
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 60.0).clamp(100.0, 420.0));
                egui::Frame::new()
                    .fill(crate::ui::skin::PANEL)
                    .stroke(egui::Stroke::new(1.0, crate::ui::skin::COPPER))
                    .corner_radius(0)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(17.0).color(crate::ui::skin::TEXT));
                    });
            });
    }

    /// L'écran de mort : la cause, puis réapparaître ou retourner au menu.
    pub(crate) fn draw_dead(&mut self, ctx: &egui::Context, message: &str) {
        ctx.layer_painter(egui::LayerId::background()).rect_filled(
            ctx.content_rect(),
            0.0,
            Color32::from_rgba_unmultiplied(120, 0, 0, 110),
        );
        let mut respawn = false;
        let mut quit = false;
        Self::menu_window(ctx, "Vous êtes mort", |ui| {
            ui.label(
                RichText::new(message)
                    .size(18.0)
                    .color(crate::ui::skin::MUTED),
            );
            ui.add_space(12.0);
            respawn = button(ui, "Réapparaître");
            let label = if self.server.store.is_some() {
                "Sauvegarder et retourner au menu"
            } else {
                "Retourner au menu"
            };
            quit = button(ui, label);
        });
        if respawn {
            self.send_now(voxel_sim::net::protocol::ClientMsg::Respawn);
        } else if quit {
            self.client.request = Some(Request::Title);
        }
    }
}
