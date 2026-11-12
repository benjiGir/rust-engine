//! Le script du mod Magie, écrit **uniquement avec l'API publique** (`mod_sdk`).
//!
//! - **Le mana** : une réserve qui se remplit seule (1 par seconde, réglable dans la page d'options)
//!   et dont le maximum monte avec le niveau d'expérience. Une jauge bleue l'affiche à l'écran
//!   (`hud_bar`). Elle est sauvegardée avec le monde (`set_world_value`).
//! - **Les sorts** se composent à la **table des runes** : une rune d'effet (flamme, soin, saut)
//!   et jusqu'à deux modificateurs (puissance, portée). Chaque modificateur renforce le sort et
//!   coûte du mana en plus. Le sort inscrit est celui que la baguette lance.
//! - **La baguette** (`magie:wand`, `usable`) lance le sort inscrit sur un clic droit
//!   (`Event::ItemUsed`) ; le **cristal de mana** (`magie:mana_crystal`) rend du mana.
//! - Les commandes `/mana` et `/sort` donnent l'état.

use mod_sdk::ui::{Menu, UiEvent};
use mod_sdk::{Block, Event, EventKind, Item, MachineCall, Mod, Pos, Sides};

/// Le mana maximal à l'expérience 0, et ce que chaque niveau ajoute.
const BASE_MAX_MANA: f32 = 50.0;
const MANA_PER_LEVEL: f32 = 10.0;
/// Ce qu'un cristal de mana rend.
const CRYSTAL_MANA: f32 = 40.0;
/// Les trois cases de la table : une rune d'effet, deux modificateurs.
const TABLE_SLOTS: usize = 3;

const RUNE_FLAME: &str = "magie:rune_flame";
const RUNE_HEAL: &str = "magie:rune_heal";
const RUNE_BLINK: &str = "magie:rune_blink";
const RUNE_POWER: &str = "magie:rune_power";
const RUNE_REACH: &str = "magie:rune_reach";

/// Ce que fait un sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    /// Une explosion de feu à l'endroit visé.
    Flame,
    /// De la vie rendue au lanceur.
    Heal,
    /// Un saut vers l'endroit visé.
    Blink,
}

/// Un sort : un effet et des modificateurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spell {
    effect: Effect,
    power: u32,
    reach: u32,
}

impl Spell {
    /// Le sort que ces runes composent : une rune d'effet, puis des modificateurs (au plus deux).
    fn from_runes(runes: &[Option<String>]) -> Option<Spell> {
        let mut effect = None;
        let (mut power, mut reach) = (0, 0);
        for key in runes.iter().flatten() {
            match key.as_str() {
                RUNE_FLAME | RUNE_HEAL | RUNE_BLINK if effect.is_some() => return None,
                RUNE_FLAME => effect = Some(Effect::Flame),
                RUNE_HEAL => effect = Some(Effect::Heal),
                RUNE_BLINK => effect = Some(Effect::Blink),
                RUNE_POWER => power += 1,
                RUNE_REACH => reach += 1,
                _ => return None,
            }
        }
        Some(Spell {
            effect: effect?,
            power,
            reach,
        })
    }

    /// Le texte qu'on sauvegarde : `flame+power+reach`.
    fn encode(&self) -> String {
        let mut out = match self.effect {
            Effect::Flame => "flame",
            Effect::Heal => "heal",
            Effect::Blink => "blink",
        }
        .to_string();
        for _ in 0..self.power {
            out.push_str("+power");
        }
        for _ in 0..self.reach {
            out.push_str("+reach");
        }
        out
    }

    fn decode(text: &str) -> Option<Spell> {
        let mut parts = text.split('+');
        let effect = match parts.next()? {
            "flame" => Effect::Flame,
            "heal" => Effect::Heal,
            "blink" => Effect::Blink,
            _ => return None,
        };
        let mut spell = Spell {
            effect,
            power: 0,
            reach: 0,
        };
        for part in parts {
            match part {
                "power" => spell.power += 1,
                "reach" => spell.reach += 1,
                _ => return None,
            }
        }
        Some(spell)
    }

    /// Le mana qu'il coûte : le sort de base, plus 6 par modificateur.
    fn cost(&self) -> f32 {
        let base = match self.effect {
            Effect::Flame => 12.0,
            Effect::Heal => 15.0,
            Effect::Blink => 10.0,
        };
        base + 6.0 * (self.power + self.reach) as f32
    }

    /// Le multiple de la puissance : +60 % par rune de puissance.
    fn power_factor(&self) -> f32 {
        1.0 + 0.6 * self.power as f32
    }

    /// Le multiple de la portée : +50 % par rune de portée.
    fn reach_factor(&self) -> f32 {
        1.0 + 0.5 * self.reach as f32
    }

