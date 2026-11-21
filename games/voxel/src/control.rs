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
use engine_voxel::BlockId;
use glam::{IVec3, Vec3};

use crate::intents::Outcome;
use crate::player::{self, MoveMode, SPRINT_SPEED, WALK_SPEED};
use crate::protocol::ClientMsg;
use crate::{Mode, VoxelGame};

/// Le port par défaut (`VOXEL_CONTROL=1`).
pub const DEFAULT_PORT: u16 = 7878;

/// L'aide renvoyée par `help`.
const HELP: &str = "state | goto x y z | look yaw pitch | lookat x y z \
| walk secondes [forward|back|left|right] [sprint] [jump] | jump | stop | wait secondes \
| block x y z | break [x y z] | place [x y z bloc] | give objet [nombre] | select 1-36 \
| inventory | mode walk|fly | time 0-1 | culling on|off | damage n [perce] | eat | hold left|right|off | hunger n | mobs | attack | swing | use | mods | swing | travel dimension | carts | cart sorte x y z | ride [numéro] | dismount | couple a b | cartstock numéro objet [nombre] | console [texte] | texts | click x y [right] | clicktext texte [right] | type texte | key Nom [ctrl] [shift] [alt] | respawn | pause | resume | modoptions mod | optionsevent button|toggle|slider id [valeur] | backpack | placebag | deposit | topup | takeall | sort | quickmove 1-36 | container x y z | bag [numéro] | drops | capture on|off | xp n | summon créature | screenshot fichier.png [largeur hauteur] | enchant sorte niveau [sorte niveau...] | attributes [open|close|modified on|off] | affix attribut fixe [part] | hover x y | equip n | sheet | throw | loot [rareté] [objet] | gem sorte qualité [n] | tier n | tiers | progress kills|bosses|rarity|depth n | elite [créature] | invader [créature] | elites | slay [all] | cursor n|clear | open tiers|salvage|reforge | sockets n | clearinv | pending x y z objet [rareté_min] | table x y z | shelves x y z [étagère] [n] | offers x y z | screen enchant x y z|anvil|library x y z|guide | book sorte niveau [...] | libadd x y z sorte niveau | library x y z | points n | charms | quests [chapitre] | quest done|claim|check|handin|choose|scan|reset|pin|select|ui|view|close ... | questpin id | screen quests [chapitre] | shape forme [n|all] | rebuild [all] | shaping | pieces x y z | ghost [nom] [x y z] | ghost remove [nom] | ghost list | quit";

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
    /// Manger l'aliment tenu : le bouton de repas est tenu le temps d'un repas (1,6 s de jeu).
    Eat,
    /// Tenir un bouton de la souris sur ce qui est visé, comme si la souris était capturée :
    /// `Some(false)` le gauche (miner, frapper), `Some(true)` le droit (utiliser, manger) ;
    /// `None` les lâche.
    Hold(Option<bool>),
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
    /// Les commandes des formes : tailler, reconstituer, lire une case (voir `shape_ctl.rs`).
    Shapes(crate::shape_ctl::Command),
    /// Les joueurs fantômes : ajouter, retirer, lister (voir `ghosts.rs`).
    Ghosts(crate::ghosts::Command),
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
        "hold" => {
            count(&[1])?;
            match args[0] {
                "left" => Command::Hold(Some(false)),
                "right" => Command::Hold(Some(true)),
                "off" => Command::Hold(None),
                other => bail!("hold : left, right ou off (pas « {other} »)"),
            }
        }
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
                            None => match crate::shape_ctl::parse(other, &args) {
                                Some(command) => Command::Shapes(command?),
                                None => match crate::ghosts::parse(other, &args) {
                                    Some(command) => Command::Ghosts(command?),
                                    None => {
                                        bail!("commande inconnue « {other} » (voir `help`)")
                                    }
                                },
                            },
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
    /// `eat` : le temps de jeu qu'il reste avant de lâcher le bouton de repas.
    eat_release: Option<f32>,
    /// `hold` : les boutons de la souris tenus par la commande.
    pub(crate) hold: Option<Hold>,
}

