// GÉNÉRÉ par `cargo xtask hooks` depuis `crates/mod_sdk/hooks.ron` : ne pas modifier.
//! Les numéros de l'interface des mods, côté jeu.

/// Les numéros des événements (`on_event`).
/// BlockBroken : un bloc a été cassé.
pub const EVENT_BLOCK_BROKEN: i32 = 1;
/// BlockPlaced : un bloc a été posé.
pub const EVENT_BLOCK_PLACED: i32 = 2;
/// EntityDied : une créature est morte.
pub const EVENT_ENTITY_DIED: i32 = 3;
/// Tick : un tick de machines (20 par seconde).
pub const EVENT_TICK: i32 = 4;
/// PlayerDamaged : le joueur a perdu de la vie.
pub const EVENT_PLAYER_DAMAGED: i32 = 5;
/// ChunkGenerated : un chunk non vide vient d'être généré pour la première fois (pas relu d'une sauvegarde) : le mod peut le décorer.
pub const EVENT_CHUNK_GENERATED: i32 = 6;

/// Les numéros des appels de machines (`on_machines`).
/// Placed : elles viennent d'être posées.
pub const MACHINES_PLACED: i32 = 0;
/// Loaded : leur chunk vient d'être chargé (les ticks programmés ne sont pas sauvegardés).
pub const MACHINES_LOADED: i32 = 1;
/// NeighborChanged : un voisin a changé.
pub const MACHINES_NEIGHBOR: i32 = 2;
/// ScheduledTick : leur tick programmé est arrivé.
pub const MACHINES_TICK: i32 = 3;
