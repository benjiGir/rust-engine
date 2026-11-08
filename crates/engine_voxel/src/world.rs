//! Le monde : une table de chunks indexée par leur position.

use std::collections::HashMap;
use std::sync::Arc;

use glam::IVec3;

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;

const SIZE: i32 = CHUNK_SIZE as i32;

/// Tous les chunks chargés. Un chunk absent est considéré comme de l'air.
///
/// La clé est la position du chunk en « unités de chunk » :
/// le chunk (1, 0, 0) contient les blocs x = 32..64.
///
/// Les chunks sont rangés dans des `Arc<Chunk>` (« Atomically Reference Counted ») :
/// plusieurs propriétaires peuvent partager le même chunk, même sur des threads
/// différents. Cloner un `Arc` ne copie pas les 64 Ko du chunk, cela incrémente
/// juste un compteur. C'est ce qui permet d'envoyer un chunk et ses voisins à un
/// thread de maillage pendant que le thread principal continue à les lire.
///
/// Le monde garde aussi le [`BlockRegistry`] : lui seul sait si un numéro de bloc
/// est solide, transparent, de quelle couleur...
#[derive(Debug, Clone)]
pub struct World {
    chunks: HashMap<IVec3, Arc<Chunk>>,
    registry: Arc<BlockRegistry>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Un monde vide avec les blocs de base intégrés.
    pub fn new() -> Self {
        Self::with_registry(BlockRegistry::builtin())
    }

    pub fn with_registry(registry: Arc<BlockRegistry>) -> Self {
        Self {
            chunks: HashMap::new(),
            registry,
        }
    }

    pub fn registry(&self) -> &Arc<BlockRegistry> {
        &self.registry
    }

    /// Remplace le registre (rechargement à chaud). Les numéros de blocs doivent
    /// désigner les mêmes blocs dans l'ancien et le nouveau registre.
    pub fn set_registry(&mut self, registry: Arc<BlockRegistry>) {
        self.registry = registry;
    }

    /// Lumière (ciel, blocs) à une position monde, chacune de 0 à 15.
    ///
    /// Un chunk absent est de l'air en plein ciel (15, 0) ; sous le monde, tout est noir.
    pub fn light(&self, pos: IVec3) -> (u8, u8) {
        if pos.y < 0 {
            return (0, 0);
        }
        let (chunk, [x, y, z]) = Self::split(pos);
        self.chunks
            .get(&chunk)
            .map_or((15, 0), |c| (c.sky_light(x, y, z), c.block_light(x, y, z)))
    }

    /// Vrai si le bloc à cette position arrête le joueur.
    pub fn is_solid(&self, pos: IVec3) -> bool {
        self.registry.is_solid(self.block(pos))
    }

    /// Vrai si le joueur peut viser le bloc à cette position.
    pub fn is_selectable(&self, pos: IVec3) -> bool {
        self.registry.is_selectable(self.block(pos))
    }

    /// Sépare une position monde en (chunk, position locale dans ce chunk).
    ///
    /// `div_euclid` / `rem_euclid` arrondissent vers -∞, contrairement à `/` et `%` :
    /// `-1 / 32 == 0` et `-1 % 32 == -1`, alors que `(-1).div_euclid(32) == -1`
    /// et `(-1).rem_euclid(32) == 31`. Le bloc x = -1 est bien le dernier du chunk -1.
    pub fn split(pos: IVec3) -> (IVec3, [usize; 3]) {
        let chunk = IVec3::new(
            pos.x.div_euclid(SIZE),
            pos.y.div_euclid(SIZE),
            pos.z.div_euclid(SIZE),
        );
        let local = [
            pos.x.rem_euclid(SIZE) as usize,
            pos.y.rem_euclid(SIZE) as usize,
            pos.z.rem_euclid(SIZE) as usize,
        ];
        (chunk, local)
    }

    /// Le chunk qui contient un point de l'espace (par exemple la caméra).
    pub fn chunk_containing(point: glam::Vec3) -> IVec3 {
        Self::split(point.floor().as_ivec3()).0
    }

    /// Le bloc à une position monde (de l'air si le chunk n'existe pas).
    pub fn block(&self, pos: IVec3) -> BlockId {
        let (chunk, [x, y, z]) = Self::split(pos);
        // `map_or` : la valeur par défaut si `None`, sinon la fonction appliquée.
        self.chunks
            .get(&chunk)
            .map_or(BlockId::AIR, |c| c.get(x, y, z))
    }

