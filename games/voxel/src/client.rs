//! L'état de la fenêtre, à part de celui du serveur (lot 4 du multijoueur, voir
//! `docs/plan-multijoueur.md`) : le serveur ne doit jamais le lire. Tout ce que la fenêtre montre
//! lui arrive par messages (`ServerMsg`), et tout ce qu'elle fait part en intentions (`ClientMsg`).

pub mod culling;
pub mod death;
pub mod entities;
pub mod input;
pub mod intents;
pub mod interact;
pub mod join;
pub mod player_view;
pub mod predicted;
pub mod screen_actions;
pub mod sky_view;
pub mod stream_upload;
pub mod world;

use std::collections::HashMap;
use std::time::Instant;

use engine_render::{Camera, MeshId, RenderStats, Renderer};
use engine_voxel::RayHit;
use glam::IVec3;

use crate::{
    Demo, Request, audio, control::bench, control::screenshots, render::chest_lids,
    render::conduit_fx, render::far_terrain, render::hand, render::water_fx, ui, ui::menus,
};
use voxel_sim::server::stats::FrameStats;
use voxel_sim::{net::protocol, settings, world::soundscape};

/// L'état de la fenêtre : ce que le serveur ne doit pas lire (lot 4 du multijoueur,
/// `docs/plan-multijoueur.md`). Le rendu, la caméra, l'interface, le son, les effets, la réplique
/// du monde et des entités, la main et la prédiction, les captures et le bench.
pub(crate) struct ClientSide {
    /// Le dernier regard envoyé au serveur (`ClientMsg::Look`).
    pub(crate) last_look: (f32, f32),
    pub(crate) renderer: Renderer,
    pub(crate) camera: Camera,
    pub(crate) mouse_captured: bool,
    /// Le bras à la première personne.
    pub(crate) hand: hand::Hand,
    /// Ce que le jeu demande à l'application (nouveau monde, menu, quitter).
    pub(crate) request: Option<Request>,
    /// Les options attendent une touche pour cette action.
    pub(crate) waiting_key: Option<settings::Action>,
    /// Les mondes sauvegardés, listés à l'ouverture de l'écran de choix.
    pub(crate) worlds: Vec<menus::SavedWorld>,
    /// Le monde que voit la fenêtre : la réplique des chunks envoyés par le serveur (voir
    /// `client/world.rs`). Tout ce qui se dessine ou se vise la lit.
    pub(crate) view: crate::client::world::ClientWorld,
    /// Le maillage GPU de chaque chunk affiché.
    pub(crate) meshes: HashMap<IVec3, MeshId>,
    /// Les pixels des textures de blocs (pour les icônes de l'inventaire).
    pub(crate) block_textures: Vec<Vec<u8>>,
    pub(crate) ui: ui::GameUi,
    pub(crate) debug: ui::DebugInfo,
    /// Le bloc visé, mis à jour à chaque image.
    pub(crate) target: Option<RayHit>,
    /// `Some(vitesse)` en mode vol automatique.
    pub(crate) autofly: Option<f32>,
    pub(crate) demo: Option<Demo>,
    /// `Some` en mode `VOXEL_SCREENSHOTS` : la visite guidée en cours.
    pub(crate) tour: Option<screenshots::Tour>,
    pub(crate) water_fx: water_fx::WaterFx,
    /// Les feuilles qui tombent (voir `render/leaves_fx.rs`).
    pub(crate) leaves_fx: crate::render::leaves_fx::LeavesFx,
    pub(crate) conduit_fx: conduit_fx::ConduitFx,
    /// Le terrain lointain simplifié (voir `render/far_terrain.rs`), s'il est activé.
    pub(crate) far: Option<far_terrain::FarTerrain>,
    /// Le budget d'envoi au GPU et ses compteurs (voir `client/stream_upload.rs`).
    pub(crate) uploads: stream_upload::Uploads,
    /// Les maillages des structures mobiles, et la structure dont on tient la barre.
    pub(crate) grid_meshes: crate::vehicles::ships::GridMeshes,
    pub(crate) soundscape: soundscape::Soundscape,
    /// L'état des écrans d'enchantement avancé et des charmes (voir `enchanting/ui.rs`).
    pub(crate) enchant_ui: crate::enchanting::ui::EnchantUi,
    /// Un morceau est en cours jusqu'à cette date.
    pub(crate) music_until: Option<Instant>,
    /// Le brouillard par beau temps (début, fin) : la pluie le resserre.
    pub(crate) base_fog: (f32, f32),
    /// `VOXEL_QUIT_AFTER=secondes` : quitter proprement (avec sauvegarde) après ce délai.
    pub(crate) quit_at: Option<Instant>,
    /// `None` si le son est coupé ou sans carte son.
    pub(crate) audio: Option<audio::Audio>,
    pub(crate) chest_lids: chest_lids::ChestLids,
    /// Distance marchée depuis le dernier bruit de pas.
    pub(crate) step_distance: f32,
    pub(crate) was_in_liquid: bool,
    pub(crate) frame_stats: FrameStats,
    pub(crate) last_render: RenderStats,
    /// Le culling des grottes : quels chunks sont visibles depuis la caméra.
    pub(crate) culling: culling::CaveCulling,
    /// `VOXEL_BENCH` : le benchmark reproductible (voir `control/bench.rs`).
    pub(crate) bench: Option<bench::Bench>,
    /// Les intentions du joueur de la fenêtre, en attente d'être appliquées (voir
    /// `server/intents.rs`).
    pub(crate) outbox: Vec<protocol::ClientMsg>,
    /// La durée de la dernière image (le simulateur crédite la première image d'un minage).
    pub(crate) frame_dt: f32,
    /// Le dernier état du bouton de repas envoyé au simulateur.
    pub(crate) eating_sent: bool,
    /// Les définitions des blocs et le contenu (objets, recettes, guide), les mêmes qu'au serveur :
    /// chargés ensemble, rechargés ensemble. Un client du réseau les chargera de ses propres mods.
    pub(crate) registry: std::sync::Arc<engine_voxel::BlockRegistry>,
    pub(crate) content: voxel_sim::content::Content,
    /// Le bloc en cours de minage (bouton gauche maintenu) et l'avancement prédit, de 0 à 1 ; le
    /// serveur contrôle le sien (`PlayerState::dig`).
    pub(crate) mining: Option<(glam::IVec3, f32)>,
    /// Le bouton gauche est tenu sans rien à casser : le bras frappe quand même, en boucle.
    pub(crate) air_swing: bool,
    /// La barre envoyée en dernier (`ClientMsg::Steer`).
    pub(crate) steer_sent: f32,
    /// L'horloge et la météo que voit la fenêtre, copiées du serveur (voir `net/sky_view.rs`).
    pub(crate) day: voxel_sim::world::daytime::DayCycle,
    pub(crate) weather: voxel_sim::world::weather::Weather,
    /// Le lien avec l'hôte, pour un invité (voir `client/join.rs`) ; `None` : cette fenêtre fait
    /// tourner le monde.
    pub(crate) remote: Option<crate::client::join::RemoteLink>,
    /// L'écoute des parties annoncées, ouverte sur l'écran « Multijoueur ».
    pub(crate) lan: Option<engine_net::Discovery>,
    /// Le serveur intégré qui tourne dans son thread (`VOXEL_SERVER_THREAD=1`) : le lâcher l'arrête
    /// et attend sa sauvegarde.
    pub(crate) integrated: Option<voxel_sim::net::dedicated::Integrated>,
    /// Les joueurs de la partie (`ServerMsg::PlayerList`), montrés tant que Tab est tenu.
    pub(crate) online: Vec<String>,
    /// Le corps du joueur, simulé ici (voir `net/predicted.rs`).
    pub(crate) body: voxel_sim::net::predicted::PredictedBody,
}
