//! L'inventaire : 9 cases de barre d'objets (touches 1 à 9), 27 de réserve, 4 d'armure,
//! et la grille d'artisanat en cours.
//!
//! On y range des **objets** ([`ItemId`]), pas des blocs : un outil, un lingot ou un bloc
//! de pierre. Ce qui s'empile s'empile jusqu'à sa pile maximale (64, ou 1 pour un outil) ;
//! un outil garde son usure (`damage`). Quand `damage` a son bit 31 à 1, il ne porte qu'un
//! numéro de fiche (usure, enchantements...) : voir `item_data.rs`.
//!
//! On le sauvegarde avec une **palette** : la liste des identifiants texte des objets
//! présents, et pour chaque case un numéro dans cette palette. Les numéros d'objets
//! changent quand on ajoute un mod ; les identifiants texte, jamais.

use crate::enchanting::{self, Enchantment};
use engine_voxel::{
    ArmorSlot, Ingredient, ItemId, ItemRegistry, Recipe, SavedInventory, SavedStack,
};

pub const HOTBAR: usize = 9;
pub const SLOTS: usize = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    pub item: ItemId,
    pub count: u32,
    /// L'usure (outils, armures) : 0 = neuf.
    pub damage: u32,
}

impl ItemStack {
    pub fn new(item: ItemId, count: u32) -> Self {
        Self {
            item,
            count,
            damage: 0,
        }
    }
}

/// Une case quelque part : l'inventaire, l'armure, une grille d'artisanat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Main(usize),
    Armor(usize),
    Craft(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// Les cases 0 à 8 forment la barre d'objets, 9 à 35 la réserve.
    pub slots: [Option<ItemStack>; SLOTS],
    /// Tête, torse, jambes, pieds.
    pub armor: [Option<ItemStack>; 4],
    /// La grille d'artisanat (2 × 2 dans l'inventaire, 3 × 3 sur l'établi), par rangées.
    pub craft: [Option<ItemStack>; 9],
    /// La case choisie dans la barre.
    pub selected: usize,
    /// La pile « tenue à la souris » quand un écran est ouvert.
    pub cursor: Option<ItemStack>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            armor: [None; 4],
            craft: [None; 9],
            selected: 0,
            cursor: None,
        }
    }
}

/// Ce que fait un clic sur une case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Clic gauche : prendre, poser, compléter ou échanger toute la pile.
    Primary,
    /// Clic droit : main vide, prendre la moitié de la pile (arrondie au-dessus) ;
    /// une pile en main, en poser un seul objet.
    Secondary,
    /// Prendre `n` objets de la case (Maj + clic droit, puis le nombre choisi).
    Take(u32),
}

/// Un clic sur une case avec la pile en main (`cursor`). `accepts` : la case
/// accepte-t-elle cet objet (une case d'armure ne prend qu'une armure de son
/// emplacement) ?
pub fn click_slot(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &ItemRegistry,
    accepts: impl Fn(ItemId) -> bool,
    click: Click,
) {
    click_slot_mult(slot, cursor, items, accepts, click, 1);
}

/// Comme [`click_slot`], pour une case qui accepte des piles `mult` fois plus grandes (un
/// conteneur à amélioration « pile »). La main, elle, garde la taille normale : une pile trop
/// grande se prend en plusieurs fois.
pub fn click_slot_mult(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &ItemRegistry,
    accepts: impl Fn(ItemId) -> bool,
    click: Click,
    mult: u32,
) {
    if let Some(held) = cursor
        && !accepts(held.item)
    {
        return;
    }
    match click {
        Click::Primary => click_whole(slot, cursor, items, mult),
        Click::Secondary => match cursor {
            None => {
                let half = slot.map_or(0, |s| s.count.div_ceil(2));
                take(slot, cursor, items, half);
            }
            Some(_) => place_one(slot, cursor, items, mult),
        },
        Click::Take(n) => take(slot, cursor, items, n),
    }
}

/// L'objet est-il épargné par l'usure ce coup-ci ? Solidité de niveau L : une chance
/// sur L + 1 d'être épargné par coup, par tirage `roll` entre 0 et 1.
fn spared(damage: u32, roll: f32) -> bool {
    let level = enchanting::level_of(damage, Enchantment::UNBREAKING);
    level > 0 && roll < f32::from(level) / f32::from(level + 1)
}

