//! Le stockage à paliers : coffres qu'on améliore sur place, emplacements d'amélioration,
//! contrôleur qui réunit les coffres voisins.
//!
//! # La disposition des cases
//!
//! Un coffre à paliers garde tout dans l'inventaire générique de son entité de bloc
//! ([`crate::Inventory`]), sans champ de plus (le format des sauvegardes ne change pas). Ses
//! cases sont rangées en trois blocs, dans cet ordre ([`Layout`]) :
//!
//! 1. les cases **principales** (`main`), les seules que voient les tuyaux d'objets ;
//! 2. les **emplacements d'amélioration** (`upgrades`), une amélioration par case ;
//! 3. les cases **filtre** (`filters`) : des cases fantômes, où l'on pose un exemplaire d'un objet
//!    pour le désigner. Rien n'est consommé, et elles ne lâchent jamais rien quand le bloc casse.
//!    Elles sont par groupes de [`FILTER_SLOTS`] : le groupe 0 sert au Néant, le groupe 1 (sacs
//!    seulement) au Ramassage.
//!
//! Monter de palier ne change donc que la position des blocs 2 et 3 ([`relayout`]) : les cases
//! principales gardent leurs numéros, un coffre posé avant cette fonction (27 cases) se complète
//! tout seul ([`ensure_layout`]).
//!
//! # Les améliorations
//!
//! Une amélioration est un objet rangé dans un emplacement ([`Upgrade`]). Celles d'un coffre :
//! - **Pile ×2**, **Pile ×4** : la taille maximale des piles est multipliée (jamais plus de ×8) ;
//! - **Compactage** : 9 objets identiques deviennent le bloc que la recette 3 × 3 fabrique avec
//!   eux ([`CompactTable`], lue dans les recettes d'artisanat, jamais écrite à la main) ;
//! - **Néant** : détruit les objets du filtre qui entrent ; sans filtre, détruit ce qui ne
//!   rentre plus quand le conteneur est plein ;
//! - **Entonnoir** : chaque seconde, prend une pile dans le conteneur du dessus et en pousse une
//!   dans celui du dessous, en respectant les ports ([`StorageChest`]).
//!
//! Les sacs à dos (côté jeu) en ajoutent d'autres, qui agissent sur le joueur : Ramassage,
//! Aimant, Réapprovisionnement, Nourriture.
//!
//! # Le contrôleur
//!
//! [`controller_members`] retrouve les coffres reliés au contrôleur par contact (64 au plus, dans
//! un rayon de 16 blocs). Les fonctions `controller_*` les voient comme un seul inventaire : les
//! objets regroupés par sorte, un dépôt qui complète d'abord les piles de tous les coffres.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::Arc;

use glam::IVec3;

use crate::capabilities::{Inventory, ItemStack, MAX_STACK};
use crate::crafting::{Ingredient, Recipe, Recipes};
use crate::grid::{BlockAccess, GridPos};
use crate::items::ItemRegistry;
use crate::machines::{BlockBehavior, MachineContext};
use crate::registry::BlockRegistry;

/// Cases d'un groupe de filtre.
pub const FILTER_SLOTS: usize = 4;
/// Le multiplicateur de piles ne dépasse jamais cette valeur.
pub const MULTIPLIER_CAP: u32 = 8;
/// Le contrôleur réunit au plus ce nombre de conteneurs...
pub const CONTROLLER_MAX_CONTAINERS: usize = 64;
/// ... dans ce rayon (en blocs, autour de lui).
pub const CONTROLLER_RADIUS: i32 = 16;
/// Ticks entre deux passages d'un coffre amélioré (compactage, néant, entonnoir) : une seconde.
pub const UPGRADE_PERIOD: u32 = 20;
/// Le bloc du contrôleur (son comportement porte le même nom).
pub const CONTROLLER_KEY: &str = "core:storage_controller";

/// Le rangement des cases d'un conteneur à paliers (voir la documentation du module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub main: usize,
    pub upgrades: usize,
    pub filters: usize,
}

/// Ce que contient une case d'après sa position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Main,
    Upgrade,
    /// Une case filtre, de ce groupe.
    Filter(usize),
}

impl Layout {
    pub const fn new(main: usize, upgrades: usize, filters: usize) -> Self {
        Self {
            main,
            upgrades,
            filters,
        }
    }

    pub fn total(self) -> usize {
        self.main + self.upgrades + self.filters
    }

    pub fn main_range(self) -> Range<usize> {
        0..self.main
    }

    pub fn upgrade_range(self) -> Range<usize> {
        self.main..self.main + self.upgrades
    }

    /// Les cases d'un groupe de filtre (vide si ce groupe n'existe pas).
    pub fn filter_range(self, group: usize) -> Range<usize> {
        let first = self.main + self.upgrades + group * FILTER_SLOTS;
        let end = (self.main + self.upgrades + self.filters).min(first + FILTER_SLOTS);
        first.min(end)..end
    }

    pub fn filter_groups(self) -> usize {
        self.filters.div_ceil(FILTER_SLOTS)
    }

    pub fn kind_of(self, index: usize) -> Option<SlotKind> {
        if index < self.main {
            Some(SlotKind::Main)
        } else if index < self.main + self.upgrades {
            Some(SlotKind::Upgrade)
        } else if index < self.total() {
            Some(SlotKind::Filter(
                (index - self.main - self.upgrades) / FILTER_SLOTS,
            ))
        } else {
            None
        }
    }
}

/// Un palier de coffre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChestTier {
    /// Le bloc (et son comportement, du même nom).
    pub key: &'static str,
    pub name: &'static str,
    pub layout: Layout,
}

/// Du coffre de base au coffre de saphir.
pub const CHEST_TIERS: [ChestTier; 5] = [
    ChestTier {
        key: "core:chest",
        name: "Coffre",
        layout: Layout::new(27, 1, FILTER_SLOTS),
    },
    ChestTier {
        key: "core:copper_chest",
        name: "Coffre de cuivre",
        layout: Layout::new(45, 2, FILTER_SLOTS),
    },
    ChestTier {
        key: "core:iron_chest",
        name: "Coffre de fer",
        layout: Layout::new(54, 3, FILTER_SLOTS),
    },
    ChestTier {
        key: "core:gold_chest",
        name: "Coffre d'or",
        layout: Layout::new(81, 4, FILTER_SLOTS),
    },
    ChestTier {
        key: "core:sapphire_chest",
        name: "Coffre de saphir",
        layout: Layout::new(108, 5, FILTER_SLOTS),
    },
];

/// Le numéro de palier du bloc ou du comportement `key` (0 : le coffre de base).
pub fn chest_tier_of(key: &str) -> Option<usize> {
    CHEST_TIERS.iter().position(|t| t.key == key)
}

