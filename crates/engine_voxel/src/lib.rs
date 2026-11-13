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
mod circuits;
mod conduit_shell;
pub use conduit_shell::{ConduitShell, conduit_shell};
mod connections;
mod containers;
mod crafting;
mod dimension;
mod far;
mod farming;
mod fluid_sim;
mod fluids;
mod generator;
mod grid;
mod items;
mod light;
mod machines;
mod mesh;
mod network_visual;
pub use network_visual::{TransitContent, VisualTransfer};
mod networks;
mod pathfind;
mod physics;
mod plants;
mod raycast;
mod recipe_index;
mod registry;
mod save;
mod ship;
mod streaming;
mod subgrid;
mod tech;
mod trees;
mod visibility;
mod world;

pub use biome::{BiomeDef, BiomeId, BiomeKind, BiomeRegistry, OreDef, PlantDef, Relief};
pub use block::BlockId;
pub use block_rules::{Settled, is_supported, settle};
pub use capabilities::{
    Capabilities, EnergyStorage, Face, FluidTank, Inventory, Io, ItemStack, Kinetic, MAX_STACK,
    Sides,
};
pub use chunk::{BlockEntity, CHUNK_SIZE, Chunk, DEFAULT_LIGHT};
pub use circuits::{FACINGS, PUSH_LIMIT, facing_of, placement_state};
pub use connections::ConnectedShape;
pub use containers::{
    BREW_INPUT, BREW_OUTPUT, BREW_REAGENT, CHEST_SLOTS, Chest, FURNACE_FUEL, FURNACE_INPUT,
    FURNACE_OUTPUT, Furnace, FurnaceState, brew_progress, container_slots,
    register as register_containers,
};
pub use crafting::{
    BrewRecipe, Brewing, Ingredient, LootEntry, LootTables, MachineRecipe, MachineRecipes, Recipe,
    Recipes, Smelting, SmeltingRecipe,
};
pub use dimension::{DimensionDef, DimensionRegistry, OVERWORLD, SkyDef, WorldKind};
pub use far::{
    CELLS as FAR_CELLS, FarConfig, FarEvent, FarMesh, FarStats, FarStreamer, FarTile, build_tile,
    select_tiles,
};
pub use farming::{
    FarmBlocks, IRRIGATION_REACH, fertilized, is_farmland, is_irrigated, is_ripe, next_stage,
    tilled,
};
pub use fluid_sim::{FULL as FLUID_FULL, FluidKind, FluidReport, FluidSim};
pub use fluids::{FluidDef, FluidRegistry};
pub use generator::{
    Blueprint, ChestLoot, ChestLootEntry, Column, DEEPSTONE_LEVEL, FREEZING, LAVA_LEVEL, SEA_LEVEL,
    Spot, StructureDef, StructureKind, StructureSet, StructureSite, TerrainGenerator, TreeSite,
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
    ChunkMesh, FLAG_CUTOUT, FLAG_FILLED, FLAG_SWAY, MeshVertex, Mesher, Neighbourhood,
    TEXTURE_MASK, mesh_culled, mesh_greedy,
};
pub use networks::{
    ENERGY_RATE, Endpoint, FLUID_RATE, Network, NetworkKind, NetworkStats, Networks,
};
pub use pathfind::{Path, PathRules, find_path};
pub use physics::{GRAVITY, PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
pub use plants::{Precipitation, precipitation_at};
pub use raycast::{RayHit, ray_aabb, raycast};
pub use recipe_index::{Choice, RecipeCategory, RecipeIndex, RecipeLayout, RecipeView};
pub use registry::{
    BlockBox, BlockDef, BlockRegistry, BlockRotation, BlockSounds, CORE_NAMESPACE, RotationAxis,
    Shape, StateKind, Support, Tint,
};
pub use save::{
    ChunkToSave, FORMAT_VERSION, LoadedChunk, PlayerSave, REGION_SIZE, SaveReport, SaveStore,
    SavedInventory, SavedStack, UNKNOWN_BLOCK, UnknownBlocks, WorldMeta, write_atomic,
};
pub use ship::{ShipInput, StepReport, step as step_ship};
pub use streaming::{ChunkStreamer, StreamEvent, StreamStats, StreamingConfig};
pub use subgrid::{
    AssemblyError, MAX_BLOCKS, SubGrid, SubGrids, extract, land, landing_cells, load_grids,
    quarter_turns, rotate_facing, rotate_y, save_grids,
};
pub use trees::{Part, Rng64, TreeBlocks, TreeKind, Wood, grow as grow_tree};
pub use visibility::{FACE_DIRECTIONS, FaceConnections, VisibilityGraph};
pub use world::World;
