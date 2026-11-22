//! La bibliothèque d'enchantements : un bloc qui range les enchantements des livres en **points**.
//!
//! # Les points
//!
//! Un enchantement de niveau `n` vaut `2^(n-1)` points : 1, 2, 4, 8... (deux livres de niveau `n`
//! font un niveau `n + 1`). La bibliothèque garde, pour chaque sorte, ses points et le **plus haut
//! niveau déjà déposé**. Son plafond est donné par le bloc (`library_level` de `blocks.ron`) :
//! elle ne range pas un niveau au-dessus, et pas plus de `2^plafond` points par sorte. Un livre qui
//! ne rentre pas est refusé en entier (rien n'est perdu).
//!
//! On en sort un livre enchanté **au niveau choisi**, de 1 jusqu'au plus haut niveau déjà déposé,
//! tant qu'il reste assez de points : le niveau `n` coûte `2^(n-1)` points, et un livre vierge.
//!
//! # Où c'est rangé
//!
//! Dans les octets libres de l'entité du bloc (`BlockEntity::data`, en postcard : sorte, points,
//! plus haut niveau), donc sauvegardé avec le chunk, sans changer son format. Une bibliothèque cassée
//! rend ce qu'elle contient sous forme de livres enchantés.
//!
//! # Les tuyaux d'objets
//!
//! Le bloc a deux cases : la case 0 **reçoit** (les réseaux d'objets y déposent des livres), la case 1
//! **rend** (ce qui est refusé : un objet qui n'est pas un livre enchanté, ou un livre qui ne rentre
//! pas). Une fois par seconde, le comportement natif ([`Library`]) range le livre de la case 0.

use std::collections::BTreeMap;
use std::rc::Rc;

use engine_voxel::{
    BlockBehavior, BlockRegistry, EnchantingStats, GridPos, Inventory, MachineContext, Machines,
};
use serde::{Deserialize, Serialize};

use crate::enchanting::books::ENCHANTED_BOOK;
use crate::enchanting::{self, Enchantment};

/// Le nom du comportement dans `blocks.ron`.
pub const BEHAVIOR: &str = "core:enchant_library";
/// Les cases du bloc : l'entrée et la sortie.
pub const INPUT: usize = 0;
pub const OUTPUT: usize = 1;
/// Le délai entre deux rangements, en ticks (20 par seconde).
const PERIOD: u32 = 20;

/// Les points d'un niveau : `2^(niveau - 1)` (0 pour le niveau 0).
pub fn points_of_level(level: u8) -> u32 {
    if level == 0 {
        0
    } else {
        1u32 << (level - 1).min(30)
    }
}

/// Ce que la bibliothèque ne range pas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Le livre n'a aucun enchantement.
    Empty,
    /// Un niveau au-dessus de ce que la bibliothèque sait ranger.
    TooHigh(Enchantment, u8),
    /// La sorte est pleine.
    Full(Enchantment),
}

impl Refusal {
    pub fn text(self) -> String {
        match self {
            Self::Empty => "Ce livre n'a aucun enchantement".into(),
            Self::TooHigh(kind, level) => format!(
                "{} {} est trop haut pour cette bibliothèque",
                kind.name(),
                enchanting::roman(level)
            ),
            Self::Full(kind) => format!("{} est plein", kind.name()),
        }
    }
}

/// Le contenu d'une bibliothèque : par sorte, (points, plus haut niveau déposé).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    entries: BTreeMap<u8, (u32, u8)>,
}

impl Library {
    /// Relit une bibliothèque depuis les octets libres d'une entité (vide si illisible).
    pub fn read(data: &[u8]) -> Self {
        postcard::from_bytes(data).unwrap_or_default()
    }

    pub fn write(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }

    /// Le plus de points qu'une sorte garde, pour une bibliothèque de ce niveau maximal.
    pub fn capacity(max_level: u8) -> u32 {
        1u32 << max_level.min(30)
    }

    pub fn points(&self, kind: Enchantment) -> u32 {
        self.entries.get(&kind.id()).map_or(0, |e| e.0)
    }

