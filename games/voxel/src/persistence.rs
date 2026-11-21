//! Sauvegarde et chargement du monde, côté jeu.
//!
//! Le moteur (`engine_voxel::SaveStore`) sait écrire des chunks ; ici on décide quand
//! (toutes les 30 secondes et à la fermeture) et on y ajoute l'état du jeu : heure,
//! joueur, bloc choisi.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use engine_voxel::{BlockRegistry, FORMAT_VERSION, PlayerFile, PlayerSave, SaveStore, WorldMeta};
use glam::Vec3;

use crate::VoxelGame;
use crate::player::MoveMode;
use crate::players::{PlayerState, uuid_name};

pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// La racine des sauvegardes : `saves/` à côté de `assets`, ou le dossier de `VOXEL_SAVES`
/// (chemin absolu, ou relatif au dossier de lancement). Les mondes du menu, de `VOXEL_WORLD` et
/// du décor du menu principal vivent tous dedans : pointer `VOXEL_SAVES` vers un dossier
/// temporaire isole entièrement une séance de test des vraies sauvegardes.
pub fn saves_root(project_root: &Path) -> PathBuf {
    saves_root_from(std::env::var_os("VOXEL_SAVES"), project_root)
}

fn saves_root_from(variable: Option<std::ffi::OsString>, project_root: &Path) -> PathBuf {
    match variable {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => project_root.join("saves"),
    }
}

