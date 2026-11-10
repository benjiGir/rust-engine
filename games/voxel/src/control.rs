//! Piloter le jeu par commandes texte : `VOXEL_CONTROL=7878` (ou `=1`, le port 7878).
//!
//! Le jeu écoute sur `127.0.0.1` (jamais le réseau) : un outil envoie une ligne de texte
//! (`state`, `goto 10 70 -4`, `break`...) et reçoit une ligne de JSON. Pratique pour
//! tester en jouant sans clavier (macOS bloque les touches simulées), écrire des
//! scénarios ou laisser un agent jouer. Client : `cargo xtask ctl state "walk 2"`,
//! ou `nc 127.0.0.1 7878`.
//!
//! Les threads : un thread accepte les connexions, un thread par client lit ses lignes.
//! Chaque ligne part vers le thread principal par un canal, avec un canal de réponse ;
//! le jeu l'exécute pendant l'image (il est le seul à toucher au monde), puis répond.
//! Le client attend donc au plus une image, ou la durée d'un `wait`.
//!
//! Idée reprise d'Horizon (`docs/agent-control.md`), réécrite ici.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use crossbeam_channel::{Receiver, Sender};
use engine_core::Context;
use engine_voxel::BlockId;
use glam::{IVec3, Vec3};

use crate::player::{self, MoveMode, SPRINT_SPEED, WALK_SPEED};
use crate::{Mode, VoxelGame};

/// Le port par défaut (`VOXEL_CONTROL=1`).
pub const DEFAULT_PORT: u16 = 7878;

/// L'aide renvoyée par `help`.
const HELP: &str = "state | goto x y z | look yaw pitch | lookat x y z \
| walk secondes [forward|back|left|right] [sprint] [jump] | jump | stop | wait secondes \
| block x y z | break [x y z] | place [x y z bloc] | give objet [nombre] | select 1-9 \
| inventory | mode walk|fly | time 0-1 | culling on|off | damage n | screenshot fichier.png [largeur hauteur] | quit";

/// Une ligne reçue, et le chemin de sa réponse.
pub struct Request {
    pub line: String,
    reply: Sender<String>,
}

impl Request {
    /// Répond (une ligne de JSON). Si le client est parti, tant pis.
    pub fn reply(self, json: String) {
        let _ = self.reply.send(json);
    }
}

/// Le serveur de commandes. Il vit dans l'application, pas dans un monde : il survit
/// aux changements de monde.
pub struct ControlServer {
    requests: Receiver<Request>,
    addr: SocketAddr,
}

impl ControlServer {
    /// `VOXEL_CONTROL=port` (ou `1` pour le port par défaut) ; `None` sinon.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("VOXEL_CONTROL").ok()?;
        let port = match value.trim().parse::<u16>() {
            Ok(1) => DEFAULT_PORT,
            Ok(port) => port,
            Err(_) => {
                log::error!("VOXEL_CONTROL={value} : un numéro de port est attendu");
                return None;
            }
        };
        match Self::start(port) {
            Ok(server) => {
                log::info!("commandes : écoute sur {}", server.addr());
                Some(server)
            }
            Err(err) => {
                log::error!("commandes : port {port} indisponible : {err:#}");
                None
            }
        }
    }

    /// Écoute sur `127.0.0.1:port` (0 : un port libre, pour les tests).
    pub fn start(port: u16) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let (sender, requests) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("commandes".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let sender = sender.clone();
                    let spawned = std::thread::Builder::new()
                        .name("commandes-client".into())
                        .spawn(move || serve_client(stream, sender));
                    if let Err(err) = spawned {
                        log::error!("commandes : client refusé : {err}");
                    }
                }
            })?;
        Ok(Self { requests, addr })
    }

    /// Les lignes arrivées depuis le dernier appel.
    pub fn poll(&self) -> impl Iterator<Item = Request> + '_ {
        self.requests.try_iter()
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Lit les lignes d'un client, une par une : chacune attend sa réponse avant la suivante.
fn serve_client(stream: TcpStream, sender: Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let (reply, answer) = crossbeam_channel::bounded(1);
        if sender.send(Request { line, reply }).is_err() {
            return; // Le jeu s'est fermé.
        }
        // Une réponse abandonnée (monde changé pendant un `wait`) : on le dit.
        let json = answer
            .recv()
            .unwrap_or_else(|_| error("commande abandonnée (le monde a changé ?)"));
        if writeln!(writer, "{json}").is_err() {
            return;
        }
    }
}

