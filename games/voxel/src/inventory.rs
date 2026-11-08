//! L'inventaire : 9 cases de barre d'objets (touches 1 à 9) et 27 de réserve.
//!
//! Casser un bloc le met dans l'inventaire ; en poser un le retire. Les objets du même
//! type s'empilent par 64. On le sauvegarde avec des identifiants texte, comme les chunks.

use engine_voxel::{BlockId, BlockRegistry};

pub const HOTBAR: usize = 9;
pub const SLOTS: usize = 36;
pub const STACK_SIZE: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemStack {
    pub item: BlockId,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// Les cases 0 à 8 forment la barre d'objets, 9 à 35 la réserve.
    pub slots: [Option<ItemStack>; SLOTS],
    /// La case choisie dans la barre.
    pub selected: usize,
    /// La pile « tenue à la souris » quand l'inventaire est ouvert.
    pub cursor: Option<ItemStack>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
        }
    }
}

impl Inventory {
    /// Range `count` objets ; renvoie ce qui n'a pas trouvé de place.
    ///
    /// D'abord les piles existantes du même objet (barre puis réserve), puis les cases
    /// vides : on complète avant d'entamer une nouvelle pile.
    pub fn add(&mut self, item: BlockId, mut count: u32) -> u32 {
        for slot in self.slots.iter_mut().flatten() {
            if slot.item == item && slot.count < STACK_SIZE {
                let moved = count.min(STACK_SIZE - slot.count);
                slot.count += moved;
                count -= moved;
                if count == 0 {
                    return 0;
                }
            }
        }
        for slot in self.slots.iter_mut().filter(|s| s.is_none()) {
            let moved = count.min(STACK_SIZE);
            *slot = Some(ItemStack { item, count: moved });
            count -= moved;
            if count == 0 {
                return 0;
            }
        }
        count
    }

    /// L'objet de la case choisie.
    pub fn selected_item(&self) -> Option<BlockId> {
        self.slots[self.selected].map(|s| s.item)
    }

    /// Retire un objet de la case choisie (on vient de le poser).
    pub fn consume_selected(&mut self) -> Option<BlockId> {
        let slot = &mut self.slots[self.selected];
        let stack = slot.as_mut()?;
        let item = stack.item;
        stack.count -= 1;
        if stack.count == 0 {
            *slot = None;
        }
        Some(item)
    }

    /// Un clic sur une case, inventaire ouvert : prendre, poser, fusionner ou échanger
    /// avec la pile tenue à la souris.
    pub fn click(&mut self, index: usize) {
        let slot = &mut self.slots[index];
        match (slot.as_mut(), self.cursor.as_mut()) {
            // Même objet : on complète la pile de la case.
            (Some(stack), Some(held)) if stack.item == held.item => {
                let moved = held.count.min(STACK_SIZE - stack.count);
                stack.count += moved;
                held.count -= moved;
                if held.count == 0 {
                    self.cursor = None;
                }
            }
            // Sinon (prendre, poser, ou deux objets différents) : on échange.
            _ => std::mem::swap(slot, &mut self.cursor),
        }
    }