/// Les boutons que `hold` tient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hold {
    pub left: bool,
    pub right: bool,
    /// Le premier tour : le bouton vient d'être pressé (un clic, pas seulement tenu).
    pub fresh: bool,
}

impl VoxelGame {
    /// L'intention de marche commandée pour ce pas de simulation, s'il y en a une.
    pub(crate) fn remote_intent(&mut self, dt: f32) -> Option<(Vec3, bool)> {
        let can_sprint = self.server.can_sprint();
        let walk = self.remote.walk.as_mut()?;
        let forward = self.client.camera.forward_flat();
        let right = forward.cross(Vec3::Y);
        let direction = (forward * walk.forward + right * walk.right).normalize_or_zero();
        // Comme au clavier : affamé, on ne court plus.
        let speed = if walk.sprint && can_sprint {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        } * self.server.players.me_mut().gear.move_speed();
        let intent = (direction * speed, walk.jump);
        walk.seconds -= dt;
        if walk.seconds <= 0.0 {
            self.remote.walk = None;
        }
        Some(intent)
    }

    /// Fait avancer les `wait` : ceux qui sont écoulés répondent avec l'état du jeu.
    pub(crate) fn update_remote(&mut self, dt: f32) {
        if let Some(left) = &mut self.remote.eat_release {
            *left -= dt;
            if *left <= 0.0 {
                self.remote.eat_release = None;
                self.send_now(ClientMsg::Eat { held: false });
            }
        }
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
    pub(crate) fn run_command(&mut self, request: Request) {
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
        let json = match self.execute(command) {
            Ok(json) => json,
            Err(err) => error(&format!("{err:#}")),
        };
        request.reply(json);
    }

    /// Exécute une commande. Celles qui simulent le joueur (`use`, `attack`, `select`, `eat`,
    /// `respawn`, `throw`, `place` sans coordonnées) fabriquent les mêmes messages que ses entrées ;
    /// `break` et `place x y z bloc` passent par la console (`Command`), comme `/give` ou `/tp`.
    pub(crate) fn execute(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Break(pos) => {
                let pos = match pos {
                    Some(pos) => pos,
                    None => self.aimed()?.block_pos,
                };
                self.through_console(format!("break {} {} {}", pos.x, pos.y, pos.z))
            }
            Command::Place(Some((pos, key))) => {
                self.through_console(format!("place {} {} {} {key}", pos.x, pos.y, pos.z))
            }
            Command::Place(None) => {
                self.require_playing("place")?;
                self.server.outcome = Outcome::Nothing;
                let aim = self.body_aim();
                self.send_now(ClientMsg::Use {
                    aim,
                    sneaking: false,
                });
                match self.server.outcome {
                    Outcome::Placed(pos) => Ok(format!("{{\"ok\":true,\"pos\":{}}}", ivec3(pos))),
                    Outcome::Nothing => {
                        bail!("place : impossible ici (rien en main, pas de place, pas de support)")
                    }
                    _ => Ok("{\"ok\":true,\"used\":true}".to_string()),
                }
            }
            Command::Use => {
                self.require_playing("use")?;
                self.server.outcome = Outcome::Nothing;
                let aim = self.body_aim();
                self.send_now(ClientMsg::Use {
                    aim,
                    sneaking: false,
                });
                match self.server.outcome {
                    Outcome::Placed(pos) => {
                        Ok(format!("{{\"ok\":true,\"placed\":{}}}", ivec3(pos)))
                    }
                    Outcome::Nothing => bail!("use : rien à faire ici"),
                    _ => Ok(ok()),
                }
            }
            Command::Attack => {
                self.require_playing("attack")?;
                let aim = self.body_aim();
                // Ce que le client dit viser : la créature, sinon le wagon, sous le regard.
                let entity = self
                    .claimed_target(aim)
                    .context("attack : aucune créature ni aucun wagon visé à portée")?;
                self.server.outcome = Outcome::Nothing;
                self.send_now(ClientMsg::Attack { aim, entity });
                match self.server.outcome {
                    Outcome::Struck(strike) => Ok(format!(
                        "{{\"ok\":true,\"damage\":{:.1},\"crit\":{},\"healed\":{:.2}}}",
                        strike.damage, strike.crit, strike.healed
                    )),
                    Outcome::Punched => Ok(ok()),
                    _ => bail!("attack : la créature vient d'être frappée (invulnérable)"),
                }
            }
            Command::Select(slot) => {
                self.require_playing("select")?;
                self.send_now(ClientMsg::SelectSlot(slot.min(usize::from(u8::MAX)) as u8));
                Ok(self.state_json())
            }
            Command::Eat => {
                self.require_playing("eat")?;
                // L'aliment en main, sinon le premier de l'inventaire.
                let items = &self.client.content.items;
                let is_food = |slot: &Option<crate::inventory::ItemStack>| {
                    slot.is_some_and(|s| items.get(s.item).food.is_some())
                };
                let inventory = &self.server.players.me().inventory;
                let index = if is_food(&inventory.slots[inventory.selected]) {
                    inventory.selected
                } else {
                    inventory
                        .slots
                        .iter()
                        .position(is_food)
                        .context("eat : aucun aliment dans l'inventaire")?
                };
                if !self
                    .server
                    .players
                    .me()
                    .hunger
                    .hungry(&self.server.hunger_rules)
                {
                    bail!("eat : rassasié");
                }
                if index != self.server.players.me().inventory.selected {
                    self.send_now(ClientMsg::SelectSlot(index as u8));
                }
                self.send_now(ClientMsg::Eat { held: true });
                // Le bouton reste tenu le temps d'un repas, puis il est lâché (`update_remote`).
                self.remote.eat_release = Some(self.server.hunger_rules.eat_time + 0.15);
                Ok(self.state_json())
            }
            Command::Hold(button) => {
                self.require_playing("hold")?;
                self.remote.hold = button.map(|right| Hold {
                    left: !right,
                    right,
                    fresh: true,
                });
                Ok(ok())
            }
            Command::Respawn => {
                if !matches!(self.mode, Mode::Dead(_)) {
                    bail!("respawn : le joueur n'est pas mort");
                }
                self.send_now(ClientMsg::Respawn);
                Ok(self.state_json())
            }
            Command::Swing => {
                self.require_playing("swing")?;
                self.client.hand.punch();
                Ok(ok())
            }
            other => self.execute_inner(other),
        }
    }

