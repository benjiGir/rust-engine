//! Où le jeu trouve ses fichiers (voir `docs/plan-installation.md`).
//!
//! Deux points suffisent : **la racine**, le dossier où le joueur range ce qu'il ajoute et ce que le
//! jeu écrit (mods, packs, mondes, réglages), et **le jeu de base** (blocs, objets, textures, sons du
//! cœur), livré et jamais modifié. Les autres chemins s'en déduisent ([`GamePaths`]).
//!
//! La racine se cherche dans cet ordre :
//!
//! 1. `--data-dir <dossier>` (passé à [`init`]) ou `VOXEL_DATA` : tout ce qui s'écrit et s'ajoute est là
//!    (plusieurs serveurs sur une machine, des essais) ; le jeu de base reste celui de l'installation ;
//! 2. le dossier de l'exécutable, s'il contient `game/` : c'est une installation (pour un `.app` macOS,
//!    le dossier qui contient l'application) ;
//! 3. le dépôt, en développement : le parent de `assets/` (`VOXEL_ASSETS`, sinon `./assets`, sinon le
//!    dépôt où le jeu a été compilé).
//!
//! **Une installation en lecture seule** (Program Files, /Applications) : la racine devient le dossier
//! utilisateur du système (`%APPDATA%\Voxel`, `~/Library/Application Support/Voxel`,
//! `~/.local/share/voxel`) ; les mods et les packs livrés restent lus dans l'installation
//! ([`GamePaths::shipped`]), après ceux du joueur.
//!
//! `VOXEL_ASSETS`, `VOXEL_SAVES` et `VOXEL_SERVER_CONFIG` restent des surcharges ponctuelles du jeu de
//! base, des mondes et de la configuration du serveur.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Le nom du dossier du jeu de base dans une installation (`assets/` dans le dépôt).
pub const GAME_DIR: &str = "game";

/// Comment la racine a été trouvée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `--data-dir` ou `VOXEL_DATA`.
    DataDir,
    /// Le dossier de l'exécutable (une installation).
    Install,
    /// Le dossier utilisateur du système : l'installation est en lecture seule.
    UserDir,
    /// Le dépôt (développement).
    Repository,
}

/// Les dossiers et fichiers du jeu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePaths {
    pub layout: Layout,
    /// Ce que le joueur ajoute et ce que le jeu écrit.
    pub root: PathBuf,
    /// Le jeu de base (`game/` installé, `assets/` dans le dépôt).
    pub game: PathBuf,
    pub mods: PathBuf,
    pub texture_packs: PathBuf,
    pub shader_packs: PathBuf,
    pub saves: PathBuf,
    /// Les captures d'écran (F2).
    pub screenshots: PathBuf,
    /// Le journal du dernier lancement (`latest.log`) et les précédents.
    pub logs: PathBuf,
    pub settings: PathBuf,
    pub mod_settings: PathBuf,
    pub server_config: PathBuf,
    /// L'installation, quand la racine est ailleurs parce qu'elle est en lecture seule : ses mods et
    /// ses packs livrés sont lus après ceux de la racine.
    pub shipped: Option<PathBuf>,
}

impl GamePaths {
    /// `name` dans la racine, puis dans l'installation s'il y a lieu : le premier passe devant.
    fn layered(&self, name: &str) -> Vec<PathBuf> {
        std::iter::once(&self.root)
            .chain(&self.shipped)
            .map(|dir| dir.join(name))
            .collect()
    }

    /// Le jeu tourne-t-il depuis le dépôt (`cargo run`, avec ou sans `--data-dir`) ? Seulement alors
    /// les sources du moteur (shaders) se relisent à chaud : une installation embarque les siennes.
    pub fn from_repository(&self) -> bool {
        !matches!(self.layout, Layout::Install | Layout::UserDir)
            && self.game.file_name().is_some_and(|name| name != GAME_DIR)
    }

    /// Les dossiers de mods, celui du joueur d'abord.
    pub fn mods_dirs(&self) -> Vec<PathBuf> {
        self.layered("mods")
    }

