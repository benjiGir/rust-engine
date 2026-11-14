//! Les livres, les tomes et l'extraction des enchantements.
//!
//! # Les objets
//!
//! - **Livre** (`core:book`) : vierge ; posé dans la main devant une table d'enchantement, il reçoit
//!   des enchantements de tout genre et devient un **livre enchanté** (`core:enchanted_book`), qui les
//!   porte dans sa fiche (voir `item_data.rs`), comme n'importe quel objet enchanté ;
//! - **Tomes** (`core:tome_weapons`, `core:tome_tools`, `core:tome_armor`) : des livres qui ne
//!   reçoivent que les enchantements de leur type (armes, outils, armures), ce qui laisse choisir ce
//!   qu'on enchante. Une table en fait aussi des livres enchantés ;
//! - **Tome d'extraction** (`core:tome_extraction`) : retire tous les enchantements d'un objet et les
//!   range dans un livre enchanté (il consomme un livre vierge de l'inventaire). L'objet reste, avec
//!   son usure, ses affixes et ses gemmes.
//!
//! # Les gestes
//!
//! Le livre ou le tome d'extraction se tient **au curseur** dans l'écran d'une table d'enchantement
//! ou d'une enclume simple, puis on clique sur un objet de l'inventaire :
//!
//! - livre enchanté sur un objet : chaque enchantement qui convient à l'objet est appliqué. Une sorte
//!   absente s'ajoute (dans la limite du nombre d'enchantements par objet) ; une sorte déjà là prend
//!   le niveau le plus haut des deux, et **monte d'un niveau** si les deux sont égaux, sans dépasser
//!   le niveau maximal de la sorte. Rien n'est appliqué (et le livre est gardé) si rien ne change ;
//! - tome d'extraction sur un objet enchanté : voir ci-dessus.

use engine_voxel::ItemDef;

use crate::adventure_data::AdventureData;
use crate::enchant_data::{EnchantDef, Target};
use crate::enchanting::{self, Enchantment};

pub const BOOK: &str = "core:book";
pub const ENCHANTED_BOOK: &str = "core:enchanted_book";
pub const TOME_EXTRACTION: &str = "core:tome_extraction";

/// Les sortes de livres vierges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookKind {
    /// Le livre : tous les enchantements.
    Plain,
    Weapons,
    Tools,
    Armor,
}

impl BookKind {
    /// La sorte d'un objet (par sa clé), s'il en est une.
    pub fn of_key(key: &str) -> Option<Self> {
        Some(match key {
            BOOK => Self::Plain,
            "core:tome_weapons" => Self::Weapons,
            "core:tome_tools" => Self::Tools,
            "core:tome_armor" => Self::Armor,
            _ => return None,
        })
    }

    /// Cette sorte accepte-t-elle cet enchantement ? Un enchantement « durable » (Solidité) va dans
    /// tous les tomes.
    pub fn accepts(self, enchant: &EnchantDef) -> bool {
        let has = |wanted: Target| {
            enchant
                .targets
                .iter()
                .any(|t| *t == wanted || *t == Target::Durable)
        };
        match self {
            Self::Plain => true,
            Self::Weapons => has(Target::Weapon),
            Self::Tools => has(Target::Digger),
            Self::Armor => has(Target::Armor),
        }
    }
}

/// Le niveau d'une sorte après l'apport d'un livre : le plus haut des deux, et un de plus s'ils
/// sont égaux, sans dépasser `hard_max`. `None` si rien ne change.
pub fn merged_level(existing: u8, offered: u8, hard_max: u8) -> Option<u8> {
    let target = match existing.cmp(&offered) {
        std::cmp::Ordering::Equal => existing + 1,
        _ => existing.max(offered),
    }
    .min(hard_max);
    (target > existing).then_some(target)
}

