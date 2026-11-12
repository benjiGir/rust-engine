//! Le script du mod Donjons, écrit **uniquement avec l'API publique** (`mod_sdk`). Le moteur génère
//! les salles (voir `structures.ron`) ; ce script leur donne vie :
//!
//! - **Le piège à pointes** (`donjons:spike_trap`) blesse le joueur qui marche dessus.
//! - **La cage à gardiens** (`donjons:spawner`) fait surgir des rôdeurs quand le joueur approche,
//!   puis disparaît. Le **sceau** (`donjons:boss_spawner`) fait de même pour le boss.
//! - **Le gardien de la crypte** (`donjons:warden`) est un boss dirigé par `Mod::mobs` : il poursuit
//!   le joueur, charge, frappe fort, appelle des renforts à mi-vie, et affiche sa jauge. À sa mort il
//!   laisse un coffre de trésor.

use mod_sdk::mobs::MobView;
use mod_sdk::{Block, Event, EventKind, MachineCall, Mod, Pos, Sides};

/// Les ticks (20 par seconde) entre deux vérifications d'un piège, et d'un générateur.
const TRAP_PERIOD: u32 = 6;
const SPAWNER_PERIOD: u32 = 20;
/// Ce que fait un piège.
const SPIKE_DAMAGE: f32 = 3.0;
/// À quelle distance un générateur se déclenche (blocs).
const SPAWNER_RANGE: f32 = 12.0;
const BOSS_RANGE: f32 = 14.0;
/// Au-delà de cette distance, le gardien de la crypte ne voit pas le joueur.
const WARDEN_SIGHT: f32 = 26.0;

/// Les états d'esprit du gardien, rangés dans sa mémoire (`mob_state`) : le bas du mot est la phase,
/// puis un compteur, puis le drapeau « renforts appelés ».
const IDLE: u32 = 0;
const CHASE: u32 = 1;
const CHARGE: u32 = 2;
const PHASE_MASK: u32 = 0xF;
const TIMER_SHIFT: u32 = 4;
const TIMER_MASK: u32 = 0xFF;
const SUMMONED: u32 = 1 << 12;

#[derive(Default)]
struct Donjons {
    spike: Block,
    spawner: Block,
    boss_spawner: Block,
    chest: Block,
    warden: Option<u32>,
    /// Une jauge de boss est affichée.
    bar_shown: bool,
}

/// Le joueur est-il sur cette case (ses pieds dans le cube) ? `reach` : la demi-largeur tolérée.
fn player_on(pos: Pos, reach: f32) -> bool {
    let Some(p) = mod_sdk::player() else {
        return false;
    };
    let feet_y = p.eye[1] - 1.62;
    (p.eye[0] - (pos.x as f32 + 0.5)).abs() < reach
        && (p.eye[2] - (pos.z as f32 + 0.5)).abs() < reach
        && feet_y > pos.y as f32 - 0.3
        && feet_y < pos.y as f32 + 1.0
}

