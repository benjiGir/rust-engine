//! Les circuits : un signal de 0 à 15 qui voyage dans des câbles, des interrupteurs qui
//! l'émettent, et des mécanismes qui le reçoivent (herse, pistons, lampes).
//!
//! Tout repose sur la puissance (`BlockEntity::power`) de la phase 9b : un bloc émet une
//! puissance, ses voisins actifs sont prévenus et lisent celle de leurs voisins
//! ([`received_power`](crate::machines::received_power)).
//!
//! - **Câble** : relie ses voisins câbles en un réseau. À chaque changement, le réseau
//!   entier est recalculé d'un coup : la puissance est celle de la meilleure source moins la
//!   distance. Pas de signal fantôme qui s'éteint lentement, et un réseau de 2 000 câbles
//!   coûte un parcours, pas 2 000 réveils.
//! - **Levier** (clic : marche/arrêt) et **bouton** (un signal d'une demi-seconde).
//! - **Inverseur** et **répéteur** : directionnels (l'entrée est derrière, la sortie devant),
//!   avec un délai. Un répéteur se règle d'un clic ; un inverseur plus une boucle donne une horloge.
//! - **Herse** : s'ouvre quand elle reçoit du signal.
//! - **Piston** (et piston collant) : pousse jusqu'à 12 blocs, et le collant tire un bloc en
//!   se rétractant. Il refuse de bouger un bloc qui a un état (coffre, câble...).
//!
//! Un bloc qui change d'apparence (levier baissé, lampe allumée) est un autre bloc, comme la
//! lampe : changer de bloc efface son état actif, donc [`restyle`] le recopie.

use std::rc::Rc;

use glam::{IVec3, Vec3};
use rustc_hash::FxHashSet as HashSet;

use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::grid::{BlockAccess, GridPos};
use crate::machines::{BlockBehavior, MAX_POWER, MachineContext, received_power};
use crate::registry::BlockRegistry;

/// Les six directions, dans l'ordre où elles sont rangées dans `BlockEntity::data[0]`.
pub const FACINGS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Y,
    IVec3::NEG_Y,
    IVec3::Z,
    IVec3::NEG_Z,
];

/// Un réseau de câbles n'est pas parcouru au-delà de ce nombre (le reste attendra).
const MAX_NETWORK: usize = 4096;
/// Un piston pousse au plus ce nombre de blocs.
pub const PUSH_LIMIT: usize = 12;
/// Un bouton émet pendant ce nombre de ticks (0,5 s).
const BUTTON_TICKS: u32 = 10;
/// Un piston réagit avec ce retard, pour ne pas s'agiter à chaque variation.
const PISTON_DELAY: u32 = 2;

/// La direction rangée dans l'état d'un bloc (+x par défaut).
pub fn facing_of(entity: Option<&BlockEntity>) -> IVec3 {
    entity
        .and_then(|e| e.data.first())
        .map_or(IVec3::X, |&i| FACINGS[i as usize % 6])
}

fn facing_index(direction: IVec3) -> u8 {
    FACINGS.iter().position(|&f| f == direction).unwrap_or(0) as u8
}

/// L'état d'un bloc orienté qu'on vient de poser, selon la direction du regard du joueur.
/// `None` si ce comportement n'a pas d'orientation. Les pistons visent dans les six
/// directions ; l'inverseur et le répéteur seulement à l'horizontale.
pub fn placement_state(behavior: &str, look: Vec3) -> Option<BlockEntity> {
    let horizontal = match behavior {
        "core:inverter" | "core:repeater" => true,
        "core:piston" | "core:sticky_piston" | "core:propeller" => false,
        _ => return None,
    };
    let mut look = look;
    if horizontal {
        look.y = 0.0;
    }
    let axis = look.abs().max_position();
    let mut direction = IVec3::ZERO;
    direction[axis] = if look[axis] >= 0.0 { 1 } else { -1 };
    Some(BlockEntity {
        data: vec![facing_index(direction), 0],
        ..BlockEntity::default()
    })
}

fn is_directional(registry: &BlockRegistry, block: BlockId) -> bool {
    matches!(
        registry.def(block).behavior.as_deref(),
        Some("core:inverter" | "core:repeater")
    )
}