/// La disposition des cases d'un coffre à paliers, d'après son comportement.
pub fn chest_layout(behavior: &str) -> Option<Layout> {
    chest_tier_of(behavior).map(|t| CHEST_TIERS[t].layout)
}

/// La disposition des cases du bloc `block`, s'il est un coffre à paliers.
pub fn layout_of(blocks: &BlockRegistry, block: crate::BlockId) -> Option<Layout> {
    blocks.def(block).behavior.as_deref().and_then(chest_layout)
}

/// Le bloc est-il le contrôleur de stockage ?
pub fn is_controller(blocks: &BlockRegistry, block: crate::BlockId) -> bool {
    blocks.def(block).behavior.as_deref() == Some(CONTROLLER_KEY)
}

/// Un kit de montée de palier (un objet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kit {
    pub key: &'static str,
    /// Le palier atteint.
    pub to: usize,
    /// Un kit direct monte n'importe quel palier inférieur ; un kit simple, celui juste en dessous.
    pub direct: bool,
}

pub const KITS: [Kit; 7] = [
    Kit {
        key: "core:copper_kit",
        to: 1,
        direct: false,
    },
    Kit {
        key: "core:iron_kit",
        to: 2,
        direct: false,
    },
    Kit {
        key: "core:gold_kit",
        to: 3,
        direct: false,
    },
    Kit {
        key: "core:sapphire_kit",
        to: 4,
        direct: false,
    },
    Kit {
        key: "core:iron_direct_kit",
        to: 2,
        direct: true,
    },
    Kit {
        key: "core:gold_direct_kit",
        to: 3,
        direct: true,
    },
    Kit {
        key: "core:sapphire_direct_kit",
        to: 4,
        direct: true,
    },
];

pub fn kit_of(key: &str) -> Option<Kit> {
    KITS.iter().find(|k| k.key == key).copied()
}

/// Le palier qu'atteint un coffre de palier `current` avec ce kit, s'il convient.
pub fn kit_target(key: &str, current: usize) -> Option<usize> {
    let kit = kit_of(key)?;
    let fits = if kit.direct {
        current < kit.to
    } else {
        current + 1 == kit.to
    };
    fits.then_some(kit.to)
}

/// Monte le coffre en `pos` au palier `to` : le bloc change, son état actif garde tous ses objets
/// (les cases principales ne bougent pas) et s'agrandit. `false` si ce n'est pas un coffre à
/// paliers ou si `to` n'est pas un palier au-dessus.
pub fn upgrade_chest(grid: &mut (impl BlockAccess + ?Sized), pos: IVec3, to: usize) -> bool {
    let registry = grid.registry().clone();
    let Some(from) = registry
        .def(grid.block(pos))
        .behavior
        .as_deref()
        .and_then(chest_tier_of)
    else {
        return false;
    };
    let Some(new_block) = CHEST_TIERS.get(to).and_then(|t| registry.id(t.key)) else {
        return false;
    };
    if to <= from {
        return false;
    }
    let facing = registry.def(grid.block(pos)).facing_turns();
    let new_block = registry.variant(new_block, &[facing]);
    let mut entity = grid.block_entity(pos).cloned().unwrap_or_default();
    let inventory = entity
        .caps_mut()
        .inventory
        .get_or_insert_with(|| Inventory::new(CHEST_TIERS[from].layout.total()));
    inventory.slots = relayout(
        std::mem::take(&mut inventory.slots),
        CHEST_TIERS[from].layout,
        CHEST_TIERS[to].layout,
    );
    // Changer le bloc efface son état actif : on le remet aussitôt.
    grid.set_block(pos, new_block);
    grid.set_block_entity(pos, Some(entity))
}

/// Une amélioration, rangée dans un emplacement d'amélioration sous forme d'objet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Upgrade {
    Stack2,
    Stack4,
    Compact,
    Void,
    Funnel,
    Pickup,
    Magnet,
    Restock,
    Feed,
}

impl Upgrade {
    pub const ALL: [Upgrade; 9] = [
        Upgrade::Stack2,
        Upgrade::Stack4,
        Upgrade::Compact,
        Upgrade::Void,
        Upgrade::Funnel,
        Upgrade::Pickup,
        Upgrade::Magnet,
        Upgrade::Restock,
        Upgrade::Feed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Stack2 => "core:upgrade_stack_2",
            Self::Stack4 => "core:upgrade_stack_4",
            Self::Compact => "core:upgrade_compact",
            Self::Void => "core:upgrade_void",
            Self::Funnel => "core:upgrade_funnel",
            Self::Pickup => "core:upgrade_pickup",
            Self::Magnet => "core:upgrade_magnet",
            Self::Restock => "core:upgrade_restock",
            Self::Feed => "core:upgrade_feed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|u| u.key() == key)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Stack2 => "Pile ×2",
            Self::Stack4 => "Pile ×4",
            Self::Compact => "Compactage",
            Self::Void => "Néant",
            Self::Funnel => "Entonnoir",
            Self::Pickup => "Ramassage",
            Self::Magnet => "Aimant",
            Self::Restock => "Réapprovisionnement",
            Self::Feed => "Nourriture",
        }
    }

    /// Ce qu'elle fait, en une phrase (fiche de l'objet).
    pub fn help(self) -> &'static str {
        match self {
            Self::Stack2 => "Les piles du conteneur sont deux fois plus grandes.",
            Self::Stack4 => "Les piles du conteneur sont quatre fois plus grandes (×8 au plus).",
            Self::Compact => {
                "Neuf objets identiques deviennent le bloc que la recette 3 × 3 en fait."
            }
            Self::Void => {
                "Détruit les objets du filtre qui entrent ; sans filtre, ce qui ne rentre plus."
            }
            Self::Funnel => {
                "Chaque seconde : une pile du conteneur au-dessus, une vers celui du dessous."
            }
            Self::Pickup => "Un objet ramassé va d'abord dans le sac (filtre facultatif).",
            Self::Magnet => "Attire les objets au sol dans un rayon de 6 blocs.",
            Self::Restock => "Complète une pile de la barre qui passe sous un quart.",
            Self::Feed => "Mange un aliment du sac quand la faim a assez baissé.",
        }
    }

    /// Marche dans un coffre à paliers.
    pub fn in_chest(self) -> bool {
        matches!(
            self,
            Self::Stack2 | Self::Stack4 | Self::Compact | Self::Void | Self::Funnel
        )
    }

    /// Marche dans un sac à dos.
    pub fn in_backpack(self) -> bool {
        self != Self::Funnel
    }

    /// Le groupe de cases filtre qu'elle lit, s'il y en a un.
    pub fn filter_group(self) -> Option<usize> {
        match self {
            Self::Void => Some(0),
            Self::Pickup => Some(1),
            _ => None,
        }
    }

    fn multiplier(self) -> u32 {
        match self {
            Self::Stack2 => 2,
            Self::Stack4 => 4,
            _ => 1,
        }
    }
}

