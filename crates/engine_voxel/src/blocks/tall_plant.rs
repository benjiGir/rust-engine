//! Les plantes de deux blocs de haut (lot N5 de `docs/plan-nature.md`) : la grande herbe, le tournesol.
//! Un pied et une tête, tous deux au comportement `core:tall_plant` ; la tête a la clé du pied suivie
//! de `_top` (`core:sunflower` et `core:sunflower_top`). Elles se posent et se cassent ensemble, comme
//! les deux moitiés d'une porte (voir `paired_block.rs`).

use glam::IVec3;

use crate::{BlockId, BlockRegistry};

/// Le comportement des deux blocs d'une plante haute.
pub const BEHAVIOR: &str = "core:tall_plant";

fn is_tall(registry: &BlockRegistry, block: BlockId) -> bool {
    registry.def(block).behavior.as_deref() == Some(BEHAVIOR)
}

/// La tête d'un pied de plante haute (`None` : ce n'est pas un pied).
pub fn top_of(registry: &BlockRegistry, block: BlockId) -> Option<BlockId> {
    if !is_tall(registry, block) {
        return None;
    }
    let key = &registry.def(block).key;
    if key.ends_with("_top") {
        return None;
    }
    registry.id(&format!("{key}_top"))
}

/// L'autre moitié d'une plante haute posée en `pos` : la tête au-dessus d'un pied, le pied sous une
/// tête.
pub(crate) fn partner(
    registry: &BlockRegistry,
    pos: IVec3,
    block: BlockId,
) -> Option<(IVec3, BlockId)> {
    if !is_tall(registry, block) {
        return None;
    }
    let key = &registry.def(block).key;
    match key.strip_suffix("_top") {
        Some(stem) => Some((pos - IVec3::Y, registry.id(stem)?)),
        None => Some((pos + IVec3::Y, registry.id(&format!("{key}_top"))?)),
    }
}

/// Une plante haute que le joueur pose : le pied en `pos` et la tête au-dessus.
pub fn placement(
    registry: &BlockRegistry,
    block: BlockId,
    pos: IVec3,
) -> Option<[(IVec3, BlockId); 2]> {
    Some([(pos, block), (pos + IVec3::Y, top_of(registry, block)?)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_foot_and_the_head_know_each_other() {
        let registry = BlockRegistry::builtin();
        let foot = registry.id("core:sunflower").unwrap();
        let head = registry.id("core:sunflower_top").unwrap();
        assert_eq!(top_of(&registry, foot), Some(head));
        assert_eq!(top_of(&registry, head), None);
        let at = IVec3::new(3, 70, -2);
        assert_eq!(partner(&registry, at, foot), Some((at + IVec3::Y, head)));
        assert_eq!(partner(&registry, at + IVec3::Y, head), Some((at, foot)));
        assert_eq!(
            placement(&registry, foot, at),
            Some([(at, foot), (at + IVec3::Y, head)])
        );
        // Une plante ordinaire n'a pas de tête.
        let poppy = registry.id("core:poppy").unwrap();
        assert_eq!(top_of(&registry, poppy), None);
        assert_eq!(partner(&registry, at, poppy), None);
    }
}