/// Le dossier d'un monde : `<racine des sauvegardes>/<nom>`. Le nom vient de `VOXEL_WORLD`
/// (« world » par défaut), la racine de [`saves_root`].
pub fn world_dir(assets: &Path) -> PathBuf {
    let name = std::env::var("VOXEL_WORLD").unwrap_or_else(|_| "world".into());
    saves_root(assets.parent().unwrap_or(Path::new("."))).join(name)
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
    /// Toutes les 30 secondes : lance une sauvegarde en tâche de fond. À chaque image :
    /// récupère le résultat de la précédente.
    pub(crate) fn autosave(&mut self) {
        let Some(store) = self.server.store.clone() else {
            return;
        };
        let polled = self.streamer_mut().poll_save();
        if let Some(result) = polled {
            let elapsed = self.server.save_started.elapsed();
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
        if self.server.last_autosave.elapsed() < AUTOSAVE_INTERVAL {
            return;
        }
        self.server.last_autosave = Instant::now();
        // `world.ron` est minuscule : on l'écrit directement, à la racine du monde.
        let meta_store = self
            .server
            .dims
            .meta_store
            .clone()
            .unwrap_or_else(|| store.clone());
        if let Err(err) = meta_store.save_meta(&self.world_meta()) {
            log::error!("sauvegarde de world.ron : {err:#}");
        }
        self.save_players();
        if let Err(err) = engine_voxel::save_grids(store.dir(), &self.server.mods.subgrids.borrow())
        {
            log::error!("sauvegarde des structures mobiles : {err:#}");
        }
        crate::carts::save(&self.server.ecs, store.dir());
        self.save_backpacks();
        self.save_quests();
        self.save_item_data();
        self.server.mods.save_world_data();
        if self.streamer_mut().begin_save() {
            self.server.save_started = Instant::now();
        }
    }
}

/// Ce que le joueur local a sauvegardé : son fichier de `players/` s'il en a un, sinon (un monde
/// d'avant les fichiers de joueurs) le joueur de `world.ron`, qui devient l'hôte. Le résultat est le
/// `WorldMeta` dont le reste du chargement se sert, avec le joueur, l'inventaire et l'aventure du
/// fichier. Un fichier illisible est signalé et on retombe sur `world.ron`.
pub fn with_player_file(
    mut meta: WorldMeta,
    root: Option<&SaveStore>,
    local_uuid: u128,
) -> WorldMeta {
    let Some(root) = root else {
        return meta;
    };
    match root.load_player(&uuid_name(local_uuid)) {
        Ok(Some(file)) => {
            log::info!("joueur « {} » relu depuis players/", file.name);
            meta.player = Some(file.player);
            // L'inventaire du fichier remplace aussi l'ancien format de `world.ron`.
            meta.items = file.items;
            meta.inventory = Vec::new();
            meta.adventure = file.adventure;
        }
        Ok(None) => {}
        Err(err) => log::error!("{err:#}"),
    }
    meta
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

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Où est un joueur et comment il va, tel qu'on l'écrit sur disque. Le joueur local regarde où
    /// regarde la caméra ; un autre, où regarde son corps.
    pub(crate) fn player_save(&self, player: &PlayerState) -> PlayerSave {
        let local = player.id == self.players.local_id();
        let (yaw, pitch) = if local {
            (player.yaw, player.pitch)
        } else {
            let yaw = self
                .ecs
                .get::<crate::components::Transform>(player.entity)
                .map_or(0.0, |t| t.yaw);
            (yaw, 0.0)
        };
        PlayerSave {
            position: crate::player::eye(&self.ecs, player.entity, 1.0).to_array(),
            yaw,
            pitch,
            mode: match crate::player::mode(&self.ecs, player.entity) {
                MoveMode::Walk => "walk".into(),
                MoveMode::Fly => "fly".into(),
            },
            selected_block: player
                .inventory
                .selected_item()
                .map(|id| self.content.items.get(id).key.clone()),
            food: Some(player.hunger.food),
            saturation: Some(player.hunger.saturation),
            spawn_point: player.bed.map(|b| b.to_array()),
            xp: Some(player.experience.points),
        }
    }

    /// Le fichier d'un joueur (`players/<identifiant>.ron`).
    pub(crate) fn player_file(&self, player: &PlayerState) -> PlayerFile {
        PlayerFile {
            name: player.name.clone(),
            player: self.player_save(player),
            items: Some(player.inventory.to_save(&self.content.items)),
            adventure: player.adventure.progress.to_save(),
        }
    }

    /// L'état du jeu à écrire dans `world.ron`. Le joueur hôte (le local) y figure aussi, pour
    /// qu'une version du jeu sans fichiers de joueurs ouvre encore le monde.
    pub(crate) fn world_meta(&self) -> WorldMeta {
        let host = self.players.me();
        WorldMeta {
            format_version: FORMAT_VERSION,
            // La graine du monde : celle du générateur de la dimension y ajoute son décalage.
            seed: self.dims.base_seed,
            time_of_day: self.day.time,
            player: Some(self.player_save(host)),
            mods: loaded_mods(&self.registry),
            inventory: Vec::new(),
            items: Some(host.inventory.to_save(&self.content.items)),
            difficulty: self.difficulty,
            // En partance pour une autre dimension : `world.ron` annonce déjà l'arrivée (le jeu se
            // sauvegarde encore une fois en se déchargeant : il ne doit pas l'effacer).
            dimension: self
                .dims
                .leaving
                .clone()
                .unwrap_or_else(|| self.dims.current.key.clone()),
            arriving: self.dims.leaving.is_some(),
            adventure: host.adventure.progress.to_save(),
            generation: Some(self.dims.generation.clone()),
        }
    }

    /// Écrit le fichier de chaque joueur dans `players/` (à la racine du monde).
    pub(crate) fn save_players(&self) {
        let Some(store) = self.dims.meta_store.clone().or_else(|| self.store.clone()) else {
            return;
        };
        for player in self.players.iter().filter(|p| !p.ghost) {
            if let Err(err) = store.save_player(&uuid_name(player.uuid), &self.player_file(player))
            {
                log::error!("sauvegarde du joueur {} : {err:#}", player.name);
            }
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
        self.save_players();
        if let Err(err) = engine_voxel::save_grids(store.dir(), &self.mods.subgrids.borrow()) {
            log::error!("sauvegarde des structures mobiles : {err:#}");
        }
        crate::carts::save(&self.ecs, store.dir());
        self.save_backpacks();
        self.save_quests();
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

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("voxel-persistence-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Le `world.ron` d'un monde d'avant les joueurs : l'hôte y est à (1, 2, 3), 5 de nourriture.
    fn old_meta() -> WorldMeta {
        ron::from_str(
            r#"(format_version: 4, seed: 9, time_of_day: 0.5, mods: ["core"],
                player: Some((position: (1.0, 2.0, 3.0), yaw: 0.0, pitch: 0.0, mode: "walk",
                              selected_block: None, food: Some(5.0))))"#,
        )
        .unwrap()
    }

    fn file_at(name: &str, x: f32) -> PlayerFile {
        PlayerFile {
            name: name.into(),
            player: PlayerSave {
                position: [x, 80.0, 0.0],
                yaw: 0.0,
                pitch: 0.0,
                mode: "walk".into(),
                selected_block: None,
                food: Some(20.0),
                saturation: None,
                spawn_point: None,
                xp: None,
            },
            items: None,
            adventure: Default::default(),
        }
    }

    #[test]
    fn a_world_without_player_files_gives_its_player_to_the_host() {
        let store = SaveStore::open(temp("old")).unwrap();
        assert!(
            !store.players_dir().exists(),
            "un monde d'avant n'a pas de players/"
        );
        let loaded = with_player_file(old_meta(), Some(&store), 0xA11CE);
        assert_eq!(
            loaded,
            old_meta(),
            "rien ne change : le joueur vient de world.ron"
        );
        assert_eq!(
            saved_eye(Some(&loaded)).unwrap().0,
            Vec3::new(1.0, 2.0, 3.0)
        );
        // Lire un ancien monde n'écrit rien : on ne touche pas aux mondes avant leur sauvegarde.
        assert!(!store.players_dir().exists());
    }

    #[test]
    fn the_host_is_read_from_their_own_file_and_not_from_a_guest_file() {
        let store = SaveStore::open(temp("two")).unwrap();
        store
            .save_player(&uuid_name(0xA11CE), &file_at("Alice", 100.0))
            .unwrap();
        store
            .save_player(&uuid_name(0xB0B), &file_at("Bob", -100.0))
            .unwrap();
        let alice = with_player_file(old_meta(), Some(&store), 0xA11CE);
        let bob = with_player_file(old_meta(), Some(&store), 0xB0B);
        assert_eq!(saved_eye(Some(&alice)).unwrap().0.x, 100.0);
        assert_eq!(saved_eye(Some(&bob)).unwrap().0.x, -100.0);
        // Un nouveau venu : pas de fichier, donc le joueur de `world.ron` (le départ de l'hôte).
        let newcomer = with_player_file(old_meta(), Some(&store), 0xC0FFEE);
        assert_eq!(saved_eye(Some(&newcomer)).unwrap().0.x, 1.0);
    }

    #[test]
    fn an_unreadable_player_file_falls_back_to_world_ron() {
        let store = SaveStore::open(temp("broken")).unwrap();
        std::fs::create_dir_all(store.players_dir()).unwrap();
        std::fs::write(
            store.players_dir().join(format!("{}.ron", uuid_name(7))),
            "???",
        )
        .unwrap();
        let loaded = with_player_file(old_meta(), Some(&store), 7);
        assert_eq!(saved_eye(Some(&loaded)).unwrap().0.x, 1.0);
    }

    #[test]
    fn world_dir_is_next_to_assets() {
        // Sans `VOXEL_WORLD` ni `VOXEL_SAVES` dans l'environnement du test, les valeurs par défaut.
        if std::env::var("VOXEL_WORLD").is_err() && std::env::var("VOXEL_SAVES").is_err() {
            let dir = world_dir(Path::new("/jeu/assets"));
            assert_eq!(dir, PathBuf::from("/jeu/saves/world"));
        }
    }

    #[test]
    fn saves_root_follows_voxel_saves_only_when_it_is_set() {
        let project = Path::new("/jeu");
        assert_eq!(saves_root_from(None, project), PathBuf::from("/jeu/saves"));
        assert_eq!(
            saves_root_from(Some("".into()), project),
            PathBuf::from("/jeu/saves"),
            "une variable vide ne déplace rien"
        );
        assert_eq!(
            saves_root_from(Some("/tmp/essai".into()), project),
            PathBuf::from("/tmp/essai")
        );
    }
}
