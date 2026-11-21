//! Les wagons côté joueur : poser, monter, atteler, casser, ouvrir leur inventaire, et les
//! blocs qui les servent (rails détecteurs, chargeurs et déchargeurs). La physique et le dessin
//! sont dans `carts.rs`.
//!
//! - **Clic droit** avec un wagon en main sur un rail : le pose. Sur un wagonnet : on s'y assoit
//!   (Maj pour descendre). Sur un wagon coffre, trémie ou une locomotive : son inventaire.
//!   Avec l'attelage : un premier wagon, puis un second (les deux sont attelés ; un nouveau clic
//!   sur la même paire les détache).
//! - **Clic gauche** : un coup fait rougir le wagon, le second le casse (il lâche son objet et
//!   son contenu).
//! - **Chargeur et déchargeur** : un wagon coffre ou trémie arrêté sur un rail voisin est rempli
//!   depuis le chargeur, ou vidé dans le déchargeur ; le bloc émet un signal quand il a fini
//!   (pour relancer un rail accélérateur).
//! - **Rails détecteurs** : le système de pas fixe de `carts.rs` range les changements, que
//!   [`VoxelGame::update_cart_blocks`] applique aux machines.

use crate::key_codes::KeyCodes;
use std::collections::HashSet;

use engine_voxel::{Inventory, ItemStack, RailKind, rail_at};

use super::*;
use crate::carts::{Cart, CartCargo, CartConfig, CartKind, CartPending, CartSignals};

/// Les chargeurs passent tous les 5 pas de simulation (12 fois par seconde).
const LOADER_EVERY: u32 = 5;
/// Combien d'objets un chargeur déplace à chaque passage.
const LOADER_BATCH: u32 = 4;
/// Distance au-delà de laquelle l'attelage ne relie plus deux wagons.
const COUPLING_REACH: f32 = 6.0;

impl VoxelGame {
    /// Chaque image : la vitesse maximale des options, et Maj pour descendre.
    pub(crate) fn update_carts(&mut self, ctx: &Context) {
        if let Some(mut config) = self.server.ecs.get_resource_mut::<CartConfig>() {
            config.max_speed = self
                .settings
                .cart_max_speed
                .clamp(1.0, engine_voxel::SPEED_LIMIT);
        }
        let sneak = ctx
            .input
            .key_pressed(self.settings.keys.key(Action::Descend))
            || ctx.input.key_pressed(KeyCode::ShiftRight);
        if sneak && self.client.mouse_captured && !self.client.ui.inventory_open {
            self.send_now(crate::protocol::ClientMsg::Dismount);
        }
    }

    /// Les wagons autour du joueur, pour la commande `carts`.
    pub(crate) fn carts_json(&self) -> String {
        let mut list = Vec::new();
        for entity in carts::all(&self.server.ecs) {
            let (Some(cart), Some(transform)) = (
                self.server.ecs.get::<Cart>(entity),
                self.server.ecs.get::<Transform>(entity),
            ) else {
                continue;
            };
            let items: u32 = self
                .server
                .ecs
                .get::<CartCargo>(entity)
                .map_or(0, |c| c.0.slots.iter().flatten().map(|s| s.count).sum());
            let rail = cart
                .rail_block()
                .map_or("null".to_string(), |b| format!("[{},{},{}]", b.x, b.y, b.z));
            let p = transform.position;
            list.push(format!(
                "{{\"id\":{},\"kind\":\"{}\",\"pos\":[{:.2},{:.2},{:.2}],\"speed\":{:.2},\"on_rail\":{},\"rail\":{},\"rider\":{},\"links\":{:?},\"items\":{},\"burn\":{:.1},\"frozen\":{}}}",
                cart.id,
                cart.kind.name(),
                p.x,
                p.y,
                p.z,
                cart.speed(),
                cart.on_rail(),
                rail,
                cart.rider.is_some(),
                cart.links,
                items,
                cart.burn,
                cart.frozen,
            ));
        }
        format!("{{\"ok\":true,\"carts\":[{}]}}", list.join(","))
    }

    /// Le wagon de ce numéro.
    pub(crate) fn cart_by_id(&self, id: u32) -> Option<Entity> {
        carts::all(&self.server.ecs)
            .into_iter()
            .find(|&e| self.server.ecs.get::<Cart>(e).is_some_and(|c| c.id == id))
    }

