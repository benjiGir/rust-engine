//! Un chunk : 32×32×32 blocs rangés dans un seul tableau.

use rustc_hash::FxHashMap as HashMap;

use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::capabilities::Capabilities;

/// L'état attaché à un bloc « actif » (une machine, une lampe, une horloge...).
///
/// Comme dans Minecraft, une machine n'est pas une entité de l'ECS : c'est un bloc, plus
/// ce petit état rangé dans son chunk. Il est donc sauvegardé, déchargé et rechargé avec
/// le chunk, sans rien faire de spécial, et ne coûte rien tant que la machine dort.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BlockEntity {
    /// La puissance émise vers les voisins, de 0 à 15 (comme la redstone).
    pub power: u8,
    /// Des données libres, que le comportement du bloc (natif ou mod) interprète à sa façon.
    pub data: Vec<u8>,
    /// Énergie, fluides, objets, rotation : les conteneurs génériques que les machines
    /// de mods différents savent s'échanger. Dans une boîte (8 octets quand il n'y en a pas) :
    /// la plupart des blocs actifs n'en ont pas besoin.
    pub caps: Option<Box<Capabilities>>,
}

impl BlockEntity {
    /// Les capacités, créées vides au premier usage.
    pub fn caps_mut(&mut self) -> &mut Capabilities {
        self.caps.get_or_insert_with(Default::default)
    }
}

/// Côté d'un chunk, en blocs.
pub const CHUNK_SIZE: usize = 32;
pub(crate) const VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// La lumière d'un bloc tient dans un octet : deux niveaux de 0 à 15 (4 bits chacun).
/// Les 4 bits de poids fort : la lumière du ciel ; les 4 de poids faible : celle des
/// blocs (torches...). Deux canaux séparés car le ciel change avec l'heure, pas les torches.
pub const DEFAULT_LIGHT: u8 = 0xF0;

/// Les blocs d'un chunk, stockés « à plat ».
///
/// `Box<[BlockId]>` est un tableau de taille fixe alloué sur le tas.
/// On évite `[BlockId; VOLUME]` directement : 64 Ko sur la pile à chaque
/// déplacement du chunk, c'est lent et peut faire déborder la pile.
///
/// À côté des blocs, la lumière : un octet par bloc (voir [`DEFAULT_LIGHT`]).
/// Par défaut, plein ciel (15) et aucune torche (0), la lumière d'un chunk d'air.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    blocks: Box<[BlockId]>,
    light: Box<[u8]>,
    /// Les blocs actifs du chunk, par position ([`Chunk::index`]). Presque toujours vide.
    block_entities: HashMap<u16, BlockEntity>,
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
            light: vec![DEFAULT_LIGHT; VOLUME].into_boxed_slice(),
            block_entities: HashMap::default(),
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

    /// Tous les blocs, dans l'ordre de [`Chunk::index`].
    pub fn blocks(&self) -> &[BlockId] {
        &self.blocks
    }

    /// Un chunk à partir de ses blocs (lumière par défaut). `None` si la taille est fausse.
    pub fn from_blocks(blocks: Vec<BlockId>) -> Option<Self> {
        (blocks.len() == VOLUME).then(|| Self {
            blocks: blocks.into_boxed_slice(),
            light: vec![DEFAULT_LIGHT; VOLUME].into_boxed_slice(),
            block_entities: HashMap::default(),
        })
    }

    /// L'état actif du bloc en (x, y, z), s'il en a un.
    pub fn block_entity(&self, x: usize, y: usize, z: usize) -> Option<&BlockEntity> {
        self.block_entities.get(&(Self::index(x, y, z) as u16))
    }

    pub fn block_entity_mut(&mut self, x: usize, y: usize, z: usize) -> Option<&mut BlockEntity> {
        self.block_entities.get_mut(&(Self::index(x, y, z) as u16))
    }

    pub fn set_block_entity(&mut self, x: usize, y: usize, z: usize, entity: Option<BlockEntity>) {
        let index = Self::index(x, y, z) as u16;
        match entity {
            Some(entity) => self.block_entities.insert(index, entity),
            None => self.block_entities.remove(&index),
        };
    }

    /// Tous les blocs actifs : (position dans le chunk, état).
    pub fn block_entities(&self) -> impl Iterator<Item = (usize, &BlockEntity)> {
        self.block_entities.iter().map(|(&i, e)| (i as usize, e))
    }

    /// Lumière du ciel, de 0 à 15. `>> 4` garde les 4 bits de poids fort.
    #[inline]
    pub fn sky_light(&self, x: usize, y: usize, z: usize) -> u8 {
        self.light[Self::index(x, y, z)] >> 4
    }

    /// Lumière des blocs, de 0 à 15. `& 0x0F` garde les 4 bits de poids faible.
    #[inline]
    pub fn block_light(&self, x: usize, y: usize, z: usize) -> u8 {
        self.light[Self::index(x, y, z)] & 0x0F
    }

    #[inline]
    pub fn set_sky_light(&mut self, x: usize, y: usize, z: usize, level: u8) {
        let cell = &mut self.light[Self::index(x, y, z)];
        *cell = (*cell & 0x0F) | (level << 4);
    }

    #[inline]
    pub fn set_block_light(&mut self, x: usize, y: usize, z: usize, level: u8) {
        let cell = &mut self.light[Self::index(x, y, z)];
        *cell = (*cell & 0xF0) | (level & 0x0F);
    }

    /// Remplace toute la lumière (calculée par une tâche de fond).
    pub fn set_light_data(&mut self, light: Box<[u8]>) {
        assert_eq!(light.len(), VOLUME, "taille de lumière invalide");
        self.light = light;
    }

    /// Remet la lumière par défaut (plein ciel, aucune torche).
    pub fn reset_light(&mut self) {
        self.light.fill(DEFAULT_LIGHT);
    }

    pub fn light_data(&self) -> &[u8] {
        &self.light
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

    #[test]
    fn light_channels_are_independent() {
        let mut chunk = Chunk::new();
        assert_eq!(
            (chunk.sky_light(1, 2, 3), chunk.block_light(1, 2, 3)),
            (15, 0)
        );
        chunk.set_block_light(1, 2, 3, 9);
        chunk.set_sky_light(1, 2, 3, 4);
        assert_eq!(
            (chunk.sky_light(1, 2, 3), chunk.block_light(1, 2, 3)),
            (4, 9)
        );
        chunk.set_sky_light(1, 2, 3, 0);
        assert_eq!(chunk.block_light(1, 2, 3), 9);
    }
}