    fn describe(&self) -> String {
        let name = match self.effect {
            Effect::Flame => "Flamme",
            Effect::Heal => "Soin",
            Effect::Blink => "Saut",
        };
        format!(
            "{name} (puissance {}, portée {}) : {} mana",
            self.power,
            self.reach,
            self.cost()
        )
    }
}

#[derive(Default)]
struct Magie {
    wand: Option<Item>,
    crystal: Option<Item>,
    table: Block,
    /// Le mana actuel, chargé au premier besoin depuis les données du monde.
    mana: Option<f32>,
    ticks: u64,
    /// Ce que la jauge montre déjà (pour ne la redessiner que si elle change).
    shown: Option<(i32, i32)>,
}

/// Le mana maximal : il monte avec le niveau d'expérience.
fn max_mana() -> f32 {
    BASE_MAX_MANA + MANA_PER_LEVEL * mod_sdk::player().map_or(0.0, |p| p.level)
}

/// Le mana qui revient chaque seconde (réglable dans la page d'options).
fn regen_per_second() -> f32 {
    mod_sdk::setting_f32("regen", 1.0).clamp(0.1, 10.0)
}

fn current_spell() -> Option<Spell> {
    Spell::decode(&mod_sdk::world_value("spell")?)
}

impl Magie {
    fn mana(&mut self) -> f32 {
        *self.mana.get_or_insert_with(|| {
            mod_sdk::world_value("mana")
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(max_mana)
        })
    }

    fn set_mana(&mut self, value: f32) {
        self.mana = Some(value.clamp(0.0, max_mana()));
    }

    fn save_mana(&mut self) {
        let mana = self.mana();
        mod_sdk::set_world_value("mana", &format!("{mana:.1}"));
    }

    /// Met la jauge de mana à jour si ce qu'elle montre a changé.
    fn update_hud(&mut self) {
        let (now, max) = (self.mana().floor() as i32, max_mana().floor() as i32);
        if self.shown != Some((now, max)) {
            self.shown = Some((now, max));
            mod_sdk::hud_bar(
                0,
                now as f32 / max.max(1) as f32,
                [0.25, 0.5, 1.0],
                &format!("Mana {now}/{max}"),
            );
        }
    }

    /// Lance le sort inscrit, si le lanceur a assez de mana.
    fn cast(&mut self) {
        let Some(spell) = current_spell() else {
            mod_sdk::say("Aucun sort inscrit : compose-en un à la table des runes.");
            return;
        };
        if self.mana() < spell.cost() {
            mod_sdk::say(&format!(
                "Pas assez de mana ({:.0}/{:.0}).",
                self.mana(),
                spell.cost()
            ));
            return;
        }
        let Some(player) = mod_sdk::player() else {
            return;
        };
        let (eye, look) = (player.eye, player.look);
        // Où le regard touche un bloc, sinon un point au bout de la portée.
        let aim = |range: f32| {
            mod_sdk::player_target(range)
                .map(|p| (p, true))
                .unwrap_or_else(|| {
                    (
                        [
                            eye[0] + look[0] * range,
                            eye[1] + look[1] * range,
                            eye[2] + look[2] * range,
                        ],
                        false,
                    )
                })
        };
        match spell.effect {
            Effect::Flame => {
                let (point, _) = aim(18.0 * spell.reach_factor());
                mod_sdk::hurt_area(
                    point,
                    2.5 + 0.5 * spell.power as f32,
                    5.0 * spell.power_factor(),
                );
            }
            Effect::Heal => {
                mod_sdk::heal_player(5.0 * spell.power_factor());
                if spell.reach > 0 {
                    mod_sdk::give_effect("regeneration", 4.0 * spell.reach as f32, 1.0);
                }
            }
            Effect::Blink => {
                let (point, hit) = aim(12.0 * spell.reach_factor());
                // Contre un mur, on s'arrête un peu avant ; les pieds sont sous les yeux.
                let back = if hit { 1.2 } else { 0.0 };
                let eye_at = [
                    point[0] - look[0] * back,
                    point[1] - look[1] * back,
                    point[2] - look[2] * back,
                ];
                mod_sdk::teleport_player(eye_at[0], eye_at[1] - 1.62, eye_at[2]);
                if spell.power > 0 {
                    mod_sdk::give_effect("speed", 2.0 * spell.power as f32, 1.0);
                }
            }
        }
        let left = self.mana() - spell.cost();
        self.set_mana(left);
        self.save_mana();
        self.update_hud();
    }
}

/// Le sort que composent les runes posées sur la table (en `pos`).
fn table_spell(pos: Pos) -> Option<Spell> {
    let runes: Vec<Option<String>> = (0..TABLE_SLOTS)
        .map(|i| mod_sdk::slot(pos, i).map(|(item, _)| item))
        .collect();
    Spell::from_runes(&runes)
}

