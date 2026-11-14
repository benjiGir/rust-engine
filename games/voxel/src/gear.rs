//! L'équipement du joueur vu comme des attributs : ce module rassemble les sources (objet en
//! main, armure portée, potions), garde les [`Attributes`] calculés, et les branche dans le jeu
//! (combat, armure, minage, marche, butin, portée, expérience). Les formules sont dans
//! `attributes.rs` (pur et testé) ; ici, le lien avec l'inventaire et le monde.
//!
//! # Quand on recalcule
//!
//! Pas à chaque image : [`Gear::refresh`] compare une **empreinte** de l'équipement (pile en main
//! et pièces d'armure, sans leur usure ; les effets de potion actifs ; la révision du magasin des
//! fiches) à la dernière, et ne recalcule que si elle a changé. Une usure qui monte ou un effet qui
//! s'écoule ne changent donc rien ; changer d'objet en main, enchanter, enfiler une armure ou
//! boire une potion si.
//!
//! # Les sources
//!
//! - **Objet en main** (un outil seulement) : ses dégâts de base (au-dessus du coup de poing), les
//!   effets de ses enchantements « en main » (Tranchant, Efficacité, Butin, Fortune...) et les
//!   affixes de sa fiche ;
//! - **Armure portée** : les points de protection de chaque pièce, les effets de ses enchantements
//!   « portés » (Protection, Épines, Légèreté, Vivacité...) et les affixes de sa fiche ;
//! - **Potions** : Force (dégâts en plus), Vitesse (marche en plus). Régénération et Soin gardent
//!   leur propre mécanique (voir `potions.rs`).
//!
//! Les gemmes serties dans les emplacements de la fiche ajoutent leurs bonus au même endroit (voir
//! `gems.rs`) ; leur effet dépend de la catégorie de l'objet qui les porte.

use std::hash::{Hash, Hasher};

use engine_ecs::Entity;
use engine_voxel::ItemRegistry;
use glam::Vec3;

use crate::VoxelGame;
use crate::attributes::{self, Attribute, Attributes, BaseAttributes, Modifier, Source};
use crate::components::{Health, Mob};
use crate::enchant_data::Use;
use crate::enchanting;
use crate::inventory::{Inventory, ItemStack};
use crate::item_data;
use crate::potions::{Effect, Effects};

/// Les attributs du joueur et ce qu'il faut pour les garder à jour.
pub struct Gear {
    base: BaseAttributes,
    attrs: Attributes,
    signature: Option<u64>,
}

/// Le résultat d'un coup du joueur sur une créature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strike {
    /// Les dégâts infligés (après critique et armure de la cible).
    pub damage: f32,
    pub crit: bool,
    /// La vie rendue par le vol de vie.
    pub healed: f32,
}

impl Gear {
    pub fn new(base: BaseAttributes) -> Self {
        Self {
            base,
            attrs: Attributes::plain(base),
            signature: None,
        }
    }

    pub fn attrs(&self) -> &Attributes {
        &self.attrs
    }

    pub fn value(&self, attribute: Attribute) -> f32 {
        self.attrs.value(attribute)
    }

    /// Les points d'armure portés.
    pub fn armor(&self) -> f32 {
        self.value(Attribute::Armor)
    }

    /// La portée du bras, en blocs.
    pub fn reach(&self) -> f32 {
        self.value(Attribute::Reach)
    }

    /// Le facteur de vitesse de marche.
    pub fn move_speed(&self) -> f32 {
        self.value(Attribute::MoveSpeed)
    }

    /// Recalcule les attributs si l'équipement a changé depuis le dernier appel. `true` si c'est
    /// le cas.
    pub fn refresh(
        &mut self,
        inventory: &Inventory,
        items: &ItemRegistry,
        effects: &Effects,
    ) -> bool {
        let signature = signature(inventory, effects);
        if self.signature == Some(signature) {
            return false;
        }
        self.signature = Some(signature);
        self.attrs = Attributes::compute(self.base, sources(inventory, items, effects, &self.base));
        true
    }
}

