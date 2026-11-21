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
use crate::players::PlayerId;
use crate::server_side::VoxelServer;

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
            port: crate::net_host::DEFAULT_PORT,
            max_players: 8,
            view_distance: 12,
            whitelist: Vec::new(),
            operators: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// `server.ron` à la racine du projet, ou le fichier que nomme `VOXEL_SERVER_CONFIG`.
    pub fn path(root: &Path) -> PathBuf {
        std::env::var_os("VOXEL_SERVER_CONFIG")
            .map_or_else(|| root.join("server.ron"), PathBuf::from)
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
    pub(crate) fn hide_local(&mut self) {
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
pub fn run() -> Result<()> {
    let root = crate::project_root();
    let config = ServerConfig::load_or_create(&root)?;
    let mut settings = crate::settings::Settings::load(&root);
    settings.player_name = "Serveur".into();
    settings.render_distance = config.view_distance.clamp(2, 32);
    let launch = Launch {
        world: Some(crate::persistence::saves_root(&root).join(&config.world)),
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
                println!("entrée standard fermée : arrêt");
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
        if report.elapsed() >= REPORT_EVERY {
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
    println!("serveur arrêté, monde sauvegardé");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut server = crate::server_boot::tests::headless();
        server.hide_local();
        assert!(crate::mobs::player_spots(&server.ecs).is_empty());
        assert!(server.players.local().ghost);
    }
}
