//! Le script du mod Forge, écrit **uniquement avec l'API publique** (`mod_sdk`).
//!
//! - **Le four à alliages** (`forge:alloy_furnace`) : trois cases d'entrée, une de combustible, une
//!   de sortie. Une recette d'alliage (`machine_recipes.ron`, type `forge:alloy`) avance tant que
//!   le combustible brûle.
//! - **L'enclume** (`forge:anvil`) : deux cases, un marteau (jamais consommé), une sortie. Un bouton
//!   par pièce (plaque, tête de pioche, de hache, lame d'épée) ; chacun est un type de recette.

use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, MachineCall, Mod, Pos, Sides};

/// Les cases du four.
const FURNACE_INPUTS: usize = 3;
const FUEL: usize = 3;
const FURNACE_OUT: usize = 4;
const FURNACE_SLOTS: u32 = 5;
/// Les cases de l'enclume.
const ANVIL_INPUTS: usize = 2;
const HAMMER: usize = 2;
const ANVIL_OUT: usize = 3;
const ANVIL_SLOTS: u32 = 4;
const STACK: u32 = 64;

const ALLOY: &str = "forge:alloy";
const HAMMER_ITEM: &str = "forge:bronze_hammer";
/// Les boutons de l'enclume : (numéro, nom affiché, type de recette).
const FORGES: [(u32, &str, &str); 4] = [
    (1, "Plaque", "forge:anvil_plate"),
    (2, "Tête de pioche", "forge:anvil_pickaxe"),
    (3, "Tête de hache", "forge:anvil_axe"),
    (4, "Lame d'épée", "forge:anvil_sword"),
];
/// Le bouton qui répare l'outil de la première case avec un lingot d'acier de la seconde.
const REPAIR_BUTTON: u32 = 5;
const REPAIR_INGOT: &str = "forge:steel_ingot";

/// Ce qui brûle, et combien de ticks (20 par seconde).
fn fuel_ticks(item: &str) -> Option<u32> {
    match item {
        "core:coal" | "core:charcoal" => Some(1600),
        "core:stick" => Some(100),
        _ => None,
    }
}

#[derive(Default)]
struct Forge {
    furnace: Block,
    anvil: Block,
}

/// L'état du four, rangé dans les données du bloc : combustible restant, durée totale de la
/// charge en cours, progression de la recette (en ticks).
#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Fire {
    left: u32,
    total: u32,
    progress: u32,
}

impl Fire {
    fn load(pos: Pos) -> Fire {
        let Some(d) = mod_sdk::data(pos).filter(|d| d.len() >= 12) else {
            return Fire::default();
        };
        let n = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        Fire {
            left: n(0),
            total: n(4),
            progress: n(8),
        }
    }

    fn save(&self, pos: Pos) {
        let mut bytes = Vec::with_capacity(12);
        for v in [self.left, self.total, self.progress] {
            bytes.extend(v.to_le_bytes());
        }
        mod_sdk::set_data(pos, &bytes);
    }
}

/// Le contenu des cases `first..first + count`, au format de `find_recipe`.
fn contents(pos: Pos, first: usize, count: usize) -> Vec<(String, u32)> {
    let mut all: Vec<(String, u32)> = Vec::new();
    for i in first..first + count {
        if let Some((item, n)) = mod_sdk::slot(pos, i) {
            match all.iter_mut().find(|(k, _)| *k == item) {
                Some((_, t)) => *t += n,
                None => all.push((item, n)),
            }
        }
    }
    all
}

fn find(kind: &str, pos: Pos, first: usize, count: usize) -> Option<mod_sdk::Recipe> {
    let have = contents(pos, first, count);
    let refs: Vec<(&str, u32)> = have.iter().map(|(k, n)| (k.as_str(), *n)).collect();
    mod_sdk::find_recipe(kind, &refs)
}

/// La case de sortie peut-elle recevoir ces objets ?
fn room(pos: Pos, slot: usize, outputs: &[(String, u32)]) -> bool {
    let Some((item, count)) = outputs.first() else {
        return false;
    };
    match mod_sdk::slot(pos, slot) {
        None => *count <= STACK,
        Some((have, n)) => have == *item && n + count <= STACK,
    }
}

/// Retire `need` exemplaires de `item` des cases `first..first + count`.
fn take(pos: Pos, first: usize, count: usize, item: &str, mut need: u32) {
    for i in first..first + count {
        if need == 0 {
            return;
        }
        let Some((have, n)) = mod_sdk::slot(pos, i) else {
            continue;
        };
        if have != item {
            continue;
        }
        let used = n.min(need);
        need -= used;
        let left = n - used;
        mod_sdk::set_slot(pos, i, (left > 0).then_some((item, left)));
    }
}