/// Une commande reconnue.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    State,
    /// Les pieds du joueur à cette position.
    Goto(Vec3),
    /// Lacet et tangage, en degrés.
    Look(f32, f32),
    LookAt(Vec3),
    Walk(Walk),
    Stop,
    Wait(f32),
    Block(IVec3),
    /// `None` : le bloc visé, comme un minage terminé.
    Break(Option<IVec3>),
    /// `None` : l'objet tenu sur le bloc visé, comme un clic droit.
    Place(Option<(IVec3, String)>),
    Give(String, u32),
    /// Case de la barre, de 0 à 8.
    Select(usize),
    Inventory,
    Mode(MoveMode),
    Time(f32),
    /// Le culling des grottes allumé ou non.
    Culling(bool),
    /// Un coup au joueur (armure et invulnérabilité comprises).
    Damage(f32),
    Screenshot(PathBuf, u32, u32),
    Quit,
}

/// Une marche commandée, relative au regard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Walk {
    pub seconds: f32,
    /// Avant (+) / arrière (-), puis droite (+) / gauche (-).
    pub forward: f32,
    pub right: f32,
    pub sprint: bool,
    pub jump: bool,
}

/// Lit une ligne de commande.
pub fn parse(line: &str) -> Result<Command> {
    let mut words = line.split_whitespace();
    let name = words.next().context("commande vide")?.to_ascii_lowercase();
    let args: Vec<&str> = words.collect();
    let number = |i: usize| -> Result<f32> {
        let word = args
            .get(i)
            .with_context(|| format!("argument {} manquant", i + 1))?;
        word.parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())
            .with_context(|| format!("« {word} » n'est pas un nombre"))
    };
    let vec3 = |from: usize| -> Result<Vec3> {
        Ok(Vec3::new(
            number(from)?,
            number(from + 1)?,
            number(from + 2)?,
        ))
    };
    let block_pos = |from: usize| -> Result<IVec3> { Ok(vec3(from)?.floor().as_ivec3()) };
    let count = |expected: &[usize]| -> Result<()> {
        if expected.contains(&args.len()) {
            Ok(())
        } else {
            bail!("{name} : mauvais nombre d'arguments (voir `help`)")
        }
    };
    let command = match name.as_str() {
        "help" => Command::Help,
        "state" => Command::State,
        "goto" => {
            count(&[3])?;
            Command::Goto(vec3(0)?)
        }
        "look" => {
            count(&[2])?;
            Command::Look(number(0)?, number(1)?)
        }
        "lookat" => {
            count(&[3])?;
            Command::LookAt(vec3(0)?)
        }
        "walk" => {
            let seconds = number(0)?;
            let mut walk = Walk {
                seconds,
                forward: 1.0,
                right: 0.0,
                sprint: false,
                jump: false,
            };
            for word in &args[1..] {
                match *word {
                    "forward" => (walk.forward, walk.right) = (1.0, 0.0),
                    "back" => (walk.forward, walk.right) = (-1.0, 0.0),
                    "left" => (walk.forward, walk.right) = (0.0, -1.0),
                    "right" => (walk.forward, walk.right) = (0.0, 1.0),
                    "sprint" => walk.sprint = true,
                    "jump" => walk.jump = true,
                    other => bail!("walk : « {other} » inconnu"),
                }
            }
            if !(0.0..=60.0).contains(&seconds) {
                bail!("walk : de 0 à 60 secondes");
            }
            Command::Walk(walk)
        }
        // Un saut sur place : l'intention de sauter pendant un pas de simulation.
        "jump" => Command::Walk(Walk {
            seconds: 0.05,
            forward: 0.0,
            right: 0.0,
            sprint: false,
            jump: true,
        }),
        "stop" => Command::Stop,
        "wait" => {
            count(&[1])?;
            let seconds = number(0)?;
            if !(0.0..=120.0).contains(&seconds) {
                bail!("wait : de 0 à 120 secondes");
            }
            Command::Wait(seconds)
        }
        "block" => {
            count(&[3])?;
            Command::Block(block_pos(0)?)
        }
        "break" => {
            count(&[0, 3])?;
            Command::Break((args.len() == 3).then(|| block_pos(0)).transpose()?)
        }
        "place" => {
            count(&[0, 4])?;
            let target = if args.len() == 4 {
                Some((block_pos(0)?, qualified(args[3])))
            } else {
                None
            };
            Command::Place(target)
        }
        "give" => {
            count(&[1, 2])?;
            let amount = if args.len() == 2 {
                args[1]
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("give : le nombre doit être un entier positif")?
            } else {
                1
            };
            Command::Give(qualified(args[0]), amount)
        }
        "select" => {
            count(&[1])?;
            let slot: usize = args[0]
                .parse()
                .ok()
                .filter(|s| (1..=9).contains(s))
                .context("select : une case de 1 à 9")?;
            Command::Select(slot - 1)
        }
        "inventory" => Command::Inventory,
        "mode" => {
            count(&[1])?;
            match args[0] {
                "walk" => Command::Mode(MoveMode::Walk),
                "fly" => Command::Mode(MoveMode::Fly),
                other => bail!("mode : walk ou fly, pas « {other} »"),
            }
        }
        "time" => {
            count(&[1])?;
            Command::Time(number(0)?.rem_euclid(1.0))
        }
        "damage" => {
            count(&[1])?;
            let amount = number(0)?;
            if amount <= 0.0 {
                bail!("damage : un nombre positif");
            }
            Command::Damage(amount)
        }
        "culling" => {
            count(&[1])?;
            match args[0] {
                "on" => Command::Culling(true),
                "off" => Command::Culling(false),
                other => bail!("culling : on ou off, pas « {other} »"),
            }
        }
        "screenshot" => {
            count(&[1, 3])?;
            let (width, height) = if args.len() == 3 {
                (number(1)? as u32, number(2)? as u32)
            } else {
                (1280, 720)
            };
            if !(16..=4096).contains(&width) || !(16..=4096).contains(&height) {
                bail!("screenshot : de 16 à 4096 pixels de côté");
            }
            Command::Screenshot(PathBuf::from(args[0]), width, height)
        }
        "quit" => Command::Quit,
        other => bail!("commande inconnue « {other} » (voir `help`)"),
    };
    Ok(command)
}

