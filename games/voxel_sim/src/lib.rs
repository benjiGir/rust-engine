//! La simulation du jeu voxel, sans fenêtre (lot 8 du plan multijoueur) : le serveur et ce qu'il
//! fait tourner (monde, ECS, joueurs, créatures, mods, quêtes, aventure), le protocole et le réseau,
//! les bots et le serveur dédié. Elle ne dépend d'aucune bibliothèque d'affichage (règle `HEADLESS` de
//! check-arch) : le jeu (`games/voxel`) l'appelle par `voxel_sim::` et y ajoute la fenêtre.
//!
//! Les modules sont rangés par domaine (voir `docs/plan-rangement.md`) : `server/`, `net/`, `world/`,
//! `player/`, `mobs/`, `enchanting/`, `adventure/`, `quests/`, `storage/`, `vehicles/`, `shaping/`,
//! `modding/` ; les mêmes noms rangent les écrans côté fenêtre.

pub mod adventure;
pub mod common;
pub mod components;
pub mod content;
pub mod enchanting;
pub mod events;
pub mod launch;
pub mod logs;
pub mod mobs;
pub mod modding;
pub mod net;
pub mod paths;
pub mod player;
pub mod quests;
pub mod server;
pub mod settings;
pub mod shaping;
pub mod shared;
pub mod storage;
pub mod vehicles;
pub mod world;

#[cfg(any(test, feature = "testing"))]
pub use common::test_registry;
pub use common::{
    DEFAULT_SEED, EventReaders, FLY_ALTITUDE, assets_dir, env_parse, load_registry, project_root,
};
pub use components::{Collider, Transform};
pub use events::BlockPlaced;
pub use launch::{Launch, Request};
pub use mobs::MobRegistry;
pub use player::{FLY_SPEED, FLY_SPRINT_FACTOR, MoveMode, SPRINT_SPEED, WALK_SPEED};
pub use server::VoxelServer;
pub use server::stats::FrameStats;
pub use settings::Action;
pub use shared::StreamerExt;
pub use world::daytime::DayCycle;
