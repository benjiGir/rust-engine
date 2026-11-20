//! Les sacs à dos : un contenu qui suit le sac, et des améliorations qui agissent sur le joueur.
//!
//! # Où est le contenu
//!
//! L'objet « sac à dos » (`max_stack` 1) ne contient rien lui-même : son champ `damage` porte un
//! **numéro de sac** (de 1 à 65 535, jamais plus : les bits hauts de `damage` sont ceux de
//! l'enchantement et des fiches d'objets (bit 31, voir `item_data.rs`), que le sac ne doit pas
//! imiter). Le contenu est dans le [`BackpackStore`], le
//! magasin des sacs du monde, sauvegardé dans `backpacks.bin` à la racine du monde, à côté de
//! `world.ron` (donc partagé par toutes les dimensions). Un sac jeté au sol, rangé dans un coffre
//! ou ramassé garde son numéro : c'est la pile qui voyage, pas le contenu.
//!
//! Un sac neuf (fabriqué, `give`, explorateur de recettes) a `damage` 0 : il reçoit son numéro au
//! premier usage ([`BackpackStore::number_for`]). Le palier suit l'objet : un sac qui monte de palier
//! par recette garde son numéro (voir `Inventory::take_craft_result`), et le magasin range son
//! contenu à la nouvelle disposition la prochaine fois qu'il le voit.
//!
//! Les sacs ne sont ni des outils ni des armures : aucune usure, aucun enchantement. Ils n'ont pas
//! de durabilité, et leur numéro ne dépasse jamais les 16 bits bas de `damage`.
//!
//! Un sac **ne contient pas de sac** : tous les chemins qui y rangent un objet passent par
//! [`accepts`].
//!
//! # Les améliorations qui agissent sur le joueur
//!
//! Elles agissent tant que le sac est dans l'inventaire du joueur, ouvert ou non (voir [`carried`]) :
//! - **Ramassage** : un objet ramassé va d'abord dans le sac ([`pickup`]), s'il passe le filtre ;
//! - **Aimant** : les objets au sol dans un rayon de [`MAGNET_RADIUS`] blocs viennent vers le joueur ;
//! - **Réapprovisionnement** : une pile de la barre sous le quart de sa taille est complétée
//!   ([`restock`]) ;
//! - **Nourriture** : quand la faim a assez baissé pour ne rien gaspiller, le sac nourrit
//!   ([`feed_choice`]).
//!
//! Chacune se coupe d'un clic dans l'écran du sac (un bit par emplacement dans `Bag::disabled`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use engine_voxel::{
    CompactTable, Fill, ItemId, ItemRegistry, Layout, Policy, Upgrade, filter_items,
    installed_upgrades, maintain, storage_insert,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::VoxelGame;
use crate::inventory::{HOTBAR, Inventory, ItemStack};

/// Le plus grand numéro de sac : les 16 bits bas de `damage`.
pub const MAX_NUMBER: u32 = 0xFFFF;
/// Jusqu'où l'aimant attire les objets, en blocs.
pub const MAGNET_RADIUS: f32 = 6.0;
/// La vitesse des objets attirés, en blocs par seconde.
const MAGNET_SPEED: f32 = 4.5;
/// Secondes entre deux passages des améliorations lentes (réapprovisionnement, nourriture).
const PERIOD: f32 = 0.5;
const FILE: &str = "backpacks.bin";
/// Le groupe de filtre du Ramassage (celui du Néant est le 0).
pub const PICKUP_FILTER: usize = 1;

/// Un palier de sac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagTier {
    pub key: &'static str,
    pub name: &'static str,
    pub layout: Layout,
}

/// Du cuir au saphir : 27 cases et 1 amélioration, jusqu'à 108 et 4 (deux groupes de filtre).
pub const BAG_TIERS: [BagTier; 4] = [
    BagTier {
        key: "core:backpack_leather",
        name: "Sac à dos en cuir",
        layout: Layout::new(27, 1, 8),
    },
    BagTier {
        key: "core:backpack_copper",
        name: "Sac à dos de cuivre",
        layout: Layout::new(54, 2, 8),
    },
    BagTier {
        key: "core:backpack_iron",
        name: "Sac à dos de fer",
        layout: Layout::new(81, 3, 8),
    },
    BagTier {
        key: "core:backpack_sapphire",
        name: "Sac à dos de saphir",
        layout: Layout::new(108, 4, 8),
    },
];

/// Le palier d'un sac d'après la clé de son objet.
pub fn tier_of(key: &str) -> Option<usize> {
    BAG_TIERS.iter().position(|t| t.key == key)
}