/// Les améliorations posées dans les emplacements, sauf celles que `disabled` (un bit par
/// emplacement) a coupées.
pub fn installed(slots: &[Option<ItemStack>], layout: Layout, disabled: u32) -> Vec<Upgrade> {
    layout
        .upgrade_range()
        .enumerate()
        .filter(|(n, _)| disabled & (1 << n) == 0)
        .filter_map(|(_, i)| slots.get(i)?.as_ref())
        .filter_map(|s| Upgrade::from_key(&s.item))
        .collect()
}

/// Le multiplicateur de taille des piles (1 sans amélioration, 8 au plus).
pub fn stack_multiplier(upgrades: &[Upgrade]) -> u32 {
    upgrades
        .iter()
        .map(|u| u.multiplier())
        .product::<u32>()
        .min(MULTIPLIER_CAP)
}

/// Les objets désignés par un groupe de cases filtre.
pub fn filter_items(slots: &[Option<ItemStack>], layout: Layout, group: usize) -> Vec<String> {
    layout
        .filter_range(group)
        .filter_map(|i| slots.get(i)?.as_ref().map(|s| s.item.clone()))
        .collect()
}

/// Ce que le Néant détruit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Voiding {
    /// Pas de Néant.
    None,
    /// Les objets du filtre.
    Filter(Vec<String>),
    /// Sans filtre : ce qui ne rentre plus.
    Overflow,
}

/// Les règles qu'imposent les améliorations à un conteneur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub mult: u32,
    pub void: Voiding,
}

impl Policy {
    pub const PLAIN: Policy = Policy {
        mult: 1,
        void: Voiding::None,
    };

    pub fn of(slots: &[Option<ItemStack>], layout: Layout, disabled: u32) -> Self {
        let ups = installed(slots, layout, disabled);
        let void = if ups.contains(&Upgrade::Void) {
            let filter = filter_items(slots, layout, 0);
            if filter.is_empty() {
                Voiding::Overflow
            } else {
                Voiding::Filter(filter)
            }
        } else {
            Voiding::None
        };
        Self {
            mult: stack_multiplier(&ups),
            void,
        }
    }

    /// Le Néant détruit-il cet objet dès qu'il entre ?
    pub fn voids(&self, item: &str) -> bool {
        matches!(&self.void, Voiding::Filter(list) if list.iter().any(|i| i == item))
    }
}

/// La taille maximale d'une pile de `item` : celle de l'objet fois `mult` (ce qui est usé ou
/// enchanté ne s'empile pas).
pub fn stack_limit(items: Option<&ItemRegistry>, item: &str, damage: u32, mult: u32) -> u32 {
    let base = items
        .and_then(|r| r.id(item).map(|id| r.get(id).max_stack))
        .unwrap_or(MAX_STACK)
        .max(1);
    if damage != 0 { base } else { base * mult }
}

/// Comment ranger : sur les piles du même objet seulement, ou aussi dans les cases vides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Any,
    PilesOnly,
}

/// Combien d'objets de plus ces cases accepteraient (au plus `limit` par pile).
pub fn room_in(
    slots: &[Option<ItemStack>],
    indices: &[usize],
    item: &str,
    damage: u32,
    limit: u32,
) -> u32 {
    let mut room = 0;
    for &i in indices {
        match slots.get(i) {
            Some(Some(s)) if damage == 0 && s.damage == 0 && s.item == item => {
                room += limit.saturating_sub(s.count);
            }
            Some(None) => room += limit,
            _ => {}
        }
    }
    room
}

/// Range jusqu'à `count` objets dans ces cases (les piles du même objet d'abord) ; renvoie ce qui
/// est rangé.
pub fn put_in(
    slots: &mut [Option<ItemStack>],
    indices: &[usize],
    item: &str,
    damage: u32,
    count: u32,
    limit: u32,
    fill: Fill,
) -> u32 {
    let mut left = count;
    if damage == 0 {
        for &i in indices {
            if left == 0 {
                break;
            }
            if let Some(Some(s)) = slots.get_mut(i)
                && s.damage == 0
                && s.item == item
            {
                let n = left.min(limit.saturating_sub(s.count));
                s.count += n;
                left -= n;
            }
        }
    }
    if fill == Fill::Any {
        for &i in indices {
            if left == 0 {
                break;
            }
            if let Some(slot @ None) = slots.get_mut(i) {
                let n = left.min(limit.max(1));
                *slot = Some(ItemStack {
                    item: item.to_string(),
                    count: n,
                    damage,
                });
                left -= n;
            }
        }
    }
    count - left
}

/// Combien d'objets ce conteneur accepte (ce qu'il range, plus ce que le Néant détruit).
pub fn accept(
    slots: &[Option<ItemStack>],
    layout: Layout,
    policy: &Policy,
    items: Option<&ItemRegistry>,
    item: &str,
    damage: u32,
    count: u32,
) -> u32 {
    if policy.voids(item) || policy.void == Voiding::Overflow {
        return count;
    }
    let indices: Vec<usize> = layout.main_range().collect();
    let limit = stack_limit(items, item, damage, policy.mult);
    room_in(slots, &indices, item, damage, limit).min(count)
}

/// Range `count` objets : renvoie combien sont pris (rangés ou détruits par le Néant).
#[allow(clippy::too_many_arguments)]
pub fn insert(
    slots: &mut [Option<ItemStack>],
    layout: Layout,
    policy: &Policy,
    items: Option<&ItemRegistry>,
    item: &str,
    damage: u32,
    count: u32,
    fill: Fill,
) -> u32 {
    if policy.voids(item) {
        return count;
    }
    let indices: Vec<usize> = layout.main_range().collect();
    let limit = stack_limit(items, item, damage, policy.mult);
    let stored = put_in(slots, &indices, item, damage, count, limit, fill);
    if policy.void == Voiding::Overflow {
        count
    } else {
        stored
    }
}

/// Complète un conteneur ancien (27 cases avant les paliers) ou neuf : les cases manquantes sont
/// ajoutées, rien ne bouge.
pub fn ensure_layout(slots: &mut Vec<Option<ItemStack>>, layout: Layout) {
    if slots.len() >= layout.total() {
        return;
    }
    let old = Layout::new(slots.len().min(layout.main), 0, 0);
    let moved = relayout(std::mem::take(slots), old, layout);
    *slots = moved;
}

