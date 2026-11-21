//! La part serveur de `enchant_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `enchant_ui.rs`).

use crate::enchant_books::{self, BookKind, ENCHANTED_BOOK};
use crate::enchant_library::{self, Library};
use crate::enchant_table::{self, Offer, TableStats};
use crate::enchanting::{self, Enchantment};
use crate::inventory::ItemStack;
use glam::IVec3;

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Range ce que contient la case d'entrée (les réseaux y déposent) avant d'afficher.
    fn absorb_library_input(&mut self, pos: IVec3, max: u8) {
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos) {
            enchant_library::absorb(entity, max);
        }
    }

    fn write_library(&mut self, pos: IVec3, library: &Library) {
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos) {
            entity.data = library.write();
        }
    }

    /// Les stats de la table posée en `pos` (voir `enchant_table.rs`).
    pub(crate) fn table_stats(&self, pos: IVec3) -> TableStats {
        table_stats_in(&self.registry, self.streamer().world(), pos)
    }

    /// Les octets et le plafond de la bibliothèque en `pos`.
    fn library_state(&self, pos: IVec3) -> Option<(Library, u8)> {
        library_in(&self.registry, &self.streamer(), pos)
    }

    /// Dépose un livre enchanté (une pile) dans la bibliothèque. `Err(texte)` s'il est refusé.
    fn deposit_book(&mut self, pos: IVec3, book: &ItemStack) -> Result<(), String> {
        let Some((mut library, max)) = self.library_state(pos) else {
            return Err("Ce n'est pas une bibliothèque".into());
        };
        let enchants = enchanting::enchantments_of(book.damage);
        library.deposit(&enchants, max).map_err(|r| r.text())?;
        self.write_library(pos, &library);
        enchanting::release(book.damage);
        Ok(())
    }

    /// Paie un lot (points d'expérience et saphirs) et l'applique à la pile en main : un objet
    /// reçoit les enchantements, un livre ou un tome devient un livre enchanté.
    fn buy_offer(&mut self, offer: &Offer, gem: Option<engine_voxel::ItemId>) {
        let Some(gem) = gem else { return };
        if self.players.me_mut().experience.points < offer.points
            || !self.players.me_mut().inventory.remove(gem, offer.gems)
        {
            return;
        }
        self.players.me_mut().experience.points -= offer.points;
        let selected = self.players.me_mut().inventory.selected;
        let items = self.content.items.clone();
        let Some(held) = self.players.me_mut().inventory.slots[selected] else {
            return;
        };
        if BookKind::of_key(&items.get(held.item).key).is_some() {
            // Un livre vierge : un exemplaire est consommé, et un livre enchanté le remplace.
            if let Some(slot) = self.players.me_mut().inventory.slots[selected].as_mut() {
                slot.count -= 1;
                if slot.count == 0 {
                    self.players.me_mut().inventory.slots[selected] = None;
                }
            }
            if let Some(enchanted) = items.id(ENCHANTED_BOOK) {
                self.give_or_drop(ItemStack {
                    item: enchanted,
                    count: 1,
                    damage: offer.apply(0),
                });
            }
        } else if let Some(slot) = self.players.me_mut().inventory.slots[selected].as_mut() {
            slot.damage = offer.apply(slot.damage);
        }
        // La table propose autre chose la prochaine fois.
        self.enchant_seed = self
            .enchant_seed
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        self.toast(&offer.label());
        self.bell();
    }

    /// Range la case d'entrée de la bibliothèque (les réseaux y déposent), si elle n'est pas vide.
    pub(crate) fn absorb_library(&mut self, pos: IVec3) {
        let Some((_, max)) = self.library_state(pos) else {
            return;
        };
        let waiting = self
            .streamer()
            .block_entity(pos)
            .and_then(|e| {
                e.caps
                    .as_ref()?
                    .inventory
                    .as_ref()?
                    .slots
                    .get(enchant_library::INPUT)
                    .cloned()
                    .flatten()
            })
            .is_some();
        if waiting {
            self.absorb_library_input(pos, max);
        }
    }

    pub(crate) fn library_take_output(&mut self, pos: IVec3) {
        let items = self.content.items.clone();
        let mut taken = None;
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos)
            && let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut())
            && let Some(slot) = inventory.slots.get_mut(enchant_library::OUTPUT)
        {
            taken = slot.take();
        }
        if let Some(out) = taken
            && let Some(id) = items.id(&out.item)
        {
            self.give_or_drop(ItemStack {
                item: id,
                count: out.count,
                damage: out.damage,
            });
        }
    }

    pub(crate) fn library_deposit_cursor(&mut self, pos: IVec3) {
        self.absorb_library(pos);
        let book_id = self.content.items.id(ENCHANTED_BOOK);
        let Some(book) = self
            .players
            .me()
            .inventory
            .cursor
            .filter(|s| Some(s.item) == book_id && enchanting::is_enchanted(s.damage))
        else {
            return;
        };
        match self.deposit_book(pos, &book) {
            Ok(()) => {
                self.players.me_mut().inventory.cursor = None;
                self.toast("Livre rangé dans la bibliothèque");
                self.bell();
            }
            Err(why) => self.toast(&why),
        }
    }

    pub(crate) fn library_deposit_all(&mut self, pos: IVec3) {
        self.absorb_library(pos);
        let book_id = self.content.items.id(ENCHANTED_BOOK);
        let mut stored = 0;
        for i in 0..self.players.me().inventory.slots.len() {
            let Some(stack) = self.players.me().inventory.slots[i] else {
                continue;
            };
            if Some(stack.item) != book_id || !enchanting::is_enchanted(stack.damage) {
                continue;
            }
            if self.deposit_book(pos, &stack).is_ok() {
                self.players.me_mut().inventory.slots[i] = None;
                stored += 1;
            }
        }
        self.toast(&format!("{stored} livre(s) rangé(s)"));
        if stored > 0 {
            self.bell();
        }
    }

    /// Sort un livre enchanté de la bibliothèque, au niveau choisi, et consomme un livre vierge.
    pub(crate) fn extract_from_library(&mut self, pos: IVec3, kind: Enchantment, level: u8) {
        self.absorb_library(pos);
        let items = self.content.items.clone();
        let (Some(blank), Some(enchanted)) =
            (items.id(enchant_books::BOOK), items.id(ENCHANTED_BOOK))
        else {
            return;
        };
        let Some((mut library, _)) = self.library_state(pos) else {
            return;
        };
        if !library.extract(kind, level) || !self.players.me_mut().inventory.remove(blank, 1) {
            return;
        }
        self.write_library(pos, &library);
        self.give_or_drop(ItemStack {
            item: enchanted,
            count: 1,
            damage: enchant_books::book_damage(&[(kind, level)]),
        });
        self.toast(&enchanting::label(kind, level));
        self.bell();
    }

    /// `EnchantBuy` : le serveur refait la liste des lots de la table ouverte pour l'objet en main,
    /// et achète celui-ci s'il est abordable.
    pub(crate) fn buy_enchant_offer(&mut self, pos: IVec3, index: usize) {
        let stats = self.table_stats(pos);
        let Some(stack) = self.players.me().inventory.selected_stack() else {
            return;
        };
        let items = self.content.items.clone();
        let Some(subject) = table_subject(&items, &stack) else {
            return;
        };
        let data = crate::adventure_data::current();
        let key = items.get(stack.item).key.clone();
        let Some(offer) = enchant_table::offers(
            &data,
            subject,
            &key,
            stack.damage,
            self.enchant_seed,
            &stats,
        )
        .and_then(|list| list.into_iter().nth(index)) else {
            log::warn!("lot d'enchantement inconnu ({index}) : ignoré");
            return;
        };
        self.buy_offer(&offer, items.id("core:sapphire"));
    }
}

