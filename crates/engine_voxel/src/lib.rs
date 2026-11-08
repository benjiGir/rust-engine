//! `engine_voxel` : le monde en blocs.
//!
//! - [`BlockId`] : le numéro d'un bloc ; [`BlockRegistry`] : ses propriétés, lues
//!   dans `assets/core/blocks.ron` et dans les mods.
//! - [`Chunk`] : un cube de 32×32×32 blocs, l'unité de stockage et de maillage.
//! - [`World`] : tous les chunks, accessibles en coordonnées monde.
//! - [`TerrainGenerator`] : le terrain procédural (bruit de Perlin, biomes, grottes).
//! - [`mesh_culled`] et [`mesh_greedy`] : transforment un chunk en triangles.
//! - [`ChunkStreamer`] : charge et maille les chunks autour du joueur, en tâche de fond.
//! - [`raycast`] : le bloc visé. [`PlayerBody`] : la physique du joueur.
//!
//! Cette crate ne connaît pas wgpu : le jeu passe les sommets au renderer.

mod block;
mod chunk;
mod generator;
mod mesh;
mod physics;
mod raycast;
mod registry;
mod streaming;
mod world;

pub use block::BlockId;
pub use chunk::{CHUNK_SIZE, Chunk};
pub use generator::{Biome, Column, TerrainGenerator, WORLD_HEIGHT_CHUNKS};
pub use mesh::{ChunkMesh, MeshVertex, Mesher, Neighbourhood, mesh_culled, mesh_greedy};
pub use physics::{PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
pub use raycast::{RayHit, raycast};
pub use registry::{BlockDef, BlockRegistry, CORE_NAMESPACE};
pub use streaming::{ChunkStreamer, StreamEvent, StreamStats, StreamingConfig};
pub use world::World;