/// Les mêmes objets, rangés selon une autre disposition (montée de palier) : chaque bloc de
/// cases garde ses objets dans le même ordre.
pub fn relayout(slots: Vec<Option<ItemStack>>, old: Layout, new: Layout) -> Vec<Option<ItemStack>> {
    let mut out: Vec<Option<ItemStack>> = vec![None; new.total()];
    let mut copy = |from: Range<usize>, to: usize, count: usize| {
        for n in 0..count.min(from.len()) {
            out[to + n] = slots.get(from.start + n).cloned().flatten();
        }
    };
    copy(old.main_range(), 0, new.main);
    copy(old.upgrade_range(), new.main, new.upgrades);
    for group in 0..old.filter_groups().min(new.filter_groups()) {
        copy(
            old.filter_range(group),
            new.main + new.upgrades + group * FILTER_SLOTS,
            FILTER_SLOTS,
        );
    }
    out
}

/// Ce que le compactage sait faire : pour chaque objet, celui qui le remplace par neuf.
///
/// Lu dans les recettes d'artisanat : une recette 3 × 3 dont les neuf cases sont le même objet
/// et qui donne un autre objet, **à condition qu'une recette redonne neuf exemplaires depuis le
/// résultat** (on ne compacte que ce qu'on peut défaire).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactTable {
    map: HashMap<String, String>,
}

impl CompactTable {
    pub fn from_recipes(recipes: &Recipes, items: &ItemRegistry) -> Self {
        let uniform = |cells: &[Option<Ingredient>]| match cells.first() {
            Some(Some(Ingredient::Item(id)))
                if cells.iter().all(|c| c.as_ref() == cells[0].as_ref()) =>
            {
                Some(*id)
            }
            _ => None,
        };
        let mut packs = Vec::new();
        let mut unpacks = Vec::new();
        for recipe in recipes.iter() {
            match recipe {
                Recipe::Shaped {
                    width: 3,
                    height: 3,
                    cells,
                    result: (out, 1),
                } => {
                    if let Some(from) = uniform(cells).filter(|from| from != out) {
                        packs.push((from, *out));
                    }
                }
                Recipe::Shapeless {
                    ingredients,
                    result: (out, 9),
                } if ingredients.len() == 1 => {
                    if let Ingredient::Item(from) = &ingredients[0] {
                        unpacks.push((*from, *out));
                    }
                }
                _ => {}
            }
        }
        let map = packs
            .into_iter()
            .filter(|(from, out)| unpacks.contains(&(*out, *from)))
            .map(|(from, out)| (items.get(from).key.clone(), items.get(out).key.clone()))
            .collect();
        Self { map }
    }