/// Les octets et le plafond d'une bibliothèque, lus dans `streamer`.
pub(crate) fn library_in(
    registry: &engine_voxel::BlockRegistry,
    streamer: &engine_voxel::ChunkStreamer,
    pos: IVec3,
) -> Option<(Library, u8)> {
    let max = enchant_library::max_level_of(
        registry
            .def(streamer.world().block(pos))
            .enchanting
            .as_ref(),
    );
    if max == 0 {
        return None;
    }
    let library = streamer
        .block_entity(pos)
        .map_or_else(Library::default, |e| Library::read(&e.data));
    Some((library, max))
}

/// Les stats d'une table, lues dans `world` (celui du serveur, ou la réplique de la fenêtre).
pub(crate) fn table_stats_in(
    registry: &engine_voxel::BlockRegistry,
    world: &engine_voxel::World,
    pos: IVec3,
) -> TableStats {
    enchant_table::read_stats(
        pos,
        |p| !registry.def(world.block(p)).solid,
        |p| {
            registry
                .def(world.block(p))
                .enchanting
                .clone()
                .filter(|s| s.library_level == 0)
        },
    )
}

/// Le sujet de la table : la pile en main, objet à enchanter ou livre à remplir.
pub(crate) fn table_subject<'a>(
    items: &'a engine_voxel::ItemRegistry,
    stack: &crate::inventory::ItemStack,
) -> Option<crate::enchant_table::Subject<'a>> {
    let def = items.get(stack.item);
    if def.key == crate::enchant_books::ENCHANTED_BOOK {
        return None;
    }
    Some(match crate::enchant_books::BookKind::of_key(&def.key) {
        Some(kind) => crate::enchant_table::Subject::Book(kind),
        None => crate::enchant_table::Subject::Item(def),
    })
}
