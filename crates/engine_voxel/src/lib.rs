//! `engine_voxel` : le monde en blocs.
//!
//! - [`BlockId`] : le numéro d'un bloc ; [`BlockRegistry`] : ses propriétés, lues
//!   dans `assets/core/blocks.ron` et dans les mods.
//! - [`Chunk`] : un cube de 32×32×32 blocs, l'unité de stockage et de maillage.
//! - [`World`] : tous les chunks, accessibles en coordonnées monde.
//! - [`TerrainGenerator`] : le terrain procédural (bruit de Perlin, biomes, grottes).
//! - [`mesh_culled`], [`mesh_greedy`] et [`mesh_binary`] : transforment un chunk en triangles.
//! - [`ChunkStreamer`] : charge et maille les chunks autour du joueur, en tâche de fond. Serveur
//!   (sans maillage) ou réplique du client, avec [`ChunkSender`] pour les envois.
//! - [`raycast`] : le bloc visé. [`PlayerBody`] : la physique du joueur.
//! - [`SaveStore`] : la sauvegarde du monde sur disque.
//!
//! Cette crate ne connaît pas wgpu : le jeu passe les sommets au renderer.

pub mod bed;
mod biome;
mod block;
mod block_rules;
mod capabilities;
mod chunk;
mod chunk_sender;
mod circuits;
mod conduit_shell;
pub use conduit_shell::{ConduitShell, conduit_shell};
mod connections;
mod containers;
mod crafting;
pub mod density_graph;
mod dimension;
pub mod door;
pub mod double_chest;
mod edited_surface;
mod far;
mod far_props;
mod farming;
mod fluid_sim;
mod fluids;
mod generation;
mod generator;
mod grid;
mod items;
mod light;
mod machines;
mod mesh;
mod network_visual;
pub use network_visual::{TransitContent, VisualTransfer};
mod networks;
pub mod paired_block;
mod pathfind;
mod physics;
mod pipeline_timing;
mod plants;
mod rail_motion;
mod rail_save;
mod rails;
mod raycast;
mod recipe_index;
mod registry;
mod save;
pub mod shaped;
mod ship;
mod storage;
mod streaming;
mod subgrid;
mod tech;
mod transfer;
mod trees;
mod visibility;
mod world;
mod worldmap;
mod worldmap_blocks;

