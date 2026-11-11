//! L'enchantement : une propriété d'un outil ou d'une armure, achetée à la table
//! d'enchantement avec des niveaux d'expérience et des saphirs.
//!
//! Elle est rangée dans les bits hauts de l'usure (`ItemStack::damage`) : l'usure occupe
//! les 16 bits bas, le niveau les bits 16 à 23, l'enchantement les bits 24 à 31. Ainsi elle
//! suit l'objet partout (coffres, sauvegarde, inventaire) sans changer un seul format, et un
//! objet enchanté ne s'empile jamais (son « usure » n'est plus nulle).
//!
//! - **Efficacité** : mine plus vite (+30 % par niveau).
//! - **Tranchant** : plus de dégâts (+0,75 par niveau), pour l'épée et la hache.
//! - **Solidité** : l'objet s'use moins (une chance sur « niveau + 1 » d'être épargné).
//! - **Protection** : chaque pièce d'armure protège d'un point de plus par niveau.

use engine_voxel::{ItemDef, ToolKind};

/// L'usure : les 16 bits bas.
pub const WEAR_MASK: u32 = 0xFFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enchantment {
    Efficiency = 1,
    Sharpness = 2,
    Unbreaking = 3,
    Protection = 4,
}

impl Enchantment {
    pub const ALL: [Enchantment; 4] = [
        Self::Efficiency,
        Self::Sharpness,
        Self::Unbreaking,
        Self::Protection,
    ];

    fn from_id(id: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|e| *e as u32 == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Efficiency => "Efficacité",
            Self::Sharpness => "Tranchant",
            Self::Unbreaking => "Solidité",
            Self::Protection => "Protection",
        }
    }

    pub fn max_level(self) -> u8 {
        3
    }

    /// Cet enchantement convient-il à cet objet ?
    pub fn applies_to(self, item: &ItemDef) -> bool {
        match self {
            Self::Efficiency => item.tool.is_some_and(|t| {
                matches!(
                    t.kind,
                    ToolKind::Pickaxe
                        | ToolKind::Axe
                        | ToolKind::Shovel
                        | ToolKind::Hoe
                        | ToolKind::Shears
                )
            }),
            Self::Sharpness => item
                .tool
                .is_some_and(|t| matches!(t.kind, ToolKind::Sword | ToolKind::Axe)),
            Self::Unbreaking => item.durability.is_some(),
            Self::Protection => item.armor.is_some(),
        }
    }
}

/// L'usure seule d'une pile (sans l'enchantement).
pub fn wear(damage: u32) -> u32 {
    damage & WEAR_MASK
}

/// L'enchantement rangé dans une pile, et son niveau.
pub fn enchantment_of(damage: u32) -> Option<(Enchantment, u8)> {
    let level = ((damage >> 16) & 0xFF) as u8;
    let kind = Enchantment::from_id(damage >> 24)?;
    (level > 0).then_some((kind, level))
}

/// Le même `damage`, avec cet enchantement (au niveau voulu) et la même usure.
pub fn with_enchantment(damage: u32, kind: Enchantment, level: u8) -> u32 {
    wear(damage) | (u32::from(level) << 16) | ((kind as u32) << 24)
}

/// Le niveau d'un enchantement précis (0 si absent).
pub fn level_of(damage: u32, kind: Enchantment) -> u8 {
    enchantment_of(damage).map_or(0, |(k, l)| if k == kind { l } else { 0 })
}

/// « Efficacité II ».
pub fn label(kind: Enchantment, level: u8) -> String {
    const ROMAN: [&str; 4] = ["", "I", "II", "III"];
    format!("{} {}", kind.name(), ROMAN[usize::from(level).min(3)])
}

/// Une proposition de la table : l'enchantement, son niveau et ce qu'il coûte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    pub kind: Enchantment,
    pub level: u8,
    /// Niveaux d'expérience dépensés.
    pub levels: u32,
    /// Saphirs dépensés.
    pub gems: u32,
}

