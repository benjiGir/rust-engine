//! Le serveur dédié : `voxel --server` fait tourner un monde sans fenêtre, ouvert au réseau local.
//!
//! Il lit `server.ron` à la racine du projet (ou le fichier que nomme `VOXEL_SERVER_CONFIG`), créé
//! avec les valeurs par défaut s'il manque : le monde (un dossier de `saves/`), le port, le nombre de joueurs, la distance de vue, la liste
//! blanche, les opérateurs. Le serveur garde le joueur « hôte » que tout `VoxelServer` a, mais
//! caché (`Hidden`) : ni vu, ni visé, ni sauvegardé. La boucle fait ce que fait la fenêtre de
//! l'hôte (réseau, image, pas fixes à 60 Hz, envois, sauvegarde automatique), et une console lit
//! l'entrée standard : `list`, `say`, `kick`, `save`, `stop`.
//!
//! Le binaire lie encore les bibliothèques d'affichage : la simulation sortira dans sa propre crate
//! plus tard. `stop`, la fin de l'entrée standard ou Ctrl+C (et SIGTERM) font partir les joueurs,
//! sauvegardent et arrêtent.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::launch::Launch;
use crate::player::players::PlayerId;
use crate::server::VoxelServer;

/// Un pas fixe (60 par seconde, comme la fenêtre).
const STEP: f32 = 1.0 / 60.0;
/// Au plus ce nombre de pas pour rattraper un retard (au-delà, le monde ralentit).
const MAX_CATCH_UP: u32 = 5;
/// L'intervalle des mesures écrites dans le terminal.
const REPORT_EVERY: Duration = Duration::from_secs(10);

/// `server.ron`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Le nom de la partie, annoncé sur le réseau local.
    pub name: String,
    /// Le dossier du monde dans `saves/`.
    pub world: String,
    /// La graine d'un monde neuf (`None` : au hasard).
    pub seed: Option<u32>,
    pub port: u16,
    pub max_players: usize,
    /// La distance de vue envoyée aux joueurs, en chunks.
    pub view_distance: i32,
    /// Les noms admis (vide : tout le monde).
    pub whitelist: Vec<String>,
    /// Les joueurs qui peuvent taper des commandes de jeu (voir `server_commands.rs`).
    pub operators: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "Serveur voxel".into(),
            world: "serveur".into(),
            seed: None,
            port: crate::net::host::DEFAULT_PORT,
            max_players: 8,
            view_distance: 12,
            whitelist: Vec::new(),
            operators: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// `server.ron` à la racine du projet, ou le fichier que nomme `VOXEL_SERVER_CONFIG`.
    /// `server.ron` (voir `paths.rs` : la racine du jeu, `--data-dir`, ou `VOXEL_SERVER_CONFIG`).
    pub fn path(_root: &Path) -> PathBuf {
        crate::paths::get().server_config.clone()
    }

    /// Lit `server.ron`, ou l'écrit avec les valeurs par défaut s'il n'existe pas.
    pub fn load_or_create(root: &Path) -> Result<Self> {
        let path = Self::path(root);
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("lecture de {}", path.display()))?;
            return ron::from_str(&text).with_context(|| format!("{} illisible", path.display()));
        }
        let config = Self::default();
        let text = ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::default())?;
        std::fs::write(&path, text).with_context(|| format!("écriture de {}", path.display()))?;
        log::info!("{} créé avec les valeurs par défaut", path.display());
        Ok(config)
    }
}

/// Une commande de la console du serveur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    List,
    Say(String),
    Kick(String),
    Save,
    Stop,
    Help,
}

pub fn parse_console(line: &str) -> Option<Result<ConsoleCommand, String>> {
    let line = line.trim().trim_start_matches('/');
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    Some(match name {
        "" => return None,
        "list" => Ok(ConsoleCommand::List),
        "say" if !rest.is_empty() => Ok(ConsoleCommand::Say(rest.to_string())),
        "kick" if !rest.is_empty() => Ok(ConsoleCommand::Kick(rest.to_string())),
        "save" => Ok(ConsoleCommand::Save),
        "stop" | "quit" => Ok(ConsoleCommand::Stop),
        "help" => Ok(ConsoleCommand::Help),
        other => Err(format!("commande inconnue « {other} » (voir `help`)")),
    })
}

impl VoxelServer {
    /// Le joueur « hôte » d'un serveur dédié n'est personne : caché, porté par rien, jamais sauvegardé.
    pub fn hide_local(&mut self) {
        let entity = self.players.entity();
        self.ecs.insert(entity, crate::components::Hidden);
        self.ecs.insert(entity, crate::components::Flying);
        self.players.local_mut().ghost = true;
    }