/// L'empreinte de ce qui compte pour les attributs (voir la doc du module).
pub fn signature(inventory: &Inventory, effects: &Effects) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // L'usure d'une pile simple vit dans les bits bas de `damage` : on ne la regarde pas.
    let key = |stack: &Option<ItemStack>| {
        stack.map(|s| {
            let damage = if item_data::is_extended(s.damage) {
                s.damage
            } else {
                s.damage & !enchanting::WEAR_MASK
            };
            (s.item, damage)
        })
    };
    key(&inventory.selected_stack()).hash(&mut hasher);
    for piece in &inventory.armor {
        key(piece).hash(&mut hasher);
    }
    for active in effects.list() {
        (active.effect, active.power.to_bits()).hash(&mut hasher);
    }
    item_data::revision().hash(&mut hasher);
    hasher.finish()
}

/// Les modificateurs d'une pile : les enchantements et les affixes de sa fiche.
fn stack_modifiers(
    stack: &ItemStack,
    items: &ItemRegistry,
    held: bool,
    base: &BaseAttributes,
) -> Vec<Modifier> {
    let def = items.get(stack.item);
    let mut modifiers = Vec::new();
    if held && let Some(tool) = def.tool {
        // Un outil frappe au moins aussi fort que le poing : seul l'excédent est un bonus.
        modifiers.push(Modifier::flat(
            Attribute::Damage,
            (tool.damage - base.damage).max(0.0),
        ));
    }
    if let Some(armor) = def.armor
        && !held
    {
        // Les points de l'armure ; Protection s'y ajoute par ses effets (ci-dessous).
        modifiers.push(Modifier::flat(Attribute::Armor, armor.protection as f32));
    }
    // Les enchantements : leurs effets sont dans les données (`enchantments.ron`), un bonus
    // d'attribut par niveau, compté quand l'objet est en main (outil) ou porté (armure).
    let data = crate::adventure_data::current();
    for (kind, level) in enchanting::enchantments_of(stack.damage) {
        let Some(enchant) = data.enchant_by_number(kind.id()) else {
            continue;
        };
        for effect in &enchant.effects {
            let counts = match effect.on {
                Use::Held => held && def.tool.is_some(),
                Use::Worn => !held && def.armor.is_some(),
            };
            if counts {
                let level = f32::from(level);
                modifiers.push(Modifier {
                    attribute: effect.attribute,
                    flat: effect.flat * level,
                    percent: effect.percent * level,
                });
            }
        }
    }
    // Les affixes et les gemmes d'un outil en main ou d'une pièce portée.
    if (held && def.tool.is_some() || !held && def.armor.is_some())
        && let Some(sheet) = item_data::sheet(stack.damage)
    {
        for affix in &sheet.affixes {
            if let Some(attribute) = Attribute::from_id(affix.attribute) {
                modifiers.push(Modifier {
                    attribute,
                    flat: affix.flat,
                    percent: affix.percent,
                });
            }
        }
        modifiers.extend(crate::gems::modifiers(
            &crate::adventure_data::current(),
            def,
            &sheet,
        ));
    }
    modifiers
}

/// Toutes les sources du joueur, dans l'ordre : la main, l'armure (tête aux pieds), les potions.
pub fn sources(
    inventory: &Inventory,
    items: &ItemRegistry,
    effects: &Effects,
    base: &BaseAttributes,
) -> Vec<Source> {
    let mut out = Vec::new();
    let mut add = |stack: &ItemStack, held: bool| {
        let mut modifiers = stack_modifiers(stack, items, held, base);
        modifiers.retain(|m| m.flat != 0.0 || m.percent != 0.0);
        if !modifiers.is_empty() {
            out.push(Source {
                label: sheet_name(stack).unwrap_or_else(|| items.get(stack.item).name.clone()),
                modifiers,
            });
        }
    };
    if let Some(stack) = inventory.selected_stack() {
        add(&stack, true);
    }
    for piece in inventory.armor.iter().flatten() {
        add(piece, false);
    }
    for active in effects.list() {
        let modifier = match active.effect {
            Effect::Strength => Modifier::flat(Attribute::Damage, active.power),
            Effect::Speed => Modifier::percent(Attribute::MoveSpeed, active.power),
            _ => continue,
        };
        out.push(Source {
            label: active.effect.name().to_string(),
            modifiers: vec![modifier],
        });
    }
    out
}

/// Le nom donné par la fiche (objets à affixes), s'il y en a un.
fn sheet_name(stack: &ItemStack) -> Option<String> {
    item_data::sheet(stack.damage).and_then(|s| s.name)
}

