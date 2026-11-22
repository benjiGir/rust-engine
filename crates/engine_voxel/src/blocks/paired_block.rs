//! Les deux cellules d'un lit ou d'une porte se modifient ensemble.

use glam::{IVec3, Vec3};

use crate::{BlockId, BlockRegistry, blocks::bed, blocks::door};

pub fn partner(registry: &BlockRegistry, pos: IVec3, block: BlockId) -> Option<(IVec3, BlockId)> {
    bed::partner(registry, pos, block).or_else(|| door::partner(registry, pos, block))
}

pub fn display_parts(registry: &BlockRegistry, block: BlockId) -> Option<[(BlockId, Vec3); 2]> {
    bed::display_parts(registry, block).or_else(|| door::display_parts(registry, block))
}

pub(crate) fn turned(registry: &BlockRegistry, block: BlockId, k: u8) -> BlockId {
    let block = bed::turned(registry, block, k);
    door::turned(registry, block, k)
}
