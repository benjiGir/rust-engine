//! Le grand coffre : deux coffres simples du même palier, posés côte à côte et tournés du même
//! côté, se réunissent en un seul meuble de deux blocs.
//!
//! - Chaque moitié est une variante du coffre (état `ChestPart`) : la **gauche** (vue de face)
//!   et la **droite**. Leurs boîtes s'étirent jusqu'à la jointure, la serrure est au milieu.
//! - **La moitié gauche garde tout** : un inventaire au double de cases principales et
//!   d'emplacements d'amélioration ([`doubled`]), les filtres d'un coffre simple. La moitié
//!   droite n'a pas d'inventaire : [`storage_pos`] renvoie vers la gauche (écran, tuyaux,
//!   contrôleur). Les améliorations agissent donc sur tout le grand coffre.
//! - Dans l'inventaire réuni, la première moitié des cases principales et des emplacements
//!   d'amélioration revient à la moitié gauche, la seconde à la droite : [`split`] les rend
//!   à chacune quand on casse l'une d'elles (la moitié cassée lâche les siennes, l'autre
//!   redevient un coffre simple avec les siennes et les filtres).
//!
//! Le format des sauvegardes ne change pas : un coffre d'avant (`core:chest[facing=east]`) est
//! lu comme un coffre seul.

use glam::IVec3;

use crate::grid::BlockAccess;
use crate::registry::{BlockBox, BlockRegistry, StateKind};
use crate::tech::capabilities::{Inventory, ItemStack};
use crate::tech::storage::{Layout, chest_layout, ensure_layout};
use crate::{BlockEntity, BlockId};

/// Ce qu'est un coffre à paliers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Single,
    /// Vue de face, la moitié de gauche (côté +X du modèle) : elle garde l'inventaire.
    Left,
    Right,
}

/// La forme d'une boîte du coffre pour la moitié `value` (1 : gauche, 2 : droite), dans le
/// repère du modèle (face avant au nord). Les boîtes qui touchent presque le bord de la jointure
/// s'y étirent ; une pièce centrale (la serrure) glisse contre la jointure, à demi-largeur,
/// pour que les deux moitiés en forment une seule.
pub(crate) fn half_box(b: BlockBox, value: u8) -> BlockBox {
    const NEAR: f32 = 2.0 / 16.0 + 1e-4;
    let mut out = b;
    let width = b.max.x - b.min.x;
    match value {
        // La jointure est du côté -X (la moitié droite est à -X).
        1 => {
            if b.min.x <= NEAR {
                out.min.x = 0.0;
            } else if b.max.x < 1.0 - NEAR {
                out.min.x = 0.0;
                out.max.x = width / 2.0;
            }
        }
        2 => {
            if b.max.x >= 1.0 - NEAR {
                out.max.x = 1.0;
            } else if b.min.x > NEAR {
                out.min.x = 1.0 - width / 2.0;
                out.max.x = 1.0;
            }
        }
        _ => {}
    }
    out
}

/// La disposition du grand coffre fait de deux coffres de disposition `single`.
pub fn doubled(single: Layout) -> Layout {
    Layout::new(2 * single.main, 2 * single.upgrades, single.filters)
}

/// La moitié que représente `block`, s'il est un coffre qui peut se réunir.
pub fn part(registry: &BlockRegistry, block: BlockId) -> Option<Part> {
    let def = registry.def(block);
    let index = def.states.iter().position(|s| *s == StateKind::ChestPart)?;
    Some(match def.state_values.get(index) {
        Some(1) => Part::Left,
        Some(2) => Part::Right,
        _ => Part::Single,
    })
}

/// Le côté +X du modèle, une fois le coffre tourné.
fn side(facing: u8) -> IVec3 {
    [IVec3::X, IVec3::Z, IVec3::NEG_X, IVec3::NEG_Z][usize::from(facing % 4)]
}

fn with_part(registry: &BlockRegistry, block: BlockId, part: Part) -> BlockId {
    let facing = registry.def(block).facing_turns();
    let value = match part {
        Part::Single => 0,
        Part::Left => 1,
        Part::Right => 2,
    };
    registry.variant(block, &[facing, value])
}

/// La position et le bloc exact attendus de l'autre moitié.
pub fn partner(registry: &BlockRegistry, pos: IVec3, block: BlockId) -> Option<(IVec3, BlockId)> {
    let side = side(registry.def(block).facing_turns());
    match part(registry, block)? {
        Part::Single => None,
        Part::Left => Some((pos - side, with_part(registry, block, Part::Right))),
        Part::Right => Some((pos + side, with_part(registry, block, Part::Left))),
    }
}

