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
| block x y z | break [x y z] | place [x y z bloc] | give objet [nombre] | select 1-36 \
| inventory | mode walk|fly | time 0-1 | culling on|off | damage n [perce] | eat | hunger n | mobs | attack | swing | use | mods | swing | travel dimension | carts | cart sorte x y z | ride [numéro] | dismount | couple a b | cartstock numéro objet [nombre] | console [texte] | texts | click x y [right] | clicktext texte [right] | type texte | key Nom [ctrl] [shift] [alt] | respawn | pause | resume | modoptions mod | optionsevent button|toggle|slider id [valeur] | backpack | placebag | deposit | topup | takeall | sort | quickmove 1-36 | container x y z | bag [numéro] | drops | capture on|off | xp n | summon créature | screenshot fichier.png [largeur hauteur] | enchant sorte niveau [sorte niveau...] | attributes [open|close|modified on|off] | affix attribut fixe [part] | hover x y | equip n | sheet | throw | loot [rareté] [objet] | gem sorte qualité [n] | tier n | tiers | progress kills|bosses|rarity|depth n | elite [créature] | invader [créature] | elites | slay [all] | cursor n|clear | open tiers|salvage|reforge | sockets n | clearinv | pending x y z objet [rareté_min] | table x y z | shelves x y z [étagère] [n] | offers x y z | screen enchant x y z|anvil|library x y z|guide | book sorte niveau [...] | libadd x y z sorte niveau | library x y z | points n | charms | quests [chapitre] | quest done|claim|check|handin|choose|scan|reset|pin|select|ui|view|close ... | questpin id | screen quests [chapitre] | quit";

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
    /// Pour les essais : met des objets (et leur usure) dans une case de l'inventaire d'un bloc
    /// (`count` 0 : la vide).
    Stock(IVec3, usize, String, u32, u32),
    /// Case de la barre, de 0 à 8.
    Select(usize),
    Inventory,
    Mode(MoveMode),
    Time(f32),
    /// Détache la structure qui contient ce bloc, la repose, les liste, ou braque la barre.
    Assemble(IVec3),
    Land(u32),
    Ships,
    Steer(u32, f32),
    /// Les capacités d'un bloc actif (énergie, fluides, rotation).
    Inspect(IVec3),
    /// Le bilan des réseaux de câbles, de tuyaux et d'arbres.
    Networks,
    /// Les structures à moins de ce nombre de blocs du joueur.
    Structures(i32),
    /// Le temps qu'il fait : `weather clear|rain|storm`.
    Weather(crate::weather::Sky),
    /// Le culling des grottes allumé ou non.
    Culling(bool),
    /// Un coup au joueur (armure et invulnérabilité comprises).
    /// Dégâts, et les points d'armure que ce coup contourne (perforation).
    Damage(f32, f32),
    /// Manger tout de suite l'aliment tenu.
    Eat,
    /// Les créatures proches : type, position, vie.
    Mobs,
    /// Frapper la créature (ou le wagon) visée (comme un clic gauche).
    Attack,
    /// Un coup de bras dans le vide (pour voir l'animation).
    Swing,
    /// Un clic droit sur le bloc visé : un lit, un conteneur, sinon poser.
    Use,
    /// Réapparaître, depuis l'écran de mort.
    Respawn,
    /// Les wagons : les lister, en poser un sur le rail en x y z, monter dans un wagonnet (le plus
    /// proche, ou ce numéro), descendre, atteler deux wagons, ranger des objets dans l'un d'eux.
    Carts,
    Cart(crate::carts::CartKind, IVec3),
    Ride(Option<u32>),
    Dismount,
    Couple(u32, u32),
    CartStock(u32, String, u32),
    /// La pause, la reprise, la page d'options d'un mod, et une action du joueur dans cette page.
    /// La consommation de temps de chaque mod.
    Mods,
    /// Passe dans une dimension (`mon_mod:ma_dimension`) : le monde est rechargé.
    Travel(String),
    /// Ouvre la console (sans texte) ou y valide une ligne.
    Console(Option<String>),
    /// Les commandes du stockage : sac, gestes sur un conteneur, contenu (voir `storage_ctl.rs`).
    Storage(crate::storage_ctl::Command),
    /// Les commandes des objets et des attributs (voir `item_ctl.rs`).
    Items(crate::item_ctl::Command),
    /// Les commandes de l'aventure : butin, gemmes, paliers, élites (voir `adventure_ctl.rs`).
    Adventure(crate::adventure_ctl::Command),
    /// Les commandes du livre de quêtes (voir `quest_ctl.rs`).
    Quests(crate::quest_ctl::Command),
    /// Les commandes de l'enchantement avancé : stats de table, livres, bibliothèque (voir `enchant_ctl.rs`).
    Enchanting(crate::enchant_ctl::Command),
    /// Les textes affichés (avec leur position en points), pour viser un bouton.
    UiTexts,
    /// Un clic de souris en (x, y) points ; vrai : bouton droit.
    UiClick(f32, f32, bool),
    /// Un clic sur le premier texte qui contient ceci.
    UiClickText(String, bool),
    /// Du texte tapé au clavier.
    UiType(String),
    /// Une touche (nom egui) avec ses modificateurs : ctrl, shift, alt.
    UiKey(String, Vec<String>),
    Pause,
    Resume,
    ModOptions(String),
    OptionsEvent(i32, i32, i32),
    /// Gagner des points d'expérience.
    Xp(u32),
    /// Faire apparaître une créature à 3 blocs devant le joueur.
    Summon(String),
    /// Régler la faim (de 0 à 20), pour les tests.
    Hunger(f32),
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
        "stock" => {
            count(&[6, 7])?;
            Command::Stock(
                block_pos(0)?,
                args[3].parse().context("stock : case invalide")?,
                qualified(args[4]),
                args[5].parse().context("stock : nombre invalide")?,
                args.get(6)
                    .map_or(Ok(0), |d| d.parse())
                    .context("stock : usure invalide")?,
            )
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
                .filter(|s| (1..=36).contains(s))
                .context(
                    "select : une case de 1 à 36 (les neuf premières sont la barre d'objets)",
                )?;
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
        "inspect" => {
            count(&[3])?;
            Command::Inspect(block_pos(0)?)
        }
        "networks" => Command::Networks,
        "assemble" => {
            count(&[3])?;
            Command::Assemble(block_pos(0)?)
        }
        "land" => {
            count(&[1])?;
            Command::Land(
                args[0]
                    .parse()
                    .context("land : le numéro d'une structure (voir ships)")?,
            )
        }
        "ships" => Command::Ships,
        "carts" => Command::Carts,
        "cart" => {
            count(&[4])?;
            let kind = crate::carts::CartKind::parse(args[0]).with_context(|| {
                format!(
                    "cart : sorte inconnue « {} » (minecart, chest, hopper, locomotive)",
                    args[0]
                )
            })?;
            Command::Cart(kind, block_pos(1)?)
        }
        "ride" => {
            count(&[0, 1])?;
            Command::Ride(match args.first() {
                Some(id) => Some(
                    id.parse()
                        .context("ride : le numéro d'un wagon (voir carts)")?,
                ),
                None => None,
            })
        }
        "dismount" => Command::Dismount,
        "couple" => {
            count(&[2])?;
            Command::Couple(
                args[0]
                    .parse()
                    .context("couple : deux numéros de wagons (voir carts)")?,
                args[1]
                    .parse()
                    .context("couple : deux numéros de wagons (voir carts)")?,
            )
        }
        "cartstock" => {
            count(&[2, 3])?;
            let n = if args.len() == 3 {
                number(2)?.max(0.0) as u32
            } else {
                1
            };
            Command::CartStock(
                args[0]
                    .parse()
                    .context("cartstock : le numéro d'un wagon (voir carts)")?,
                qualified(args[1]),
                n,
            )
        }
        "steer" => {
            count(&[2])?;
            Command::Steer(
                args[0]
                    .parse()
                    .context("steer : le numéro d'une structure (voir ships)")?,
                number(1)?.clamp(-1.0, 1.0),
            )
        }
        "structures" => {
            count(&[0, 1])?;
            let radius = if args.is_empty() { 600.0 } else { number(0)? };
            Command::Structures(radius.clamp(16.0, 4000.0) as i32)
        }
        "weather" => {
            count(&[1])?;
            Command::Weather(crate::weather::Sky::parse(args[0]).ok_or_else(|| {
                anyhow::anyhow!("temps inconnu « {} » (clear, rain ou storm)", args[0])
            })?)
        }
        "eat" => Command::Eat,
        "mobs" => Command::Mobs,
        "attack" => Command::Attack,
        "swing" => Command::Swing,
        "use" => Command::Use,
        "respawn" => Command::Respawn,
        "mods" => Command::Mods,
        "travel" => {
            count(&[1])?;
            Command::Travel(args[0].to_string())
        }
        "texts" => Command::UiTexts,
        "click" => {
            count(&[2, 3])?;
            Command::UiClick(number(0)?, number(1)?, args.get(2) == Some(&"right"))
        }
        "clicktext" => {
            let right = args.last() == Some(&"right");
            let words = &args[..args.len() - usize::from(right)];
            if words.is_empty() {
                bail!("clicktext : quel texte cliquer ?");
            }
            Command::UiClickText(words.join(" "), right)
        }
        "type" => {
            if args.is_empty() {
                bail!("type : quel texte taper ?");
            }
            Command::UiType(args.join(" "))
        }
        "key" => {
            if args.is_empty() {
                bail!("key : quelle touche ? (Enter, Escape, ArrowUp, A...)");
            }
            Command::UiKey(
                args[0].to_string(),
                args[1..].iter().map(|m| m.to_string()).collect(),
            )
        }
        "console" => Command::Console((!args.is_empty()).then(|| args.join(" "))),
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "modoptions" => {
            count(&[1])?;
            Command::ModOptions(args[0].to_string())
        }
        "optionsevent" => {
            count(&[2, 3])?;
            let id: i32 = args[1].parse().context("identifiant")?;
            let value = args.get(2).copied().unwrap_or("0");
            match args[0] {
                "button" => Command::OptionsEvent(engine_mods::menu::UI_BUTTON, id, 0),
                "toggle" => Command::OptionsEvent(
                    engine_mods::menu::UI_TOGGLE,
                    id,
                    i32::from(value == "1" || value == "true"),
                ),
                "slider" => {
                    let v: f32 = value.parse().context("valeur")?;
                    Command::OptionsEvent(engine_mods::menu::UI_SLIDER, id, v.to_bits() as i32)
                }
                other => bail!("action inconnue « {other} » (button, toggle ou slider)"),
            }
        }
        "summon" => {
            count(&[1])?;
            Command::Summon(qualified(args[0]))
        }
        "xp" => {
            count(&[1])?;
            Command::Xp(number(0)?.max(0.0) as u32)
        }
        "hunger" => {
            count(&[1])?;
            Command::Hunger(number(0)?)
        }
        "damage" => {
            count(&[1, 2])?;
            let amount = number(0)?;
            if amount <= 0.0 {
                bail!("damage : un nombre positif");
            }
            let pierce = if args.len() == 2 {
                number(1)?.max(0.0)
            } else {
                0.0
            };
            Command::Damage(amount, pierce)
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
        "screen" if args.first() == Some(&"quests") => {
            Command::Quests(crate::quest_ctl::parse(&name, &args).expect("screen quests")?)
        }
        other => match crate::storage_ctl::parse(other, &args) {
            Some(command) => Command::Storage(command?),
            None => match crate::item_ctl::parse(other, &args) {
                Some(command) => Command::Items(command?),
                None => match crate::adventure_ctl::parse(other, &args) {
                    Some(command) => Command::Adventure(command?),
                    None => match crate::enchant_ctl::parse(other, &args) {
                        Some(command) => Command::Enchanting(command?),
                        None => match crate::quest_ctl::parse(other, &args) {
                            Some(command) => Command::Quests(command?),
                            None => bail!("commande inconnue « {other} » (voir `help`)"),
                        },
                    },
                },
            },
        },
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
        let can_sprint = self.can_sprint();
        let walk = self.remote.walk.as_mut()?;
        let forward = self.camera.forward_flat();
        let right = forward.cross(Vec3::Y);
        let direction = (forward * walk.forward + right * walk.right).normalize_or_zero();
        // Comme au clavier : affamé, on ne court plus.
        let speed = if walk.sprint && can_sprint {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        } * self.gear.move_speed();
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

    pub(crate) fn execute(&mut self, ctx: &mut Context, command: Command) -> Result<String> {
        let playing = self.mode == Mode::Playing;
        let needs_world = !matches!(
            command,
            Command::Help
                | Command::State
                | Command::Screenshot(..)
                | Command::Culling(_)
                | Command::Respawn
                | Command::Resume
                | Command::Mods
                | Command::Console(_)
                | Command::UiTexts
                | Command::UiClick(..)
                | Command::UiClickText(..)
                | Command::UiType(_)
                | Command::UiKey(..)
                | Command::ModOptions(_)
                | Command::OptionsEvent(..)
                | Command::Inventory
                | Command::Mobs
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
            Command::Stock(pos, slot, key, amount, wear) => {
                let mut streamer = self.streamer_mut();
                let inventory = streamer
                    .block_entity_mut(pos)
                    .and_then(|e| e.caps.as_mut())
                    .and_then(|c| c.inventory.as_mut())
                    .with_context(|| format!("stock : pas d'inventaire en {}", ivec3(pos)))?;
                let cell = inventory
                    .slots
                    .get_mut(slot)
                    .with_context(|| format!("stock : pas de case {slot}"))?;
                *cell = (amount > 0).then_some(engine_voxel::ItemStack {
                    item: key,
                    count: amount,
                    damage: wear,
                });
                Ok("{\"ok\":true}".to_string())
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
                // Pour les essais : une case hors de la barre peut être tenue en main.
                self.inventory.selected = slot.min(self.inventory.slots.len().saturating_sub(1));
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
            Command::Inspect(pos) => {
                let streamer = self.streamer();
                let block = json_str(&self.registry.def(streamer.world().block(pos)).key);
                let Some(caps) = streamer
                    .block_entity(pos)
                    .and_then(|e| e.caps.as_deref().map(|c| (e.power, c)))
                else {
                    return Ok(format!(
                        "{{\"ok\":true,\"block\":{block},\"capabilities\":null}}"
                    ));
                };
                let (power, caps) = caps;
                let energy = caps.energy.as_ref().map_or("null".to_string(), |e| {
                    format!(
                        "{{\"amount\":{},\"capacity\":{},\"role\":\"{:?}\"}}",
                        e.amount,
                        e.capacity,
                        e.sides.io()
                    )
                });
                let tanks: Vec<String> = caps
                    .tanks
                    .iter()
                    .map(|t| {
                        format!(
                            "{{\"fluid\":{},\"amount\":{},\"capacity\":{},\"role\":\"{:?}\"}}",
                            t.fluid.as_deref().map_or("null".to_string(), json_str),
                            t.amount,
                            t.capacity,
                            t.sides.io()
                        )
                    })
                    .collect();
                let kinetic = caps.kinetic.as_ref().map_or("null".to_string(), |k| {
                    format!(
                        "{{\"speed\":{},\"capacity\":{},\"impact\":{}}}",
                        k.speed, k.stress_capacity, k.stress_impact
                    )
                });
                // Les cases occupées : `[case, objet, nombre, usure]`.
                let inventory: Vec<String> = caps.inventory.as_ref().map_or_else(Vec::new, |i| {
                    i.slots
                        .iter()
                        .enumerate()
                        .filter_map(|(n, s)| {
                            s.as_ref().map(|s| {
                                format!("[{n},{},{},{}]", json_str(&s.item), s.count, s.damage)
                            })
                        })
                        .collect()
                });
                Ok(format!(
                    "{{\"ok\":true,\"block\":{block},\"power\":{power},\"energy\":{energy},\"tanks\":[{}],\"kinetic\":{kinetic},\"items\":[{}]}}",
                    tanks.join(","),
                    inventory.join(",")
                ))
            }
            Command::Assemble(pos) => {
                let id = self.assemble(pos).map_err(|e| anyhow::anyhow!(e))?;
                Ok(format!("{{\"ok\":true,\"ship\":{}}}", id.0))
            }
            Command::Land(id) => {
                let placed = self
                    .disassemble(engine_voxel::GridId(id))
                    .map_err(|e| anyhow::anyhow!(e))?;
                Ok(format!("{{\"ok\":true,\"placed\":{placed}}}"))
            }
            Command::Steer(id, value) => {
                if let Some(mut control) = self.ecs.get_resource_mut::<crate::ships::ShipControl>()
                {
                    control.forced = (value != 0.0).then_some((engine_voxel::GridId(id), value));
                }
                Ok(ok())
            }
            Command::Carts => Ok(self.carts_json()),
            Command::Cart(kind, pos) => {
                let look = self.camera.forward();
                let entity =
                    crate::carts::place(&mut self.ecs, kind, pos, look).with_context(|| {
                        format!("cart : pas de rail en {} {} {}", pos.x, pos.y, pos.z)
                    })?;
                let id = self
                    .ecs
                    .get::<crate::carts::Cart>(entity)
                    .map_or(0, |c| c.id);
                Ok(format!("{{\"ok\":true,\"cart\":{id}}}"))
            }
            Command::Ride(id) => {
                let entity = match id {
                    Some(id) => self
                        .cart_by_id(id)
                        .with_context(|| format!("ride : pas de wagon {id}"))?,
                    None => {
                        let feet = player::body(&self.ecs, self.player).position;
                        let mut best: Option<(f32, engine_ecs::Entity)> = None;
                        self.ecs
                            .query::<(&crate::components::Transform, &crate::carts::Cart)>()
                            .for_each(|e, (t, c)| {
                                let d = t.position.distance(feet);
                                if c.kind == crate::carts::CartKind::Minecart
                                    && c.rider.is_none()
                                    && best.is_none_or(|(b, _)| d < b)
                                {
                                    best = Some((d, e));
                                }
                            });
                        best.map(|(_, e)| e)
                            .context("ride : aucun wagonnet libre")?
                    }
                };
                if !self.ride(entity) {
                    bail!(
                        "ride : impossible de monter (vol, wagon plein ou ce n'est pas un wagonnet)"
                    );
                }
                Ok(ok())
            }
            Command::Dismount => {
                if !self.dismount() {
                    bail!("dismount : le joueur n'est pas dans un wagon");
                }
                Ok(ok())
            }
            Command::Couple(a, b) => {
                let (Some(x), Some(y)) = (self.cart_by_id(a), self.cart_by_id(b)) else {
                    bail!("couple : wagon inconnu (voir carts)");
                };
                let linked = crate::carts::toggle_coupling(&self.ecs, x, y)
                    .context("couple : il faut deux wagons différents")?;
                Ok(format!("{{\"ok\":true,\"coupled\":{linked}}}"))
            }
            Command::CartStock(id, item, count) => {
                let entity = self
                    .cart_by_id(id)
                    .with_context(|| format!("cartstock : pas de wagon {id}"))?;
                let mut cargo = self
                    .ecs
                    .get_mut::<crate::carts::CartCargo>(entity)
                    .context("cartstock : ce wagon n'a pas d'inventaire")?;
                let put = cargo.0.insert(None, &item, count, false);
                Ok(format!("{{\"ok\":true,\"put\":{put}}}"))
            }
            Command::Ships => {
                let subs = self.mods.subgrids.borrow();
                let list: Vec<String> = subs
                    .ids()
                    .iter()
                    .map(|id| {
                        let g = subs.get(*id).expect("identifiant connu").borrow();
                        format!(
                            "{{\"id\":{},\"blocks\":{},\"pos\":{},\"yaw\":{:.3},\"velocity\":{}}}",
                            id.0,
                            g.len(),
                            vec3(g.position),
                            g.yaw,
                            vec3(g.velocity)
                        )
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"ships\":[{}]}}", list.join(",")))
            }
            Command::Networks => {
                let stats = self.mods.machines.networks().stats();
                Ok(format!(
                    "{{\"ok\":true,\"networks\":{},\"conduits\":{},\"endpoints\":{},\"rebuilt\":{}}}",
                    stats.networks, stats.conduits, stats.endpoints, stats.rebuilt
                ))
            }
            Command::Structures(radius) => {
                let feet = player::body(&self.ecs, self.player).position;
                let streamer = self.streamer();
                let generator = streamer.generator();
                let list: Vec<String> = generator
                    .structure_sites_near(feet.x.floor() as i32, feet.z.floor() as i32, radius)
                    .iter()
                    .map(|site| {
                        let key = generator
                            .structures()
                            .and_then(|s| s.defs().get(site.def))
                            .map_or("?", |d| d.key.as_str());
                        let y = generator.column(site.x, site.z).height;
                        format!(
                            "{{\"kind\":{},\"pos\":[{},{},{}]}}",
                            json_str(key),
                            site.x,
                            y,
                            site.z
                        )
                    })
                    .collect();
                Ok(format!(
                    "{{\"ok\":true,\"structures\":[{}]}}",
                    list.join(",")
                ))
            }
            Command::Weather(sky) => {
                self.weather.set(sky);
                Ok(format!("{{\"ok\":true,\"weather\":\"{}\"}}", sky.name()))
            }
            Command::Damage(amount, pierce) => {
                let dealt = self.hurt_player_pierced(
                    amount,
                    crate::survival::DamageCause::Command,
                    crate::attributes::Pierce::flat(pierce),
                );
                Ok(format!(
                    "{{\"ok\":true,\"dealt\":{}}}",
                    dealt.map_or("null".into(), |d| format!("{d:.2}"))
                ))
            }
            Command::Mobs => {
                let registry = self
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .clone();
                let mut mobs: Vec<(f32, String)> = Vec::new();
                let eye = player::eye(&self.ecs, self.player, 1.0);
                self.ecs
                    .query::<(&crate::components::Transform, &crate::components::Health, &crate::components::Mob)>()
                    .for_each(|_, (transform, health, mob)| {
                        let key = registry.get(mob.kind).map_or("?", |d| d.key.as_str());
                        let distance = transform.position.distance(eye);
                        mobs.push((
                            distance,
                            format!(
                                "{{\"kind\":{},\"pos\":{},\"health\":{:.1},\"distance\":{distance:.1}}}",
                                json_str(key),
                                vec3(transform.position),
                                health.current
                            ),
                        ));
                    });
                mobs.sort_by(|a, b| a.0.total_cmp(&b.0));
                let list: Vec<String> = mobs.into_iter().take(20).map(|(_, m)| m).collect();
                Ok(format!("{{\"ok\":true,\"mobs\":[{}]}}", list.join(",")))
            }
            Command::Use => {
                if self.use_cart() || self.use_charm() || self.use_guide() {
                    return Ok(ok());
                }
                let hit = self.aimed()?;
                if self.use_bed(hit.block_pos) {
                    return Ok(self.state_json());
                }
                if self.open_container() || self.use_machine() || self.use_mod_item() {
                    return Ok(ok());
                }
                let pos = self.place_at_target().context("use : rien à faire ici")?;
                Ok(format!("{{\"ok\":true,\"placed\":{}}}", ivec3(pos)))
            }
            Command::Summon(key) => {
                let kind = self
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .id(&key)
                    .with_context(|| format!("summon : créature inconnue « {key} »"))?;
                let feet = player::body(&self.ecs, self.player).position;
                let at = feet + self.camera.forward_flat() * 3.0;
                crate::mobs::spawn_mob(&mut self.ecs, kind, at).context("summon : impossible")?;
                Ok(format!("{{\"ok\":true,\"pos\":{}}}", vec3(at)))
            }
            Command::Xp(points) => {
                self.grant_xp(points);
                Ok(self.state_json())
            }
            Command::UiTexts => Ok(format!(
                "{{\"ok\":true,\"texts\":{}}}",
                self.ui.input.texts_json()
            )),
            Command::UiClick(x, y, right) => {
                let button = if right {
                    engine_render::egui::PointerButton::Secondary
                } else {
                    engine_render::egui::PointerButton::Primary
                };
                self.ui.input.click(engine_render::egui::pos2(x, y), button);
                Ok(ok())
            }
            Command::UiClickText(text, right) => {
                let at = self.ui.input.find(&text).with_context(|| {
                    format!("clicktext : aucun texte « {text} » à l'écran (voir texts)")
                })?;
                let button = if right {
                    engine_render::egui::PointerButton::Secondary
                } else {
                    engine_render::egui::PointerButton::Primary
                };
                self.ui.input.click(at, button);
                Ok(format!(
                    "{{\"ok\":true,\"x\":{:.0},\"y\":{:.0}}}",
                    at.x, at.y
                ))
            }
            Command::UiType(text) => {
                self.ui.input.text(&text);
                Ok(ok())
            }
            Command::UiKey(name, mods) => {
                let key = engine_render::egui::Key::from_name(&name)
                    .with_context(|| format!("key : touche inconnue « {name} »"))?;
                let mut modifiers = engine_render::egui::Modifiers::NONE;
                for m in &mods {
                    match m.as_str() {
                        "ctrl" => modifiers.ctrl = true,
                        "shift" => modifiers.shift = true,
                        "alt" => modifiers.alt = true,
                        other => bail!("key : modificateur inconnu « {other} » (ctrl, shift, alt)"),
                    }
                }
                self.ui.input.key(key, modifiers);
                Ok(ok())
            }
            Command::Console(line) => {
                match line {
                    Some(line) => self.ui.console.submitted.push(line),
                    None => self.open_console(),
                }
                Ok(ok())
            }
            Command::Travel(key) => {
                if self.travel(&key) {
                    Ok(ok())
                } else {
                    bail!(
                        "voyage impossible vers « {key} » (dimensions : {})",
                        self.dimension_names().join(", ")
                    )
                }
            }
            Command::Storage(command) => self.storage_command(command),
            Command::Items(command) => self.item_command(command),
            Command::Adventure(command) => self.adventure_command(command),
            Command::Enchanting(command) => self.enchant_command(command),
            Command::Quests(command) => self.quest_command(command),
            Command::Mods => {
                let disabled = self.mods.disabled();
                let rows: Vec<String> = self
                    .mods
                    .mod_stats()
                    .into_iter()
                    .map(|(id, s)| {
                        let off = disabled
                            .iter()
                            .find(|(m, _)| *m == id)
                            .map_or("null".to_string(), |(_, why)| json_str(why));
                        format!(
                            "{{\"id\":{},\"disabled\":{off},\"budget\":{},\"fuel_last_tick\":{},\"fuel_peak_tick\":{},\"calls\":{},\"skipped_calls\":{},\"overrun_ticks\":{}}}",
                            json_str(&id),
                            engine_mods::TICK_BUDGET,
                            s.fuel_last_tick,
                            s.fuel_peak_tick,
                            s.calls,
                            s.skipped_calls,
                            s.overrun_ticks
                        )
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"mods\":[{}]}}", rows.join(",")))
            }
            Command::Pause => {
                self.mode = Mode::Paused(crate::menus::PausePage::Main);
                Ok(ok())
            }
            Command::Resume => {
                self.leave_mod_options();
                self.mode = Mode::Playing;
                Ok(ok())
            }
            Command::ModOptions(id) => {
                self.open_mod_options(&id);
                Ok(ok())
            }
            Command::OptionsEvent(kind, id, value) => {
                self.ui.options_events.push((kind, id, value));
                Ok(ok())
            }
            Command::Respawn => {
                if !matches!(self.mode, Mode::Dead(_)) {
                    bail!("respawn : le joueur n'est pas mort");
                }
                self.respawn();
                Ok(self.state_json())
            }
            Command::Attack => {
                let eye = player::eye(&self.ecs, self.player, 1.0);
                let forward = self.camera.forward();
                // Un wagon visé : un coup (le second le casse).
                if crate::mobs::hit_mob(&self.ecs, eye, forward, self.gear.reach()).is_none()
                    && self.punch_cart()
                {
                    return Ok(ok());
                }
                let (mob, _) = crate::mobs::hit_mob(&self.ecs, eye, forward, self.gear.reach())
                    .context("attack : aucune créature ni aucun wagon visé à portée")?;
                let strike = self
                    .strike_mob(mob, forward)
                    .context("attack : la créature vient d'être frappée (invulnérable)")?;
                self.exert(self.hunger_rules.attack);
                Ok(format!(
                    "{{\"ok\":true,\"damage\":{:.1},\"crit\":{},\"healed\":{:.2}}}",
                    strike.damage, strike.crit, strike.healed
                ))
            }
            Command::Swing => {
                self.hand.punch();
                Ok(ok())
            }
            Command::Eat => {
                // L'aliment en main, sinon le premier de l'inventaire.
                let items = &self.content.items;
                let is_food = |slot: &Option<crate::inventory::ItemStack>| {
                    slot.is_some_and(|s| items.get(s.item).food.is_some())
                };
                let index = if is_food(&self.inventory.slots[self.inventory.selected]) {
                    self.inventory.selected
                } else {
                    self.inventory
                        .slots
                        .iter()
                        .position(is_food)
                        .context("eat : aucun aliment dans l'inventaire")?
                };
                if !self.hunger.hungry(&self.hunger_rules) {
                    bail!("eat : rassasié");
                }
                let slot = &mut self.inventory.slots[index];
                let Some(stack) = slot.as_mut() else {
                    bail!("eat : case vide");
                };
                let food = items.get(stack.item).food.context("eat : pas un aliment")?;
                stack.count -= 1;
                if stack.count == 0 {
                    *slot = None;
                }
                self.hunger
                    .eat(&self.hunger_rules, food.nutrition, food.saturation);
                Ok(self.state_json())
            }
            Command::Hunger(food) => {
                self.hunger.food = food.clamp(0.0, self.hunger_rules.max_food);
                self.hunger.saturation = self.hunger.saturation.min(self.hunger.food);
                Ok(self.state_json())
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
    pub(crate) fn aimed(&mut self) -> Result<engine_voxel::RayHit> {
        let eye = player::eye(&self.ecs, self.player, 1.0);
        let hit = engine_voxel::raycast(
            self.streamer().world(),
            eye,
            self.camera.forward(),
            self.gear.reach(),
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
            Mode::Dead(_) => "dead",
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
\"dimension\":{},\"biome\":{},\"target\":{target},\"selected\":{selected},\"walking\":{},\
\"chunks\":{{\"loaded\":{},\"queued\":{},\"idle\":{}}},\
\"render\":{{\"drawn\":{},\"hidden\":{},\"total\":{},\"triangles\":{},\"cave_culling\":{}}},\
\"survival\":{{\"air\":{:.2},\"burning\":{:.2},\"protection\":{},\
\"food\":{:.2},\"saturation\":{:.2},\"exhaustion\":{:.3},\"bed\":{},\"death\":{},\"xp\":{},\"level\":{}}}}}",
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
            json_str(&self.dims.current.key),
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
            self.hunger.food,
            self.hunger.saturation,
            self.hunger.exhaustion,
            self.bed.map_or("null".into(), ivec3),
            match &self.mode {
                Mode::Dead(message) => json_str(message),
                _ => "null".into(),
            },
            self.experience.points,
            self.experience.level().0,
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
pub(crate) fn json_str(text: &str) -> String {
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
        assert_eq!(
            parse("inspect 1 2 3").unwrap(),
            Command::Inspect(IVec3::new(1, 2, 3))
        );
        assert_eq!(
            parse("click 10 20 right").unwrap(),
            Command::UiClick(10.0, 20.0, true)
        );
        assert_eq!(
            parse("clicktext Ranger tout").unwrap(),
            Command::UiClickText("Ranger tout".into(), false)
        );
        assert_eq!(
            parse("clicktext Ranger right").unwrap(),
            Command::UiClickText("Ranger".into(), true)
        );
        assert_eq!(
            parse("type /mana 5").unwrap(),
            Command::UiType("/mana 5".into())
        );
        assert_eq!(
            parse("key Enter ctrl").unwrap(),
            Command::UiKey("Enter".into(), vec!["ctrl".into()])
        );
        assert!(parse("key").is_err() && parse("type").is_err());
        assert_eq!(parse("networks").unwrap(), Command::Networks);
        assert_eq!(
            parse("assemble 1 2 3").unwrap(),
            Command::Assemble(IVec3::new(1, 2, 3))
        );
        assert_eq!(parse("land 2").unwrap(), Command::Land(2));
        assert_eq!(parse("steer 1 -2").unwrap(), Command::Steer(1, -1.0));
        assert_eq!(parse("ships").unwrap(), Command::Ships);
        assert_eq!(
            parse("weather storm").unwrap(),
            Command::Weather(crate::weather::Sky::Storm)
        );
        assert!(parse("weather hail").is_err());
    }

    #[test]
    fn bad_commands_explain_why() {
        for (line, expected) in [
            ("", "vide"),
            ("dance", "inconnue"),
            ("goto 1 2", "arguments"),
            ("goto 1 deux 3", "« deux »"),
            ("select 0", "1 à 36"),
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