pub use biome::{BiomeDef, BiomeId, BiomeKind, BiomeRegistry, OreDef, PlantDef, Relief};
pub use block::BlockId;
pub use block_rules::{Settled, is_supported, settle};
pub use capabilities::{
    Capabilities, EnergyStorage, Face, FluidTank, Inventory, Io, ItemStack, Kinetic, MAX_STACK,
    Sides,
};
pub use chunk::{BlockEntity, CHUNK_SIZE, Chunk, DEFAULT_LIGHT, WireError};
pub use chunk_sender::{ChunkSender, DEFAULT_SEND_BUDGET};
pub use circuits::{FACINGS, PUSH_LIMIT, facing_of, placement_state};
pub use connections::ConnectedShape;
pub use containers::{
    BREW_INPUT, BREW_OUTPUT, BREW_REAGENT, CHEST_SLOTS, Chest, FURNACE_FUEL, FURNACE_INPUT,
    FURNACE_OUTPUT, Furnace, FurnaceState, LOADER_SLOTS, brew_progress, container_slots,
    register as register_containers,
};
pub use crafting::{
    BrewRecipe, Brewing, Ingredient, LootEntry, LootTables, MachineRecipe, MachineRecipes, Recipe,
    Recipes, Smelting, SmeltingRecipe,
};
pub use dimension::{DimensionDef, DimensionRegistry, OVERWORLD, SkyDef, WorldKind};
pub use edited_surface::{ChunkSurface, EditedSurfaces};
pub use far::{
    CELLS as FAR_CELLS, FarConfig, FarDebug, FarEvent, FarMesh, FarStats, FarStreamer, FarTile,
    build_tile, build_tile_debug, build_tile_with, far_allowance, select_tiles,
    select_tiles_stable,
};
pub use farming::{
    FarmBlocks, IRRIGATION_REACH, fertilized, is_farmland, is_irrigated, is_ripe, next_stage,
    tilled,
};
pub use fluid_sim::{FULL as FLUID_FULL, FluidKind, FluidReport, FluidSim};
pub use fluids::{FluidDef, FluidRegistry};
pub use generation::{
    DataRecord, DimensionRecord, GenerationIssue, GenerationMeta, GenerationVersion, KindRecord,
};
pub use generator::{
    Blueprint, ChestLoot, ChestLootEntry, Climate, Column, DEEPSTONE_LEVEL, FREEZING, LAVA_LEVEL,
    MAX_CHUNK_Y, MIN_CHUNK_Y, PENDING_LOOT, SEA_LEVEL, Spot, StructureDef, StructureKind,
    StructureSet, StructureSite, TerrainGenerator, TreeSite, V3Parameters, WORLD_HEIGHT_CHUNKS,
    pending_loot,
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
    ChunkMesh, FLAG_CUTOUT, FLAG_FILLED, FLAG_SWAY, FLAG_TURN_SHIFT, MeshVertex, Mesher,
    Neighbourhood, TEXTURE_MASK, mesh_binary, mesh_culled, mesh_greedy,
};
pub use networks::{
    ENERGY_RATE, Endpoint, FLUID_RATE, Network, NetworkKind, NetworkStats, Networks,
};
pub use pathfind::{Path, PathRules, find_path};
pub use physics::{GRAVITY, PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
pub use pipeline_timing::{PipelineReport, Stage as PipelineStage, StageReport};
pub use plants::{Precipitation, precipitation_at};
pub use rail_motion::{
    DEFAULT_MAX_SPEED, Derail, Drive, MAX_STEP, RailMotion, SPEED_LIMIT, bump, couple,
};
pub use rail_save::{SavedCart, SavedRail, load_carts, save_carts};
pub use rails::{
    RAIL_HEIGHT, Rail, RailBehavior, RailEnd, RailKind, RailShape, SIDES as RAIL_SIDES,
    choose_shape, follow as follow_rail, links as rail_links, rail_at,
};
pub use raycast::{RayHit, ray_aabb, raycast};
pub use recipe_index::{Choice, RecipeCategory, RecipeIndex, RecipeLayout, RecipeView};
pub use registry::{
    BlockBox, BlockDef, BlockRegistry, BlockRotation, BlockSounds, CORE_NAMESPACE, EnchantingStats,
    RotationAxis, Shape, StateKind, Support, Tint,
};
pub use save::{
    AdventureSave, ChunkToSave, FORMAT_VERSION, LoadedChunk, PlayerFile, PlayerSave, REGION_SIZE,
    SaveReport, SaveStore, SavedInventory, SavedStack, UNKNOWN_BLOCK, UnknownBlocks, WorldMeta,
    write_atomic,
};
pub use ship::{ShipInput, StepReport, step as step_ship};
pub use storage::{
    CHEST_TIERS, CONTROLLER_KEY, CONTROLLER_MAX_CONTAINERS, CONTROLLER_RADIUS, ChestTier,
    CompactTable, Controller, FILTER_SLOTS, Fill, Group, KITS, Kit, Layout, MULTIPLIER_CAP, Policy,
    SlotKind, StorageChest, UPGRADE_PERIOD, Upgrade, Voiding, accept as storage_accept,
    chest_layout, chest_tier_of, controller_groups, controller_insert, controller_members,
    controller_room, controller_take, ensure_layout, filter_items, insert as storage_insert,
    installed as installed_upgrades, is_controller, kit_of, kit_target, layout_of, maintain,
    put_in, relayout, room_in, stack_limit, stack_multiplier, upgrade_chest,
};
pub use streaming::{
    ChunkChange, ChunkStreamer, ChunkUpdate, StreamEvent, StreamStats, StreamingConfig, View,
};
pub use subgrid::{
    AssemblyError, MAX_BLOCKS, SubGrid, SubGrids, extract, land, landing_cells, load_grids,
    quarter_turns, rotate_facing, rotate_y, save_grids,
};
pub use transfer::{TransferBudget, TransferStats};
pub use trees::{Part, Rng64, TreeBlocks, TreeKind, Wood, grow as grow_tree};
pub use visibility::{FACE_DIRECTIONS, FaceConnections, VisibilityGraph};
pub use world::World;
pub use worldmap::{MapArea, MapKind, MapSamples, Raster};
pub use worldmap_blocks::{
    BlockMapKind, BlockMapSamples, SectionAxis, SectionSpec, render_section,
};
