//! Le script du mod Mécanique, écrit **uniquement avec l'API publique** (`mod_sdk`).
//!
//! - **Les arbres et les engrenages** n'ont pas de script : ce sont des conduits de rotation
//!   déclarés dans `blocks.ron` (`conduit: Kinetic`), que le jeu relie en réseau.
//! - **La manivelle** (`mecanique:crank`) est une source : un clic droit la fait tourner
//!   quelques secondes (vitesse et durée réglables dans la page d'options du mod).
//! - **La presse** (`mecanique:press`) consomme de la rotation : elle transforme l'objet de sa
//!   case d'entrée en celui de sa case de sortie, selon les recettes `mecanique:press` de
//!   `machine_recipes.ron`, d'autant plus vite que le réseau tourne vite. Son menu (clic droit)
//!   montre les deux cases, la progression et la vitesse.

use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, MachineCall, Mod, Pos, Sides};

/// Ce que la presse demande au réseau, et ce que la manivelle peut fournir.
const PRESS_IMPACT: f32 = 4.0;
const CRANK_CAPACITY: f32 = 8.0;
/// À cette vitesse (tr/min) la presse avance d'un tick de recette par tick de jeu.
const REFERENCE_SPEED: f32 = 16.0;
const INPUT: usize = 0;
const OUTPUT: usize = 1;
const STACK: u32 = 64;
const PRESS_RECIPES: &str = "mecanique:press";

/// Vitesse de la manivelle (tr/min) et durée de son tour (secondes), réglables par le joueur.
fn crank_speed() -> f32 {
    mod_sdk::setting_f32("crank_speed", 32.0).clamp(1.0, 256.0)
}

fn crank_ticks() -> u32 {
    (mod_sdk::setting_f32("crank_seconds", 5.0).clamp(1.0, 60.0) * 20.0) as u32
}

#[derive(Default)]
struct Mecanique {
    crank: Block,
    press: Block,
}

/// La progression de la presse (en ticks de recette), rangée dans les données du bloc.
fn progress(pos: Pos) -> f32 {
    match mod_sdk::data(pos).as_deref() {
        Some(&[a, b, c, d, ..]) => f32::from_le_bytes([a, b, c, d]),
        _ => 0.0,
    }
}

fn set_progress(pos: Pos, value: f32) {
    mod_sdk::set_data(pos, &value.to_le_bytes());
}

/// La recette que la presse peut faire avec le contenu de sa case d'entrée.
fn current_recipe(pos: Pos) -> Option<mod_sdk::Recipe> {
    let (item, count) = mod_sdk::slot(pos, INPUT)?;
    mod_sdk::find_recipe(PRESS_RECIPES, &[(&item, count)])
}

/// La case de sortie peut-elle recevoir ces objets ?
fn output_has_room(pos: Pos, outputs: &[(String, u32)]) -> bool {
    let Some((item, count)) = outputs.first() else {
        return false;
    };
    match mod_sdk::slot(pos, OUTPUT) {
        None => *count <= STACK,
        Some((have, n)) => have == *item && n + count <= STACK,
    }
}

/// Un tick de presse : avance la recette si elle tourne, la termine si c'est le moment.
/// Renvoie vrai si la presse doit se réveiller au tick suivant.
fn run_press(pos: Pos) -> bool {
    let speed = mod_sdk::kinetic_speed(pos).abs();
    let recipe = current_recipe(pos);
    let Some(recipe) = recipe.filter(|r| output_has_room(pos, &r.outputs)) else {
        // Rien à faire : la presse dort (une case modifiée ou la rotation la réveillera).
        set_progress(pos, 0.0);
        return false;
    };
    if speed == 0.0 {
        return false;
    }
    let done = progress(pos) + speed / REFERENCE_SPEED;
    if done < recipe.ticks as f32 {
        set_progress(pos, done);
        return true;
    }
    // Terminé : on consomme l'entrée et on range la sortie.
    set_progress(pos, 0.0);
    if let Some((item, count)) = mod_sdk::slot(pos, INPUT) {
        let need = recipe.inputs.first().map_or(1, |(_, n)| *n);
        let left = count.saturating_sub(need);
        mod_sdk::set_slot(pos, INPUT, (left > 0).then_some((item.as_str(), left)));
    }
    if let Some((item, n)) = recipe.outputs.first() {
        let total = mod_sdk::slot(pos, OUTPUT).map_or(0, |(_, have)| have) + n;
        mod_sdk::set_slot(pos, OUTPUT, Some((item.as_str(), total)));
    }
    true
}