/// Deux piles qui peuvent n'en faire qu'une : même objet, neuves.
fn stackable(a: &ItemStack, b: &ItemStack) -> bool {
    a.item == b.item && a.damage == 0 && b.damage == 0
}

/// Prend jusqu'à `n` objets de la case, dans la main vide ou dans la pile tenue s'il
/// s'agit du même objet (sans dépasser sa taille maximale).
fn take(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &ItemRegistry,
    n: u32,
) {
    let Some(stack) = slot.as_mut() else { return };
    let moved = match cursor {
        None => n.min(stack.count),
        Some(held) if stackable(held, stack) => {
            let room = items.get(held.item).max_stack.saturating_sub(held.count);
            n.min(stack.count).min(room)
        }
        Some(_) => 0,
    };
    if moved == 0 {
        return;
    }
    match cursor {
        Some(held) => held.count += moved,
        None => {
            *cursor = Some(ItemStack {
                count: moved,
                ..*stack
            })
        }
    }
    stack.count -= moved;
    if stack.count == 0 {
        *slot = None;
    }
}

/// Pose un seul objet de la pile tenue : dans une case vide, ou sur une pile du même
/// objet qui a encore de la place.
fn place_one(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &ItemRegistry,
    mult: u32,
) {
    let Some(held) = cursor.as_mut() else { return };
    match slot {
        None => {
            *slot = Some(ItemStack { count: 1, ..*held });
        }
        Some(stack)
            if stackable(stack, held) && stack.count < items.get(stack.item).max_stack * mult =>
        {
            stack.count += 1;
        }
        Some(_) => return,
    }
    held.count -= 1;
    if held.count == 0 {
        *cursor = None;
    }
}

/// Le clic gauche : toute la pile. Dans une case à piles `mult` fois plus grandes, la main ne
/// prend jamais plus que la taille normale, et un échange qui ferait déborder est refusé.
fn click_whole(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &ItemRegistry,
    mult: u32,
) {
    match (slot.as_mut(), cursor.as_mut()) {
        // Même objet, empilable : on complète la pile de la case.
        (Some(stack), Some(held))
            if stack.item == held.item && held.damage == 0 && stack.damage == 0 =>
        {
            let max = items.get(stack.item).max_stack * mult;
            let moved = held.count.min(max.saturating_sub(stack.count));
            stack.count += moved;
            held.count -= moved;
            if held.count == 0 {
                *cursor = None;
            }
        }
        // Une pile plus grande que la main ne peut porter : on en prend ce qui tient.
        (Some(stack), None) if stack.count > items.get(stack.item).max_stack => {
            let n = items.get(stack.item).max_stack;
            take(slot, cursor, items, n);
        }
        // Échanger deux piles différentes : seulement si la grosse pile tient dans la main.
        (Some(stack), Some(_)) if stack.count > items.get(stack.item).max_stack => {}
        // Sinon (prendre, poser) : on échange.
        _ => std::mem::swap(slot, cursor),
    }
}

impl Inventory {
    /// Range `count` objets neufs ; renvoie ce qui n'a pas trouvé de place.
    pub fn add(&mut self, item: ItemId, count: u32, items: &ItemRegistry) -> u32 {
        self.add_stack(ItemStack::new(item, count), items)
    }

