//! La mort et la réapparition (phase O4), et le lit qui fixe le point de réapparition.
//!
//! - Vie à 0 : l'inventaire tombe par terre là où le joueur est mort, et l'écran de mort
//!   dit pourquoi (la cause du dernier coup, voir `survival.rs`). Le monde continue.
//! - « Réapparaître » : au lit, s'il existe encore, sinon au point de départ du monde ;
//!   vie, faim, air pleins.
//! - Le lit (clic droit) : fixe le point de réapparition, et la nuit fait dormir jusqu'au
//!   matin, sauf si une créature hostile rôde.

use engine_render::egui::{self, Color32, RichText};
use engine_voxel::bed;
use glam::{IVec3, Vec3};

use crate::components::{Health, Mob, Transform};
use crate::menus::button;
use crate::player;
use crate::survival::{Hunger, Vitals};
use crate::{Mode, Request, VoxelGame};

/// Une créature hostile plus proche que ça empêche de dormir.
const MONSTER_RADIUS: f32 = 8.0;
/// L'heure du réveil (le soleil vient de se lever).
const MORNING: f32 = 0.27;
/// Combien de temps un message reste affiché.
const TOAST_SECONDS: f32 = 3.0;

impl VoxelGame {
    /// À chaque pas : la vie à 0, c'est la mort.
    pub(crate) fn check_death(&mut self) {
        if self.mode != Mode::Playing
            || self.players.me().dead
            || player::health(&self.ecs, self.players.entity()).current > 0.0
        {
            return;
        }
        let message = self
            .players
            .me_mut()
            .vitals
            .last_cause
            .map_or("Vous êtes mort.", |cause| cause.death_message());
        // Ce qu'il tenait à la souris et la grille d'artisanat reviennent dans l'inventaire, puis
        // tout tombe là où le joueur est mort. La fenêtre ferme son écran en recevant `Died`.
        self.close_menu();
        let feet = player::body(&self.ecs, self.players.entity())
            .position
            .floor()
            .as_ivec3();
        let stacks = self.players.me_mut().inventory.take_all();
        let dropped: u32 = stacks.iter().map(|s| s.count).sum();
        for stack in stacks {
            self.drop_stack(feet, stack);
        }
        self.players.me_mut().mining = None;
        self.players.me_mut().hunger.chewing = None;
        // L'expérience est perdue.
        let (level, _) = self.players.me_mut().experience.level();
        self.players.me_mut().experience = crate::survival::Experience::default();
        log::info!("{message} ({dropped} objets lâchés en {feet}, niveau {level} perdu)");
        self.players.me_mut().dead = true;
        let id = self.players.current_id();
        self.replies.push((
            id,
            crate::protocol::ServerMsg::Died {
                message: message.to_string(),
            },
        ));
    }

    /// Réapparaître : au lit s'il existe encore, sinon au départ du monde.
    pub(crate) fn respawn(&mut self) {
        let feet = self.respawn_point();
        player::set_eye(
            &self.ecs,
            self.players.entity(),
            feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
        );
        if let Some(mut health) = self.ecs.get_mut::<Health>(self.players.entity()) {
            health.current = health.max;
        }
        self.players.me_mut().vitals = Vitals::new(&self.survival);
        self.players.me_mut().hunger = Hunger::new(&self.hunger_rules);
        self.players.me_mut().effects.clear();
        self.players.me_mut().dead = false;
        let id = self.players.current_id();
        self.replies
            .push((id, crate::protocol::ServerMsg::Respawned));
        // Le jeu a déplacé le joueur : son prochain mouvement n'est pas à comparer à l'ancien.
        self.forget_last_move();
        log::info!("réapparition en {feet}");
    }

    /// Où réapparaître : sur le lit (s'il n'a pas été cassé), sinon au point de départ.
    fn respawn_point(&mut self) -> Vec3 {
        if let Some(bed) = self.players.me_mut().bed {
            let head = bed::head_at(self.streamer().world(), bed);
            if let Some(head) = head {
                return head.as_vec3() + Vec3::new(0.5, 1.0, 0.5);
            }
            self.players.me_mut().bed = None;
            self.toast("Votre lit a disparu : retour au point de départ");
        }
        let spawn = self.spawn;
        Vec3::new(
            spawn.x as f32 + 0.5,
            spawn.y as f32 + 1.4,
            spawn.z as f32 + 0.5,
        )
    }

    /// Un clic droit sur un lit : le point de réapparition, et dormir la nuit.
    /// Renvoie `false` si `pos` n'est pas un lit.
    pub(crate) fn use_bed(&mut self, pos: IVec3) -> bool {
        let head = bed::head_at(self.streamer().world(), pos);
        let Some(head) = head else {
            return false;
        };
        self.players.me_mut().bed = Some(head);
        if !self.day.is_night() {
            self.toast("Point de réapparition fixé");
            return true;
        }
        if self.monster_nearby(head.as_vec3()) {
            self.toast("Impossible de dormir : des créatures rôdent");
            return true;
        }
        self.day.time = MORNING;
        self.toast("Vous avez dormi jusqu'au matin");
        true
    }

    /// Une créature hostile à moins de [`MONSTER_RADIUS`] blocs ?
    fn monster_nearby(&self, center: Vec3) -> bool {
        let registry = self
            .ecs
            .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
            .clone();
        let mut found = false;
        self.ecs
            .query::<(&Transform, &Mob)>()
            .for_each(|_, (transform, mob)| {
                let hostile = registry.get(mob.kind).is_some_and(|d| d.hostile);
                found |= hostile && transform.position.distance(center) < MONSTER_RADIUS;
            });
        found
    }

    /// Le message en cours, s'il n'a pas expiré.
    pub(crate) fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = &self.ui.toast else {
            return;
        };
        if since.elapsed().as_secs_f32() > TOAST_SECONDS {
            self.ui.toast = None;
            return;
        }
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(
                    0.0,
                    if self.mode == crate::Mode::Playing && !self.ui.inventory_open {
                        self.ui.target_bottom + 8.0
                    } else {
                        12.0
                    },
                ),
            )
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 60.0).clamp(100.0, 420.0));
                egui::Frame::new()
                    .fill(crate::skin::PANEL)
                    .stroke(egui::Stroke::new(1.0, crate::skin::COPPER))
                    .corner_radius(0)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).size(17.0).color(crate::skin::TEXT));
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
            ui.label(RichText::new(message).size(18.0).color(crate::skin::MUTED));
            ui.add_space(12.0);
            respawn = button(ui, "Réapparaître");
            let label = if self.store.is_some() {
                "Sauvegarder et retourner au menu"
            } else {
                "Retourner au menu"
            };
            quit = button(ui, label);
        });
        if respawn {
            self.send_now(crate::protocol::ClientMsg::Respawn);
        } else if quit {
            self.request = Some(Request::Title);
        }
    }
}