/// Consomme les ingrédients de la recette et range sa production.
fn complete(pos: Pos, first: usize, count: usize, out: usize, recipe: &mod_sdk::Recipe) {
    for (item, n) in &recipe.inputs {
        take(pos, first, count, item, *n);
    }
    if let Some((item, n)) = recipe.outputs.first() {
        let total = mod_sdk::slot(pos, out).map_or(0, |(_, have)| have) + n;
        mod_sdk::set_slot(pos, out, Some((item.as_str(), total)));
    }
}

/// Un tick du four. `true` s'il doit se réveiller au tick suivant (feu allumé ou travail à faire).
fn run_furnace(pos: Pos) -> bool {
    let mut fire = Fire::load(pos);
    let recipe = find(ALLOY, pos, 0, FURNACE_INPUTS).filter(|r| room(pos, FURNACE_OUT, &r.outputs));
    // Allumer : une recette attend et le feu est éteint.
    if fire.left == 0
        && recipe.is_some()
        && let Some((item, n)) = mod_sdk::slot(pos, FUEL)
        && let Some(ticks) = fuel_ticks(&item)
    {
        let left = n - 1;
        mod_sdk::set_slot(pos, FUEL, (left > 0).then_some((item.as_str(), left)));
        fire.left = ticks;
        fire.total = ticks;
    }
    if fire.left > 0 {
        fire.left -= 1;
        if let Some(recipe) = &recipe {
            fire.progress += 1;
            if fire.progress >= recipe.ticks {
                fire.progress = 0;
                complete(pos, 0, FURNACE_INPUTS, FURNACE_OUT, recipe);
            }
        } else {
            fire.progress = 0;
        }
    } else {
        fire.progress = 0;
    }
    fire.save(pos);
    // Sans combustible, une recette en attente n'est pas du travail : on vérifiera plus tard.
    fire.left > 0 || (recipe.is_some() && mod_sdk::slot(pos, FUEL).is_some())
}

fn furnace_menu(pos: Pos) -> Menu {
    let fire = Fire::load(pos);
    let flame = if fire.total > 0 {
        fire.left as f32 / fire.total as f32
    } else {
        0.0
    };
    let recipe = find(ALLOY, pos, 0, FURNACE_INPUTS);
    let (text, fraction) = match &recipe {
        Some(r) if r.ticks > 0 => (
            "Fusion",
            (fire.progress as f32 / r.ticks as f32).clamp(0.0, 1.0),
        ),
        _ => ("Alliage : fer + charbon, ou cuivre + malachite", 0.0),
    };
    Menu::new("Four à alliages")
        .bar(if fire.left > 0 { "Allumé" } else { "Éteint" }, flame)
        .row_begin()
        .slots(0, FURNACE_INPUTS as u16, 3)
        .bar(text, fraction)
        .slots(FURNACE_OUT as u16, 1, 1)
        .row_end()
        .label("Combustible (charbon, bâton) :")
        .slots(FUEL as u16, 1, 1)
}

/// L'état du marteau, pour le menu : « Marteau : 150 / 200 usages ».
fn hammer_label(pos: Pos) -> String {
    match mod_sdk::slot(pos, HAMMER) {
        Some((item, _)) if item == HAMMER_ITEM => {
            let max = mod_sdk::item_durability(HAMMER_ITEM);
            let used = mod_sdk::slot_damage(pos, HAMMER).unwrap_or(0);
            format!(
                "Marteau de bronze : {} / {max} usages",
                max.saturating_sub(used)
            )
        }
        _ => "Marteau de bronze :".to_string(),
    }
}

/// Un coup de marteau : une usure de plus, et il se brise quand il n'en peut plus.
fn wear_hammer(pos: Pos) {
    let used = mod_sdk::slot_damage(pos, HAMMER).unwrap_or(0) + 1;
    if used >= mod_sdk::item_durability(HAMMER_ITEM) {
        mod_sdk::set_slot(pos, HAMMER, None);
        mod_sdk::say("Le marteau de bronze se brise.");
    } else {
        mod_sdk::set_slot_damage(pos, HAMMER, used);
    }
}

