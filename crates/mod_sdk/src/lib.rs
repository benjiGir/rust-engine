//! Le kit pour écrire un mod du jeu voxel en Rust.
//!
//! Un mod est une bibliothèque compilée en WebAssembly (`wasm32-unknown-unknown`).
//! Il implémente le trait [`Mod`] et l'exporte avec [`export_mod!`] :
//!
//! ```ignore
//! use mod_sdk::{Event, Mod, Pos};
//!
//! #[derive(Default)]
//! struct MonMod;
//!
//! impl Mod for MonMod {
//!     fn init(&mut self) {
//!         mod_sdk::subscribe(mod_sdk::EventKind::BlockBroken);
//!     }
//!     fn event(&mut self, event: Event) {
//!         if let Event::BlockBroken { pos, .. } = event {
//!             mod_sdk::log(&format!("bloc cassé en {pos:?}"));
//!         }
//!     }
//! }
//!
//! mod_sdk::export_mod!(MonMod);
//! ```
//!
//! Le guide complet (ce qu'un mod peut faire, avec des exemples) est dans `docs/modding.md` ; cette
//! page en est la référence : chaque fonction, type et méthode de [`Mod`], documentés.
//! (`cargo xtask docs` la génère.)
//!
//! Le jeu et le mod ne s'échangent que des entiers : les textes et les listes passent
//! par la mémoire du mod. Ce kit cache ces détails (décrits dans `docs/modding.md`).

// La référence du SDK est ce que lit l'auteur d'un mod : tout ce qui est public doit être documenté.
#![warn(missing_docs)]

mod ffi;
mod host;
pub mod mobs;
pub mod ui;

use std::cell::RefCell;

pub use host::{
    Item, PlayerInfo, Recipe, add_energy, add_inventory, add_kinetic, add_tank, block_base,
    block_id, call_mod, close_menu, consume_held, data, dimension, energy_extract, energy_insert,
    energy_stored, find_recipe, fluid_amount, fluid_drain, fluid_fill, get_block, give_effect,
    give_item, give_stack, heal_player, hud_bar, hud_clear, hurt_area, item_count, item_extract,
    item_id, item_insert, kinetic_speed, log, mob_attack, mob_jump, mob_kind, mob_move, mob_state,
    mod_loaded, player, player_target, power, random, received_power, register_command, say,
    schedule_tick, set_block, set_data, set_kinetic, set_power, set_setting, set_slot,
    set_world_value, setting, setting_bool, setting_f32, setting_i32, show_menu, show_options,
    slot, spawn_entity, subscribe, teleport_player, travel_to, world_value,
};

/// Une grille de blocs : le monde (`Grid::WORLD`), ou une structure mobile (plus tard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Grid(pub u32);

impl Grid {
    /// Le monde.
    pub const WORLD: Grid = Grid(0);
}

/// Une position de bloc : une grille, et des coordonnées dans cette grille. Sur une
/// structure qui bouge, les coordonnées restent les mêmes : la machine ne voit rien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Pos {
    /// La grille.
    pub grid: Grid,
    /// Abscisse.
    pub x: i32,
    /// Hauteur.
    pub y: i32,
    /// Profondeur.
    pub z: i32,
}

impl Pos {
    /// Une position dans le monde.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self::in_grid(Grid::WORLD, x, y, z)
    }

    /// Une position dans une grille donnée.
    pub const fn in_grid(grid: Grid, x: i32, y: i32, z: i32) -> Self {
        Self { grid, x, y, z }
    }

    /// Décale la position (dans la même grille) ; `d.grid` est ignorée.
    pub const fn offset(self, d: Pos) -> Self {
        Self::in_grid(self.grid, self.x + d.x, self.y + d.y, self.z + d.z)
    }

    /// Pour une position de chunk : le coin minimal du chunk en blocs (32 blocs de côté).
    pub const fn chunk_origin(self) -> Pos {
        Pos::in_grid(
            self.grid,
            self.x * CHUNK_SIZE,
            self.y * CHUNK_SIZE,
            self.z * CHUNK_SIZE,
        )
    }

    /// Les six voisins directs.
    pub fn neighbors(self) -> [Pos; 6] {
        DIRECTIONS.map(|d| self.offset(d))
    }
}

/// Le côté d'un chunk, en blocs.
pub const CHUNK_SIZE: i32 = 32;

/// Les six directions : +X, -X, +Y, -Y, +Z, -Z.
pub const DIRECTIONS: [Pos; 6] = [
    Pos::new(1, 0, 0),
    Pos::new(-1, 0, 0),
    Pos::new(0, 1, 0),
    Pos::new(0, -1, 0),
    Pos::new(0, 0, 1),
    Pos::new(0, 0, -1),
];