/// La puissance que `from` envoie vers son voisin `to` : un inverseur ou un répéteur n'émet
/// que devant lui ; les autres blocs, dans toutes les directions.
pub fn power_toward(grid: &(impl BlockAccess + ?Sized), from: IVec3, to: IVec3) -> u8 {
    let Some(entity) = grid.block_entity(from) else {
        return 0;
    };
    if is_directional(grid.registry(), grid.block(from)) && from + facing_of(Some(entity)) != to {
        return 0;
    }
    entity.power
}

/// Change le bloc en `at` en gardant son état actif (modifié par `change`).
pub(crate) fn restyle(
    ctx: &mut MachineContext,
    at: GridPos,
    block: BlockId,
    change: impl FnOnce(&mut BlockEntity),
) {
    let mut entity = ctx.block_entity(at).unwrap_or_default();
    change(&mut entity);
    if ctx.block(at) != block {
        ctx.set_block(at, block);
    }
    ctx.ensure_block_entity(at);
    ctx.with_block_entity(at, |e| *e = entity);
}

fn id(registry: &BlockRegistry, key: &str) -> Option<BlockId> {
    registry.id(key)
}

// ----- Le câble -----

pub struct Wire {
    off: BlockId,
    on: BlockId,
}

impl Wire {
    fn is_wire(&self, block: BlockId) -> bool {
        block == self.off || block == self.on
    }

    /// Le réseau de câbles qui contient `start` (parcours en largeur, borné).
    fn network(&self, grid: &(impl BlockAccess + ?Sized), start: IVec3) -> Vec<IVec3> {
        let mut seen: HashSet<IVec3> = HashSet::default();
        seen.insert(start);
        let mut cells = vec![start];
        let mut next = 0;
        while next < cells.len() && cells.len() < MAX_NETWORK {
            let cell = cells[next];
            next += 1;
            for d in FACINGS {
                let n = cell + d;
                if self.is_wire(grid.block(n)) && seen.insert(n) {
                    cells.push(n);
                }
            }
        }
        cells
    }

    /// La puissance de chaque câble du réseau : la meilleure source, moins la distance.
    fn solve(&self, grid: &(impl BlockAccess + ?Sized), cells: &[IVec3]) -> Vec<u8> {
        let index: rustc_hash::FxHashMap<IVec3, usize> =
            cells.iter().enumerate().map(|(i, &c)| (c, i)).collect();
        // Les sources : tout voisin qui n'est pas un câble.
        let mut power: Vec<u8> = cells
            .iter()
            .map(|&c| {
                FACINGS
                    .iter()
                    .map(|&d| c + d)
                    .filter(|n| !index.contains_key(n))
                    .map(|n| power_toward(grid, n, c))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        // Du plus fort au plus faible : un câble à `p` donne `p - 1` à ses voisins.
        for level in (2..=MAX_POWER).rev() {
            for i in 0..cells.len() {
                if power[i] != level {
                    continue;
                }
                for d in FACINGS {
                    if let Some(&j) = index.get(&(cells[i] + d))
                        && power[j] < level - 1
                    {
                        power[j] = level - 1;
                    }
                }
            }
        }
        power
    }
}

impl BlockBehavior for Wire {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.neighbor_changed(ctx, positions);
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let mut done: HashSet<GridPos> = HashSet::default();
        for &start in positions {
            if done.contains(&start) {
                continue;
            }
            let Some(grid) = ctx.grids.get(start.grid) else {
                continue;
            };
            let (cells, power) = {
                let grid = grid.borrow();
                if !self.is_wire(grid.block(start.pos)) {
                    continue;
                }
                let cells = self.network(&*grid, start.pos);
                let power = self.solve(&*grid, &cells);
                (cells, power)
            };
            for (&cell, &p) in cells.iter().zip(&power) {
                let at = GridPos::new(start.grid, cell);
                done.insert(at);
                let current = ctx.block_entity(at).map_or(0, |e| e.power);
                let block = if p > 0 { self.on } else { self.off };
                if current != p || ctx.block(at) != block || ctx.block_entity(at).is_none() {
                    restyle(ctx, at, block, |e| e.power = p);
                    ctx.notify_neighbors(at);
                }
            }
        }
    }
}

// ----- Levier et bouton -----

/// Un interrupteur : deux blocs (relâché, actionné). `pulse` : un bouton, qui se relâche seul.
pub struct Switch {
    off: BlockId,
    on: BlockId,
    pulse: bool,
}

impl Switch {
    fn set(&self, ctx: &mut MachineContext, at: GridPos, active: bool) {
        let block = if active { self.on } else { self.off };
        let power = if active { MAX_POWER } else { 0 };
        restyle(ctx, at, block, |e| e.power = power);
        ctx.notify_neighbors(at);
    }
}

impl BlockBehavior for Switch {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.ensure_block_entity(at);
        }
    }

    fn used(&self, ctx: &mut MachineContext, at: GridPos) -> bool {
        let active = ctx.power(at) > 0;
        if self.pulse {
            if !active {
                self.set(ctx, at, true);
                ctx.schedule(at, BUTTON_TICKS);
            }
        } else {
            self.set(ctx, at, !active);
        }
        true
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            if self.pulse && ctx.power(at) > 0 {
                self.set(ctx, at, false);
            }
        }
    }
}