    /// Le plus haut niveau déjà déposé pour cette sorte (0 : aucun).
    pub fn best(&self, kind: Enchantment) -> u8 {
        self.entries.get(&kind.id()).map_or(0, |e| e.1)
    }

    /// Les sortes rangées, avec leurs points et leur plus haut niveau.
    pub fn entries(&self) -> Vec<(Enchantment, u32, u8)> {
        self.entries
            .iter()
            .filter(|(_, (points, _))| *points > 0)
            .map(|(&id, &(points, best))| (Enchantment(id), points, best))
            .collect()
    }

    /// Range les enchantements d'un livre. Tout ou rien : si un seul ne rentre pas, rien n'est rangé.
    pub fn deposit(&mut self, book: &[(Enchantment, u8)], max_level: u8) -> Result<(), Refusal> {
        if book.is_empty() {
            return Err(Refusal::Empty);
        }
        let capacity = Self::capacity(max_level);
        // Une sorte peut apparaître une fois par livre ; on vérifie avant d'écrire.
        for &(kind, level) in book {
            if level > max_level {
                return Err(Refusal::TooHigh(kind, level));
            }
            if self.points(kind) + points_of_level(level) > capacity {
                return Err(Refusal::Full(kind));
            }
        }
        for &(kind, level) in book {
            let entry = self.entries.entry(kind.id()).or_insert((0, 0));
            entry.0 += points_of_level(level);
            entry.1 = entry.1.max(level);
        }
        Ok(())
    }

    /// Le niveau le plus haut qu'on peut sortir maintenant : au plus le plus haut déposé, et il faut
    /// assez de points.
    pub fn extractable(&self, kind: Enchantment) -> u8 {
        let mut level = self.best(kind);
        while level > 0 && points_of_level(level) > self.points(kind) {
            level -= 1;
        }
        level
    }

    /// Sort un enchantement au niveau choisi. `false` (rien ne change) s'il est trop haut ou s'il
    /// manque des points.
    pub fn extract(&mut self, kind: Enchantment, level: u8) -> bool {
        if level == 0 || level > self.best(kind) || points_of_level(level) > self.points(kind) {
            return false;
        }
        if let Some(entry) = self.entries.get_mut(&kind.id()) {
            entry.0 -= points_of_level(level);
        }
        true
    }

    /// Tout ce que la bibliothèque contient, en livres d'un enchantement (quand on la casse) : les
    /// niveaux les plus hauts d'abord, jamais au-dessus du plafond.
    pub fn into_books(self, max_level: u8) -> Vec<(Enchantment, u8)> {
        let mut out = Vec::new();
        for (id, (mut points, _)) in self.entries {
            while points > 0 {
                let bits = (u32::BITS - points.leading_zeros()) as u8;
                let level = bits.min(max_level).max(1);
                out.push((Enchantment(id), level));
                points = points.saturating_sub(points_of_level(level));
            }
        }
        out
    }
}

/// Le plafond d'une bibliothèque d'après ses stats de bloc (0 : ce n'est pas une bibliothèque).
pub fn max_level_of(stats: Option<&EnchantingStats>) -> u8 {
    stats.map_or(0, |s| s.library_level)
}

/// Range ce qui attend dans les cases du bloc : le livre d'entrée entre dans la bibliothèque, tout
/// le reste va à la sortie. `true` si quelque chose a bougé.
pub fn absorb(entity: &mut engine_voxel::BlockEntity, max_level: u8) -> bool {
    let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
        return false;
    };
    if inventory.slots.len() < 2 {
        inventory.slots.resize(2, None);
    }
    let Some(stack) = inventory.slots[INPUT].clone() else {
        return false;
    };
    let mut library = Library::read(&entity.data);
    let mut moved = false;
    if stack.item == ENCHANTED_BOOK && stack.count == 1 {
        let book = enchanting::enchantments_of(stack.damage);
        if library.deposit(&book, max_level).is_ok() {
            enchanting::release(stack.damage);
            inventory.slots[INPUT] = None;
            entity.data = library.write();
            return true;
        }
    }
    // Refusé, ou pas un livre enchanté : la sortie le rend, si elle est libre.
    if inventory.slots[OUTPUT].is_none() {
        inventory.slots[OUTPUT] = inventory.slots[INPUT].take();
        moved = true;
    }
    moved
}

