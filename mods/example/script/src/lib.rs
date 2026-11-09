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

use mod_sdk::{Block, Event, EventKind, MachineCall, Mod, Pos};

/// 2 secondes à 20 ticks par seconde.
const BREAK_PERIOD: u32 = 40;
/// 1 chance sur 8.
const RUBY_CHANCE: u32 = 8;

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
}

impl Breaker {
    fn load(pos: Pos) -> Breaker {
        match mod_sdk::data(pos).as_deref() {
            Some(&[x, y, z, armed, ..]) => Breaker {
                facing: Pos::new(x as i8 as i32, y as i8 as i32, z as i8 as i32),
                armed: armed != 0,
            },
            // Pas encore de données : tourné vers +X.
            _ => Breaker {
                facing: Pos::new(1, 0, 0),
                armed: false,
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
            ],
        );
    }
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
                if mod_sdk::random().is_multiple_of(RUBY_CHANCE) {
                    mod_sdk::give_item(self.ruby, 1);
                    mod_sdk::log("un rubis !");
                }
            }
            Event::BlockPlaced { pos, block, facing } if block == self.breaker => {
                Breaker {
                    facing,
                    armed: false,
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
                        mod_sdk::schedule_tick(pos, BREAK_PERIOD);
                    }
                }
                // Un voisin a changé : alimenté maintenant ? Alors on s'arme.
                MachineCall::NeighborChanged => {
                    if !breaker.armed && mod_sdk::received_power(pos) > 0 {
                        breaker.armed = true;
                        breaker.save(pos);
                        mod_sdk::schedule_tick(pos, BREAK_PERIOD);
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
