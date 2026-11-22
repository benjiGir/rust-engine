//! Les gestes des livres dans le jeu : un livre enchanté ou un tome d'extraction au curseur, un clic
//! sur un objet. Les règles (niveaux, application, extraction) sont dans `enchant_books.rs`.

use crate::enchanting;
use crate::enchanting::books::{
    BOOK, BookKind, ENCHANTED_BOOK, TOME_EXTRACTION, apply_book, extract,
};
use crate::player::inventory::{Click, ItemStack, Slot};
use crate::server::screens::Screen;

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un livre enchanté ou un tome d'extraction au curseur, un clic sur un objet de l'inventaire,
    /// dans l'écran d'une table d'enchantement ou d'une enclume. `true` si le clic est traité.
    pub fn try_book_click(&mut self, slot: Slot, click: Click) -> bool {
        if click != Click::Primary
            || !matches!(
                self.players.me().screen,
                Some(Screen::Enchant(_) | Screen::Anvil)
            )
        {
            return false;
        }
        let (Some(held), Some(target)) = (
            self.players.me_mut().inventory.cursor,
            *self.players.me_mut().inventory.slot_mut(slot),
        ) else {
            return false;
        };
        let data = crate::adventure::data::current();
        let items = self.content.items.clone();
        let held_key = items.get(held.item).key.clone();
        let target_def = items.get(target.item);
        if target_def.key == ENCHANTED_BOOK || BookKind::of_key(&target_def.key).is_some() {
            return false;
        }
        if held_key == ENCHANTED_BOOK {
            let book = enchanting::enchantments_of(held.damage);
            if target_def.tool.is_none() && target_def.armor.is_none() {
                return false;
            }
            let Some((damage, changed)) = apply_book(&data, target_def, target.damage, &book)
            else {
                self.toast("Ce livre n'apporte rien à cet objet");
                return true;
            };
            if let Some(stack) = self.players.me_mut().inventory.slot_mut(slot) {
                stack.damage = damage;
            }
            enchanting::release(held.damage);
            self.players.me_mut().inventory.cursor = None;
            let names: Vec<String> = changed
                .iter()
                .map(|&(kind, level)| enchanting::label(kind, level))
                .collect();
            self.toast(&names.join(" + "));
            self.bell();
            return true;
        }
        if held_key == TOME_EXTRACTION {
            let Some((stripped, book)) = extract(target.damage) else {
                self.toast("Cet objet n'a aucun enchantement");
                return true;
            };
            let (Some(blank), Some(enchanted)) = (items.id(BOOK), items.id(ENCHANTED_BOOK)) else {
                return false;
            };
            if !self.players.me_mut().inventory.remove(blank, 1) {
                self.toast("Il faut un livre vierge dans l'inventaire");
                return true;
            }
            if let Some(stack) = self.players.me_mut().inventory.slot_mut(slot) {
                stack.damage = stripped;
            }
            if let Some(cursor) = self.players.me_mut().inventory.cursor.as_mut() {
                cursor.count -= 1;
                if cursor.count == 0 {
                    self.players.me_mut().inventory.cursor = None;
                }
            }
            self.give_or_drop(ItemStack {
                item: enchanted,
                count: 1,
                damage: book,
            });
            self.toast("Enchantements extraits sur un livre");
            self.bell();
            return true;
        }
        false
    }

    /// Le petit son des gestes d'enchantement.
    pub fn bell(&mut self) {
        self.sound("core:enchant");
    }

    /// Range une pile dans l'inventaire, sinon la jette aux pieds du joueur.
    pub fn give_or_drop(&mut self, stack: ItemStack) {
        let left = self.collect_stack(stack);
        if left > 0 {
            let at = crate::player::body(&self.ecs, self.players.entity())
                .position
                .floor()
                .as_ivec3();
            self.drop_stack(
                at,
                ItemStack {
                    count: left,
                    ..stack
                },
            );
        }
    }
}
