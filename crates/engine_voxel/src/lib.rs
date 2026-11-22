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

pub mod blocks;
/// Dimensions communes à la flamme posée et à la flamme des objets.
mod motion;
pub mod worldgen;
pub mod torch {
    use crate::BlockBox;
    use glam::Vec3;

    pub const KEY: &str = "core:torch";
    pub const FLAME: BlockBox = BlockBox {
        min: Vec3::new(4.0 / 16.0, 9.0 / 16.0, 4.0 / 16.0),
        max: Vec3::new(12.0 / 16.0, 1.0, 12.0 / 16.0),
    };
}

mod block;
mod chunk;
pub use tech::conduit_shell::{ConduitShell, conduit_shell};
mod edited_surface;
mod fluids;
mod grid;
mod items;
mod light;
mod mesh;
pub use tech::network_visual::{TransitContent, VisualTransfer};
mod physics;
mod raycast;
mod registry;
mod save;
pub mod shaped;
mod streaming;
mod tech;
mod visibility;
mod world;

pub use block::BlockId;
pub use blocks::block_rules::{Settled, is_supported, settle};
pub use blocks::connections::ConnectedShape;
pub use blocks::farming::{
    FarmBlocks, IRRIGATION_REACH, fertilized, is_farmland, is_irrigated, is_ripe, next_stage,
    tilled,
};
pub use chunk::{BlockEntity, CHUNK_SIZE, Chunk, DEFAULT_LIGHT, WireError};
pub use edited_surface::{ChunkSurface, EditedSurfaces};
pub use fluids::sim::{FULL as FLUID_FULL, FluidKind, FluidReport, FluidSim};
pub use fluids::{FluidDef, FluidRegistry};
pub use grid::{BlockAccess, GridId, GridPos, Grids};
pub use items::{
    ArmorDef, ArmorSlot, Category, FoodDef, Icon, ItemDef, ItemId, ItemRegistry, ToolDef, ToolKind,
    break_time, can_harvest,
};
pub use light::{ColumnLight, LightRegion, MAX_LIGHT, apply_column_light, update_light};
pub use mesh::{
    ChunkMesh, FLAG_CUTOUT, FLAG_FILLED, FLAG_SWAY, FLAG_TURN_SHIFT, MeshVertex, Mesher,
    Neighbourhood, TEXTURE_MASK, mesh_binary, mesh_culled, mesh_greedy,
};
pub use motion::pathfind::{Path, PathRules, find_path};
pub use motion::rail_motion::{
    DEFAULT_MAX_SPEED, Derail, Drive, MAX_STEP, RailMotion, SPEED_LIMIT, bump, couple,
};
pub use motion::rail_save::{SavedCart, SavedRail, load_carts, save_carts};
pub use motion::rails::{
    RAIL_HEIGHT, Rail, RailBehavior, RailEnd, RailKind, RailShape, SIDES as RAIL_SIDES,
    choose_shape, follow as follow_rail, links as rail_links, rail_at,
};
pub use motion::ship::{ShipInput, StepReport, step as step_ship};
pub use motion::subgrid::{
    AssemblyError, MAX_BLOCKS, SubGrid, SubGrids, extract, land, landing_cells, load_grids,
    quarter_turns, rotate_facing, rotate_y, save_grids,
};
pub use physics::{GRAVITY, PLAYER_EYE_HEIGHT, PLAYER_HEIGHT, PLAYER_WIDTH, PlayerBody};
pub use raycast::{RayHit, ray_aabb, raycast};
pub use registry::{
    BlockBox, BlockDef, BlockRegistry, BlockRotation, BlockSounds, CORE_NAMESPACE, EnchantingStats,
    RotationAxis, Shape, StateKind, Support, Tint,
};
pub use save::{
    AdventureSave, ChunkToSave, FORMAT_VERSION, LoadedChunk, PlayerFile, PlayerSave, REGION_SIZE,
    SaveReport, SaveStore, SavedInventory, SavedStack, UNKNOWN_BLOCK, UnknownBlocks, WorldMeta,
    write_atomic,
};
pub use streaming::chunk_sender::{ChunkSender, DEFAULT_SEND_BUDGET};
pub use streaming::pipeline_timing::{PipelineReport, Stage as PipelineStage, StageReport};
pub use streaming::transfer::{TransferBudget, TransferStats};
pub use streaming::{
    ChunkChange, ChunkStreamer, ChunkUpdate, StreamEvent, StreamStats, StreamingConfig, View,
};
pub use tech::capabilities::{
    Capabilities, EnergyStorage, Face, FluidTank, Inventory, Io, ItemStack, Kinetic, MAX_STACK,
    Sides,
};
pub use tech::circuits::{FACINGS, PUSH_LIMIT, facing_of, placement_state};
pub use tech::containers::{
    BREW_INPUT, BREW_OUTPUT, BREW_REAGENT, CHEST_SLOTS, Chest, FURNACE_FUEL, FURNACE_INPUT,
    FURNACE_OUTPUT, Furnace, FurnaceState, LOADER_SLOTS, brew_progress, container_slots,
    register as register_containers,
};
pub use tech::crafting::{
    BrewRecipe, Brewing, Ingredient, LootEntry, LootTables, MachineRecipe, MachineRecipes, Recipe,
    Recipes, Smelting, SmeltingRecipe,
};
pub use tech::machines::{
    BlockBehavior, Clock, Lamp, MACHINE_TICKS_PER_SECOND, MAX_POWER, MachineAction, MachineContext,
    Machines, Scheduler, TickReport, received_power,
};
pub use tech::networks::{
    ENERGY_RATE, Endpoint, FLUID_RATE, Network, NetworkKind, NetworkStats, Networks,
};
pub use tech::recipe_index::{Choice, RecipeCategory, RecipeIndex, RecipeLayout, RecipeView};
pub use tech::storage::{
    CHEST_TIERS, CONTROLLER_KEY, CONTROLLER_MAX_CONTAINERS, CONTROLLER_RADIUS, ChestTier,
    CompactTable, Controller, FILTER_SLOTS, Fill, Group, KITS, Kit, Layout, MULTIPLIER_CAP, Policy,
    SlotKind, StorageChest, UPGRADE_PERIOD, Upgrade, Voiding, accept as storage_accept,
    chest_layout, chest_tier_of, controller_groups, controller_insert, controller_members,
    controller_room, controller_take, ensure_layout, filter_items, insert as storage_insert,
    installed as installed_upgrades, is_controller, kit_of, kit_target, layout_of, maintain,
    put_in, relayout, room_in, stack_limit, stack_multiplier, upgrade_chest,
};
pub use visibility::{FACE_DIRECTIONS, FaceConnections, VisibilityGraph};
pub use world::World;
pub use worldgen::biome::{BiomeDef, BiomeId, BiomeKind, BiomeRegistry, OreDef, PlantDef, Relief};
pub use worldgen::dimension::{DimensionDef, DimensionRegistry, OVERWORLD, SkyDef, WorldKind};
pub use worldgen::far::{
    CELLS as FAR_CELLS, FarConfig, FarDebug, FarEvent, FarMesh, FarStats, FarStreamer, FarTile,
    build_tile, build_tile_debug, build_tile_with, far_allowance, select_tiles,
    select_tiles_stable,
};
pub use worldgen::generation::{
    DataRecord, DimensionRecord, GenerationIssue, GenerationMeta, GenerationVersion, KindRecord,
};
pub use worldgen::generator::{
    Blueprint, ChestLoot, ChestLootEntry, Climate, Column, DEEPSTONE_LEVEL, FREEZING, LAVA_LEVEL,
    MAX_CHUNK_Y, MIN_CHUNK_Y, PENDING_LOOT, SEA_LEVEL, Spot, StructureDef, StructureKind,
    StructureSet, StructureSite, TerrainGenerator, TreeSite, V3Parameters, WORLD_HEIGHT_CHUNKS,
    pending_loot,
};
pub use worldgen::plants::{Precipitation, precipitation_at};
pub use worldgen::trees::{Part, Rng64, TreeBlocks, TreeKind, Wood, grow as grow_tree};
pub use worldgen::worldmap::{MapArea, MapKind, MapSamples, Raster};
pub use worldgen::worldmap_blocks::{
    BlockMapKind, BlockMapSamples, SectionAxis, SectionSpec, render_section,
};
