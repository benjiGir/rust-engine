//! Ce que la simulation et la fenêtre partagent à la racine : les dossiers du projet et des assets, le
//! registre des blocs, les lecteurs d'événements, quelques réglages par variable d'environnement.

use crate::events;
use crate::events::{BlockBroken, BlockPlaced, EntityDied, PlayerDamaged};
use anyhow::{Context as _, Result};
use engine_ecs::{EventReader, Events};
use engine_voxel::BlockRegistry;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_SEED: u32 = 42;

/// Hauteur de vol au départ et en mode `VOXEL_AUTOFLY`, au-dessus du sol.
pub(crate) const FLY_ALTITUDE: f32 = 30.0;

/// La racine du projet (le dossier qui contient `assets/` et `saves/`).
pub(crate) fn project_root() -> PathBuf {
    assets_dir()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Les curseurs du jeu sur ses propres événements.
pub(crate) struct EventReaders {
    pub(crate) broken: EventReader,
    pub(crate) placed: EventReader,
    pub(crate) died: EventReader,
    pub(crate) damaged: EventReader,
    pub(crate) attacked: EventReader,
    /// Les morts de créatures, pour leur butin (`died` sert aux sons).
    pub(crate) loot: EventReader,
}

impl EventReaders {
    pub(crate) fn new(ecs: &engine_ecs::World) -> Self {
        Self {
            broken: ecs.resource::<Events<BlockBroken>>().reader(),
            placed: ecs.resource::<Events<BlockPlaced>>().reader(),
            died: ecs.resource::<Events<EntityDied>>().reader(),
            damaged: ecs.resource::<Events<PlayerDamaged>>().reader(),
            attacked: ecs.resource::<Events<events::PlayerAttacked>>().reader(),
            loot: ecs.resource::<Events<EntityDied>>().reader(),
        }
    }
}

/// Lit le registre des blocs : `assets/core` puis les mods de `mods/`.
pub(crate) fn load_registry(mods: &[(String, PathBuf)]) -> Result<BlockRegistry> {
    let assets = assets_dir();
    let registry = BlockRegistry::load_mods(&assets, mods)
        .with_context(|| format!("chargement des blocs depuis {}", assets.display()))?;
    let keys: Vec<&str> = registry
        .iter()
        .skip(1)
        .map(|(_, def)| def.key.as_str())
        .collect();
    log::info!("{} blocs chargés : {}", keys.len(), keys.join(", "));
    Ok(registry)
}

/// Le registre complet (core et mods, dans l'ordre), pour les tests.
#[cfg(test)]
pub(crate) fn test_registry() -> BlockRegistry {
    load_registry(&crate::modding::mod_dirs(&crate::modding::resolve_order(
        &assets_dir(),
    )))
    .unwrap()
}

/// `VOXEL_ASSETS`, sinon `./assets`, sinon le dossier du dépôt (connu à la compilation).
pub(crate) fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VOXEL_ASSETS") {
        return dir.into();
    }
    let local = PathBuf::from("assets");
    if local.is_dir() {
        return local;
    }
    // `env!` lit une variable au moment de la compilation : ici, le dossier de cette crate.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Lit une variable d'environnement et la convertit (`u32`, `i32`...).
/// `T: FromStr` : n'importe quel type qui sait se lire depuis du texte.
pub(crate) fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}