/// Une face d'un bloc, dans l'ordre de [`DIRECTIONS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    /// Face +X.
    PosX = 0,
    /// Face -X.
    NegX = 1,
    /// Face +Y (dessus).
    PosY = 2,
    /// Face -Y (dessous).
    NegY = 3,
    /// Face +Z.
    PosZ = 4,
    /// Face -Z.
    NegZ = 5,
}

impl Face {
    /// Les six faces.
    pub const ALL: [Face; 6] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    /// La direction de cette face (`Face::PosY` → `(0, 1, 0)`).
    pub fn direction(self) -> Pos {
        DIRECTIONS[self as usize]
    }
}

/// Un ensemble de faces (bit `i` = face `i`), pour dire par où un conteneur échange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sides(pub u8);

impl Sides {
    /// Les six faces.
    pub const ALL: Sides = Sides(0b11_1111);
    /// Aucune face.
    pub const NONE: Sides = Sides(0);

    /// Une seule face.
    pub const fn only(face: Face) -> Sides {
        Sides(1 << face as u8)
    }

    /// Les mêmes faces, plus celle-ci.
    pub const fn with(self, face: Face) -> Sides {
        Sides(self.0 | 1 << face as u8)
    }
}

/// Un type de bloc (son numéro, attribué au chargement : à retrouver avec [`block_id`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Block(pub u16);

impl Block {
    /// L'air.
    pub const AIR: Block = Block(0);
}

/// Les événements auxquels un mod peut s'abonner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EventKind {
    /// Un bloc a été cassé ([`Event::BlockBroken`]).
    BlockBroken = ffi::EVENT_BLOCK_BROKEN,
    /// Un bloc a été posé ([`Event::BlockPlaced`]).
    BlockPlaced = ffi::EVENT_BLOCK_PLACED,
    /// Une créature est morte ([`Event::EntityDied`]).
    EntityDied = ffi::EVENT_ENTITY_DIED,
    /// Un tick de machines, 20 par seconde ([`Event::Tick`]).
    Tick = ffi::EVENT_TICK,
    /// Le joueur a perdu de la vie ([`Event::PlayerDamaged`]).
    PlayerDamaged = ffi::EVENT_PLAYER_DAMAGED,
    /// Un chunk neuf vient d'être généré ([`Event::ChunkGenerated`]).
    ChunkGenerated = ffi::EVENT_CHUNK_GENERATED,
    /// Un clic droit avec un objet `usable` ([`Event::ItemUsed`]).
    ItemUsed = ffi::EVENT_ITEM_USED,
}

#[derive(Debug, Clone, PartialEq)]
/// Un événement du jeu, reçu dans [`Mod::event`] après [`subscribe`].
pub enum Event {
    /// Un bloc a été cassé.
    BlockBroken {
        /// Où il était.
        pos: Pos,
        /// Ce qu'il était.
        block: Block,
    },
    /// `facing` : la direction (horizontale) vers laquelle regardait le joueur.
    BlockPlaced {
        /// Où il a été posé.
        pos: Pos,
        /// Ce qu'il est.
        block: Block,
        /// La direction horizontale du regard du joueur qui l'a posé.
        facing: Pos,
    },
    /// Une créature est morte.
    EntityDied {
        /// Son type (`"core:pig"`).
        kind: String,
        /// Où elle est morte.
        pos: Pos,
    },
    /// Un tick de machines : son numéro.
    Tick(u64),
    /// Le joueur a perdu de la vie.
    PlayerDamaged {
        /// Les dégâts subis (après l'armure).
        amount: f32,
        /// La vie qui lui reste.
        health_left: f32,
    },
    /// Le joueur a fait un clic droit avec cet objet en main (un objet `usable` d'`items.ron`),
    /// hors d'une action du jeu (poser un bloc, ouvrir un coffre...). [`player`] donne où il
    /// regarde.
    ItemUsed {
        /// L'objet utilisé (son numéro : à comparer avec [`item_id`]).
        item: Item,
    },
    /// Un chunk non vide vient d'être généré pour la première fois (pas relu d'une sauvegarde).
    /// `chunk` : ses coordonnées de chunk ; son coin minimal est [`Pos::chunk_origin`]. Pour le
    /// décorer, écrire dans ce chunk seulement (`set_block` hors des chunks chargés échoue).
    ChunkGenerated {
        /// Les coordonnées du chunk (32 blocs de côté).
        chunk: Pos,
    },
}

