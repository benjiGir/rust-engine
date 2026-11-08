//! Le rechargement à chaud : modifier un shader, une texture, un son ou un `blocks.ron`
//! met le jeu à jour sans le relancer.
//!
//! `notify` demande au système d'exploitation de nous prévenir quand un fichier change.
//! Ses notifications arrivent sur un thread à lui : il nous les passe par un **canal**,
//! et le thread principal les lit à chaque image, sans jamais attendre (`try_recv`).
//!
//! Un éditeur enregistre souvent en plusieurs étapes (fichier temporaire, renommage...) :
//! on attend que les notifications se calment 200 ms avant de recharger, pour ne le
//! faire qu'une fois et sur un fichier complet (« debounce »).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, unbounded};
use engine_voxel::BlockRegistry;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

const QUIET_TIME: Duration = Duration::from_millis(200);

/// Ce qu'il faut recharger.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changes {
    pub shader: bool,
    pub textures: bool,
    pub blocks: bool,
    pub sounds: bool,
}

impl Changes {
    pub const ALL: Self = Self {
        shader: true,
        textures: true,
        blocks: true,
        sounds: true,
    };

    pub fn any(self) -> bool {
        self.shader || self.textures || self.blocks || self.sounds
    }

    /// Le type de fichier décide de ce qu'il faut recharger.
    pub fn from_paths<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> Self {
        let mut changes = Self::default();
        for path in paths {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            match path.extension().and_then(|e| e.to_str()) {
                Some("wgsl") => changes.shader = true,
                Some("png") => changes.textures = true,
                Some("wav" | "ogg") => changes.sounds = true,
                Some("ron") if name == "blocks.ron" => changes.blocks = true,
                _ => {}
            }
        }
        changes
    }
}

pub struct HotReload {
    /// Le surveillant doit rester en vie : s'il est détruit (`Drop`), la surveillance s'arrête.
    _watcher: RecommendedWatcher,
    receiver: Receiver<PathBuf>,
    pending: HashSet<PathBuf>,
    last_event: Instant,
}

impl HotReload {
    /// Surveille ces dossiers (et leurs sous-dossiers).
    pub fn new(dirs: &[PathBuf]) -> Result<Self> {
        let (sender, receiver) = unbounded();
        // La closure tourne sur le thread de `notify` : elle doit être `Send`.
        // Elle ne fait qu'envoyer les chemins dans le canal.
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    for path in event.paths {
                        let _ = sender.send(path);
                    }
                }
            })?;
        for dir in dirs.iter().filter(|d| d.exists()) {
            watcher.watch(dir, RecursiveMode::Recursive)?;
            log::info!("rechargement à chaud : surveillance de {}", dir.display());
        }
        Ok(Self {
            _watcher: watcher,
            receiver,
            pending: HashSet::new(),
            last_event: Instant::now(),
        })
    }

    /// À chaque image : renvoie ce qu'il faut recharger, une fois les fichiers stables.
    pub fn poll(&mut self) -> Option<Changes> {
        while let Ok(path) = self.receiver.try_recv() {
            self.pending.insert(path);
            self.last_event = Instant::now();
        }
        if self.pending.is_empty() || self.last_event.elapsed() < QUIET_TIME {
            return None;
        }
        let changes = Changes::from_paths(&self.pending);
        self.pending.clear();
        changes.any().then_some(changes)
    }
}

/// Le résultat d'un rechargement de `blocks.ron`.
pub enum RegistryReload {
    /// Le nouveau registre, prêt à remplacer l'ancien.
    Applied(BlockRegistry),
    /// La liste des blocs a changé : les numéros des chunks ne correspondraient plus.
    Incompatible(String),
    /// Fichier invalide : on garde l'ancien registre.
    Failed(anyhow::Error),
}

/// Relit les fichiers de blocs. On n'accepte que des changements de propriétés : mêmes
/// blocs, dans le même ordre. Ajouter ou retirer un bloc demande de relancer le jeu.
pub fn reload_registry(
    current: &BlockRegistry,
    assets: &Path,
    mods: Option<&Path>,
) -> RegistryReload {
    let fresh = match BlockRegistry::load(assets, mods) {
        Ok(fresh) => fresh,
        Err(err) => return RegistryReload::Failed(err),
    };
    let keys = |r: &BlockRegistry| r.iter().map(|(_, d)| d.key.clone()).collect::<Vec<_>>();
    if keys(current) != keys(&fresh) {
        return RegistryReload::Incompatible(
            "la liste des blocs a changé : relancez le jeu pour l'appliquer".into(),
        );
    }
    RegistryReload::Applied(fresh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_types_decide_what_to_reload() {
        let paths = [
            PathBuf::from("crates/engine_render/src/chunk_shader.wgsl"),
            PathBuf::from("assets/core/textures/dirt.png"),
            PathBuf::from("assets/core/notes.txt"),
        ];
        let changes = Changes::from_paths(&paths);
        assert_eq!(
            changes,
            Changes {
                shader: true,
                textures: true,
                blocks: false,
                sounds: false
            }
        );
        let ron = [
            PathBuf::from("mods/example/blocks.ron"),
            PathBuf::from("saves/world/world.ron"),
        ];
        let changes = Changes::from_paths(&ron);
        assert!(
            changes.blocks && !changes.shader,
            "world.ron n'est pas un fichier de blocs"
        );
    }

    fn temp_assets(name: &str, core_blocks: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("voxel-reload-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets/core")).unwrap();
        std::fs::write(root.join("assets/core/blocks.ron"), core_blocks).unwrap();
        root
    }

    #[test]
    fn invalid_blocks_file_keeps_the_old_registry() {
        let current = BlockRegistry::builtin();
        let root = temp_assets("invalid", "[ (id: \"core:stone\", name: ");
        match reload_registry(&current, &root.join("assets"), None) {
            RegistryReload::Failed(err) => assert!(format!("{err:#}").contains("RON")),
            _ => panic!("un fichier invalide doit être refusé"),
        }
    }

    #[test]
    fn property_changes_apply_but_new_blocks_need_a_restart() {
        let source = include_str!("../../../assets/core/blocks.ron");
        let current =
            BlockRegistry::load(&temp_assets("base", source).join("assets"), None).unwrap();

        let recoloured = source.replace("(0.5, 0.5, 0.52)", "(0.9, 0.1, 0.1)");
        let root = temp_assets("recoloured", &recoloured);
        match reload_registry(&current, &root.join("assets"), None) {
            RegistryReload::Applied(fresh) => {
                let stone = fresh.id("core:stone").unwrap();
                assert_eq!(fresh.def(stone).color, [0.9, 0.1, 0.1]);
            }
            _ => panic!("un changement de couleur doit s'appliquer"),
        }

        let added = source.replacen(
            '[',
            r#"[(id: "core:new", name: "N", color: (0.0, 0.0, 0.0)),"#,
            1,
        );
        let root = temp_assets("added", &added);
        assert!(matches!(
            reload_registry(&current, &root.join("assets"), None),
            RegistryReload::Incompatible(_)
        ));
    }
}