/// Applique les enchantements d'un livre à un objet. Renvoie le nouveau `damage` et ce qui a
/// changé, ou `None` si rien ne convient (ou rien ne monte).
pub fn apply_book(
    data: &AdventureData,
    item: &ItemDef,
    damage: u32,
    book: &[(Enchantment, u8)],
) -> Option<(u32, Vec<(Enchantment, u8)>)> {
    let mut damage = damage;
    let mut changed = Vec::new();
    for &(kind, offered) in book {
        let Some(def) = data.enchant_by_number(kind.id()) else {
            continue;
        };
        if !def.applies_to(item) {
            continue;
        }
        let existing = enchanting::level_of(damage, kind);
        if existing == 0 && enchanting::enchantments_of(damage).len() >= data.max_enchantments {
            continue;
        }
        let Some(level) = merged_level(existing, offered.min(def.hard_max), def.hard_max) else {
            continue;
        };
        damage = enchanting::with_enchantment(damage, kind, level);
        changed.push((kind, level));
    }
    (!changed.is_empty()).then_some((damage, changed))
}

/// Le `damage` d'un livre enchanté qui porte ces enchantements.
pub fn book_damage(enchants: &[(Enchantment, u8)]) -> u32 {
    enchants.iter().fold(0, |damage, &(kind, level)| {
        enchanting::with_enchantment(damage, kind, level)
    })
}