/// Pourquoi le jeu réveille des machines du mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineCall {
    /// Elles viennent d'être posées.
    Placed,
    /// Leur chunk vient d'être chargé.
    Loaded,
    /// Un de leurs voisins a changé (bloc ou puissance).
    NeighborChanged,
    /// Le tick qu'elles avaient programmé ([`schedule_tick`]) est arrivé.
    ScheduledTick,
}

/// Un mod. Toutes les méthodes ont une version par défaut qui ne fait rien.
pub trait Mod {
    /// Appelée une fois au chargement : s'abonner aux événements, chercher ses blocs.
    fn init(&mut self) {}
    /// Un événement auquel le mod est abonné.
    fn event(&mut self, _event: Event) {}
    /// Les machines du mod à réveiller, **toutes en un seul appel** : passer d'un monde à
    /// l'autre coûte cher, donc le jeu regroupe.
    fn machines(&mut self, _call: MachineCall, _positions: &[Pos]) {}
    /// Le joueur fait un clic droit sur une machine du mod. Renvoyer `true` prend le clic en
    /// compte (le jeu ne tente plus de poser un bloc) ; `false` le laisse passer.
    fn used(&mut self, _pos: Pos) -> bool {
        false
    }
    /// Le joueur a agi dans le menu de la machine en `pos` (voir [`ui::Menu`]).
    fn ui(&mut self, _pos: Pos, _event: ui::UiEvent) {}
    /// La page d'options du mod (menu pause) : son contenu actuel, ou `None` s'il n'en a pas.
    /// Appelée à l'ouverture de la page, puis après chaque action du joueur. Le mod doit aussi
    /// avoir `options: true` dans son `mod.ron`.
    fn options(&mut self) -> Option<ui::Menu> {
        None
    }
    /// Une action du joueur dans la page d'options : en général, un `set_setting`.
    fn options_event(&mut self, _event: ui::UiEvent) {}
    /// Un autre mod appelle la fonction `name` de ce mod (voir [`call_mod`]) : la réponse, ou
    /// `None` pour « je ne sais pas faire ça ».
    fn call(&mut self, _name: &str, _args: &[u8]) -> Option<Vec<u8>> {
        None
    }
    /// Le joueur a tapé la commande `/name args` déclarée avec [`register_command`]. La réponse
    /// s'affiche dans la console (une ligne par `\n`) ; `None` : rien à dire.
    fn command(&mut self, _name: &str, _args: &str) -> Option<String> {
        None
    }
    /// Les créatures à comportement du mod (`brain: true` dans `entities.ron`) qui sont près du
    /// joueur, quatre fois par seconde, **toutes en un seul appel** (voir [`mobs`]). Répondre
    /// avec [`mob_move`], [`mob_jump`], [`mob_attack`] et [`mob_state`].
    fn mobs(&mut self, _mobs: &[mobs::MobView]) {}
}

// ----- La mécanique d'export (utilisée par `export_mod!`) -----