// ----- La herse -----

/// Une porte qui s'ouvre sous tension : fermée (un bloc plein), ouverte (on passe).
pub struct Gate {
    closed: BlockId,
    open: BlockId,
}

impl BlockBehavior for Gate {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.neighbor_changed(ctx, positions);
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let powered = ctx
                .grids
                .get(at.grid)
                .is_some_and(|g| received_power(&*g.borrow(), at.pos) > 0);
            let wanted = if powered { self.open } else { self.closed };
            if ctx.block(at) != wanted {
                ctx.set_block(at, wanted);
            }
        }
    }
}

// ----- Inverseur et répéteur -----

/// Lit l'entrée derrière lui, et émet devant après un délai : le répéteur recopie, l'inverseur
/// inverse. Deux blocs (éteint, émet). `data[0]` : la direction, `data[1]` : le réglage du
/// délai (répéteur), de 0 à 3.
pub struct Logic {
    off: BlockId,
    on: BlockId,
    invert: bool,
}

impl Logic {
    fn delay(&self, ctx: &MachineContext, at: GridPos) -> u32 {
        let setting = ctx
            .block_entity(at)
            .and_then(|e| e.data.get(1).copied())
            .unwrap_or(0) as u32;
        2 * (setting.min(3) + 1)
    }
}

impl BlockBehavior for Logic {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.ensure_block_entity(at);
            // Un inverseur sans entrée doit émettre : on le réveille.
            ctx.schedule(at, 1);
        }
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let delay = self.delay(ctx, at);
            ctx.schedule(at, delay);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let Some(entity) = ctx.block_entity(at) else {
                continue;
            };
            let back = at.pos - facing_of(Some(&entity));
            let input = ctx
                .grids
                .get(at.grid)
                .is_some_and(|g| power_toward(&*g.borrow(), back, at.pos) > 0);
            let power = if input != self.invert { MAX_POWER } else { 0 };
            if power != entity.power {
                let block = if power > 0 { self.on } else { self.off };
                restyle(ctx, at, block, |e| e.power = power);
                ctx.notify_neighbors(at);
            }
        }
    }

    /// Un clic sur un répéteur règle son délai (2, 4, 6 puis 8 ticks).
    fn used(&self, ctx: &mut MachineContext, at: GridPos) -> bool {
        if self.invert {
            return false;
        }
        ctx.with_block_entity(at, |e| {
            if e.data.len() < 2 {
                e.data.resize(2, 0);
            }
            e.data[1] = (e.data[1] + 1) % 4;
        })
        .is_some()
    }
}

// ----- Les pistons -----

pub struct Piston {
    sticky: bool,
    off: BlockId,
    on: BlockId,
    head: BlockId,
}

impl Piston {
    /// Un bloc que le piston a le droit de déplacer.
    fn movable(&self, ctx: &MachineContext, registry: &BlockRegistry, at: GridPos) -> bool {
        let block = ctx.block(at);
        let def = registry.def(block);
        !block.is_air()
            && !def.unbreakable
            && def.hardness >= 0.0
            && !def.liquid
            && block != self.head
            && !crate::bed::is_part(registry, block)
            && ctx.block_entity(at).is_none()
    }

    /// Une case où un bloc poussé peut aller : de l'air, de l'eau, une plante.
    fn empty(registry: &BlockRegistry, block: BlockId) -> bool {
        let def = registry.def(block);
        block.is_air() || def.liquid || (!def.solid && def.shape != crate::registry::Shape::Cube)
    }

