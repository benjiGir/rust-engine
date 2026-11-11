//! Le script du mod d'exemple, compilé en WebAssembly par `cargo xtask build-mods`.
//!
//! - **Le casseur** (`example:breaker`) : alimenté (par une horloge voisine...), il casse
//!   le bloc devant lui et donne l'objet au joueur, au plus une fois toutes les 2 secondes.
//!   « Devant » : la direction où regardait le joueur en le posant.
//!
//!   Comme un distributeur de Minecraft, il réagit à une **impulsion** : quand un voisin
//!   lui envoie de la puissance, il s'« arme » ; son tick suivant casse et le désarme.
//!   (Vérifier seulement « suis-je alimenté ? » à heure fixe tomberait toujours au mauvais
//!   moment avec une horloge de même rythme : elle serait éteinte à chaque vérification.)
//!
//!   Ses données (sauvegardées avec le monde) : `[dx, dy, dz, armé]`.
//! - **Les rubis** : casser de la pierre donne parfois (1 fois sur 8) un rubis en plus.

use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, Event, EventKind, MachineCall, Mod, Pos};

/// Par défaut : 2 secondes à 20 ticks par seconde (réglable dans la page d'options du mod).
const DEFAULT_BREAK_PERIOD: i32 = 40;
/// Par défaut : 1 chance sur 8 (réglable de même).
const DEFAULT_RUBY_CHANCE: i32 = 8;

/// Le délai du casseur, en ticks, tel que le joueur l'a réglé.
fn break_period() -> u32 {
    mod_sdk::setting_i32("break_period", DEFAULT_BREAK_PERIOD).clamp(5, 200) as u32
}

/// Un rubis tombe 1 fois sur N.
fn ruby_chance() -> u32 {
    mod_sdk::setting_i32("ruby_chance", DEFAULT_RUBY_CHANCE).clamp(1, 64) as u32
}

#[derive(Default)]
struct Example {
    stone: Block,
    ruby: Block,
    breaker: Block,
}

/// L'état d'un casseur, rangé dans les données de son bloc.
struct Breaker {
    facing: Pos,
    armed: bool,
    /// Éteint depuis son menu : il ne s'arme plus.
    disabled: bool,
}

impl Breaker {
    fn load(pos: Pos) -> Breaker {
        match mod_sdk::data(pos).as_deref() {
            Some([x, y, z, armed, rest @ ..]) => Breaker {
                facing: Pos::new(*x as i8 as i32, *y as i8 as i32, *z as i8 as i32),
                armed: *armed != 0,
                // Les casseurs sauvegardés avant le menu n'ont que 4 octets : actifs.
                disabled: rest.first().is_some_and(|&d| d != 0),
            },
            // Pas encore de données : tourné vers +X.
            _ => Breaker {
                facing: Pos::new(1, 0, 0),
                armed: false,
                disabled: false,
            },
        }
    }

    fn save(&self, pos: Pos) {
        let f = self.facing;
        mod_sdk::set_data(
            pos,
            &[
                f.x as i8 as u8,
                f.y as i8 as u8,
                f.z as i8 as u8,
                self.armed as u8,
                self.disabled as u8,
            ],
        );
    }
}

/// Le menu d'un casseur (clic droit) : un état, un interrupteur et un bouton.
fn breaker_menu(pos: Pos) -> Menu {
    let breaker = Breaker::load(pos);
    let f = breaker.facing;
    Menu::new("Casseur")
        .label(&format!("Casse vers ({}, {}, {})", f.x, f.y, f.z))
        .bar(
            if breaker.armed { "Armé" } else { "Au repos" },
            breaker.armed as u8 as f32,
        )
        .separator()
        .toggle(1, "Actif", !breaker.disabled)
        .button(2, "Casser maintenant")
}

impl Example {
    fn try_break(&self, pos: Pos, facing: Pos) {
        let target = pos.offset(facing);
        let block = mod_sdk::get_block(target);
        // On ne casse ni l'air ni un autre casseur.
        if block != Block::AIR && block != self.breaker && mod_sdk::set_block(target, Block::AIR) {
            mod_sdk::give_item(block, 1);
        }
    }
}

impl Mod for Example {
    fn options(&mut self) -> Option<Menu> {
        Some(
            Menu::new("Mod d'exemple")
                .label("Réglages enregistrés pour tous les mondes.")
                .slider(
                    1,
                    "Délai du casseur (ticks)",
                    5.0,
                    200.0,
                    break_period() as f32,
                )
                .slider(
                    2,
                    "Chance de rubis (1 sur N)",
                    1.0,
                    64.0,
                    ruby_chance() as f32,
                ),
        )
    }

    fn options_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Slider(1, v) => {
                mod_sdk::set_setting("break_period", &(v.round() as i32).to_string());
            }
            UiEvent::Slider(2, v) => {
                mod_sdk::set_setting("ruby_chance", &(v.round() as i32).to_string());
            }
            _ => {}
        }
    }

    fn used(&mut self, pos: Pos) -> bool {
        mod_sdk::show_menu(pos, &breaker_menu(pos));
        true
    }

    fn ui(&mut self, pos: Pos, event: UiEvent) {
        let mut breaker = Breaker::load(pos);
        match event {
            UiEvent::Toggle(1, on) => {
                breaker.disabled = !on;
                breaker.save(pos);
            }
            UiEvent::Button(2) => self.try_break(pos, breaker.facing),
            UiEvent::Refresh => {}
            _ => return,
        }
        // Le menu montre l'état à jour.
        mod_sdk::show_menu(pos, &breaker_menu(pos));
    }

    fn init(&mut self) {
        let find = |key: &str| mod_sdk::block_id(key).unwrap_or_default();
        self.stone = find("core:stone");
        self.ruby = find("example:ruby");
        self.breaker = find("example:breaker");
        mod_sdk::subscribe(EventKind::BlockBroken);
        mod_sdk::subscribe(EventKind::BlockPlaced);
        mod_sdk::log("prêt : casseur et rubis");
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::BlockBroken { block, .. } if block == self.stone => {
                if mod_sdk::random().is_multiple_of(ruby_chance()) {
                    mod_sdk::give_item(self.ruby, 1);
                    mod_sdk::log("un rubis !");
                }
            }
            Event::BlockPlaced { pos, block, facing } if block == self.breaker => {
                Breaker {
                    facing,
                    armed: false,
                    disabled: false,
                }
                .save(pos);
            }
            _ => {}
        }
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            let mut breaker = Breaker::load(pos);
            match call {
                // Posé, rechargé : rien à faire tant qu'aucune impulsion n'arrive. S'il
                // était armé au moment de la sauvegarde, on reprend.
                MachineCall::Placed | MachineCall::Loaded => {
                    if breaker.armed {
                        mod_sdk::schedule_tick(pos, break_period());
                    }
                }
                // Un voisin a changé : alimenté maintenant ? Alors on s'arme.
                MachineCall::NeighborChanged => {
                    if !breaker.armed && !breaker.disabled && mod_sdk::received_power(pos) > 0 {
                        breaker.armed = true;
                        breaker.save(pos);
                        mod_sdk::schedule_tick(pos, break_period());
                    }
                }
                MachineCall::ScheduledTick => {
                    if breaker.armed {
                        self.try_break(pos, breaker.facing);
                        breaker.armed = false;
                        breaker.save(pos);
                    }
                }
            }
        }
    }
}

mod_sdk::export_mod!(Example);
