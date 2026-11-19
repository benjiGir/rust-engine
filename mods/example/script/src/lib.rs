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
//! - **Les rubis** (ceux du jeu, `core:ruby`) : casser de la pierre donne parfois (1 fois sur 8)
//!   un rubis en plus ; la commande `rubis` en donne ; un chunk neuf reçoit parfois un minerai de
//!   rubis posé sur le sol.

use mod_sdk::mobs::MobView;
use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, Event, EventKind, Item, MachineCall, Mod, Pos};

/// À cette distance (blocs), un gardien voit le joueur et le poursuit.
const GUARD_SIGHT: f32 = 18.0;
/// Par défaut : 2 secondes à 20 ticks par seconde (réglable dans la page d'options du mod).
const DEFAULT_BREAK_PERIOD: i32 = 40;
/// Par défaut : 1 chance sur 8 (réglable de même).
const DEFAULT_RUBY_CHANCE: i32 = 8;

/// Le délai du casseur, en ticks, tel que le joueur l'a réglé.
fn break_period() -> u32 {
    mod_sdk::setting_i32("break_period", DEFAULT_BREAK_PERIOD).clamp(5, 200) as u32
}

/// Un chunk neuf reçoit un minerai de rubis en surface 1 fois sur N (0 : jamais).
fn surface_ruby_chance() -> u32 {
    mod_sdk::setting_i32("surface_ruby_chance", 24).clamp(0, 1000) as u32
}

/// Décore un chunk neuf : un minerai de rubis posé sur le sol, à un endroit tiré au hasard. On descend
/// le long d'une colonne jusqu'à trouver de l'air au-dessus d'un bloc plein. Écrire dans le
/// chunk annoncé seulement : les blocs hors des chunks chargés ne se posent pas.
fn decorate(chunk: Pos, ruby: Block) {
    let chance = surface_ruby_chance();
    if chance == 0 || !mod_sdk::random().is_multiple_of(chance) {
        return;
    }
    let origin = chunk.chunk_origin();
    let (dx, dz) = (
        (mod_sdk::random() % mod_sdk::CHUNK_SIZE as u32) as i32,
        (mod_sdk::random() % mod_sdk::CHUNK_SIZE as u32) as i32,
    );
    for dy in (0..mod_sdk::CHUNK_SIZE - 1).rev() {
        let here = Pos::new(origin.x + dx, origin.y + dy, origin.z + dz);
        let above = Pos::new(here.x, here.y + 1, here.z);
        if mod_sdk::get_block(here) != Block::AIR && mod_sdk::get_block(above) == Block::AIR {
            mod_sdk::set_block(above, ruby);
            return;
        }
    }
}

/// Un rubis tombe 1 fois sur N.
fn ruby_chance() -> u32 {
    mod_sdk::setting_i32("ruby_chance", DEFAULT_RUBY_CHANCE).clamp(1, 64) as u32
}

#[derive(Default)]
struct Example {
    stone: Block,
    /// Le rubis du jeu (un objet) et son minerai (un bloc).
    ruby: Item,
    ruby_ore: Block,
    breaker: Block,
    /// Le type de créature du gardien (`brain: true` : ce script le dirige).
    guard: Option<u32>,
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
    /// Les gardiens proches, tous d'un coup. Trois états d'esprit rangés dans la mémoire de la
    /// créature : 0 il rôde, 1 il poursuit le joueur, 2 il fuit (blessé).
    fn mobs(&mut self, mobs: &[MobView]) {
        let Some(guard) = self.guard else { return };
        for m in mobs.iter().filter(|m| m.kind == guard) {
            let [dx, _, dz] = m.to_player;
            let distance = m.distance_to_player();
            let mood = if m.health < m.max_health * 0.3 {
                2
            } else if distance < GUARD_SIGHT {
                1
            } else {
                0
            };
            if mood != m.state {
                mod_sdk::mob_state(m.id, mood);
            }
            match mood {
                2 => mod_sdk::mob_move(m.id, -dx, -dz, 4.0),
                // Il poursuit en cherchant son chemin : un mur ne l'arrête plus.
                1 if distance > 2.0 => mod_sdk::mob_move_to(
                    m.id,
                    [
                        m.position[0] + m.to_player[0],
                        m.position[1] + m.to_player[1],
                        m.position[2] + m.to_player[2],
                    ],
                    3.0,
                ),
                1 => {
                    mod_sdk::mob_move(m.id, 0.0, 0.0, 0.0);
                    mod_sdk::mob_attack(m.id, 2.0);
                }
                _ => match mod_sdk::random() % 4 {
                    // Au repos : parfois il s'arrête, parfois il part dans une direction au hasard.
                    0 => mod_sdk::mob_move(m.id, 0.0, 0.0, 0.0),
                    1 => {
                        let angle = (mod_sdk::random() % 628) as f32 / 100.0;
                        mod_sdk::mob_move(m.id, angle.cos(), angle.sin(), 1.0);
                    }
                    _ => {}
                },
            }
        }
    }

    fn command(&mut self, name: &str, args: &str) -> Option<String> {
        if name != "rubis" {
            return None;
        }
        let count = args.trim().parse::<u32>().unwrap_or(1).clamp(1, 64);
        mod_sdk::give_stack(self.ruby, count);
        Some(format!("{count} rubis donné(s)."))
    }

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
        self.ruby = mod_sdk::item_id("core:ruby").unwrap_or_default();
        self.ruby_ore = find("core:ruby_ore");
        self.breaker = find("example:breaker");
        self.guard = mod_sdk::mob_kind("example:guard");
        mod_sdk::subscribe(EventKind::BlockBroken);
        mod_sdk::subscribe(EventKind::BlockPlaced);
        mod_sdk::subscribe(EventKind::ChunkGenerated);
        mod_sdk::register_command(
            "rubis",
            "[nombre] : se donner des rubis (1 par défaut, 64 au plus)",
        );
        mod_sdk::log("prêt : casseur et rubis");
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::ChunkGenerated { chunk } => decorate(chunk, self.ruby_ore),
            Event::BlockBroken { block, .. } if block == self.stone => {
                if mod_sdk::random().is_multiple_of(ruby_chance()) {
                    mod_sdk::give_stack(self.ruby, 1);
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