/// `stone` → `core:stone` : le préfixe du jeu de base est facultatif.
fn qualified(key: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("core:{key}")
    }
}

/// Ce que le pilotage garde d'une image à l'autre, dans le monde en cours.
#[derive(Default)]
pub struct Remote {
    /// La marche en cours et le temps qu'il lui reste.
    walk: Option<Walk>,
    /// Les `wait` en cours : le temps de jeu restant, et à qui répondre.
    waits: Vec<(f32, Request)>,
}

impl VoxelGame {
    /// L'intention de marche commandée pour ce pas de simulation, s'il y en a une.
    pub(crate) fn remote_intent(&mut self, dt: f32) -> Option<(Vec3, bool)> {
        let walk = self.remote.walk.as_mut()?;
        let forward = self.camera.forward_flat();
        let right = forward.cross(Vec3::Y);
        let direction = (forward * walk.forward + right * walk.right).normalize_or_zero();
        let speed = if walk.sprint {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        let intent = (direction * speed, walk.jump);
        walk.seconds -= dt;
        if walk.seconds <= 0.0 {
            self.remote.walk = None;
        }
        Some(intent)
    }

    /// Fait avancer les `wait` : ceux qui sont écoulés répondent avec l'état du jeu.
    pub(crate) fn update_remote(&mut self, dt: f32) {
        if self.remote.waits.is_empty() {
            return;
        }
        for wait in &mut self.remote.waits {
            wait.0 -= dt;
        }
        let (done, waiting) = std::mem::take(&mut self.remote.waits)
            .into_iter()
            .partition(|(left, _)| *left <= 0.0);
        self.remote.waits = waiting;
        for (_, request) in done {
            request.reply(self.state_json());
        }
    }

    /// Exécute une commande et y répond (tout de suite, sauf `wait`).
    pub(crate) fn run_command(&mut self, ctx: &mut Context, request: Request) {
        let _span = engine_trace::span("commande");
        log::debug!("commande : {}", request.line);
        let command = match parse(&request.line) {
            Ok(command) => command,
            Err(err) => return request.reply(error(&format!("{err:#}"))),
        };
        if let Command::Wait(seconds) = command {
            self.remote.waits.push((seconds, request));
            return;
        }
        let json = match self.execute(ctx, command) {
            Ok(json) => json,
            Err(err) => error(&format!("{err:#}")),
        };
        request.reply(json);
    }

    fn execute(&mut self, ctx: &mut Context, command: Command) -> Result<String> {
        let playing = self.mode == Mode::Playing;
        let needs_world = !matches!(
            command,
            Command::Help
                | Command::State
                | Command::Screenshot(..)
                | Command::Culling(_)
                | Command::Quit
        );
        if needs_world && !playing {
            bail!("pas en jeu (menu ou pause) : lancer un monde, par exemple avec VOXEL_WORLD");
        }
        match command {
            Command::Help => Ok(format!("{{\"ok\":true,\"help\":{}}}", json_str(HELP))),
            Command::State | Command::Wait(_) => Ok(self.state_json()),
            Command::Goto(feet) => {
                player::set_eye(
                    &self.ecs,
                    self.player,
                    feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
                );
                Ok(self.state_json())
            }
            Command::Look(yaw, pitch) => {
                self.camera.yaw = yaw.to_radians();
                self.camera.pitch = pitch.to_radians().clamp(
                    -engine_render::Camera::MAX_PITCH,
                    engine_render::Camera::MAX_PITCH,
                );
                self.target = None;
                Ok(self.state_json())
            }
            Command::LookAt(point) => {
                let eye = player::eye(&self.ecs, self.player, 1.0);
                let direction = (point - eye).normalize_or_zero();
                if direction == Vec3::ZERO {
                    bail!("lookat : le point est déjà dans l'œil");
                }
                self.camera.yaw = direction.z.atan2(direction.x);
                self.camera.pitch = direction.y.asin().clamp(
                    -engine_render::Camera::MAX_PITCH,
                    engine_render::Camera::MAX_PITCH,
                );
                self.target = None;
                Ok(self.state_json())
            }
            Command::Walk(walk) => {
                if player::mode(&self.ecs, self.player) == MoveMode::Fly {
                    bail!("walk : en vol, utiliser goto ou `mode walk`");
                }
                self.remote.walk = Some(walk);
                Ok(ok())
            }
            Command::Stop => {
                self.remote.walk = None;
                Ok(ok())
            }
            Command::Block(pos) => {
                let streamer = self.streamer();
                let world = streamer.world();
                let block = world.block(pos);
                let (sky, light) = world.light(pos);
                Ok(format!(
                    "{{\"ok\":true,\"pos\":{},\"block\":{},\"loaded\":{},\"light\":[{sky},{light}]}}",
                    ivec3(pos),
                    json_str(&self.registry.def(block).key),
                    streamer.is_generated(engine_voxel::World::chunk_containing(pos.as_vec3())),
                ))
            }
            Command::Break(pos) => {
                let pos = match pos {
                    Some(pos) => pos,
                    None => self.aimed()?.block_pos,
                };
                let block = self.streamer().world().block(pos);
                if block.is_air() {
                    bail!("break : rien à casser en {}", ivec3(pos));
                }
                let key = self.registry.def(block).key.clone();
                if !self.break_block(pos) {
                    bail!("break : {key} est incassable ou hors du monde chargé");
                }
                Ok(format!(
                    "{{\"ok\":true,\"broken\":{},\"pos\":{}}}",
                    json_str(&key),
                    ivec3(pos)
                ))
            }
            Command::Place(None) => {
                self.aimed()?;
                let pos = self.place_at_target().context(
                    "place : impossible ici (rien en main, pas de place, pas de support)",
                )?;
                Ok(format!("{{\"ok\":true,\"pos\":{}}}", ivec3(pos)))
            }
            Command::Place(Some((pos, key))) => {
                let block = self
                    .registry
                    .id(&key)
                    .with_context(|| format!("place : bloc inconnu « {key} »"))?;
                if block == BlockId::AIR {
                    bail!("place : pour enlever un bloc, utiliser break");
                }
                if !self.edit(pos, block) {
                    bail!("place : {} est hors du monde chargé", ivec3(pos));
                }
                Ok(format!("{{\"ok\":true,\"pos\":{}}}", ivec3(pos)))
            }
            Command::Give(key, amount) => {
                let item = self
                    .content
                    .items
                    .id(&key)
                    .with_context(|| format!("give : objet inconnu « {key} »"))?;
                let left = self.inventory.add(item, amount, &self.content.items);
                Ok(format!(
                    "{{\"ok\":true,\"given\":{},\"full\":{left}}}",
                    amount - left
                ))
            }
            Command::Select(slot) => {
                self.inventory.selected = slot;
                Ok(self.state_json())
            }
            Command::Inventory => {
                let items = &self.content.items;
                let slots: Vec<String> = self
                    .inventory
                    .slots
                    .iter()
                    .enumerate()
                    .filter_map(|(i, stack)| {
                        let stack = stack.as_ref()?;
                        Some(format!(
                            "{{\"slot\":{},\"item\":{},\"count\":{},\"damage\":{}}}",
                            i + 1,
                            json_str(&items.get(stack.item).key),
                            stack.count,
                            stack.damage
                        ))
                    })
                    .collect();
                Ok(format!(
                    "{{\"ok\":true,\"selected\":{},\"slots\":[{}]}}",
                    self.inventory.selected + 1,
                    slots.join(",")
                ))
            }
            Command::Mode(mode) => {
                if player::mode(&self.ecs, self.player) != mode {
                    player::toggle_mode(&mut self.ecs, self.player);
                }
                self.remote.walk = None;
                Ok(self.state_json())
            }
            Command::Time(time) => {
                self.day.time = time;
                Ok(self.state_json())
            }
            Command::Damage(amount) => {
                let dealt = self.hurt_player(amount, crate::survival::DamageCause::Command);
                Ok(format!(
                    "{{\"ok\":true,\"dealt\":{}}}",
                    dealt.map_or("null".into(), |d| format!("{d:.2}"))
                ))
            }
            Command::Culling(enabled) => {
                self.culling.enabled = enabled;
                Ok(ok())
            }
            Command::Screenshot(path, width, height) => {
                let (ui, _) = self.build_ui(crate::ui::offscreen_input(width, height));
                self.update_visibility(width as f32 / height as f32);
                let capture = self
                    .renderer
                    .capture(&self.camera, width, height, Some(ui))?;
                if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir)?;
                }
                crate::screenshots::save_png(&path, capture.width, capture.height, &capture.rgba)?;
                let path = std::path::absolute(&path).unwrap_or(path);
                Ok(format!(
                    "{{\"ok\":true,\"path\":{},\"width\":{},\"height\":{}}}",
                    json_str(&path.display().to_string()),
                    capture.width,
                    capture.height
                ))
            }
            Command::Quit => {
                ctx.exit();
                Ok(ok())
            }
        }
    }

    /// Le bloc visé, recalculé maintenant (la commande a pu tourner la tête).
    fn aimed(&mut self) -> Result<engine_voxel::RayHit> {
        let eye = player::eye(&self.ecs, self.player, 1.0);
        let hit = engine_voxel::raycast(
            self.streamer().world(),
            eye,
            self.camera.forward(),
            crate::REACH,
        );
        self.target = hit;
        hit.context("aucun bloc visé à portée")
    }

    /// L'état du jeu, en une ligne de JSON.
    fn state_json(&self) -> String {
        let mode = match &self.mode {
            Mode::Title(_) => "title",
            Mode::Playing => "playing",
            Mode::Paused(_) => "paused",
        };
        let feet = player::body(&self.ecs, self.player).position;
        // L'état de la physique est dans le `Collider` (le `PlayerBody` est recréé).
        let collider = self
            .ecs
            .get::<crate::Collider>(self.player)
            .map(|c| *c)
            .unwrap_or_default();
        let health = player::health(&self.ecs, self.player);
        let streamer = self.streamer();
        let generator = streamer.generator();
        let biome = generator
            .biome(
                generator
                    .column(feet.x.floor() as i32, feet.z.floor() as i32)
                    .biome,
            )
            .name
            .clone();
        let target = match self.target {
            Some(hit) => format!(
                "{{\"pos\":{},\"block\":{},\"face\":{},\"distance\":{:.2}}}",
                ivec3(hit.block_pos),
                json_str(&self.registry.def(streamer.world().block(hit.block_pos)).key),
                ivec3(hit.normal),
                hit.distance
            ),
            None => "null".into(),
        };
        let selected = match self.inventory.selected_stack() {
            Some(stack) => format!(
                "{{\"slot\":{},\"item\":{},\"count\":{}}}",
                self.inventory.selected + 1,
                json_str(&self.content.items.get(stack.item).key),
                stack.count
            ),
            None => format!("{{\"slot\":{},\"item\":null}}", self.inventory.selected + 1),
        };
        let stats = streamer.stats();
        format!(
            "{{\"ok\":true,\"mode\":\"{mode}\",\"move\":\"{}\",\"pos\":{},\"on_ground\":{},\"in_liquid\":{},\"frozen\":{},\
\"yaw\":{:.1},\"pitch\":{:.1},\"health\":[{:.1},{:.1}],\"time\":{:.3},\"clock\":{},\
\"biome\":{},\"target\":{target},\"selected\":{selected},\"walking\":{},\
\"chunks\":{{\"loaded\":{},\"queued\":{},\"idle\":{}}},\
\"render\":{{\"drawn\":{},\"hidden\":{},\"total\":{},\"triangles\":{},\"cave_culling\":{}}},\
\"survival\":{{\"air\":{:.2},\"burning\":{:.2},\"protection\":{}}}}}",
            // Les mêmes mots que la commande `mode`.
            match player::mode(&self.ecs, self.player) {
                MoveMode::Walk => "walk",
                MoveMode::Fly => "fly",
            },
            vec3(feet),
            collider.on_ground,
            collider.in_liquid,
            collider.frozen,
            self.camera.yaw.to_degrees(),
            self.camera.pitch.to_degrees(),
            health.current,
            health.max,
            self.day.time,
            json_str(&self.day.clock()),
            json_str(&biome),
            self.remote.walk.is_some(),
            stats.loaded,
            stats.queued,
            streamer.is_idle(),
            self.last_render.meshes_drawn,
            self.last_render.meshes_hidden,
            self.last_render.meshes_total,
            self.last_render.triangles,
            self.culling.enabled,
            self.vitals.air,
            self.vitals.burning,
            self.inventory.protection(&self.content.items),
        )
    }
}