    /// Range une pile (avec son usure). D'abord les piles existantes du même objet, puis
    /// les cases vides : on complète avant d'entamer une nouvelle pile.
    pub fn add_stack(&mut self, stack: ItemStack, items: &ItemRegistry) -> u32 {
        let max = items.get(stack.item).max_stack.max(1);
        let mut count = stack.count;
        if stack.damage == 0 {
            for slot in self.slots.iter_mut().flatten() {
                if slot.item == stack.item && slot.damage == 0 && slot.count < max {
                    let moved = count.min(max - slot.count);
                    slot.count += moved;
                    count -= moved;
                    if count == 0 {
                        return 0;
                    }
                }
            }
        }
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            let moved = count.min(max);
            *slot = Some(ItemStack {
                item: stack.item,
                count: moved,
                damage: stack.damage,
            });
            count -= moved;
            if count == 0 {
                return 0;
            }
        }
        count
    }

    pub fn selected_stack(&self) -> Option<ItemStack> {
        self.slots[self.selected]
    }

    /// L'objet de la case choisie.
    pub fn selected_item(&self) -> Option<ItemId> {
        self.selected_stack().map(|s| s.item)
    }

    /// Retire un objet de la case choisie (on vient de le poser).
    /// Vide tout l'inventaire (cases, armure, grille d'artisanat, main) et rend les
    /// piles : à la mort, elles tombent par terre.
    pub fn take_all(&mut self) -> Vec<ItemStack> {
        self.slots
            .iter_mut()
            .chain(self.armor.iter_mut())
            .chain(self.craft.iter_mut())
            .chain(std::iter::once(&mut self.cursor))
            .filter_map(Option::take)
            .collect()
    }

    pub fn consume_selected(&mut self) -> Option<ItemId> {
        let slot = &mut self.slots[self.selected];
        let stack = slot.as_mut()?;
        let item = stack.item;
        stack.count -= 1;
        if stack.count == 0 {
            *slot = None;
        }
        Some(item)
    }

    /// Use l'outil en main ; renvoie `true` s'il vient de casser.
    /// `roll` (0 à 1) : le tirage de l'enchantement Solidité, qui épargne parfois l'objet.
    pub fn wear_selected(&mut self, items: &ItemRegistry, roll: f32) -> bool {
        let slot = &mut self.slots[self.selected];
        let Some(stack) = slot.as_mut() else {
            return false;
        };
        let Some(durability) = items.get(stack.item).durability else {
            return false;
        };
        if spared(stack.damage, roll) {
            return false;
        }
        enchanting::add_wear(&mut stack.damage, 1);
        if enchanting::wear(stack.damage) >= durability {
            enchanting::release(stack.damage);
            *slot = None;
            return true;
        }
        false
    }

    /// L'armure encaisse un coup : chaque pièce portée s'use d'un point. Renvoie les
    /// pièces qui se sont brisées.
    pub fn wear_armor(&mut self, items: &ItemRegistry, roll: f32) -> Vec<ItemId> {
        let mut broken = Vec::new();
        for slot in &mut self.armor {
            let Some(stack) = slot.as_mut() else { continue };
            let Some(durability) = items.get(stack.item).durability else {
                continue;
            };
            if spared(stack.damage, roll) {
                continue;
            }
            enchanting::add_wear(&mut stack.damage, 1);
            if enchanting::wear(stack.damage) >= durability {
                broken.push(stack.item);
                enchanting::release(stack.damage);
                *slot = None;
            }
        }
        broken
    }

    pub fn slot_mut(&mut self, slot: Slot) -> &mut Option<ItemStack> {
        match slot {
            Slot::Main(i) => &mut self.slots[i],
            Slot::Armor(i) => &mut self.armor[i],
            Slot::Craft(i) => &mut self.craft[i],
        }
    }

    /// Un clic sur une case de l'inventaire, de l'armure ou de la grille.
    pub fn click(&mut self, slot: Slot, items: &ItemRegistry, click: Click) {
        let mut cursor = self.cursor.take();
        let target = self.slot_mut(slot);
        match slot {
            Slot::Armor(i) => click_slot(
                target,
                &mut cursor,
                items,
                |item| {
                    items
                        .get(item)
                        .armor
                        .is_some_and(|a| a.slot == ArmorSlot::ALL[i])
                },
                click,
            ),
            _ => click_slot(target, &mut cursor, items, |_| true, click),
        }
        self.cursor = cursor;
    }

    /// La protection totale de l'armure portée.
    pub fn protection(&self, items: &ItemRegistry) -> u32 {
        self.armor
            .iter()
            .flatten()
            .filter_map(|s| {
                let base = items.get(s.item).armor.map(|a| a.protection)?;
                Some(base + u32::from(enchanting::level_of(s.damage, Enchantment::PROTECTION)))
            })
            .sum()
    }

    /// Les objets de la grille d'artisanat (`size × size`), pour chercher une recette.
    pub fn craft_grid(&self, size: usize) -> Vec<Option<ItemId>> {
        (0..size * size)
            .map(|i| self.craft[(i / size) * 3 + i % size].map(|s| s.item))
            .collect()
    }

    /// Prend le résultat de la recette : un objet de chaque case de la grille est consommé.
    /// Le résultat va dans la main (s'il y a la place).
    pub fn take_craft_result(
        &mut self,
        recipe: &Recipe,
        size: usize,
        items: &ItemRegistry,
    ) -> bool {
        let (item, count) = recipe.result();
        let room = match self.cursor {
            None => true,
            Some(held) => held.item == item && held.count + count <= items.get(item).max_stack,
        };
        if !room {
            return false;
        }
        // Un sac qui monte de palier garde son numéro, donc son contenu.
        let inherited =
            crate::backpacks::inherited_number(self.craft.iter().flatten().copied(), item, items);
        for i in 0..size * size {
            let cell = &mut self.craft[(i / size) * 3 + i % size];
            if let Some(stack) = cell {
                stack.count -= 1;
                if stack.count == 0 {
                    *cell = None;
                }
            }
        }
        match &mut self.cursor {
            Some(held) => held.count += count,
            None => {
                self.cursor = Some(ItemStack {
                    damage: inherited,
                    ..ItemStack::new(item, count)
                })
            }
        }
        true
    }

    /// Range tout ce qui traîne (la grille, la main) dans l'inventaire ; renvoie ce qui
    /// n'a pas trouvé de place (à lâcher par terre).
    pub fn put_away(&mut self, items: &ItemRegistry) -> Vec<ItemStack> {
        let mut loose: Vec<ItemStack> = self.craft.iter_mut().filter_map(Option::take).collect();
        loose.extend(self.cursor.take());
        loose
            .into_iter()
            .filter_map(|stack| {
                let left = self.add_stack(stack, items);
                (left > 0).then_some(ItemStack {
                    count: left,
                    ..stack
                })
            })
            .collect()
    }

    /// Le nombre total d'un objet.
    pub fn count(&self, item: ItemId) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count)
            .sum()
    }

    /// Retire `count` objets de la barre et de la réserve. `false` (et rien n'est retiré)
    /// s'il n'y en a pas assez.
    pub fn remove(&mut self, item: ItemId, count: u32) -> bool {
        if self.count(item) < count {
            return false;
        }
        let mut left = count;
        for slot in self.slots.iter_mut() {
            let Some(stack) = slot else { continue };
            if stack.item != item || left == 0 {
                continue;
            }
            let taken = stack.count.min(left);
            stack.count -= taken;
            left -= taken;
            if stack.count == 0 {
                *slot = None;
            }
        }
        true
    }

    /// Les cases à prendre pour réunir les ingrédients d'une recette (une par ingrédient),
    /// `None` s'il en manque. Pour le livre de recettes.
    fn find_ingredients(&self, ingredients: &[&Ingredient]) -> Option<Vec<usize>> {
        let mut left: Vec<u32> = self
            .slots
            .iter()
            .map(|s| s.map_or(0, |s| s.count))
            .collect();
        let mut taken = Vec::new();
        // Les ingrédients précis d'abord : un groupe ne prend pas l'objet exact qu'un
        // autre ingrédient demande.
        let mut order: Vec<&&Ingredient> = ingredients.iter().collect();
        order.sort_by_key(|i| matches!(i, Ingredient::Any(_)));
        for ingredient in order {
            let index = (0..SLOTS).find(|&i| {
                left[i] > 0 && self.slots[i].is_some_and(|s| ingredient.matches(s.item))
            })?;
            left[index] -= 1;
            taken.push(index);
        }
        Some(taken)
    }

    /// Une recette faisable avec ce qu'on a ?
    pub fn can_craft(&self, recipe: &Recipe) -> bool {
        self.find_ingredients(&recipe.ingredients()).is_some()
    }

    /// Fabrique une recette du livre : prend les ingrédients dans l'inventaire et range le
    /// résultat. Renvoie le reste qui n'a pas trouvé de place (0 si tout est rangé).
    pub fn craft_from_book(&mut self, recipe: &Recipe, items: &ItemRegistry) -> Option<u32> {
        let taken = self.find_ingredients(&recipe.ingredients())?;
        let (item, count) = recipe.result();
        let inherited = crate::backpacks::inherited_number(
            taken.iter().filter_map(|&i| self.slots[i]),
            item,
            items,
        );
        for i in taken {
            let slot = &mut self.slots[i];
            if let Some(stack) = slot {
                stack.count -= 1;
                if stack.count == 0 {
                    *slot = None;
                }
            }
        }
        Some(self.add_stack(
            ItemStack {
                damage: inherited,
                ..ItemStack::new(item, count)
            },
            items,
        ))
    }

    /// Pour `world.ron` : une palette d'identifiants et des cases qui y renvoient.
    pub fn to_save(&self, items: &ItemRegistry) -> SavedInventory {
        let mut copy = self.clone();
        copy.put_away(items);
        let mut palette: Vec<String> = Vec::new();
        let mut save = |slot: &Option<ItemStack>| {
            slot.map(|s| {
                let key = &items.get(s.item).key;
                let index = palette.iter().position(|k| k == key).unwrap_or_else(|| {
                    palette.push(key.clone());
                    palette.len() - 1
                });
                SavedStack {
                    item: index as u32,
                    count: s.count,
                    damage: s.damage,
                }
            })
        };
        let slots = copy.slots.iter().map(&mut save).collect();
        let armor = copy.armor.iter().map(&mut save).collect();
        SavedInventory {
            palette,
            slots,
            armor,
            selected: self.selected as u32,
        }
    }

    /// L'inverse de [`Inventory::to_save`]. Un objet d'un mod absent est perdu (avec un avertissement).
    pub fn from_save(saved: &SavedInventory, items: &ItemRegistry) -> Self {
        let mut inventory = Self::default();
        let ids: Vec<Option<ItemId>> = saved
            .palette
            .iter()
            .map(|key| {
                let id = items.id(key);
                if id.is_none() {
                    log::warn!("objet « {key} » inconnu (mod absent ?) : retiré de l'inventaire");
                }
                id
            })
            .collect();
        let load = |s: &Option<SavedStack>| {
            let s = s.as_ref()?;
            let item = (*ids.get(s.item as usize)?)?;
            Some(ItemStack {
                item,
                count: s.count.clamp(1, items.get(item).max_stack.max(1)),
                damage: s.damage,
            })
        };
        for (slot, saved) in inventory.slots.iter_mut().zip(&saved.slots) {
            *slot = load(saved);
        }
        for (slot, saved) in inventory.armor.iter_mut().zip(&saved.armor) {
            *slot = load(saved);
        }
        inventory.selected = (saved.selected as usize).min(HOTBAR - 1);
        inventory
    }

    /// L'ancien format (avant les objets) : (identifiant de bloc, quantité) par case.
    pub fn from_old_save(saved: &[Option<(String, u32)>], items: &ItemRegistry) -> Self {
        let mut inventory = Self::default();
        for (slot, saved) in inventory.slots.iter_mut().zip(saved) {
            if let Some((key, count)) = saved
                && let Some(item) = items.id(key)
            {
                *slot = Some(ItemStack::new(item, *count));
            }
        }
        inventory
    }
}