    /// Est-ce un rail détecteur allumé ? (pour les essais)
    #[allow(dead_code)]
    pub(crate) fn detector_lit(&self, pos: IVec3) -> bool {
        rail_at(&*self.server.streamer(), pos).is_some_and(|r| r.kind == RailKind::Detector)
            && self
                .server
                .ecs
                .get_resource::<CartSignals>()
                .is_some_and(|s| s.lit.contains(&pos))
    }
}

/// Déplace jusqu'à `budget` objets de `from` vers `to`, type par type.
pub fn move_items(from: &mut Inventory, to: &mut Inventory, budget: u32) -> u32 {
    let mut kinds: Vec<String> = Vec::new();
    for stack in from.slots.iter().flatten() {
        if !kinds.contains(&stack.item) {
            kinds.push(stack.item.clone());
        }
    }
    let mut moved = 0;
    for item in kinds {
        if moved >= budget {
            break;
        }
        let want = from.count(&item).min(budget - moved);
        let taken = to.insert(None, &item, want, false);
        if taken > 0 {
            from.extract(None, Some(&item), taken, false);
            moved += taken;
        }
    }
    moved
}

/// Plus rien à déplacer : `from` est vide, ou `to` ne peut rien recevoir de ce qu'il contient.
pub fn stuck(from: &Inventory, to: &mut Inventory) -> bool {
    from.slots
        .iter()
        .flatten()
        .all(|s: &ItemStack| to.insert(None, &s.item, 1, true) == 0)
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Déplace quelques objets entre le bloc `at` et le wagon (`load` : du bloc vers le wagon),
    /// si `work` (le wagon est arrêté). Renvoie `true` quand il n'y a plus rien à déplacer : tout est passé, ou la destination est pleine.
    fn transfer(&mut self, cart: Entity, at: IVec3, load: bool, work: bool) -> bool {
        let mut streamer = self.streamer_mut();
        let Some(entity) = streamer.block_entity_mut(at) else {
            return false;
        };
        let Some(block_inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
            return false;
        };
        let Some(mut cargo) = self.ecs.get_mut::<CartCargo>(cart) else {
            return false;
        };
        let (from, to) = if load {
            (block_inventory, &mut cargo.0)
        } else {
            (&mut cargo.0, block_inventory)
        };
        if work {
            move_items(from, to, LOADER_BATCH);
        }
        stuck(from, to)
    }

    /// Le wagon trémie prend quelques objets du conteneur en `above`.
    fn pull_from_above(&mut self, cart: Entity, above: IVec3) {
        let mut streamer = self.streamer_mut();
        let Some(source) = streamer
            .block_entity_mut(above)
            .and_then(|e| e.caps.as_mut())
            .and_then(|c| c.inventory.as_mut())
        else {
            return;
        };
        if let Some(mut cargo) = self.ecs.get_mut::<CartCargo>(cart) {
            move_items(source, &mut cargo.0, LOADER_BATCH);
        }
    }

    /// Les chargeurs et déchargeurs voisins d'un wagon coffre ou trémie arrêté sur un rail.
    fn run_loaders(&mut self) {
        // Un wagon à l'arrêt est chargé ; un wagon qui roule (qui repart) garde le signal tant qu'il
        // est sur le rail voisin, pour que l'accélérateur finisse de le pousser.
        let mut present: Vec<(Entity, IVec3, bool)> = Vec::new();
        let mut hoppers: Vec<(Entity, IVec3)> = Vec::new();
        for entity in carts::all(&self.ecs) {
            let Some(cart) = self.ecs.get::<Cart>(entity) else {
                continue;
            };
            if matches!(cart.kind, CartKind::Chest | CartKind::Hopper)
                && !cart.frozen
                && let Some(block) = cart.rail_block()
            {
                present.push((entity, block, cart.speed() < 0.1));
                if cart.kind == CartKind::Hopper {
                    hoppers.push((entity, block));
                }
            }
        }
        // Le wagon trémie vide le conteneur posé au-dessus du rail (sauf rail activateur alimenté).
        for (cart, rail) in hoppers {
            let cut = rail_at(&*self.streamer(), rail)
                .is_some_and(|r| r.kind == RailKind::Activator && r.powered);
            if !cut {
                self.pull_from_above(cart, rail + IVec3::Y);
            }
        }
        let mut finished: HashSet<IVec3> = HashSet::new();
        for (cart, rail, halted) in present {
            for offset in engine_voxel::FACINGS {
                let at = rail + offset;
                let behavior = {
                    let block = self.streamer().world().block(at);
                    self.registry.def(block).behavior.clone()
                };
                let load = match behavior.as_deref() {
                    Some("core:cart_loader") => true,
                    Some("core:cart_unloader") => false,
                    _ => continue,
                };
                if self.transfer(cart, at, load, halted) {
                    finished.insert(at);
                }
            }
        }
        // Le signal : allumé quand le travail est fini, éteint sinon (ou si le wagon est parti).
        let previous = std::mem::take(&mut self.ecs.resource_mut::<CartSignals>().loaders);
        for &at in finished.difference(&previous) {
            self.mods.set_power(at, 15);
        }
        for &at in previous.difference(&finished) {
            self.mods.set_power(at, 0);
        }
        self.ecs.resource_mut::<CartSignals>().loaders = finished;
    }

    /// À chaque pas fixe : les signaux des rails détecteurs, puis (de temps en temps) les chargeurs.
    pub(crate) fn update_cart_blocks(&mut self) {
        if self.ecs.get_resource::<CartSignals>().is_none() {
            return;
        }
        let pending = std::mem::take(&mut self.ecs.resource_mut::<CartSignals>().pending);
        for (pos, on) in pending {
            self.mods.set_power(pos, if on { 15 } else { 0 });
        }
        let due = {
            let mut signals = self.ecs.resource_mut::<CartSignals>();
            signals.tick += 1;
            signals.tick.is_multiple_of(LOADER_EVERY)
        };
        if due {
            self.run_loaders();
        }
    }

    /// S'asseoir dans un wagonnet.
    pub(crate) fn ride(&mut self, cart: Entity) -> bool {
        if player::mode(&self.ecs, self.players.entity()) == MoveMode::Fly {
            self.toast("Pas en vol");
            return false;
        }
        if carts::board(&mut self.ecs, self.players.entity(), cart) {
            self.punch();
            self.toast("Maj : descendre. Avancer : pousser.");
            true
        } else {
            self.toast("Le wagonnet est occupé");
            false
        }
    }

    /// Pose le wagon tenu sur le rail visé.
    fn place_cart(&mut self, kind: CartKind) -> bool {
        let aim = self.aim();
        let Some(hit) = aim.hit else {
            return false;
        };
        if rail_at(&*self.streamer(), hit.block_pos).is_none() {
            return false;
        }
        // Pas de deux wagons au même endroit.
        let spot = hit.block_pos.as_vec3() + Vec3::new(0.5, 0.1, 0.5);
        let mut crowded = false;
        self.ecs
            .query::<(&Transform, &Cart)>()
            .for_each(|_, (t, _)| crowded |= t.position.distance(spot) < 0.9);
        if crowded {
            self.toast("Un wagon est déjà là");
            return false;
        }
        let look = aim.forward;
        if carts::place(&mut self.ecs, kind, hit.block_pos, look).is_none() {
            return false;
        }
        self.players.me_mut().inventory.consume_selected();
        self.punch();
        self.sound("core:place_hard");
        true
    }

    /// La clé de l'objet tenu en main.
    fn held_key(&self) -> Option<String> {
        let item = self.players.me().inventory.selected_item()?;
        Some(self.content.items.get(item).key.clone())
    }

    /// Un clic de l'attelage sur un wagon : le premier est retenu, le second est relié au premier.
    fn couple_click(&mut self, entity: Entity) -> bool {
        let first = self
            .ecs
            .resource::<CartPending>()
            .0
            .filter(|&e| self.ecs.is_alive(e));
        let Some(first) = first else {
            self.ecs.resource_mut::<CartPending>().0 = Some(entity);
            self.toast("Attelage : choisis un second wagon");
            return true;
        };
        self.ecs.resource_mut::<CartPending>().0 = None;
        if first == entity {
            self.toast("Attelage annulé");
            return true;
        }
        let position = |world: &engine_ecs::World, e| world.get::<Transform>(e).map(|t| t.position);
        let (Some(a), Some(b)) = (position(&self.ecs, first), position(&self.ecs, entity)) else {
            return true;
        };
        if a.distance(b) > COUPLING_REACH {
            self.toast("Ces wagons sont trop éloignés");
            return true;
        }
        match carts::toggle_coupling(&self.ecs, first, entity) {
            Some(true) => self.toast("Wagons attelés"),
            Some(false) => self.toast("Wagons détachés"),
            None => {}
        }
        self.punch();
        true
    }

    /// Casse un wagon : son objet et son contenu tombent à sa place.
    pub(crate) fn break_cart(&mut self, entity: Entity) {
        let Some((kind, position)) = self
            .ecs
            .get::<Cart>(entity)
            .map(|c| c.kind)
            .zip(self.ecs.get::<Transform>(entity).map(|t| t.position))
        else {
            return;
        };
        let contents = carts::remove(&mut self.ecs, entity);
        let at = (position + Vec3::Y * 0.3).floor().as_ivec3();
        let items = self.content.items.clone();
        if let Some(id) = items.id(&kind.item_key()) {
            self.drop_stack(at, inventory::ItemStack::new(id, 1));
        }
        for stack in contents {
            if let Some(item) = items.id(&stack.item) {
                self.drop_stack(
                    at,
                    inventory::ItemStack {
                        item,
                        count: stack.count,
                        damage: stack.damage,
                    },
                );
            }
        }
    }

    /// Un clic droit : poser un wagon sur un rail, atteler, monter ou ouvrir un wagon.
    pub(crate) fn use_cart(&mut self) -> bool {
        let held = self.held_key();
        if held.as_deref() == Some("core:coupler") {
            return self.aimed_cart().is_some_and(|e| self.couple_click(e));
        }
        if let Some(kind) = held.as_deref().and_then(CartKind::from_item_key)
            && self.place_cart(kind)
        {
            return true;
        }
        let Some(entity) = self.aimed_cart() else {
            return false;
        };
        let Some(kind) = self.ecs.get::<Cart>(entity).map(|c| c.kind) else {
            return false;
        };
        if kind == CartKind::Minecart {
            self.ride(entity)
        } else {
            let Some(id) = self.ecs.get::<crate::entities::NetId>(entity).map(|id| *id) else {
                return false;
            };
            self.open_screen(ui::Screen::Cart(id));
            true
        }
    }

    /// Un coup de poing sur un wagon : le premier le fait rougir, le second le casse.
    pub(crate) fn punch_cart(&mut self) -> bool {
        let Some(entity) = self.aimed_cart() else {
            return false;
        };
        let broken = {
            let Some(mut cart) = self.ecs.get_mut::<Cart>(entity) else {
                return false;
            };
            let broken = cart.hurt > 0.0;
            cart.hurt = 1.2;
            broken
        };
        self.punch();
        self.sound("core:break_soft");
        if broken {
            self.break_cart(entity);
        }
        true
    }

    /// Le wagon visé, s'il est plus près que le bloc visé.
    pub(crate) fn aimed_cart(&self) -> Option<Entity> {
        let aim = self.aim();
        let block_distance = aim
            .hit
            .map_or(self.players.me().gear.reach(), |hit| hit.distance);
        carts::hit_cart(&self.ecs, aim.eye, aim.forward, block_distance).map(|(e, _)| e)
    }
}