    pub fn result_of(&self, item: &str) -> Option<&str> {
        self.map.get(item).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Le passage des améliorations passives : le Néant à filtre vide les objets désignés, le
/// compactage remplace chaque lot de 9 objets identiques. Renvoie `true` si quelque chose a changé.
pub fn maintain(
    slots: &mut [Option<ItemStack>],
    layout: Layout,
    disabled: u32,
    items: Option<&ItemRegistry>,
    table: &CompactTable,
) -> bool {
    let policy = Policy::of(slots, layout, disabled);
    let ups = installed(slots, layout, disabled);
    let mut changed = false;
    if let Voiding::Filter(list) = &policy.void {
        for i in layout.main_range() {
            if slots
                .get(i)
                .and_then(Option::as_ref)
                .is_some_and(|s| list.contains(&s.item))
            {
                slots[i] = None;
                changed = true;
            }
        }
    }
    if ups.contains(&Upgrade::Compact) {
        changed |= compact(slots, layout, &policy, items, table);
    }
    changed
}

fn compact(
    slots: &mut [Option<ItemStack>],
    layout: Layout,
    policy: &Policy,
    items: Option<&ItemRegistry>,
    table: &CompactTable,
) -> bool {
    let main: Vec<usize> = layout.main_range().collect();
    let mut kinds: Vec<String> = main
        .iter()
        .filter_map(|&i| slots.get(i)?.as_ref())
        .filter(|s| s.damage == 0 && table.result_of(&s.item).is_some())
        .map(|s| s.item.clone())
        .collect();
    kinds.sort();
    kinds.dedup();
    let mut changed = false;
    for kind in kinds {
        let total: u32 = main
            .iter()
            .filter_map(|&i| slots.get(i)?.as_ref())
            .filter(|s| s.damage == 0 && s.item == kind)
            .map(|s| s.count)
            .sum();
        let packs = total / 9;
        let Some(result) = table.result_of(&kind) else {
            continue;
        };
        if packs == 0 {
            continue;
        }
        // Retire 9 par bloc en commençant par les dernières cases (les premières gardent leur
        // place), puis range les blocs ; si un bloc ne rentre pas, on remet le reste.
        let mut to_remove = packs * 9;
        for &i in main.iter().rev() {
            if to_remove == 0 {
                break;
            }
            if let Some(Some(s)) = slots.get_mut(i)
                && s.damage == 0
                && s.item == kind
            {
                let n = to_remove.min(s.count);
                s.count -= n;
                to_remove -= n;
                if s.count == 0 {
                    slots[i] = None;
                }
            }
        }
        let limit = stack_limit(items, result, 0, policy.mult);
        let stored = put_in(slots, &main, result, 0, packs, limit, Fill::Any);
        if stored < packs {
            let back = (packs - stored) * 9;
            let limit = stack_limit(items, &kind, 0, policy.mult);
            put_in(slots, &main, &kind, 0, back, limit, Fill::Any);
        }
        changed |= stored > 0;
    }
    changed
}

// ----- Le comportement des coffres à paliers et du contrôleur -----

/// Le comportement des coffres à paliers (`behavior: "core:copper_chest"`...) : crée et complète
/// l'inventaire, applique les améliorations passives et l'entonnoir tant qu'il y en a.
pub struct StorageChest {
    items: Arc<ItemRegistry>,
    table: CompactTable,
}

impl StorageChest {
    pub fn new(items: Arc<ItemRegistry>, recipes: &Recipes) -> Self {
        let table = CompactTable::from_recipes(recipes, &items);
        Self { items, table }
    }

    /// Complète l'inventaire du coffre en `at` ; renvoie sa disposition et ses améliorations.
    fn prepare(&self, ctx: &mut MachineContext, at: GridPos) -> Option<Vec<Upgrade>> {
        let registry = ctx.registry()?;
        let layout = layout_of(&registry, ctx.block(at))?;
        ctx.grids.write_or_create(at, |e| {
            let caps = e.caps_mut();
            let inv = caps
                .inventory
                .get_or_insert_with(|| Inventory::new(layout.total()));
            ensure_layout(&mut inv.slots, layout);
            installed(&inv.slots, layout, 0)
        })
    }

    fn tick(&self, ctx: &mut MachineContext, at: GridPos) -> bool {
        let Some(ups) = self.prepare(ctx, at) else {
            return false;
        };
        let Some(registry) = ctx.registry() else {
            return false;
        };
        let Some(layout) = layout_of(&registry, ctx.block(at)) else {
            return false;
        };
        let items = &self.items;
        let table = &self.table;
        ctx.grids.write(at, |e| {
            if let Some(inv) = e.caps.as_mut().and_then(|c| c.inventory.as_mut()) {
                maintain(&mut inv.slots, layout, 0, Some(items), table);
            }
        });
        if ups.contains(&Upgrade::Funnel) {
            self.funnel(ctx, &registry, at, layout);
        }
        ups.iter()
            .any(|u| matches!(u, Upgrade::Compact | Upgrade::Void | Upgrade::Funnel))
    }

    /// L'entonnoir : une pile du dessus vers ce coffre, une pile de ce coffre vers le dessous.
    fn funnel(
        &self,
        ctx: &mut MachineContext,
        registry: &BlockRegistry,
        at: GridPos,
        layout: Layout,
    ) {
        let above = at.offset(IVec3::Y);
        let below = at.offset(IVec3::NEG_Y);
        // Tirer.
        if let Some(give) = port_slots(registry, ctx.block(above), true)
            && let Some(stack) = ctx.grids.read(above, |e| first_stack(e, &give)).flatten()
        {
            let moved = ctx
                .grids
                .write(at, |e| {
                    let inv = e.caps.as_mut()?.inventory.as_mut()?;
                    let policy = Policy::of(&inv.slots, layout, 0);
                    Some(insert(
                        &mut inv.slots,
                        layout,
                        &policy,
                        Some(&self.items),
                        &stack.item,
                        stack.damage,
                        stack.count,
                        Fill::Any,
                    ))
                })
                .flatten()
                .unwrap_or(0);
            if moved > 0 {
                take_from(ctx, above, &give, &stack.item, stack.damage, moved);
            }
        }
        // Pousser.
        if let Some(take) = port_slots(registry, ctx.block(below), false) {
            let main: Vec<usize> = layout.main_range().collect();
            let stack = ctx.grids.read(at, |e| first_stack(e, &main)).flatten();
            if let Some(stack) = stack {
                let below_layout = layout_of(registry, ctx.block(below));
                let moved = ctx
                    .grids
                    .write(below, |e| {
                        let inv = e.caps.as_mut()?.inventory.as_mut()?;
                        Some(match below_layout {
                            Some(l) => {
                                let policy = Policy::of(&inv.slots, l, 0);
                                insert(
                                    &mut inv.slots,
                                    l,
                                    &policy,
                                    Some(&self.items),
                                    &stack.item,
                                    stack.damage,
                                    stack.count,
                                    Fill::Any,
                                )
                            }
                            None => put_in(
                                &mut inv.slots,
                                &take,
                                &stack.item,
                                stack.damage,
                                stack.count,
                                stack_limit(Some(&self.items), &stack.item, stack.damage, 1),
                                Fill::Any,
                            ),
                        })
                    })
                    .flatten()
                    .unwrap_or(0);
                if moved > 0 {
                    take_from(ctx, at, &main, &stack.item, stack.damage, moved);
                }
            }
        }
    }
}

/// Les cases qu'un conteneur offre à un entonnoir : celles de ses ports déclarés (sauf s'il
/// filtre par recette), ou toutes les cases principales d'un coffre ; `None` pour le reste.
fn port_slots(registry: &BlockRegistry, block: crate::BlockId, giving: bool) -> Option<Vec<usize>> {
    let def = registry.def(block);
    match &def.ports {
        Some(ports) if ports.accepts.is_none() && ports.filter.is_none() => Some(if giving {
            ports.outputs.clone()
        } else {
            ports.inputs.clone()
        }),
        Some(_) => None,
        None => def
            .behavior
            .as_deref()
            .and_then(chest_layout)
            .map(|l| l.main_range().collect()),
    }
}

fn first_stack(entity: &crate::BlockEntity, slots: &[usize]) -> Option<ItemStack> {
    let inv = entity.caps.as_ref()?.inventory.as_ref()?;
    slots
        .iter()
        .filter_map(|&i| inv.slots.get(i)?.clone())
        .next()
}

fn take_from(
    ctx: &mut MachineContext,
    at: GridPos,
    slots: &[usize],
    item: &str,
    damage: u32,
    mut count: u32,
) {
    ctx.grids.write(at, |e| {
        let Some(inv) = e.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
            return;
        };
        for &i in slots {
            if count == 0 {
                break;
            }
            if let Some(Some(s)) = inv.slots.get_mut(i)
                && s.item == item
                && s.damage == damage
            {
                let n = count.min(s.count);
                s.count -= n;
                count -= n;
                if s.count == 0 {
                    inv.slots[i] = None;
                }
            }
        }
    });
}

impl BlockBehavior for StorageChest {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            if let Some(ups) = self.prepare(ctx, at)
                && ups
                    .iter()
                    .any(|u| matches!(u, Upgrade::Compact | Upgrade::Void | Upgrade::Funnel))
            {
                ctx.schedule(at, 1);
            }
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            if self.tick(ctx, at) {
                ctx.schedule(at, UPGRADE_PERIOD);
            }
        }
    }
}

/// Le contrôleur : son état est un inventaire sans case, ce qui suffit pour que les tuyaux
/// d'objets s'y branchent (ils déposent alors dans les coffres reliés).
pub struct Controller;

impl BlockBehavior for Controller {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.grids.write_or_create(at, |e| {
                if e.caps.as_ref().is_none_or(|c| c.inventory.is_none()) {
                    e.caps_mut().inventory = Some(Inventory::new(0));
                }
            });
        }
    }
}

// ----- Le contrôleur : un inventaire fait de tous les coffres voisins -----

/// Les coffres reliés au contrôleur en `origin` : on part de lui et on suit les coffres qui se
/// touchent (par une face), dans l'ordre de découverte.
pub fn controller_members(grid: &(impl BlockAccess + ?Sized), origin: IVec3) -> Vec<IVec3> {
    let registry = grid.registry().clone();
    let mut seen: HashSet<IVec3> = HashSet::new();
    seen.insert(origin);
    let mut queue = VecDeque::from([origin]);
    let mut members = Vec::new();
    while let Some(at) = queue.pop_front() {
        for d in crate::machines::NEIGHBOURS {
            let next = at + d;
            let far = (next - origin).abs().max_element() > CONTROLLER_RADIUS;
            if far || !seen.insert(next) {
                continue;
            }
            if layout_of(&registry, grid.block(next)).is_none() {
                continue;
            }
            if members.len() >= CONTROLLER_MAX_CONTAINERS {
                return members;
            }
            members.push(next);
            queue.push_back(next);
        }
    }
    members
}