    fn extend(&self, ctx: &mut MachineContext, at: GridPos, registry: &BlockRegistry) {
        let dir = facing_of(ctx.block_entity(at).as_ref());
        // La file de blocs à pousser, jusqu'à une case vide.
        let mut chain = 0;
        loop {
            let cell = at.offset(dir * (chain as i32 + 1));
            if Self::empty(registry, ctx.block(cell)) {
                break;
            }
            if chain >= PUSH_LIMIT || !self.movable(ctx, registry, cell) {
                return;
            }
            chain += 1;
        }
        // Du dernier au premier, pour ne rien écraser.
        for i in (0..chain).rev() {
            let from = at.offset(dir * (i as i32 + 1));
            let to = at.offset(dir * (i as i32 + 2));
            let block = ctx.block(from);
            ctx.set_block(to, block);
            ctx.set_block(from, BlockId::AIR);
        }
        ctx.set_block(at.offset(dir), self.head);
        restyle(ctx, at, self.on, |_| {});
    }

    fn retract(&self, ctx: &mut MachineContext, at: GridPos, registry: &BlockRegistry) {
        let dir = facing_of(ctx.block_entity(at).as_ref());
        let front = at.offset(dir);
        if ctx.block(front) == self.head {
            ctx.set_block(front, BlockId::AIR);
        }
        // Le piston collant ramène le bloc devant la tête.
        if self.sticky {
            let behind = at.offset(dir * 2);
            if self.movable(ctx, registry, behind) {
                let block = ctx.block(behind);
                ctx.set_block(front, block);
                ctx.set_block(behind, BlockId::AIR);
            }
        }
        restyle(ctx, at, self.off, |_| {});
    }
}

impl BlockBehavior for Piston {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.ensure_block_entity(at);
            ctx.schedule(at, PISTON_DELAY);
        }
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.schedule(at, PISTON_DELAY);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let Some(registry) = ctx.registry() else {
            return;
        };
        for &at in positions {
            let powered = ctx
                .grids
                .get(at.grid)
                .is_some_and(|g| received_power(&*g.borrow(), at.pos) > 0);
            let extended = ctx.block(at) == self.on;
            if powered && !extended {
                self.extend(ctx, at, &registry);
            } else if !powered && extended {
                self.retract(ctx, at, &registry);
            }
        }
    }
}

