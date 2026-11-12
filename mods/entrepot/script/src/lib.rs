//! Le script du mod Entrepôt, écrit **uniquement avec l'API publique** (`mod_sdk`).
//!
//! - **Le grand coffre** (`entrepot:vault`) : 54 cases et un bouton « Ranger » qui regroupe les
//!   piles et trie les objets.
//! - **Le trieur** (`entrepot:sorter`) : un modèle (case 0) ; seul cet objet entre par les conduits.
//! - **La moissonneuse** (`entrepot:harvester`) : récolte et replante les cultures mûres autour d'elle.
//! - **La foreuse** (`entrepot:drill`) : alimentée, creuse un puits droit et garde le butin.
//!
//! Les objets circulent entre ces blocs par les conduits d'objets (les courroies de Mécanique) :
//! ce sont les `ports` de `blocks.ron` qui disent où le réseau peut déposer et prendre.

use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, MachineCall, Mod, Pos, Sides};

/// Les cases de chaque machine.
const VAULT_SLOTS: u32 = 54;
const SORTER_SLOTS: u32 = 9;
const MACHINE_SLOTS: u32 = 9;

/// Ticks (20 par seconde) entre deux passes de la moissonneuse, et son rayon (blocs).
const HARVEST_PERIOD: u32 = 40;
const HARVEST_RADIUS: i32 = 3;
/// Ticks entre deux blocs creusés, entre deux vérifications d'une foreuse arrêtée, et la
/// profondeur maximale du puits.
const DRILL_PERIOD: u32 = 20;
const DRILL_IDLE: u32 = 60;
const DRILL_DEPTH: u8 = 48;

#[derive(Default)]
struct Entrepot {
    vault: Block,
    sorter: Block,
    harvester: Block,
    drill: Block,
}

/// Les cultures du jeu sont des blocs numérotés par stade (`core:wheat_0` à `core:wheat_3`) :
/// une culture est mûre quand le stade suivant n'existe pas. Renvoie le bloc à replanter (le
/// stade 0) si `block` est une culture mûre.
fn ripe_crop(block: Block) -> Option<Block> {
    let key = mod_sdk::block_key(block)?;
    let (stem, stage) = key.rsplit_once('_')?;
    let stage: u32 = stage.parse().ok()?;
    if stage == 0 || mod_sdk::block_id(&format!("{stem}_{}", stage + 1)).is_some() {
        return None;
    }
    mod_sdk::block_id(&format!("{stem}_0"))
}

/// Range tout ce butin dans l'inventaire du bloc, ou rien du tout. `true` si tout a tenu.
fn store_all(pos: Pos, loot: &[(String, u32)]) -> bool {
    if !loot
        .iter()
        .all(|(item, n)| mod_sdk::item_insert(pos, None, item, *n, true) == *n)
    {
        return false;
    }
    for (item, n) in loot {
        mod_sdk::item_insert(pos, None, item, *n, false);
    }
    true
}

impl Entrepot {
    /// Un passage de la moissonneuse : chaque culture mûre à portée est cueillie et replantée,
    /// à condition que toute la récolte tienne dans ses cases.
    fn harvest(&self, pos: Pos) {
        for dx in -HARVEST_RADIUS..=HARVEST_RADIUS {
            for dz in -HARVEST_RADIUS..=HARVEST_RADIUS {
                let at = Pos::new(pos.x + dx, pos.y, pos.z + dz);
                let Some(seed) = ripe_crop(mod_sdk::get_block(at)) else {
                    continue;
                };
                let loot = mod_sdk::block_drops(mod_sdk::get_block(at));
                if store_all(pos, &loot) {
                    mod_sdk::set_block(at, seed);
                }
            }
        }
    }

    /// Un coup de foreuse : le prochain bloc du puits est cassé, son butin rangé. `true` si le
    /// travail avance (sinon la foreuse attend : plus de place, du vide, une roche trop dure).
    fn dig(&self, pos: Pos) -> bool {
        let depth = mod_sdk::data(pos)
            .and_then(|d| d.first().copied())
            .unwrap_or(0);
        if depth >= DRILL_DEPTH {
            return false;
        }
        let target = Pos::new(pos.x, pos.y - 1 - depth as i32, pos.z);
        let block = mod_sdk::get_block(target);
        if block == Block::AIR {
            // Un trou déjà creusé : on le traverse sans effort.
            mod_sdk::set_data(pos, &[depth + 1]);
            return true;
        }
        let key = mod_sdk::block_key(block).unwrap_or_default();
        if key == "core:bedrock" || key == "core:water" || key == "core:lava" {
            return false;
        }
        let loot = mod_sdk::block_drops(block);
        if !store_all(pos, &loot) {
            return false;
        }
        mod_sdk::set_block(target, Block::AIR);
        mod_sdk::set_data(pos, &[depth + 1]);
        true
    }
}