    /// Les dossiers de packs de textures, celui du joueur d'abord.
    pub fn texture_pack_dirs(&self) -> Vec<PathBuf> {
        self.layered("texture_packs")
    }

    /// Les dossiers de packs de shaders, celui du joueur d'abord.
    pub fn shader_pack_dirs(&self) -> Vec<PathBuf> {
        self.layered("shader_packs")
    }
}

/// Ce qui oriente la recherche : rassemblé à part pour que les tests la rejouent sans toucher à
/// l'environnement du processus.
#[derive(Debug, Clone, Default)]
pub struct Clues {
    /// `--data-dir` ou `VOXEL_DATA`.
    pub data_dir: Option<PathBuf>,
    /// L'exécutable en cours.
    pub exe: Option<PathBuf>,
    /// `VOXEL_ASSETS`.
    pub assets: Option<PathBuf>,
    /// Le dossier de lancement.
    pub cwd: Option<PathBuf>,
    /// Le dépôt où le jeu a été compilé.
    pub repository: PathBuf,
    /// `VOXEL_SAVES`.
    pub saves: Option<PathBuf>,
    /// `VOXEL_SERVER_CONFIG`.
    pub server_config: Option<PathBuf>,
    /// Le dossier utilisateur du système, pour une installation en lecture seule.
    pub user_dir: Option<PathBuf>,
}

impl Clues {
    /// Les indices de ce processus.
    pub fn from_env(data_dir: Option<PathBuf>) -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            data_dir: data_dir.or_else(|| var("VOXEL_DATA")),
            exe: std::env::current_exe().ok(),
            assets: var("VOXEL_ASSETS"),
            cwd: std::env::current_dir().ok(),
            repository: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
            saves: var("VOXEL_SAVES"),
            server_config: var("VOXEL_SERVER_CONFIG"),
            user_dir: user_dir(&var),
        }
    }
}

/// Le dossier où un programme range les données de l'utilisateur, selon le système.
fn user_dir(var: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(windows) {
        var("APPDATA").map(|dir| dir.join("Voxel"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support/Voxel"))
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".local/share")))
            .map(|dir| dir.join("voxel"))
    }
}

/// Peut-on écrire dans ce dossier ? On essaie : les droits seuls ne disent pas tout (disque monté en
/// lecture seule, application macOS en quarantaine...).
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".voxel-ecriture-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Le dossier d'installation qui contient l'exécutable : le premier de ses parents (quatre au plus,
/// pour sortir d'un `Voxel.app/Contents/MacOS`) qui a un dossier `game/`.
fn install_root(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .take(4)
        .find(|dir| dir.join(GAME_DIR).is_dir())
        .map(Path::to_path_buf)
}

/// Le jeu de base en développement : `VOXEL_ASSETS`, sinon `assets/` du dossier de lancement, sinon
/// celui du dépôt.
fn repository_game(clues: &Clues) -> PathBuf {
    if let Some(assets) = &clues.assets {
        return assets.clone();
    }
    if let Some(local) = clues.cwd.as_ref().map(|cwd| cwd.join("assets"))
        && local.is_dir()
    {
        return local;
    }
    clues.repository.join("assets")
}

