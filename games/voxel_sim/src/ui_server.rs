//! La part serveur de `ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `ui.rs`).

use crate::inventory::{Click, ItemStack, Slot, click_slot};
use engine_ecs::Entity;
use engine_voxel::ItemId;
use glam::IVec3;

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// La pile rangée dans une case d'un wagon.
    pub fn cart_stack(&self, cart: Entity, index: usize) -> Option<ItemStack> {
        let cargo = self.ecs.get::<crate::carts::CartCargo>(cart)?;
        let stored = cargo.0.slots.get(index)?.as_ref()?;
        Some(ItemStack {
            item: self.content.items.id(&stored.item)?,
            count: stored.count,
            damage: stored.damage,
        })
    }

    /// Un clic sur une case d'un coffre ou d'une machine : la pile est relue puis réécrite
    /// dans l'entité du bloc (voir `storage_ui.rs` pour les règles des coffres à paliers).
    fn click_container(&mut self, pos: IVec3, index: usize, click: Click) -> bool {
        self.click_storage(crate::storage_ui_server::Target::Block(pos), index, click)
    }

    /// Un clic sur une case d'un wagon.
    fn click_cart(&mut self, cart: Entity, index: usize, click: Click) -> bool {
        let mut slot = self.cart_stack(cart, index);
        let mut cursor = self.players.me_mut().inventory.cursor.take();
        // Une locomotive n'accepte que du combustible.
        let fuel = self
            .ecs
            .get::<crate::carts::Cart>(cart)
            .is_some_and(|c| c.kind == crate::carts::CartKind::Locomotive);
        let accepts = |item: ItemId| !fuel || self.content.smelting.burn_ticks(item).is_some();
        click_slot(&mut slot, &mut cursor, &self.content.items, accepts, click);
        self.players.me_mut().inventory.cursor = cursor;
        let items = self.content.items.clone();
        if let Some(mut cargo) = self.ecs.get_mut::<crate::carts::CartCargo>(cart)
            && let Some(target) = cargo.0.slots.get_mut(index)
        {
            *target = slot.map(|s| engine_voxel::ItemStack {
                item: items.get(s.item).key.clone(),
                count: s.count,
                damage: s.damage,
            });
        }
        true
    }

    /// Applique un clic à une case du joueur ou d'un conteneur.
    pub fn click_target(&mut self, target: SlotRef, click: Click) -> bool {
        match target {
            SlotRef::Player(slot) => {
                // Une gemme ou un sceau au curseur sur un objet : sertir, retirer, ajouter.
                // Un livre, un tome ou une potion au curseur : voir `enchant_books.rs` et `charms.rs`.
                if !self.try_gem_click(slot, click)
                    && !self.try_book_click(slot, click)
                    && !self.try_charm_recharge(slot, click)
                {
                    self.players
                        .me_mut()
                        .inventory
                        .click(slot, &self.content.items, click);
                }
                true
            }
            // Résolue par le serveur avant d'arriver ici (voir `slot_ref_for`).
            SlotRef::Open(_) => false,
            SlotRef::Container(pos, index) => self.click_container(pos, index, click),
            SlotRef::Cart(cart, index) => self.click_cart(cart, index, click),
            SlotRef::Bag(number, index) => {
                self.click_storage(crate::storage_ui_server::Target::Bag(number), index, click)
            }
        }
    }
}

/// Une case, du joueur ou d'un conteneur (coffre, four).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotRef {
    Player(Slot),
    /// Une case du conteneur ouvert, quel qu'il soit : c'est ainsi que la fenêtre les désigne (le
    /// serveur sait ce que le joueur a ouvert, voir `slot_ref_for`).
    Open(usize),
    Container(IVec3, usize),
    /// Une case de l'inventaire d'un wagon.
    Cart(Entity, usize),
    /// Une case d'un sac à dos (son numéro, la case).
    Bag(u32, usize),
}
