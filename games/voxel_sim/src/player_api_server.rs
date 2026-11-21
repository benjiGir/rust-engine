//! La part serveur de `player_api.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `player_api.rs`).

use crate::components::{Collider, Health, Mob, PreviousPosition, Transform, Velocity};
use crate::player;
use engine_mods::ModEvent;
use engine_mods::player::PlayerSnapshot;
use engine_voxel::MachineAction;
use glam::Vec3;

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
        self.send_correct(self.players.current_id(), false);
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
    pub fn player_snapshot(&self) -> PlayerSnapshot {
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
    pub fn apply_player_action(&mut self, action: MachineAction) {
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
    pub fn use_mod_item(&mut self) -> bool {
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

/// Une jauge de mod à l'écran.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HudBar {
    pub label: String,
    pub fraction: f32,
    pub color: [f32; 3],
}

/// Un script ne téléporte pas le joueur plus loin que cela d'un coup (blocs).
pub const MAX_TELEPORT: f32 = 64.0;
