//! Les gestes des charmes de potion dans le jeu : activer d'un clic droit, recharger avec la potion,
//! user les charmes actifs chaque seconde. Les règles (bits, usure, bulle d'aide) sont dans `charms.rs`.

use crate::VoxelGame;
use crate::charms::{charge, is_active, is_empty, recharged, wear_one, with_active};
use crate::inventory::{Click, ItemStack, Slot};
use crate::potions::{Effect, PotionDef};

/// Combien de secondes un effet de charme dure après son dernier renouvellement.
const REFRESH_SECONDS: f32 = 3.0;
/// Le délai entre deux soins d'un charme de soin, en secondes.
const HEAL_EVERY: u32 = 10;

impl VoxelGame {
    /// Un clic droit avec un charme en main : l'active ou le désactive. `true` si c'était un charme.
    pub(crate) fn use_charm(&mut self) -> bool {
        let Some(stack) = self.players.me_mut().inventory.selected_stack() else {
            return false;
        };
        let data = crate::adventure_data::current();
        let items = self.content.items.clone();
        let def = items.get(stack.item);
        if data.charm_of(&def.key).is_none() {
            return false;
        }
        let durability = def.durability.unwrap_or(1);
        if is_empty(stack.damage, durability) {
            self.toast("Le charme est vide : recharge-le avec sa potion");
            return true;
        }
        let active = !is_active(stack.damage);
        let selected = self.players.me_mut().inventory.selected;
        if let Some(held) = self.players.me_mut().inventory.slots[selected].as_mut() {
            held.damage = with_active(held.damage, active);
        }
        self.toast(&format!(
            "{} {}",
            def.name,
            if active { "activé" } else { "désactivé" }
        ));
        true
    }

    /// Une potion au curseur, un clic sur un charme qu'elle recharge. `true` si c'est fait.
    pub(crate) fn try_charm_recharge(&mut self, slot: Slot, click: Click) -> bool {
        if click != Click::Primary {
            return false;
        }
        let (Some(held), Some(target)) = (
            self.players.me_mut().inventory.cursor,
            *self.players.me_mut().inventory.slot_mut(slot),
        ) else {
            return false;
        };
        let data = crate::adventure_data::current();
        let items = self.content.items.clone();
        let Some(charm) = data.charm_of(&items.get(target.item).key) else {
            return false;
        };
        if items.id(&charm.potion) != Some(held.item) {
            return false;
        }
        let durability = items.get(target.item).durability.unwrap_or(1);
        if charge(target.damage, durability) >= 1.0 {
            self.toast("Ce charme est déjà plein");
            return true;
        }
        if let Some(stack) = self.players.me_mut().inventory.slot_mut(slot) {
            stack.damage = recharged(stack.damage);
        }
        if let Some(cursor) = self.players.me_mut().inventory.cursor.as_mut() {
            cursor.count -= 1;
            if cursor.count == 0 {
                self.players.me_mut().inventory.cursor = None;
            }
        }
        // La fiole vide est rendue.
        if let Some(flask) = items.id("core:flask") {
            let left = self.collect_stack(ItemStack::new(flask, 1));
            if left > 0 {
                let at = crate::player::body(&self.ecs, self.players.entity())
                    .position
                    .floor()
                    .as_ivec3();
                self.drop_stack(at, ItemStack::new(flask, left));
            }
        }
        self.toast(&format!("{} rechargé", items.get(target.item).name));
        self.bell();
        true
    }

    /// Chaque seconde : les charmes actifs de l'inventaire renouvellent leur effet et s'usent.
    pub(crate) fn update_charms(&mut self, dt: f32) {
        self.client.enchant_ui.charm_clock += dt;
        if self.client.enchant_ui.charm_clock < 1.0 {
            return;
        }
        self.client.enchant_ui.charm_clock -= 1.0;
        self.client.enchant_ui.charm_beats += 1;
        let data = crate::adventure_data::current();
        if data.charms.is_empty() {
            return;
        }
        let items = self.content.items.clone();
        let mut emptied = Vec::new();
        let mut heal = 0.0;
        let me = self.players.me_mut();
        for slot in me.inventory.slots.iter_mut().flatten() {
            if !is_active(slot.damage) {
                continue;
            }
            let def = items.get(slot.item);
            let Some(charm) = data.charm_of(&def.key) else {
                continue;
            };
            let Some(potion) = items.id(&charm.potion).and_then(|id| self.potions.get(id)) else {
                continue;
            };
            match potion.effect {
                Effect::Healing => {
                    if self
                        .client
                        .enchant_ui
                        .charm_beats
                        .is_multiple_of(HEAL_EVERY)
                    {
                        heal += potion.power * 0.5;
                    }
                }
                _ => {
                    me.effects.drink(PotionDef {
                        seconds: REFRESH_SECONDS,
                        ..potion
                    });
                }
            }
            if wear_one(&mut slot.damage, def.durability.unwrap_or(1)) {
                emptied.push(def.name.clone());
            }
        }
        if heal > 0.0 {
            self.heal(heal);
        }
        for name in emptied {
            self.toast(&format!("{name} est épuisé"));
        }
    }
}
