//! L'enchantement : des propriétés d'un outil ou d'une armure, achetées à la table
//! d'enchantement (voir `enchanting/table.rs`) avec des points d'expérience et des saphirs, ou
//! reçues d'un livre (voir `enchanting/books.rs`).
//!
//! # Où elles sont rangées
//!
//! Un objet peut avoir **plusieurs** enchantements, mais ils tiennent dans `ItemStack::damage`
//! de deux façons, pour que les anciens objets restent valides :
//!
//! - **pile simple** (le bit 31 est à 0) : un seul enchantement, dans les bits hauts de l'usure.
//!   L'usure occupe les 16 bits bas, le niveau les bits 16 à 23, l'enchantement les bits 24 à 30
//!   (les numéros restent sous 128, donc le bit 31 n'est jamais touché). C'est le format des
//!   mondes déjà joués ;
//! - **pile étendue** (le bit 31 est à 1) : `damage` ne porte qu'un numéro de fiche, et la fiche
//!   (voir `player/item_data.rs`) contient l'usure et la liste des enchantements.
//!
//! Un objet passe de la première forme à la seconde (usure et enchantement recopiés) dès qu'il
//! reçoit un deuxième enchantement ([`with_enchantment`], [`edit_sheet`]). Dans les deux cas
//! l'enchantement suit l'objet partout (coffres, sauvegarde, inventaire), et un objet enchanté
//! ne s'empile jamais (son `damage` n'est plus nul).
//!
//! # Les enchantements sont des données
//!
//! Chacun est décrit dans `enchantments.ron` (voir `enchanting/data.rs`) : son numéro stable, ses
//! niveaux (plafonnés selon l'Eterna de la table), ce qui peut le porter et ses effets, des bonus
//! d'attributs par niveau (voir `player/attributes.rs` et `player/gear.rs`). Douze sont fournis :
//! Efficacité, Tranchant, Solidité, Protection, Butin, Vol de vie, Fortune, Toucher de soie,
//! Aspiration, Épines, Légèreté et Vivacité. Solidité est la seule qui agit ailleurs que par les
//! attributs : l'objet s'use moins (une chance sur « niveau + 1 » d'être épargné).

pub mod book_play;
pub mod books;
pub mod data;
pub mod effects;
pub mod library;
pub mod table;
pub mod ui;

use engine_voxel::ItemDef;

use crate::player::item_data::{self, ItemData};

/// L'usure : les 16 bits bas.
pub const WEAR_MASK: u32 = 0xFFFF;

/// Une sorte d'enchantement : son numéro stable (voir `enchantments.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Enchantment(pub u8);

impl Enchantment {
    // Les sortes du jeu de base. Les données les décrivent toutes ; ces noms ne servent qu'au code
    // qui doit en désigner une (Solidité à l'usure) et aux tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const EFFICIENCY: Self = Self(1);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const SHARPNESS: Self = Self(2);
    pub const UNBREAKING: Self = Self(3);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const PROTECTION: Self = Self(4);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const LOOTING: Self = Self(5);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const LIFE_STEAL: Self = Self(6);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const FORTUNE: Self = Self(7);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const SILK_TOUCH: Self = Self(8);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const MAGNET: Self = Self(9);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const THORNS: Self = Self(10);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const FEATHER: Self = Self(11);
    #[cfg_attr(not(test), allow(dead_code))]
    pub const SWIFTNESS: Self = Self(12);

    /// Le numéro stable de la sorte (celui qu'on sauvegarde).
    pub fn id(self) -> u8 {
        self.0
    }

    /// La sorte de ce numéro, si les données la connaissent (un mod disparu : `None`).
    pub fn from_id(id: u32) -> Option<Self> {
        let number = u8::try_from(id).ok()?;
        crate::adventure::data::current()
            .enchant_by_number(number)
            .map(|_| Self(number))
    }

