//! Les transferts rapides entre l'inventaire du joueur et un conteneur, sans interface.
//!
//! Tout travaille sur des listes de cases (`[Option<ItemStack>]`) et des listes de numéros de
//! cases à toucher : l'écran (voir `storage_ui.rs`) choisit les cases, ce module décide où va chaque
//! objet. Quatre gestes :
//!
//! - **Déposer** ([`move_all`]) : chaque pile de la source va dans la destination. D'abord on
//!   complète les piles du même objet (et de même usure : un outil usé ne s'empile jamais) déjà
//!   présentes, partout ; ensuite seulement on remplit les cases vides. Ce qui ne rentre pas reste
//!   dans la source ;
//! - **Compléter** : comme Déposer, mais seulement pour les objets que la destination contient
//!   déjà (`only_present`) ;
//! - **Tout prendre** : le même geste, dans l'autre sens ;
//! - **Maj + clic** ([`move_slot`]) : une seule pile, avec la même règle ;
//! - **Trier** ([`sort`]) : regroupe les piles et les range par catégorie puis par nom.
//!
//! Chaque côté a ses [`Rules`] : la taille maximale des piles (multipliée par les améliorations
//! « pile » d'un coffre), ce qu'il refuse (un sac ne contient pas de sac) et ce que le Néant détruit.
//! Une pile trop grande pour l'autre côté passe en plusieurs fois, case après case.

use engine_voxel::{Category, ItemId, ItemRegistry};

use crate::inventory::ItemStack;

/// Ce que le Néant d'un conteneur détruit à l'entrée.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Voids {
    #[default]
    Nothing,
    /// Ces objets-là.
    Items(Vec<ItemId>),
    /// Tout ce qui ne rentre plus.
    Overflow,
}

/// Les règles d'un côté d'un transfert.
pub struct Rules<'a> {
    pub items: &'a ItemRegistry,
    /// Le multiplicateur de taille des piles de ce côté (1 : la taille de l'objet).
    pub mult: u32,
    /// Cet objet peut-il entrer ici ?
    pub accepts: &'a dyn Fn(ItemId) -> bool,
    pub voids: Voids,
}

impl<'a> Rules<'a> {
    /// Un côté sans amélioration ni refus : l'inventaire du joueur.
    pub fn plain(items: &'a ItemRegistry) -> Self {
        Self {
            items,
            mult: 1,
            accepts: &|_| true,
            voids: Voids::Nothing,
        }
    }

    fn base_max(&self, item: ItemId) -> u32 {
        if (item.0 as usize) < self.items.len() {
            self.items.get(item).max_stack.max(1)
        } else {
            1
        }
    }

    /// La taille maximale d'une pile de cette sorte ici.
    pub fn limit(&self, item: ItemId, damage: u32) -> u32 {
        let base = self.base_max(item);
        if damage == 0 { base * self.mult } else { base }
    }

    fn destroys(&self, item: ItemId) -> bool {
        matches!(&self.voids, Voids::Items(list) if list.contains(&item))
    }
}

fn same_pile(rules: &Rules, a: &ItemStack, b: &ItemStack) -> bool {
    crate::inventory::stackable(rules.items, a, b)
}

/// Range la pile `stack` dans ces cases ; renvoie combien d'objets sont rangés.
fn put(
    to: &mut [Option<ItemStack>],
    to_idx: &[usize],
    stack: ItemStack,
    rules: &Rules,
    fill_empty: bool,
) -> u32 {
    let limit = rules.limit(stack.item, stack.damage);
    let mut left = stack.count;
    for &i in to_idx {
        if left == 0 {
            break;
        }
        if let Some(Some(s)) = to.get_mut(i)
            && same_pile(rules, s, &stack)
        {
            let n = left.min(limit.saturating_sub(s.count));
            s.count += n;
            left -= n;
        }
    }
    if fill_empty {
        for &i in to_idx {
            if left == 0 {
                break;
            }
            if let Some(slot @ None) = to.get_mut(i) {
                let n = left.min(limit);
                *slot = Some(ItemStack { count: n, ..stack });
                left -= n;
            }
        }
    }
    stack.count - left
}