/// L'autre moitié, si elle est bien en place.
pub fn paired(grid: &(impl BlockAccess + ?Sized), pos: IVec3) -> Option<IVec3> {
    let (other, expected) = partner(grid.registry(), pos, grid.block(pos))?;
    (grid.block(other) == expected).then_some(other)
}

/// Où se trouve l'inventaire du coffre en `pos` : la moitié gauche pour une moitié droite.
pub fn storage_pos(grid: &(impl BlockAccess + ?Sized), pos: IVec3) -> IVec3 {
    match part(grid.registry(), grid.block(pos)) {
        Some(Part::Right) => paired(grid, pos).unwrap_or(pos),
        _ => pos,
    }
}

/// La disposition d'un coffre à paliers selon sa moitié : double à gauche, aucune à droite
/// (elle n'a pas d'inventaire).
pub(crate) fn layout(registry: &BlockRegistry, block: BlockId) -> Option<Layout> {
    let single = registry
        .def(block)
        .behavior
        .as_deref()
        .and_then(chest_layout)?;
    match part(registry, block) {
        Some(Part::Left) => Some(doubled(single)),
        Some(Part::Right) => None,
        _ => Some(single),
    }
}

/// Les cases de l'inventaire en `pos`, complétées à la disposition `layout`.
fn slots_of(
    grid: &(impl BlockAccess + ?Sized),
    pos: IVec3,
    layout: Layout,
) -> Vec<Option<ItemStack>> {
    let mut slots = grid
        .block_entity(pos)
        .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
        .map(|i| i.slots.clone())
        .unwrap_or_default();
    ensure_layout(&mut slots, layout);
    slots
}

/// Pose ces cases dans l'entité du bloc en `pos` (le reste de l'entité est gardé).
fn store(
    grid: &mut (impl BlockAccess + ?Sized),
    pos: IVec3,
    entity: Option<BlockEntity>,
    slots: Option<Vec<Option<ItemStack>>>,
) {
    let entity = match slots {
        Some(slots) => {
            let mut entity = entity.unwrap_or_default();
            let caps = entity.caps_mut();
            let inventory = caps.inventory.get_or_insert_with(|| Inventory::new(0));
            inventory.slots = slots;
            Some(entity)
        }
        None => entity.map(|mut e| {
            if let Some(caps) = e.caps.as_mut() {
                caps.inventory = None;
            }
            e
        }),
    };
    grid.set_block_entity(pos, entity);
}

/// L'inventaire réuni de deux coffres simples (disposition `single`) : les cases principales de
/// la gauche puis de la droite, leurs améliorations dans le même ordre, les filtres de la gauche
/// (ou de la droite si la gauche n'en a pas).
fn join(
    l: &[Option<ItemStack>],
    r: &[Option<ItemStack>],
    single: Layout,
) -> Vec<Option<ItemStack>> {
    let mut merged = Vec::with_capacity(doubled(single).total());
    merged.extend_from_slice(&l[single.main_range()]);
    merged.extend_from_slice(&r[single.main_range()]);
    merged.extend_from_slice(&l[single.upgrade_range()]);
    merged.extend_from_slice(&r[single.upgrade_range()]);
    let filters = single.main + single.upgrades..single.total();
    let source = if l[filters.clone()].iter().any(Option::is_some) {
        l
    } else {
        r
    };
    merged.extend_from_slice(&source[filters]);
    merged
}

/// La part de la moitié `n` (0 : gauche, 1 : droite) d'un inventaire réuni, en coffre simple ;
/// avec les filtres si `filters`.
fn own(
    slots: &[Option<ItemStack>],
    single: Layout,
    n: usize,
    filters: bool,
) -> Vec<Option<ItemStack>> {
    let mut out = Vec::with_capacity(single.total());
    let ups = 2 * single.main + n * single.upgrades;
    out.extend_from_slice(&slots[n * single.main..(n + 1) * single.main]);
    out.extend_from_slice(&slots[ups..ups + single.upgrades]);
    if filters {
        out.extend_from_slice(&slots[2 * (single.main + single.upgrades)..doubled(single).total()]);
    } else {
        out.extend(std::iter::repeat_n(None, single.filters));
    }
    out
}

/// Un grand coffre qui change de palier : chaque moitié garde ses propres objets.
pub(crate) fn relayout_double(
    slots: &[Option<ItemStack>],
    old: Layout,
    new: Layout,
) -> Vec<Option<ItemStack>> {
    let mut slots = slots.to_vec();
    ensure_layout(&mut slots, doubled(old));
    let half = |n: usize| crate::tech::storage::relayout(own(&slots, old, n, n == 0), old, new);
    join(&half(0), &half(1), new)
}