    /// Pose un bloc. Le chunk est créé s'il n'existait pas.
    pub fn set_block(&mut self, pos: IVec3, block: BlockId) {
        let (chunk, [x, y, z]) = Self::split(pos);
        self.chunk_mut(chunk).set(x, y, z, block);
    }

    pub fn chunk(&self, pos: IVec3) -> Option<&Chunk> {
        // `as_ref` sur l'`Arc` : on prête le chunk lui-même, pas son enveloppe.
        self.chunks.get(&pos).map(|c| c.as_ref())
    }

    /// Une copie de l'`Arc` du chunk : un nouveau propriétaire, sans copier les blocs.
    pub fn chunk_arc(&self, pos: IVec3) -> Option<Arc<Chunk>> {
        self.chunks.get(&pos).cloned()
    }

    /// Accès en écriture à un chunk, créé s'il n'existait pas.
    ///
    /// `Arc::make_mut` est du « copy-on-write » : si personne d'autre ne partage
    /// ce chunk, on le modifie sur place ; sinon (un thread est en train de le
    /// mailler), on en fait d'abord une copie privée. L'autre thread garde l'ancienne
    /// version intacte : pas besoin de verrou.
    pub fn chunk_mut(&mut self, pos: IVec3) -> &mut Chunk {
        Arc::make_mut(self.chunks.entry(pos).or_default())
    }

    /// Ajoute (ou remplace) un chunk déjà construit, par exemple par un thread de génération.
    pub fn insert(&mut self, pos: IVec3, chunk: Arc<Chunk>) {
        self.chunks.insert(pos, chunk);
    }

    /// Retire un chunk. La mémoire est libérée quand le dernier `Arc` disparaît.
    pub fn remove(&mut self, pos: IVec3) -> Option<Arc<Chunk>> {
        self.chunks.remove(&pos)
    }

    /// Les positions de tous les chunks chargés, dans un ordre quelconque.
    /// `impl Iterator` : on renvoie « un itérateur » sans nommer son type exact.
    pub fn chunk_positions(&self) -> impl Iterator<Item = IVec3> + '_ {
        self.chunks.keys().copied()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Le coin minimum d'un chunk, en coordonnées monde.
    pub fn chunk_origin(chunk: IVec3) -> IVec3 {
        chunk * SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::{dirt, sand, stone};

    #[test]
    fn split_handles_negative_coordinates() {
        assert_eq!(World::split(IVec3::new(0, 0, 0)), (IVec3::ZERO, [0, 0, 0]));
        assert_eq!(
            World::split(IVec3::new(31, 32, 33)),
            (IVec3::new(0, 1, 1), [31, 0, 1])
        );
        assert_eq!(
            World::split(IVec3::new(-1, -32, -33)),
            (IVec3::new(-1, -1, -2), [31, 0, 31])
        );
    }

    #[test]
    fn blocks_round_trip_across_chunks() {
        let mut world = World::new();
        let positions = [IVec3::new(-1, 0, 0), IVec3::new(32, 5, -40)];
        for pos in positions {
            world.set_block(pos, dirt());
        }
        assert_eq!(world.chunk_count(), 2);
        for pos in positions {
            assert_eq!(world.block(pos), dirt());
        }
        assert_eq!(world.block(IVec3::new(1000, 0, 0)), BlockId::AIR);
    }

    #[test]
    fn editing_a_shared_chunk_copies_it() {
        let mut world = World::new();
        world.set_block(IVec3::ZERO, stone());
        // Un « thread de maillage » garde une référence sur l'ancienne version.
        let snapshot = world.chunk_arc(IVec3::ZERO).unwrap();
        world.set_block(IVec3::ZERO, sand());
        assert_eq!(snapshot.get(0, 0, 0), stone());
        assert_eq!(world.block(IVec3::ZERO), sand());
    }

    #[test]
    fn chunk_containing_rounds_down() {
        assert_eq!(
            World::chunk_containing(glam::Vec3::new(-0.5, 31.9, 32.0)),
            IVec3::new(-1, 0, 1)
        );
    }
}