thread_local! {
    /// La réponse à l'appel d'un autre mod, que le jeu vient lire.
    static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// Le tampon où le jeu dépose les données d'un appel (événement, positions).
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[doc(hidden)]
pub mod __private {
    use super::*;

    /// Le jeu demande de la place pour `len` octets : on la prend dans notre tampon.
    pub fn alloc(len: i32) -> i32 {
        BUFFER.with_borrow_mut(|buffer| {
            buffer.clear();
            buffer.resize(len.max(0) as usize, 0);
            buffer.as_mut_ptr() as usize as i32
        })
    }

    fn ints(bytes: &[u8]) -> Vec<i32> {
        // `as_chunks::<4>()` : des tableaux `[u8; 4]` (la taille est connue à la compilation).
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&c| i32::from_le_bytes(c))
            .collect()
    }

    /// Décode l'événement que le jeu vient d'écrire dans le tampon.
    pub fn event(kind: i32, len: i32) -> Option<Event> {
        BUFFER.with_borrow(|buffer| {
            decode_event(kind, &buffer[..(len.max(0) as usize).min(buffer.len())])
        })
    }

    pub fn decode_event(kind: i32, bytes: &[u8]) -> Option<Event> {
        let v = ints(bytes);
        let pos = |i: usize| Some(Pos::new(*v.get(i)?, *v.get(i + 1)?, *v.get(i + 2)?));
        // Une position de grille : 4 entiers (grille, x, y, z).
        let grid_pos = |i: usize| {
            Some(Pos::in_grid(
                Grid(*v.get(i)? as u32),
                *v.get(i + 1)?,
                *v.get(i + 2)?,
                *v.get(i + 3)?,
            ))
        };
        Some(match kind {
            ffi::EVENT_BLOCK_BROKEN => Event::BlockBroken {
                pos: grid_pos(0)?,
                block: Block(*v.get(4)? as u16),
            },
            ffi::EVENT_BLOCK_PLACED => Event::BlockPlaced {
                pos: grid_pos(0)?,
                block: Block(*v.get(4)? as u16),
                facing: pos(5)?,
            },
            ffi::EVENT_ENTITY_DIED => Event::EntityDied {
                pos: pos(0)?,
                kind: String::from_utf8_lossy(bytes.get(12..)?).into_owned(),
            },
            ffi::EVENT_TICK => {
                Event::Tick(*v.first()? as u32 as u64 | (*v.get(1)? as u32 as u64) << 32)
            }
            ffi::EVENT_ITEM_USED => Event::ItemUsed {
                item: Item(*v.first()? as u16),
            },
            ffi::EVENT_CHUNK_GENERATED => Event::ChunkGenerated {
                chunk: grid_pos(0)?,
            },
            ffi::EVENT_PLAYER_DAMAGED => Event::PlayerDamaged {
                amount: f32::from_bits(*v.first()? as u32),
                health_left: f32::from_bits(*v.get(1)? as u32),
            },
            _ => return None,
        })
    }

    /// Les positions que le jeu vient d'écrire dans le tampon.
    pub fn positions(count: i32) -> Vec<Pos> {
        BUFFER.with_borrow(|buffer| {
            let v = ints(buffer);
            v.as_chunks::<4>()
                .0
                .iter()
                .take(count.max(0) as usize)
                .map(|&[grid, x, y, z]| Pos::in_grid(Grid(grid as u32), x, y, z))
                .collect()
        })
    }

    /// Décode l'appel que le jeu vient d'écrire dans le tampon : (nom, arguments).
    pub fn call_request(len: i32) -> Option<(String, Vec<u8>)> {
        BUFFER.with_borrow(|buffer| {
            let bytes = &buffer[..(len.max(0) as usize).min(buffer.len())];
            let name_len = u16::from_le_bytes(bytes.get(..2)?.try_into().ok()?) as usize;
            let name = String::from_utf8(bytes.get(2..2 + name_len)?.to_vec()).ok()?;
            Some((name, bytes.get(2 + name_len..)?.to_vec()))
        })
    }

    /// Les créatures que le jeu vient d'écrire dans le tampon.
    pub fn mobs(count: i32) -> Vec<mobs::MobView> {
        BUFFER.with_borrow(|buffer| decode_mobs(buffer, count))
    }

    pub fn decode_mobs(bytes: &[u8], count: i32) -> Vec<mobs::MobView> {
        let word = |r: &[u8], i: usize| -> [u8; 4] { r[i * 4..i * 4 + 4].try_into().unwrap() };
        let float = |r: &[u8], i: usize| f32::from_le_bytes(word(r, i));
        bytes
            .as_chunks::<{ mobs::RECORD_BYTES }>()
            .0
            .iter()
            .take(count.max(0) as usize)
            .map(|r| {
                let r = &r[..];
                let flags = u32::from_le_bytes(word(r, 10));
                mobs::MobView {
                    id: mobs::MobId(u32::from_le_bytes(word(r, 0))),
                    kind: u32::from_le_bytes(word(r, 1)),
                    position: [float(r, 2), float(r, 3), float(r, 4)],
                    health: float(r, 5),
                    max_health: float(r, 6),
                    to_player: [float(r, 7), float(r, 8), float(r, 9)],
                    on_ground: flags & mobs::FLAG_ON_GROUND != 0,
                    in_liquid: flags & mobs::FLAG_IN_LIQUID != 0,
                    hurt: flags & mobs::FLAG_HURT != 0,
                    state: u32::from_le_bytes(word(r, 11)),
                }
            })
            .collect()
    }

    /// Garde la réponse pour que le jeu la lise ; renvoie sa taille.
    pub fn set_reply(reply: Vec<u8>) -> i32 {
        REPLY.with_borrow_mut(|r| {
            *r = reply;
            r.len() as i32
        })
    }

    pub fn reply_ptr() -> i32 {
        REPLY.with_borrow(|r| r.as_ptr() as usize as i32)
    }

    pub fn ui_event(kind: i32, id: i32, value: i32) -> Option<crate::ui::UiEvent> {
        crate::ui::UiEvent::decode(kind, id, value)
    }

    pub fn machine_call(kind: i32) -> Option<MachineCall> {
        Some(match kind {
            ffi::MACHINES_PLACED => MachineCall::Placed,
            ffi::MACHINES_LOADED => MachineCall::Loaded,
            ffi::MACHINES_NEIGHBOR => MachineCall::NeighborChanged,
            ffi::MACHINES_TICK => MachineCall::ScheduledTick,
            _ => return None,
        })
    }
}