pub fn is_backpack(key: &str) -> bool {
    tier_of(key).is_some()
}

/// Les lignes que la fiche d'un sac ajoute : ses cases, et son numéro (vide pour un autre objet).
pub fn card_lines(key: &str, damage: u32) -> Vec<(String, engine_render::egui::Color32)> {
    let Some(tier) = tier_of(key) else {
        return Vec::new();
    };
    let layout = BAG_TIERS[tier].layout;
    vec![
        (
            format!(
                "{} cases · {} amélioration(s)",
                layout.main, layout.upgrades
            ),
            crate::skin::TEXT,
        ),
        (
            if damage == 0 {
                "Pas encore utilisé : B ou clic droit".to_string()
            } else {
                format!("Sac n° {damage}")
            },
            crate::skin::MUTED,
        ),
    ]
}

/// Un sac refuse les sacs : à passer comme règle `accepts` de tout rangement dans un sac.
pub fn accepts(items: &ItemRegistry, item: ItemId) -> bool {
    !is_backpack(&items.get(item).key)
}

/// Le numéro que prend le sac issu d'une recette : celui du sac consommé (0 si aucun sac n'y
/// était, ou si le résultat n'est pas un sac).
pub fn inherited_number(
    mut consumed: impl Iterator<Item = ItemStack>,
    result: ItemId,
    items: &ItemRegistry,
) -> u32 {
    if !is_backpack(&items.get(result).key) {
        return 0;
    }
    consumed
        .find(|s| is_backpack(&items.get(s.item).key))
        .map_or(0, |s| s.damage)
}

/// Le contenu d'un sac, dans l'ordre des cases de la disposition de son palier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bag {
    pub tier: u8,
    pub slots: Vec<Option<engine_voxel::ItemStack>>,
    /// Un bit par emplacement d'amélioration : 1 = coupée.
    pub disabled: u32,
}

impl Bag {
    pub fn new(tier: usize) -> Self {
        Self {
            tier: tier as u8,
            slots: vec![None; BAG_TIERS[tier].layout.total()],
            disabled: 0,
        }
    }

    pub fn layout(&self) -> Layout {
        BAG_TIERS[usize::from(self.tier).min(BAG_TIERS.len() - 1)].layout
    }

    /// Range le contenu selon la disposition de `tier` (montée de palier).
    fn set_tier(&mut self, tier: usize) {
        if usize::from(self.tier) == tier {
            return;
        }
        let old = self.layout();
        self.slots =
            engine_voxel::relayout(std::mem::take(&mut self.slots), old, BAG_TIERS[tier].layout);
        self.tier = tier as u8;
    }
}

/// Le magasin des sacs du monde.
#[derive(Debug, Default, PartialEq)]
pub struct BackpackStore {
    bags: BTreeMap<u32, Bag>,
    next: u32,
    dirty: bool,
    /// Le temps écoulé depuis le dernier passage des améliorations lentes.
    pub timer: f32,
}

#[derive(Serialize, Deserialize)]
struct SavedStore {
    version: u32,
    next: u32,
    bags: Vec<(u32, Bag)>,
}

impl BackpackStore {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.bags.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.bags.is_empty()
    }

    pub fn bag(&self, number: u32) -> Option<&Bag> {
        self.bags.get(&number)
    }

    /// Le sac, à modifier (il sera sauvegardé).
    pub fn bag_mut(&mut self, number: u32) -> Option<&mut Bag> {
        self.dirty = true;
        self.bags.get_mut(&number)
    }

    /// Le numéro du sac de cette pile, avec le contenu rangé selon son palier : un sac neuf
    /// (`damage` 0) reçoit un numéro, un numéro inconnu (le fichier a été perdu) retrouve un sac
    /// vide. `None` si les 65 535 numéros sont pris.
    pub fn number_for(&mut self, stack: &mut ItemStack, tier: usize) -> Option<u32> {
        let before = self.dirty;
        if stack.damage == 0 {
            self.next = self.next.max(1);
            while self.bags.contains_key(&self.next) {
                self.next += 1;
            }
            if self.next > MAX_NUMBER {
                return None;
            }
            stack.damage = self.next;
            self.next += 1;
        }
        let number = stack.damage & MAX_NUMBER;
        let known = self.bags.get(&number).map(|b| usize::from(b.tier));
        if known != Some(tier) {
            self.bags
                .entry(number)
                .or_insert_with(|| Bag::new(tier))
                .set_tier(tier);
            self.dirty = true;
        } else {
            self.dirty = before;
        }
        Some(number)
    }

    /// Y a-t-il quelque chose à écrire ?
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Écrit `backpacks.bin` dans le dossier du monde (le fichier n'existe qu'une fois un sac
    /// créé).
    pub fn save(&mut self, dir: &Path) -> Result<()> {
        if self.bags.is_empty() {
            return Ok(());
        }
        let saved = SavedStore {
            version: 1,
            next: self.next,
            bags: self.bags.iter().map(|(n, b)| (*n, b.clone())).collect(),
        };
        let bytes = postcard::to_allocvec(&saved).context("encodage des sacs à dos")?;
        engine_voxel::write_atomic(&dir.join(FILE), &bytes)?;
        self.dirty = false;
        Ok(())
    }

    /// Relit `backpacks.bin` (un magasin vide s'il n'existe pas). Un fichier illisible est mis de
    /// côté (`backpacks.bin.illisible`), jamais écrasé.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match postcard::from_bytes::<SavedStore>(&bytes) {
            Ok(saved) => {
                log::info!("{} sac(s) à dos chargé(s)", saved.bags.len());
                Self {
                    bags: saved.bags.into_iter().collect(),
                    next: saved.next,
                    dirty: false,
                    timer: 0.0,
                }
            }
            Err(err) => {
                log::error!("{} illisible ({err}) : mis de côté", path.display());
                let _ = std::fs::copy(&path, dir.join("backpacks.bin.illisible"));
                Self::default()
            }
        }
    }
}

