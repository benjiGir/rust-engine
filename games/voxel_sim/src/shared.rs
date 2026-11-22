//! Le monde de blocs, partagé entre le jeu, les systèmes de l'ECS et les scripts des mods.
//!
//! Le streamer vit dans un `Rc<RefCell<...>>` : un compteur de références (`Rc`) pour
//! qu'il ait plusieurs propriétaires (le jeu, la ressource ECS, chaque mod WebAssembly),
//! et un `RefCell` pour vérifier les emprunts à l'exécution. Les scripts de mods en ont
//! besoin : `wasmtime` exige que l'état des fonctions hôtes soit possédé (`'static`),
//! il ne peut pas contenir une simple référence vers un champ du jeu.

use std::cell::RefCell;
use std::rc::Rc;

use engine_voxel::{ChunkStreamer, SubGrids};

/// La ressource ECS qui donne accès au monde.
pub struct SharedStreamer(pub Rc<RefCell<ChunkStreamer>>);

/// La ressource ECS qui donne accès aux structures mobiles (voir `vehicles/ships.rs`).
pub struct SharedSubGrids(pub Rc<RefCell<SubGrids>>);

/// `world.streamer().borrow()` dans les systèmes. Renvoie une copie du `Rc` (une
/// incrémentation de compteur) : l'emprunt de la ressource ECS est rendu aussitôt.
pub trait StreamerExt {
    fn streamer(&self) -> Rc<RefCell<ChunkStreamer>>;
}

impl StreamerExt for engine_ecs::World {
    fn streamer(&self) -> Rc<RefCell<ChunkStreamer>> {
        self.resource::<SharedStreamer>().0.clone()
    }
}
