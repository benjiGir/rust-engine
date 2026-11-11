//! Les fonctions que le jeu fournit au mod, enveloppées dans une API sûre.
//!
//! Les imports bruts sont générés depuis le catalogue `hooks.ron` (`ffi.rs`) : sur
//! WebAssembly, des imports du module `voxel` ; ailleurs (compilation native du workspace,
//! tests), des versions qui ne font rien, pour que le kit compile partout.

use crate::{Block, EventKind, Face, Pos, Sides};

use crate::ffi;

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

/// Le bloc de base d'un bloc à états (voir `docs/modding.md`) : un escalier orienté vers l'est
/// donne l'escalier de base. Un bloc ordinaire se renvoie lui-même.
pub fn block_base(block: Block) -> Block {
    // SAFETY: des entiers seulement.
    Block(unsafe { ffi::block_base(block.0 as i32) } as u16)
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

/// Un objet (bloc ou non) : son numéro, attribué au chargement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Item(pub u16);

/// Le numéro d'un objet à partir de son identifiant (`"example:ruby"`), s'il existe.
pub fn item_id(key: &str) -> Option<Item> {
    let (ptr, len) = addr(key.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `key`.
    let id = unsafe { ffi::item_id(ptr, len) };
    (id >= 0).then_some(Item(id as u16))
}

/// Donne `count` objets au joueur (contrairement à [`give_item`], l'objet n'a pas besoin
/// d'être un bloc).
pub fn give_stack(item: Item, count: u32) {
    // SAFETY: des entiers seulement.
    unsafe { ffi::give_stack(item.0 as i32, count.min(i32::MAX as u32) as i32) }
}

/// Une recette de machine trouvée par [`find_recipe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Durée, en ticks (20 par seconde).
    pub ticks: u32,
    /// Énergie totale consommée.
    pub energy: u64,
    /// Ce que la recette consomme : (identifiant d'objet, nombre).
    pub inputs: Vec<(String, u32)>,
    /// Ce qu'elle produit.
    pub outputs: Vec<(String, u32)>,
}

fn parse_stacks(text: &str) -> Vec<(String, u32)> {
    text.split(',')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, n) = part.split_once('*')?;
            Some((key.to_string(), n.parse().ok()?))
        })
        .collect()
}

/// Cherche une recette du type `kind` (`"example:press"`, déclarée dans le
/// `machine_recipes.ron` d'un mod) faisable avec ces objets (identifiant, nombre).
pub fn find_recipe(kind: &str, available: &[(&str, u32)]) -> Option<Recipe> {
    let input = available
        .iter()
        .map(|(key, n)| format!("{key}*{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let (kind_ptr, kind_len) = addr(kind.as_bytes());
    let (in_ptr, in_len) = addr(input.as_bytes());
    let mut buffer = vec![0u8; 256];
    loop {
        let (out_ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte lit dans `kind` et `input`, et écrit au plus `cap` octets dans `buffer`.
        let len = unsafe { ffi::recipe_find(kind_ptr, kind_len, in_ptr, in_len, out_ptr, cap) };
        if len < 0 {
            return None;
        }
        if len as usize > buffer.len() {
            buffer = vec![0u8; len as usize];
            continue;
        }
        let text = String::from_utf8_lossy(&buffer[..len as usize]).into_owned();
        let mut parts = text.splitn(4, ';');
        return Some(Recipe {
            ticks: parts.next()?.parse().ok()?,
            energy: parts.next()?.parse().ok()?,
            inputs: parse_stacks(parts.next()?),
            outputs: parse_stacks(parts.next()?),
        });
    }
}

/// Ouvre le menu de la machine en `pos` pour le joueur, ou met à jour celui qui est ouvert.
/// Un menu trop gros (voir [`crate::ui::MAX_MENU_BYTES`]) est ignoré.
pub fn show_menu(pos: Pos, menu: &crate::ui::Menu) {
    let (ptr, len) = addr(menu.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `menu`.
    unsafe { ffi::ui_show(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, len) }
}

/// Ferme le menu du mod, s'il est ouvert.
pub fn close_menu() {
    // SAFETY: aucun argument.
    unsafe { ffi::ui_close() }
}

/// Affiche (ou met à jour) la page d'options du mod dans le menu pause.
pub fn show_options(menu: &crate::ui::Menu) {
    let (ptr, len) = addr(menu.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `menu`.
    unsafe { ffi::options_show(ptr, len) }
}

/// Un réglage du mod (texte), tel que le joueur l'a réglé ; `None` s'il n'a jamais été fixé.
pub fn setting(key: &str) -> Option<String> {
    let (key_ptr, key_len) = addr(key.as_bytes());
    let mut buffer = vec![0u8; 64];
    loop {
        let (out_ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte lit dans `key` et écrit au plus `cap` octets dans `buffer`.
        let len = unsafe { ffi::setting_get(key_ptr, key_len, out_ptr, cap) };
        if len < 0 {
            return None;
        }
        if len as usize <= buffer.len() {
            buffer.truncate(len as usize);
            return Some(String::from_utf8_lossy(&buffer).into_owned());
        }
        buffer = vec![0u8; len as usize];
    }
}

/// Enregistre un réglage du mod (sauvegardé pour tous les mondes). `false` si la clé ou la
/// valeur est trop longue, ou s'il y a trop de réglages.
pub fn set_setting(key: &str, value: &str) -> bool {
    let (key_ptr, key_len) = addr(key.as_bytes());
    let (val_ptr, val_len) = addr(value.as_bytes());
    // SAFETY: l'hôte lit dans `key` et `value`, vivants pendant l'appel.
    unsafe { ffi::setting_set(key_ptr, key_len, val_ptr, val_len) != 0 }
}

/// Un réglage lu comme nombre décimal, ou `default`.
pub fn setting_f32(key: &str, default: f32) -> f32 {
    setting(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Un réglage lu comme entier, ou `default`.
pub fn setting_i32(key: &str, default: i32) -> i32 {
    setting(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Un réglage lu comme booléen (`true`/`false`), ou `default`.
pub fn setting_bool(key: &str, default: bool) -> bool {
    setting(key).and_then(|v| v.parse().ok()).unwrap_or(default)
}
