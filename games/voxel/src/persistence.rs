//! Sauvegarde et chargement du monde, côté jeu.
//!
//! Le moteur (`engine_voxel::SaveStore`) sait écrire des chunks ; ici on décide quand
//! (toutes les 30 secondes et à la fermeture) et on y ajoute l'état du jeu : heure,
//! joueur, bloc choisi.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use engine_voxel::{BlockRegistry, FORMAT_VERSION, PlayerSave, SaveStore, WorldMeta};
use glam::Vec3;

use crate::VoxelGame;
use crate::player::MoveMode;

pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Le dossier d'un monde : `saves/<nom>` à côté de `assets`. Le nom vient de
/// `VOXEL_WORLD` (« world » par défaut).
pub fn world_dir(assets: &Path) -> PathBuf {
    let name = std::env::var("VOXEL_WORLD").unwrap_or_else(|_| "world".into());
    assets
        .parent()
        .unwrap_or(Path::new("."))
        .join("saves")
        .join(name)
}

/// Ouvre le monde et lit `world.ron` s'il existe.
pub fn open_world(dir: PathBuf) -> Result<(Arc<SaveStore>, Option<WorldMeta>)> {
    let store = SaveStore::open(dir)?;
    let meta = store.load_meta()?;
    match &meta {
        Some(meta) => log::info!(
            "monde chargé depuis {} (graine {})",
            store.dir().display(),
            meta.seed
        ),
        None => log::info!("nouveau monde dans {}", store.dir().display()),
    }
    Ok((Arc::new(store), meta))
}

/// Les espaces de noms chargés (`core`, puis les mods).
pub fn loaded_mods(registry: &BlockRegistry) -> Vec<String> {
    let mut mods: Vec<String> = Vec::new();
    for (_, def) in registry.iter() {
        if let Some((namespace, _)) = def.key.split_once(':')
            && !mods.iter().any(|m| m == namespace)
        {
            mods.push(namespace.to_string());
        }
    }
    mods
}

/// Prévient si un mod utilisé par la sauvegarde n'est plus installé.
pub fn check_mods(meta: &WorldMeta, registry: &BlockRegistry) {
    let loaded = loaded_mods(registry);
    for missing in meta.mods.iter().filter(|m| !loaded.contains(m)) {
        log::warn!(
            "le monde utilisait le mod « {missing} », absent : ses blocs seront « inconnus »"
        );
    }
}

impl VoxelGame {
    /// L'état du jeu à écrire dans `world.ron`.
    pub(crate) fn world_meta(&self) -> WorldMeta {
        let selected = self
            .inventory
            .selected_item()
            .map(|id| self.content.items.get(id).key.clone());
        WorldMeta {
            format_version: FORMAT_VERSION,
            // La graine du monde : celle du générateur de la dimension y ajoute son décalage.
            seed: self.dims.base_seed,
            time_of_day: self.day.time,
            player: Some(PlayerSave {
                position: crate::player::eye(&self.ecs, self.player, 1.0).to_array(),
                yaw: self.camera.yaw,
                pitch: self.camera.pitch,
                mode: match crate::player::mode(&self.ecs, self.player) {
                    MoveMode::Walk => "walk".into(),
                    MoveMode::Fly => "fly".into(),
                },
                selected_block: selected,
                food: Some(self.hunger.food),
                saturation: Some(self.hunger.saturation),
                spawn_point: self.bed.map(|b| b.to_array()),
                xp: Some(self.experience.points),
            }),
            mods: loaded_mods(&self.registry),
            inventory: Vec::new(),
            items: Some(self.inventory.to_save(&self.content.items)),
            difficulty: self.difficulty,
            // En partance pour une autre dimension : `world.ron` annonce déjà l'arrivée (le jeu se
            // sauvegarde encore une fois en se déchargeant : il ne doit pas l'effacer).
            dimension: self
                .dims
                .leaving
                .clone()
                .unwrap_or_else(|| self.dims.current.key.clone()),
            arriving: self.dims.leaving.is_some(),
        }
    }