/// Trouve la racine et le jeu de base, puis en déduit le reste.
pub fn locate(clues: &Clues) -> GamePaths {
    let install = clues.exe.as_deref().and_then(install_root);
    // Le jeu de base : `VOXEL_ASSETS` d'abord, puis l'installation, puis le dépôt.
    let game = match (&clues.assets, &install) {
        (Some(assets), _) => assets.clone(),
        (None, Some(install)) => install.join(GAME_DIR),
        (None, None) => repository_game(clues),
    };
    let (layout, root) = match (&clues.data_dir, &install) {
        (Some(data), _) => (Layout::DataDir, data.clone()),
        (None, Some(install)) if clues.assets.is_none() => match &clues.user_dir {
            Some(user) if !writable(install) => (Layout::UserDir, user.clone()),
            _ => (Layout::Install, install.clone()),
        },
        _ => (
            Layout::Repository,
            game.parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        ),
    };
    let shipped = (layout == Layout::UserDir)
        .then(|| install.clone())
        .flatten();
    GamePaths {
        layout,
        shipped,
        mods: root.join("mods"),
        texture_packs: root.join("texture_packs"),
        shader_packs: root.join("shader_packs"),
        saves: clues.saves.clone().unwrap_or_else(|| root.join("saves")),
        screenshots: root.join("screenshots"),
        logs: root.join("logs"),
        settings: root.join("settings.ron"),
        mod_settings: root.join("mod_settings.ron"),
        server_config: clues
            .server_config
            .clone()
            .unwrap_or_else(|| root.join("server.ron")),
        game,
        root,
    }
}

static PATHS: OnceLock<GamePaths> = OnceLock::new();

/// Fixe les chemins du processus, au tout début de `main` (`--data-dir`), avant le journal (qui
/// s'écrit dans `logs/`) : [`crate::logs::init`] dit ensuite où ils sont. Sans appel, [`get`] les
/// cherche d'après l'environnement. Rend les chemins retenus (ceux d'un appel précédent s'il y en a eu).
pub fn init(data_dir: Option<PathBuf>) -> &'static GamePaths {
    PATHS.get_or_init(|| {
        let paths = locate(&Clues::from_env(data_dir));
        // Le dossier utilisateur n'existe pas forcément encore : le jeu y écrit dès le lancement.
        if paths.layout == Layout::UserDir {
            let _ = std::fs::create_dir_all(&paths.root);
        }
        paths
    })
}

/// Les chemins du processus.
pub fn get() -> &'static GamePaths {
    init(None)
}