/// La destination contient-elle déjà cet objet ?
fn holds(to: &[Option<ItemStack>], to_idx: &[usize], item: ItemId) -> bool {
    to_idx.iter().any(|&i| {
        to.get(i)
            .and_then(Option::as_ref)
            .is_some_and(|s| s.item == item)
    })
}

/// Déplace la pile `from[at]` vers la destination ; renvoie le nombre d'objets partis (rangés ou
/// détruits). `fill_empty` : faux pour la première passe (compléter seulement).
fn move_one(
    from: &mut [Option<ItemStack>],
    at: usize,
    to: &mut [Option<ItemStack>],
    to_idx: &[usize],
    rules: &Rules,
    fill_empty: bool,
) -> u32 {
    let Some(stack) = from.get(at).copied().flatten() else {
        return 0;
    };
    if !(rules.accepts)(stack.item) {
        return 0;
    }
    if rules.destroys(stack.item) {
        from[at] = None;
        return stack.count;
    }
    let mut moved = put(to, to_idx, stack, rules, fill_empty);
    // Le Néant sans filtre détruit ce qui ne rentre plus (à la dernière passe).
    if fill_empty && rules.voids == Voids::Overflow {
        moved = stack.count;
    }
    if moved >= stack.count {
        from[at] = None;
    } else if let Some(s) = from[at].as_mut() {
        s.count = stack.count - moved;
    }
    moved
}

/// Maj + clic : envoie toute la pile `from[at]` de l'autre côté.
pub fn move_slot(
    from: &mut [Option<ItemStack>],
    at: usize,
    to: &mut [Option<ItemStack>],
    to_idx: &[usize],
    rules: &Rules,
) -> u32 {
    let first = move_one(from, at, to, to_idx, rules, false);
    first + move_one(from, at, to, to_idx, rules, true)
}

/// Déposer (ou Tout prendre) : chaque pile des cases `from_idx` va dans la destination. Avec
/// `only_present`, seulement les objets que la destination contient déjà (Compléter). Renvoie le
/// nombre d'objets partis.
pub fn move_all(
    from: &mut [Option<ItemStack>],
    from_idx: &[usize],
    to: &mut [Option<ItemStack>],
    to_idx: &[usize],
    rules: &Rules,
    only_present: bool,
) -> u32 {
    let mut moved = 0;
    // Les objets que la destination avait avant : « Compléter » ne crée jamais une nouvelle sorte.
    let present: Vec<ItemId> = from_idx
        .iter()
        .filter_map(|&i| from.get(i).copied().flatten())
        .map(|s| s.item)
        .filter(|&item| holds(to, to_idx, item))
        .collect();
    let wanted = |item: ItemId| !only_present || present.contains(&item);
    // Première passe : on complète les piles existantes, partout.
    for &i in from_idx {
        if from
            .get(i)
            .copied()
            .flatten()
            .is_some_and(|s| wanted(s.item))
        {
            moved += move_one(from, i, to, to_idx, rules, false);
        }
    }
    // Seconde passe : les cases vides.
    for &i in from_idx {
        if from
            .get(i)
            .copied()
            .flatten()
            .is_some_and(|s| wanted(s.item))
        {
            moved += move_one(from, i, to, to_idx, rules, true);
        }
    }
    moved
}

fn category_rank(category: Category) -> u8 {
    match category {
        Category::Block => 0,
        Category::Material => 1,
        Category::Tool => 2,
        Category::Armor => 3,
        Category::Food => 4,
    }
}