    /// Le nombre total d'un objet.
    #[cfg(test)]
    pub fn count(&self, item: BlockId) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count)
            .sum()
    }

    /// Pour `world.ron` : chaque case en (identifiant texte, quantité).
    pub fn to_save(&self, registry: &BlockRegistry) -> Vec<Option<(String, u32)>> {
        // La pile tenue à la souris retourne dans l'inventaire (ou se perd s'il est plein).
        let mut copy = self.clone();
        if let Some(held) = copy.cursor.take() {
            copy.add(held.item, held.count);
        }
        copy.slots
            .iter()
            .map(|slot| slot.map(|s| (registry.def(s.item).key.clone(), s.count)))
            .collect()
    }

    /// L'inverse de [`Inventory::to_save`]. Un objet d'un mod absent est perdu (avec un avertissement).
    pub fn from_save(saved: &[Option<(String, u32)>], registry: &BlockRegistry) -> Self {
        let mut inventory = Self::default();
        for (slot, saved) in inventory.slots.iter_mut().zip(saved) {
            let Some((key, count)) = saved else { continue };
            match registry.id(key) {
                Some(item) => {
                    *slot = Some(ItemStack {
                        item,
                        count: (*count).clamp(1, STACK_SIZE),
                    })
                }
                None => {
                    log::warn!("objet « {key} » inconnu (mod absent ?) : retiré de l'inventaire")
                }
            }
        }
        inventory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks() -> (BlockRegistry, BlockId, BlockId) {
        let registry = BlockRegistry::builtin().as_ref().clone();
        let stone = registry.id("core:stone").unwrap();
        let dirt = registry.id("core:dirt").unwrap();
        (registry, stone, dirt)
    }

    #[test]
    fn stacks_fill_up_before_new_ones_start() {
        let (_, stone, dirt) = blocks();
        let mut inv = Inventory::default();
        assert_eq!(inv.add(stone, 10), 0);
        assert_eq!(inv.add(dirt, 1), 0);
        assert_eq!(inv.add(stone, 60), 0);
        // 10 + 60 = 70 : une pile pleine de 64 et une nouvelle de 6.
        assert_eq!(
            inv.slots[0],
            Some(ItemStack {
                item: stone,
                count: 64
            })
        );
        assert_eq!(
            inv.slots[1],
            Some(ItemStack {
                item: dirt,
                count: 1
            })
        );
        assert_eq!(
            inv.slots[2],
            Some(ItemStack {
                item: stone,
                count: 6
            })
        );
        assert_eq!(inv.count(stone), 70);
    }

    #[test]
    fn overflow_is_returned_when_full() {
        let (_, stone, _) = blocks();
        let mut inv = Inventory::default();
        assert_eq!(inv.add(stone, 36 * 64 + 5), 5);
        assert_eq!(inv.count(stone), 36 * 64);
    }

    #[test]
    fn placing_consumes_the_selected_slot() {
        let (_, stone, _) = blocks();
        let mut inv = Inventory::default();
        inv.add(stone, 2);
        assert_eq!(inv.consume_selected(), Some(stone));
        assert_eq!(inv.consume_selected(), Some(stone));
        assert_eq!(inv.consume_selected(), None);
        assert_eq!(inv.slots[0], None);
    }

    #[test]
    fn clicking_picks_up_merges_and_swaps() {
        let (_, stone, dirt) = blocks();
        let mut inv = Inventory::default();
        inv.slots[0] = Some(ItemStack {
            item: stone,
            count: 40,
        });
        inv.slots[1] = Some(ItemStack {
            item: stone,
            count: 30,
        });
        inv.slots[2] = Some(ItemStack {
            item: dirt,
            count: 5,
        });
        inv.click(0); // prendre les 40 pierres
        assert_eq!(
            inv.cursor,
            Some(ItemStack {
                item: stone,
                count: 40
            })
        );
        inv.click(1); // compléter la pile de 30 : 64, il en reste 6 en main
        assert_eq!(inv.slots[1].unwrap().count, 64);
        assert_eq!(inv.cursor.unwrap().count, 6);
        inv.click(2); // objet différent : échange
        assert_eq!(
            inv.cursor,
            Some(ItemStack {
                item: dirt,
                count: 5
            })
        );
        assert_eq!(
            inv.slots[2],
            Some(ItemStack {
                item: stone,
                count: 6
            })
        );
        inv.click(0); // case vide : on pose
        assert_eq!(inv.cursor, None);
        assert_eq!(
            inv.slots[0],
            Some(ItemStack {
                item: dirt,
                count: 5
            })
        );
    }

    #[test]
    fn save_round_trip_and_unknown_items() {
        let (registry, stone, dirt) = blocks();
        let mut inv = Inventory::default();
        inv.add(stone, 70);
        inv.cursor = Some(ItemStack {
            item: dirt,
            count: 3,
        });
        let mut saved = inv.to_save(&registry);
        assert_eq!(saved[0], Some(("core:stone".into(), 64)));
        // La pile en main a été rangée.
        assert!(saved.contains(&Some(("core:dirt".into(), 3))));
        saved[5] = Some(("absent:item".into(), 9));
        let back = Inventory::from_save(&saved, &registry);
        assert_eq!(back.count(stone), 70);
        assert_eq!(back.count(dirt), 3);
        assert_eq!(back.slots[5], None);
    }
}