/// Regroupe les piles du coffre et les trie par objet. Les objets qui ne se cumulent pas (outils
/// usés...) ne bougent pas : on ne touche qu'aux cases d'objets empilables.
fn tidy(pos: Pos) {
    let mut slots: Vec<usize> = Vec::new();
    let mut stacks: Vec<(String, u32)> = Vec::new();
    for i in 0..VAULT_SLOTS as usize {
        let Some((item, count)) = mod_sdk::slot(pos, i) else {
            continue;
        };
        if mod_sdk::item_max_stack(&item) <= 1 {
            continue;
        }
        slots.push(i);
        match stacks.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += count,
            None => stacks.push((item, count)),
        }
    }
    stacks.sort();
    // Des piles pleines, puis le reste ; jamais plus de piles qu'avant.
    let mut sorted: Vec<(String, u32)> = Vec::new();
    for (item, mut count) in stacks {
        let max = mod_sdk::item_max_stack(&item).max(1);
        while count > 0 {
            let n = count.min(max);
            sorted.push((item.clone(), n));
            count -= n;
        }
    }
    for (k, &slot) in slots.iter().enumerate() {
        match sorted.get(k) {
            Some((item, n)) => mod_sdk::set_slot(pos, slot, Some((item, *n))),
            None => mod_sdk::set_slot(pos, slot, None),
        };
    }
}

fn vault_menu(pos: Pos) -> Menu {
    let used = (0..VAULT_SLOTS as usize)
        .filter(|&i| mod_sdk::slot(pos, i).is_some())
        .count();
    Menu::new("Grand coffre")
        .slots(0, VAULT_SLOTS as u16, 9)
        .row_begin()
        .button(1, "Ranger")
        .label(&format!("{used} / {VAULT_SLOTS} cases"))
        .row_end()
}

fn sorter_menu(pos: Pos) -> Menu {
    let model = match mod_sdk::slot(pos, 0) {
        Some((item, _)) => format!("Seul entre : {item}"),
        None => "Sans modèle : tout entre".to_string(),
    };
    Menu::new("Trieur")
        .label("Modèle (déposez un exemplaire) :")
        .slots(0, 1, 1)
        .label(&model)
        .separator()
        .label("Reçu :")
        .slots(1, SORTER_SLOTS as u16 - 1, 4)
}

fn harvester_menu() -> Menu {
    Menu::new("Moissonneuse")
        .label(&format!(
            "Récolte et replante les cultures mûres à {HARVEST_RADIUS} blocs, au même niveau."
        ))
        .slots(0, MACHINE_SLOTS as u16, 9)
}

fn drill_menu(pos: Pos) -> Menu {
    let depth = mod_sdk::data(pos)
        .and_then(|d| d.first().copied())
        .unwrap_or(0);
    let powered = mod_sdk::received_power(pos) > 0;
    Menu::new("Foreuse")
        .bar(
            if powered { "Alimentée" } else { "Éteinte" },
            powered as u8 as f32,
        )
        .bar(
            &format!("Puits : {depth} / {DRILL_DEPTH}"),
            depth as f32 / DRILL_DEPTH as f32,
        )
        .slots(0, MACHINE_SLOTS as u16, 9)
        .row_begin()
        .button(1, "Remonter la mèche")
        .row_end()
}

impl Mod for Entrepot {
    fn init(&mut self) {
        let find = |key: &str| mod_sdk::block_id(key).unwrap_or_default();
        self.vault = find("entrepot:vault");
        self.sorter = find("entrepot:sorter");
        self.harvester = find("entrepot:harvester");
        self.drill = find("entrepot:drill");
        mod_sdk::log("prêt : grand coffre, trieur, moissonneuse, foreuse");
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            let block = mod_sdk::block_base(mod_sdk::get_block(pos));
            let slots = if block == self.vault {
                VAULT_SLOTS
            } else if block == self.sorter {
                SORTER_SLOTS
            } else {
                MACHINE_SLOTS
            };
            match call {
                MachineCall::Placed => {
                    mod_sdk::add_inventory(pos, slots, Sides::ALL);
                    if block == self.harvester {
                        mod_sdk::schedule_tick(pos, HARVEST_PERIOD);
                    } else if block == self.drill {
                        mod_sdk::schedule_tick(pos, DRILL_IDLE);
                    }
                }
                MachineCall::Loaded => {
                    if block == self.harvester {
                        mod_sdk::schedule_tick(pos, HARVEST_PERIOD);
                    } else if block == self.drill {
                        mod_sdk::schedule_tick(pos, DRILL_IDLE);
                    }
                }
                MachineCall::ScheduledTick => {
                    if block == self.harvester {
                        self.harvest(pos);
                        mod_sdk::schedule_tick(pos, HARVEST_PERIOD);
                    } else if block == self.drill {
                        // Éteinte, elle ne creuse pas ; en marche, un bloc par période.
                        let worked = mod_sdk::received_power(pos) > 0 && self.dig(pos);
                        mod_sdk::schedule_tick(pos, if worked { DRILL_PERIOD } else { DRILL_IDLE });
                    }
                }
                MachineCall::NeighborChanged => {}
            }
        }
    }

    fn used(&mut self, pos: Pos) -> bool {
        let block = mod_sdk::block_base(mod_sdk::get_block(pos));
        let menu = if block == self.vault {
            vault_menu(pos)
        } else if block == self.sorter {
            sorter_menu(pos)
        } else if block == self.harvester {
            harvester_menu()
        } else if block == self.drill {
            drill_menu(pos)
        } else {
            return false;
        };
        mod_sdk::show_menu(pos, &menu);
        true
    }

    fn ui(&mut self, pos: Pos, event: UiEvent) {
        let block = mod_sdk::block_base(mod_sdk::get_block(pos));
        match event {
            UiEvent::Button(1) if block == self.vault => tidy(pos),
            UiEvent::Button(1) if block == self.drill => {
                mod_sdk::set_data(pos, &[0]);
            }
            UiEvent::Refresh => {}
            _ => return,
        }
        // Le menu montre l'état à jour.
        self.used(pos);
    }
}

mod_sdk::export_mod!(Entrepot);