impl crate::server_side::VoxelServer {
    /// Descendre du wagonnet (Maj).
    pub(crate) fn dismount(&mut self) -> bool {
        if !self.ecs.has::<carts::Riding>(self.players.entity()) {
            return false;
        }
        carts::dismount(&mut self.ecs, self.players.entity());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
        })
    }

    #[test]
    fn loaders_move_a_few_items_at_a_time_and_report_when_done_or_full() {
        let mut chest = Inventory::new(2);
        let mut loader = Inventory::new(3);
        loader.slots[0] = stack("core:stone", 10);
        loader.slots[1] = stack("core:coal", 3);
        assert!(!stuck(&loader, &mut chest));
        assert_eq!(move_items(&mut loader, &mut chest, 4), 4);
        assert_eq!(chest.count("core:stone"), 4);
        assert_eq!(loader.count("core:stone"), 6);
        // Le wagon se remplit : plus rien ne passe, le chargeur a « fini ».
        for _ in 0..10 {
            move_items(&mut loader, &mut chest, 4);
        }
        assert!(stuck(&loader, &mut chest), "wagon plein");
        assert!(chest.slots.iter().all(Option::is_some));
        // Un chargeur vide a fini aussi.
        let mut empty = Inventory::new(3);
        let mut big = Inventory::new(27);
        assert!(stuck(&empty, &mut big));
        assert_eq!(move_items(&mut empty, &mut big, 4), 0);
    }
}