/// La distance (blocs) du joueur à cette case.
fn player_distance(pos: Pos) -> Option<f32> {
    let p = mod_sdk::player()?;
    let d = [
        p.eye[0] - (pos.x as f32 + 0.5),
        p.eye[1] - (pos.y as f32 + 0.5),
        p.eye[2] - (pos.z as f32 + 0.5),
    ];
    Some((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
}

impl Donjons {
    /// Un générateur : à l'approche du joueur, il fait surgir `kind` (`count` fois autour de lui) puis
    /// s'efface. Renvoie vrai s'il s'est déclenché.
    fn trigger(&self, pos: Pos, range: f32, kind: &str, count: u32) -> bool {
        if player_distance(pos).is_none_or(|d| d > range) {
            return false;
        }
        for i in 0..count {
            // Autour de la cage, en cercle, un peu au-dessus du sol.
            let angle = i as f32 / count.max(1) as f32 * std::f32::consts::TAU;
            mod_sdk::spawn_entity(
                kind,
                pos.x as f32 + 0.5 + 1.8 * angle.cos(),
                pos.y as f32 + 0.5,
                pos.z as f32 + 0.5 + 1.8 * angle.sin(),
            );
        }
        mod_sdk::set_block(pos, Block::AIR);
        true
    }

    /// Un coup du gardien : des ordres à une créature de son lot.
    fn run_warden(&self, m: &MobView) {
        let distance = m.distance_to_player();
        let (dx, dz) = (m.to_player[0], m.to_player[2]);
        let phase = m.state & PHASE_MASK;
        let timer = (m.state >> TIMER_SHIFT) & TIMER_MASK;
        let summoned = m.state & SUMMONED != 0;
        let keep = |phase: u32, timer: u32, summoned: bool| {
            phase | (timer << TIMER_SHIFT) | if summoned { SUMMONED } else { 0 }
        };
        // À mi-vie, il appelle des renforts (une seule fois).
        if !summoned && m.health < m.max_health * 0.5 {
            for i in 0..3 {
                let angle = i as f32 * 2.1;
                mod_sdk::spawn_entity(
                    "core:prowler",
                    m.position[0] + 2.5 * angle.cos(),
                    m.position[1] + 0.5,
                    m.position[2] + 2.5 * angle.sin(),
                );
            }
            mod_sdk::mob_state(m.id, keep(phase, timer, true));
            return;
        }
        if distance > WARDEN_SIGHT {
            // Il ne voit pas le joueur : il reste en place.
            if phase != IDLE {
                mod_sdk::mob_state(m.id, keep(IDLE, 0, summoned));
            }
            mod_sdk::mob_move(m.id, 0.0, 0.0, 0.0);
            return;
        }
        match phase {
            CHARGE => {
                // Une charge : il fonce, et frappe s'il touche.
                if distance <= 2.8 {
                    mod_sdk::mob_attack(m.id, 8.0);
                    mod_sdk::mob_state(m.id, keep(CHASE, 6, summoned));
                } else if timer == 0 {
                    mod_sdk::mob_state(m.id, keep(CHASE, 8, summoned));
                } else {
                    mod_sdk::mob_move(m.id, dx, dz, 7.0);
                    mod_sdk::mob_state(m.id, keep(CHARGE, timer - 1, summoned));
                }
            }
            _ => {
                // Il poursuit ; à bonne distance, et s'il n'a pas chargé depuis un moment, il charge.
                if distance <= 2.8 {
                    mod_sdk::mob_move(m.id, 0.0, 0.0, 0.0);
                    mod_sdk::mob_attack(m.id, 6.0);
                } else {
                    // Il poursuit le joueur en cherchant son chemin (la charge, elle, fonce droit).
                    let target = [
                        m.position[0] + m.to_player[0],
                        m.position[1] + m.to_player[1],
                        m.position[2] + m.to_player[2],
                    ];
                    mod_sdk::mob_move_to(m.id, target, 2.6);
                }
                if timer > 0 {
                    mod_sdk::mob_state(m.id, keep(CHASE, timer - 1, summoned));
                } else if (6.0..14.0).contains(&distance) {
                    mod_sdk::mob_state(m.id, keep(CHARGE, 5, summoned));
                } else if phase != CHASE {
                    mod_sdk::mob_state(m.id, keep(CHASE, 0, summoned));
                }
            }
        }
    }
}

impl Mod for Donjons {
    fn init(&mut self) {
        let block = |key: &str| mod_sdk::block_id(key).unwrap_or_default();
        self.spike = block("donjons:spike_trap");
        self.spawner = block("donjons:spawner");
        self.boss_spawner = block("donjons:boss_spawner");
        self.chest = block("core:chest");
        self.warden = mod_sdk::mob_kind("donjons:warden");
        mod_sdk::subscribe(EventKind::EntityDied);
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            let block = mod_sdk::block_base(mod_sdk::get_block(pos));
            match call {
                MachineCall::Placed | MachineCall::Loaded => {
                    let period = if block == self.spike {
                        TRAP_PERIOD
                    } else {
                        SPAWNER_PERIOD
                    };
                    mod_sdk::schedule_tick(pos, period);
                }
                MachineCall::ScheduledTick => {
                    if block == self.spike {
                        if player_on(pos, 0.85) {
                            mod_sdk::hurt_player(SPIKE_DAMAGE);
                        }
                        mod_sdk::schedule_tick(pos, TRAP_PERIOD);
                    } else if block == self.spawner {
                        if !self.trigger(pos, SPAWNER_RANGE, "core:prowler", 3) {
                            mod_sdk::schedule_tick(pos, SPAWNER_PERIOD);
                        }
                    } else if block == self.boss_spawner
                        && !self.trigger(pos, BOSS_RANGE, "donjons:warden", 1)
                    {
                        mod_sdk::schedule_tick(pos, SPAWNER_PERIOD);
                    }
                }
                MachineCall::NeighborChanged => {}
            }
        }
    }

    /// Les gardiens proches, tous d'un coup ; la jauge du boss est mise à jour, ou retirée quand
    /// il n'y en a plus (le jeu envoie alors un lot vide).
    fn mobs(&mut self, mobs: &[MobView]) {
        let Some(warden) = self.warden else { return };
        let mut boss = None;
        for m in mobs.iter().filter(|m| m.kind == warden) {
            self.run_warden(m);
            boss = Some(*m);
        }
        match boss {
            Some(m) => {
                mod_sdk::hud_bar(
                    1,
                    m.health / m.max_health.max(1.0),
                    [0.8, 0.1, 0.1],
                    &format!("Gardien de la crypte {:.0}/{:.0}", m.health, m.max_health),
                );
                self.bar_shown = true;
            }
            None if self.bar_shown => {
                mod_sdk::hud_clear(1);
                self.bar_shown = false;
            }
            None => {}
        }
    }

    /// À sa mort, le gardien laisse un coffre de trésor à sa place.
    fn event(&mut self, event: Event) {
        let Event::EntityDied { kind, pos } = event else {
            return;
        };
        if kind != "donjons:warden" {
            return;
        }
        mod_sdk::hud_clear(1);
        self.bar_shown = false;
        if mod_sdk::get_block(pos) == Block::AIR && mod_sdk::set_block(pos, self.chest) {
            mod_sdk::add_inventory(pos, 27, Sides::ALL);
            mod_sdk::set_slot(pos, 0, Some(("core:gold_ingot", 12)));
            mod_sdk::set_slot(pos, 1, Some(("core:sapphire", 4)));
            mod_sdk::set_slot(pos, 2, Some(("core:iron_ingot", 16)));
            mod_sdk::say("Le gardien s'effondre : un trésor apparaît.");
        }
    }
}

mod_sdk::export_mod!(Donjons);