    /// Toutes les 30 secondes : lance une sauvegarde en tâche de fond. À chaque image :
    /// récupère le résultat de la précédente.
    pub(crate) fn autosave(&mut self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let polled = self.streamer_mut().poll_save();
        if let Some(result) = polled {
            let elapsed = self.save_started.elapsed();
            match result {
                Ok(report) => log::info!(
                    "sauvegarde : {} chunks, {} régions, {:.1} Ko, en {:.0?} (en tâche de fond)",
                    report.chunks,
                    report.regions,
                    report.bytes as f64 / 1024.0,
                    elapsed
                ),
                Err(err) => log::error!("sauvegarde échouée : {err:#}"),
            }
        }
        if self.last_autosave.elapsed() < AUTOSAVE_INTERVAL {
            return;
        }
        self.last_autosave = Instant::now();
        // `world.ron` est minuscule : on l'écrit directement, à la racine du monde.
        let meta_store = self
            .dims
            .meta_store
            .clone()
            .unwrap_or_else(|| store.clone());
        if let Err(err) = meta_store.save_meta(&self.world_meta()) {
            log::error!("sauvegarde de world.ron : {err:#}");
        }
        if let Err(err) = engine_voxel::save_grids(store.dir(), &self.mods.subgrids.borrow()) {
            log::error!("sauvegarde des structures mobiles : {err:#}");
        }
        crate::carts::save(&self.ecs, store.dir());
        self.save_backpacks();
        self.save_item_data();
        self.mods.save_world_data();
        if self.streamer_mut().begin_save() {
            self.save_started = Instant::now();
        }
    }

    /// Sauvegarde complète, en attendant la fin (à la fermeture du jeu).
    pub(crate) fn save_now(&mut self) {
        let _span = engine_trace::span("sauvegarde complète");
        let Some(store) = self.store.clone() else {
            return;
        };
        let start = Instant::now();
        let meta_store = self
            .dims
            .meta_store
            .clone()
            .unwrap_or_else(|| store.clone());
        let meta = meta_store.save_meta(&self.world_meta());
        if let Err(err) = engine_voxel::save_grids(store.dir(), &self.mods.subgrids.borrow()) {
            log::error!("sauvegarde des structures mobiles : {err:#}");
        }
        crate::carts::save(&self.ecs, store.dir());
        self.save_backpacks();
        self.save_item_data();
        self.mods.save_world_data();
        let saved = self.streamer_mut().save_blocking();
        match (meta, saved) {
            (Ok(()), Ok(report)) => log::info!(
                "monde sauvegardé dans {} : {} chunks, {:.1} Ko, en {:.0?}",
                store.dir().display(),
                report.chunks,
                report.bytes as f64 / 1024.0,
                start.elapsed()
            ),
            (Err(err), _) | (_, Err(err)) => log::error!("sauvegarde échouée : {err:#}"),
        }
    }
}

/// La position des yeux sauvegardée, ou `None`.
pub fn saved_eye(meta: Option<&WorldMeta>) -> Option<(Vec3, f32, f32, MoveMode)> {
    let player = meta?.player.as_ref()?;
    let mode = if player.mode == "fly" {
        MoveMode::Fly
    } else {
        MoveMode::Walk
    };
    Some((Vec3::from(player.position), player.yaw, player.pitch, mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mods_are_listed_once_in_load_order() {
        let mut registry = BlockRegistry::builtin().as_ref().clone();
        registry
            .add_source(
                "example",
                r#"[(id: "example:ruby", name: "R", color: (1.0, 0.0, 0.0))]"#,
                "t",
            )
            .unwrap();
        assert_eq!(loaded_mods(&registry), ["core", "example"]);
    }

    #[test]
    fn world_dir_is_next_to_assets() {
        // Sans `VOXEL_WORLD` dans l'environnement du test, le nom par défaut.
        if std::env::var("VOXEL_WORLD").is_err() {
            let dir = world_dir(Path::new("/jeu/assets"));
            assert_eq!(dir, PathBuf::from("/jeu/saves/world"));
        }
    }
}
