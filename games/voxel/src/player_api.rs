//! Ce que les scripts des mods peuvent faire du joueur : le lire (`player`), lui rendre de la vie,
//! le téléporter, lui donner un effet, blesser les créatures autour d'un point, afficher des
//! jauges à l'écran, et être prévenus d'un clic droit avec un objet `usable` en main.
//!
//! Le script lit un **instantané** du joueur (rafraîchi à chaque pas de simulation) ; ses ordres
//! reviennent au jeu comme les autres `MachineAction`, appliqués ici.

use std::collections::BTreeMap;

use engine_mods::ModEvent;
use engine_mods::player::PlayerSnapshot;
use engine_voxel::MachineAction;
use glam::Vec3;

use crate::VoxelGame;
use crate::components::{Collider, Health, Mob, PreviousPosition, Transform, Velocity};
use crate::player;

/// Au plus ce nombre de jauges par mod.
const MAX_BARS_PER_MOD: usize = 4;
/// Un script ne téléporte pas le joueur plus loin que cela d'un coup (blocs).
const MAX_TELEPORT: f32 = 64.0;

/// Une jauge de mod à l'écran.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HudBar {
    pub label: String,
    pub fraction: f32,
    pub color: [f32; 3],
}

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

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    fn teleport_player(&mut self, to: Vec3) {
        let from = player::body(&self.ecs, self.players.entity()).position;
        // Seulement vers un endroit chargé et pas trop loin : un script ne perd pas le joueur.
        let loaded = self
            .streamer()
            .world()
            .chunk(engine_voxel::World::chunk_containing(to))
            .is_some();
        if !loaded || from.distance(to) > MAX_TELEPORT {
            return;
        }
        if let Some(mut transform) = self.ecs.get_mut::<Transform>(self.players.entity()) {
            transform.position = to;
        }
        if let Some(mut previous) = self.ecs.get_mut::<PreviousPosition>(self.players.entity()) {
            previous.0 = to;
        }
        if let Some(mut velocity) = self.ecs.get_mut::<Velocity>(self.players.entity()) {
            velocity.0 = Vec3::ZERO;
        }
        // La fenêtre du joueur place sa caméra (et sa prédiction) au même endroit.
        let id = self.players.current_id();
        self.replies.push((
            id,
            crate::protocol::ServerMsg::Correct {
                position: to + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
            },
        ));
        self.forget_last_move();
    }

    /// Envoie une jauge de mod (ou son retrait) à tous les joueurs.
    fn send_mod_hud(&mut self, mod_id: String, id: u32, bar: Option<HudBar>) {
        let ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
        for player in ids {
            self.replies.push((
                player,
                crate::protocol::ServerMsg::ModHud {
                    mod_id: mod_id.clone(),
                    id,
                    bar: bar.clone(),
                },
            ));
        }
    }

    /// Blesse les créatures proches d'un point ; elles reculent à l'opposé.
    fn hurt_area(&mut self, center: Vec3, radius: f32, damage: f32) {
        let mut hit = Vec::new();
        self.ecs
            .query::<(&Transform, &Mob, &Health, &Collider)>()
            .for_each(|entity, (transform, _, _, collider)| {
                let middle = transform.position + Vec3::Y * (collider.height * 0.5);
                if middle.distance(center) <= radius {
                    hit.push((entity, (middle - center).normalize_or_zero()));
                }
            });
        for (entity, away) in hit {
            crate::mobs::damage_mob(&mut self.ecs, entity, damage, away);
        }
    }

    fn give_player_effect(&mut self, name: &str, seconds: f32, power: f32) {
        use crate::potions::{Effect, PotionDef};
        let effect = match name {
            "speed" => Effect::Speed,
            "strength" => Effect::Strength,
            "regeneration" => Effect::Regeneration,
            "fire_resistance" => Effect::FireResistance,
            "healing" => Effect::Healing,
            _ => return,
        };
        let healed = self.players.me_mut().effects.drink(PotionDef {
            effect,
            seconds,
            power,
        });
        if healed > 0.0 {
            self.heal(healed);
        }
    }

    /// L'état du joueur tel que les scripts le voient.
    pub(crate) fn player_snapshot(&self) -> PlayerSnapshot {
        let health = player::health(&self.ecs, self.players.entity());
        // La visée de l'action en cours (un clic droit avec un objet de mod), sinon le dernier regard
        // connu du joueur.
        let (eye, look) = match self.acting {
            Some(aim) => (aim.eye, aim.forward),
            None => (
                player::eye(&self.ecs, self.players.entity(), 1.0),
                self.players.me().look,
            ),
        };
        PlayerSnapshot {
            eye,
            look: look.normalize_or_zero(),
            health: health.current,
            max_health: health.max,
            level: self.players.me().experience.level().0 as f32,
        }
    }

    /// Applique un ordre de script concernant le joueur ou l'écran (les autres sont ailleurs).
    pub(crate) fn apply_player_action(&mut self, action: MachineAction) {
        match action {
            MachineAction::PlayerHeal { amount } => self.heal(amount),
            MachineAction::PlayerHurt { amount } => {
                let from = player::body(&self.ecs, self.players.entity()).position;
                crate::events::send(
                    &self.ecs,
                    crate::events::PlayerAttacked {
                        player: Some(self.players.local_id()),
                        amount,
                        from,
                        pierce: 0.0,
                    },
                );
            }
            MachineAction::QuestSignal { id } => self.players.me_mut().quest_book.note_signal(id),
            MachineAction::Say { text } => self.chat(&text),
            MachineAction::ConsumeHeld { count } => {
                for _ in 0..count {
                    self.players.me_mut().inventory.consume_selected();
                }
            }
            MachineAction::PlayerTeleport { position } => self.teleport_player(position),
            MachineAction::PlayerEffect {
                name,
                seconds,
                power,
            } => self.give_player_effect(&name, seconds, power),
            MachineAction::HurtArea {
                center,
                radius,
                damage,
            } => self.hurt_area(center, radius, damage),
            MachineAction::HudBar {
                mod_id,
                id,
                fraction,
                color,
                label,
            } => {
                // Les jauges d'un mod valent pour tous les joueurs : chaque fenêtre les range.
                let bar = HudBar {
                    label,
                    fraction,
                    color,
                };
                self.send_mod_hud(mod_id, id, Some(bar));
            }
            MachineAction::HudClear { mod_id, id } => self.send_mod_hud(mod_id, id, None),
            _ => {}
        }
    }

    /// Un clic droit avec un objet `usable` en main : les scripts abonnés à `ItemUsed` en sont
    /// prévenus. Renvoie vrai si un script l'a reçu (le clic est alors pris : on ne pose rien).
    pub(crate) fn use_mod_item(&mut self) -> bool {
        let Some(item) = self.players.me_mut().inventory.selected_item() else {
            return false;
        };
        if !self.content.items.get(item).usable
            || !self.mods.has_subscribers(engine_mods::abi::EVENT_ITEM_USED)
        {
            return false;
        }
        let snapshot = self.player_snapshot();
        self.mods.set_player(snapshot);
        let actions = self.mods.dispatch(&ModEvent::ItemUsed { item });
        self.apply_machine_actions(actions);
        self.punch();
        true
    }
}
