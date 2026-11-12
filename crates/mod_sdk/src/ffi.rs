// GÉNÉRÉ par `cargo xtask hooks` depuis `crates/mod_sdk/hooks.ron` : ne pas modifier.
//! Les fonctions que le jeu fournit au mod (module d'import `voxel`), sans enveloppe : `host.rs` les rend sûres.

#![allow(dead_code)]

/// Les numéros des événements (`on_event`).
/// BlockBroken : un bloc a été cassé.
pub const EVENT_BLOCK_BROKEN: i32 = 1;
/// BlockPlaced : un bloc a été posé.
pub const EVENT_BLOCK_PLACED: i32 = 2;
/// EntityDied : une créature est morte.
pub const EVENT_ENTITY_DIED: i32 = 3;
/// Tick : un tick de machines (20 par seconde).
pub const EVENT_TICK: i32 = 4;
/// PlayerDamaged : le joueur a perdu de la vie.
pub const EVENT_PLAYER_DAMAGED: i32 = 5;
/// ChunkGenerated : un chunk non vide vient d'être généré pour la première fois (pas relu d'une sauvegarde) : le mod peut le décorer.
pub const EVENT_CHUNK_GENERATED: i32 = 6;

/// Les numéros des appels de machines (`on_machines`).
/// Placed : elles viennent d'être posées.
pub const MACHINES_PLACED: i32 = 0;
/// Loaded : leur chunk vient d'être chargé (les ticks programmés ne sont pas sauvegardés).
pub const MACHINES_LOADED: i32 = 1;
/// NeighborChanged : un voisin a changé.
pub const MACHINES_NEIGHBOR: i32 = 2;
/// ScheduledTick : leur tick programmé est arrivé.
pub const MACHINES_TICK: i32 = 3;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "voxel")]
unsafe extern "C" {
    /// Écrit le texte dans le journal du jeu, préfixé du nom du mod.
    pub fn log(ptr: i32, len: i32);
    /// S'abonne à un type d'événement (voir les événements) : `on_event` le recevra.
    pub fn subscribe(kind: i32);
    /// Un nombre pseudo-aléatoire positif (reproductible pour une même graine).
    pub fn random() -> i32;
    /// Donne `count` objets du bloc `id` au joueur.
    pub fn give_item(id: i32, count: i32);
    /// Fait apparaître une créature (identifiant texte, par exemple `core:pig`) à cette position.
    pub fn spawn_entity(ptr: i32, len: i32, x: f32, y: f32, z: f32);
    /// Le numéro d'un bloc à partir de son identifiant (`core:stone`) ; −1 s'il n'existe pas.
    pub fn block_id(ptr: i32, len: i32) -> i32;
    /// Le bloc de base d'un bloc à états : `example:stairs[facing=east]` donne `example:stairs`. Un bloc ordinaire se renvoie lui-même.
    pub fn block_base(id: i32) -> i32;
    /// Le bloc à cette position (0 : de l'air, ou une grille inconnue).
    pub fn get_block(g: i32, x: i32, y: i32, z: i32) -> i32;
    /// Pose un bloc ; les voisins sont prévenus. 1 si c'est fait.
    pub fn set_block(g: i32, x: i32, y: i32, z: i32, id: i32) -> i32;
    /// La puissance émise par ce bloc (0 à 15).
    pub fn get_power(g: i32, x: i32, y: i32, z: i32) -> i32;
    /// La plus forte puissance reçue des 6 voisins.
    pub fn received_power(g: i32, x: i32, y: i32, z: i32) -> i32;
    /// Change la puissance émise (0 à 15) ; les voisins sont prévenus. 1 si c'est fait.
    pub fn set_power(g: i32, x: i32, y: i32, z: i32, power: i32) -> i32;
    /// Copie l'état libre du bloc (au plus `cap` octets) ; renvoie sa taille, −1 sans état.
    pub fn get_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, cap: i32) -> i32;
    /// Remplace l'état libre du bloc, sauvegardé avec le monde. 1 si c'est fait.
    pub fn set_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
    /// Réveille la machine dans `delay` ticks (20 par seconde).
    pub fn schedule_tick(g: i32, x: i32, y: i32, z: i32, delay: i32);
    /// Donne au bloc un stockage d'énergie, ouvert sur les faces `sides` (6 bits). 1 si c'est fait.
    pub fn add_energy(g: i32, x: i32, y: i32, z: i32, capacity: i64, rate: i64, sides: i32) -> i32;
    /// Ajoute un réservoir de `capacity` mB. 1 si c'est fait.
    pub fn add_tank(g: i32, x: i32, y: i32, z: i32, capacity: i32, sides: i32) -> i32;
    /// Donne au bloc un inventaire de `slots` cases. 1 si c'est fait.
    pub fn add_inventory(g: i32, x: i32, y: i32, z: i32, slots: i32, sides: i32) -> i32;
    /// Donne au bloc une rotation (vitesse, capacité et consommation de contrainte). 1 si c'est fait.
    pub fn add_kinetic(g: i32, x: i32, y: i32, z: i32, sides: i32) -> i32;
    /// L'énergie stockée ; −1 sans stockage.
    pub fn energy_stored(g: i32, x: i32, y: i32, z: i32) -> i64;
    /// Insère de l'énergie par cette face ; renvoie la quantité acceptée.
    pub fn energy_insert(
        g: i32,
        x: i32,
        y: i32,
        z: i32,
        face: i32,
        amount: i64,
        simulate: i32,
    ) -> i64;
    /// Retire de l'énergie par cette face ; renvoie la quantité retirée.
    pub fn energy_extract(
        g: i32,
        x: i32,
        y: i32,
        z: i32,
        face: i32,
        amount: i64,
        simulate: i32,
    ) -> i64;
    /// La quantité (mB) de ce fluide dans les réservoirs du bloc.
    pub fn fluid_amount(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
    /// Remplit avec ce fluide par cette face ; renvoie la quantité acceptée (mB).
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
    /// Vide ce fluide par cette face ; renvoie la quantité retirée (mB).
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
    /// Le nombre de cet objet dans l'inventaire du bloc.
    pub fn item_count(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32;
    /// Range des objets par cette face ; renvoie le nombre accepté.
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
    /// Sort des objets par cette face ; renvoie le nombre retiré.
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
    /// Lit une case de l'inventaire du bloc : écrit `objet*nombre` (au plus `cap` octets) et renvoie la taille ; −1 si la case est vide ou n'existe pas.
    pub fn slot_get(g: i32, x: i32, y: i32, z: i32, slot: i32, ptr: i32, cap: i32) -> i32;
    /// Remplace une case de l'inventaire par `objet*nombre` (texte vide : la vider). 1 si c'est fait.
    pub fn slot_set(g: i32, x: i32, y: i32, z: i32, slot: i32, ptr: i32, len: i32) -> i32;
    /// La vitesse de rotation (tr/min, signée).
    pub fn kinetic_speed(g: i32, x: i32, y: i32, z: i32) -> f32;
    /// Règle la vitesse, la capacité et la consommation de contrainte. 1 si c'est fait.
    pub fn set_kinetic(
        g: i32,
        x: i32,
        y: i32,
        z: i32,
        speed: f32,
        capacity: f32,
        impact: f32,
    ) -> i32;
    /// Ouvre (ou met à jour) le menu de la machine à cette position : `len` octets décrivant le menu (voir `Menu`). Le joueur le voit à l'écran.
    pub fn ui_show(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32);
    /// Ferme le menu du mod s'il est ouvert.
    pub fn ui_close();
    /// Affiche (ou met à jour) la page d'options du mod : `len` octets décrivant le menu.
    pub fn options_show(ptr: i32, len: i32);
    /// Copie la valeur (texte) du réglage du mod (au plus `cap` octets) ; renvoie sa taille, −1 s'il n'existe pas.
    pub fn setting_get(key_ptr: i32, key_len: i32, out_ptr: i32, cap: i32) -> i32;
    /// Enregistre un réglage du mod (clé de 64 octets, valeur de 1 024, 256 clés au plus). 1 si c'est fait.
    pub fn setting_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    /// Copie la valeur (texte) d'une donnée du mod **dans ce monde** (au plus `cap` octets) ; renvoie sa taille, −1 si elle n'existe pas.
    pub fn world_get(key_ptr: i32, key_len: i32, out_ptr: i32, cap: i32) -> i32;
    /// Enregistre une donnée du mod dans ce monde (clé de 64 octets, valeur de 64 Kio, 1 024 clés au plus), sauvegardée avec le monde. 1 si c'est fait.
    pub fn world_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    /// Le numéro d'un type de créature (`example:guard`) tel que le montrent les créatures reçues dans `on_mobs` ; −1 s'il n'existe pas.
    pub fn mob_kind(ptr: i32, len: i32) -> i32;
    /// Pendant `on_mobs` : la créature marche dans la direction (dx, dz) à `speed` blocs par seconde (au plus le double de sa vitesse de base) ; elle se tourne vers là. Une vitesse nulle l'arrête. Valable jusqu'au prochain appel.
    pub fn mob_move(mob: i32, dx: f32, dz: f32, speed: f32);
    /// Pendant `on_mobs` : la créature saute (si elle touche le sol).
    pub fn mob_jump(mob: i32);
    /// Pendant `on_mobs` : la créature blesse le joueur s'il est à portée de bras (3 blocs) et si elle n'a pas frappé depuis une seconde.
    pub fn mob_attack(mob: i32, damage: f32);
    /// Pendant `on_mobs` : la mémoire de la créature (un entier de votre choix, rendu dans `state` au prochain appel).
    pub fn mob_state(mob: i32, value: i32);
    /// Fait passer le joueur dans cette dimension (`mon_mod:ma_dimension`) : le monde est sauvegardé puis rechargé, et le joueur arrive au sol aux mêmes x et z. 1 si le voyage est lancé, 0 si la dimension est inconnue ou si le monde n'est pas sauvegardé.
    pub fn travel_to(ptr: i32, len: i32) -> i32;
    /// Copie l'identifiant de la dimension où se trouve le joueur (`core:overworld`, `mon_mod:ma_dimension`) ; renvoie sa taille.
    pub fn dimension(out_ptr: i32, cap: i32) -> i32;
    /// Déclare la commande `/nom` du mod (à appeler dans `init`) avec sa ligne d'aide : le joueur la tape dans la console, et le mod la reçoit dans `on_command`.
    pub fn register_command(name_ptr: i32, name_len: i32, help_ptr: i32, help_len: i32);
    /// Appelle la fonction `name` du mod `mod` avec ces octets (64 Kio au plus) ; copie sa réponse (au plus `cap` octets) et renvoie sa taille. −1 si le mod est absent, désactivé, hors budget, déjà en cours d'appel (pas de cycle) ou s'il ne répond pas.
    pub fn call_mod(
        mod_ptr: i32,
        mod_len: i32,
        name_ptr: i32,
        name_len: i32,
        arg_ptr: i32,
        arg_len: i32,
        out_ptr: i32,
        cap: i32,
    ) -> i32;
    /// 1 si le mod (son identifiant) est chargé et actif.
    pub fn mod_loaded(ptr: i32, len: i32) -> i32;
    /// Le numéro d'un objet (bloc ou non) à partir de son identifiant (`example:ruby`) ; −1 s'il n'existe pas.
    pub fn item_id(ptr: i32, len: i32) -> i32;
    /// Donne `count` objets de numéro `id` (voir `item_id`) au joueur.
    pub fn give_stack(id: i32, count: i32);
    /// Cherche une recette du type donné (`example:press`) faisable avec les objets `cle*nombre,cle*nombre` de l'entrée. Écrit `ticks;énergie;entrées;sorties` (même format) dans la sortie ; renvoie sa taille, −1 sans recette.
    pub fn recipe_find(
        kind_ptr: i32,
        kind_len: i32,
        in_ptr: i32,
        in_len: i32,
        out_ptr: i32,
        out_cap: i32,
    ) -> i32;
}

/// Hors WebAssembly (compilation native du workspace, tests des mods) : des
/// fonctions inertes, mêmes signatures.
#[cfg(not(target_arch = "wasm32"))]
mod inert {
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn log(_: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn subscribe(_: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn random() -> i32 {
        4
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn give_item(_: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn spawn_entity(_: i32, _: i32, _: f32, _: f32, _: f32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn block_id(_: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn block_base(_: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn get_block(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn set_block(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn get_power(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn received_power(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn set_power(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn get_data(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn set_data(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn schedule_tick(_: i32, _: i32, _: i32, _: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn add_energy(_: i32, _: i32, _: i32, _: i32, _: i64, _: i64, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn add_tank(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn add_inventory(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn add_kinetic(_: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn energy_stored(_: i32, _: i32, _: i32, _: i32) -> i64 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn energy_insert(_: i32, _: i32, _: i32, _: i32, _: i32, _: i64, _: i32) -> i64 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn energy_extract(_: i32, _: i32, _: i32, _: i32, _: i32, _: i64, _: i32) -> i64 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn fluid_amount(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
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
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
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
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn item_count(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
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
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
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
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn slot_get(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn slot_set(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn kinetic_speed(_: i32, _: i32, _: i32, _: i32) -> f32 {
        0.0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn set_kinetic(_: i32, _: i32, _: i32, _: i32, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn ui_show(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn ui_close() {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn options_show(_: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn setting_get(_: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn setting_set(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn world_get(_: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn world_set(_: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mob_kind(_: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mob_move(_: i32, _: f32, _: f32, _: f32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mob_jump(_: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mob_attack(_: i32, _: f32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mob_state(_: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn travel_to(_: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn dimension(_: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn register_command(_: i32, _: i32, _: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn call_mod(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn mod_loaded(_: i32, _: i32) -> i32 {
        0
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn item_id(_: i32, _: i32) -> i32 {
        -1
    }
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn give_stack(_: i32, _: i32) {}
    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
    pub unsafe fn recipe_find(_: i32, _: i32, _: i32, _: i32, _: i32, _: i32) -> i32 {
        -1
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use inert::*;
