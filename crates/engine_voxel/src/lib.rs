//! `engine_voxel` : le monde en blocs.
//!
//! - [`BlockId`] : la nature d'un bloc (air, herbe, terre, pierre).
//! - [`Chunk`] : un cube de 32×32×32 blocs, l'unité de stockage et de maillage.
//! - [`World`] : tous les chunks, accessibles en coordonnées monde.
//! - [`TerrainGenerator`] : le terrain procédural (bruit de Perlin, biomes, grottes).
//! - [`mesh_culled`] et [`mesh_greedy`] : transforment un chunk en triangles.
//! - [`ChunkStreamer`] : charge et maille les chunks autour du joueur, en tâche de fond.
//!
//! Cette crate ne connaît pas wgpu : le jeu passe les sommets au renderer.

mod block;
mod chunk;
mod generator;
mod mesh;
mod streaming;
mod world;

pub use block::BlockId;
pub use chunk::{CHUNK_SIZE, Chunk};
pub use generator::{Biome, Column, TerrainGenerator, WORLD_HEIGHT_CHUNKS};
pub use mesh::{ChunkMesh, MeshVertex, Mesher, Neighbourhood, mesh_culled, mesh_greedy};
pub use streaming::{ChunkStreamer, StreamEvent, StreamStats, StreamingConfig};
pub use world::World;