/// Une sorte d'objet vue par le contrôleur, avec son total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub item: String,
    pub damage: u32,
    pub count: u32,
}

/// Le contenu de tous ces coffres, regroupé par sorte, le plus abondant d'abord.
pub fn controller_groups(grid: &(impl BlockAccess + ?Sized), members: &[IVec3]) -> Vec<Group> {
    let registry = grid.registry().clone();
    let mut totals: HashMap<(String, u32), u32> = HashMap::new();
    for &at in members {
        let Some(layout) = layout_of(&registry, grid.block(at)) else {
            continue;
        };
        let Some(inv) = grid
            .block_entity(at)
            .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
        else {
            continue;
        };
        for i in layout.main_range() {
            if let Some(Some(s)) = inv.slots.get(i) {
                *totals.entry((s.item.clone(), s.damage)).or_default() += s.count;
            }
        }
    }
    let mut groups: Vec<Group> = totals
        .into_iter()
        .map(|((item, damage), count)| Group {
            item,
            damage,
            count,
        })
        .collect();
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.item.cmp(&b.item))
            .then_with(|| a.damage.cmp(&b.damage))
    });
    groups
}

/// Combien d'objets de cette sorte les coffres reliés accepteraient encore.
pub fn controller_room(
    grid: &(impl BlockAccess + ?Sized),
    members: &[IVec3],
    items: Option<&ItemRegistry>,
    item: &str,
    damage: u32,
) -> u32 {
    let registry = grid.registry().clone();
    let mut room = 0u32;
    for &at in members {
        let Some(layout) = layout_of(&registry, grid.block(at)) else {
            continue;
        };
        let Some(inv) = grid
            .block_entity(at)
            .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
        else {
            continue;
        };
        let policy = Policy::of(&inv.slots, layout, 0);
        let indices: Vec<usize> = layout.main_range().collect();
        let limit = stack_limit(items, item, damage, policy.mult);
        room = room.saturating_add(room_in(&inv.slots, &indices, item, damage, limit));
    }
    room
}

/// Dépose `count` objets dans les coffres reliés : d'abord on complète les piles de tous les
/// coffres, puis on remplit les cases vides. Renvoie ce qui est rangé.
pub fn controller_insert(
    grid: &mut (impl BlockAccess + ?Sized),
    members: &[IVec3],
    items: Option<&ItemRegistry>,
    item: &str,
    damage: u32,
    count: u32,
) -> u32 {
    let registry = grid.registry().clone();
    let mut left = count;
    for fill in [Fill::PilesOnly, Fill::Any] {
        for &at in members {
            if left == 0 {
                break;
            }
            let Some(layout) = layout_of(&registry, grid.block(at)) else {
                continue;
            };
            let Some(inv) = grid
                .block_entity_mut(at)
                .and_then(|e| e.caps.as_mut()?.inventory.as_mut())
            else {
                continue;
            };
            ensure_layout(&mut inv.slots, layout);
            let policy = Policy::of(&inv.slots, layout, 0);
            let indices: Vec<usize> = layout.main_range().collect();
            let limit = stack_limit(items, item, damage, policy.mult);
            left -= put_in(&mut inv.slots, &indices, item, damage, left, limit, fill);
        }
    }
    count - left
}