/// Les trois propositions pour un objet. L'enchantement proposé dépend de l'objet et de la
/// graine de la table (elle change après chaque enchantement) ; plus on paie, plus le niveau
/// est haut. `None` si l'objet ne peut rien recevoir, ou s'il est déjà enchanté.
pub fn offers(item: &ItemDef, damage: u32, seed: u32) -> Option<[Offer; 3]> {
    if enchantment_of(damage).is_some() {
        return None;
    }
    let fitting: Vec<Enchantment> = Enchantment::ALL
        .into_iter()
        .filter(|e| e.applies_to(item))
        .collect();
    if fitting.is_empty() {
        return None;
    }
    let mut state = u64::from(seed) ^ (item.key.len() as u64).wrapping_mul(0x9E37_79B9);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let pick = |i: usize, next: &mut dyn FnMut() -> usize| {
        let kind = fitting[next() % fitting.len()];
        Offer {
            kind,
            level: (i as u8 + 1).min(kind.max_level()),
            levels: 2 * i as u32 + 1,
            gems: i as u32 + 1,
        }
    };
    Some([pick(0, &mut next), pick(1, &mut next), pick(2, &mut next)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn the_enchantment_lives_beside_the_wear() {
        let damage = with_enchantment(17, Enchantment::Sharpness, 2);
        assert_eq!(wear(damage), 17);
        assert_eq!(enchantment_of(damage), Some((Enchantment::Sharpness, 2)));
        assert_eq!(level_of(damage, Enchantment::Sharpness), 2);
        assert_eq!(level_of(damage, Enchantment::Efficiency), 0);
        // Plain objects have none; wear alone never looks like an enchantment.
        assert_eq!(enchantment_of(0), None);
        assert_eq!(enchantment_of(123), None);
        // Re-enchanting keeps the wear.
        assert_eq!(
            wear(with_enchantment(damage, Enchantment::Unbreaking, 1)),
            17
        );
        assert_eq!(label(Enchantment::Efficiency, 3), "Efficacité III");
    }

    #[test]
    fn each_enchantment_fits_the_right_items() {
        let items = items();
        let get = |key: &str| items.get(items.id(key).unwrap());
        assert!(Enchantment::Efficiency.applies_to(get("core:iron_pickaxe")));
        assert!(!Enchantment::Efficiency.applies_to(get("core:iron_sword")));
        assert!(Enchantment::Sharpness.applies_to(get("core:iron_sword")));
        assert!(Enchantment::Sharpness.applies_to(get("core:iron_axe")));
        assert!(!Enchantment::Sharpness.applies_to(get("core:iron_pickaxe")));
        assert!(Enchantment::Protection.applies_to(get("core:iron_helmet")));
        assert!(!Enchantment::Protection.applies_to(get("core:iron_sword")));
        assert!(Enchantment::Unbreaking.applies_to(get("core:iron_helmet")));
        // Un bloc ou un aliment ne reçoit rien.
        assert!(offers(get("core:stone"), 0, 1).is_none());
        assert!(offers(get("core:bread"), 0, 1).is_none());
    }

    #[test]
    fn offers_grow_with_the_price_and_change_with_the_table_seed() {
        let items = items();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let offers = offers(sword, 0, 7).unwrap();
        for (i, offer) in offers.iter().enumerate() {
            assert!(offer.kind.applies_to(sword));
            assert_eq!(offer.level as usize, i + 1);
            assert_eq!(offer.gems as usize, i + 1);
            assert_eq!(offer.levels as usize, 2 * i + 1);
        }
        let again = super::offers(sword, 0, 7).unwrap();
        assert_eq!(offers, again, "déterministe");
        let kinds = |seed| super::offers(sword, 0, seed).unwrap().map(|o| o.kind as u8);
        assert!(
            (0..40).any(|s| kinds(s) != kinds(7)),
            "la graine change l'offre"
        );
        // Déjà enchanté : plus rien à proposer.
        let enchanted = with_enchantment(0, Enchantment::Sharpness, 1);
        assert!(super::offers(sword, enchanted, 7).is_none());
    }

    #[test]
    fn unbreaking_spares_wear_and_protection_adds_up() {
        use crate::inventory::{Inventory, ItemStack};
        let items = items();
        let pickaxe = items.id("core:iron_pickaxe").unwrap();
        let helmet = items.id("core:iron_helmet").unwrap();
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack {
            item: pickaxe,
            count: 1,
            damage: with_enchantment(0, Enchantment::Unbreaking, 3),
        });
        // Solidité III : un tirage bas épargne l'objet, un tirage haut non.
        assert!(!inventory.wear_selected(&items, 0.1));
        assert_eq!(wear(inventory.slots[0].unwrap().damage), 0);
        assert!(!inventory.wear_selected(&items, 0.95));
        assert_eq!(wear(inventory.slots[0].unwrap().damage), 1);
        // L'enchantement survit à l'usure.
        assert_eq!(
            enchantment_of(inventory.slots[0].unwrap().damage),
            Some((Enchantment::Unbreaking, 3))
        );
        // Protection : +niveau par pièce.
        let plain = ItemStack::new(helmet, 1);
        inventory.armor[0] = Some(plain);
        let base = inventory.protection(&items);
        inventory.armor[0] = Some(ItemStack {
            damage: with_enchantment(0, Enchantment::Protection, 2),
            ..plain
        });
        assert_eq!(inventory.protection(&items), base + 2);
    }

    #[test]
    fn enchanted_items_never_stack_and_survive_saving() {
        use crate::inventory::{Inventory, ItemStack};
        let items = items();
        let sword = items.id("core:iron_sword").unwrap();
        let mut inventory = Inventory::default();
        let enchanted = ItemStack {
            item: sword,
            count: 1,
            damage: with_enchantment(5, Enchantment::Sharpness, 2),
        };
        inventory.slots[3] = Some(enchanted);
        let saved = inventory.to_save(&items);
        let back = Inventory::from_save(&saved, &items);
        assert_eq!(back.slots[3], Some(enchanted));
    }

    #[test]
    fn spending_levels_keeps_the_progress_towards_the_next() {
        use crate::survival::Experience;
        let mut xp = Experience { points: 0 };
        xp.add(Experience::cost(0) + Experience::cost(1) + Experience::cost(2) + 3);
        assert_eq!(xp.level(), (3, 3));
        assert!(!xp.clone().spend_levels(4));
        assert!(xp.spend_levels(2));
        assert_eq!(xp.level().0, 1);
        assert_eq!(xp.level().1, 3, "l'avancement est gardé");
        assert!(xp.spend_levels(1));
        assert_eq!(xp.level().0, 0);
    }
}
