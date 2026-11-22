//! Les effets des enchantements qui ne sont pas de simples bonus de combat : Fortune, Toucher de
//! soie, Aspiration, Épines et Légèreté. Ils lisent tous un **attribut** (voir
//! `player/attributes.rs`), rempli par les enchantements de l'objet en main ou de l'armure portée
//! (voir `player/gear.rs`) : ce module ne connaît aucun enchantement par son nom, seulement ce que
//! les attributs disent.
//!
//! - **Fortune** : un minerai (un bloc qui donne de l'expérience) lâche de 1 à « niveau + 1 » fois
//!   son butin ([`fortune_count`]) ;
//! - **Toucher de soie** : un bloc cassé avec le bon outil lâche le bloc lui-même ;
//! - **Aspiration** : le butin d'un bloc cassé, ou d'une créature tuée à côté du joueur, va
//!   directement dans l'inventaire (ce qui n'y tient pas tombe par terre) ;
//! - **Épines** : une créature qui frappe le joueur reçoit des dégâts ;
//! - **Légèreté** : une part des dégâts de chute est évitée.

use engine_voxel::{BlockId, ToolKind};
use glam::{IVec3, Vec3};

use crate::components::{Mob, Transform};
use crate::player::attributes::Attribute;
use crate::player::inventory::ItemStack;
use crate::player::survival::DamageCause;

/// À quelle distance d'une créature qui frappe les épines renvoient leurs dégâts, en blocs.
const THORNS_RANGE: f32 = 3.5;

/// Le nombre d'objets d'un minerai avec Fortune : de 1 à « niveau + 1 » fois, tiré avec `roll`
/// (de 0 à 1, exclu).
pub fn fortune_count(count: u32, level: u32, roll: f32) -> u32 {
    if level == 0 {
        return count;
    }
    let factor = 1 + (roll * (level + 1) as f32).floor().min(level as f32) as u32;
    count * factor
}

/// Les dégâts de chute après Légèreté (`resist` : la part évitée, de 0 à 1).
pub fn fall_after_feather(amount: f32, resist: f32) -> f32 {
    amount * (1.0 - resist.clamp(0.0, 1.0))
}

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Ce que lâche un bloc, par terre à sa place (ou dans l'inventaire avec Aspiration).
    pub fn drop_loot(&mut self, pos: IVec3, block: BlockId, tool: Option<ToolKind>, harvest: bool) {
        let seed = self.next_random();
        let mut rng = engine_voxel::Rng64::new(seed);
        let def = self.registry.def(block).clone();
        // Toucher de soie : le bloc lui-même (jamais un fluide ni un bloc de passage).
        let silk = tool.is_some()
            && harvest
            && self.players.me_mut().gear.value(Attribute::SilkTouch) > 0.0
            && def.placeable
            && !def.liquid
            && !def.unbreakable;
        let loot = if silk {
            let base = self.registry.def(def.base.unwrap_or(block));
            self.content
                .items
                .id(&base.key)
                .map(|item| vec![(item, 1)])
                .unwrap_or_default()
        } else {
            self.content
                .loot
                .roll(block, tool, harvest, &self.content.items, &mut rng)
        };
        // Fortune : les minerais.
        let fortune = self.players.me_mut().gear.value(Attribute::Fortune).round() as u32;
        let ore = def.xp != (0, 0);
        for (item, count) in loot {
            let count = if ore && !silk {
                fortune_count(count, fortune, rng.next_f32())
            } else {
                count
            };
            let count = self.loot_count(count);
            self.give_loot(pos, ItemStack::new(item, count), true);
        }
    }

    /// Une créature vient de frapper le joueur depuis `from` : les Épines lui renvoient des dégâts.
    pub fn reflect_thorns(&mut self, from: Vec3) {
        let thorns = self.players.me_mut().gear.value(Attribute::Thorns);
        if thorns <= 0.0 {
            return;
        }
        let mut nearest: Option<(f32, engine_ecs::Entity)> = None;
        self.ecs
            .query::<(&Transform, &Mob)>()
            .for_each(|entity, (transform, _)| {
                let distance = transform.position.distance(from);
                if distance <= THORNS_RANGE && nearest.is_none_or(|(best, _)| distance < best) {
                    nearest = Some((distance, entity));
                }
            });
        if let Some((_, mob)) = nearest {
            let at = crate::player::body(&self.ecs, self.players.entity()).position;
            crate::mobs::damage_mob(&mut self.ecs, mob, thorns, at);
            let me = self.players.local_id();
            self.ecs.insert(mob, crate::player::players::LastHitBy(me));
        }
    }

    /// Lâche un butin : directement dans l'inventaire si le joueur a Aspiration (et que `near` dit
    /// que le butin est à sa portée), par terre sinon ou pour ce qui ne tient pas.
    pub fn give_loot(&mut self, pos: IVec3, stack: ItemStack, near: bool) {
        if stack.count == 0 {
            return;
        }
        let stack = if near && self.players.me_mut().gear.value(Attribute::Magnet) > 0.0 {
            let left = self.collect_stack(stack);
            if left == 0 {
                return;
            }
            ItemStack {
                count: left,
                ..stack
            }
        } else {
            stack
        };
        self.drop_stack(pos, stack);
    }

    /// Les dégâts d'un coup de l'environnement, après Légèreté pour une chute.
    pub fn environment_damage(&self, amount: f32, cause: DamageCause) -> f32 {
        if cause == DamageCause::Fall {
            fall_after_feather(amount, self.players.me().gear.value(Attribute::FallResist))
        } else {
            amount
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fortune_multiplies_by_one_to_level_plus_one() {
        // Sans Fortune, rien ne change.
        assert_eq!(fortune_count(2, 0, 0.99), 2);
        // Niveau 3 : un facteur de 1 à 4, selon le tirage.
        let factors: Vec<u32> = [0.0, 0.26, 0.5, 0.76, 0.999]
            .into_iter()
            .map(|roll| fortune_count(1, 3, roll))
            .collect();
        assert_eq!(factors, vec![1, 2, 3, 4, 4]);
        assert_eq!(fortune_count(3, 2, 0.99), 9, "multiplie la pile entière");
        // Un tirage tout en haut ne dépasse jamais le maximum.
        assert_eq!(fortune_count(1, 1, 1.0), 2);
    }

    #[test]
    fn feather_removes_a_share_of_the_fall_damage() {
        assert_eq!(fall_after_feather(10.0, 0.0), 10.0);
        assert!((fall_after_feather(10.0, 0.36) - 6.4).abs() < 1e-5);
        assert_eq!(fall_after_feather(10.0, 2.0), 0.0, "borné à 100 %");
    }
}