fn table_menu(pos: Pos) -> Menu {
    let preview = match table_spell(pos) {
        Some(spell) => format!("Sort composé : {}", spell.describe()),
        None => "Pose une rune d'effet, puis des runes de puissance ou de portée.".to_string(),
    };
    let current = current_spell().map_or_else(
        || "Sort inscrit : aucun".to_string(),
        |s| format!("Sort inscrit : {}", s.describe()),
    );
    Menu::new("Table des runes")
        .label(&preview)
        .slots(0, TABLE_SLOTS as u16, TABLE_SLOTS as u8)
        .button(1, "Inscrire ce sort")
        .separator()
        .label(&current)
}

impl Mod for Magie {
    fn init(&mut self) {
        self.wand = mod_sdk::item_id("magie:wand");
        self.crystal = mod_sdk::item_id("magie:mana_crystal");
        self.table = mod_sdk::block_id("magie:rune_table").unwrap_or_default();
        mod_sdk::subscribe(EventKind::ItemUsed);
        mod_sdk::subscribe(EventKind::Tick);
        mod_sdk::register_command("mana", "ton mana actuel");
        mod_sdk::register_command(
            "sort",
            "[effet+modificateurs] : le sort inscrit (avec un argument : en inscrire un sans runes, pour essayer)",
        );
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::ItemUsed { item } if Some(item) == self.wand => self.cast(),
            Event::ItemUsed { item } if Some(item) == self.crystal => {
                let now = self.mana();
                self.set_mana(now + CRYSTAL_MANA);
                mod_sdk::consume_held(1);
                self.save_mana();
                self.update_hud();
            }
            Event::Tick(_) => {
                self.ticks += 1;
                let (mana, max) = (self.mana(), max_mana());
                if mana < max {
                    // 20 ticks par seconde.
                    self.set_mana(mana + regen_per_second() / 20.0);
                }
                // La jauge est dessinée dès le premier tick, puis mise à jour 4 fois par seconde.
                if self.shown.is_none() || self.ticks.is_multiple_of(5) {
                    self.update_hud();
                }
                // Le mana est sauvegardé toutes les 5 secondes.
                if self.ticks.is_multiple_of(100) {
                    self.save_mana();
                }
            }
            _ => {}
        }
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        if call == MachineCall::Placed {
            for &pos in positions {
                mod_sdk::add_inventory(pos, TABLE_SLOTS as u32, Sides::ALL);
            }
        }
    }

    fn used(&mut self, pos: Pos) -> bool {
        if mod_sdk::block_base(mod_sdk::get_block(pos)) != self.table {
            return false;
        }
        mod_sdk::show_menu(pos, &table_menu(pos));
        true
    }

    fn ui(&mut self, pos: Pos, event: UiEvent) {
        match event {
            UiEvent::Button(1) => match table_spell(pos) {
                Some(spell) => {
                    // Les runes sont consommées : le sort est inscrit.
                    for i in 0..TABLE_SLOTS {
                        mod_sdk::set_slot(pos, i, None);
                    }
                    mod_sdk::set_world_value("spell", &spell.encode());
                    mod_sdk::say(&format!("Sort inscrit : {}", spell.describe()));
                }
                None => mod_sdk::say("Il faut une rune d'effet, et pas plus d'une."),
            },
            UiEvent::Refresh => {}
            _ => return,
        }
        mod_sdk::show_menu(pos, &table_menu(pos));
    }

    fn command(&mut self, name: &str, args: &str) -> Option<String> {
        match name {
            "mana" => Some(format!("Mana : {:.0}/{:.0}", self.mana(), max_mana())),
            // `/sort flame+power` inscrit un sort sans runes (pour essayer : c'est une triche).
            "sort" if !args.trim().is_empty() => match Spell::decode(args.trim()) {
                Some(spell) => {
                    mod_sdk::set_world_value("spell", &spell.encode());
                    Some(format!("Sort inscrit : {}", spell.describe()))
                }
                None => Some(
                    "Sort inconnu. Exemple : /sort flame+power+reach (flame, heal, blink)"
                        .to_string(),
                ),
            },
            "sort" => Some(current_spell().map_or_else(
                || "Aucun sort inscrit.".to_string(),
                |s| format!("Sort inscrit : {}", s.describe()),
            )),
            _ => None,
        }
    }

    fn options(&mut self) -> Option<Menu> {
        Some(
            Menu::new("Magie")
                .label("Réglages enregistrés pour tous les mondes.")
                .slider(1, "Mana rendu par seconde", 0.1, 10.0, regen_per_second()),
        )
    }

    fn options_event(&mut self, event: UiEvent) {
        if let UiEvent::Slider(1, v) = event {
            mod_sdk::set_setting("regen", &format!("{v:.1}"));
        }
    }
}

mod_sdk::export_mod!(Magie);