    /// Toutes les sortes connues, dans l'ordre des données.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn all() -> Vec<Self> {
        crate::adventure::data::current()
            .enchantments
            .iter()
            .map(|e| Self(e.number))
            .collect()
    }

    /// La sorte d'après son identifiant (`core:fortune`) ou son nom court (`fortune`).
    pub fn from_key(key: &str) -> Option<Self> {
        let data = crate::adventure::data::current();
        let full = if key.contains(':') {
            key.to_string()
        } else {
            format!("core:{key}")
        };
        data.enchant_by_id(&full).map(|e| Self(e.number))
    }

    fn with_def<R>(self, f: impl FnOnce(&crate::enchanting::data::EnchantDef) -> R) -> Option<R> {
        crate::adventure::data::current()
            .enchant_by_number(self.0)
            .map(f)
    }

    pub fn name(self) -> String {
        self.with_def(|d| d.name.clone())
            .unwrap_or_else(|| "Enchantement inconnu".into())
    }

    /// Le niveau le plus haut qu'un livre ou une bibliothèque porte.
    pub fn hard_max(self) -> u8 {
        self.with_def(|d| d.hard_max).unwrap_or(1)
    }

    /// Le niveau le plus haut que propose une table d'Eterna `eterna`.
    pub fn cap_for(self, eterna: f32) -> u8 {
        self.with_def(|d| d.cap_for(eterna)).unwrap_or(1)
    }

    /// Une sorte rare : la table ne la propose qu'avec une étagère « trésor ».
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_treasure(self) -> bool {
        self.with_def(|d| d.treasure).unwrap_or(false)
    }

    /// Cet enchantement convient-il à cet objet ?
    pub fn applies_to(self, item: &ItemDef) -> bool {
        self.with_def(|d| d.applies_to(item)).unwrap_or(false)
    }
}