/// Le branchement dans le jeu.
impl VoxelGame {
    /// À chaque pas : recalcule les attributs si l'équipement a changé, aligne la vie maximale du
    /// joueur, et rend la vie de la régénération.
    pub(crate) fn refresh_gear(&mut self, dt: f32) {
        self.gear
            .refresh(&self.inventory, &self.content.items, &self.effects);
        let max = self.gear.value(Attribute::MaxHealth);
        let regen = self.gear.value(Attribute::Regen);
        if let Some(mut health) = self.ecs.get_mut::<Health>(self.player) {
            if (health.max - max).abs() > f32::EPSILON {
                health.max = max;
                health.current = health.current.min(max);
            }
            if regen > 0.0 && health.current > 0.0 {
                health.current = (health.current + regen * dt).min(health.max);
            }
        }
    }

    /// Les points d'armure d'une créature (0 par défaut).
    fn mob_armor(&self, mob: Entity) -> f32 {
        self.ecs
            .get::<Mob>(mob)
            .and_then(|m| {
                self.ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .get(m.kind)
                    .map(|d| d.armor)
            })
            .unwrap_or(0.0)
            + crate::elites::armor_of(&self.ecs, mob)
    }

    /// Un coup du joueur sur une créature : critique, armure de la cible (que la perforation
    /// contourne), recul, puis vol de vie. `None` si la créature vient d'être frappée.
    pub(crate) fn strike_mob(&mut self, mob: Entity, from: Vec3) -> Option<Strike> {
        let roll = engine_voxel::Rng64::new(self.next_random()).next_f32();
        let attrs = self.gear.attrs();
        let (damage, crit) = attributes::roll_hit(
            attrs.value(Attribute::Damage),
            attrs.value(Attribute::CritChance),
            attrs.value(Attribute::CritDamage),
            roll,
        );
        let dealt = attributes::damage_after_armor(
            damage,
            self.mob_armor(mob),
            attrs.pierce(),
            self.survival.armor_per_point,
            self.survival.armor_max,
        );
        let steal = attrs.value(Attribute::LifeSteal);
        if !crate::mobs::damage_mob(&mut self.ecs, mob, dealt, from) {
            return None;
        }
        let healed = attributes::life_steal(dealt, steal);
        if healed > 0.0 {
            self.heal(healed);
        }
        if crit {
            self.toast("Coup critique !");
        }
        Some(Strike {
            damage: dealt,
            crit,
            healed,
        })
    }

    /// Le butin d'une pile avec la chance de butin du joueur.
    pub(crate) fn loot_count(&mut self, count: u32) -> u32 {
        let loot = self.gear.value(Attribute::LootChance);
        if loot <= 0.0 {
            return count;
        }
        let roll = engine_voxel::Rng64::new(self.next_random()).next_f32();
        attributes::bonus_count(count, loot, roll)
    }

    /// L'expérience gagnée avec le gain d'expérience du joueur.
    pub(crate) fn scaled_xp(&self, points: u32) -> u32 {
        attributes::scaled_xp(points, self.gear.value(Attribute::XpGain))
    }

