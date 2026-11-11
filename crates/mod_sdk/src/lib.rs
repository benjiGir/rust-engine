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
//! Le jeu et le mod ne s'échangent que des entiers : les textes et les listes passent
//! par la mémoire du mod. Ce kit cache ces détails (décrits dans `docs/modding.md`).

mod ffi;
mod host;

use std::cell::RefCell;

pub use host::{
    Item, Recipe, add_energy, add_inventory, add_kinetic, add_tank, block_base, block_id, data,
    energy_extract, energy_insert, energy_stored, find_recipe, fluid_amount, fluid_drain,
    fluid_fill, get_block, give_item, give_stack, item_count, item_extract, item_id, item_insert,
    kinetic_speed, log, power, random, received_power, schedule_tick, set_block, set_data,
    set_kinetic, set_power, spawn_entity, subscribe,
};

/// Une grille de blocs : le monde (`Grid::WORLD`), ou une structure mobile (plus tard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Grid(pub u32);

impl Grid {
    pub const WORLD: Grid = Grid(0);
}

/// Une position de bloc : une grille, et des coordonnées dans cette grille. Sur une
/// structure qui bouge, les coordonnées restent les mêmes : la machine ne voit rien.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Pos {
    pub grid: Grid,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    /// Une position dans le monde.
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self::in_grid(Grid::WORLD, x, y, z)
    }

    pub const fn in_grid(grid: Grid, x: i32, y: i32, z: i32) -> Self {
        Self { grid, x, y, z }
    }

    /// Décale la position (dans la même grille) ; `d.grid` est ignorée.
    pub const fn offset(self, d: Pos) -> Self {
        Self::in_grid(self.grid, self.x + d.x, self.y + d.y, self.z + d.z)
    }

    /// Les six voisins directs.
    pub fn neighbors(self) -> [Pos; 6] {
        DIRECTIONS.map(|d| self.offset(d))
    }
}

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
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
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
    pub const ALL: Sides = Sides(0b11_1111);
    pub const NONE: Sides = Sides(0);

    pub const fn only(face: Face) -> Sides {
        Sides(1 << face as u8)
    }

    pub const fn with(self, face: Face) -> Sides {
        Sides(self.0 | 1 << face as u8)
    }
}

/// Un type de bloc (son numéro, attribué au chargement : à retrouver avec [`block_id`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Block(pub u16);

impl Block {
    pub const AIR: Block = Block(0);
}

/// Les événements auxquels un mod peut s'abonner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EventKind {
    BlockBroken = ffi::EVENT_BLOCK_BROKEN,
    BlockPlaced = ffi::EVENT_BLOCK_PLACED,
    EntityDied = ffi::EVENT_ENTITY_DIED,
    Tick = ffi::EVENT_TICK,
    PlayerDamaged = ffi::EVENT_PLAYER_DAMAGED,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    BlockBroken {
        pos: Pos,
        block: Block,
    },
    /// `facing` : la direction (horizontale) vers laquelle regardait le joueur.
    BlockPlaced {
        pos: Pos,
        block: Block,
        facing: Pos,
    },
    EntityDied {
        kind: String,
        pos: Pos,
    },
    Tick(u64),
    PlayerDamaged {
        amount: f32,
        health_left: f32,
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
}

// ----- La mécanique d'export (utilisée par `export_mod!`) -----

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
/// jeu appelle (`mod_init`, `on_event`, `on_machines`, `sdk_alloc`).
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
