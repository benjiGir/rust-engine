//! Les créatures à comportement : une créature déclarée `brain: true` dans `entities.ron` n'a
//! pas l'IA du jeu, c'est **le script de son mod** qui la dirige. Le jeu lui montre ses créatures
//! par lot, quatre fois par seconde ([`crate::Mod::mobs`]), et il répond avec [`mob_move`],
//! [`mob_jump`], [`mob_attack`] et [`mob_state`]. Un seul appel pour toutes les créatures
//! proches : c'est ce qui rend ce modèle bon marché.
//!
//! ```ignore
//! fn mobs(&mut self, mobs: &[MobView]) {
//!     for m in mobs {
//!         let dist = (m.to_player[0].powi(2) + m.to_player[2].powi(2)).sqrt();
//!         if dist < 16.0 {
//!             mod_sdk::mob_move(m.id, m.to_player[0], m.to_player[2], 3.0); // vers le joueur
//!         }
//!     }
//! }
//! ```

/// Les mêmes constantes que le jeu (un test les compare).
pub const RECORD_BYTES: usize = 12 * 4;
pub const FLAG_ON_GROUND: u32 = 1;
pub const FLAG_IN_LIQUID: u32 = 1 << 1;
pub const FLAG_HURT: u32 = 1 << 2;

/// Le numéro d'une créature, valable seulement pendant l'appel qui l'a donné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MobId(pub u32);

/// Ce que le jeu montre d'une créature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobView {
    pub id: MobId,
    /// Son type : à comparer avec [`crate::mob_kind`] (`mob_kind("example:guard")`).
    pub kind: u32,
    /// La position de ses pieds.
    pub position: [f32; 3],
    pub health: f32,
    pub max_health: f32,
    /// Du pied de la créature vers le joueur (x, y, z).
    pub to_player: [f32; 3],
    pub on_ground: bool,
    pub in_liquid: bool,
    /// Elle vient d'être frappée.
    pub hurt: bool,
    /// Ce que le script a rangé avec [`crate::mob_state`] (0 au départ).
    pub state: u32,
}

impl MobView {
    /// La distance horizontale au joueur.
    pub fn distance_to_player(&self) -> f32 {
        (self.to_player[0].powi(2) + self.to_player[2].powi(2)).sqrt()
    }
}