/// Exporte un type qui implémente [`Mod`] (et `Default`) : génère les fonctions que le
/// jeu appelle (`mod_init`, `on_event`, `on_machines`, `on_use`, `sdk_alloc`).
#[macro_export]
macro_rules! export_mod {
    ($t:ty) => {
        thread_local! {
            static __MOD: ::core::cell::RefCell<$t> = ::core::cell::RefCell::new(<$t as ::core::default::Default>::default());
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn sdk_alloc(len: i32) -> i32 {
            $crate::__private::alloc(len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn mod_init() {
            __MOD.with_borrow_mut(|m| $crate::Mod::init(m));
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_event(kind: i32, _ptr: i32, len: i32) {
            if let Some(event) = $crate::__private::event(kind, len) {
                __MOD.with_borrow_mut(|m| $crate::Mod::event(m, event));
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_use(grid: i32, x: i32, y: i32, z: i32) -> i32 {
            let pos = $crate::Pos::in_grid($crate::Grid(grid as u32), x, y, z);
            __MOD.with_borrow_mut(|m| $crate::Mod::used(m, pos)) as i32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_ui(kind: i32, grid: i32, x: i32, y: i32, z: i32, id: i32, value: i32) {
            if let Some(event) = $crate::__private::ui_event(kind, id, value) {
                let pos = $crate::Pos::in_grid($crate::Grid(grid as u32), x, y, z);
                __MOD.with_borrow_mut(|m| $crate::Mod::ui(m, pos, event));
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_options(kind: i32, id: i32, value: i32) {
            __MOD.with_borrow_mut(|m| {
                if let Some(event) = $crate::__private::ui_event(kind, id, value) {
                    $crate::Mod::options_event(m, event);
                }
                if kind != $crate::ui::UI_CLOSED
                    && let Some(menu) = $crate::Mod::options(m)
                {
                    $crate::show_options(&menu);
                }
            });
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_call(_ptr: i32, len: i32) -> i32 {
            match $crate::__private::call_request(len) {
                Some((name, args)) => {
                    match __MOD.with_borrow_mut(|m| $crate::Mod::call(m, &name, &args)) {
                        Some(reply) => $crate::__private::set_reply(reply),
                        None => -1,
                    }
                }
                None => -1,
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_mobs(_ptr: i32, count: i32) {
            let mobs = $crate::__private::mobs(count);
            __MOD.with_borrow_mut(|m| $crate::Mod::mobs(m, &mobs));
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_command(_ptr: i32, len: i32) -> i32 {
            match $crate::__private::call_request(len) {
                Some((name, args)) => {
                    let args = String::from_utf8_lossy(&args).into_owned();
                    match __MOD.with_borrow_mut(|m| $crate::Mod::command(m, &name, &args)) {
                        Some(text) => $crate::__private::set_reply(text.into_bytes()),
                        None => -1,
                    }
                }
                None => -1,
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn call_reply() -> i32 {
            $crate::__private::reply_ptr()
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn on_machines(kind: i32, _ptr: i32, count: i32) {
            if let Some(call) = $crate::__private::machine_call(kind) {
                let positions = $crate::__private::positions(count);
                __MOD.with_borrow_mut(|m| $crate::Mod::machines(m, call, &positions));
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::__private::decode_event;
    use super::*;

    fn bytes(ints: &[i32]) -> Vec<u8> {
        ints.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    #[test]
    fn events_are_decoded() {
        assert_eq!(
            decode_event(1, &bytes(&[0, 1, -2, 3, 7])),
            Some(Event::BlockBroken {
                pos: Pos::new(1, -2, 3),
                block: Block(7)
            })
        );
        assert_eq!(
            decode_event(4, &bytes(&[5, 1])),
            Some(Event::Tick(5 | 1 << 32))
        );
        let mut died = bytes(&[0, 64, 0]);
        died.extend(b"core:zombie");
        assert!(
            matches!(decode_event(3, &died), Some(Event::EntityDied { kind, .. }) if kind == "core:zombie")
        );
        // Données tronquées : pas d'événement plutôt qu'un plantage.
        assert_eq!(decode_event(1, &bytes(&[1, 2])), None);
    }
}
