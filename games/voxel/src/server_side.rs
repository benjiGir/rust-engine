//! L'état du serveur, à part de celui de la fenêtre (lot 4 du multijoueur, voir
//! `docs/plan-multijoueur.md`) : le monde, l'ECS, les mods, les joueurs, et ce qui répond à leurs
//! intentions. Il ne lit jamais la fenêtre (`ClientSide`) : ce qu'elle doit montrer part en messages.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use engine_ecs::Schedule;
use engine_voxel::ChunkStreamer;
use glam::IVec3;

use crate::{
    DayCycle, EventReaders, backpacks, dimensions, intents, modding, players, potions, protocol,
    stress, survival, trading, villagers, weather,
};

/// L'état du serveur : le monde et ce qui le fait vivre (lot 4 du multijoueur,
/// `docs/plan-multijoueur.md`). Il ne lit jamais la fenêtre (`ClientSide`).
pub(crate) struct VoxelServer {
    /// Le temps depuis le dernier pas des charmes, et le nombre de pas (voir `charms.rs`).
    pub(crate) charm_clock: f32,
    pub(crate) charm_beats: u32,
    /// Ce qui a été envoyé à chaque joueur, et les envois de chaque pas (voir `server_sends.rs`).
    pub(crate) sends: crate::server_sends::ServerSends,
    /// Un script de mod peut ouvrir son menu, seulement pendant le clic droit qui le demande.
    pub(crate) allow_mod_menu: bool,
    /// Les durées des éditions de l'image (pour les mesures de la fenêtre).
    pub(crate) edit_times: Vec<std::time::Duration>,
    /// Un changement de dimension demandé : la fenêtre recharge le monde (voir `dimensions.rs`).
    pub(crate) travel: Option<crate::launch::Request>,
    /// Les lignes de console à exécuter par la fenêtre hôte (voir `intents.rs::handle_command`).
    pub(crate) pending_commands: Vec<String>,
    /// Les blocs, et le reste du contenu (objets, recettes...) : partagés par `Arc`.
    pub(crate) registry: Arc<engine_voxel::BlockRegistry>,
    pub(crate) content: crate::content::Content,
    /// Le monde ECS : le joueur, les créatures, et des ressources partagées par les
    /// systèmes (le streamer des chunks, le registre des créatures, les événements).
    pub(crate) ecs: engine_ecs::World,
    pub(crate) schedule: Schedule,
    /// L'état du hasard du jeu (butin, objets lâchés).
    pub(crate) random: u64,
    /// L'eau et la lave qui coulent.
    pub(crate) fluids: engine_voxel::FluidSim,
    /// Pas de simulation depuis la dernière étape des fluides (une étape tous les 6 pas :
    /// 10 par seconde).
    pub(crate) fluid_steps: u32,
    /// Le bilan de la dernière étape des fluides.
    pub(crate) fluid_report: engine_voxel::FluidReport,
    /// Le monde de blocs du **serveur**, partagé avec l'ECS (ressource `SharedStreamer`) et les mods.
    /// Il charge, éclaire et sauvegarde, mais ne maille rien.
    pub(crate) world: Rc<RefCell<ChunkStreamer>>,
    /// Les machines et les scripts des mods.
    pub(crate) mods: modding::ModSystem,
    /// Les dossiers des mods, dans l'ordre de chargement.
    pub(crate) mod_dirs: Vec<(String, PathBuf)>,
    /// Les lecteurs d'événements qui les transmettent aux mods.
    pub(crate) mod_readers: EventReaders,
    /// `VOXEL_STRESS=N` : le test de charge.
    pub(crate) stress: Option<stress::StressTest>,
    /// Le point d'apparition (et de réapparition) : la surface d'une colonne de terre ferme.
    pub(crate) spawn: IVec3,
    /// Les joueurs : l'entité, l'inventaire, la survie... de chacun. Le joueur local est celui de
    /// la fenêtre (voir `players.rs`).
    pub(crate) players: players::Players,
    /// Le jeu s'abonne lui-même à ses événements (pour les sons) : un lecteur par type.
    pub(crate) readers: EventReaders,
    pub(crate) day: DayCycle,
    pub(crate) weather: weather::Weather,
    pub(crate) helm: Option<engine_voxel::GridId>,
    /// 0 paisible, 1 normale, 2 difficile.
    pub(crate) difficulty: u8,
    /// Les définitions des potions.
    pub(crate) potions: potions::Potions,
    /// Ce que propose la table d'enchantement : change après chaque enchantement.
    pub(crate) enchant_seed: u32,
    pub(crate) villages: villagers::Villages,
    pub(crate) trades: trading::TradeBook,
    /// La sauvegarde (absente en mode démo, vol automatique ou captures, sauf si
    /// `VOXEL_WORLD` est donné : ces modes modifient le monde).
    pub(crate) store: Option<Arc<engine_voxel::SaveStore>>,
    /// Les dimensions du monde et celle où l'on est (voir `dimensions.rs`).
    pub(crate) dims: dimensions::Dimensions,
    /// Le contenu des sacs à dos du monde (voir `backpacks.rs`).
    pub(crate) backpacks: backpacks::BackpackStore,
    pub(crate) last_autosave: Instant,
    pub(crate) save_started: Instant,
    /// Les règles de la survie (`player.ron`) et de la faim ; l'état de chaque joueur est dans
    /// `players`.
    pub(crate) survival: survival::SurvivalRules,
    pub(crate) hunger_rules: survival::HungerRules,
    /// La visée de l'action en cours de traitement, si `handle` en traite une.
    pub(crate) acting: Option<intents::ActingAim>,
    /// Les réponses du simulateur, en attente d'être appliquées par le client.
    pub(crate) replies: Vec<(players::PlayerId, protocol::ServerMsg)>,
    /// Ce que le dernier message a produit, pour `ctl`.
    pub(crate) outcome: intents::Outcome,
    /// La réponse du dernier message `Command`.
    pub(crate) command_reply: Option<intents::CommandReply>,
    /// `VOXEL_CHECK_MOVES=1` : le simulateur contrôle chaque mouvement du joueur local.
    pub(crate) check_moves: bool,
    /// Le numéro du pas fixe (pour les messages `Move`).
    pub(crate) sim_step: u64,
}