/// Le comportement du bloc : range un livre par seconde.
pub struct LibraryBehavior;

fn max_level_at(ctx: &MachineContext, at: GridPos) -> u8 {
    ctx.registry().map_or(0, |r| {
        max_level_of(r.def(ctx.block(at)).enchanting.as_ref())
    })
}

impl BlockBehavior for LibraryBehavior {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.grids.write_or_create(at, |e| {
                if e.caps.as_ref().is_none_or(|c| c.inventory.is_none()) {
                    e.caps_mut().inventory = Some(Inventory::new(2));
                }
            });
            ctx.schedule(at, PERIOD);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let max = max_level_at(ctx, at);
            ctx.grids.write(at, |e| absorb(e, max));
            ctx.schedule(at, PERIOD);
        }
    }
}

/// Branche le comportement sur les blocs qui le demandent (`behavior: "core:enchant_library"`).
pub fn register(machines: &mut Machines, blocks: &BlockRegistry) {
    let behavior: Rc<dyn BlockBehavior> = Rc::new(LibraryBehavior);
    for (id, def) in blocks.iter() {
        if def.behavior.as_deref() == Some(BEHAVIOR) {
            machines.register(id, behavior.clone());
        }
    }
}

/// Les livres enchantés que rend une bibliothèque cassée (rien pour un autre bloc).
pub fn broken_books(
    registry: &BlockRegistry,
    block: engine_voxel::BlockId,
    entity: Option<&engine_voxel::BlockEntity>,
    items: &engine_voxel::ItemRegistry,
) -> Vec<crate::player::inventory::ItemStack> {
    let max = max_level_of(registry.def(block).enchanting.as_ref());
    let (Some(entity), Some(book)) = (entity, items.id(ENCHANTED_BOOK)) else {
        return Vec::new();
    };
    if max == 0 {
        return Vec::new();
    }
    Library::read(&entity.data)
        .into_books(max)
        .into_iter()
        .map(|(kind, level)| crate::player::inventory::ItemStack {
            item: book,
            count: 1,
            damage: crate::enchanting::books::book_damage(&[(kind, level)]),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::{BlockEntity, ItemStack};

    /// Un livre enchanté qui porte ces enchantements.
    fn book_stack(enchants: &[(Enchantment, u8)]) -> ItemStack {
        ItemStack {
            item: ENCHANTED_BOOK.to_string(),
            count: 1,
            damage: crate::enchanting::books::book_damage(enchants),
        }
    }

    fn sharp(level: u8) -> (Enchantment, u8) {
        (Enchantment::SHARPNESS, level)
    }

    #[test]
    fn a_level_is_worth_two_to_the_level_minus_one_points() {
        let points: Vec<u32> = (0..=6).map(points_of_level).collect();
        assert_eq!(points, vec![0, 1, 2, 4, 8, 16, 32]);
        // Deux livres de niveau n font un niveau n + 1.
        for n in 1..10 {
            assert_eq!(2 * points_of_level(n), points_of_level(n + 1));
        }
    }

    #[test]
    fn deposit_adds_points_and_keeps_the_best_level() {
        let mut library = Library::default();
        library
            .deposit(&[sharp(3), (Enchantment::FORTUNE, 1)], 10)
            .unwrap();
        assert_eq!(library.points(Enchantment::SHARPNESS), 4);
        assert_eq!(library.points(Enchantment::FORTUNE), 1);
        library.deposit(&[sharp(2)], 10).unwrap();
        assert_eq!(library.points(Enchantment::SHARPNESS), 6);
        assert_eq!(
            library.best(Enchantment::SHARPNESS),
            3,
            "le plus haut déposé"
        );
        assert_eq!(library.best(Enchantment::LOOTING), 0);
        assert_eq!(library.entries().len(), 2);
    }

    #[test]
    fn the_library_refuses_what_exceeds_its_ceiling_and_keeps_everything() {
        let mut library = Library::default();
        // Un niveau au-dessus du plafond : refusé.
        assert_eq!(
            library.deposit(&[sharp(5)], 4),
            Err(Refusal::TooHigh(Enchantment::SHARPNESS, 5))
        );
        // Un livre vide aussi.
        assert_eq!(library.deposit(&[], 4), Err(Refusal::Empty));
        // Plafond de 4 : au plus 16 points par sorte ; 8 + 8 = 16 entrent, un de plus non.
        library.deposit(&[sharp(4)], 4).unwrap();
        library.deposit(&[sharp(4)], 4).unwrap();
        assert_eq!(library.points(Enchantment::SHARPNESS), 16);
        assert_eq!(
            library.deposit(&[sharp(1)], 4),
            Err(Refusal::Full(Enchantment::SHARPNESS))
        );
        // Tout ou rien : un livre à deux sortes dont une est pleine ne dépose pas l'autre.
        let before = library.clone();
        assert!(
            library
                .deposit(&[(Enchantment::FORTUNE, 2), sharp(1)], 4)
                .is_err()
        );
        assert_eq!(library, before);
    }

    #[test]
    fn extraction_needs_points_and_stops_at_the_best_level_deposited() {
        let mut library = Library::default();
        // Quatre livres de niveau 2 : 8 points, mais jamais rien de plus haut que 2.
        for _ in 0..4 {
            library.deposit(&[sharp(2)], 10).unwrap();
        }
        assert_eq!(library.points(Enchantment::SHARPNESS), 8);
        assert_eq!(library.extractable(Enchantment::SHARPNESS), 2);
        assert!(!library.extract(Enchantment::SHARPNESS, 3), "jamais déposé");
        assert!(library.extract(Enchantment::SHARPNESS, 2));
        assert!(library.extract(Enchantment::SHARPNESS, 1));
        assert_eq!(library.points(Enchantment::SHARPNESS), 5);
        // Un livre de niveau 4 déposé : 8 points d'un coup.
        library.deposit(&[sharp(4)], 10).unwrap();
        assert_eq!(library.points(Enchantment::SHARPNESS), 13);
        assert_eq!(library.extractable(Enchantment::SHARPNESS), 4);
        assert!(library.extract(Enchantment::SHARPNESS, 4));
        assert_eq!(library.points(Enchantment::SHARPNESS), 5);
        // Il manque des points : 5 ne suffisent pas pour un niveau 4 (8), mais pour le 3 (4).
        assert_eq!(library.extractable(Enchantment::SHARPNESS), 3);
        assert!(!library.extract(Enchantment::SHARPNESS, 4));
        assert!(library.extract(Enchantment::SHARPNESS, 3));
        assert_eq!(library.points(Enchantment::SHARPNESS), 1);
        assert!(!library.extract(Enchantment::SHARPNESS, 0));
    }

    #[test]
    fn nothing_is_created_from_nothing() {
        // Déposer n livres et tout sortir ne rend jamais plus de points que déposés.
        let mut library = Library::default();
        let mut deposited = 0;
        for level in [1, 3, 2, 5, 4] {
            library.deposit(&[sharp(level)], 10).unwrap();
            deposited += points_of_level(level);
        }
        let mut taken = 0;
        loop {
            let level = library.extractable(Enchantment::SHARPNESS);
            if level == 0 {
                break;
            }
            assert!(library.extract(Enchantment::SHARPNESS, level));
            taken += points_of_level(level);
        }
        assert_eq!(taken + library.points(Enchantment::SHARPNESS), deposited);
    }

    #[test]
    fn a_broken_library_gives_its_points_back_as_books() {
        let mut library = Library::default();
        library.deposit(&[sharp(5)], 10).unwrap();
        library.deposit(&[sharp(2)], 10).unwrap();
        library.deposit(&[(Enchantment::LOOTING, 1)], 10).unwrap();
        let books = library.clone().into_books(10);
        // 16 + 2 = 18 points de Tranchant : un livre V (16) et un livre II (2) ; Butin I.
        assert!(books.contains(&sharp(5)));
        assert!(books.contains(&sharp(2)));
        assert!(books.contains(&(Enchantment::LOOTING, 1)));
        let total: u32 = books
            .iter()
            .filter(|(k, _)| *k == Enchantment::SHARPNESS)
            .map(|&(_, l)| points_of_level(l))
            .sum();
        assert_eq!(total, 18);
        // Jamais au-dessus du plafond : 18 points avec un plafond de 3 : 4 + 4 + 4 + 4 + 2.
        let low = library.into_books(3);
        assert!(low.iter().all(|&(_, level)| level <= 3));
        let total: u32 = low
            .iter()
            .filter(|(k, _)| *k == Enchantment::SHARPNESS)
            .map(|&(_, l)| points_of_level(l))
            .sum();
        assert_eq!(total, 18);
    }

    #[test]
    fn the_library_survives_saving_as_block_bytes() {
        let mut library = Library::default();
        library
            .deposit(&[sharp(3), (Enchantment::MAGNET, 1)], 10)
            .unwrap();
        let bytes = library.write();
        assert_eq!(Library::read(&bytes), library);
        // Des octets vides ou illisibles : une bibliothèque vide.
        assert_eq!(Library::read(&[]), Library::default());
        assert_eq!(Library::read(&[0xFF, 0xFF, 0xFF, 0xFF]).entries().len(), 0);
    }

    fn entity_with(stack: Option<ItemStack>) -> BlockEntity {
        let mut entity = BlockEntity::default();
        let mut inventory = Inventory::new(2);
        inventory.slots[INPUT] = stack;
        entity.caps_mut().inventory = Some(inventory);
        entity
    }

    #[test]
    fn a_book_in_the_input_slot_is_absorbed_and_junk_goes_to_the_output() {
        // Un livre enchanté entre : la case se vide, les points montent.
        let mut entity = entity_with(Some(book_stack(&[sharp(3)])));
        assert!(absorb(&mut entity, 10));
        let inventory = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert!(inventory.slots[INPUT].is_none() && inventory.slots[OUTPUT].is_none());
        assert_eq!(
            Library::read(&entity.data).points(Enchantment::SHARPNESS),
            4
        );
        // Une pierre : refusée, rendue par la sortie.
        let mut entity = entity_with(Some(ItemStack {
            item: "core:stone".into(),
            count: 5,
            damage: 0,
        }));
        assert!(absorb(&mut entity, 10));
        let inventory = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert!(inventory.slots[INPUT].is_none());
        assert_eq!(inventory.slots[OUTPUT].as_ref().unwrap().item, "core:stone");
        // Un livre trop haut : rendu aussi, et la bibliothèque n'a rien reçu.
        let mut entity = entity_with(Some(book_stack(&[sharp(7)])));
        assert!(absorb(&mut entity, 4));
        let inventory = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert_eq!(
            inventory.slots[OUTPUT].as_ref().unwrap().item,
            ENCHANTED_BOOK
        );
        assert_eq!(Library::read(&entity.data), Library::default());
        // Sortie déjà occupée : le refus attend dans l'entrée (rien n'est écrasé).
        let mut entity = entity_with(Some(ItemStack {
            item: "core:stone".into(),
            count: 1,
            damage: 0,
        }));
        entity
            .caps
            .as_mut()
            .unwrap()
            .inventory
            .as_mut()
            .unwrap()
            .slots[OUTPUT] = Some(ItemStack {
            item: "core:dirt".into(),
            count: 1,
            damage: 0,
        });
        assert!(!absorb(&mut entity, 10));
        // Rien à ranger.
        assert!(!absorb(&mut entity_with(None), 10));
    }

    #[test]
    fn the_library_block_is_declared_in_the_core_blocks() {
        let registry = crate::test_registry();
        let id = registry.id("core:enchant_library").unwrap();
        let def = registry.def(id);
        assert_eq!(def.behavior.as_deref(), Some(BEHAVIOR));
        assert_eq!(max_level_of(def.enchanting.as_ref()), 10);
        let ports = def.ports.as_ref().unwrap();
        assert_eq!(
            (ports.inputs.clone(), ports.outputs.clone()),
            (vec![0], vec![1])
        );
        // Le comportement se branche sur ce bloc.
        let mut machines = Machines::with_natives(&registry);
        register(&mut machines, &registry);
        assert!(machines.has_behavior(id));
    }
}