/// Combien de fois la manivelle a tourné dans ce monde (une donnée du mod, sauvegardée avec lui).
fn crank_turns() -> u32 {
    mod_sdk::world_value("crank_turns")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn press_menu(pos: Pos) -> Menu {
    let speed = mod_sdk::kinetic_speed(pos).abs();
    let (label, fraction) = match current_recipe(pos) {
        Some(recipe) if recipe.ticks > 0 => (
            "Pressage".to_string(),
            (progress(pos) / recipe.ticks as f32).clamp(0.0, 1.0),
        ),
        Some(_) => ("Pressage".to_string(), 1.0),
        None => ("Dépose un lingot ou un pavé".to_string(), 0.0),
    };
    let status = if speed > 0.0 {
        format!("Rotation : {speed:.0} tr/min")
    } else {
        "Pas de rotation : relie une manivelle".to_string()
    };
    Menu::new("Presse")
        .label(&status)
        .label(&format!(
            "Tours de manivelle dans ce monde : {}",
            crank_turns()
        ))
        .row_begin()
        .slots(INPUT as u16, 1, 1)
        .bar(&label, fraction)
        .slots(OUTPUT as u16, 1, 1)
        .row_end()
}

impl Mecanique {
    fn setup(&mut self, pos: Pos, block: Block) {
        if block == self.crank {
            // Une source : elle n'impose sa vitesse que pendant son tour.
            mod_sdk::add_kinetic(pos, Sides::ALL);
            mod_sdk::set_kinetic(pos, 0.0, CRANK_CAPACITY, 0.0);
        } else if block == self.press {
            mod_sdk::add_kinetic(pos, Sides::ALL);
            mod_sdk::set_kinetic(pos, 0.0, 0.0, PRESS_IMPACT);
            mod_sdk::add_inventory(pos, 2, Sides::ALL);
            mod_sdk::schedule_tick(pos, 1);
        }
    }
}

impl Mod for Mecanique {
    fn init(&mut self) {
        self.crank = mod_sdk::block_id("mecanique:crank").unwrap_or_default();
        self.press = mod_sdk::block_id("mecanique:press").unwrap_or_default();
        mod_sdk::register_command("tours", "combien de tours de manivelle dans ce monde");
        mod_sdk::register_command(
            "manivelle",
            "<tr/min> : la vitesse de la manivelle (sans argument : la lire)",
        );
    }

    fn command(&mut self, name: &str, args: &str) -> Option<String> {
        match name {
            "tours" => Some(format!(
                "La manivelle a tourné {} fois dans ce monde.",
                crank_turns()
            )),
            "manivelle" => match args.trim().parse::<f32>() {
                Ok(speed) if (1.0..=256.0).contains(&speed) => {
                    mod_sdk::set_setting("crank_speed", &speed.round().to_string());
                    Some(format!("Manivelle réglée à {} tr/min.", speed.round()))
                }
                Ok(_) => Some("La vitesse va de 1 à 256 tr/min.".to_string()),
                Err(_) if args.trim().is_empty() => {
                    Some(format!("Manivelle à {} tr/min.", crank_speed()))
                }
                Err(_) => Some("Usage : /manivelle <tr/min>".to_string()),
            },
            _ => None,
        }
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            let block = mod_sdk::block_base(mod_sdk::get_block(pos));
            match call {
                MachineCall::Placed => self.setup(pos, block),
                MachineCall::Loaded => {
                    if block == self.press {
                        mod_sdk::schedule_tick(pos, 1);
                    }
                }
                MachineCall::NeighborChanged => {}
                MachineCall::ScheduledTick => {
                    if block == self.press {
                        if run_press(pos) {
                            mod_sdk::schedule_tick(pos, 1);
                        }
                    } else if block == self.crank {
                        // Fin du tour : la manivelle s'arrête.
                        mod_sdk::set_kinetic(pos, 0.0, CRANK_CAPACITY, 0.0);
                    }
                }
            }
        }
    }

    fn used(&mut self, pos: Pos) -> bool {
        let block = mod_sdk::block_base(mod_sdk::get_block(pos));
        if block == self.crank {
            mod_sdk::set_kinetic(pos, crank_speed(), CRANK_CAPACITY, 0.0);
            mod_sdk::schedule_tick(pos, crank_ticks());
            mod_sdk::set_world_value("crank_turns", &(crank_turns() + 1).to_string());
            true
        } else if block == self.press {
            mod_sdk::show_menu(pos, &press_menu(pos));
            true
        } else {
            false
        }
    }

    fn ui(&mut self, pos: Pos, event: UiEvent) {
        if event == UiEvent::Refresh {
            mod_sdk::show_menu(pos, &press_menu(pos));
        }
    }

    /// Ce que Mécanique offre aux autres mods (`mod_sdk::call_mod("mecanique", nom, args)`) :
    /// `crank_speed` (f32, tr/min) et `crank_turns` (u32, dans ce monde).
    fn call(&mut self, name: &str, _args: &[u8]) -> Option<Vec<u8>> {
        match name {
            "crank_speed" => Some(crank_speed().to_le_bytes().to_vec()),
            "crank_turns" => Some(crank_turns().to_le_bytes().to_vec()),
            _ => None,
        }
    }

    fn options(&mut self) -> Option<Menu> {
        Some(
            Menu::new("Mécanique")
                .label("Réglages enregistrés pour tous les mondes.")
                .slider(
                    1,
                    "Vitesse de la manivelle (tr/min)",
                    1.0,
                    256.0,
                    crank_speed(),
                )
                .slider(
                    2,
                    "Durée d'un tour (secondes)",
                    1.0,
                    60.0,
                    crank_ticks() as f32 / 20.0,
                ),
        )
    }

    fn options_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Slider(1, v) => {
                mod_sdk::set_setting("crank_speed", &v.round().to_string());
            }
            UiEvent::Slider(2, v) => {
                mod_sdk::set_setting("crank_seconds", &v.round().to_string());
            }
            _ => {}
        }
    }
}

mod_sdk::export_mod!(Mecanique);