/// Trier : regroupe les piles des cases `idx` (les neuves du même objet fusionnent jusqu'à la
/// taille maximale), puis les range dans l'ordre des catégories, puis des noms. Les cases en
/// trop se vident.
pub fn sort(slots: &mut [Option<ItemStack>], idx: &[usize], rules: &Rules) {
    let mut stacks: Vec<ItemStack> = idx
        .iter()
        .filter_map(|&i| slots.get_mut(i).and_then(Option::take))
        .collect();
    // Regroupement : chaque objet neuf est refait en piles pleines.
    let mut merged: Vec<ItemStack> = Vec::new();
    stacks.sort_by_key(|s| (s.item, s.damage));
    for stack in stacks {
        let limit = rules.limit(stack.item, stack.damage);
        if let Some(last) = merged.last_mut()
            && same_pile(rules, last, &stack)
            && last.count < limit
        {
            let n = stack.count.min(limit - last.count);
            last.count += n;
            if stack.count > n {
                merged.push(ItemStack {
                    count: stack.count - n,
                    ..stack
                });
            }
        } else {
            merged.push(stack);
        }
    }
    let key = |s: &ItemStack| {
        let def = rules.items.get(s.item);
        (
            category_rank(def.category),
            def.name.to_lowercase(),
            s.item,
            std::cmp::Reverse(s.count),
            s.damage,
        )
    };
    merged.sort_by_key(key);
    for (n, &i) in idx.iter().enumerate() {
        if let Some(slot) = slots.get_mut(i) {
            *slot = merged.get(n).copied();
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

    fn id(items: &ItemRegistry, key: &str) -> ItemId {
        items.id(key).unwrap()
    }

    fn st(items: &ItemRegistry, key: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack::new(id(items, key), count))
    }

    fn worn(items: &ItemRegistry, key: &str, damage: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: id(items, key),
            count: 1,
            damage,
        })
    }

    fn total(slots: &[Option<ItemStack>], item: ItemId) -> u32 {
        slots
            .iter()
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count)
            .sum()
    }

    /// 36 cases de joueur : la barre est 0 à 8, la réserve 9 à 35.
    fn player() -> Vec<Option<ItemStack>> {
        vec![None; 36]
    }

    fn main_slots() -> Vec<usize> {
        (9..36).collect()
    }

    #[test]
    fn deposit_completes_existing_piles_before_using_empty_slots() {
        let items = items();
        let mut me = player();
        me[9] = st(&items, "core:stone", 40);
        me[10] = st(&items, "core:stone", 30);
        let mut chest = vec![None; 27];
        chest[3] = st(&items, "core:stone", 50);
        chest[7] = st(&items, "core:stone", 60);
        let idx: Vec<usize> = (0..27).collect();
        let rules = Rules::plain(&items);
        let moved = move_all(&mut me, &main_slots(), &mut chest, &idx, &rules, false);
        assert_eq!(moved, 70);
        // 50 + 14 = 64 et 60 + 4 = 64 d'abord, puis une nouvelle pile pour le reste (52).
        assert_eq!(chest[3].unwrap().count, 64);
        assert_eq!(chest[7].unwrap().count, 64);
        assert_eq!(chest[0].unwrap().count, 70 - 14 - 4);
        assert_eq!(total(&me, id(&items, "core:stone")), 0);
    }

    #[test]
    fn worn_tools_never_merge_and_unmatched_wear_goes_to_an_empty_slot() {
        let items = items();
        let mut me = player();
        me[9] = worn(&items, "core:iron_pickaxe", 5);
        let mut chest = vec![None; 4];
        chest[0] = worn(&items, "core:iron_pickaxe", 9);
        let idx: Vec<usize> = (0..4).collect();
        let rules = Rules::plain(&items);
        move_all(&mut me, &main_slots(), &mut chest, &idx, &rules, false);
        assert_eq!(chest[0].unwrap().damage, 9);
        assert_eq!(
            chest[1].unwrap().damage,
            5,
            "l'usure est gardée, pas de fusion"
        );
        assert!(me[9].is_none());
    }

    #[test]
    fn what_does_not_fit_stays_in_the_inventory() {
        let items = items();
        let mut me = player();
        me[9] = st(&items, "core:stone", 64);
        me[10] = st(&items, "core:dirt", 64);
        let mut chest = vec![None; 1];
        let rules = Rules::plain(&items);
        let moved = move_all(&mut me, &main_slots(), &mut chest, &[0], &rules, false);
        assert_eq!(moved, 64);
        assert_eq!(chest[0].unwrap().item, id(&items, "core:stone"));
        assert_eq!(
            me[10].unwrap().count,
            64,
            "la terre reste : le coffre est plein"
        );
    }

    #[test]
    fn the_hotbar_is_spared_by_deposit() {
        let items = items();
        let mut me = player();
        me[0] = st(&items, "core:stone", 10);
        me[8] = st(&items, "core:dirt", 10);
        me[9] = st(&items, "core:stone", 5);
        let mut chest = vec![None; 27];
        let idx: Vec<usize> = (0..27).collect();
        move_all(
            &mut me,
            &main_slots(),
            &mut chest,
            &idx,
            &Rules::plain(&items),
            false,
        );
        assert_eq!(me[0].unwrap().count, 10);
        assert_eq!(me[8].unwrap().count, 10);
        assert!(me[9].is_none());
        assert_eq!(total(&chest, id(&items, "core:stone")), 5);
    }

    #[test]
    fn top_up_only_moves_what_the_container_already_holds() {
        let items = items();
        let mut me = player();
        me[9] = st(&items, "core:stone", 10);
        me[10] = st(&items, "core:dirt", 10);
        me[11] = st(&items, "core:stone", 64);
        let mut chest = vec![None; 27];
        chest[5] = st(&items, "core:stone", 60);
        let idx: Vec<usize> = (0..27).collect();
        let moved = move_all(
            &mut me,
            &main_slots(),
            &mut chest,
            &idx,
            &Rules::plain(&items),
            true,
        );
        assert_eq!(moved, 10 + 64);
        assert_eq!(
            me[10].unwrap().count,
            10,
            "la terre n'est pas dans le coffre : elle reste"
        );
        assert_eq!(total(&chest, id(&items, "core:dirt")), 0);
        assert_eq!(total(&chest, id(&items, "core:stone")), 134);
    }

    #[test]
    fn take_all_goes_the_other_way_and_splits_big_stacks() {
        let items = items();
        let mut me = player();
        me[0] = st(&items, "core:stone", 10);
        let mut chest = vec![None; 27];
        // Une pile ×4 : 200 pierres dans une case.
        chest[0] = st(&items, "core:stone", 200);
        let idx: Vec<usize> = (0..27).collect();
        let all: Vec<usize> = (0..36).collect();
        let moved = move_all(
            &mut chest,
            &idx,
            &mut me,
            &all,
            &Rules::plain(&items),
            false,
        );
        assert_eq!(moved, 200);
        assert!(
            me.iter().flatten().all(|s| s.count <= 64),
            "jamais plus de 64 par case"
        );
        assert_eq!(total(&me, id(&items, "core:stone")), 210);
        assert!(chest[0].is_none());
        // Un inventaire plein : tout ne passe pas, et le reste reste dans le coffre.
        let mut full: Vec<Option<ItemStack>> = vec![st(&items, "core:dirt", 64); 36];
        chest[0] = st(&items, "core:stone", 200);
        let moved = move_all(
            &mut chest,
            &idx,
            &mut full,
            &all,
            &Rules::plain(&items),
            false,
        );
        assert_eq!(moved, 0);
        assert_eq!(chest[0].unwrap().count, 200);
    }

    #[test]
    fn shift_click_moves_one_whole_stack_following_the_stacking_rule() {
        let items = items();
        let mut me = player();
        me[3] = st(&items, "core:stone", 50);
        let mut chest = vec![None; 3];
        chest[1] = st(&items, "core:stone", 40);
        let rules = Rules::plain(&items);
        let moved = move_slot(&mut me, 3, &mut chest, &[0, 1, 2], &rules);
        assert_eq!(moved, 50);
        assert_eq!(chest[1].unwrap().count, 64);
        assert_eq!(chest[0].unwrap().count, 26);
        assert!(me[3].is_none());
    }

    #[test]
    fn stack_upgrades_raise_the_limit_of_the_receiving_side() {
        let items = items();
        let mut me = player();
        me[9] = st(&items, "core:stone", 64);
        me[10] = st(&items, "core:stone", 64);
        let mut chest = vec![None; 2];
        let rules = Rules {
            mult: 4,
            ..Rules::plain(&items)
        };
        move_all(&mut me, &main_slots(), &mut chest, &[0, 1], &rules, false);
        assert_eq!(chest[0].unwrap().count, 128, "une pile ×4 : 256 au plus");
        assert!(chest[1].is_none());
    }

    #[test]
    fn a_container_can_refuse_items() {
        let items = items();
        let pickaxe = id(&items, "core:iron_pickaxe");
        let mut me = player();
        me[9] = worn(&items, "core:iron_pickaxe", 0);
        me[10] = st(&items, "core:stone", 1);
        let mut chest = vec![None; 4];
        let refuse = move |item: ItemId| item != pickaxe;
        let rules = Rules {
            accepts: &refuse,
            ..Rules::plain(&items)
        };
        move_all(
            &mut me,
            &main_slots(),
            &mut chest,
            &[0, 1, 2, 3],
            &rules,
            false,
        );
        assert!(me[9].is_some(), "refusé : reste chez le joueur");
        assert!(me[10].is_none());
    }

    #[test]
    fn void_destroys_listed_items_or_the_overflow() {
        let items = items();
        let stone = id(&items, "core:stone");
        let mut me = player();
        me[9] = st(&items, "core:stone", 10);
        me[10] = st(&items, "core:dirt", 10);
        let mut chest = vec![None; 1];
        let rules = Rules {
            voids: Voids::Items(vec![stone]),
            ..Rules::plain(&items)
        };
        move_all(&mut me, &main_slots(), &mut chest, &[0], &rules, false);
        assert!(me[9].is_none() && chest[0].unwrap().item == id(&items, "core:dirt"));
        // Sans filtre : le surplus disparaît quand c'est plein.
        me[9] = st(&items, "core:stone", 10);
        let rules = Rules {
            voids: Voids::Overflow,
            ..Rules::plain(&items)
        };
        move_all(&mut me, &main_slots(), &mut chest, &[0], &rules, false);
        assert!(me[9].is_none(), "détruit : le coffre est plein");
        assert_eq!(chest[0].unwrap().item, id(&items, "core:dirt"));
    }

    #[test]
    fn sort_merges_piles_then_orders_by_category_and_name() {
        let items = items();
        let mut chest = vec![None; 10];
        chest[0] = st(&items, "core:stone", 30);
        chest[2] = st(&items, "core:iron_ingot", 5);
        chest[4] = st(&items, "core:stone", 50);
        chest[5] = worn(&items, "core:iron_pickaxe", 3);
        chest[8] = st(&items, "core:bread", 2);
        chest[9] = st(&items, "core:dirt", 7);
        let idx: Vec<usize> = (0..10).collect();
        sort(&mut chest, &idx, &Rules::plain(&items));
        let seen: Vec<(ItemId, u32)> = chest.iter().flatten().map(|s| (s.item, s.count)).collect();
        // Blocs (par nom : Pierre, Terre), puis matières, outils, nourriture.
        let names: Vec<&str> = seen
            .iter()
            .map(|(i, _)| items.get(*i).name.as_str())
            .collect();
        let first_stone = names.iter().position(|n| *n == "Pierre").unwrap();
        let dirt = names.iter().position(|n| *n == "Terre").unwrap();
        assert!(first_stone < dirt);
        assert_eq!(seen[first_stone], (id(&items, "core:stone"), 64));
        assert_eq!(
            seen[first_stone + 1],
            (id(&items, "core:stone"), 16),
            "30 + 50 = 64 + 16"
        );
        assert_eq!(chest.iter().flatten().count(), 6);
        assert!(
            chest[6..].iter().all(Option::is_none),
            "les cases en trop sont vides"
        );
        let ranks: Vec<u8> = seen
            .iter()
            .map(|(i, _)| category_rank(items.get(*i).category))
            .collect();
        assert!(
            ranks.windows(2).all(|w| w[0] <= w[1]),
            "les catégories se suivent"
        );
        assert_eq!(total(&chest, id(&items, "core:stone")), 80);
    }
}
