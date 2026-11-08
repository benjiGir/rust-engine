//! Un chunk : 32×32×32 blocs rangés dans un seul tableau.

use crate::block::BlockId;

/// Côté d'un chunk, en blocs.
pub const CHUNK_SIZE: usize = 32;
const VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// Les blocs d'un chunk, stockés « à plat ».
///
/// `Box<[BlockId]>` est un tableau de taille fixe alloué sur le tas.
/// On évite `[BlockId; VOLUME]` directement : 64 Ko sur la pile à chaque
/// déplacement du chunk, c'est lent et peut faire déborder la pile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    blocks: Box<[BlockId]>,
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

impl Chunk {
    /// Un chunk rempli d'air.
    pub fn new() -> Self {
        Self {
            // `vec![x; n]` puis `into_boxed_slice` : on fige la taille du tableau.
            blocks: vec![BlockId::AIR; VOLUME].into_boxed_slice(),
        }
    }

    /// Position dans le tableau. On parcourt x, puis z, puis y :
    /// une couche horizontale entière est donc contiguë en mémoire.
    #[inline]
    pub const fn index(x: usize, y: usize, z: usize) -> usize {
        x + z * CHUNK_SIZE + y * CHUNK_SIZE * CHUNK_SIZE
    }

    /// Coordonnées locales, chacune dans `0..CHUNK_SIZE`.
    /// Hors limites, l'indexation du tableau panique : c'est un bug de l'appelant.
    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.blocks[Self::index(x, y, z)]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        self.blocks[Self::index(x, y, z)] = block;
    }

    /// Vrai si le chunk ne contient que de l'air (rien à mailler).
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|b| b.is_air())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::stone;

    #[test]
    fn index_covers_the_whole_volume_once() {
        let mut seen = vec![false; VOLUME];
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let i = Chunk::index(x, y, z);
                    assert!(!seen[i], "index {i} utilisé deux fois");
                    seen[i] = true;
                }
            }
        }
        assert!(seen.into_iter().all(|s| s));
    }

    #[test]
    fn set_then_get() {
        let mut chunk = Chunk::new();
        assert!(chunk.is_empty());
        chunk.set(1, 2, 3, stone());
        assert_eq!(chunk.get(1, 2, 3), stone());
        assert_eq!(chunk.get(3, 2, 1), BlockId::AIR);
        assert!(!chunk.is_empty());
    }
}