    /// Une ligne de la console du serveur. Renvoie `false` pour s'arrêter.
    fn run_console(&mut self, command: ConsoleCommand) -> bool {
        match command {
            ConsoleCommand::List => {
                let names: Vec<String> = self
                    .players
                    .iter()
                    .filter(|p| !p.ghost)
                    .map(|p| p.name.clone())
                    .collect();
                println!("{} joueur(s) : {}", names.len(), names.join(", "));
            }
            ConsoleCommand::Say(text) => {
                self.tell_everyone(&format!("[Serveur] {text}"));
            }
            ConsoleCommand::Kick(name) => {
                let found: Option<PlayerId> = self
                    .players
                    .iter()
                    .find(|p| !p.ghost && p.name == name)
                    .map(|p| p.id);
                match found {
                    Some(id) => self.leave(id, "renvoyé par le serveur"),
                    None => println!("personne ne s'appelle « {name} »"),
                }
            }
            ConsoleCommand::Save => {
                self.save_now();
                println!("monde sauvegardé");
            }
            ConsoleCommand::Stop => return false,
            ConsoleCommand::Help => println!("list | say texte | kick nom | save | stop"),
        }
        true
    }
}

/// Les ticks d'un intervalle : par seconde, durée moyenne et p99 (en ms), joueurs. Vide la liste.
fn tick_report(ticks: &mut Vec<Duration>, seconds: f32, players: usize) -> String {
    let mut ms: Vec<f32> = ticks.drain(..).map(|t| t.as_secs_f32() * 1000.0).collect();
    ms.sort_by(f32::total_cmp);
    let mean = ms.iter().sum::<f32>() / ms.len().max(1) as f32;
    let p99 = ms
        .get((ms.len() * 99 / 100).min(ms.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);
    format!(
        "{:.1} ticks/s, {mean:.2} ms par tick (p99 {p99:.2} ms), {players} joueur(s)",
        ms.len() as f32 / seconds.max(0.001)
    )
}

/// Lance le serveur dédié et le fait tourner jusqu'à `stop`.
/// La boucle d'un serveur sans fenêtre : réseau, image, pas fixes à 60 Hz, envois, sauvegarde
/// automatique, et les lignes de `lines` (console ; `None` ou `stop` arrêtent). À l'arrêt, les joueurs
/// partent (sauvegardés) et le monde est sauvegardé. `report_ticks` : écrire les mesures des ticks.
fn serve(server: &mut VoxelServer, lines: &mpsc::Receiver<Option<String>>, report_ticks: bool) {
    let mut last = Instant::now();
    let mut lag = 0.0f32;
    let mut ticks: Vec<Duration> = Vec::new();
    let mut report = Instant::now();
    'running: loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.25);
        last = now;
        while let Ok(line) = lines.try_recv() {
            let Some(line) = line else {
                log::info!("console fermée : arrêt");
                break 'running;
            };
            match parse_console(&line) {
                Some(Ok(command)) => {
                    if !server.run_console(command) {
                        break 'running;
                    }
                }
                Some(Err(err)) => println!("{err}"),
                None => {}
            }
        }
        server.poll_net();
        server.frame(dt);
        server.stream_world();
        server.send_world();
        lag += dt;
        let mut steps = 0;
        while lag >= STEP && steps < MAX_CATCH_UP {
            server.sim_step += 1;
            let tick = Instant::now();
            server.fixed_step(STEP);
            ticks.push(tick.elapsed());
            lag -= STEP;
            steps += 1;
        }
        if report_ticks && report.elapsed() >= REPORT_EVERY {
            let seconds = report.elapsed().as_secs_f32();
            report = Instant::now();
            let players = server.players.iter().filter(|p| !p.ghost).count();
            println!("{}", tick_report(&mut ticks, seconds, players));
        }
        lag = lag.min(STEP * MAX_CATCH_UP as f32);
        server.replicate_player_views();
        // Les réponses du joueur caché ne vont nulle part.
        let _ = server.route_replies();
        server.end_net_frame();
        server.autosave();
        // Le reste du pas : le serveur dort, sans brûler un cœur.
        let spent = now.elapsed();
        if let Some(rest) = Duration::from_secs_f32(STEP).checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
    let ids: Vec<PlayerId> = server
        .players
        .iter()
        .filter(|p| !p.ghost)
        .map(|p| p.id)
        .collect();
    for id in ids {
        server.leave(id, "arrêt du serveur");
    }
    let _ = server.route_replies();
    server.save_now();
    log::info!("serveur arrêté, monde sauvegardé");
}

pub fn run() -> Result<()> {
    let root = crate::project_root();
    let config = ServerConfig::load_or_create(&root)?;
    let mut settings = crate::settings::Settings::load(&root);
    settings.player_name = "Serveur".into();
    // Le joueur caché ne porte l'identifiant de personne : un joueur de cette machine peut entrer.
    settings.player_uuid = crate::player::players::uuid_name(0);
    settings.render_distance = config.view_distance.clamp(2, 32);
    let launch = Launch {
        world: Some(crate::paths::get().saves.join(&config.world)),
        seed: config.seed,
        title: false,
        difficulty: None,
        dimension: None,
        join: None,
    };
    let mut server = VoxelServer::boot(&launch, &settings, STEP, false, false)?.server;
    server.hide_local();
    server.operators = config.operators.clone();
    let port = server.open_lan(config.port)?;
    if let Some(net) = &mut server.net {
        net.max_players = Some(config.max_players.max(1));
        net.whitelist = config.whitelist.clone();
        net.set_announced_name(&config.name);
    }
    println!(
        "« {} » ouvert sur le port {port} (monde {}, {} joueurs au plus). Tapez `help`.",
        config.name, config.world, config.max_players
    );

    // La console : une ligne de l'entrée standard par commande ; sa fin arrête le serveur. Ctrl+C
    // (et SIGTERM) passent par le même chemin : un arrêt demandé, puis la sauvegarde.
    let (lines_tx, lines) = mpsc::channel::<Option<String>>();
    let interrupt = lines_tx.clone();
    ctrlc::set_handler(move || {
        let _ = interrupt.send(Some("stop".to_string()));
    })
    .context("Ctrl+C ne peut pas être intercepté")?;
    std::thread::Builder::new()
        .name("console du serveur".into())
        .spawn(move || {
            for line in std::io::stdin().lines() {
                let Ok(line) = line else { break };
                if lines_tx.send(Some(line)).is_err() {
                    return;
                }
            }
            let _ = lines_tx.send(None);
        })?;

    serve(&mut server, &lines, true);
    println!("serveur arrêté, monde sauvegardé");
    Ok(())
}

/// Un serveur intégré : le monde tourne dans son propre thread, et la fenêtre le rejoint comme un
/// invité, sur la boucle locale (`VOXEL_SERVER_THREAD=1`, voir le jeu). Le lâcher l'arrête et attend la
/// sauvegarde.
pub struct Integrated {
    /// Le port de la boucle locale où d'autres clients peuvent le rejoindre (les tests).
    pub port: u16,
    /// Le joueur de la fenêtre, déjà accueilli par un lien en mémoire (à prendre une fois).
    pub joined: Option<crate::net::join::Joined>,
    lines: mpsc::Sender<Option<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Integrated {
    /// Arrête le serveur et attend qu'il ait sauvegardé.
    pub fn stop(&mut self) {
        let _ = self.lines.send(Some("stop".to_string()));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Integrated {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Construit le monde `world` dans un thread à lui (le serveur ne change jamais de thread : il y est
/// construit) et l'y fait tourner. `owner` (le joueur de la fenêtre) y est opérateur.
pub fn spawn_integrated(
    world: std::path::PathBuf,
    settings: &crate::settings::Settings,
    owner: &str,
    owner_uuid: u128,
) -> Result<Integrated> {
    let mut settings = settings.clone();
    // Le joueur « hôte » du serveur est caché : il ne porte pas l'identifiant du joueur de la fenêtre,
    // qui le rejoint comme un invité et retrouve son propre fichier.
    settings.player_name = "Serveur".into();
    settings.player_uuid = crate::player::players::uuid_name(0);
    let owner = owner.to_string();
    let (lines_tx, lines) = mpsc::channel::<Option<String>>();
    let (ready_tx, ready) = mpsc::channel::<Result<(u16, crate::net::join::Joined), String>>();
    let thread = std::thread::Builder::new()
        .name("serveur intégré".into())
        .spawn(move || {
            let launch = Launch {
                world: Some(world),
                seed: None,
                title: false,
                difficulty: None,
                dimension: None,
                join: None,
            };
            let mut server = match VoxelServer::boot(&launch, &settings, STEP, false, false) {
                Ok(boot) => boot.server,
                Err(err) => {
                    let _ = ready_tx.send(Err(format!("{err:#}")));
                    return;
                }
            };
            server.hide_local();
            server.operators = vec![owner.clone()];
            match server.open_local() {
                Ok(port) => {
                    // Le joueur de la fenêtre entre par un lien en mémoire.
                    let joined = server.join_local(&owner, owner_uuid);
                    let _ = ready_tx.send(Ok((port, joined)));
                }
                Err(err) => {
                    let _ = ready_tx.send(Err(err.to_string()));
                    return;
                }
            }
            serve(&mut server, &lines, false);
        })?;
    let (port, joined) = ready
        .recv()
        .context("le serveur intégré s'est arrêté avant d'écouter")?
        .map_err(|err| anyhow::anyhow!("serveur intégré : {err}"))?;
    log::info!("serveur intégré dans son thread, sur 127.0.0.1:{port}");
    Ok(Integrated {
        port,
        joined: Some(joined),
        lines: lines_tx,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_integrated_server_runs_in_its_thread_and_saves_when_stopped() {
        let dir = std::env::temp_dir().join(format!("voxel-integre-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = crate::settings::Settings::default();
        let mut integrated = spawn_integrated(dir.clone(), &settings, "Alice", 0xa11ce).unwrap();
        // La fenêtre est déjà dans la partie, par le lien en mémoire ; un bot entre par le réseau.
        let window = integrated.joined.take().unwrap();
        assert!(window.link.connection().is_none());
        // Le contenu, pour se présenter comme la fenêtre le ferait.
        let shell = crate::server::boot::tests::headless();
        let content = crate::net::host::content_fingerprint(&shell.registry, &shell.content.items);
        let address = format!("127.0.0.1:{}", integrated.port);
        let mut bot =
            crate::net::bot::Bot::join(&address, "Bob", 0xb0b, content, shell.registry.clone())
                .unwrap();
        bot.wait_until(Duration::from_secs(60), || {}, |b| b.chunks_received > 0)
            .unwrap();
        // La fenêtre reçoit le monde sans rien décoder.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut chunks = 0;
        while chunks == 0 {
            for msg in window.link.receive().unwrap() {
                if matches!(msg, crate::net::protocol::ServerMsg::Chunk { .. }) {
                    chunks += 1;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "aucun chunk par le lien en mémoire"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        bot.leave();
        integrated.stop();
        assert!(dir.join("world.ron").exists(), "le monde est sauvegardé");
        let player = dir.join("players").join(format!(
            "{}.ron",
            crate::player::players::uuid_name(0xa11ce)
        ));
        assert!(player.exists(), "le joueur de la fenêtre est sauvegardé");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tick_report_gives_rate_mean_and_p99() {
        let mut ticks: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let line = tick_report(&mut ticks, 2.0, 3);
        assert_eq!(
            line,
            "50.0 ticks/s, 50.50 ms par tick (p99 100.00 ms), 3 joueur(s)"
        );
        assert!(ticks.is_empty());
    }

    #[test]
    fn console_commands_parse() {
        assert_eq!(parse_console("list"), Some(Ok(ConsoleCommand::List)));
        assert_eq!(
            parse_console("/say bonjour à tous"),
            Some(Ok(ConsoleCommand::Say("bonjour à tous".into())))
        );
        assert_eq!(
            parse_console("kick Bob"),
            Some(Ok(ConsoleCommand::Kick("Bob".into())))
        );
        assert!(matches!(parse_console("kick"), Some(Err(_))));
        assert_eq!(parse_console("   "), None);
        assert_eq!(parse_console("stop"), Some(Ok(ConsoleCommand::Stop)));
    }

    #[test]
    fn the_server_config_round_trips_and_fills_missing_fields() {
        let config: ServerConfig = ron::from_str("(port: 30000, whitelist: [\"Alice\"])").unwrap();
        assert_eq!(config.port, 30000);
        assert_eq!(config.whitelist, vec!["Alice".to_string()]);
        assert_eq!(config.max_players, ServerConfig::default().max_players);
        let text = ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::default()).unwrap();
        assert_eq!(ron::from_str::<ServerConfig>(&text).unwrap(), config);
    }

    #[test]
    fn the_hidden_host_is_neither_seen_nor_targeted() {
        let mut server = crate::server::boot::tests::headless();
        server.hide_local();
        assert!(crate::mobs::player_spots(&server.ecs).is_empty());
        assert!(server.players.local().ghost);
    }
}