/// Le départ d'un nouveau monde : le Guide du voyageur et de quoi tenir la première journée.
/// Tout le reste se gagne (le livre de quêtes commence par la première bûche).
pub fn starter_inventory(items: &ItemRegistry) -> Inventory {
    let mut inventory = Inventory::default();
    for (key, count) in [("core:guide", 1), ("core:bread", 3)] {
        if let Some(id) = items.id(key) {
            inventory.add(id, count, items);
        }
    }
    inventory
}

#[cfg(test)]
mod tests {
    use engine_voxel::{BlockRegistry, Recipes};

    use super::*;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&BlockRegistry::builtin()).unwrap()
    }

    fn id(items: &ItemRegistry, key: &str) -> ItemId {
        items.id(key).unwrap()
    }

    #[test]
    fn right_click_takes_half_then_places_one_by_one() {
        let items = items();
        let torch = id(&items, "core:torch");
        let mut slot = Some(ItemStack::new(torch, 7));
        let mut cursor = None;
        // Main vide : la moitié, arrondie au-dessus.
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!(cursor.map(|c| c.count), Some(4));
        assert_eq!(slot.map(|s| s.count), Some(3));
        // Pile en main : un seul objet posé, dans une case vide puis sur la même pile.
        let mut empty = None;
        click_slot(&mut empty, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!(empty.map(|s| s.count), Some(1));
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!(slot.map(|s| s.count), Some(4));
        assert_eq!(cursor.map(|c| c.count), Some(2));
        // Un autre objet dans la case : rien ne bouge.
        let mut other = Some(ItemStack::new(id(&items, "core:oak_log"), 3));
        click_slot(&mut other, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!(other.map(|s| s.count), Some(3));
        assert_eq!(cursor.map(|c| c.count), Some(2));
        // Le dernier objet posé vide la main.
        click_slot(&mut empty, &mut cursor, &items, |_| true, Click::Secondary);
        click_slot(&mut empty, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!(cursor, None);
        assert_eq!(empty.map(|s| s.count), Some(3));
        // Une pile d'un seul objet : la moitié arrondie, c'est l'objet entier.
        let mut one = Some(ItemStack::new(torch, 1));
        click_slot(&mut one, &mut cursor, &items, |_| true, Click::Secondary);
        assert_eq!((one, cursor.map(|c| c.count)), (None, Some(1)));
    }

    #[test]
    fn take_n_respects_the_slot_and_the_held_stack() {
        let items = items();
        let torch = id(&items, "core:torch");
        let max = items.get(torch).max_stack;
        let mut slot = Some(ItemStack::new(torch, 20));
        let mut cursor = None;
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Take(5));
        assert_eq!(
            (slot.map(|s| s.count), cursor.map(|c| c.count)),
            (Some(15), Some(5))
        );
        // Plus que la case n'en a : tout ce qu'elle a.
        click_slot(&mut slot, &mut cursor, &items, |_| true, Click::Take(100));
        assert_eq!((slot, cursor.map(|c| c.count)), (None, Some(20)));
        // La pile tenue ne dépasse pas sa taille maximale.
        let mut full = Some(ItemStack::new(torch, max));
        click_slot(&mut full, &mut cursor, &items, |_| true, Click::Take(max));
        assert_eq!(cursor.map(|c| c.count), Some(max));
        assert_eq!(full.map(|s| s.count), Some(20));
        // Un objet différent en main : on ne prend rien.
        let mut logs = Some(ItemStack::new(id(&items, "core:oak_log"), 4));
        click_slot(&mut logs, &mut cursor, &items, |_| true, Click::Take(2));
        assert_eq!(logs.map(|s| s.count), Some(4));
    }

    #[test]
    fn armor_wears_out_and_breaks() {
        let items = items();
        let mut inv = Inventory::default();
        let chest = id(&items, "core:iron_chestplate");
        let durability = items.get(chest).durability.unwrap();
        inv.armor[1] = Some(ItemStack::new(chest, 1));
        for _ in 0..durability - 1 {
            assert!(inv.wear_armor(&items, 1.0).is_empty());
        }
        assert_eq!(
            inv.wear_armor(&items, 1.0),
            vec![chest],
            "le dernier coup la brise"
        );
        assert!(inv.armor[1].is_none());
    }

    #[test]
    fn take_all_empties_everything() {
        let items = items();
        let mut inv = Inventory::default();
        inv.add(id(&items, "core:stick"), 70, &items);
        inv.armor[1] = Some(ItemStack::new(id(&items, "core:iron_chestplate"), 1));
        inv.craft[4] = Some(ItemStack::new(id(&items, "core:coal"), 3));
        inv.cursor = Some(ItemStack::new(id(&items, "core:coal"), 2));
        let taken: u32 = inv.take_all().iter().map(|s| s.count).sum();
        assert_eq!(taken, 70 + 1 + 3 + 2);
        assert!(inv.take_all().is_empty(), "plus rien");
    }

    #[test]
    fn stacks_respect_their_maximum() {
        let items = items();
        let (stone, pickaxe) = (id(&items, "core:stone"), id(&items, "core:iron_pickaxe"));
        let mut inv = Inventory::default();
        assert_eq!(inv.add(stone, 70, &items), 0);
        assert_eq!(inv.slots[0].unwrap().count, 64);
        assert_eq!(inv.slots[1].unwrap().count, 6);
        // Les outils ne s'empilent pas.
        inv.add(pickaxe, 2, &items);
        assert_eq!(inv.slots[2].unwrap().count, 1);
        assert_eq!(inv.slots[3].unwrap().count, 1);
        assert_eq!(inv.add(stone, 64 * 40, &items), 64 * 40 - (64 * 32 + 58));
    }

    #[test]
    fn tools_wear_out_and_break() {
        let items = items();
        let pickaxe = id(&items, "core:wooden_pickaxe");
        let durability = items.get(pickaxe).durability.unwrap();
        let mut inv = Inventory::default();
        inv.add(pickaxe, 1, &items);
        for _ in 0..durability - 1 {
            assert!(!inv.wear_selected(&items, 1.0));
        }
        assert_eq!(inv.selected_stack().unwrap().damage, durability - 1);
        assert!(
            inv.wear_selected(&items, 1.0),
            "il casse à la dernière utilisation"
        );
        assert!(inv.selected_stack().is_none());
    }

    #[test]
    fn armor_slots_only_take_their_piece() {
        let items = items();
        let (helmet, boots) = (
            id(&items, "core:iron_helmet"),
            id(&items, "core:iron_boots"),
        );
        let mut inv = Inventory {
            cursor: Some(ItemStack::new(boots, 1)),
            ..Default::default()
        };
        inv.click(Slot::Armor(0), &items, Click::Primary); // la tête refuse des bottes
        assert!(inv.armor[0].is_none());
        inv.click(Slot::Armor(3), &items, Click::Primary);
        assert_eq!(inv.armor[3].unwrap().item, boots);
        inv.cursor = Some(ItemStack::new(helmet, 1));
        inv.click(Slot::Armor(0), &items, Click::Primary);
        assert_eq!(inv.protection(&items), 4);
    }

    #[test]
    fn crafting_in_the_grid_and_from_the_book() {
        let items = items();
        let recipes = Recipes::builtin(&items).unwrap();
        let (log, planks, stick) = (
            id(&items, "core:oak_log"),
            id(&items, "core:oak_planks"),
            id(&items, "core:stick"),
        );
        let mut inv = Inventory::default();
        // Une bûche dans la grille 2 × 2 : 4 planches.
        inv.craft[4] = Some(ItemStack::new(log, 2));
        let recipe = recipes.find(&inv.craft_grid(2), 2).unwrap().clone();
        assert!(inv.take_craft_result(&recipe, 2, &items));
        assert_eq!(inv.cursor, Some(ItemStack::new(planks, 4)));
        assert_eq!(inv.craft[4].unwrap().count, 1);
        // Le livre : des bâtons avec les planches de l'inventaire.
        inv.put_away(&items);
        let sticks = recipes.iter().find(|r| r.result().0 == stick).unwrap();
        assert!(inv.can_craft(sticks));
        assert_eq!(inv.craft_from_book(sticks, &items), Some(0));
        assert_eq!(inv.count(stick), 4);
        assert_eq!(inv.count(planks), 2);
    }

    #[test]
    fn save_round_trip_with_palette() {
        let items = items();
        let (stone, pickaxe, helmet) = (
            id(&items, "core:stone"),
            id(&items, "core:iron_pickaxe"),
            id(&items, "core:golden_helmet"),
        );
        let mut inv = Inventory::default();
        inv.add(stone, 70, &items);
        inv.add_stack(
            ItemStack {
                item: pickaxe,
                count: 1,
                damage: 17,
            },
            &items,
        );
        inv.armor[0] = Some(ItemStack::new(helmet, 1));
        inv.selected = 2;
        let mut saved = inv.to_save(&items);
        assert_eq!(
            saved.palette,
            vec!["core:stone", "core:iron_pickaxe", "core:golden_helmet"]
        );
        let back = Inventory::from_save(&saved, &items);
        assert_eq!(back, inv);
        // Un objet d'un mod disparu est retiré, le reste est gardé.
        saved.palette[0] = "absent:thing".into();
        let back = Inventory::from_save(&saved, &items);
        assert_eq!(back.count(stone), 0);
        assert_eq!(back.slots[2].unwrap().damage, 17);
    }
}