/// Un sac porté : la case de l'inventaire, son numéro et son palier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carried {
    pub slot: usize,
    pub number: u32,
    pub tier: usize,
}

/// Les sacs numérotés que le joueur porte (barre ou réserve), dans l'ordre des cases. Le magasin
/// est mis à jour : le palier de l'objet fait foi.
pub fn carried(
    inventory: &Inventory,
    items: &ItemRegistry,
    store: &mut BackpackStore,
) -> Vec<Carried> {
    let mut out = Vec::new();
    for (slot, stack) in inventory.slots.iter().enumerate() {
        let Some(stack) = stack.filter(|s| s.damage != 0) else {
            continue;
        };
        let Some(tier) = tier_of(&items.get(stack.item).key) else {
            continue;
        };
        let mut copy = stack;
        if let Some(number) = store.number_for(&mut copy, tier) {
            out.push(Carried { slot, number, tier });
        }
    }
    out
}

/// Les améliorations actives d'un sac.
pub fn active(bag: &Bag) -> Vec<Upgrade> {
    installed_upgrades(&bag.slots, bag.layout(), bag.disabled)
}

/// Ramassage : range d'abord `stack` dans les sacs portés qui ont l'amélioration (et dont le filtre
/// laisse passer l'objet). Renvoie ce qui n'est pas entré.
pub fn pickup(
    store: &mut BackpackStore,
    bags: &[Carried],
    items: &ItemRegistry,
    stack: ItemStack,
) -> u32 {
    let key = &items.get(stack.item).key;
    if is_backpack(key) {
        return stack.count;
    }
    let mut left = stack.count;
    for c in bags {
        if left == 0 {
            break;
        }
        let Some(bag) = store.bag(c.number) else {
            continue;
        };
        let layout = bag.layout();
        if !active(bag).contains(&Upgrade::Pickup) {
            continue;
        }
        let filter = filter_items(&bag.slots, layout, PICKUP_FILTER);
        if !filter.is_empty() && !filter.contains(key) {
            continue;
        }
        let Some(bag) = store.bag_mut(c.number) else {
            continue;
        };
        let policy = Policy::of(&bag.slots, layout, bag.disabled);
        left -= storage_insert(
            &mut bag.slots,
            layout,
            &policy,
            Some(items),
            key,
            stack.damage,
            left,
            Fill::Any,
        );
    }
    left
}

