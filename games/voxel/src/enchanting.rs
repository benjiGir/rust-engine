//! L'enchantement : des propriétés d'un outil ou d'une armure, achetées à la table
//! d'enchantement avec des niveaux d'expérience et des saphirs.
//!
//! # Où elles sont rangées
//!
//! Un objet peut avoir **plusieurs** enchantements, mais ils tiennent dans `ItemStack::damage`
//! de deux façons, pour que les anciens objets restent valides :
//!
//! - **pile simple** (le bit 31 est à 0) : un seul enchantement, dans les bits hauts de l'usure.
//!   L'usure occupe les 16 bits bas, le niveau les bits 16 à 23, l'enchantement les bits 24 à 30
//!   (les sortes restent sous 128, donc le bit 31 n'est jamais touché). C'est le format des
//!   mondes déjà joués ;
//! - **pile étendue** (le bit 31 est à 1) : `damage` ne porte qu'un numéro de fiche, et la fiche
//!   (voir `item_data.rs`) contient l'usure et la liste des enchantements.
//!
//! Un objet passe de la première forme à la seconde (usure et enchantement recopiés) dès qu'il
//! reçoit un deuxième enchantement ([`with_enchantment`], [`edit_sheet`]). Dans les deux cas
//! l'enchantement suit l'objet partout (coffres, sauvegarde, inventaire), et un objet enchanté
//! ne s'empile jamais (son `damage` n'est plus nul).
//!
//! # Les effets
//!
//! Ils passent par les attributs (voir `attributes.rs`) :
//!
//! - **Efficacité** : mine plus vite (+30 % par niveau).
//! - **Tranchant** : plus de dégâts (+0,75 par niveau), pour l'épée et la hache.
//! - **Solidité** : l'objet s'use moins (une chance sur « niveau + 1 » d'être épargné).
//! - **Protection** : chaque pièce d'armure protège d'un point de plus par niveau.
//!
//! La table ([`offers`]) propose trois lots ; le plus cher donne jusqu'à trois enchantements
//! différents, dans la limite de [`MAX_ENCHANTMENTS`] par objet.

use engine_voxel::{ItemDef, ToolKind};

use crate::item_data::{self, ItemData};