fn ok() -> String {
    "{\"ok\":true}".into()
}

fn error(message: &str) -> String {
    format!("{{\"ok\":false,\"error\":{}}}", json_str(message))
}

fn vec3(v: Vec3) -> String {
    format!("[{:.3},{:.3},{:.3}]", v.x, v.y, v.z)
}

fn ivec3(v: IVec3) -> String {
    format!("[{},{},{}]", v.x, v.y, v.z)
}

/// Une chaîne JSON, entre guillemets, échappée.
fn json_str(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_parsed() {
        assert_eq!(parse("state").unwrap(), Command::State);
        assert_eq!(
            parse("goto 1 70.5 -3").unwrap(),
            Command::Goto(Vec3::new(1.0, 70.5, -3.0))
        );
        assert_eq!(
            parse("break -1.5 64 2").unwrap(),
            Command::Break(Some(IVec3::new(-2, 64, 2)))
        );
        assert_eq!(parse("break").unwrap(), Command::Break(None));
        assert_eq!(
            parse("place 0 65 0 stone").unwrap(),
            Command::Place(Some((IVec3::new(0, 65, 0), "core:stone".into())))
        );
        assert_eq!(
            parse("give example:ruby 3").unwrap(),
            Command::Give("example:ruby".into(), 3)
        );
        assert_eq!(parse("SELECT 9").unwrap(), Command::Select(8));
        assert_eq!(
            parse("walk 2 left sprint").unwrap(),
            Command::Walk(Walk {
                seconds: 2.0,
                forward: 0.0,
                right: -1.0,
                sprint: true,
                jump: false,
            })
        );
        assert_eq!(parse("time 1.25").unwrap(), Command::Time(0.25));
    }

    #[test]
    fn bad_commands_explain_why() {
        for (line, expected) in [
            ("", "vide"),
            ("dance", "inconnue"),
            ("goto 1 2", "arguments"),
            ("goto 1 deux 3", "« deux »"),
            ("select 0", "1 à 9"),
            ("walk 2 sideways", "« sideways »"),
            ("give stone 0", "entier positif"),
        ] {
            let message = format!("{:#}", parse(line).unwrap_err());
            assert!(message.contains(expected), "{line} → {message}");
        }
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
        assert_eq!(error("non"), r#"{"ok":false,"error":"non"}"#);
    }

    #[test]
    fn a_client_gets_one_reply_per_line() {
        let server = ControlServer::start(0).unwrap();
        let client = std::thread::spawn({
            let addr = server.addr();
            move || {
                let mut stream = TcpStream::connect(addr).unwrap();
                writeln!(stream, "state\n\nhelp").unwrap();
                let mut lines = BufReader::new(stream).lines();
                let first = lines.next().unwrap().unwrap();
                let second = lines.next().unwrap().unwrap();
                (first, second)
            }
        });
        // Le « jeu » : répond à chaque ligne avec la ligne elle-même (la vide est sautée).
        let mut answered = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while answered < 2 {
            assert!(std::time::Instant::now() < deadline, "pas de requête reçue");
            for request in server.poll() {
                let line = request.line.clone();
                request.reply(json_str(&line));
                answered += 1;
            }
            std::thread::yield_now();
        }
        assert_eq!(
            client.join().unwrap(),
            ("\"state\"".to_string(), "\"help\"".to_string())
        );
    }
}
