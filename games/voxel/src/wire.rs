//! Ce que les messages portent par `Arc` (chunks, structures, livre de quêtes), sur le réseau.
//!
//! En mémoire, le serveur et la fenêtre de l'hôte se passent les `Arc` sans copie. Sur le réseau,
//! chaque champ passe par l'encodage de son type : `Chunk::to_wire` (palette de numéros, lumière,
//! blocs actifs, fluides), `SubGrid::to_wire`, et `postcard` pour le livre. Les numéros de blocs sont
//! ceux du serveur : la poignée de main a vérifié que le client a le même contenu.
//!
//! Relire une structure demande le registre des blocs : le client réseau l'enregistre avec
//! [`set_registry`] avant de lire ses premiers messages.

use std::sync::{Arc, RwLock};

use engine_voxel::{BlockRegistry, Chunk, SubGrid};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

static REGISTRY: RwLock<Option<Arc<BlockRegistry>>> = RwLock::new(None);

/// Le registre avec lequel les structures reçues sont relues.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "le client réseau (lot 5d) l'enregistre")
)]
pub fn set_registry(registry: Arc<BlockRegistry>) {
    *REGISTRY.write().unwrap_or_else(|e| e.into_inner()) = Some(registry);
}

fn registry() -> Option<Arc<BlockRegistry>> {
    REGISTRY.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `#[serde(with = "crate::wire::chunk")]` pour un `Arc<Chunk>`.
pub mod chunk {
    use super::*;

    pub fn serialize<S: Serializer>(chunk: &Arc<Chunk>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&chunk.to_wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Chunk>, D::Error> {
        let bytes: Vec<u8> = serde_bytes_vec(d)?;
        Chunk::from_wire(&bytes)
            .map(Arc::new)
            .map_err(D::Error::custom)
    }
}

/// `#[serde(with = "crate::wire::grid")]` pour un `Arc<SubGrid>`.
pub mod grid {
    use super::*;

    pub fn serialize<S: Serializer>(grid: &Arc<SubGrid>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&grid.to_wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<SubGrid>, D::Error> {
        let bytes: Vec<u8> = serde_bytes_vec(d)?;
        let registry =
            registry().ok_or_else(|| D::Error::custom("registre des blocs pas encore connu"))?;
        SubGrid::from_wire(&bytes, registry)
            .map(Arc::new)
            .map_err(D::Error::custom)
    }
}

/// `#[serde(with = "crate::wire::shared")]` pour un `Arc<T>` sérialisable.
pub mod shared {
    use super::*;

    pub fn serialize<T: Serialize, S: Serializer>(value: &Arc<T>, s: S) -> Result<S::Ok, S::Error> {
        T::serialize(value, s)
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Arc<T>, D::Error> {
        T::deserialize(d).map(Arc::new)
    }
}

/// Des octets, lus en un seul bloc (postcard les écrit avec leur longueur).
fn serde_bytes_vec<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    struct Bytes;
    impl<'de> serde::de::Visitor<'de> for Bytes {
        type Value = Vec<u8>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "des octets")
        }

        fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
            Ok(v.to_vec())
        }

        fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
            Ok(v)
        }
    }
    d.deserialize_bytes(Bytes)
}

#[cfg(test)]
mod tests {
    use crate::entities::{GridSnapshot, NetId};
    use crate::protocol::ServerMsg;
    use engine_voxel::{BlockId, Chunk, GridId, SubGrid};
    use glam::IVec3;
    use std::sync::Arc;

    fn round_trip(msg: &ServerMsg) -> ServerMsg {
        let bytes = postcard::to_stdvec(msg).unwrap();
        postcard::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn chunks_structures_and_the_quest_book_cross_the_network() {
        let registry = Arc::new(crate::test_registry());
        super::set_registry(registry.clone());
        let mut chunk = Chunk::new();
        chunk.set(3, 4, 5, BlockId(2));
        let chunk = Arc::new(chunk);
        let ServerMsg::Chunk { pos, chunk: back } = round_trip(&ServerMsg::Chunk {
            pos: IVec3::new(1, -2, 3),
            chunk: chunk.clone(),
        }) else {
            panic!("pas un chunk");
        };
        assert_eq!(pos, IVec3::new(1, -2, 3));
        assert_eq!(back.get(3, 4, 5), BlockId(2));
        let changed = ServerMsg::ChunkChanged {
            pos: IVec3::ZERO,
            chunk,
            edits: vec![IVec3::ONE],
            remesh: true,
        };
        let ServerMsg::ChunkChanged { edits, remesh, .. } = round_trip(&changed) else {
            panic!("pas un changement");
        };
        assert_eq!((edits, remesh), (vec![IVec3::ONE], true));
        let mut grid = SubGrid::new(GridId(4), registry);
        grid.yaw = 1.5;
        let msg = ServerMsg::GridBlocks {
            id: NetId(9),
            grid: GridSnapshot(Arc::new(grid)),
        };
        let ServerMsg::GridBlocks { id, grid } = round_trip(&msg) else {
            panic!("pas une structure");
        };
        assert_eq!((id, grid.0.id, grid.0.yaw), (NetId(9), GridId(4), 1.5));
        let book = Arc::new(crate::quests::Quests::default());
        assert_eq!(
            round_trip(&ServerMsg::QuestBook(book.clone())),
            ServerMsg::QuestBook(book)
        );
    }
}
