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

/// L'identifiant texte d'un bloc (`"core:stone"`), qui est aussi le nom de l'objet qu'il donne.
pub fn block_key(block: Block) -> Option<String> {
    let mut buffer = vec![0u8; 64];
    loop {
        let (ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte écrit au plus `cap` octets à `ptr`, qui pointent dans `buffer`.
        let len = unsafe { ffi::block_key(block.0 as i32, ptr, cap) };
        if len < 0 {
            return None;
        }
        if len as usize > buffer.len() {
            buffer = vec![0u8; len as usize];
            continue;
        }
        return Some(String::from_utf8_lossy(&buffer[..len as usize]).into_owned());
    }
}

/// Ce que lâcherait ce bloc cassé : des (objet, nombre), tirés au hasard selon les tables de
/// butin du jeu et des mods (une culture mûre donne du blé et des graines, un minerai son
/// produit...).
pub fn block_drops(block: Block) -> Vec<(String, u32)> {
    let mut buffer = vec![0u8; 128];
    loop {
        let (ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte écrit au plus `cap` octets à `ptr`, qui pointent dans `buffer`.
        let len = unsafe { ffi::block_drops(block.0 as i32, ptr, cap) };
        if len < 0 {
            return Vec::new();
        }
        if len as usize > buffer.len() {
            // Le tirage a eu lieu : on le relit plus grand, il sera différent ; sans importance.
            buffer = vec![0u8; len as usize];
            continue;
        }
        let text = String::from_utf8_lossy(&buffer[..len as usize]).into_owned();
        return text
            .split(';')
            .filter_map(|part| {
                let (item, count) = part.rsplit_once('*')?;
                Some((item.to_string(), count.parse().ok()?))
            })
            .collect();
    }
}

/// Le bloc de base d'un bloc à états (voir `docs/modding.md`) : un escalier orienté vers l'est
/// donne l'escalier de base. Un bloc ordinaire se renvoie lui-même.
pub fn block_base(block: Block) -> Block {
    // SAFETY: des entiers seulement.
    Block(unsafe { ffi::block_base(block.0 as i32) } as u16)
}

/// Le bloc à cette position (l'air si la position est hors du monde ou d'une grille inconnue).
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

/// Insère de l'énergie dans le stock du bloc par la face `from` (`None` : de l'intérieur) ; renvoie la quantité acceptée. `simulate` : seulement demander combien serait accepté.
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

/// Retire de l'énergie du stock du bloc par la face `from` ; renvoie la quantité retirée. `simulate` : seulement la demander.
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

/// Remplit les réservoirs du bloc avec ce fluide (`"core:water"`) par la face `from` ; renvoie la quantité acceptée, en millibuckets. `simulate` : seulement la demander.
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

/// Le nombre de cet objet (`"core:coal"`) dans l'inventaire du bloc.
pub fn item_count(pos: Pos, item: &str) -> u32 {
    let (ptr, len) = addr(item.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `item`.
    unsafe { ffi::item_count(pos.grid.0 as i32, pos.x, pos.y, pos.z, ptr, len) as u32 }
}

/// Range des objets dans l'inventaire du bloc par la face `from` ; renvoie le nombre accepté. `simulate` : seulement le demander.
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

/// Combien d'exemplaires de cet objet tiennent dans une case (1 pour un outil) ; 0 si l'objet
/// n'existe pas.
pub fn item_max_stack(key: &str) -> u32 {
    let (ptr, len) = addr(key.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `key`.
    unsafe { ffi::item_max_stack(ptr, len) as u32 }
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

/// Ce que contient une case de l'inventaire du bloc : (identifiant d'objet, nombre), ou `None`
/// si elle est vide ou n'existe pas.
pub fn slot(pos: Pos, slot: usize) -> Option<(String, u32)> {
    let mut buffer = vec![0u8; 64];
    loop {
        let (ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte écrit au plus `cap` octets à `ptr`, qui pointent dans `buffer`.
        let len = unsafe {
            ffi::slot_get(
                pos.grid.0 as i32,
                pos.x,
                pos.y,
                pos.z,
                slot as i32,
                ptr,
                cap,
            )
        };
        if len < 0 {
            return None;
        }
        if len as usize > buffer.len() {
            buffer = vec![0u8; len as usize];
            continue;
        }
        let text = String::from_utf8_lossy(&buffer[..len as usize]).into_owned();
        let (item, count) = text.rsplit_once('*')?;
        return Some((item.to_string(), count.parse().ok()?));
    }
}

/// L'usure de l'objet dans une case (0 : neuf ; il casse quand elle atteint [`item_durability`]),
/// ou `None` si la case est vide ou n'existe pas.
pub fn slot_damage(pos: Pos, slot: usize) -> Option<u32> {
    // SAFETY: des nombres seulement.
    let damage = unsafe { ffi::slot_damage(pos.grid.0 as i32, pos.x, pos.y, pos.z, slot as i32) };
    u32::try_from(damage).ok()
}

/// Règle l'usure de l'objet dans une case. `false` si la case est vide ou n'existe pas.
/// [`set_slot`] remet l'usure à zéro : la régler après avoir posé l'objet.
pub fn set_slot_damage(pos: Pos, slot: usize, damage: u32) -> bool {
    // SAFETY: des nombres seulement.
    unsafe {
        ffi::slot_set_damage(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            slot as i32,
            damage.min(i32::MAX as u32) as i32,
        ) != 0
    }
}

/// Le nombre d'utilisations d'un objet avant qu'il casse (outils, armures) ; 0 s'il n'use pas.
pub fn item_durability(key: &str) -> u32 {
    let (ptr, len) = addr(key.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `key`.
    unsafe { ffi::item_durability(ptr, len) as u32 }
}

/// Remplace une case de l'inventaire du bloc (`None` : la vider). `false` si la case n'existe
/// pas ou si l'objet est inconnu.
pub fn set_slot(pos: Pos, slot: usize, stack: Option<(&str, u32)>) -> bool {
    let text = stack.map_or_else(String::new, |(item, count)| format!("{item}*{count}"));
    let (ptr, len) = addr(text.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `text`.
    unsafe {
        ffi::slot_set(
            pos.grid.0 as i32,
            pos.x,
            pos.y,
            pos.z,
            slot as i32,
            ptr,
            len,
        ) != 0
    }
}

/// Une donnée du mod **propre à ce monde**, sauvegardée avec lui (contrairement aux
/// [`setting`], qui suivent le joueur). Du texte : de quoi ranger des nombres, des listes, du JSON.
pub fn world_value(key: &str) -> Option<String> {
    let (key_ptr, key_len) = addr(key.as_bytes());
    let mut buffer = vec![0u8; 256];
    loop {
        let (out_ptr, cap) = (buffer.as_mut_ptr() as usize as i32, buffer.len() as i32);
        // SAFETY: l'hôte lit dans `key` et écrit au plus `cap` octets dans `buffer`.
        let len = unsafe { ffi::world_get(key_ptr, key_len, out_ptr, cap) };
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

/// Enregistre une donnée du mod dans ce monde (64 Kio au plus par valeur, 1 024 clés).
pub fn set_world_value(key: &str, value: &str) -> bool {
    let (key_ptr, key_len) = addr(key.as_bytes());
    let (val_ptr, val_len) = addr(value.as_bytes());
    // SAFETY: l'hôte lit dans `key` et `value`, vivants pendant l'appel.
    unsafe { ffi::world_set(key_ptr, key_len, val_ptr, val_len) != 0 }
}

/// Déclare la commande `/nom` du mod (dans [`crate::Mod::init`]) : `help` est sa ligne d'aide
/// dans `/help`. Le nom : minuscules, chiffres et `_`, 32 caractères au plus.
pub fn register_command(name: &str, help: &str) {
    let (name_ptr, name_len) = addr(name.as_bytes());
    let (help_ptr, help_len) = addr(help.as_bytes());
    // SAFETY: l'hôte lit dans `name` et `help`, vivants pendant l'appel.
    unsafe { ffi::register_command(name_ptr, name_len, help_ptr, help_len) }
}

/// Le numéro d'un type de créature (`"example:guard"`), à comparer avec `MobView::kind`.
pub fn mob_kind(key: &str) -> Option<u32> {
    let (ptr, len) = addr(key.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `key`.
    let id = unsafe { ffi::mob_kind(ptr, len) };
    (id >= 0).then_some(id as u32)
}

/// Ordre pendant [`crate::Mod::mobs`] : la créature marche vers (dx, dz) à `speed` blocs par
/// seconde (0 : elle s'arrête). Valable jusqu'au prochain appel.
pub fn mob_move(mob: crate::mobs::MobId, dx: f32, dz: f32, speed: f32) {
    // SAFETY: des nombres seulement.
    unsafe { ffi::mob_move(mob.0 as i32, dx, dz, speed) }
}

/// Ordre pendant [`crate::Mod::mobs`] : la créature va jusqu'au point `to` (x, y, z) **en cherchant
/// son chemin** : elle contourne les murs, saute les marches et évite la lave, à `speed` blocs par
/// seconde. Elle continue seule entre deux appels (le jeu recalcule le chemin si le but bouge ou
/// si elle se coince) ; si le but est inaccessible, elle va au plus près. [`mob_move`] ou un
/// nouveau `mob_move_to` change son ordre.
pub fn mob_move_to(mob: crate::mobs::MobId, to: [f32; 3], speed: f32) {
    // SAFETY: des nombres seulement.
    unsafe { ffi::mob_move_to(mob.0 as i32, to[0], to[1], to[2], speed) }
}

/// Ordre pendant [`crate::Mod::mobs`] : la créature saute (si elle touche le sol).
pub fn mob_jump(mob: crate::mobs::MobId) {
    // SAFETY: un entier seulement.
    unsafe { ffi::mob_jump(mob.0 as i32) }
}

/// Ordre pendant [`crate::Mod::mobs`] : la créature blesse le joueur s'il est à portée de bras
/// (3 blocs), au plus une fois par seconde.
pub fn mob_attack(mob: crate::mobs::MobId, damage: f32) {
    // SAFETY: des nombres seulement.
    unsafe { ffi::mob_attack(mob.0 as i32, damage) }
}

/// Ordre pendant [`crate::Mod::mobs`] : range un entier dans la mémoire de la créature (il revient
/// dans `MobView::state`). Pour un état d'esprit : 0 repos, 1 poursuite...
pub fn mob_state(mob: crate::mobs::MobId, value: u32) {
    // SAFETY: des entiers seulement.
    unsafe { ffi::mob_state(mob.0 as i32, value as i32) }
}

/// Fait passer le joueur dans cette dimension (`"mon_mod:ma_dimension"`, ou `"core:overworld"` pour
/// revenir) : le monde est sauvegardé puis rechargé, et le joueur arrive au sol aux mêmes x et z.
/// Les scripts repartent à neuf après le voyage : ce qui doit rester va dans
/// [`set_world_value`]. Renvoie `true` si le voyage est lancé.
pub fn travel_to(dimension: &str) -> bool {
    let (ptr, len) = addr(dimension.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `dimension`.
    unsafe { ffi::travel_to(ptr, len) != 0 }
}

/// L'identifiant de la dimension où se trouve le joueur.
pub fn dimension() -> String {
    let mut buffer = vec![0u8; 64];
    // SAFETY: l'hôte écrit au plus `cap` octets à `ptr`, qui pointent dans `buffer`.
    let len = unsafe { ffi::dimension(buffer.as_mut_ptr() as usize as i32, buffer.len() as i32) };
    buffer.truncate((len.max(0) as usize).min(buffer.len()));
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Au plus ce nombre d'octets pour les arguments et la réponse d'un appel entre mods.
pub const MAX_CALL_BYTES: usize = 64 * 1024;

/// Appelle la fonction `name` d'un autre mod (son [`crate::Mod::call`]) avec ces octets et
/// renvoie sa réponse. `None` si le mod est absent ou désactivé, s'il ne répond pas, s'il est
/// hors budget ce tick-ci, ou si l'appel ferait un cycle (A appelle B qui rappelle A).
/// Déclarez le mod appelé dans les `dependencies` de votre `mod.ron` : il est chargé avant vous.
pub fn call_mod(mod_id: &str, name: &str, args: &[u8]) -> Option<Vec<u8>> {
    let (mod_ptr, mod_len) = addr(mod_id.as_bytes());
    let (name_ptr, name_len) = addr(name.as_bytes());
    let (arg_ptr, arg_len) = addr(args);
    // Une seule tentative, avec la plus grande réponse permise : rappeler la fonction pour
    // lire une réponse plus longue la relancerait (et ses effets avec).
    let mut buffer = vec![0u8; MAX_CALL_BYTES];
    // SAFETY: l'hôte lit dans `mod_id`, `name` et `args`, et écrit au plus `cap` octets dans `buffer`.
    let len = unsafe {
        ffi::call_mod(
            mod_ptr,
            mod_len,
            name_ptr,
            name_len,
            arg_ptr,
            arg_len,
            buffer.as_mut_ptr() as usize as i32,
            buffer.len() as i32,
        )
    };
    if len < 0 {
        return None;
    }
    buffer.truncate((len as usize).min(MAX_CALL_BYTES));
    Some(buffer)
}

/// Vrai si le mod (son identifiant) est chargé et actif.
pub fn mod_loaded(mod_id: &str) -> bool {
    let (ptr, len) = addr(mod_id.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `mod_id`.
    unsafe { ffi::mod_loaded(ptr, len) != 0 }
}

/// Envoie le signal `name` du mod au livre de quêtes : les tâches `Signal("<mod>:name")` des
/// quêtes du mod se remplissent (le jeu ajoute toujours l'espace de noms du mod appelant : un
/// mod ne peut remplir que ses propres signaux). Le nom : minuscules, chiffres, `_`, `-`, `.` et
/// `/`, 64 octets au plus. Renvoie `false` si le nom est refusé.
pub fn quest_signal(name: &str) -> bool {
    let (ptr, len) = addr(name.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `name`.
    unsafe { ffi::quest_signal(ptr, len) != 0 }
}

/// Vrai si la quête (son identifiant complet, `"core:wood/workbench"`) est terminée dans ce
/// monde. Faux aussi pour une quête inconnue.
pub fn quest_done(quest: &str) -> bool {
    let (ptr, len) = addr(quest.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `quest`.
    unsafe { ffi::quest_done(ptr, len) != 0 }
}

/// L'état du joueur, vu d'un script.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerInfo {
    /// Ses yeux (x, y, z).
    pub eye: [f32; 3],
    /// Son regard, de longueur 1.
    pub look: [f32; 3],
    /// Sa vie, et sa vie maximale.
    pub health: f32,
    /// Sa vie maximale.
    pub max_health: f32,
    /// Son niveau d'expérience.
    pub level: f32,
}

/// L'état du joueur, rafraîchi à chaque pas de simulation (`None` : pas de joueur).
pub fn player() -> Option<PlayerInfo> {
    let mut floats = [0f32; 9];
    // SAFETY: l'hôte écrit au plus 36 octets à `ptr`, qui pointent dans `floats`.
    let n = unsafe { ffi::player_info(floats.as_mut_ptr() as usize as i32, 36) };
    (n == 9).then(|| PlayerInfo {
        eye: [floats[0], floats[1], floats[2]],
        look: [floats[3], floats[4], floats[5]],
        health: floats[6],
        max_health: floats[7],
        level: floats[8],
    })
}

/// Le point où le regard du joueur touche un bloc, à moins de `max_dist` blocs.
pub fn player_target(max_dist: f32) -> Option<[f32; 3]> {
    let mut point = [0f32; 3];
    // SAFETY: l'hôte écrit 12 octets à `ptr`, qui pointent dans `point`.
    let hit = unsafe { ffi::player_target(max_dist, point.as_mut_ptr() as usize as i32) };
    (hit != 0).then_some(point)
}

/// Rend de la vie au joueur.
pub fn heal_player(amount: f32) {
    // SAFETY: un nombre seulement.
    unsafe { ffi::player_heal(amount) }
}

/// Téléporte le joueur (ses pieds en `x, y, z`), à 64 blocs au plus d'où il est.
pub fn teleport_player(x: f32, y: f32, z: f32) {
    // SAFETY: des nombres seulement.
    unsafe { ffi::player_teleport(x, y, z) }
}

/// Donne un effet au joueur : `"speed"`, `"strength"`, `"regeneration"`, `"fire_resistance"` ou
/// `"healing"`, pendant `seconds` secondes, avec cette puissance.
pub fn give_effect(effect: &str, seconds: f32, power: f32) {
    let (ptr, len) = addr(effect.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `effect`.
    unsafe { ffi::player_effect(ptr, len, seconds, power) }
}

/// Blesse les créatures à moins de `radius` blocs de ce point (16 au plus) ; elles reculent.
pub fn hurt_area(center: [f32; 3], radius: f32, damage: f32) {
    // SAFETY: des nombres seulement.
    unsafe { ffi::hurt_area(center[0], center[1], center[2], radius, damage) }
}

/// Blesse le joueur (l'armure compte, et il est invulnérable un instant après un coup) : un piège.
pub fn hurt_player(amount: f32) {
    // SAFETY: un nombre seulement.
    unsafe { ffi::hurt_player(amount) }
}

/// Écrit une ligne dans la console du joueur (un message).
pub fn say(text: &str) {
    let (ptr, len) = addr(text.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `text`.
    unsafe { ffi::say(ptr, len) }
}

/// Retire `count` exemplaires de l'objet que le joueur tient en main (au plus 64).
pub fn consume_held(count: u32) {
    // SAFETY: un entier seulement.
    unsafe { ffi::consume_held(count.min(64) as i32) }
}

/// Affiche (ou met à jour) une jauge du mod à l'écran : `fraction` de 0 à 1, `color` en (r, g, b)
/// de 0 à 1, et son texte. `id` distingue vos jauges (quatre au plus).
pub fn hud_bar(id: u32, fraction: f32, color: [f32; 3], label: &str) {
    let (ptr, len) = addr(label.as_bytes());
    // SAFETY: l'hôte lit `len` octets à `ptr`, dans `label`.
    unsafe { ffi::hud_bar(id as i32, fraction, color[0], color[1], color[2], ptr, len) }
}

/// Retire une jauge de l'écran.
pub fn hud_clear(id: u32) {
    // SAFETY: un entier seulement.
    unsafe { ffi::hud_clear(id as i32) }
}
