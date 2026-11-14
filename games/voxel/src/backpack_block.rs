//! Les sacs à dos posés au sol : Maj + clic droit avec un sac en main pose un bloc à son image
//! (`core:backpack_<palier>_placed`, voir `blocks.ron`). Le bloc n'a pas d'objet à lui : l'objet
//! sac le pose (`places` dans `items.ron`), et l'entité du bloc retient le **numéro du sac**.
//!
//! - Un clic droit sur le sac posé ouvre son contenu, comme depuis l'inventaire.
//! - Cassé (à la main, par une explosion…), il rend le même sac, avec son numéro : le contenu,
//!   rangé dans le magasin des sacs (`backpacks.rs`), n'a jamais bougé.
//! - Posé, un sac n'est plus porté : ses améliorations (aimant, ramassage…) ne jouent plus.

use engine_voxel::{BlockEntity, BlockId, BlockRegistry, ItemRegistry};
use glam::IVec3;

use crate::VoxelGame;
use crate::backpacks::{self, MAX_NUMBER};
use crate::inventory::ItemStack;

/// Le comportement des blocs « sac posé ».
pub const BEHAVIOR: &str = "core:placed_backpack";
/// Ce que la clé du bloc posé ajoute à celle du sac.
const SUFFIX: &str = "_placed";

/// Le numéro de sac écrit dans les données de l'entité (4 octets, petit-boutiste).
pub fn write_number(number: u32) -> Vec<u8> {
    number.to_le_bytes().to_vec()
}

/// Le numéro de sac lu dans les données de l'entité (`None` s'il manque ou sort des bornes).
pub fn read_number(data: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = data.get(..4)?.try_into().ok()?;
    let number = u32::from_le_bytes(bytes);
    (1..=MAX_NUMBER).contains(&number).then_some(number)
}

/// Le bloc est-il un sac posé (quelle que soit son orientation) ?
pub fn is_placed_bag(registry: &BlockRegistry, block: BlockId) -> bool {
    registry.def(block).behavior.as_deref() == Some(BEHAVIOR)
}

/// L'objet sac qui correspond à un sac posé : `core:backpack_iron_placed` → `core:backpack_iron`.
pub fn bag_item_key(registry: &BlockRegistry, block: BlockId) -> Option<String> {
    let def = registry.def(block);
    let base = def.base.map_or(def, |b| registry.def(b));
    let key = base.key.strip_suffix(SUFFIX)?;
    backpacks::is_backpack(key).then(|| key.to_string())
}

/// Ce que rend un sac posé qu'on casse : le sac, avec son numéro. Vide pour un autre bloc.
pub fn broken_bag(
    registry: &BlockRegistry,
    block: BlockId,
    entity: Option<&BlockEntity>,
    items: &ItemRegistry,
) -> Option<ItemStack> {
    if !is_placed_bag(registry, block) {
        return None;
    }
    let item = items.id(&bag_item_key(registry, block)?)?;
    // Un bloc sans numéro (posé par un mod, par une commande) rend un sac neuf.
    let damage = entity.and_then(|e| read_number(&e.data)).unwrap_or(0);
    Some(ItemStack {
        item,
        count: 1,
        damage,
    })
}

impl VoxelGame {
    /// Maj + clic droit avec un sac en main : le pose au sol. `true` si l'objet en main est un sac
    /// (même si la pose échoue : on ne veut pas ouvrir le sac à la place).
    pub(crate) fn place_backpack(&mut self) -> bool {
        let Some(stack) = self.inventory.selected_stack() else {
            return false;
        };
        if !backpacks::is_backpack(&self.content.items.get(stack.item).key) {
            return false;
        }
        let slot = self.inventory.selected;
        let Some(number) = self.bag_number_at(slot) else {
            self.toast("Plus aucun numéro de sac disponible");
            return true;
        };
        let Some(pos) = self.place_at_target() else {
            return true;
        };
        let mut entity = self
            .streamer()
            .block_entity(pos)
            .cloned()
            .unwrap_or_default();
        entity.data = write_number(number);
        self.streamer_mut().set_block_entity(pos, Some(entity));
        true
    }

    /// Le numéro du sac posé en `pos`, s'il y en a un.
    pub(crate) fn placed_bag_number(&self, pos: IVec3) -> Option<u32> {
        let streamer = self.streamer();
        if !is_placed_bag(&self.registry, streamer.world().block(pos)) {
            return None;
        }
        read_number(&streamer.block_entity(pos)?.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_bag_number_survives_the_block_entity() {
        for number in [1, 2, 300, MAX_NUMBER] {
            assert_eq!(read_number(&write_number(number)), Some(number));
        }
        assert_eq!(read_number(&[]), None);
        assert_eq!(read_number(&write_number(0)), None);
        assert_eq!(read_number(&write_number(MAX_NUMBER + 1)), None);
    }

    #[test]
    fn every_bag_places_its_own_block_and_breaks_back_into_itself() {
        let blocks = crate::test_registry();
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let items = ItemRegistry::load_mods(&blocks, &assets, &[]).unwrap();
        for tier in &backpacks::BAG_TIERS {
            let bag = items.get(items.id(tier.key).unwrap());
            let placed = blocks.id(bag.places.as_deref().unwrap()).unwrap();
            assert!(is_placed_bag(&blocks, placed), "{}", tier.key);
            // Le bloc n'a pas d'objet à lui : c'est le sac qui le pose.
            assert!(!blocks.def(placed).placeable);
            // Toutes les orientations rendent le même sac, avec son numéro.
            for variant in blocks
                .iter()
                .map(|(id, _)| id)
                .filter(|&id| id == placed || blocks.def(id).base == Some(placed))
            {
                let entity = BlockEntity {
                    data: write_number(42),
                    ..Default::default()
                };
                let back = broken_bag(&blocks, variant, Some(&entity), &items).unwrap();
                assert_eq!(items.get(back.item).key, tier.key);
                assert_eq!(back.damage, 42);
            }
        }
        // Un autre bloc ne rend rien de plus.
        let chest = blocks.id("core:chest").unwrap();
        assert!(broken_bag(&blocks, chest, None, &items).is_none());
    }
}
