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
//! - [`SaveStore`] : la sauvegarde du monde sur disque.
//!
//! Cette crate ne connaît pas wgpu : le jeu passe les sommets au renderer.

mod biome;
mod block;
mod block_rules;
mod capabilities;
mod chunk;
mod containers;
mod crafting;
mod fluid_sim;
mod fluids;
mod generator;
mod grid;
mod items;
mod light;
mod machines;
mod mesh;
mod physics;
mod plants;
mod raycast;
mod registry;
mod save;
mod streaming;
mod trees;
mod visibility;
mod world;

pub use biome::{BiomeDef, BiomeId, BiomeKind, BiomeRegistry, OreDef, Relief};
pub use block::BlockId;
pub use block_rules::{Settled, is_supported, settle};
pub use capabilities::{
    Capabilities, EnergyStorage, Face, FluidTank, Inventory, ItemStack, Kinetic, MAX_STACK, Sides,
};
pub use chunk::{BlockEntity, CHUNK_SIZE, Chunk, DEFAULT_LIGHT};
pub use containers::{
    CHEST_SLOTS, Chest, FURNACE_FUEL, FURNACE_INPUT, FURNACE_OUTPUT, Furnace, FurnaceState,
    container_slots, register as register_containers,
};
pub use crafting::{Ingredient, LootEntry, LootTables, Recipe, Recipes, Smelting, SmeltingRecipe};
pub use fluid_sim::{FULL as FLUID_FULL, FluidKind, FluidReport, FluidSim};
pub use fluids::{FluidDef, FluidRegistry};
pub use generator::{
    Column, DEEPSTONE_LEVEL, FREEZING, LAVA_LEVEL, SEA_LEVEL, TerrainGenerator, TreeSite,
    WORLD_HEIGHT_CHUNKS,
};
pub use grid::{BlockAccess, GridId, GridPos, Grids};
pub use items::{
    ArmorDef, ArmorSlot, Category, FoodDef, Icon, ItemDef, ItemId, ItemRegistry, ToolDef, ToolKind,
    break_time, can_harvest,
};
pub use light::{ColumnLight, LightRegion, MAX_LIGHT, apply_column_light, update_light};
pub use machines::{
    BlockBehavior, Clock, Lamp, MACHINE_TICKS_PER_SECOND, MAX_POWER, MachineAction, MachineContext,
    Machines, Scheduler, TickReport, received_power,
};
pub use mesh::{
    ChunkMesh, FLAG_CUTOUT, FLAG_SWAY, MeshVertex, Mesher, Neighbourhood, TEXTURE_MASK,
    mesh_culled, mesh_greedy,
};
pub use physics::{PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
pub use raycast::{RayHit, ray_aabb, raycast};
pub use registry::{BlockDef, BlockRegistry, BlockSounds, CORE_NAMESPACE, Shape, Support, Tint};
pub use save::{
    ChunkToSave, FORMAT_VERSION, LoadedChunk, PlayerSave, REGION_SIZE, SaveReport, SaveStore,
    SavedInventory, SavedStack, UNKNOWN_BLOCK, UnknownBlocks, WorldMeta, write_atomic,
};
pub use streaming::{ChunkStreamer, StreamEvent, StreamStats, StreamingConfig};
pub use trees::{Part, Rng64, TreeBlocks, TreeKind, Wood, grow as grow_tree};
pub use visibility::{FACE_DIRECTIONS, FaceConnections, VisibilityGraph};
pub use world::World;