/// Tous les comportements de circuits, par nom (`core:wire`...). Un comportement absent
/// (le registre n'a pas ses blocs) est simplement omis.
pub fn natives(registry: &BlockRegistry) -> Vec<(&'static str, Rc<dyn BlockBehavior>)> {
    let mut out: Vec<(&'static str, Rc<dyn BlockBehavior>)> = Vec::new();
    if let (Some(off), Some(on)) = (id(registry, "core:wire"), id(registry, "core:wire_on")) {
        out.push(("core:wire", Rc::new(Wire { off, on })));
    }
    if let (Some(off), Some(on)) = (id(registry, "core:lever"), id(registry, "core:lever_on")) {
        out.push((
            "core:lever",
            Rc::new(Switch {
                off,
                on,
                pulse: false,
            }),
        ));
    }
    if let (Some(off), Some(on)) = (id(registry, "core:button"), id(registry, "core:button_on")) {
        out.push((
            "core:button",
            Rc::new(Switch {
                off,
                on,
                pulse: true,
            }),
        ));
    }
    if let (Some(closed), Some(open)) = (id(registry, "core:gate"), id(registry, "core:gate_open"))
    {
        out.push(("core:gate", Rc::new(Gate { closed, open })));
    }
    for (name, invert) in [("inverter", true), ("repeater", false)] {
        if let (Some(off), Some(on)) = (
            id(registry, &format!("core:{name}")),
            id(registry, &format!("core:{name}_on")),
        ) {
            let key: &'static str = if invert {
                "core:inverter"
            } else {
                "core:repeater"
            };
            out.push((key, Rc::new(Logic { off, on, invert })));
        }
    }
    let head = id(registry, "core:piston_head");
    for (key, sticky) in [("core:piston", false), ("core:sticky_piston", true)] {
        let name = key.trim_start_matches("core:");
        if let (Some(off), Some(on), Some(head)) = (
            id(registry, key),
            id(registry, &format!("core:{name}_on")),
            head,
        ) {
            out.push((
                key,
                Rc::new(Piston {
                    sticky,
                    off,
                    on,
                    head,
                }),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::machines::Machines;
    use crate::machines::tests::loaded_streamer;
    use crate::streaming::ChunkStreamer;

    struct Rig {
        s: RefCell<ChunkStreamer>,
        machines: Machines,
    }

    const ORIGIN: IVec3 = IVec3::new(4, 100, 4);

    impl Rig {
        fn new() -> Self {
            let s = loaded_streamer();
            let machines = Machines::with_natives(s.borrow().world().registry());
            let rig = Self { s, machines };
            // Un sol de pierre et de l'air au-dessus.
            let stone = rig.id("core:stone");
            for x in -2..30 {
                for z in -2..8 {
                    for y in 0..8 {
                        let block = if y == 0 { stone } else { BlockId::AIR };
                        rig.s
                            .borrow_mut()
                            .set_block(ORIGIN + IVec3::new(x, y - 1, z), block);
                    }
                }
            }
            rig
        }

        fn id(&self, key: &str) -> BlockId {
            self.s.borrow().world().registry().id(key).unwrap()
        }

        fn at(&self, x: i32, y: i32, z: i32) -> IVec3 {
            ORIGIN + IVec3::new(x, y, z)
        }

        fn place(&mut self, pos: IVec3, key: &str) {
            let block = self.id(key);
            assert!(self.s.borrow_mut().set_block(pos, block));
            self.machines.block_changed(&self.s, pos, &mut Vec::new());
        }

        /// Un bloc orienté, comme le place le jeu.
        fn place_facing(&mut self, pos: IVec3, key: &str, look: Vec3) {
            self.place(pos, key);
            let behavior = self
                .s
                .borrow()
                .world()
                .registry()
                .def(self.id(key))
                .behavior
                .clone()
                .unwrap();
            let state = placement_state(&behavior, look).unwrap();
            self.s.borrow_mut().set_block_entity(pos, Some(state));
        }

        fn tick(&mut self, n: usize) {
            for _ in 0..n {
                self.machines.run_tick(&self.s, &mut Vec::new());
            }
        }

        fn click(&mut self, pos: IVec3) -> bool {
            self.machines.use_block(&self.s, pos, &mut Vec::new())
        }

        fn block(&self, pos: IVec3) -> BlockId {
            self.s.borrow().world().block(pos)
        }

        fn power(&self, pos: IVec3) -> u8 {
            self.s.borrow().block_entity(pos).map_or(0, |e| e.power)
        }

        fn is(&self, pos: IVec3, key: &str) -> bool {
            self.block(pos) == self.id(key)
        }
    }

    #[test]
    fn a_lever_powers_a_wire_line_that_decays_and_lights_a_lamp() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:lever");
        for x in 1..=10 {
            rig.place(rig.at(x, 0, 0), "core:wire");
        }
        rig.place(rig.at(11, 0, 0), "core:lamp");
        rig.tick(5);
        assert_eq!(rig.power(rig.at(1, 0, 0)), 0, "levier relâché");
        assert!(rig.click(rig.at(0, 0, 0)));
        rig.tick(30);
        assert!(rig.is(rig.at(0, 0, 0), "core:lever_on"));
        // 15 au contact du levier, puis un de moins à chaque câble.
        for x in 1..=10 {
            assert_eq!(rig.power(rig.at(x, 0, 0)), 16 - x as u8, "câble {x}");
        }
        assert!(rig.is(rig.at(10, 0, 0), "core:wire_on"));
        assert!(rig.is(rig.at(11, 0, 0), "core:lamp_lit"), "la lampe suit");
        // Éteint : tout retombe d'un coup, sans signal fantôme.
        assert!(rig.click(rig.at(0, 0, 0)));
        rig.tick(30);
        for x in 1..=10 {
            assert_eq!(rig.power(rig.at(x, 0, 0)), 0, "câble {x}");
            assert!(rig.is(rig.at(x, 0, 0), "core:wire"));
        }
        assert!(rig.is(rig.at(11, 0, 0), "core:lamp"));
    }

    #[test]
    fn a_signal_dies_after_fifteen_wires() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:lever");
        for x in 1..=20 {
            rig.place(rig.at(x, 0, 0), "core:wire");
        }
        rig.click(rig.at(0, 0, 0));
        rig.tick(40);
        assert_eq!(rig.power(rig.at(15, 0, 0)), 1);
        assert_eq!(rig.power(rig.at(16, 0, 0)), 0);
        assert_eq!(rig.power(rig.at(20, 0, 0)), 0);
    }

    #[test]
    fn a_button_pulses_then_releases() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:button");
        rig.place(rig.at(1, 0, 0), "core:lamp");
        rig.click(rig.at(0, 0, 0));
        rig.tick(3);
        assert!(rig.is(rig.at(1, 0, 0), "core:lamp_lit"));
        assert!(rig.is(rig.at(0, 0, 0), "core:button_on"));
        rig.tick(20);
        assert!(rig.is(rig.at(0, 0, 0), "core:button"));
        assert!(rig.is(rig.at(1, 0, 0), "core:lamp"));
    }

    #[test]
    fn a_gate_opens_under_power_and_closes_again() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:lever");
        rig.place(rig.at(1, 0, 0), "core:gate");
        rig.tick(3);
        assert!(rig.is(rig.at(1, 0, 0), "core:gate"));
        rig.click(rig.at(0, 0, 0));
        rig.tick(3);
        assert!(rig.is(rig.at(1, 0, 0), "core:gate_open"));
        rig.click(rig.at(0, 0, 0));
        rig.tick(3);
        assert!(rig.is(rig.at(1, 0, 0), "core:gate"));
    }

    #[test]
    fn a_piston_pushes_a_line_and_retracts() {
        let mut rig = Rig::new();
        // Un piston qui regarde vers +x, deux blocs devant, puis du vide.
        rig.place_facing(rig.at(1, 0, 0), "core:piston", Vec3::X);
        rig.place(rig.at(2, 0, 0), "core:stone");
        rig.place(rig.at(3, 0, 0), "core:dirt");
        rig.place(rig.at(1, 0, 1), "core:lever");
        rig.tick(5);
        rig.click(rig.at(1, 0, 1));
        rig.tick(8);
        assert!(rig.is(rig.at(1, 0, 0), "core:piston_on"));
        assert!(rig.is(rig.at(2, 0, 0), "core:piston_head"));
        assert!(rig.is(rig.at(3, 0, 0), "core:stone"));
        assert!(rig.is(rig.at(4, 0, 0), "core:dirt"));
        // Rétracté : la tête disparaît, les blocs restent où ils sont (non collant).
        rig.click(rig.at(1, 0, 1));
        rig.tick(8);
        assert!(rig.is(rig.at(1, 0, 0), "core:piston"));
        assert!(rig.block(rig.at(2, 0, 0)).is_air());
        assert!(rig.is(rig.at(3, 0, 0), "core:stone"));
    }

    #[test]
    fn a_sticky_piston_pulls_the_block_back() {
        let mut rig = Rig::new();
        rig.place_facing(rig.at(1, 0, 0), "core:sticky_piston", Vec3::X);
        rig.place(rig.at(2, 0, 0), "core:stone");
        rig.place(rig.at(1, 0, 1), "core:lever");
        rig.tick(5);
        rig.click(rig.at(1, 0, 1));
        rig.tick(8);
        assert!(rig.is(rig.at(3, 0, 0), "core:stone"));
        rig.click(rig.at(1, 0, 1));
        rig.tick(8);
        assert!(rig.is(rig.at(2, 0, 0), "core:stone"), "tiré en arrière");
        assert!(rig.block(rig.at(3, 0, 0)).is_air());
    }

    #[test]
    fn a_piston_refuses_what_it_cannot_move() {
        let mut rig = Rig::new();
        // 13 blocs : un de trop.
        rig.place_facing(rig.at(0, 0, 2), "core:piston", Vec3::X);
        for x in 1..=13 {
            rig.place(rig.at(x, 0, 2), "core:stone");
        }
        rig.place(rig.at(0, 0, 3), "core:lever");
        rig.tick(5);
        rig.click(rig.at(0, 0, 3));
        rig.tick(8);
        assert!(rig.is(rig.at(0, 0, 2), "core:piston"), "pas sorti");
        // Un coffre (il a un état) n'est pas déplacé non plus.
        let mut rig = Rig::new();
        rig.place_facing(rig.at(0, 0, 2), "core:piston", Vec3::X);
        rig.place(rig.at(1, 0, 2), "core:chest");
        // Un coffre qu'on a ouvert a un état (son inventaire).
        let state = Some(BlockEntity::default());
        rig.s.borrow_mut().set_block_entity(rig.at(1, 0, 2), state);
        rig.place(rig.at(0, 0, 3), "core:lever");
        rig.click(rig.at(0, 0, 3));
        rig.tick(8);
        assert!(rig.is(rig.at(1, 0, 2), "core:chest"));
        assert!(rig.is(rig.at(0, 0, 2), "core:piston"));
    }

    #[test]
    fn an_inverter_flips_and_a_repeater_delays() {
        let mut rig = Rig::new();
        // Levier → inverseur → lampe, tous alignés vers +x.
        rig.place(rig.at(0, 0, 0), "core:lever");
        rig.place_facing(rig.at(1, 0, 0), "core:inverter", Vec3::X);
        rig.place(rig.at(2, 0, 0), "core:lamp");
        rig.tick(6);
        assert!(
            rig.is(rig.at(2, 0, 0), "core:lamp_lit"),
            "sans entrée : émet"
        );
        rig.click(rig.at(0, 0, 0));
        rig.tick(6);
        assert!(rig.is(rig.at(2, 0, 0), "core:lamp"), "avec entrée : coupé");

        // Un répéteur : la lampe s'allume après le délai, pas tout de suite.
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:lever");
        rig.place_facing(rig.at(1, 0, 0), "core:repeater", Vec3::X);
        rig.place(rig.at(2, 0, 0), "core:lamp");
        rig.tick(4);
        rig.click(rig.at(0, 0, 0));
        rig.tick(1);
        assert!(rig.is(rig.at(2, 0, 0), "core:lamp"), "trop tôt");
        rig.tick(6);
        assert!(rig.is(rig.at(2, 0, 0), "core:lamp_lit"));
        // Un clic règle un délai plus long.
        assert!(rig.click(rig.at(1, 0, 0)));
        let delay = rig.s.borrow().block_entity(rig.at(1, 0, 0)).unwrap().data[1];
        assert_eq!(delay, 1);
    }

    #[test]
    fn repeaters_only_emit_forward() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0, 0), "core:lever");
        rig.place_facing(rig.at(1, 0, 0), "core:repeater", Vec3::X);
        // À côté du répéteur (pas devant) et devant lui.
        rig.place(rig.at(1, 0, 1), "core:wire");
        rig.place(rig.at(2, 0, 0), "core:wire");
        rig.click(rig.at(0, 0, 0));
        rig.tick(20);
        assert_eq!(rig.power(rig.at(2, 0, 0)), 15, "devant : alimenté");
        assert_eq!(rig.power(rig.at(1, 0, 1)), 0, "sur le côté : rien");
    }

    #[test]
    fn two_inverters_in_a_loop_make_a_clock() {
        let mut rig = Rig::new();
        // Un inverseur dont la sortie revient à son entrée par des câbles : il oscille.
        // Entrée derrière (x = 0), sortie devant (x = 2) ; on referme la boucle par z.
        rig.place_facing(rig.at(1, 0, 0), "core:inverter", Vec3::X);
        rig.place(rig.at(2, 0, 0), "core:wire");
        rig.place(rig.at(2, 0, 1), "core:wire");
        rig.place(rig.at(1, 0, 1), "core:wire");
        rig.place(rig.at(0, 0, 1), "core:wire");
        rig.place(rig.at(0, 0, 0), "core:wire");
        let mut states = Vec::new();
        for _ in 0..80 {
            rig.tick(1);
            states.push(rig.power(rig.at(1, 0, 0)) > 0);
        }
        assert!(
            states.iter().any(|&on| on) && states.iter().any(|&on| !on),
            "pas d'oscillation : {states:?}"
        );
    }

    #[test]
    fn placement_picks_the_facing_from_the_look_direction() {
        let piston = placement_state("core:piston", Vec3::new(0.1, -0.9, 0.2)).unwrap();
        assert_eq!(facing_of(Some(&piston)), IVec3::NEG_Y);
        // Un répéteur reste à l'horizontale, même en regardant vers le bas.
        let repeater = placement_state("core:repeater", Vec3::new(-0.2, -0.9, 0.1)).unwrap();
        assert_eq!(facing_of(Some(&repeater)), IVec3::NEG_X);
        assert!(placement_state("core:lamp", Vec3::X).is_none());
    }
}