    /// Une ligne de console envoyée au simulateur dans un message `Command` ; la réponse JSON de la
    /// commande revient par `command_reply`.
    fn through_console(&mut self, line: String) -> Result<String> {
        self.require_playing(line.split_whitespace().next().unwrap_or("commande"))?;
        self.server.command_reply = None;
        self.send_now(ClientMsg::Command(line));
        self.server
            .command_reply
            .take()
            .and_then(|reply| reply.json)
            .unwrap_or_else(|| Err(anyhow::anyhow!("la commande n'a pas répondu")))
    }

    fn require_playing(&self, name: &str) -> Result<()> {
        if self.mode != Mode::Playing {
            bail!(
                "{name} : pas en jeu (menu ou pause) : lancer un monde, par exemple avec VOXEL_WORLD"
            );
        }
        Ok(())
    }

    /// Ce que le regard désigne pour un coup : la créature, sinon le wagon, plus près que le bloc. Comme
    /// la fenêtre, `ctl` choisit dans ce que le client voit (la réplique des entités).
    fn claimed_target(&self, aim: crate::protocol::Aim) -> Option<crate::entities::NetId> {
        let acting = self
            .server
            .resolve_aim(self.server.players.local_id(), aim)?;
        let block_distance = acting
            .hit
            .map_or(self.server.players.me().gear.reach(), |h| h.distance);
        self.client
            .view
            .entities
            .hit(acting.eye, acting.forward, block_distance)
    }

