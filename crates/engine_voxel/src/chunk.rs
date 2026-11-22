//! Un chunk : 32×32×32 blocs rangés dans un seul tableau.

use rustc_hash::FxHashMap as HashMap;

use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::tech::capabilities::Capabilities;

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
    /// Le biome de chaque colonne (x + 32 z), pour teinter l'herbe et les feuilles. Vide
    /// si inconnu. Il n'est pas sauvegardé : le générateur le recalcule (il ne dépend que
    /// de la graine).
    biomes: Vec<u8>,
    /// La quantité de fluide des cellules qui ne sont pas des « réservoirs » pleins
    /// (voir `fluids/sim.rs`), par position ([`Chunk::index`]), en 4096e de bloc.
    /// Un bloc de fluide absent de cette table est un réservoir : plein et stable.
    fluids: HashMap<u16, u16>,
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
            biomes: Vec::new(),
            fluids: HashMap::default(),
        }
    }

    /// La quantité de fluide d'une cellule (`None` : pas de quantité enregistrée, donc un
    /// réservoir plein si le bloc est un fluide).
    #[inline]
    pub fn fluid_amount(&self, x: usize, y: usize, z: usize) -> Option<u16> {
        if self.fluids.is_empty() {
            return None;
        }
        self.fluids.get(&(Self::index(x, y, z) as u16)).copied()
    }

    pub fn set_fluid_amount(&mut self, x: usize, y: usize, z: usize, amount: Option<u16>) {
        let index = Self::index(x, y, z) as u16;
        match amount {
            Some(a) => self.fluids.insert(index, a),
            None => self.fluids.remove(&index),
        };
    }

    /// Les quantités de fluide enregistrées : (position, quantité).
    pub fn fluid_amounts(&self) -> impl Iterator<Item = (usize, u16)> + '_ {
        self.fluids.iter().map(|(&i, &a)| (i as usize, a))
    }

    /// Le biome (son numéro dans le registre des biomes) de la colonne (x, z).
    #[inline]
    pub fn biome(&self, x: usize, z: usize) -> Option<u8> {
        self.biomes.get(x + z * CHUNK_SIZE).copied()
    }

    /// Les biomes des 32 × 32 colonnes (x + 32 z).
    pub fn set_biomes(&mut self, biomes: Vec<u8>) {
        debug_assert!(biomes.is_empty() || biomes.len() == CHUNK_SIZE * CHUNK_SIZE);
        self.biomes = biomes;
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
            biomes: Vec::new(),
            fluids: HashMap::default(),
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

/// Un chunk tel qu'il passe sur le réseau : une palette de numéros de blocs (le client a le même
/// registre, la poignée de main l'a vérifié), un indice par bloc, la lumière, les biomes, les blocs
/// actifs et les fluides. La trame le compresse.
#[derive(Debug, Serialize, Deserialize)]
struct WireChunk {
    palette: Vec<u16>,
    indices: Vec<u16>,
    light: Vec<u8>,
    biomes: Vec<u8>,
    block_entities: Vec<(u16, BlockEntity)>,
    fluids: Vec<(u16, u16)>,
}

/// Un chunk reçu illisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(pub String);

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chunk illisible : {}", self.0)
    }
}

impl std::error::Error for WireError {}

impl Chunk {
    /// Les octets d'un chunk pour le réseau (voir [`Chunk::from_wire`]).
    pub fn to_wire(&self) -> Vec<u8> {
        let mut palette: Vec<u16> = Vec::new();
        let mut lookup: HashMap<u16, u16> = HashMap::default();
        let indices = self
            .blocks
            .iter()
            .map(|id| {
                *lookup.entry(id.0).or_insert_with(|| {
                    palette.push(id.0);
                    (palette.len() - 1) as u16
                })
            })
            .collect();
        let mut block_entities: Vec<(u16, BlockEntity)> = self
            .block_entities
            .iter()
            .map(|(i, e)| (*i, e.clone()))
            .collect();
        block_entities.sort_by_key(|(i, _)| *i);
        let mut fluids: Vec<(u16, u16)> = self.fluids.iter().map(|(i, a)| (*i, *a)).collect();
        fluids.sort_unstable();
        let wire = WireChunk {
            palette,
            indices,
            light: self.light.to_vec(),
            biomes: self.biomes.clone(),
            block_entities,
            fluids,
        };
        postcard::to_allocvec(&wire).expect("un chunk se sérialise toujours")
    }

    /// Relit un chunk reçu du réseau.
    pub fn from_wire(bytes: &[u8]) -> Result<Self, WireError> {
        let wire: WireChunk = postcard::from_bytes(bytes).map_err(|e| WireError(e.to_string()))?;
        if wire.indices.len() != VOLUME || wire.light.len() != VOLUME {
            return Err(WireError("taille invalide".to_string()));
        }
        let blocks = wire
            .indices
            .iter()
            .map(|&i| {
                wire.palette
                    .get(usize::from(i))
                    .map(|&id| BlockId(id))
                    .ok_or_else(|| WireError("indice hors de la palette".to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let valid = |i: u16| usize::from(i) < VOLUME;
        if !wire.block_entities.iter().all(|(i, _)| valid(*i))
            || !wire.fluids.iter().all(|(i, _)| valid(*i))
        {
            return Err(WireError("position hors du chunk".to_string()));
        }
        Ok(Self {
            blocks: blocks.into_boxed_slice(),
            light: wire.light.into_boxed_slice(),
            block_entities: wire.block_entities.into_iter().collect(),
            biomes: wire.biomes,
            fluids: wire.fluids.into_iter().collect(),
        })
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

    #[test]
    fn a_chunk_crosses_the_network_unchanged() {
        let mut chunk = Chunk::new();
        chunk.set(1, 2, 3, BlockId(7));
        chunk.set(31, 31, 31, BlockId(300));
        chunk.set_fluid_amount(4, 5, 6, Some(2048));
        chunk.set_block_entity(1, 2, 3, Some(BlockEntity::default()));
        let bytes = chunk.to_wire();
        let back = Chunk::from_wire(&bytes).unwrap();
        assert!(back.blocks == chunk.blocks && back.light == chunk.light);
        assert_eq!(back.fluid_amount(4, 5, 6), Some(2048));
        assert_eq!(back.block_entities, chunk.block_entities);
        assert!(Chunk::from_wire(&bytes[..bytes.len() / 2]).is_err());
    }
}
