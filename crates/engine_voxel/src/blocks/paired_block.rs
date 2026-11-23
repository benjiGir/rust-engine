//! Les deux cellules d'un lit, d'une porte ou d'une plante haute se modifient ensemble.

use glam::{IVec3, Vec3};

use crate::{BlockId, BlockRegistry, blocks::bed, blocks::door, blocks::tall_plant};

pub fn partner(registry: &BlockRegistry, pos: IVec3, block: BlockId) -> Option<(IVec3, BlockId)> {
    bed::partner(registry, pos, block)
        .or_else(|| door::partner(registry, pos, block))
        .or_else(|| tall_plant::partner(registry, pos, block))
}

pub fn display_parts(registry: &BlockRegistry, block: BlockId) -> Option<[(BlockId, Vec3); 2]> {
    bed::display_parts(registry, block).or_else(|| door::display_parts(registry, block))
}

pub(crate) fn turned(registry: &BlockRegistry, block: BlockId, k: u8) -> BlockId {
    let block = bed::turned(registry, block, k);
    door::turned(registry, block, k)
}
