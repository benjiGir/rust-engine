//! Les écrans qu'un joueur peut avoir ouverts : la simulation les ouvre et les ferme, la fenêtre
//! les dessine (voir `ui.rs`).

use glam::IVec3;

/// L'écran ouvert. Il passe dans `ServerMsg::OpenScreen` : un wagon ou un habitant y est désigné
/// par son numéro réseau (`NetId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Screen {
    /// L'inventaire (E) : armure, grille 2 × 2, livre de recettes.
    Inventory,
    /// L'établi : grille 3 × 3.
    Workbench,
    Furnace(IVec3),
    /// L'alambic.
    Brewing(IVec3),
    Chest(IVec3),
    /// Un sac à dos, par son numéro (voir `storage/backpacks.rs`).
    Bag(u32),
    /// Le contrôleur de stockage (voir `storage/ui.rs`).
    Controller(IVec3),
    /// L'inventaire d'un wagon (voir `vehicles/cart_play.rs`).
    Cart(crate::net::entities::NetId),
    /// L'échange avec un habitant.
    Trade(crate::net::entities::NetId),
    /// La table d'enchantement posée à cet endroit (ses étagères sont lues autour d'elle).
    Enchant(IVec3),
    /// L'enclume simple (voir `enchanting/books.rs`).
    Anvil,
    /// La bibliothèque d'enchantements (voir `enchanting/library.rs`).
    Library(IVec3),
    /// La table de récupération (voir `adventure/forge_ui.rs`).
    Salvage,
    /// La table de réforge (voir `adventure/forge_ui.rs`).
    Reforge,
    /// L'établi de taille (voir `shaping/ui.rs`).
    Shaping,
    /// Les paliers du monde (voir `adventure/tiers_ui.rs`).
    Tiers,
    /// Le Guide du voyageur (voir `quests/guide_ui.rs`).
    Guide,
    /// Le livre de quêtes, plein écran (voir `quests/ui.rs`).
    Quests,
    /// Le menu d'une machine de mod (voir `modding/menu.rs`).
    Mod(IVec3),
    /// La console de commandes (voir `ui/console.rs`).
    Console,
}

/// L'établi de taille.
pub const SHAPING_TABLE: &str = "core:shaping_table";
