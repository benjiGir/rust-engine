//! Les potions et leurs effets : boire une potion (comme on mange) lance un effet qui dure
//! quelques dizaines de secondes. Les effets agissent sur la marche, les dégâts, la vie ou
//! la résistance au feu ; l'écran en affiche la liste et le temps restant.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use engine_voxel::{ItemId, ItemRegistry};
use serde::Deserialize;

const BUILTIN_POTIONS: &str = include_str!("../../../assets/core/potions.ron");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Effect {
    Healing,
    Regeneration,
    Speed,
    Strength,
    FireResistance,
}

impl Effect {
    pub fn name(self) -> &'static str {
        match self {
            Self::Healing => "Soin",
            Self::Regeneration => "Régénération",
            Self::Speed => "Vitesse",
            Self::Strength => "Force",
            Self::FireResistance => "Résistance au feu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PotionDef {
    pub effect: Effect,
    pub seconds: f32,
    pub power: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PotionsFile {
    potions: HashMap<String, PotionDef>,
}

/// Les potions du jeu, par objet.
#[derive(Debug, Clone, Default)]
pub struct Potions {
    by_item: HashMap<ItemId, PotionDef>,
}

impl Potions {
    pub fn builtin(items: &ItemRegistry) -> Result<Self> {
        Self::parse(BUILTIN_POTIONS, "potions.ron intégré", items)
    }

    pub fn parse(text: &str, origin: &str, items: &ItemRegistry) -> Result<Self> {
        let file: PotionsFile =
            ron::from_str(text).with_context(|| format!("{origin} : RON invalide"))?;
        let mut by_item = HashMap::new();
        for (key, def) in file.potions {
            let id = items
                .id(&key)
                .with_context(|| format!("{origin} : objet inconnu « {key} »"))?;
            by_item.insert(id, def);
        }
        Ok(Self { by_item })
    }

    pub fn get(&self, item: ItemId) -> Option<PotionDef> {
        self.by_item.get(&item).copied()
    }
}

/// Un effet en cours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Active {
    pub effect: Effect,
    pub left: f32,
    pub power: f32,
}

/// Les effets qui agissent en ce moment sur le joueur.
#[derive(Debug, Clone, Default)]
pub struct Effects {
    active: Vec<Active>,
    /// Le temps depuis le dernier point de régénération.
    regen_timer: f32,
}

impl Effects {
    /// Boire une potion. Renvoie les points de vie à rendre tout de suite (Soin).
    pub fn drink(&mut self, potion: PotionDef) -> f32 {
        if potion.effect == Effect::Healing {
            return potion.power;
        }
        // Reboire le même effet le renouvelle (le plus long gagne).
        match self.active.iter_mut().find(|a| a.effect == potion.effect) {
            Some(a) => {
                a.left = a.left.max(potion.seconds);
                a.power = a.power.max(potion.power);
            }
            None => self.active.push(Active {
                effect: potion.effect,
                left: potion.seconds,
                power: potion.power,
            }),
        }
        0.0
    }

    /// Fait passer `dt` secondes. Renvoie les points de vie régénérés.
    pub fn update(&mut self, dt: f32) -> f32 {
        for a in &mut self.active {
            a.left -= dt;
        }
        self.active.retain(|a| a.left > 0.0);
        let Some(regen) = self.power(Effect::Regeneration) else {
            self.regen_timer = 0.0;
            return 0.0;
        };
        self.regen_timer += dt;
        let mut healed = 0.0;
        while self.regen_timer >= 2.0 {
            self.regen_timer -= 2.0;
            healed += regen;
        }
        healed
    }

    pub fn power(&self, effect: Effect) -> Option<f32> {
        self.active
            .iter()
            .find(|a| a.effect == effect)
            .map(|a| a.power)
    }

    pub fn has(&self, effect: Effect) -> bool {
        self.power(effect).is_some()
    }

    pub fn list(&self) -> &[Active] {
        &self.active
    }

    pub fn clear(&mut self) {
        self.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn every_potion_exists_and_every_brew_makes_one() {
        let items = items();
        let potions = Potions::builtin(&items).unwrap();
        let healing = items.id("core:potion_healing").unwrap();
        assert_eq!(potions.get(healing).unwrap().effect, Effect::Healing);
        // Chaque recette d'alambic produit une potion décrite ici.
        let brewing = engine_voxel::Brewing::builtin(&items).unwrap();
        assert!(brewing.recipes().len() >= 5);
        for recipe in brewing.recipes() {
            assert!(
                potions.get(recipe.output).is_some(),
                "{} n'a pas d'effet",
                items.get(recipe.output).key
            );
        }
    }

    #[test]
    fn healing_is_instant_and_the_others_last_then_fade() {
        let mut effects = Effects::default();
        let heal = PotionDef {
            effect: Effect::Healing,
            seconds: 0.0,
            power: 8.0,
        };
        assert_eq!(effects.drink(heal), 8.0);
        assert!(effects.list().is_empty());

        let speed = PotionDef {
            effect: Effect::Speed,
            seconds: 10.0,
            power: 0.3,
        };
        assert_eq!(effects.drink(speed), 0.0);
        assert_eq!(effects.power(Effect::Speed), Some(0.3));
        effects.update(9.0);
        assert!(effects.has(Effect::Speed));
        // Reboire : la durée est renouvelée, pas doublée.
        effects.drink(speed);
        effects.update(9.0);
        assert!(effects.has(Effect::Speed));
        effects.update(2.0);
        assert!(!effects.has(Effect::Speed));
        assert_eq!(effects.power(Effect::Speed), None);
    }

    #[test]
    fn regeneration_heals_over_time_and_strength_adds_damage() {
        let mut effects = Effects::default();
        effects.drink(PotionDef {
            effect: Effect::Regeneration,
            seconds: 30.0,
            power: 1.0,
        });
        let healed: f32 = (0..20).map(|_| effects.update(1.0)).sum();
        assert_eq!(healed, 10.0, "1 point toutes les 2 s pendant 20 s");
        effects.drink(PotionDef {
            effect: Effect::Strength,
            seconds: 5.0,
            power: 3.0,
        });
        assert_eq!(effects.power(Effect::Strength), Some(3.0));
    }
}
