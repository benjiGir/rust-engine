//! Les fonctions que le jeu fournit au mod, enveloppées dans une API sûre.
//!
//! Sur WebAssembly, ce sont des imports du module `voxel`. Ailleurs (compilation native
//! du workspace, tests), elles n'existent pas : on les remplace par des versions qui
//! ne font rien, pour que le kit compile partout.

use crate::{Block, EventKind, Face, Pos, Sides};

#[cfg(target_arch = "wasm32")]
mod ffi {
    // Les signatures doivent correspondre exactement à celles de l'hôte
    // (`engine_mods::runtime::define_host_functions`).
    #[link(wasm_import_module = "voxel")]
    unsafe extern "C" {
        pub fn log(ptr: i32, len: i32);
        pub fn subscribe(kind: i32);
        pub fn block_id(ptr: i32, len: i32) -> i32;
        pub fn get_block(g: i32, x: i32, y: i32, z: i32) -> i32;
        pub fn set_block(g: i32, x: i32, y: i32, z: i32, id: i32) -> i32;
        pub fn get_power(g: i32, x: i32, y: i32, z: i32) -> i32;
        pub fn received_power(g: i32, x: i32, y: i32, z: i32) -> i32;
        pub fn set_power(g: i32, x: i32, y: i32, z: i32, power: i32) -> i32;
        pub fn get_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, cap: i32) -> i32;
        pub fn set_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
        pub fn schedule_tick(g: i32, x: i32, y: i32, z: i32, delay: i32);
        pub fn give_item(id: i32, count: i32);
        pub fn spawn_entity(ptr: i32, len: i32, x: f32, y: f32, z: f32);
        pub fn random() -> i32;