/// Réapprovisionnement : chaque pile de la barre sous le quart de sa taille est complétée depuis
/// les sacs portés qui ont l'amélioration. Renvoie `true` si une pile a changé.
pub fn restock(
    inventory: &mut Inventory,
    store: &mut BackpackStore,
    bags: &[Carried],
    items: &ItemRegistry,
) -> bool {
    let mut changed = false;
    for hot in 0..HOTBAR {
        let Some(stack) = inventory.slots[hot] else {
            continue;
        };
        let max = items.get(stack.item).max_stack;
        if max <= 1 || stack.damage != 0 || stack.count * 4 >= max {
            continue;
        }
        let key = items.get(stack.item).key.clone();
        let mut need = max - stack.count;
        for c in bags {
            if need == 0 {
                break;
            }
            let Some(bag) = store.bag(c.number) else {
                continue;
            };
            let layout = bag.layout();
            if !active(bag).contains(&Upgrade::Restock)
                || !bag.slots[layout.main_range()]
                    .iter()
                    .flatten()
                    .any(|s| s.item == key && s.damage == 0)
            {
                continue;
            }
            let Some(bag) = store.bag_mut(c.number) else {
                continue;
            };
            for i in layout.main_range() {
                let Some(Some(s)) = bag.slots.get_mut(i) else {
                    continue;
                };
                if s.item != key || s.damage != 0 || need == 0 {
                    continue;
                }
                let n = need.min(s.count);
                s.count -= n;
                need -= n;
                if s.count == 0 {
                    bag.slots[i] = None;
                }
            }
        }
        let moved = max - stack.count - need;
        if moved > 0 {
            if let Some(slot) = inventory.slots[hot].as_mut() {
                slot.count += moved;
            }
            changed = true;
        }
    }
    changed
}

/// Nourriture : la case du sac dont l'aliment nourrit le plus sans rien gaspiller quand il manque
/// `missing` points de faim ; `None` s'il n'y a rien d'adapté.
pub fn feed_choice(
    bag: &Bag,
    items: &ItemRegistry,
    missing: f32,
) -> Option<(usize, ItemId, engine_voxel::FoodDef)> {
    let mut best: Option<(usize, ItemId, engine_voxel::FoodDef)> = None;
    for i in bag.layout().main_range() {
        let Some(Some(s)) = bag.slots.get(i) else {
            continue;
        };
        let Some(id) = items.id(&s.item) else {
            continue;
        };
        let Some(food) = items.get(id).food else {
            continue;
        };
        // Les potions sont des aliments sans faim : ce n'est pas un repas.
        if food.nutrition <= 0.0 || food.nutrition > missing {
            continue;
        }
        if best.is_none_or(|(_, _, b)| food.nutrition > b.nutrition) {
            best = Some((i, id, food));
        }
    }
    best
}

/// Le passage des améliorations passives (compactage, néant avec filtre) des sacs portés.
pub fn maintain_carried(
    store: &mut BackpackStore,
    bags: &[Carried],
    items: &ItemRegistry,
    table: &CompactTable,
) {
    for c in bags {
        let was_dirty = store.dirty;
        if let Some(bag) = store.bag_mut(c.number) {
            let layout = bag.layout();
            let disabled = bag.disabled;
            if !maintain(&mut bag.slots, layout, disabled, Some(items), table) {
                store.dirty = was_dirty;
            }
        }
    }
}

impl VoxelGame {
    /// Ramasse un objet : les sacs d'abord (Ramassage), puis l'inventaire. Renvoie ce qui reste.
    pub(crate) fn collect_stack(&mut self, stack: ItemStack) -> u32 {
        let items = self.content.items.clone();
        let bags = carried(
            &self.players.me_mut().inventory,
            &items,
            &mut self.backpacks,
        );
        let left = if bags.is_empty() {
            stack.count
        } else {
            pickup(&mut self.backpacks, &bags, &items, stack)
        };
        if left == 0 {
            return 0;
        }
        self.players.me_mut().inventory.add_stack(
            ItemStack {
                count: left,
                ..stack
            },
            &items,
        )
    }

    /// Les améliorations des sacs portés : l'aimant à chaque pas, le reste deux fois par seconde.
    pub(crate) fn update_backpacks(&mut self, dt: f32) {
        let items = self.content.items.clone();
        let bags = carried(
            &self.players.me_mut().inventory,
            &items,
            &mut self.backpacks,
        );
        let magnet = bags.iter().any(|c| {
            self.backpacks
                .bag(c.number)
                .is_some_and(|b| active(b).contains(&Upgrade::Magnet))
        });
        self.attract_drops(magnet);
        if bags.is_empty() {
            return;
        }
        self.backpacks.timer += dt;
        if self.backpacks.timer < PERIOD {
            return;
        }
        self.backpacks.timer = 0.0;
        let table = self.content.compact.clone();
        maintain_carried(&mut self.backpacks, &bags, &items, &table);
        restock(
            &mut self.players.me_mut().inventory,
            &mut self.backpacks,
            &bags,
            &items,
        );
        self.bags_feed(&bags);
    }