/// `--data-dir <dossier>` dans les arguments, s'il y est.
pub fn data_dir_arg(args: &[String]) -> Option<PathBuf> {
    let at = args.iter().position(|a| a == "--data-dir")?;
    args.get(at + 1).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un dossier temporaire neuf pour un test.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voxel-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn repo_clues(repository: &Path) -> Clues {
        Clues {
            repository: repository.to_path_buf(),
            ..Clues::default()
        }
    }

    #[test]
    fn an_executable_next_to_game_is_an_installation() {
        let install = temp("install");
        std::fs::create_dir_all(install.join("game/core")).unwrap();
        let clues = Clues {
            exe: Some(install.join("voxel")),
            ..repo_clues(Path::new("/depot"))
        };
        let paths = locate(&clues);
        assert_eq!(paths.layout, Layout::Install);
        assert_eq!(paths.root, install);
        assert_eq!(paths.game, install.join("game"));
        assert_eq!(paths.mods, install.join("mods"));
        assert_eq!(paths.saves, install.join("saves"));
        assert_eq!(paths.settings, install.join("settings.ron"));
        assert_eq!(paths.server_config, install.join("server.ron"));
        assert_eq!(paths.shader_packs, install.join("shader_packs"));
        assert!(!paths.from_repository());
        assert_eq!(paths.screenshots, install.join("screenshots"));
        assert_eq!(paths.logs, install.join("logs"));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn a_macos_application_finds_the_folder_that_contains_it() {
        let install = temp("app");
        std::fs::create_dir_all(install.join("game")).unwrap();
        let exe = install.join("Voxel.app/Contents/MacOS/voxel");
        let paths = locate(&Clues {
            exe: Some(exe),
            ..repo_clues(Path::new("/depot"))
        });
        assert_eq!(
            (paths.layout, paths.root.clone()),
            (Layout::Install, install.clone())
        );
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn a_data_dir_holds_what_is_written_and_keeps_the_installed_game() {
        let install = temp("data-install");
        std::fs::create_dir_all(install.join("game")).unwrap();
        let data = temp("data");
        let paths = locate(&Clues {
            data_dir: Some(data.clone()),
            exe: Some(install.join("voxel-server")),
            ..repo_clues(Path::new("/depot"))
        });
        assert_eq!(paths.layout, Layout::DataDir);
        assert_eq!(paths.game, install.join("game"));
        assert_eq!(paths.saves, data.join("saves"));
        assert_eq!(paths.mods, data.join("mods"));
        assert_eq!(paths.server_config, data.join("server.ron"));
        let _ = std::fs::remove_dir_all(&install);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn without_installation_the_repository_is_the_root() {
        // L'exécutable de `cargo run` est dans `target/` : pas de `game/` autour.
        let paths = locate(&Clues {
            exe: Some(PathBuf::from("/depot/target/debug/voxel")),
            ..repo_clues(Path::new("/depot"))
        });
        assert_eq!(paths.layout, Layout::Repository);
        assert_eq!(paths.game, PathBuf::from("/depot/assets"));
        assert_eq!(paths.root, PathBuf::from("/depot"));
        assert_eq!(paths.mods, PathBuf::from("/depot/mods"));
        assert_eq!(paths.shader_packs, PathBuf::from("/depot/shader_packs"));
        assert!(paths.from_repository());
    }

    #[test]
    fn the_environment_overrides_assets_saves_and_server_config() {
        let paths = locate(&Clues {
            assets: Some(PathBuf::from("/ailleurs/assets")),
            saves: Some(PathBuf::from("/tmp/mondes")),
            server_config: Some(PathBuf::from("/tmp/serveur.ron")),
            ..repo_clues(Path::new("/depot"))
        });
        assert_eq!(paths.game, PathBuf::from("/ailleurs/assets"));
        assert_eq!(paths.root, PathBuf::from("/ailleurs"));
        assert_eq!(paths.saves, PathBuf::from("/tmp/mondes"));
        assert_eq!(paths.server_config, PathBuf::from("/tmp/serveur.ron"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_installation_writes_in_the_user_folder() {
        use std::os::unix::fs::PermissionsExt;
        let install = temp("readonly");
        std::fs::create_dir_all(install.join("game/core")).unwrap();
        let user = temp("user");
        let clues = Clues {
            exe: Some(install.join("voxel")),
            user_dir: Some(user.clone()),
            ..repo_clues(Path::new("/depot"))
        };
        // Inscriptible : l'installation reste la racine.
        assert_eq!(locate(&clues).layout, Layout::Install);

        std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o555)).unwrap();
        let paths = locate(&clues);
        std::fs::set_permissions(&install, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Lancé en administrateur, le dossier reste inscriptible : rien à vérifier.
        if paths.layout == Layout::UserDir {
            assert_eq!(paths.root, user);
            assert_eq!(paths.game, install.join("game"));
            assert_eq!(paths.saves, user.join("saves"));
            assert_eq!(paths.settings, user.join("settings.ron"));
            assert_eq!(paths.logs, user.join("logs"));
            assert_eq!(paths.shipped, Some(install.clone()));
            assert_eq!(paths.mods_dirs(), [user.join("mods"), install.join("mods")]);
            assert_eq!(
                paths.shader_pack_dirs(),
                [user.join("shader_packs"), install.join("shader_packs")]
            );
        }
        let _ = std::fs::remove_dir_all(&install);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn the_user_folder_follows_the_system() {
        let home = |name: &str| (name == "HOME").then(|| PathBuf::from("/home/ana"));
        let dir = user_dir(&home);
        if cfg!(target_os = "macos") {
            assert_eq!(
                dir,
                Some(PathBuf::from("/home/ana/Library/Application Support/Voxel"))
            );
        } else if !cfg!(windows) {
            assert_eq!(dir, Some(PathBuf::from("/home/ana/.local/share/voxel")));
        }
    }

    #[test]
    fn the_data_dir_argument_is_read() {
        let args: Vec<String> = ["voxel-server", "--data-dir", "/srv/voxel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(data_dir_arg(&args), Some(PathBuf::from("/srv/voxel")));
        assert_eq!(data_dir_arg(&args[..1]), None);
    }
}