        pub fn add_energy(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            capacity: i64,
            rate: i64,
            sides: i32,
        ) -> i32;
        pub fn add_tank(g: i32, x: i32, y: i32, z: i32, capacity: i32, sides: i32) -> i32;
        pub fn add_inventory(g: i32, x: i32, y: i32, z: i32, slots: i32, sides: i32) -> i32;
        pub fn add_kinetic(g: i32, x: i32, y: i32, z: i32, sides: i32) -> i32;
        pub fn energy_stored(g: i32, x: i32, y: i32, z: i32) -> i64;
        pub fn energy_insert(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            amount: i64,
            simulate: i32,
        ) -> i64;
        pub fn energy_extract(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            amount: i64,
            simulate: i32,
        ) -> i64;
        pub fn fluid_amount(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
        pub fn fluid_fill(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            ptr: i32,
            len: i32,
            amount: i32,
            simulate: i32,
        ) -> i32;
        pub fn fluid_drain(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            ptr: i32,
            len: i32,
            amount: i32,
            simulate: i32,
        ) -> i32;
        pub fn item_count(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
        pub fn item_insert(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            ptr: i32,
            len: i32,
            count: i32,
            simulate: i32,
        ) -> i32;
        pub fn item_extract(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            face: i32,
            ptr: i32,
            len: i32,
            count: i32,
            simulate: i32,
        ) -> i32;
        pub fn kinetic_speed(g: i32, x: i32, y: i32, z: i32) -> f32;
        pub fn set_kinetic(
            g: i32,
            x: i32,
            y: i32,
            z: i32,
            speed: f32,
            capacity: f32,
            impact: f32,
        ) -> i32;
    }
}

/// Hors WebAssembly : des fonctions inertes, mêmes signatures.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
mod ffi {
    pub unsafe fn log(_: i32, _: i32) {}
    pub unsafe fn subscribe(_: i32) {}
    pub unsafe fn block_id(_: i32, _: i32) -> i32 {
        -1
    }
    pub unsafe fn get_block(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn set_block(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn get_power(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn received_power(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn set_power(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn get_data(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    pub unsafe fn set_data(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn schedule_tick(_: i32, _: i32, _: i32, _: i32, _: i32) {}
    pub unsafe fn give_item(_: i32, _: i32) {}
    pub unsafe fn spawn_entity(_: i32, _: i32, _: f32, _: f32, _: f32) {}
    pub unsafe fn random() -> i32 {
        4
    }
    pub unsafe fn add_energy(_: i32, _: i32, _: i32, _: i32, _: i64, _: i64, _: i32) -> i32 {
        0
    }
    pub unsafe fn add_tank(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn add_inventory(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn add_kinetic(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn energy_stored(_: i32, _: i32, _: i32, _: i32) -> i64 {
        -1
    }
    pub unsafe fn energy_insert(_: i32, _: i32, _: i32, _: i32, _: i32, _: i64, _: i32) -> i64 {
        0
    }
    pub unsafe fn energy_extract(_: i32, _: i32, _: i32, _: i32, _: i32, _: i64, _: i32) -> i64 {
        0
    }
    pub unsafe fn fluid_amount(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn fluid_fill(
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> i32 {
        0
    }
    pub unsafe fn fluid_drain(
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> i32 {
        0
    }
    pub unsafe fn item_count(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    pub unsafe fn item_insert(
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> i32 {
        0
    }
    pub unsafe fn item_extract(
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> i32 {
        0
    }
    pub unsafe fn kinetic_speed(_: i32, _: i32, _: i32, _: i32) -> f32 {
        0.0
    }
    pub unsafe fn set_kinetic(_: i32, _: i32, _: i32, _: i32, _: f32, _: f32, _: f32) -> i32 {
        0
    }
}

/// Une adresse dans la mémoire du mod, telle que le jeu la comprend (32 bits).
fn addr(bytes: &[u8]) -> (i32, i32) {
    (bytes.as_ptr() as usize as i32, bytes.len() as i32)
}

/// Écrit dans le journal du jeu, préfixé du nom du mod.
pub fn log(message: &str) {
    let (ptr, len) = addr(message.as_bytes());
    // SAFETY: l'hôte ne fait que lire `len` octets à `ptr`, qui pointent dans `message`,
    // vivant pendant tout l'appel.
    unsafe { ffi::log(ptr, len) }
}

/// S'abonne à un type d'événement : [`crate::Mod::event`] le recevra.
pub fn subscribe(kind: EventKind) {
    // SAFETY: des entiers seulement.
    unsafe { ffi::subscribe(kind as i32) }
}

/// Le numéro d'un bloc à partir de son identifiant (`"core:stone"`), s'il existe.
pub fn block_id(key: &str) -> Option<Block> {
    let (ptr, len) = addr(key.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `key`.
    let id = unsafe { ffi::block_id(ptr, len) };
    (id >= 0).then_some(Block(id as u16))
}

pub fn get_block(pos: Pos) -> Block {
    // SAFETY: des entiers seulement.
    Block(unsafe { ffi::get_block(pos.grid.0 as i32, pos.x, pos.y, pos.z) } as u16)
}

/// Pose un bloc (les voisins sont prévenus). `false` si la position n'est pas chargée.
pub fn set_block(pos: Pos, block: Block) -> bool {
    // SAFETY: des entiers seulement.
    unsafe { ffi::set_block(pos.grid.0 as i32, pos.x, pos.y, pos.z, block.0 as i32) != 0 }
}

/// La puissance qu'émet ce bloc (0 à 15).
pub fn power(pos: Pos) -> u8 {
    // SAFETY: des entiers seulement.
    unsafe { ffi::get_power(pos.grid.0 as i32, pos.x, pos.y, pos.z) as u8 }
}

/// La plus forte puissance émise par un des six voisins.
pub fn received_power(pos: Pos) -> u8 {
    // SAFETY: des entiers seulement.
    unsafe { ffi::received_power(pos.grid.0 as i32, pos.x, pos.y, pos.z) as u8 }
}

/// Fixe la puissance émise par ce bloc ; les voisins sont prévenus si elle change.
pub fn set_power(pos: Pos, power: u8) -> bool {
    // SAFETY: des entiers seulement.
    unsafe { ffi::set_power(pos.grid.0 as i32, pos.x, pos.y, pos.z, power as i32) != 0 }
}

/// Les données que le mod a rangées dans ce bloc (sauvegardées avec le monde).
pub fn data(pos: Pos) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 64];
    loop {
        let (ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte écrit au plus `cap` octets à `ptr`, qui pointent dans `buffer`.
        let len = unsafe { ffi::get_data(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, cap) };
        if len < 0 {
            return None;
        }
        if len as usize <= buffer.len() {
            buffer.truncate(len as usize);
            return Some(buffer);
        }
        // Trop petit : on recommence avec la bonne taille.
        buffer = vec![0u8; len as usize];
    }
}

/// Range des données dans ce bloc (elles suivent la sauvegarde du monde).
pub fn set_data(pos: Pos, bytes: &[u8]) -> bool {
    let (ptr, len) = addr(bytes);
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `bytes`.
    unsafe { ffi::set_data(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, len) != 0 }
}

/// Demande à être réveillé dans `delay` ticks (20 par seconde) : `Mod::machines` recevra
/// [`crate::MachineCall::ScheduledTick`]. Une position déjà programmée n'est pas dupliquée.
pub fn schedule_tick(pos: Pos, delay: u32) {
    // SAFETY: des entiers seulement.
    unsafe {
        ffi::schedule_tick(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            delay.min(i32::MAX as u32) as i32,
        )
    }
}

/// Donne des objets au joueur.
pub fn give_item(block: Block, count: u32) {
    // SAFETY: des entiers seulement.
    unsafe { ffi::give_item(block.0 as i32, count.min(i32::MAX as u32) as i32) }
}

/// Fait apparaître une créature (`"core:zombie"`).
pub fn spawn_entity(kind: &str, x: f32, y: f32, z: f32) {
    let (ptr, len) = addr(kind.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `kind`.
    unsafe { ffi::spawn_entity(ptr, len, x, y, z) }
}

/// Un nombre pseudo-aléatoire (le mod n'a pas d'autre source de hasard).
pub fn random() -> u32 {
    // SAFETY: aucun argument.
    unsafe { ffi::random() as u32 }
}

// ----- Capacités : énergie, fluides, objets, rotation -----
//
// Une face `None` veut dire « de l'intérieur » (la machine elle-même) : toujours permis.
// `simulate` : seulement demander combien serait accepté, sans rien changer.

fn face(face: Option<Face>) -> i32 {
    face.map_or(-1, |f| f as i32)
}

/// Donne au bloc un stock d'énergie (à faire quand il est posé).
pub fn add_energy(pos: Pos, capacity: u64, max_rate: u64, sides: Sides) -> bool {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::add_energy(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            capacity as i64,
            max_rate as i64,
            sides.0 as i32,
        ) != 0
    }
}

/// Ajoute un réservoir de fluide au bloc (capacité en millibuckets).
pub fn add_tank(pos: Pos, capacity_mb: u32, sides: Sides) -> bool {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::add_tank(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            capacity_mb as i32,
            sides.0 as i32,
        ) != 0
    }
}

/// Donne au bloc un inventaire de `slots` cases.
pub fn add_inventory(pos: Pos, slots: u32, sides: Sides) -> bool {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::add_inventory(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            slots as i32,
            sides.0 as i32,
        ) != 0
    }
}

/// Rend le bloc « cinétique » (il tourne, comme un engrenage de Create).
pub fn add_kinetic(pos: Pos, sides: Sides) -> bool {
    // SAFETY: des nombres seulement.
    unsafe { ffi::add_kinetic(pos.grid.0 as i32, pos.x, pos.y, pos.z, sides.0 as i32) != 0 }
}

/// L'énergie stockée, `None` si le bloc n'a pas de stock.
pub fn energy_stored(pos: Pos) -> Option<u64> {
    // SAFETY: des nombres seulement.
    let stored = unsafe { ffi::energy_stored(pos.grid.0 as i32, pos.x, pos.y, pos.z) };
    (stored >= 0).then_some(stored as u64)
}

pub fn energy_insert(pos: Pos, from: Option<Face>, amount: u64, simulate: bool) -> u64 {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::energy_insert(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            amount as i64,
            simulate as i32,
        ) as u64
    }
}

pub fn energy_extract(pos: Pos, from: Option<Face>, amount: u64, simulate: bool) -> u64 {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::energy_extract(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            amount as i64,
            simulate as i32,
        ) as u64
    }
}

/// La quantité d'un fluide (`"core:water"`) dans les réservoirs du bloc, en millibuckets.
pub fn fluid_amount(pos: Pos, fluid: &str) -> u32 {
    let (ptr, len) = addr(fluid.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `fluid`.
    unsafe { ffi::fluid_amount(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, len) as u32 }
}

pub fn fluid_fill(
    pos: Pos,
    from: Option<Face>,
    fluid: &str,
    amount_mb: u32,
    simulate: bool,
) -> u32 {
    let (ptr, len) = addr(fluid.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `fluid`.
    unsafe {
        ffi::fluid_fill(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            ptr,
            len,
            amount_mb as i32,
            simulate as i32,
        ) as u32
    }
}

/// Vide un fluide précis, ou n'importe lequel avec `None`.
pub fn fluid_drain(
    pos: Pos,
    from: Option<Face>,
    fluid: Option<&str>,
    amount_mb: u32,
    simulate: bool,
) -> u32 {
    let (ptr, len) = addr(fluid.unwrap_or("").as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr` (aucun si `len` vaut 0).
    unsafe {
        ffi::fluid_drain(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            ptr,
            len,
            amount_mb as i32,
            simulate as i32,
        ) as u32
    }
}

pub fn item_count(pos: Pos, item: &str) -> u32 {
    let (ptr, len) = addr(item.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `item`.
    unsafe { ffi::item_count(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, len) as u32 }
}

pub fn item_insert(pos: Pos, from: Option<Face>, item: &str, count: u32, simulate: bool) -> u32 {
    let (ptr, len) = addr(item.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `item`.
    unsafe {
        ffi::item_insert(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            ptr,
            len,
            count as i32,
            simulate as i32,
        ) as u32
    }
}

/// Retire des objets précis, ou les premiers trouvés avec `None`.
pub fn item_extract(
    pos: Pos,
    from: Option<Face>,
    item: Option<&str>,
    count: u32,
    simulate: bool,
) -> u32 {
    let (ptr, len) = addr(item.unwrap_or("").as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr` (aucun si `len` vaut 0).
    unsafe {
        ffi::item_extract(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            face(from),
            ptr,
            len,
            count as i32,
            simulate as i32,
        ) as u32
    }
}

/// La vitesse de rotation (tours par minute, signée pour le sens).
pub fn kinetic_speed(pos: Pos) -> f32 {
    // SAFETY: des nombres seulement.
    unsafe { ffi::kinetic_speed(pos.grid.0 as i32, pos.x, pos.y, pos.z) }
}

/// Fixe la rotation d'un bloc cinétique : sa vitesse, ce qu'il peut entraîner
/// (une source) et ce qu'il consomme (une machine).
pub fn set_kinetic(pos: Pos, speed: f32, stress_capacity: f32, stress_impact: f32) -> bool {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::set_kinetic(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            speed,
            stress_capacity,
            stress_impact,
        ) != 0
    }
}