/// L'usure : les 16 bits bas.
pub const WEAR_MASK: u32 = 0xFFFF;
/// Le plus d'enchantements qu'un objet puisse porter.
pub const MAX_ENCHANTMENTS: usize = 4;

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

    /// Le numéro stable de la sorte (celui qu'on sauvegarde).
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u32) -> Option<Self> {
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

/// L'usure d'une pile (sans les enchantements).
pub fn wear(damage: u32) -> u32 {
    if item_data::is_extended(damage) {
        item_data::sheet(damage).map_or(0, |s| s.wear)
    } else {
        damage & WEAR_MASK
    }
}

/// Ajoute de l'usure à une pile, simple ou étendue (la fiche porte alors l'usure).
pub fn add_wear(damage: &mut u32, amount: u32) {
    if item_data::is_extended(*damage) {
        let number = item_data::number(*damage);
        item_data::write(|s| {
            if let Some(sheet) = s.edit(number, false) {
                sheet.wear += amount;
            }
        });
    } else {
        *damage += amount;
    }
}

/// Libère la fiche d'un objet détruit (sans effet sur une pile simple).
pub fn release(damage: u32) {
    if item_data::is_extended(damage) {
        item_data::write(|s| s.remove(item_data::number(damage)));
    }
}

/// L'enchantement unique d'une pile simple, et son niveau.
fn legacy_enchantment(damage: u32) -> Option<(Enchantment, u8)> {
    if item_data::is_extended(damage) {
        return None;
    }
    let level = ((damage >> 16) & 0xFF) as u8;
    let kind = Enchantment::from_id(damage >> 24)?;
    (level > 0).then_some((kind, level))
}

/// La fiche qu'aurait cette pile simple : son usure et son enchantement recopiés.
fn legacy_sheet(damage: u32) -> ItemData {
    ItemData {
        wear: damage & WEAR_MASK,
        enchantments: legacy_enchantment(damage)
            .into_iter()
            .map(|(kind, level)| (kind.id(), level))
            .collect(),
        ..ItemData::default()
    }
}

/// Tous les enchantements d'une pile (simple ou étendue), dans l'ordre où ils ont été reçus.
pub fn enchantments_of(damage: u32) -> Vec<(Enchantment, u8)> {
    if !item_data::is_extended(damage) {
        return legacy_enchantment(damage).into_iter().collect();
    }
    item_data::sheet(damage)
        .map(|sheet| {
            sheet
                .enchantments
                .iter()
                .filter_map(|&(id, level)| {
                    Enchantment::from_id(u32::from(id)).map(|kind| (kind, level))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// La pile a-t-elle au moins un enchantement ?
pub fn is_enchanted(damage: u32) -> bool {
    !enchantments_of(damage).is_empty()
}

/// Le niveau d'un enchantement précis (0 si absent).
pub fn level_of(damage: u32, kind: Enchantment) -> u8 {
    if !item_data::is_extended(damage) {
        return legacy_enchantment(damage).map_or(0, |(k, l)| if k == kind { l } else { 0 });
    }
    enchantments_of(damage)
        .into_iter()
        .find(|(k, _)| *k == kind)
        .map_or(0, |(_, l)| l)
}

/// Modifie la fiche d'une pile, en la créant si c'est une pile simple (c'est la **migration** :
/// l'usure et l'enchantement existants y sont recopiés, et `damage` devient le numéro de fiche).
/// Si la pile pointait une fiche perdue, une fiche neuve la remplace. `false` si le magasin est
/// plein (rien n'est changé).
pub fn edit_sheet(damage: &mut u32, f: impl FnOnce(&mut ItemData)) -> bool {
    if item_data::is_extended(*damage) {
        let number = item_data::number(*damage);
        let mut f = Some(f);
        let done = item_data::write(|s| {
            s.edit(number, true)
                .map(|sheet| (f.take().expect("appelée une fois"))(sheet))
                .is_some()
        });
        if done {
            return true;
        }
        let Some(f) = f else { return true };
        return create_sheet(damage, ItemData::default(), f);
    }
    create_sheet(damage, legacy_sheet(*damage), f)
}

fn create_sheet(damage: &mut u32, mut data: ItemData, f: impl FnOnce(&mut ItemData)) -> bool {
    f(&mut data);
    match item_data::write(|s| s.create(data)) {
        Some(number) => {
            *damage = item_data::EXTENDED | number;
            true
        }
        None => false,
    }
}

/// Le même `damage`, avec cet enchantement (au niveau voulu) en plus : le niveau est remplacé si
/// la sorte y était déjà. Une pile simple sans enchantement (ou avec la même sorte) reste simple ;
/// une pile simple qui en a déjà un autre migre vers une fiche.
pub fn with_enchantment(damage: u32, kind: Enchantment, level: u8) -> u32 {
    let simple = !item_data::is_extended(damage)
        && legacy_enchantment(damage).is_none_or(|(existing, _)| existing == kind);
    if simple {
        return (damage & WEAR_MASK) | (u32::from(level) << 16) | (u32::from(kind.id()) << 24);
    }
    let mut damage = damage;
    edit_sheet(&mut damage, |sheet| {
        match sheet
            .enchantments
            .iter_mut()
            .find(|(id, _)| *id == kind.id())
        {
            Some(existing) => existing.1 = level,
            None => sheet.enchantments.push((kind.id(), level)),
        }
    });
    damage
}

/// « Efficacité II ».
pub fn label(kind: Enchantment, level: u8) -> String {
    const ROMAN: [&str; 4] = ["", "I", "II", "III"];
    format!("{} {}", kind.name(), ROMAN[usize::from(level).min(3)])
}

/// Les libellés de tous les enchantements d'une pile (pour les bulles d'aide).
pub fn labels(damage: u32) -> Vec<String> {
    enchantments_of(damage)
        .into_iter()
        .map(|(kind, level)| label(kind, level))
        .collect()
}

/// Un lot de la table : des enchantements (plusieurs à partir du deuxième lot) et ce qu'il coûte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub enchants: Vec<(Enchantment, u8)>,
    /// Niveaux d'expérience dépensés.
    pub levels: u32,
    /// Saphirs dépensés.
    pub gems: u32,
}

impl Offer {
    /// « Efficacité II + Solidité II ».
    pub fn label(&self) -> String {
        self.enchants
            .iter()
            .map(|&(kind, level)| label(kind, level))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Le `damage` de la pile une fois ce lot appliqué.
    pub fn apply(&self, damage: u32) -> u32 {
        self.enchants
            .iter()
            .fold(damage, |d, &(kind, level)| with_enchantment(d, kind, level))
    }
}

/// Les trois lots pour un objet. Les enchantements proposés dépendent de l'objet, de ceux qu'il
/// porte déjà (on ne propose jamais deux fois la même sorte) et de la graine de la table (elle
/// change après chaque enchantement). Plus on paie, plus le niveau est haut, et le deuxième et le
/// troisième lot donnent jusqu'à deux et trois enchantements. `None` si l'objet ne peut plus rien
/// recevoir.
pub fn offers(item: &ItemDef, damage: u32, seed: u32) -> Option<[Offer; 3]> {
    let owned = enchantments_of(damage);
    let room = MAX_ENCHANTMENTS.saturating_sub(owned.len());
    let fitting: Vec<Enchantment> = Enchantment::ALL
        .into_iter()
        .filter(|e| e.applies_to(item) && !owned.iter().any(|(k, _)| k == e))
        .collect();
    if fitting.is_empty() || room == 0 {
        return None;
    }
    let mut state = u64::from(seed) ^ (item.key.len() as u64).wrapping_mul(0x9E37_79B9);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let mut lot = |i: usize| {
        let mut pool = fitting.clone();
        let count = (i + 1).min(pool.len()).min(room);
        let enchants = (0..count)
            .map(|_| {
                let kind = pool.remove(next() % pool.len());
                (kind, (i as u8 + 1).min(kind.max_level()))
            })
            .collect();
        Offer {
            enchants,
            levels: 2 * i as u32 + 1,
            gems: i as u32 + 1,
        }
    };
    Some([lot(0), lot(1), lot(2)])
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
        assert_eq!(enchantments_of(damage), vec![(Enchantment::Sharpness, 2)]);
        assert_eq!(level_of(damage, Enchantment::Sharpness), 2);
        assert_eq!(level_of(damage, Enchantment::Efficiency), 0);
        // Plain objects have none; wear alone never looks like an enchantment.
        assert!(!is_enchanted(0));
        assert!(!is_enchanted(123));
        // Re-enchanting keeps the wear.
        assert_eq!(
            wear(with_enchantment(damage, Enchantment::Unbreaking, 1)),
            17
        );
        // Le même enchantement, à un autre niveau : remplacé, et la pile reste simple.
        let again = with_enchantment(damage, Enchantment::Sharpness, 3);
        assert_eq!(enchantments_of(again), vec![(Enchantment::Sharpness, 3)]);
        assert!(!item_data::is_extended(again));
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
    fn legacy_kinds_never_touch_the_extended_bit() {
        // Les anciens objets : une sorte (bits 24 à 30) et un niveau, jamais le bit 31.
        for kind in Enchantment::ALL {
            for level in 1..=kind.max_level() {
                let damage = with_enchantment(65_535, kind, level);
                assert!(!item_data::is_extended(damage), "{kind:?} {level}");
                assert_eq!(wear(damage), 65_535);
                assert_eq!(enchantments_of(damage), vec![(kind, level)]);
            }
        }
        // Le plus grand numéro de sac à dos reste lui aussi un objet simple.
        assert!(!item_data::is_extended(crate::backpacks::MAX_NUMBER));
        assert_eq!(wear(0), 0);
    }

    #[test]
    fn a_second_enchantment_migrates_the_stack_to_a_sheet() {
        // Un vieil objet : usure 17, Tranchant II dans les bits hauts.
        let old = with_enchantment(17, Enchantment::Sharpness, 2);
        assert!(!item_data::is_extended(old));
        let migrated = with_enchantment(old, Enchantment::Unbreaking, 1);
        assert!(item_data::is_extended(migrated));
        assert_ne!(migrated, old);
        // L'usure et le premier enchantement sont recopiés, le second s'ajoute.
        assert_eq!(wear(migrated), 17);
        assert_eq!(
            enchantments_of(migrated),
            vec![(Enchantment::Sharpness, 2), (Enchantment::Unbreaking, 1)]
        );
        assert_eq!(level_of(migrated, Enchantment::Sharpness), 2);
        assert_eq!(level_of(migrated, Enchantment::Unbreaking), 1);
        assert_eq!(level_of(migrated, Enchantment::Efficiency), 0);
        // Un troisième s'ajoute dans la même fiche ; le niveau d'une sorte se remplace.
        let third = with_enchantment(migrated, Enchantment::Efficiency, 3);
        assert_eq!(third, migrated, "même numéro de fiche");
        let raised = with_enchantment(third, Enchantment::Sharpness, 3);
        assert_eq!(level_of(raised, Enchantment::Sharpness), 3);
        assert_eq!(enchantments_of(raised).len(), 3);
        // L'usure d'une pile étendue passe par la fiche.
        let mut damage = raised;
        add_wear(&mut damage, 4);
        assert_eq!(wear(damage), 21);
        assert_eq!(damage, raised);
        // Un objet détruit libère sa fiche : il n'y a plus rien à lire.
        release(damage);
        assert_eq!(wear(damage), 0);
        assert!(enchantments_of(damage).is_empty());
    }

    #[test]
    fn a_sheet_with_no_enchantment_yet_is_made_from_a_plain_stack() {
        let mut damage = 9;
        assert!(edit_sheet(&mut damage, |sheet| sheet.rarity = 2));
        assert!(item_data::is_extended(damage));
        assert_eq!(wear(damage), 9);
        assert_eq!(item_data::sheet(damage).unwrap().rarity, 2);
        // Une fiche perdue (fichier supprimé) : on repart d'une fiche neuve.
        let lost = item_data::EXTENDED | 0x7FFF_FFF0;
        let mut damage = lost;
        assert!(edit_sheet(&mut damage, |sheet| sheet.wear = 3));
        assert_ne!(damage, lost);
        assert_eq!(wear(damage), 3);
        assert_eq!(wear(lost), 0);
    }

    #[test]
    fn offers_give_several_distinct_enchantments_and_skip_those_owned() {
        let items = items();
        let axe = items.get(items.id("core:iron_axe").unwrap());
        let lots = offers(axe, 0, 11).unwrap();
        // Une hache accepte trois sortes : le troisième lot les donne toutes, une seule fois.
        assert_eq!(lots[0].enchants.len(), 1);
        assert_eq!(lots[1].enchants.len(), 2);
        assert_eq!(lots[2].enchants.len(), 3);
        for lot in &lots {
            let mut kinds: Vec<u8> = lot.enchants.iter().map(|(k, _)| k.id()).collect();
            kinds.sort_unstable();
            kinds.dedup();
            assert_eq!(kinds.len(), lot.enchants.len(), "pas de doublon");
            assert!(lot.enchants.iter().all(|(k, _)| k.applies_to(axe)));
        }
        // Appliquer le troisième lot donne trois enchantements sur la même pile.
        let damage = lots[2].apply(5);
        assert_eq!(enchantments_of(damage).len(), 3);
        assert_eq!(wear(damage), 5);
        assert!(lots[2].label().contains(" + "));
        // Ce que l'objet a déjà n'est plus proposé, et quand il ne reste rien, plus d'offre.
        let after = offers(axe, damage, 11);
        assert!(after.is_none(), "la hache a reçu toutes ses sortes");
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let one = with_enchantment(0, Enchantment::Sharpness, 1);
        let rest = offers(sword, one, 3).unwrap();
        for lot in &rest {
            assert_eq!(lot.enchants.len(), 1, "seule Solidité reste");
            assert_eq!(lot.enchants[0].0, Enchantment::Unbreaking);
        }
    }

    #[test]
    fn offers_grow_with_the_price_and_change_with_the_table_seed() {
        let items = items();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let offers = offers(sword, 0, 7).unwrap();
        for (i, offer) in offers.iter().enumerate() {
            assert_eq!(offer.enchants.len(), (i + 1).min(2), "épée : 2 sortes");
            for (kind, level) in &offer.enchants {
                assert!(kind.applies_to(sword));
                assert_eq!(*level as usize, i + 1);
            }
            assert_eq!(offer.gems as usize, i + 1);
            assert_eq!(offer.levels as usize, 2 * i + 1);
        }
        let again = super::offers(sword, 0, 7).unwrap();
        assert_eq!(offers, again, "déterministe");
        let kinds = |seed| {
            super::offers(sword, 0, seed)
                .unwrap()
                .map(|o| o.enchants[0].0.id())
        };
        assert!(
            (0..40).any(|s| kinds(s) != kinds(7)),
            "la graine change l'offre"
        );
        // Déjà enchanté : la même sorte n'est plus proposée.
        let enchanted = with_enchantment(0, Enchantment::Sharpness, 1);
        for lot in super::offers(sword, enchanted, 7).unwrap() {
            assert!(
                lot.enchants
                    .iter()
                    .all(|(k, _)| *k != Enchantment::Sharpness)
            );
        }
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
            enchantments_of(inventory.slots[0].unwrap().damage),
            vec![(Enchantment::Unbreaking, 3)]
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
    fn wear_goes_through_the_sheet_and_a_broken_tool_frees_it() {
        use crate::inventory::{Inventory, ItemStack};
        let items = items();
        let pickaxe = items.id("core:iron_pickaxe").unwrap();
        let durability = items.get(pickaxe).durability.unwrap();
        // Deux enchantements : la pile est étendue, l'usure vit dans la fiche.
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::Efficiency, 1),
            Enchantment::Unbreaking,
            3,
        );
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack {
            item: pickaxe,
            count: 1,
            damage,
        });
        assert!(
            !inventory.wear_selected(&items, 0.1),
            "Solidité III épargne"
        );
        assert!(!inventory.wear_selected(&items, 0.99));
        let stack = inventory.slots[0].unwrap();
        assert_eq!(stack.damage, damage, "le numéro ne bouge pas");
        assert_eq!(wear(stack.damage), 1);
        assert_eq!(enchantments_of(stack.damage).len(), 2);
        // Presque usé : le dernier coup la brise, et la fiche est libérée.
        add_wear(
            &mut inventory.slots[0].as_mut().unwrap().damage,
            durability - 2,
        );
        assert!(inventory.wear_selected(&items, 0.99));
        assert!(inventory.slots[0].is_none());
        assert!(item_data::sheet(damage).is_none());
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
    fn old_worlds_and_old_enchanted_items_stay_valid() {
        use crate::inventory::Inventory;
        let items = items();
        // Un `world.ron` d'avant les fiches : une épée enchantée Tranchant II, usure 5, dont le
        // `damage` est celui de l'ancien format (sorte 2 << 24, niveau 2 << 16, usure 5), et une
        // pioche sans le champ `damage`.
        let text = "(palette: [\"core:iron_sword\", \"core:iron_pickaxe\"], \
            slots: [Some((item: 0, count: 1, damage: 33685509)), Some((item: 1, count: 1))])";
        let saved: engine_voxel::SavedInventory = ron::from_str(text).unwrap();
        let inventory = Inventory::from_save(&saved, &items);
        let sword = inventory.slots[0].unwrap();
        assert_eq!(sword.damage, 33_685_509);
        assert!(!item_data::is_extended(sword.damage));
        assert_eq!(wear(sword.damage), 5);
        assert_eq!(
            enchantments_of(sword.damage),
            vec![(Enchantment::Sharpness, 2)]
        );
        assert_eq!(inventory.slots[1].unwrap().damage, 0);
        // Il se sauvegarde à l'identique tant qu'il n'a pas reçu de deuxième enchantement...
        assert_eq!(
            inventory.to_save(&items).slots[0].unwrap().damage,
            33_685_509
        );
        // ... puis migre, et la pile étendue fait l'aller-retour par `world.ron` (RON).
        let mut inventory = inventory;
        let migrated = with_enchantment(sword.damage, Enchantment::Unbreaking, 2);
        inventory.slots[0].as_mut().unwrap().damage = migrated;
        let text = ron::to_string(&inventory.to_save(&items)).unwrap();
        let back = Inventory::from_save(&ron::from_str(&text).unwrap(), &items);
        assert_eq!(back.slots[0].unwrap().damage, migrated);
        assert_eq!(wear(migrated), 5);
        assert_eq!(
            enchantments_of(migrated),
            vec![(Enchantment::Sharpness, 2), (Enchantment::Unbreaking, 2)]
        );
    }

    #[test]
    fn extended_stacks_survive_chests_wagons_and_never_stack() {
        use crate::inventory::{Click, ItemStack, click_slot};
        let items = items();
        let sword = items.id("core:iron_sword").unwrap();
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::Sharpness, 1),
            Enchantment::Unbreaking,
            1,
        );
        assert!(item_data::is_extended(damage));
        // Un coffre ou un wagon range la pile en postcard : le `damage` passe tel quel.
        let stored = engine_voxel::ItemStack {
            item: "core:iron_sword".into(),
            count: 1,
            damage,
        };
        let bytes = postcard::to_allocvec(&stored).unwrap();
        let back: engine_voxel::ItemStack = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, stored);
        assert_eq!(
            enchantments_of(back.damage).len(),
            2,
            "la fiche suit la pile"
        );
        // Deux piles étendues ne fusionnent pas : un clic les échange.
        let a = ItemStack {
            item: sword,
            count: 1,
            damage,
        };
        let other = with_enchantment(
            with_enchantment(0, Enchantment::Sharpness, 3),
            Enchantment::Unbreaking,
            3,
        );
        let b = ItemStack { damage: other, ..a };
        let (mut slot, mut cursor) = (Some(a), Some(b));
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Primary);
        assert_eq!((slot, cursor), (Some(b), Some(a)));
        // La pile jetée au sol puis ramassée garde son numéro (c'est la même valeur).
        let mut inventory = crate::inventory::Inventory::default();
        assert_eq!(inventory.add_stack(a, &items), 0);
        assert_eq!(inventory.slots[0].unwrap().damage, damage);
    }

    #[test]
    fn backpacks_and_items_stores_live_side_by_side() {
        use crate::backpacks::BackpackStore;
        let dir = std::env::temp_dir().join(format!("voxel-stores-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let items = items();
        // Un sac numéroté, et une fiche d'objet : deux fichiers distincts, relus sans se gêner.
        let mut bags = BackpackStore::default();
        let mut bag =
            crate::inventory::ItemStack::new(items.id("core:backpack_leather").unwrap(), 1);
        let number = bags.number_for(&mut bag, 0).unwrap();
        bags.save(&dir).unwrap();
        let mut sheets = item_data::ItemStore::default();
        let sheet = sheets.create(ItemData::default()).unwrap();
        sheets.save(&dir).unwrap();
        assert!(dir.join("backpacks.bin").exists() && dir.join("items.bin").exists());
        assert!(BackpackStore::load(&dir).bag(number).is_some());
        assert!(item_data::ItemStore::load(&dir).get(sheet).is_some());
        // Un sac n'a jamais le bit 31 : il n'est pas pris pour une pile à fiche.
        assert!(!item_data::is_extended(bag.damage));
        let _ = std::fs::remove_dir_all(&dir);
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