/// Le coffre simple qu'on vient de poser en `pos` rejoint un coffre simple voisin identique
/// (même palier, même orientation, à sa gauche ou à sa droite). Renvoie l'autre moitié.
pub fn merge(grid: &mut (impl BlockAccess + ?Sized), pos: IVec3) -> Option<IVec3> {
    let registry = grid.registry().clone();
    let block = grid.block(pos);
    if part(&registry, block)? != Part::Single {
        return None;
    }
    let side = side(registry.def(block).facing_turns());
    let (left, right) = if grid.block(pos + side) == block {
        (pos + side, pos)
    } else if grid.block(pos - side) == block {
        (pos, pos - side)
    } else {
        return None;
    };
    let single = layout(&registry, block)?;
    let merged = join(
        &slots_of(grid, left, single),
        &slots_of(grid, right, single),
        single,
    );
    let (left_entity, right_entity) = (
        grid.block_entity(left).cloned(),
        grid.block_entity(right).cloned(),
    );
    // Changer un bloc efface son entité : on la remet aussitôt.
    grid.set_block(left, with_part(&registry, block, Part::Left));
    grid.set_block(right, with_part(&registry, block, Part::Right));
    store(grid, left, left_entity, Some(merged));
    store(grid, right, right_entity, None);
    Some(if left == pos { right } else { left })
}