fn anvil_menu(pos: Pos) -> Menu {
    let mut menu = Menu::new("Enclume")
        .label(&hammer_label(pos))
        .slots(HAMMER as u16, 1, 1)
        .label("Lingots :")
        .row_begin()
        .slots(0, ANVIL_INPUTS as u16, 2)
        .slots(ANVIL_OUT as u16, 1, 1)
        .row_end()
        .separator()
        .row_begin();
    for (id, name, _) in FORGES {
        menu = menu.button(id, name);
    }
    menu = menu.button(REPAIR_BUTTON, "Réparer");
    menu.row_end()
        .label("Réparer : l'outil dans la première case, un lingot d'acier dans la seconde.")
}

/// Forge la pièce du bouton `button` : le marteau doit être là, la recette faisable, la sortie libre.
fn forge(pos: Pos, kind: &str) {
    if mod_sdk::slot(pos, HAMMER).is_none_or(|(item, _)| item != HAMMER_ITEM) {
        mod_sdk::say("Il faut un marteau de bronze.");
        return;
    }
    let Some(recipe) = find(kind, pos, 0, ANVIL_INPUTS) else {
        mod_sdk::say("Pas assez de lingots d'acier pour cette pièce.");
        return;
    };
    if !room(pos, ANVIL_OUT, &recipe.outputs) {
        mod_sdk::say("La sortie est occupée.");
        return;
    }
    complete(pos, 0, ANVIL_INPUTS, ANVIL_OUT, &recipe);
    wear_hammer(pos);
}

/// Répare l'outil de la case 0 : un lingot d'acier (case 1) rend un quart de sa durabilité.
fn repair(pos: Pos) {
    if mod_sdk::slot(pos, HAMMER).is_none_or(|(item, _)| item != HAMMER_ITEM) {
        mod_sdk::say("Il faut un marteau de bronze.");
        return;
    }
    let Some((tool, _)) = mod_sdk::slot(pos, 0) else {
        mod_sdk::say("Posez l'outil à réparer dans la première case.");
        return;
    };
    let max = mod_sdk::item_durability(&tool);
    let worn = mod_sdk::slot_damage(pos, 0).unwrap_or(0);
    if max == 0 || worn == 0 {
        mod_sdk::say("Cet objet n'a rien à réparer.");
        return;
    }
    if mod_sdk::slot(pos, 1).is_none_or(|(item, _)| item != REPAIR_INGOT) {
        mod_sdk::say("Il faut un lingot d'acier dans la seconde case.");
        return;
    }
    take(pos, 1, 1, REPAIR_INGOT, 1);
    mod_sdk::set_slot_damage(pos, 0, worn.saturating_sub(max.div_ceil(4)));
    wear_hammer(pos);
}

impl Mod for Forge {
    fn init(&mut self) {
        self.furnace = mod_sdk::block_id("forge:alloy_furnace").unwrap_or_default();
        self.anvil = mod_sdk::block_id("forge:anvil").unwrap_or_default();
        mod_sdk::log("prêt : four à alliages et enclume");
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            let block = mod_sdk::block_base(mod_sdk::get_block(pos));
            match call {
                MachineCall::Placed => {
                    if block == self.furnace {
                        mod_sdk::add_inventory(pos, FURNACE_SLOTS, Sides::ALL);
                        mod_sdk::schedule_tick(pos, 10);
                    } else if block == self.anvil {
                        mod_sdk::add_inventory(pos, ANVIL_SLOTS, Sides::ALL);
                    }
                }
                MachineCall::Loaded => {
                    if block == self.furnace {
                        mod_sdk::schedule_tick(pos, 10);
                    }
                }
                MachineCall::ScheduledTick if block == self.furnace => {
                    // Au travail : un tick par tick ; au repos : on regarde toutes les demi-secondes.
                    mod_sdk::schedule_tick(pos, if run_furnace(pos) { 1 } else { 10 });
                }
                _ => {}
            }
        }
    }

    fn used(&mut self, pos: Pos) -> bool {
        let block = mod_sdk::block_base(mod_sdk::get_block(pos));
        let menu = if block == self.furnace {
            furnace_menu(pos)
        } else if block == self.anvil {
            anvil_menu(pos)
        } else {
            return false;
        };
        mod_sdk::show_menu(pos, &menu);
        true
    }

    fn ui(&mut self, pos: Pos, event: UiEvent) {
        match event {
            UiEvent::Button(REPAIR_BUTTON) => repair(pos),
            UiEvent::Button(id) => {
                if let Some((_, _, kind)) = FORGES.iter().find(|(n, _, _)| *n == id) {
                    forge(pos, kind);
                }
            }
            UiEvent::Refresh => {}
            _ => return,
        }
        // Le menu montre l'état à jour.
        self.used(pos);
    }
}

mod_sdk::export_mod!(Forge);