    /// Les commandes qui ne simulent pas une entrée du joueur (et celles de la console).
    pub(crate) fn execute_inner(&mut self, command: Command) -> Result<String> {
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
                self.place_eye(feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT);
                self.server.forget_last_move();
                Ok(self.state_json())
            }
            Command::Look(yaw, pitch) => {
                self.client.camera.yaw = yaw.to_radians();
                self.client.camera.pitch = pitch.to_radians().clamp(
                    -engine_render::Camera::MAX_PITCH,
                    engine_render::Camera::MAX_PITCH,
                );
                self.client.target = None;
                Ok(self.state_json())
            }
            Command::LookAt(point) => {
                let eye = self.client.body.eye(1.0);
                let direction = (point - eye).normalize_or_zero();
                if direction == Vec3::ZERO {
                    bail!("lookat : le point est déjà dans l'œil");
                }
                self.client.camera.yaw = direction.z.atan2(direction.x);
                self.client.camera.pitch = direction.y.asin().clamp(
                    -engine_render::Camera::MAX_PITCH,
                    engine_render::Camera::MAX_PITCH,
                );
                self.client.target = None;
                Ok(self.state_json())
            }
            Command::Walk(walk) => {
                if self.client.body.mode == MoveMode::Fly {
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
                let streamer = self.server.streamer();
                let world = streamer.world();
                let block = world.block(pos);
                let (sky, light) = world.light(pos);
                Ok(format!(
                    "{{\"ok\":true,\"pos\":{},\"block\":{},\"loaded\":{},\"light\":[{sky},{light}]}}",
                    ivec3(pos),
                    json_str(&self.client.registry.def(block).key),
                    streamer.is_generated(engine_voxel::World::chunk_containing(pos.as_vec3())),
                ))
            }
            Command::Break(pos) => {
                let pos = match pos {
                    Some(pos) => pos,
                    None => self.aimed()?.block_pos,
                };
                let block = self.server.streamer().world().block(pos);
                if block.is_air() {
                    bail!("break : rien à casser en {}", ivec3(pos));
                }
                let key = self.client.registry.def(block).key.clone();
                if !self.server.break_block(pos) {
                    bail!("break : {key} est incassable ou hors du monde chargé");
                }
                Ok(format!(
                    "{{\"ok\":true,\"broken\":{},\"pos\":{}}}",
                    json_str(&key),
                    ivec3(pos)
                ))
            }
            Command::Place(Some((pos, key))) => {
                let block = self
                    .client
                    .registry
                    .id(&key)
                    .with_context(|| format!("place : bloc inconnu « {key} »"))?;
                if block == BlockId::AIR {
                    bail!("place : pour enlever un bloc, utiliser break");
                }
                if engine_voxel::bed::is_part(&self.client.registry, block) {
                    bail!("place : utiliser core:bed pour poser le lit entier");
                }
                if engine_voxel::door::is_door(self.client.registry.def(block))
                    && !self.client.registry.def(block).placeable
                {
                    bail!("place : utiliser l'objet de porte pour poser les deux moitiés");
                }
                if let Some(parts) =
                    self.server
                        .paired_placement(pos, block, self.client.camera.forward())
                {
                    self.server
                        .place_pair(parts)
                        .context("place : deux cases libres avec un support sont nécessaires")?;
                } else if !self.server.edit(pos, block) {
                    bail!("place : {} est hors du monde chargé", ivec3(pos));
                }
                Ok(format!("{{\"ok\":true,\"pos\":{}}}", ivec3(pos)))
            }
            Command::Stock(pos, slot, key, amount, wear) => {
                let mut streamer = self.server.streamer_mut();
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
                    .client
                    .content
                    .items
                    .id(&key)
                    .with_context(|| format!("give : objet inconnu « {key} »"))?;
                let left = self.server.players.me_mut().inventory.add(
                    item,
                    amount,
                    &self.client.content.items,
                );
                Ok(format!(
                    "{{\"ok\":true,\"given\":{},\"full\":{left}}}",
                    amount - left
                ))
            }
            Command::Inventory => {
                let items = &self.client.content.items;
                let slots: Vec<String> = self
                    .server
                    .players
                    .me_mut()
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
                    self.server.players.me_mut().inventory.selected + 1,
                    slots.join(",")
                ))
            }
            Command::Mode(mode) => {
                if self.client.body.mode != mode {
                    self.toggle_fly();
                }
                self.remote.walk = None;
                Ok(self.state_json())
            }
            Command::Time(time) => {
                self.server.day.time = time;
                Ok(self.state_json())
            }
            Command::Inspect(pos) => {
                let streamer = self.server.streamer();
                let block = json_str(&self.client.registry.def(streamer.world().block(pos)).key);
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
                let id = self.server.assemble(pos).map_err(|e| anyhow::anyhow!(e))?;
                Ok(format!("{{\"ok\":true,\"ship\":{}}}", id.0))
            }
            Command::Land(id) => {
                let placed = self
                    .server
                    .disassemble(engine_voxel::GridId(id))
                    .map_err(|e| anyhow::anyhow!(e))?;
                Ok(format!("{{\"ok\":true,\"placed\":{placed}}}"))
            }
            Command::Steer(id, value) => {
                if let Some(mut control) = self
                    .server
                    .ecs
                    .get_resource_mut::<crate::ships::ShipControl>()
                {
                    control.forced = (value != 0.0).then_some((engine_voxel::GridId(id), value));
                }
                Ok(ok())
            }
            Command::Carts => Ok(self.carts_json()),
            Command::Cart(kind, pos) => {
                let look = self.client.camera.forward();
                let entity = crate::carts::place(&mut self.server.ecs, kind, pos, look)
                    .with_context(|| {
                        format!("cart : pas de rail en {} {} {}", pos.x, pos.y, pos.z)
                    })?;
                let id = self
                    .server
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
                        let feet = self.client.body.body.position;
                        let mut best: Option<(f32, engine_ecs::Entity)> = None;
                        self.server
                            .ecs
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
                if !self.server.ride(entity) {
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
                let linked = crate::carts::toggle_coupling(&self.server.ecs, x, y)
                    .context("couple : il faut deux wagons différents")?;
                Ok(format!("{{\"ok\":true,\"coupled\":{linked}}}"))
            }
            Command::CartStock(id, item, count) => {
                let entity = self
                    .cart_by_id(id)
                    .with_context(|| format!("cartstock : pas de wagon {id}"))?;
                let mut cargo = self
                    .server
                    .ecs
                    .get_mut::<crate::carts::CartCargo>(entity)
                    .context("cartstock : ce wagon n'a pas d'inventaire")?;
                let put = cargo.0.insert(None, &item, count, false);
                Ok(format!("{{\"ok\":true,\"put\":{put}}}"))
            }
            Command::Ships => {
                let subs = self.server.mods.subgrids.borrow();
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
                let stats = self.server.mods.machines.networks().stats();
                Ok(format!(
                    "{{\"ok\":true,\"networks\":{},\"conduits\":{},\"endpoints\":{},\"rebuilt\":{}}}",
                    stats.networks, stats.conduits, stats.endpoints, stats.rebuilt
                ))
            }
            Command::Structures(radius) => {
                let feet = self.client.body.body.position;
                let streamer = self.server.streamer();
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
                self.server.weather.set(sky);
                Ok(format!("{{\"ok\":true,\"weather\":\"{}\"}}", sky.name()))
            }
            Command::Damage(amount, pierce) => {
                let dealt = self.server.hurt_player_pierced(
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
                    .server
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .clone();
                let mut mobs: Vec<(f32, String)> = Vec::new();
                let eye = self.client.body.eye(1.0);
                self.server.ecs
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
            Command::Summon(key) => {
                let kind = self
                    .server
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .id(&key)
                    .with_context(|| format!("summon : créature inconnue « {key} »"))?;
                let feet = self.client.body.body.position;
                let at = feet + self.client.camera.forward_flat() * 3.0;
                crate::mobs::spawn_mob(&mut self.server.ecs, kind, at)
                    .context("summon : impossible")?;
                Ok(format!("{{\"ok\":true,\"pos\":{}}}", vec3(at)))
            }
            Command::Xp(points) => {
                self.server.grant_xp(points);
                Ok(self.state_json())
            }
            Command::UiTexts => Ok(format!(
                "{{\"ok\":true,\"texts\":{}}}",
                self.client.ui.input.texts_json()
            )),
            Command::UiClick(x, y, right) => {
                let button = if right {
                    engine_render::egui::PointerButton::Secondary
                } else {
                    engine_render::egui::PointerButton::Primary
                };
                self.client
                    .ui
                    .input
                    .click(engine_render::egui::pos2(x, y), button);
                Ok(ok())
            }
            Command::UiClickText(text, right) => {
                let at = self.client.ui.input.find(&text).with_context(|| {
                    format!("clicktext : aucun texte « {text} » à l'écran (voir texts)")
                })?;
                let button = if right {
                    engine_render::egui::PointerButton::Secondary
                } else {
                    engine_render::egui::PointerButton::Primary
                };
                self.client.ui.input.click(at, button);
                Ok(format!(
                    "{{\"ok\":true,\"x\":{:.0},\"y\":{:.0}}}",
                    at.x, at.y
                ))
            }
            Command::UiType(text) => {
                self.client.ui.input.text(&text);
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
                self.client.ui.input.key(key, modifiers);
                Ok(ok())
            }
            Command::Console(line) => {
                match line {
                    Some(line) => self.client.ui.console.submitted.push(line),
                    None => self.open_console(),
                }
                Ok(ok())
            }
            Command::Travel(key) => {
                if self.server.travel(&key) {
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
            Command::Shapes(command) => self.shape_command(command),
            Command::Ghosts(command) => self.ghost_command(command),
            Command::Mods => {
                let disabled = self.server.mods.disabled();
                let rows: Vec<String> = self
                    .server.mods
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
                // Seulement depuis la pause : un joueur mort réapparaît par `respawn`.
                if matches!(self.mode, Mode::Paused(_)) {
                    self.mode = Mode::Playing;
                }
                Ok(ok())
            }
            Command::ModOptions(id) => {
                self.open_mod_options(&id);
                Ok(ok())
            }
            Command::OptionsEvent(kind, id, value) => {
                self.client.ui.options_events.push((kind, id, value));
                Ok(ok())
            }
            Command::Hunger(food) => {
                self.server.players.me_mut().hunger.food =
                    food.clamp(0.0, self.server.hunger_rules.max_food);
                self.server.players.me_mut().hunger.saturation = self
                    .server
                    .players
                    .me_mut()
                    .hunger
                    .saturation
                    .min(self.server.players.me_mut().hunger.food);
                Ok(self.state_json())
            }
            Command::Culling(enabled) => {
                self.client.culling.enabled = enabled;
                Ok(ok())
            }
            Command::Screenshot(path, width, height) => {
                let (ui, _) = self.build_ui(crate::ui::offscreen_input(width, height));
                self.update_visibility(width as f32 / height as f32);
                let capture =
                    self.client
                        .renderer
                        .capture(&self.client.camera, width, height, Some(ui))?;
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
            // Ces commandes simulent le joueur : `execute` les envoie en messages.
            Command::Use
            | Command::Attack
            | Command::Select(_)
            | Command::Eat
            | Command::Hold(_)
            | Command::Respawn
            | Command::Swing
            | Command::Place(None) => unreachable!("routées par `execute`"),
            Command::Quit => {
                // L'application se ferme à la fin de l'image (voir `update`).
                self.client.quit_at = Some(std::time::Instant::now());
                Ok(ok())
            }
        }
    }

    /// Le bloc visé, recalculé maintenant (la commande a pu tourner la tête).
    pub(crate) fn aimed(&mut self) -> Result<engine_voxel::RayHit> {
        let eye = self.client.body.eye(1.0);
        let hit = engine_voxel::raycast(
            self.view_world(),
            eye,
            self.client.camera.forward(),
            self.server.players.me().gear.reach(),
        );
        self.client.target = hit;
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
        let feet = self.client.body.body.position;
        // L'état de la physique est dans le `Collider` (le `PlayerBody` est recréé).
        let collider = self
            .server
            .ecs
            .get::<crate::Collider>(self.server.players.entity())
            .map(|c| *c)
            .unwrap_or_default();
        let health = player::health(&self.server.ecs, self.server.players.entity());
        let streamer = self.server.streamer();
        let generator = streamer.generator();
        // Le biome là où l'on est : celui d'une grotte souterraine, sinon celui de la colonne.
        let biome = generator
            .biome(generator.biome_at(
                feet.x.floor() as i32,
                feet.y.floor() as i32,
                feet.z.floor() as i32,
            ))
            .name
            .clone();
        let target = match self.client.target {
            Some(hit) => format!(
                "{{\"pos\":{},\"block\":{},\"face\":{},\"distance\":{:.2}}}",
                ivec3(hit.block_pos),
                json_str(
                    &self
                        .client
                        .registry
                        .def(streamer.world().block(hit.block_pos))
                        .key
                ),
                ivec3(hit.normal),
                hit.distance
            ),
            None => "null".into(),
        };
        let selected = match self.server.players.me().inventory.selected_stack() {
            Some(stack) => format!(
                "{{\"slot\":{},\"item\":{},\"count\":{}}}",
                self.server.players.me().inventory.selected + 1,
                json_str(&self.client.content.items.get(stack.item).key),
                stack.count
            ),
            None => format!(
                "{{\"slot\":{},\"item\":null}}",
                self.server.players.me().inventory.selected + 1
            ),
        };
        let stats = self.stream_stats();
        format!(
            "{{\"ok\":true,\"mode\":\"{mode}\",\"move\":\"{}\",\"pos\":{},\"on_ground\":{},\"in_liquid\":{},\"frozen\":{},\
\"yaw\":{:.1},\"pitch\":{:.1},\"health\":[{:.1},{:.1}],\"time\":{:.3},\"clock\":{},\
\"dimension\":{},\"biome\":{},\"target\":{target},\"selected\":{selected},\"walking\":{},\
\"chunks\":{{\"loaded\":{},\"queued\":{},\"idle\":{}}},\
\"render\":{{\"drawn\":{},\"hidden\":{},\"total\":{},\"triangles\":{},\"cave_culling\":{}}},\
\"survival\":{{\"air\":{:.2},\"burning\":{:.2},\"protection\":{},\
\"food\":{:.2},\"saturation\":{:.2},\"exhaustion\":{:.3},\"bed\":{},\"death\":{},\"xp\":{},\"level\":{}}}}}",
            // Les mêmes mots que la commande `mode`.
            match self.client.body.mode {
                MoveMode::Walk => "walk",
                MoveMode::Fly => "fly",
            },
            vec3(feet),
            collider.on_ground,
            collider.in_liquid,
            collider.frozen,
            self.client.camera.yaw.to_degrees(),
            self.client.camera.pitch.to_degrees(),
            health.current,
            health.max,
            self.server.day.time,
            json_str(&self.server.day.clock()),
            json_str(&self.server.dims.current.key),
            json_str(&biome),
            self.remote.walk.is_some(),
            stats.loaded,
            stats.queued,
            self.world_idle(),
            self.client.last_render.meshes_drawn,
            self.client.last_render.meshes_hidden,
            self.client.last_render.meshes_total,
            self.client.last_render.triangles,
            self.client.culling.enabled,
            self.server.players.me().vitals.air,
            self.server.players.me().vitals.burning,
            self.server
                .players
                .me()
                .inventory
                .protection(&self.client.content.items),
            self.server.players.me().hunger.food,
            self.server.players.me().hunger.saturation,
            self.server.players.me().hunger.exhaustion,
            self.server.players.me().bed.map_or("null".into(), ivec3),
            match &self.mode {
                Mode::Dead(message) => json_str(message),
                _ => "null".into(),
            },
            self.server.players.me().experience.points,
            self.server.players.me().experience.level().0,
        )
    }
}

fn ok() -> String {
    "{\"ok\":true}".into()
}

fn error(message: &str) -> String {
    format!("{{\"ok\":false,\"error\":{}}}", json_str(message))
}

pub(crate) fn vec3(v: Vec3) -> String {
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

impl VoxelGame {
    // (lot 4b) Méthodes de la fenêtre hôte.
    /// Les commandes de la console : `break` et `place x y z bloc` s'exécutent ici, les autres
    /// comme depuis `ctl`.
    pub(crate) fn execute_from_console(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Break(_) | Command::Place(Some(_)) => self.execute_inner(command),
            other => self.execute(other),
        }
    }
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