/// L'usure d'une pile (sans les enchantements).
pub fn wear(damage: u32) -> u32 {
    if item_data::is_pending(damage) {
        0
    } else if item_data::is_extended(damage) {
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

/// Un nombre en chiffres romains (jusqu'à 39 ; au-delà, en chiffres).
pub fn roman(level: u8) -> String {
    const TENS: [&str; 4] = ["", "X", "XX", "XXX"];
    const ONES: [&str; 10] = ["", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX"];
    if level == 0 || level >= 40 {
        return level.to_string();
    }
    format!(
        "{}{}",
        TENS[usize::from(level / 10)],
        ONES[usize::from(level % 10)]
    )
}

/// « Efficacité II ». Une sorte qui n'a qu'un niveau (Toucher de soie) s'écrit sans chiffre.
pub fn label(kind: Enchantment, level: u8) -> String {
    if kind.hard_max() == 1 {
        return kind.name();
    }
    format!("{} {}", kind.name(), roman(level))
}

/// Les libellés de tous les enchantements d'une pile (pour les bulles d'aide).
pub fn labels(damage: u32) -> Vec<String> {
    enchantments_of(damage)
        .into_iter()
        .map(|(kind, level)| label(kind, level))
        .collect()
}

/// Retire tous les enchantements d'une pile (l'usure, les affixes et les gemmes restent) et les
/// rend. Une pile simple perd les bits hauts ; une pile étendue vide la liste de sa fiche.
pub fn strip_enchantments(damage: u32) -> (u32, Vec<(Enchantment, u8)>) {
    let taken = enchantments_of(damage);
    if taken.is_empty() {
        return (damage, taken);
    }
    if !item_data::is_extended(damage) {
        return (damage & WEAR_MASK, taken);
    }
    let mut damage = damage;
    edit_sheet(&mut damage, |sheet| sheet.enchantments.clear());
    (damage, taken)
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
        let damage = with_enchantment(17, Enchantment::SHARPNESS, 2);
        assert_eq!(wear(damage), 17);
        assert_eq!(enchantments_of(damage), vec![(Enchantment::SHARPNESS, 2)]);
        assert_eq!(level_of(damage, Enchantment::SHARPNESS), 2);
        assert_eq!(level_of(damage, Enchantment::EFFICIENCY), 0);
        // Plain objects have none; wear alone never looks like an enchantment.
        assert!(!is_enchanted(0));
        assert!(!is_enchanted(123));
        // Re-enchanting keeps the wear.
        assert_eq!(
            wear(with_enchantment(damage, Enchantment::UNBREAKING, 1)),
            17
        );
        // Le même enchantement, à un autre niveau : remplacé, et la pile reste simple.
        let again = with_enchantment(damage, Enchantment::SHARPNESS, 3);
        assert_eq!(enchantments_of(again), vec![(Enchantment::SHARPNESS, 3)]);
        assert!(!item_data::is_extended(again));
        assert_eq!(label(Enchantment::EFFICIENCY, 3), "Efficacité III");
        assert_eq!(label(Enchantment::EFFICIENCY, 10), "Efficacité X");
        assert_eq!(label(Enchantment::SILK_TOUCH, 1), "Toucher de soie");
        assert_eq!(roman(0), "0");
        assert_eq!(roman(39), "XXXIX");
    }

    #[test]
    fn each_enchantment_fits_the_right_items() {
        let items = items();
        let get = |key: &str| items.get(items.id(key).unwrap());
        assert!(Enchantment::EFFICIENCY.applies_to(get("core:iron_pickaxe")));
        assert!(!Enchantment::EFFICIENCY.applies_to(get("core:iron_sword")));
        assert!(Enchantment::SHARPNESS.applies_to(get("core:iron_sword")));
        assert!(Enchantment::SHARPNESS.applies_to(get("core:iron_axe")));
        assert!(!Enchantment::SHARPNESS.applies_to(get("core:iron_pickaxe")));
        assert!(Enchantment::PROTECTION.applies_to(get("core:iron_helmet")));
        assert!(!Enchantment::PROTECTION.applies_to(get("core:iron_sword")));
        assert!(Enchantment::UNBREAKING.applies_to(get("core:iron_helmet")));
        // Un bloc ou un aliment ne reçoit rien.
        for key in ["core:stone", "core:bread"] {
            assert!(
                Enchantment::all()
                    .into_iter()
                    .all(|e| !e.applies_to(get(key)))
            );
        }
        // Les nouveaux : Légèreté pour les bottes seulement, Vivacité pour jambes et bottes.
        assert!(Enchantment::FEATHER.applies_to(get("core:iron_boots")));
        assert!(!Enchantment::FEATHER.applies_to(get("core:iron_helmet")));
        assert!(Enchantment::SWIFTNESS.applies_to(get("core:iron_leggings")));
        assert!(!Enchantment::SWIFTNESS.applies_to(get("core:iron_chestplate")));
        assert!(Enchantment::THORNS.applies_to(get("core:iron_chestplate")));
        assert!(Enchantment::FORTUNE.applies_to(get("core:iron_pickaxe")));
        assert!(!Enchantment::FORTUNE.applies_to(get("core:iron_sword")));
        assert!(Enchantment::MAGNET.applies_to(get("core:iron_sword")));
        assert!(Enchantment::MAGNET.applies_to(get("core:iron_pickaxe")));
        assert!(!Enchantment::MAGNET.applies_to(get("core:iron_helmet")));
        assert!(Enchantment::LOOTING.applies_to(get("core:iron_sword")));
    }

    #[test]
    fn legacy_kinds_never_touch_the_extended_bit() {
        // Les anciens objets : une sorte (bits 24 à 30) et un niveau, jamais le bit 31.
        for kind in Enchantment::all() {
            for level in 1..=kind.hard_max() {
                let damage = with_enchantment(65_535, kind, level);
                assert!(!item_data::is_extended(damage), "{kind:?} {level}");
                assert_eq!(wear(damage), 65_535);
                assert_eq!(enchantments_of(damage), vec![(kind, level)]);
            }
        }
        // Le plus grand numéro de sac à dos reste lui aussi un objet simple.
        assert!(!item_data::is_extended(
            crate::storage::backpacks::MAX_NUMBER
        ));
        assert_eq!(wear(0), 0);
    }

    #[test]
    fn a_second_enchantment_migrates_the_stack_to_a_sheet() {
        // Un vieil objet : usure 17, Tranchant II dans les bits hauts.
        let old = with_enchantment(17, Enchantment::SHARPNESS, 2);
        assert!(!item_data::is_extended(old));
        let migrated = with_enchantment(old, Enchantment::UNBREAKING, 1);
        assert!(item_data::is_extended(migrated));
        assert_ne!(migrated, old);
        // L'usure et le premier enchantement sont recopiés, le second s'ajoute.
        assert_eq!(wear(migrated), 17);
        assert_eq!(
            enchantments_of(migrated),
            vec![(Enchantment::SHARPNESS, 2), (Enchantment::UNBREAKING, 1)]
        );
        assert_eq!(level_of(migrated, Enchantment::SHARPNESS), 2);
        assert_eq!(level_of(migrated, Enchantment::UNBREAKING), 1);
        assert_eq!(level_of(migrated, Enchantment::EFFICIENCY), 0);
        // Un troisième s'ajoute dans la même fiche ; le niveau d'une sorte se remplace.
        let third = with_enchantment(migrated, Enchantment::EFFICIENCY, 3);
        assert_eq!(third, migrated, "même numéro de fiche");
        let raised = with_enchantment(third, Enchantment::SHARPNESS, 3);
        assert_eq!(level_of(raised, Enchantment::SHARPNESS), 3);
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
    fn unknown_kinds_are_ignored_and_new_kinds_survive_the_sheet() {
        // Un numéro que les données ne connaissent pas (un mod retiré) n'est ni lu ni perdu par un
        // autre.
        assert!(Enchantment::from_id(99).is_none());
        assert!(Enchantment::from_id(300).is_none());
        let mut damage = 0;
        assert!(edit_sheet(&mut damage, |s| {
            s.enchantments = vec![(99, 3), (Enchantment::FORTUNE.id(), 2)];
        }));
        assert_eq!(enchantments_of(damage), vec![(Enchantment::FORTUNE, 2)]);
        assert_eq!(level_of(damage, Enchantment::FORTUNE), 2);
        // Un nouvel enchantement tient dans une pile simple, au niveau de la table (jusqu'à 8).
        for kind in Enchantment::all() {
            let simple = with_enchantment(21, kind, kind.hard_max());
            assert!(!item_data::is_extended(simple));
            assert_eq!(level_of(simple, kind), kind.hard_max());
            assert_eq!(wear(simple), 21);
        }
        assert_eq!(Enchantment::from_key("fortune"), Some(Enchantment::FORTUNE));
        assert_eq!(
            Enchantment::from_key("core:thorns"),
            Some(Enchantment::THORNS)
        );
        assert_eq!(Enchantment::from_key("rien"), None);
    }

    #[test]
    fn stripping_leaves_the_wear_and_the_other_data() {
        let two = with_enchantment(
            with_enchantment(5, Enchantment::LOOTING, 2),
            Enchantment::MAGNET,
            1,
        );
        let (stripped, taken) = strip_enchantments(two);
        assert_eq!(taken.len(), 2);
        assert!(!is_enchanted(stripped));
        assert_eq!(wear(stripped), 5);
        // Rien à retirer : la pile ne change pas.
        assert_eq!(strip_enchantments(7), (7, Vec::new()));
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
    fn unbreaking_spares_wear_and_protection_adds_up() {
        use crate::player::inventory::{Inventory, ItemStack};
        let items = items();
        let pickaxe = items.id("core:iron_pickaxe").unwrap();
        let helmet = items.id("core:iron_helmet").unwrap();
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(ItemStack {
            item: pickaxe,
            count: 1,
            damage: with_enchantment(0, Enchantment::UNBREAKING, 3),
        });
        // Solidité III : un tirage bas épargne l'objet, un tirage haut non.
        assert!(!inventory.wear_selected(&items, 0.1));
        assert_eq!(wear(inventory.slots[0].unwrap().damage), 0);
        assert!(!inventory.wear_selected(&items, 0.95));
        assert_eq!(wear(inventory.slots[0].unwrap().damage), 1);
        // L'enchantement survit à l'usure.
        assert_eq!(
            enchantments_of(inventory.slots[0].unwrap().damage),
            vec![(Enchantment::UNBREAKING, 3)]
        );
        // Protection : +niveau par pièce.
        let plain = ItemStack::new(helmet, 1);
        inventory.armor[0] = Some(plain);
        let base = inventory.protection(&items);
        inventory.armor[0] = Some(ItemStack {
            damage: with_enchantment(0, Enchantment::PROTECTION, 2),
            ..plain
        });
        assert_eq!(inventory.protection(&items), base + 2);
    }

    #[test]
    fn wear_goes_through_the_sheet_and_a_broken_tool_frees_it() {
        use crate::player::inventory::{Inventory, ItemStack};
        let items = items();
        let pickaxe = items.id("core:iron_pickaxe").unwrap();
        let durability = items.get(pickaxe).durability.unwrap();
        // Deux enchantements : la pile est étendue, l'usure vit dans la fiche.
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::EFFICIENCY, 1),
            Enchantment::UNBREAKING,
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
        use crate::player::inventory::{Inventory, ItemStack};
        let items = items();
        let sword = items.id("core:iron_sword").unwrap();
        let mut inventory = Inventory::default();
        let enchanted = ItemStack {
            item: sword,
            count: 1,
            damage: with_enchantment(5, Enchantment::SHARPNESS, 2),
        };
        inventory.slots[3] = Some(enchanted);
        let saved = inventory.to_save(&items);
        let back = Inventory::from_save(&saved, &items);
        assert_eq!(back.slots[3], Some(enchanted));
    }

    #[test]
    fn old_worlds_and_old_enchanted_items_stay_valid() {
        use crate::player::inventory::Inventory;
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
            vec![(Enchantment::SHARPNESS, 2)]
        );
        assert_eq!(inventory.slots[1].unwrap().damage, 0);
        // Il se sauvegarde à l'identique tant qu'il n'a pas reçu de deuxième enchantement...
        assert_eq!(
            inventory.to_save(&items).slots[0].unwrap().damage,
            33_685_509
        );
        // ... puis migre, et la pile étendue fait l'aller-retour par `world.ron` (RON).
        let mut inventory = inventory;
        let migrated = with_enchantment(sword.damage, Enchantment::UNBREAKING, 2);
        inventory.slots[0].as_mut().unwrap().damage = migrated;
        let text = ron::to_string(&inventory.to_save(&items)).unwrap();
        let back = Inventory::from_save(&ron::from_str(&text).unwrap(), &items);
        assert_eq!(back.slots[0].unwrap().damage, migrated);
        assert_eq!(wear(migrated), 5);
        assert_eq!(
            enchantments_of(migrated),
            vec![(Enchantment::SHARPNESS, 2), (Enchantment::UNBREAKING, 2)]
        );
    }

    #[test]
    fn extended_stacks_survive_chests_wagons_and_never_stack() {
        use crate::player::inventory::{Click, ItemStack, click_slot};
        let items = items();
        let sword = items.id("core:iron_sword").unwrap();
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::SHARPNESS, 1),
            Enchantment::UNBREAKING,
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
            with_enchantment(0, Enchantment::SHARPNESS, 3),
            Enchantment::UNBREAKING,
            3,
        );
        let b = ItemStack { damage: other, ..a };
        let (mut slot, mut cursor) = (Some(a), Some(b));
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Primary);
        assert_eq!((slot, cursor), (Some(b), Some(a)));
        // La pile jetée au sol puis ramassée garde son numéro (c'est la même valeur).
        let mut inventory = crate::player::inventory::Inventory::default();
        assert_eq!(inventory.add_stack(a, &items), 0);
        assert_eq!(inventory.slots[0].unwrap().damage, damage);
    }

    #[test]
    fn backpacks_and_items_stores_live_side_by_side() {
        use crate::storage::backpacks::BackpackStore;
        let dir = std::env::temp_dir().join(format!("voxel-stores-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let items = items();
        // Un sac numéroté, et une fiche d'objet : deux fichiers distincts, relus sans se gêner.
        let mut bags = BackpackStore::default();
        let mut bag =
            crate::player::inventory::ItemStack::new(items.id("core:backpack_leather").unwrap(), 1);
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
        use crate::player::survival::Experience;
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