    /// Un aliment d'un sac, mangé si la faim a assez baissé pour ne rien perdre.
    fn bags_feed(&mut self, bags: &[Carried]) {
        let missing = self.hunger_rules.max_food - self.players.me_mut().hunger.food;
        if missing <= 0.0 {
            return;
        }
        let items = self.content.items.clone();
        for c in bags {
            let Some(bag) = self.backpacks.bag(c.number) else {
                continue;
            };
            if !active(bag).contains(&Upgrade::Feed) {
                continue;
            }
            let Some((slot, _, food)) = feed_choice(bag, &items, missing) else {
                continue;
            };
            if let Some(bag) = self.backpacks.bag_mut(c.number)
                && let Some(Some(stack)) = bag.slots.get_mut(slot)
            {
                stack.count -= 1;
                if stack.count == 0 {
                    bag.slots[slot] = None;
                }
            }
            self.players
                .me_mut()
                .hunger
                .eat(&self.hunger_rules, food.nutrition, food.saturation);
            self.sound("core:place_soft");
            return;
        }
    }

    /// L'aimant : les objets au sol proches marchent (et sautent) vers le joueur. La physique des
    /// objets ramène sinon leur vitesse à zéro à chaque pas : on leur donne une intention de
    /// marche, comme à une créature. Sans aimant (`on` faux), on la leur retire.
    fn attract_drops(&mut self, on: bool) {
        let feet = crate::player::body(&self.ecs, self.players.entity()).position + Vec3::Y * 0.6;
        let mut pulls = Vec::new();
        self.ecs
            .query::<(&crate::components::Transform, &crate::drops::DroppedItem)>()
            .for_each(|entity, (t, drop)| {
                let to = feet - t.position;
                let flat = Vec3::new(to.x, 0.0, to.z);
                let wanted =
                    on && drop.age > 0.6 && to.length() < MAGNET_RADIUS && flat.length() > 0.15;
                pulls.push((entity, wanted.then(|| flat.normalize())));
            });
        for (entity, direction) in pulls {
            if let Some(mut intent) = self.ecs.get_mut::<crate::components::MoveIntent>(entity) {
                intent.walk = direction.map_or(Vec3::ZERO, |d| d * MAGNET_SPEED);
                intent.jump = direction.is_some();
            }
        }
    }

    /// Le sac en position `slot` de l'inventaire est ouvert : son numéro (assigné s'il est neuf).
    pub(crate) fn bag_number_at(&mut self, slot: usize) -> Option<u32> {
        let mut stack = self.players.me_mut().inventory.slots[slot]?;
        let tier = tier_of(&self.content.items.get(stack.item).key)?;
        let number = self.backpacks.number_for(&mut stack, tier)?;
        self.players.me_mut().inventory.slots[slot] = Some(stack);
        Some(number)
    }

    /// Ouvre le premier sac de l'inventaire (barre d'abord).
    pub(crate) fn open_first_bag(&mut self) -> bool {
        let slot = (0..self.players.me_mut().inventory.slots.len()).find(|&i| {
            self.players.me_mut().inventory.slots[i]
                .is_some_and(|s| is_backpack(&self.content.items.get(s.item).key))
        });
        slot.is_some_and(|slot| self.open_bag_at(slot))
    }

    /// Ouvre le sac de la case `slot`.
    pub(crate) fn open_bag_at(&mut self, slot: usize) -> bool {
        let Some(number) = self.bag_number_at(slot) else {
            self.toast("Plus aucun numéro de sac disponible");
            return false;
        };
        self.open_container_screen(crate::ui::Screen::Bag(number));
        true
    }