/// Le grand coffre dont `pos` est une moitié se sépare : `pos` reste avec ses propres objets
/// (pour qu'on les lâche en le cassant), l'autre moitié redevient un coffre simple avec les
/// siens et les filtres. Renvoie l'autre moitié. Une moitié orpheline (l'autre a disparu sans
/// passer par ici) redevient aussi un coffre simple.
pub fn split(grid: &mut (impl BlockAccess + ?Sized), pos: IVec3) -> Option<IVec3> {
    let registry = grid.registry().clone();
    let block = grid.block(pos);
    let this = part(&registry, block)?;
    if this == Part::Single {
        return None;
    }
    let single_block = with_part(&registry, block, Part::Single);
    let single = layout(&registry, single_block)?;
    let other = paired(grid, pos);
    let left = match (this, other) {
        (Part::Left, _) => pos,
        (_, Some(other)) => other,
        // Une moitié droite seule n'avait rien : elle redevient un coffre vide.
        (_, None) => {
            let entity = grid.block_entity(pos).cloned();
            grid.set_block(pos, single_block);
            grid.set_block_entity(pos, entity);
            return None;
        }
    };
    let right = if left == pos { other } else { Some(pos) };
    let slots = slots_of(grid, left, doubled(single));
    // Les filtres suivent la moitié qui reste.
    let own_left = own(&slots, single, 0, pos != left);
    let own_right = own(&slots, single, 1, pos == left);
    let left_entity = grid.block_entity(left).cloned();
    grid.set_block(left, single_block);
    store(grid, left, left_entity, Some(own_left));
    if let Some(right) = right {
        let right_entity = grid.block_entity(right).cloned();
        grid.set_block(right, single_block);
        store(grid, right, right_entity, Some(own_right));
    }
    other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::GridId;
    use crate::motion::subgrid::SubGrid;

    type World = SubGrid;

    fn new_world(registry: &std::sync::Arc<BlockRegistry>) -> World {
        SubGrid::new(GridId(1), registry.clone())
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.to_string(),
            count,
            damage: 0,
        })
    }

    fn place(world: &mut World, pos: IVec3, key: &str, facing: u8) -> BlockId {
        let registry = world.registry().clone();
        let block = registry.variant(registry.id(key).unwrap(), &[facing, 0]);
        world.set_block(pos, block);
        block
    }

    fn put(world: &mut World, pos: IVec3, slots: Vec<Option<ItemStack>>) {
        let mut entity = BlockEntity::default();
        entity.caps_mut().inventory = Some(Inventory {
            slots,
            ..Inventory::new(0)
        });
        world.set_block_entity(pos, Some(entity));
    }

    fn slots(world: &World, pos: IVec3) -> Vec<Option<ItemStack>> {
        world
            .block_entity(pos)
            .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
            .map(|i| i.slots.clone())
            .unwrap_or_default()
    }

    #[test]
    fn halves_stretch_to_the_joint_and_share_the_latch() {
        let registry = BlockRegistry::builtin();
        let chest = registry.id("core:chest").unwrap();
        let single = registry.def(chest).boxes.clone();
        let left = registry.def(registry.variant(chest, &[0, 1])).boxes.clone();
        let right = registry.def(registry.variant(chest, &[0, 2])).boxes.clone();
        // Le corps touche la jointure, l'autre bord ne bouge pas.
        assert_eq!(left[0].min.x, 0.0);
        assert_eq!(left[0].max.x, single[0].max.x);
        assert_eq!(right[0].max.x, 1.0);
        assert_eq!(right[0].min.x, single[0].min.x);
        // La serrure : deux demi-pièces collées à la jointure, aussi larges qu'une seule.
        let latch = single.len() - 1;
        assert_eq!(left[latch].min.x, 0.0);
        assert_eq!(right[latch].max.x, 1.0);
        let width = |b: &BlockBox| b.max.x - b.min.x;
        assert!((width(&left[latch]) + width(&right[latch]) - width(&single[latch])).abs() < 1e-6);
    }

    #[test]
    fn old_saves_still_name_a_single_chest() {
        let registry = BlockRegistry::builtin();
        let chest = registry.id("core:chest").unwrap();
        for (facing, name) in ["north", "east", "south", "west"].iter().enumerate() {
            let old = registry.id(&format!("core:chest[facing={name}]")).unwrap();
            assert_eq!(old, registry.variant(chest, &[facing as u8, 0]));
            assert_eq!(part(&registry, old), Some(Part::Single));
        }
    }

    #[test]
    fn two_chests_merge_in_every_direction_and_split_back_without_loss() {
        let registry = BlockRegistry::builtin();
        let single = chest_layout("core:chest").unwrap();
        for facing in 0..4u8 {
            let mut world = new_world(&registry);
            let a = IVec3::new(3, 10, 3);
            let b = a + side(facing);
            place(&mut world, a, "core:chest", facing);
            let mut first = vec![None; single.total()];
            first[0] = stack("core:dirt", 5);
            first[single.main] = stack("core:stack_upgrade_2", 1);
            first[single.main + single.upgrades] = stack("core:cobblestone", 1);
            put(&mut world, a, first);
            place(&mut world, b, "core:chest", facing);
            let mut second = vec![None; single.total()];
            second[3] = stack("core:sand", 7);
            put(&mut world, b, second);

            assert_eq!(merge(&mut world, b), Some(a));
            // `b` est à +X du modèle : c'est la moitié gauche, elle garde tout.
            assert_eq!(part(&registry, world.block(b)), Some(Part::Left));
            assert_eq!(part(&registry, world.block(a)), Some(Part::Right));
            assert_eq!(storage_pos(&world, a), b);
            let merged = slots(&world, b);
            let double = doubled(single);
            assert_eq!(merged.len(), double.total());
            assert_eq!(merged[3], stack("core:sand", 7));
            assert_eq!(merged[single.main], stack("core:dirt", 5));
            assert_eq!(merged[double.main + 1], stack("core:stack_upgrade_2", 1));
            assert_eq!(
                merged[double.main + double.upgrades],
                stack("core:cobblestone", 1)
            );
            assert!(slots(&world, a).is_empty());

            // On casse la moitié gauche : elle garde ses objets, l'autre reprend les siens.
            assert_eq!(split(&mut world, b), Some(a));
            assert_eq!(part(&registry, world.block(a)), Some(Part::Single));
            assert_eq!(part(&registry, world.block(b)), Some(Part::Single));
            let (kept, broken) = (slots(&world, a), slots(&world, b));
            assert_eq!(kept[0], stack("core:dirt", 5));
            assert_eq!(kept[single.main], stack("core:stack_upgrade_2", 1));
            assert_eq!(
                kept[single.main + single.upgrades],
                stack("core:cobblestone", 1)
            );
            assert_eq!(broken[3], stack("core:sand", 7));
            assert!(broken[single.main..].iter().all(Option::is_none));
        }
    }

    #[test]
    fn only_identical_single_neighbours_merge() {
        let registry = BlockRegistry::builtin();
        let mut world = new_world(&registry);
        let a = IVec3::new(0, 10, 0);
        // Pas devant ni derrière, pas tourné autrement, pas d'un autre palier.
        place(&mut world, a + IVec3::Z, "core:chest", 0);
        place(&mut world, a + IVec3::X, "core:chest", 1);
        place(&mut world, a - IVec3::X, "core:copper_chest", 0);
        place(&mut world, a, "core:chest", 0);
        assert_eq!(merge(&mut world, a), None);
        // Un troisième coffre ne rejoint pas un grand coffre.
        let mut world = new_world(&registry);
        place(&mut world, a, "core:chest", 0);
        place(&mut world, a + IVec3::X, "core:chest", 0);
        assert!(merge(&mut world, a + IVec3::X).is_some());
        place(&mut world, a + 2 * IVec3::X, "core:chest", 0);
        assert_eq!(merge(&mut world, a + 2 * IVec3::X), None);
    }
}