/// Retire jusqu'à `count` objets de cette sorte des coffres reliés ; renvoie ce qui est retiré.
pub fn controller_take(
    grid: &mut (impl BlockAccess + ?Sized),
    members: &[IVec3],
    item: &str,
    damage: u32,
    count: u32,
) -> u32 {
    let registry = grid.registry().clone();
    let mut left = count;
    for &at in members {
        if left == 0 {
            break;
        }
        let Some(layout) = layout_of(&registry, grid.block(at)) else {
            continue;
        };
        let Some(inv) = grid
            .block_entity_mut(at)
            .and_then(|e| e.caps.as_mut()?.inventory.as_mut())
        else {
            continue;
        };
        for i in layout.main_range() {
            if left == 0 {
                break;
            }
            if let Some(Some(s)) = inv.slots.get_mut(i)
                && s.item == item
                && s.damage == damage
            {
                let n = left.min(s.count);
                s.count -= n;
                left -= n;
                if s.count == 0 {
                    inv.slots[i] = None;
                }
            }
        }
    }
    count - left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockEntity;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
        })
    }

    fn registry() -> (Arc<BlockRegistry>, Arc<ItemRegistry>) {
        let blocks = BlockRegistry::builtin();
        let items = Arc::new(ItemRegistry::builtin(&blocks).unwrap());
        (blocks, items)
    }

    fn table(items: &ItemRegistry) -> CompactTable {
        CompactTable::from_recipes(&Recipes::builtin(items).unwrap(), items)
    }

    #[test]
    fn tiers_grow_and_keep_their_main_slots_in_place() {
        for pair in CHEST_TIERS.windows(2) {
            assert!(pair[1].layout.main > pair[0].layout.main);
            assert!(pair[1].layout.upgrades > pair[0].layout.upgrades);
        }
        let old = CHEST_TIERS[0].layout;
        let new = CHEST_TIERS[2].layout;
        let mut slots = vec![None; old.total()];
        slots[0] = stack("core:stone", 5);
        slots[26] = stack("core:dirt", 7);
        slots[old.main] = stack(Upgrade::Compact.key(), 1);
        slots[old.filter_range(0).start] = stack("core:coal", 1);
        let grown = relayout(slots, old, new);
        assert_eq!(grown.len(), new.total());
        assert_eq!(grown[0], stack("core:stone", 5));
        assert_eq!(grown[26], stack("core:dirt", 7));
        assert_eq!(grown[new.main], stack(Upgrade::Compact.key(), 1));
        assert_eq!(grown[new.filter_range(0).start], stack("core:coal", 1));
        assert_eq!(grown[27..new.main].iter().flatten().count(), 0);
    }

    #[test]
    fn upgrading_a_placed_chest_keeps_its_contents() {
        use crate::subgrid::SubGrid;
        let (blocks, _) = registry();
        let mut grid = SubGrid::new(crate::GridId(1), blocks.clone());
        let pos = IVec3::new(2, 2, 2);
        let mut slots = vec![None; CHEST_TIERS[0].layout.total()];
        slots[0] = stack("core:stone", 64);
        slots[26] = stack("core:coal", 12);
        slots[CHEST_TIERS[0].layout.main] = stack(Upgrade::Compact.key(), 1);
        let north = blocks.id("core:chest").unwrap();
        put_block(&mut grid, pos, blocks.variant(north, &[1]), slots);
        assert!(upgrade_chest(&mut grid, pos, 2));
        let iron = blocks.id("core:iron_chest").unwrap();
        assert_eq!(grid.block(pos), blocks.variant(iron, &[1]));
        let after = slots_of(&grid, pos);
        let layout = CHEST_TIERS[2].layout;
        assert_eq!(after.len(), layout.total(), "les cases s'agrandissent");
        assert_eq!(after[0], stack("core:stone", 64));
        assert_eq!(after[26], stack("core:coal", 12));
        assert_eq!(after[layout.main], stack(Upgrade::Compact.key(), 1));
        // On ne redescend pas, et un bloc qui n'est pas un coffre est refusé.
        assert!(!upgrade_chest(&mut grid, pos, 1));
        let stone_pos = IVec3::new(3, 3, 3);
        grid.set_block(stone_pos, blocks.id("core:stone").unwrap());
        assert!(!upgrade_chest(&mut grid, stone_pos, 1));
    }

    #[test]
    fn an_old_chest_of_27_slots_gets_its_upgrade_slots() {
        let mut slots = vec![None; 27];
        slots[3] = stack("core:stone", 2);
        ensure_layout(&mut slots, CHEST_TIERS[0].layout);
        assert_eq!(slots.len(), CHEST_TIERS[0].layout.total());
        assert_eq!(slots[3], stack("core:stone", 2));
    }

    #[test]
    fn kits_fit_the_tier_just_below_unless_direct() {
        assert_eq!(kit_target("core:copper_kit", 0), Some(1));
        assert_eq!(kit_target("core:copper_kit", 1), None);
        assert_eq!(kit_target("core:iron_kit", 0), None);
        assert_eq!(kit_target("core:iron_direct_kit", 0), Some(2));
        assert_eq!(kit_target("core:iron_direct_kit", 2), None);
        assert_eq!(kit_target("core:stone", 0), None);
    }

    #[test]
    fn stack_upgrades_multiply_the_stack_size_up_to_eight() {
        let layout = CHEST_TIERS[2].layout;
        let (_, items) = registry();
        let mut slots = vec![None; layout.total()];
        slots[layout.main] = stack(Upgrade::Stack2.key(), 1);
        let policy = Policy::of(&slots, layout, 0);
        assert_eq!(policy.mult, 2);
        assert_eq!(stack_limit(Some(&items), "core:stone", 0, policy.mult), 128);
        slots[layout.main + 1] = stack(Upgrade::Stack4.key(), 1);
        assert_eq!(Policy::of(&slots, layout, 0).mult, 8);
        slots[layout.main + 2] = stack(Upgrade::Stack4.key(), 1);
        assert_eq!(Policy::of(&slots, layout, 0).mult, 8, "jamais plus de ×8");
        // Une amélioration coupée ne compte plus.
        assert_eq!(Policy::of(&slots, layout, 0b111).mult, 1);
        // On range 300 pierres : 128 + 128 + 44.
        let policy = Policy::of(&slots, layout, 0b100);
        assert_eq!(policy.mult, 8);
        let mut plain = vec![None; layout.total()];
        plain[layout.main] = stack(Upgrade::Stack2.key(), 1);
        let p = Policy::of(&plain, layout, 0);
        let stored = insert(
            &mut plain,
            layout,
            &p,
            Some(&items),
            "core:stone",
            0,
            300,
            Fill::Any,
        );
        assert_eq!(stored, 300);
        assert_eq!(plain[0].as_ref().unwrap().count, 128);
        assert_eq!(plain[1].as_ref().unwrap().count, 128);
        assert_eq!(plain[2].as_ref().unwrap().count, 44);
    }

    #[test]
    fn compaction_comes_from_the_crafting_recipes() {
        let (_, items) = registry();
        let table = table(&items);
        assert_eq!(
            table.result_of("core:iron_ingot"),
            Some("core:iron_block"),
            "lu dans les recettes"
        );
        assert!(table.result_of("core:stone").is_none());
        let layout = CHEST_TIERS[1].layout;
        let mut slots = vec![None; layout.total()];
        slots[0] = stack("core:iron_ingot", 20);
        slots[1] = stack("core:iron_ingot", 5);
        // Sans l'amélioration, rien ne change.
        assert!(!maintain(&mut slots, layout, 0, Some(&items), &table));
        slots[layout.main] = stack(Upgrade::Compact.key(), 1);
        assert!(maintain(&mut slots, layout, 0, Some(&items), &table));
        let count = |item: &str| -> u32 {
            slots[..layout.main]
                .iter()
                .flatten()
                .filter(|s| s.item == item)
                .map(|s| s.count)
                .sum()
        };
        assert_eq!(count("core:iron_block"), 2);
        assert_eq!(count("core:iron_ingot"), 7, "25 - 18");
        // Couper l'amélioration l'arrête.
        slots[0] = stack("core:copper_ingot", 9);
        assert!(!maintain(&mut slots, layout, 1, Some(&items), &table));
    }

    #[test]
    fn void_with_a_filter_destroys_listed_items_and_without_only_the_overflow() {
        let (_, items) = registry();
        let table = table(&items);
        let layout = Layout::new(2, 1, FILTER_SLOTS);
        let mut slots = vec![None; layout.total()];
        slots[layout.main] = stack(Upgrade::Void.key(), 1);
        // Sans filtre : on range ce qui rentre, le reste est détruit (le conteneur est « plein »).
        let policy = Policy::of(&slots, layout, 0);
        assert_eq!(policy.void, Voiding::Overflow);
        let n = insert(
            &mut slots,
            layout,
            &policy,
            Some(&items),
            "core:stone",
            0,
            64 * 3,
            Fill::Any,
        );
        assert_eq!(n, 64 * 3, "le surplus est détruit");
        assert_eq!(slots[0].as_ref().unwrap().count, 64);
        assert_eq!(slots[1].as_ref().unwrap().count, 64);
        assert_eq!(
            accept(&slots, layout, &policy, Some(&items), "core:dirt", 0, 5),
            5
        );
        // Avec un filtre : seuls les objets du filtre disparaissent.
        slots[layout.filter_range(0).start] = stack("core:stone", 1);
        let policy = Policy::of(&slots, layout, 0);
        assert!(policy.voids("core:stone") && !policy.voids("core:dirt"));
        slots[1] = stack("core:dirt", 4);
        assert_eq!(
            insert(
                &mut slots,
                layout,
                &policy,
                Some(&items),
                "core:stone",
                0,
                10,
                Fill::Any
            ),
            10
        );
        assert_eq!(
            insert(
                &mut slots,
                layout,
                &policy,
                Some(&items),
                "core:dirt",
                0,
                100,
                Fill::Any
            ),
            60,
            "le reste de la pile de terre, pas de destruction"
        );
        // Le passage supprime ce que le filtre désigne et qui était déjà là.
        assert!(maintain(&mut slots, layout, 0, Some(&items), &table));
        assert!(slots[0].is_none(), "la pierre est partie");
    }

    fn put_block(
        grid: &mut impl BlockAccess,
        pos: IVec3,
        block: crate::BlockId,
        slots: Vec<Option<ItemStack>>,
    ) {
        grid.set_block(pos, block);
        let mut e = BlockEntity::default();
        e.caps_mut().inventory = Some(Inventory {
            slots,
            sides: crate::Sides::ALL,
        });
        grid.set_block_entity(pos, Some(e));
    }

    fn slots_of(grid: &impl BlockAccess, pos: IVec3) -> Vec<Option<ItemStack>> {
        grid.block_entity(pos)
            .unwrap()
            .caps
            .as_ref()
            .unwrap()
            .inventory
            .as_ref()
            .unwrap()
            .slots
            .clone()
    }

    #[test]
    fn a_funnel_chest_moves_one_stack_down_per_pass() {
        use crate::subgrid::SubGrids;
        use crate::{ChunkStreamer, Grids, Machines, StreamingConfig, TerrainGenerator};
        use std::cell::RefCell;
        use std::rc::Rc;
        let (blocks, items) = registry();
        let recipes = Recipes::builtin(&items).unwrap();
        let streamer = RefCell::new(ChunkStreamer::new(
            TerrainGenerator::new(7),
            StreamingConfig::default(),
        ));
        let mut subs = SubGrids::new(blocks.clone());
        let id = subs.create();
        let cell = subs.get(id).unwrap();
        let chest = blocks.id("core:chest").unwrap();
        let iron = blocks.id("core:iron_chest").unwrap();
        let (top, mid, low) = (
            IVec3::new(1, 3, 1),
            IVec3::new(1, 2, 1),
            IVec3::new(1, 1, 1),
        );
        let mut top_slots = vec![None; CHEST_TIERS[0].layout.total()];
        top_slots[0] = stack("core:stone", 10);
        let mut mid_slots = vec![None; CHEST_TIERS[2].layout.total()];
        mid_slots[CHEST_TIERS[2].layout.main] = stack(Upgrade::Funnel.key(), 1);
        put_block(&mut *cell.borrow_mut(), top, chest, top_slots);
        put_block(&mut *cell.borrow_mut(), mid, iron, mid_slots);
        put_block(
            &mut *cell.borrow_mut(),
            low,
            chest,
            vec![None; CHEST_TIERS[0].layout.total()],
        );
        let mut machines = Machines::new();
        machines.register(iron, Rc::new(StorageChest::new(items.clone(), &recipes)));
        let grids = Grids::with_subgrids(&streamer, &subs);
        let mut actions = Vec::new();
        machines.grid_blocks_loaded(grids, &[GridPos::new(id, mid)], &mut actions);
        for _ in 0..(UPGRADE_PERIOD * 4) {
            machines.run_grids_tick(grids, &mut actions);
        }
        let grid = cell.borrow();
        let count = |pos: IVec3| -> u32 {
            slots_of(&*grid, pos)[..CHEST_TIERS[2].layout.main.min(27)]
                .iter()
                .flatten()
                .filter(|st| st.item == "core:stone")
                .map(|st| st.count)
                .sum()
        };
        assert_eq!(count(top) + count(mid) + count(low), 10, "rien ne se perd");
        assert!(count(low) > 0, "des pierres sont arrivées en bas");
        assert_eq!(count(top), 0, "le dessus est vidé");
    }

    #[test]
    fn the_controller_joins_touching_chests_within_its_limits() {
        use crate::subgrid::SubGrid;
        let (blocks, items) = registry();
        let mut grid = SubGrid::new(crate::GridId(1), blocks.clone());
        let controller = blocks.id(CONTROLLER_KEY).unwrap();
        let chest = blocks.id("core:chest").unwrap();
        let empty = || vec![None; CHEST_TIERS[0].layout.total()];
        let origin = IVec3::new(0, 6, 0);
        put_block(&mut grid, origin, controller, Vec::new());
        // Une rangée de coffres qui se touchent, un coffre isolé.
        for x in 1..=3 {
            put_block(&mut grid, origin + IVec3::new(x, 0, 0), chest, empty());
        }
        put_block(&mut grid, origin + IVec3::new(0, 0, 5), chest, empty());
        let members = controller_members(&grid, origin);
        assert_eq!(members.len(), 3);
        assert!(!members.contains(&(origin + IVec3::new(0, 0, 5))));
        // Le deuxième coffre contient une pile entamée.
        grid.block_entity_mut(members[1])
            .unwrap()
            .caps_mut()
            .inventory
            .as_mut()
            .unwrap()
            .slots[5] = stack("core:stone", 60);
        let stored = controller_insert(&mut grid, &members, Some(&items), "core:stone", 0, 10);
        assert_eq!(stored, 10);
        assert_eq!(
            slots_of(&grid, members[1])[5].as_ref().unwrap().count,
            64,
            "on complète d'abord"
        );
        assert_eq!(
            slots_of(&grid, members[0])[0].as_ref().unwrap().count,
            6,
            "le reste va au premier"
        );
        assert_eq!(
            controller_groups(&grid, &members),
            vec![Group {
                item: "core:stone".into(),
                damage: 0,
                count: 70
            }]
        );
        assert_eq!(
            controller_take(&mut grid, &members, "core:stone", 0, 100),
            70
        );
        assert!(controller_groups(&grid, &members).is_empty());
    }

    #[test]
    fn controller_stops_at_sixty_four_containers() {
        use crate::subgrid::SubGrid;
        let (blocks, _) = registry();
        let mut grid = SubGrid::new(crate::GridId(1), blocks.clone());
        let controller = blocks.id(CONTROLLER_KEY).unwrap();
        let chest = blocks.id("core:chest").unwrap();
        let origin = IVec3::new(0, 6, 0);
        grid.set_block(origin, controller);
        for x in 1..=16 {
            for z in 0..=7 {
                grid.set_block(origin + IVec3::new(x, 0, z), chest);
            }
        }
        assert_eq!(
            controller_members(&grid, origin).len(),
            CONTROLLER_MAX_CONTAINERS
        );
    }
}