    /// Sauvegarde le magasin des sacs dans la racine du monde (si quelque chose a changé).
    pub(crate) fn save_backpacks(&mut self) {
        if !self.backpacks.dirty() {
            return;
        }
        let Some(root) = self.dims.meta_store.clone().or_else(|| self.store.clone()) else {
            return;
        };
        if let Err(err) = self.backpacks.save(root.dir()) {
            log::error!("sauvegarde des sacs à dos : {err:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> ItemRegistry {
        let blocks = engine_voxel::BlockRegistry::builtin();
        ItemRegistry::builtin(&blocks).unwrap()
    }

    fn bag_stack(items: &ItemRegistry, tier: usize, number: u32) -> ItemStack {
        ItemStack {
            item: items.id(BAG_TIERS[tier].key).unwrap(),
            count: 1,
            damage: number,
        }
    }

    fn put(bag: &mut Bag, index: usize, key: &str, count: u32) {
        bag.slots[index] = Some(engine_voxel::ItemStack {
            item: key.into(),
            count,
            damage: 0,
        });
    }

    fn install(bag: &mut Bag, upgrade: Upgrade) {
        let layout = bag.layout();
        let slot = layout
            .upgrade_range()
            .find(|&i| bag.slots[i].is_none())
            .unwrap();
        put(bag, slot, upgrade.key(), 1);
    }

    #[test]
    fn a_new_bag_gets_a_number_once_and_keeps_it() {
        let items = items();
        let mut store = BackpackStore::default();
        let mut stack = bag_stack(&items, 0, 0);
        let first = store.number_for(&mut stack, 0).unwrap();
        assert_eq!((first, stack.damage), (1, 1));
        // Le sac revient (jeté, rangé dans un coffre...) : même numéro, même contenu.
        put(store.bag_mut(1).unwrap(), 0, "core:stone", 5);
        assert_eq!(store.number_for(&mut stack, 0), Some(1));
        assert_eq!(store.bag(1).unwrap().slots[0].as_ref().unwrap().count, 5);
        let mut other = bag_stack(&items, 0, 0);
        assert_eq!(store.number_for(&mut other, 0), Some(2));
    }

    #[test]
    fn bag_numbers_never_look_like_an_enchantment_or_wear_past_the_limit() {
        let items = items();
        let mut store = BackpackStore {
            next: MAX_NUMBER,
            ..Default::default()
        };
        let mut last = bag_stack(&items, 0, 0);
        assert_eq!(store.number_for(&mut last, 0), Some(MAX_NUMBER));
        assert!(!crate::enchanting::is_enchanted(last.damage));
        assert!(
            !crate::item_data::is_extended(last.damage),
            "jamais le bit des fiches"
        );
        let mut one_more = bag_stack(&items, 0, 0);
        assert_eq!(
            store.number_for(&mut one_more, 0),
            None,
            "plus de numéro libre"
        );
        assert_eq!(one_more.damage, 0);
    }

    #[test]
    fn bags_are_not_tools_so_wear_and_enchanting_never_touch_them() {
        let items = items();
        for tier in BAG_TIERS {
            let def = items.get(items.id(tier.key).unwrap());
            assert!(def.durability.is_none() && def.tool.is_none() && def.armor.is_none());
            assert_eq!(def.max_stack, 1);
            assert!(
                crate::enchanting::Enchantment::all()
                    .into_iter()
                    .all(|e| !e.applies_to(def)),
                "un sac ne s'enchante pas"
            );
        }
        // L'usure d'une pile de sac ne change rien, même numérotée.
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(bag_stack(&items, 0, 12));
        assert!(!inventory.wear_selected(&items, 0.0));
        assert_eq!(inventory.slots[0].unwrap().damage, 12);
    }

    #[test]
    fn the_tier_follows_the_item_and_keeps_the_contents() {
        let items = items();
        let mut store = BackpackStore::default();
        let mut stack = bag_stack(&items, 0, 0);
        store.number_for(&mut stack, 0).unwrap();
        put(store.bag_mut(1).unwrap(), 26, "core:stone", 3);
        install_upgrade(&mut store, 1, Upgrade::Compact);
        // Le sac devient un sac de fer : même numéro, cases agrandies.
        let mut iron = ItemStack {
            item: items.id(BAG_TIERS[2].key).unwrap(),
            ..stack
        };
        store.number_for(&mut iron, 2).unwrap();
        let bag = store.bag(1).unwrap();
        assert_eq!(bag.slots.len(), BAG_TIERS[2].layout.total());
        assert_eq!(bag.slots[26].as_ref().unwrap().count, 3);
        assert_eq!(
            bag.slots[BAG_TIERS[2].layout.main].as_ref().unwrap().item,
            Upgrade::Compact.key()
        );
    }

    fn install_upgrade(store: &mut BackpackStore, number: u32, upgrade: Upgrade) {
        install(store.bag_mut(number).unwrap(), upgrade);
    }

    #[test]
    fn a_crafted_upgrade_inherits_the_number_of_the_consumed_bag() {
        let items = items();
        let old = bag_stack(&items, 0, 42);
        let copper = items.id(BAG_TIERS[1].key).unwrap();
        let kit = items.id("core:copper_kit").unwrap();
        let consumed = [ItemStack::new(kit, 1), old];
        assert_eq!(inherited_number(consumed.into_iter(), copper, &items), 42);
        let stone = items.id("core:stone").unwrap();
        assert_eq!(inherited_number([old].into_iter(), stone, &items), 0);
        // Par le livre de recettes comme par la grille.
        let mut inventory = Inventory::default();
        inventory.slots[0] = Some(old);
        inventory.slots[1] = Some(ItemStack::new(kit, 1));
        let recipe = engine_voxel::Recipes::builtin(&items)
            .unwrap()
            .iter()
            .find(|r| r.result().0 == copper)
            .cloned()
            .unwrap();
        assert!(inventory.craft_from_book(&recipe, &items).is_some());
        let made = inventory
            .slots
            .iter()
            .flatten()
            .find(|s| s.item == copper)
            .unwrap();
        assert_eq!(made.damage, 42);
    }

    #[test]
    fn a_bag_never_goes_into_another_bag() {
        let items = items();
        let bag = items.id(BAG_TIERS[0].key).unwrap();
        let stone = items.id("core:stone").unwrap();
        assert!(!accepts(&items, bag));
        assert!(accepts(&items, stone));
        // Le ramassage refuse aussi un sac, même sans filtre.
        let mut store = BackpackStore::default();
        let mut carried_stack = bag_stack(&items, 0, 0);
        store.number_for(&mut carried_stack, 0).unwrap();
        install_upgrade(&mut store, 1, Upgrade::Pickup);
        let bags = [Carried {
            slot: 0,
            number: 1,
            tier: 0,
        }];
        let thrown = bag_stack(&items, 1, 9);
        assert_eq!(pickup(&mut store, &bags, &items, thrown), 1);
    }

    #[test]
    fn pickup_goes_to_the_bag_first_and_respects_the_filter() {
        let items = items();
        let mut store = BackpackStore::default();
        let mut stack = bag_stack(&items, 1, 0);
        store.number_for(&mut stack, 1).unwrap();
        install_upgrade(&mut store, 1, Upgrade::Pickup);
        let bags = [Carried {
            slot: 0,
            number: 1,
            tier: 1,
        }];
        let stone = items.id("core:stone").unwrap();
        // Sans filtre : tout entre.
        assert_eq!(
            pickup(&mut store, &bags, &items, ItemStack::new(stone, 10)),
            0
        );
        assert_eq!(store.bag(1).unwrap().slots[0].as_ref().unwrap().count, 10);
        // Avec un filtre sur la terre : la pierre n'entre plus, la terre oui.
        let layout = store.bag(1).unwrap().layout();
        put(
            store.bag_mut(1).unwrap(),
            layout.filter_range(PICKUP_FILTER).start,
            "core:dirt",
            1,
        );
        assert_eq!(
            pickup(&mut store, &bags, &items, ItemStack::new(stone, 10)),
            10
        );
        let dirt = items.id("core:dirt").unwrap();
        assert_eq!(
            pickup(&mut store, &bags, &items, ItemStack::new(dirt, 5)),
            0
        );
        // L'amélioration coupée : plus rien n'entre.
        store.bag_mut(1).unwrap().disabled = 1;
        assert_eq!(
            pickup(&mut store, &bags, &items, ItemStack::new(dirt, 5)),
            5
        );
    }

    #[test]
    fn restock_tops_up_a_hotbar_stack_that_fell_under_a_quarter() {
        let items = items();
        let stone = items.id("core:stone").unwrap();
        let mut store = BackpackStore::default();
        let mut stack = bag_stack(&items, 0, 0);
        store.number_for(&mut stack, 0).unwrap();
        install_upgrade(&mut store, 1, Upgrade::Restock);
        put(store.bag_mut(1).unwrap(), 2, "core:stone", 100);
        let bags = [Carried {
            slot: 9,
            number: 1,
            tier: 0,
        }];
        let mut inventory = Inventory::default();
        inventory.slots[3] = Some(ItemStack::new(stone, 20));
        inventory.slots[4] = Some(ItemStack::new(stone, 4));
        assert!(restock(&mut inventory, &mut store, &bags, &items));
        assert_eq!(
            inventory.slots[3].unwrap().count,
            20,
            "au-dessus du quart : intacte"
        );
        assert_eq!(inventory.slots[4].unwrap().count, 64);
        assert_eq!(store.bag(1).unwrap().slots[2].as_ref().unwrap().count, 40);
        // Plus rien dans le sac : la pile reste telle quelle.
        inventory.slots[4] = Some(ItemStack::new(stone, 2));
        store.bag_mut(1).unwrap().slots[2] = None;
        assert!(!restock(&mut inventory, &mut store, &bags, &items));
    }

    #[test]
    fn feeding_picks_the_best_meal_without_waste() {
        let items = items();
        let mut bag = Bag::new(0);
        put(&mut bag, 0, "core:bread", 2); // 5 points
        put(&mut bag, 1, "core:carrot", 2); // 3 points
        put(&mut bag, 2, "core:potion_healing", 1); // pas un repas
        let (slot, id, food) = feed_choice(&bag, &items, 6.0).unwrap();
        assert_eq!(
            (slot, items.get(id).key.as_str(), food.nutrition),
            (0, "core:bread", 5.0)
        );
        let (slot, _, _) = feed_choice(&bag, &items, 4.0).unwrap();
        assert_eq!(slot, 1, "le pain gaspillerait 1 point : la carotte");
        assert!(
            feed_choice(&bag, &items, 2.0).is_none(),
            "rien d'assez petit"
        );
    }

    #[test]
    fn the_store_survives_a_save_and_a_reload() {
        let dir = std::env::temp_dir().join(format!("voxel-bags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let items = items();
        let mut store = BackpackStore::default();
        let mut stack = bag_stack(&items, 1, 0);
        store.number_for(&mut stack, 1).unwrap();
        put(store.bag_mut(1).unwrap(), 5, "core:iron_ingot", 33);
        install_upgrade(&mut store, 1, Upgrade::Magnet);
        store.bag_mut(1).unwrap().disabled = 0b10;
        let mut other = bag_stack(&items, 0, 0);
        store.number_for(&mut other, 0).unwrap();
        store.save(&dir).unwrap();
        assert!(!store.dirty());
        let loaded = BackpackStore::load(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.bag(1), store.bag(1));
        assert_eq!(loaded.bag(2), store.bag(2));
        let mut fresh = bag_stack(&items, 0, 0);
        let mut loaded = loaded;
        assert_eq!(
            loaded.number_for(&mut fresh, 0),
            Some(3),
            "le compteur continue"
        );
        // Un fichier abîmé est mis de côté, pas écrasé en silence.
        std::fs::write(dir.join(FILE), b"pas du postcard").unwrap();
        assert!(BackpackStore::load(&dir).is_empty());
        assert!(dir.join("backpacks.bin.illisible").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_number_travels_through_the_ground_a_chest_and_the_saved_inventory() {
        let items = items();
        let bag = bag_stack(&items, 1, 321);
        // Jeté au sol, puis ramassé : la pile est la même.
        let mut world = engine_ecs::World::new();
        let feet = Vec3::new(0.0, 10.0, 0.0);
        crate::drops::spawn(&mut world, bag, feet + Vec3::Y * 0.5, 7);
        let picked = crate::drops::update(&mut world, feet, 1.0);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].1, bag);
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add_stack(picked[0].1, &items), 0);
        // Rangé dans un coffre (cases du moteur, objets par identifiant), puis repris.
        let mut chest: Vec<Option<ItemStack>> = vec![None; 27];
        let all: Vec<usize> = (0..27).collect();
        let main: Vec<usize> = (9..36).collect();
        crate::transfer::move_all(
            &mut inventory.slots,
            &(0..36).collect::<Vec<_>>(),
            &mut chest,
            &all,
            &crate::transfer::Rules::plain(&items),
            false,
        );
        assert!(inventory.slots.iter().all(Option::is_none));
        let stored: Vec<Option<engine_voxel::ItemStack>> = chest
            .iter()
            .map(|s| s.map(|s| crate::storage_ui::to_engine(&items, s)))
            .collect();
        assert_eq!(stored.iter().flatten().next().unwrap().damage, 321);
        let mut back: Vec<Option<ItemStack>> = stored
            .iter()
            .map(|s| {
                s.as_ref()
                    .and_then(|s| crate::storage_ui::to_game(&items, s))
            })
            .collect();
        crate::transfer::move_all(
            &mut back,
            &all,
            &mut inventory.slots,
            &main,
            &crate::transfer::Rules::plain(&items),
            false,
        );
        // Sauvegardé avec l'inventaire du joueur, puis relu.
        let saved = inventory.to_save(&items);
        let loaded = Inventory::from_save(&saved, &items);
        let found = loaded
            .slots
            .iter()
            .flatten()
            .find(|s| s.item == bag.item)
            .unwrap();
        assert_eq!(found.damage, 321, "le numéro a fait tout le voyage");
    }

    #[test]
    fn carried_lists_numbered_bags_in_the_whole_inventory() {
        let items = items();
        let mut store = BackpackStore::default();
        let mut inventory = Inventory::default();
        inventory.slots[2] = Some(bag_stack(&items, 0, 0)); // neuf : sans numéro, ignoré
        inventory.slots[20] = Some(bag_stack(&items, 1, 7));
        let list = carried(&inventory, &items, &mut store);
        assert_eq!(
            list,
            vec![Carried {
                slot: 20,
                number: 7,
                tier: 1
            }]
        );
        assert_eq!(store.bag(7).unwrap().layout(), BAG_TIERS[1].layout);
    }
}