    /// Sauvegarde les fiches d'objets dans la racine du monde (si quelque chose a changé).
    pub(crate) fn save_item_data(&mut self) {
        let Some(root) = self.dims.meta_store.clone().or_else(|| self.store.clone()) else {
            return;
        };
        if let Err(err) = item_data::save_if_dirty(root.dir()) {
            log::error!("sauvegarde des fiches d'objets : {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchanting::{Enchantment, with_enchantment};
    use crate::potions::PotionDef;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn stack(items: &ItemRegistry, key: &str, damage: u32) -> ItemStack {
        ItemStack {
            item: items.id(key).unwrap(),
            count: 1,
            damage,
        }
    }

    fn compute(inventory: &Inventory, items: &ItemRegistry, effects: &Effects) -> Attributes {
        let base = BaseAttributes::default();
        Attributes::compute(base, sources(inventory, items, effects, &base))
    }

    #[test]
    fn bare_handed_the_player_has_the_base_attributes() {
        let items = items();
        let attrs = compute(&Inventory::default(), &items, &Effects::default());
        for attribute in Attribute::ALL {
            assert!(!attrs.is_modified(attribute), "{attribute:?}");
        }
        assert_eq!(attrs.value(Attribute::Damage), 4.0);
    }

    #[test]
    fn the_held_tool_its_enchantments_and_potions_add_up() {
        let items = items();
        let mut inventory = Inventory::default();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let sword_damage = sword.tool.unwrap().damage;
        // Tranchant II + Solidité I : deux enchantements, donc une fiche.
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::SHARPNESS, 2),
            Enchantment::UNBREAKING,
            1,
        );
        assert!(item_data::is_extended(damage));
        inventory.slots[0] = Some(stack(&items, "core:iron_sword", damage));
        let attrs = compute(&inventory, &items, &Effects::default());
        // Les dégâts de l'épée remplacent le poing (4), plus 0,75 par niveau de Tranchant.
        assert!((attrs.value(Attribute::Damage) - (sword_damage + 1.5)).abs() < 1e-5);
        // Avec une potion de Force : +3.
        let mut effects = Effects::default();
        effects.drink(PotionDef {
            effect: Effect::Strength,
            seconds: 30.0,
            power: 3.0,
        });
        effects.drink(PotionDef {
            effect: Effect::Speed,
            seconds: 30.0,
            power: 0.2,
        });
        let attrs = compute(&inventory, &items, &effects);
        assert!((attrs.value(Attribute::Damage) - (sword_damage + 4.5)).abs() < 1e-5);
        assert!((attrs.value(Attribute::MoveSpeed) - 1.2).abs() < 1e-5);
        let detail = attrs.contributions(Attribute::Damage);
        assert_eq!(detail.len(), 2, "l'épée et la Force");
        assert_eq!(detail[1].label, Effect::Strength.name());
        // Efficacité : du minage plus rapide sur une pioche, sans toucher aux dégâts.
        inventory.slots[0] = Some(stack(
            &items,
            "core:iron_pickaxe",
            with_enchantment(0, Enchantment::EFFICIENCY, 2),
        ));
        let attrs = compute(&inventory, &items, &Effects::default());
        assert!((attrs.value(Attribute::MiningSpeed) - 1.6).abs() < 1e-5);
    }

    #[test]
    fn armor_pieces_add_their_protection_and_enchantments() {
        let items = items();
        let mut inventory = Inventory::default();
        inventory.armor[0] = Some(stack(&items, "core:iron_helmet", 0));
        let plain = compute(&inventory, &items, &Effects::default());
        // Le même total que `Inventory::protection`.
        assert_eq!(
            plain.value(Attribute::Armor),
            inventory.protection(&items) as f32
        );
        assert!(plain.value(Attribute::Armor) > 0.0);
        inventory.armor[0] = Some(stack(
            &items,
            "core:iron_helmet",
            with_enchantment(0, Enchantment::PROTECTION, 2),
        ));
        let enchanted = compute(&inventory, &items, &Effects::default());
        assert_eq!(
            enchanted.value(Attribute::Armor),
            plain.value(Attribute::Armor) + 2.0
        );
        assert_eq!(
            enchanted.value(Attribute::Armor),
            inventory.protection(&items) as f32
        );
        // Une armure tenue en main ne protège pas.
        let mut held = Inventory::default();
        held.slots[0] = inventory.armor[0];
        let attrs = compute(&held, &items, &Effects::default());
        assert_eq!(attrs.value(Attribute::Armor), 0.0);
    }

    #[test]
    fn the_new_enchantments_become_attributes_through_their_data() {
        let items = items();
        let mut inventory = Inventory::default();
        // Une épée : Butin III, Vol de vie II, Aspiration.
        let sword = [
            (Enchantment::LOOTING, 3),
            (Enchantment::LIFE_STEAL, 2),
            (Enchantment::MAGNET, 1),
        ]
        .iter()
        .fold(0, |d, &(k, l)| with_enchantment(d, k, l));
        inventory.slots[0] = Some(stack(&items, "core:iron_sword", sword));
        let attrs = compute(&inventory, &items, &Effects::default());
        assert!((attrs.value(Attribute::LootChance) - 0.6).abs() < 1e-5);
        assert!((attrs.value(Attribute::LifeSteal) - 0.06).abs() < 1e-5);
        assert_eq!(attrs.value(Attribute::Magnet), 1.0);
        assert_eq!(attrs.value(Attribute::Fortune), 0.0, "pas pour une épée");
        // Une pioche : Fortune II et Toucher de soie.
        let pickaxe = with_enchantment(
            with_enchantment(0, Enchantment::FORTUNE, 2),
            Enchantment::SILK_TOUCH,
            1,
        );
        inventory.slots[0] = Some(stack(&items, "core:iron_pickaxe", pickaxe));
        let attrs = compute(&inventory, &items, &Effects::default());
        assert_eq!(attrs.value(Attribute::Fortune), 2.0);
        assert_eq!(attrs.value(Attribute::SilkTouch), 1.0);
        assert_eq!(attrs.value(Attribute::LootChance), 0.0);
        // Des bottes : Légèreté IV, Vivacité II, Épines III. Elles comptent portées, pas en main.
        let boots = [
            (Enchantment::FEATHER, 4),
            (Enchantment::SWIFTNESS, 2),
            (Enchantment::THORNS, 3),
        ]
        .iter()
        .fold(0, |d, &(k, l)| with_enchantment(d, k, l));
        inventory.slots[0] = Some(stack(&items, "core:iron_boots", boots));
        let held = compute(&inventory, &items, &Effects::default());
        assert_eq!(held.value(Attribute::Thorns), 0.0, "tenues en main : rien");
        inventory.slots[0] = None;
        inventory.armor[3] = Some(stack(&items, "core:iron_boots", boots));
        let worn = compute(&inventory, &items, &Effects::default());
        assert!((worn.value(Attribute::FallResist) - 0.48).abs() < 1e-5);
        assert!((worn.value(Attribute::MoveSpeed) - 1.1).abs() < 1e-5);
        assert_eq!(worn.value(Attribute::Thorns), 3.0);
        // La Légèreté plafonne à 90 % même avec beaucoup de pièces (bornes de l'attribut).
        assert!(Attribute::FallResist.bounds().1 <= 0.9);
    }

    #[test]
    fn affixes_of_a_sheet_become_modifiers() {
        let items = items();
        let mut damage = 0;
        assert!(enchanting::edit_sheet(&mut damage, |sheet| {
            sheet.name = Some("Épée de la tempête".into());
            sheet.affixes.push(item_data::Affix {
                key: "tempête".into(),
                attribute: Attribute::CritChance.id(),
                flat: 0.2,
                percent: 0.0,
            });
            sheet.affixes.push(item_data::Affix {
                key: "inconnu".into(),
                attribute: 250,
                flat: 9.0,
                percent: 0.0,
            });
        }));
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(stack(&items, "core:iron_sword", damage));
        let attrs = compute(&inventory, &items, &Effects::default());
        assert!((attrs.value(Attribute::CritChance) - 0.2).abs() < 1e-5);
        let detail = attrs.contributions(Attribute::CritChance);
        assert_eq!(detail[0].label, "Épée de la tempête", "le nom de la fiche");
    }

    #[test]
    fn the_signature_ignores_wear_but_not_a_change_of_gear() {
        let items = items();
        let effects = Effects::default();
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(stack(
            &items,
            "core:iron_pickaxe",
            with_enchantment(0, Enchantment::EFFICIENCY, 1),
        ));
        let first = signature(&inventory, &effects);
        // L'usure d'une pile simple ne change rien.
        let worn = with_enchantment(5, Enchantment::EFFICIENCY, 1);
        inventory.slots[0] = Some(stack(&items, "core:iron_pickaxe", worn));
        assert_eq!(signature(&inventory, &effects), first);
        // Un autre enchantement, si.
        inventory.slots[0] = Some(stack(
            &items,
            "core:iron_pickaxe",
            with_enchantment(5, Enchantment::EFFICIENCY, 2),
        ));
        assert_ne!(signature(&inventory, &effects), first);
        // Une armure en plus, si.
        let before = signature(&inventory, &effects);
        inventory.armor[1] = Some(stack(&items, "core:iron_chestplate", 0));
        assert_ne!(signature(&inventory, &effects), before);
    }

    #[test]
    fn refresh_only_recomputes_when_the_gear_changes() {
        let items = items();
        let effects = Effects::default();
        let mut gear = Gear::new(BaseAttributes::default());
        let mut inventory = Inventory::default();
        assert!(gear.refresh(&inventory, &items, &effects), "premier calcul");
        assert!(!gear.refresh(&inventory, &items, &effects));
        inventory.slots[0] = Some(stack(&items, "core:iron_sword", 0));
        assert!(gear.refresh(&inventory, &items, &effects));
        assert!(gear.value(Attribute::Damage) > 4.0);
        assert!(!gear.refresh(&inventory, &items, &effects));
        // Un coup d'usure sur l'épée : rien à recalculer.
        inventory.wear_selected(&items, 0.99);
        assert!(!gear.refresh(&inventory, &items, &effects));
    }
}