/// L'extraction : l'objet sans ses enchantements, et le livre qui les porte. `None` si l'objet n'en
/// a pas.
pub fn extract(damage: u32) -> Option<(u32, u32)> {
    let (stripped, taken) = enchanting::strip_enchantments(damage);
    (!taken.is_empty()).then(|| (stripped, book_damage(&taken)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn def<'a>(items: &'a ItemRegistry, key: &str) -> &'a ItemDef {
        items.get(items.id(key).unwrap())
    }

    #[test]
    fn the_book_items_exist_and_never_stack_once_enchanted() {
        let items = items();
        for key in [
            BOOK,
            ENCHANTED_BOOK,
            TOME_EXTRACTION,
            "core:tome_weapons",
            "core:tome_tools",
            "core:tome_armor",
        ] {
            assert!(items.id(key).is_some(), "{key}");
        }
        assert_eq!(def(&items, ENCHANTED_BOOK).max_stack, 1);
        let book = book_damage(&[(Enchantment::SHARPNESS, 2)]);
        assert_ne!(
            book, 0,
            "un livre enchanté a un `damage` : il ne s'empile pas"
        );
    }

    #[test]
    fn a_tome_only_takes_the_enchantments_of_its_type() {
        let data = AdventureData::builtin();
        let of = |kind: BookKind| -> Vec<Enchantment> {
            data.enchantments
                .iter()
                .filter(|e| kind.accepts(e))
                .map(|e| Enchantment(e.number))
                .collect()
        };
        let weapons = of(BookKind::Weapons);
        assert!(weapons.contains(&Enchantment::SHARPNESS));
        assert!(weapons.contains(&Enchantment::LOOTING));
        assert!(weapons.contains(&Enchantment::MAGNET));
        assert!(!weapons.contains(&Enchantment::EFFICIENCY));
        assert!(!weapons.contains(&Enchantment::PROTECTION));
        let tools = of(BookKind::Tools);
        assert!(tools.contains(&Enchantment::FORTUNE));
        assert!(tools.contains(&Enchantment::SILK_TOUCH));
        assert!(!tools.contains(&Enchantment::SHARPNESS));
        let armor = of(BookKind::Armor);
        assert!(armor.contains(&Enchantment::THORNS));
        assert!(!armor.contains(&Enchantment::FORTUNE));
        // Solidité va dans tous les tomes, et le livre vierge prend tout.
        for list in [&weapons, &tools, &armor] {
            assert!(list.contains(&Enchantment::UNBREAKING));
        }
        assert_eq!(of(BookKind::Plain).len(), data.enchantments.len());
        assert_eq!(BookKind::of_key("core:tome_armor"), Some(BookKind::Armor));
        assert_eq!(BookKind::of_key("core:stone"), None);
    }

    #[test]
    fn levels_merge_like_an_anvil() {
        assert_eq!(merged_level(0, 3, 8), Some(3), "une sorte absente");
        assert_eq!(merged_level(2, 3, 8), Some(3), "le plus haut");
        assert_eq!(merged_level(3, 2, 8), None, "pas de baisse");
        assert_eq!(merged_level(3, 3, 8), Some(4), "deux égaux : un de plus");
        assert_eq!(merged_level(8, 8, 8), None, "jamais au-delà du maximum");
        assert_eq!(merged_level(0, 9, 8), Some(8), "plafonné");
        assert_eq!(merged_level(1, 1, 1), None);
    }

    #[test]
    fn a_book_applies_only_what_fits_and_respects_the_limit() {
        let items = items();
        let data = AdventureData::builtin();
        let sword = def(&items, "core:iron_sword");
        let book = [
            (Enchantment::SHARPNESS, 4),
            (Enchantment::EFFICIENCY, 5),
            (Enchantment::LOOTING, 2),
        ];
        let (damage, changed) = apply_book(&data, sword, 7, &book).unwrap();
        // Efficacité ne va pas sur une épée.
        assert_eq!(changed.len(), 2);
        assert_eq!(enchanting::level_of(damage, Enchantment::SHARPNESS), 4);
        assert_eq!(enchanting::level_of(damage, Enchantment::LOOTING), 2);
        assert_eq!(enchanting::level_of(damage, Enchantment::EFFICIENCY), 0);
        assert_eq!(enchanting::wear(damage), 7, "l'usure est gardée");
        // Le même livre une deuxième fois : Tranchant IV égal monte à V, Butin II à III.
        let (again, changed) = apply_book(&data, sword, damage, &book).unwrap();
        assert_eq!(enchanting::level_of(again, Enchantment::SHARPNESS), 5);
        assert_eq!(enchanting::level_of(again, Enchantment::LOOTING), 3);
        assert_eq!(changed.len(), 2);
        // Un livre trop faible, ou qui ne convient pas : rien.
        assert!(apply_book(&data, sword, again, &[(Enchantment::SHARPNESS, 2)]).is_none());
        assert!(apply_book(&data, sword, 0, &[(Enchantment::PROTECTION, 3)]).is_none());
        // Une pierre ne reçoit rien.
        assert!(apply_book(&data, def(&items, "core:stone"), 0, &book).is_none());
        // À la limite du nombre d'enchantements, une nouvelle sorte est refusée mais une
        // sorte déjà là peut encore monter.
        let mut full = 0;
        for (kind, level) in [
            (Enchantment::SHARPNESS, 1),
            (Enchantment::UNBREAKING, 1),
            (Enchantment::LOOTING, 1),
            (Enchantment::LIFE_STEAL, 1),
            (Enchantment::MAGNET, 1),
            (Enchantment::EFFICIENCY, 1),
        ] {
            full = enchanting::with_enchantment(full, kind, level);
        }
        assert_eq!(
            enchanting::enchantments_of(full).len(),
            data.max_enchantments
        );
        assert!(apply_book(&data, sword, full, &[(Enchantment::THORNS, 3)]).is_none());
        let (raised, _) = apply_book(&data, sword, full, &[(Enchantment::SHARPNESS, 3)]).unwrap();
        assert_eq!(enchanting::level_of(raised, Enchantment::SHARPNESS), 3);
    }

    #[test]
    fn extraction_moves_the_enchantments_to_a_book_and_keeps_the_rest() {
        let mut damage = 0;
        assert!(enchanting::edit_sheet(&mut damage, |s| {
            s.rarity = 2;
            s.name = Some("Épée rare".into());
        }));
        damage = enchanting::with_enchantment(damage, Enchantment::SHARPNESS, 3);
        damage = enchanting::with_enchantment(damage, Enchantment::UNBREAKING, 2);
        enchanting::add_wear(&mut damage, 9);
        let (stripped, book) = extract(damage).unwrap();
        assert!(!enchanting::is_enchanted(stripped));
        assert_eq!(enchanting::wear(stripped), 9);
        assert_eq!(
            crate::item_data::sheet(stripped).unwrap().name.as_deref(),
            Some("Épée rare"),
            "le nom et la rareté restent"
        );
        assert_eq!(
            enchanting::enchantments_of(book),
            vec![(Enchantment::SHARPNESS, 3), (Enchantment::UNBREAKING, 2)]
        );
        // Une pile simple perd ses bits hauts, garde son usure.
        let simple = enchanting::with_enchantment(12, Enchantment::EFFICIENCY, 2);
        let (stripped, book) = extract(simple).unwrap();
        assert_eq!(stripped, 12);
        assert_eq!(
            enchanting::enchantments_of(book),
            vec![(Enchantment::EFFICIENCY, 2)]
        );
        // Rien à extraire d'un objet nu.
        assert!(extract(5).is_none());
    }
}
