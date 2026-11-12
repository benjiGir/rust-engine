//! Les blocs qui contiennent des objets : le coffre et le four.
//!
//! Leurs objets sont dans l'inventaire générique des capacités ([`crate::Inventory`]) de
//! leur entité de bloc : sauvegardés avec le chunk, et accessibles aux mods comme
//! n'importe quel inventaire.
//!
//! # Le four
//!
//! Trois cases : l'entrée (ce qu'on fait fondre), le combustible, la sortie. Son état est
//! dans les octets libres de l'entité : le combustible qui brûle encore, sa durée totale,
//! et l'avancement de la fonte, en ticks (20 par seconde). Il travaille avec des ticks
//! programmés, un par tick tant qu'il brûle ou peut démarrer, puis il s'endort.

use std::sync::Arc;

use crate::block::BlockId;
use crate::capabilities::{Inventory, ItemStack};
use crate::chunk::BlockEntity;
use crate::crafting::{Brewing, Smelting};
use crate::grid::GridPos;
use crate::items::ItemRegistry;
use crate::machines::{BlockBehavior, MachineContext, Machines};
use crate::registry::BlockRegistry;

pub const CHEST_SLOTS: usize = 27;
pub const FURNACE_INPUT: usize = 0;
pub const FURNACE_FUEL: usize = 1;
pub const FURNACE_OUTPUT: usize = 2;
pub const BREW_INPUT: usize = 0;
pub const BREW_REAGENT: usize = 1;
pub const BREW_OUTPUT: usize = 2;

/// L'état d'un four, rangé dans les octets libres de son entité.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FurnaceState {
    /// Ticks de combustion restants, et la durée totale du combustible en cours.
    pub burn_left: u16,
    pub burn_total: u16,
    /// Avancement de la fonte en cours, et sa durée totale.
    pub progress: u16,
    pub cook_total: u16,
}

impl FurnaceState {
    pub fn read(entity: &BlockEntity) -> Self {
        let n = |i: usize| {
            entity
                .data
                .get(i * 2..i * 2 + 2)
                .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
        };
        Self {
            burn_left: n(0),
            burn_total: n(1),
            progress: n(2),
            cook_total: n(3),
        }
    }

    pub fn write(&self, entity: &mut BlockEntity) {
        entity.data = [
            self.burn_left,
            self.burn_total,
            self.progress,
            self.cook_total,
        ]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    }

    pub fn burning(&self) -> bool {
        self.burn_left > 0
    }
}

/// Crée l'inventaire d'un conteneur s'il n'existe pas.
fn ensure_inventory(ctx: &mut MachineContext, at: GridPos, slots: usize) {
    ctx.grids.write_or_create(at, |e| {
        if e.caps.as_ref().is_none_or(|c| c.inventory.is_none()) {
            e.caps_mut().inventory = Some(Inventory::new(slots));
        }
    });
}

pub struct Chest;

impl BlockBehavior for Chest {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_inventory(ctx, at, CHEST_SLOTS);
        }
    }
}

pub struct Furnace {
    items: Arc<ItemRegistry>,
    smelting: Arc<Smelting>,
}

impl Furnace {
    pub fn new(items: Arc<ItemRegistry>, smelting: Arc<Smelting>) -> Self {
        Self { items, smelting }
    }

    /// Un tick de four ; renvoie `true` s'il doit continuer au tick suivant.
    pub fn step(&self, entity: &mut BlockEntity) -> bool {
        let mut state = FurnaceState::read(entity);
        let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
            return false;
        };
        if inventory.slots.len() < 3 {
            inventory.slots.resize(3, None);
        }
        let item = |slot: &Option<ItemStack>| slot.as_ref().and_then(|s| self.items.id(&s.item));
        // Ce qu'on peut fondre : une recette pour l'entrée, et de la place à la sortie.
        let recipe = item(&inventory.slots[FURNACE_INPUT])
            .and_then(|i| self.smelting.recipe(i))
            .cloned();
        let can_smelt = recipe
            .as_ref()
            .is_some_and(|r| match &inventory.slots[FURNACE_OUTPUT] {
                None => true,
                Some(out) => {
                    self.items.id(&out.item) == Some(r.output)
                        && out.count < self.items.get(r.output).max_stack
                }
            });
        // Rallumer avec un nouveau combustible, seulement s'il y a quelque chose à fondre.
        let fuel = item(&inventory.slots[FURNACE_FUEL]).and_then(|f| self.smelting.burn_ticks(f));
        if !state.burning()
            && can_smelt
            && let Some(ticks) = fuel
        {
            take_one(&mut inventory.slots[FURNACE_FUEL]);
            state.burn_left = ticks.min(u16::MAX as u32) as u16;
            state.burn_total = state.burn_left;
        }
        if state.burning() {
            state.burn_left -= 1;
            match &recipe {
                Some(r) if can_smelt => {
                    state.cook_total = r.ticks.min(u16::MAX as u32) as u16;
                    state.progress += 1;
                    if state.progress >= state.cook_total {
                        state.progress = 0;
                        take_one(&mut inventory.slots[FURNACE_INPUT]);
                        let output = &mut inventory.slots[FURNACE_OUTPUT];
                        match output {
                            Some(stack) => stack.count += 1,
                            None => {
                                *output = Some(ItemStack {
                                    item: self.items.get(r.output).key.clone(),
                                    count: 1,
                                    damage: 0,
                                })
                            }
                        }
                    }
                }
                _ => state.progress = 0,
            }
        } else {
            // Éteint : la fonte en cours refroidit.
            state.progress = state.progress.saturating_sub(2);
        }
        let active = state.burning() || state.progress > 0 || (can_smelt && fuel.is_some());
        state.write(entity);
        active
    }
}

/// L'alambic : une fiole d'eau et un ingrédient donnent, après un moment, une potion. La
/// préparation en cours (ticks écoulés, ticks nécessaires) est dans les octets libres.
pub struct BrewingStand {
    items: Arc<ItemRegistry>,
    brewing: Arc<Brewing>,
}

/// L'avancement d'une préparation : (écoulé, total) en ticks.
pub fn brew_progress(entity: &BlockEntity) -> (u16, u16) {
    let n = |i: usize| {
        entity
            .data
            .get(i * 2..i * 2 + 2)
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]))
    };
    (n(0), n(1))
}

impl BrewingStand {
    pub fn new(items: Arc<ItemRegistry>, brewing: Arc<Brewing>) -> Self {
        Self { items, brewing }
    }

    /// Un tick ; renvoie `true` s'il doit continuer au tick suivant.
    pub fn step(&self, entity: &mut BlockEntity) -> bool {
        let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
            return false;
        };
        if inventory.slots.len() < 3 {
            inventory.slots.resize(3, None);
        }
        let item = |slot: &Option<ItemStack>| slot.as_ref().and_then(|s| self.items.id(&s.item));
        let recipe = item(&inventory.slots[BREW_INPUT])
            .zip(item(&inventory.slots[BREW_REAGENT]))
            .and_then(|(i, r)| self.brewing.recipe(i, r))
            .cloned();
        let room = |r: &crate::crafting::BrewRecipe, out: &Option<ItemStack>| match out {
            None => true,
            Some(o) => {
                self.items.id(&o.item) == Some(r.output)
                    && o.count < self.items.get(r.output).max_stack
            }
        };
        let (mut done, _) = brew_progress(entity);
        let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) else {
            return false;
        };
        let total;
        let active = match recipe {
            Some(r) if room(&r, &inventory.slots[BREW_OUTPUT]) => {
                let ticks = r.ticks.min(u16::MAX as u32) as u16;
                done += 1;
                if done >= ticks {
                    done = 0;
                    total = 0;
                    take_one(&mut inventory.slots[BREW_INPUT]);
                    take_one(&mut inventory.slots[BREW_REAGENT]);
                    match &mut inventory.slots[BREW_OUTPUT] {
                        Some(stack) => stack.count += 1,
                        slot => {
                            *slot = Some(ItemStack {
                                item: self.items.get(r.output).key.clone(),
                                count: 1,
                                damage: 0,
                            })
                        }
                    }
                } else {
                    total = ticks;
                }
                true
            }
            _ => {
                done = 0;
                total = 0;
                false
            }
        };
        entity.data = [done, total].iter().flat_map(|v| v.to_le_bytes()).collect();
        active
    }
}

impl BlockBehavior for BrewingStand {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_inventory(ctx, at, 3);
            ctx.schedule(at, 1);
        }
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            if ctx.grids.write(at, |e| self.step(e)) == Some(true) {
                ctx.schedule(at, 1);
            }
        }
    }
}

fn take_one(slot: &mut Option<ItemStack>) {
    if let Some(stack) = slot {
        stack.count -= 1;
        if stack.count == 0 {
            *slot = None;
        }
    }
}

impl BlockBehavior for Furnace {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_inventory(ctx, at, 3);
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let result = ctx.grids.write(at, |e| {
                let was_burning = FurnaceState::read(e).burning();
                let active = self.step(e);
                (active, was_burning != FurnaceState::read(e).burning())
            });
            if let Some((_, true)) = result
                && let Some(grid) = ctx.grids.get(at.grid)
            {
                grid.borrow_mut().invalidate_visuals(at.pos);
            }
            if matches!(result, Some((true, _))) {
                ctx.schedule(at, 1);
            }
        }
    }
}

/// Branche le coffre et le four sur leurs blocs (`behavior: "core:chest"`, `"core:furnace"`).
pub fn register(
    machines: &mut Machines,
    blocks: &BlockRegistry,
    items: Arc<ItemRegistry>,
    smelting: Arc<Smelting>,
    brewing: Arc<Brewing>,
) {
    let stand = std::rc::Rc::new(BrewingStand::new(items.clone(), brewing));
    let furnace = std::rc::Rc::new(Furnace::new(items, smelting));
    let chest = std::rc::Rc::new(Chest);
    for (id, def) in blocks.iter() {
        match def.behavior.as_deref() {
            Some("core:furnace") => machines.register(id, furnace.clone()),
            Some("core:chest") => machines.register(id, chest.clone()),
            Some("core:brewing_stand") => machines.register(id, stand.clone()),
            _ => {}
        }
    }
}

/// Le bloc en `pos` est-il un conteneur, et de combien de cases ?
pub fn container_slots(blocks: &BlockRegistry, block: BlockId) -> Option<usize> {
    match blocks.def(block).behavior.as_deref() {
        Some("core:chest") => Some(CHEST_SLOTS),
        Some("core:furnace" | "core:brewing_stand") => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furnace_visuals_are_invalidated_only_at_ignition_and_extinction_on_a_ship() {
        use crate::{
            BlockAccess, ChunkStreamer, Grids, StreamingConfig, SubGrids, TerrainGenerator,
        };
        use std::cell::RefCell;
        use std::rc::Rc;
        let registry = BlockRegistry::builtin();
        let (furnace, _) = furnace();
        let block = registry.id("core:furnace").unwrap();
        let streamer = RefCell::new(ChunkStreamer::new(
            TerrainGenerator::new(7),
            StreamingConfig::default(),
        ));
        let mut subs = SubGrids::new(registry);
        let id = subs.create();
        let at = GridPos::new(id, glam::IVec3::new(-1, 3, -1));
        let cell = subs.get(id).unwrap();
        cell.borrow_mut().set_block(at.pos, block);
        let mut entity = BlockEntity::default();
        let mut inventory = Inventory::new(3);
        inventory.slots[FURNACE_INPUT] = stack("core:raw_iron", 1);
        inventory.slots[FURNACE_FUEL] = stack("core:stick", 1);
        entity.caps_mut().inventory = Some(inventory);
        cell.borrow_mut().set_block_entity(at.pos, Some(entity));
        let initial_revision = cell.borrow().revision();
        let mut machines = Machines::new();
        machines.register(block, Rc::new(furnace));
        let grids = Grids::with_subgrids(&streamer, &subs);
        machines.grid_blocks_loaded(grids, &[at], &mut Vec::new());
        machines.run_grids_tick(grids, &mut Vec::new());
        assert_eq!(cell.borrow().revision(), initial_revision + 1);
        for _ in 0..20 {
            machines.run_grids_tick(grids, &mut Vec::new());
        }
        assert_eq!(
            cell.borrow().revision(),
            initial_revision + 1,
            "pas de remaillage pendant la combustion"
        );
        for _ in 0..100 {
            machines.run_grids_tick(grids, &mut Vec::new());
        }
        let grid = cell.borrow();
        assert_eq!(grid.revision(), initial_revision + 2);
        assert_eq!(grid.block(at.pos), block);
        let entity = grid.block_entity(at.pos).unwrap();
        assert!(!FurnaceState::read(entity).burning());
        assert_eq!(
            entity
                .caps
                .as_ref()
                .unwrap()
                .inventory
                .as_ref()
                .unwrap()
                .slots[FURNACE_INPUT]
                .as_ref()
                .unwrap()
                .count,
            1
        );
    }

    fn furnace() -> (Furnace, Arc<ItemRegistry>) {
        let blocks = BlockRegistry::builtin();
        let items = Arc::new(ItemRegistry::builtin(&blocks).unwrap());
        let smelting = Arc::new(Smelting::builtin(&items).unwrap());
        (Furnace::new(items.clone(), smelting), items)
    }

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: item.into(),
            count,
            damage: 0,
        })
    }

    #[test]
    fn a_furnace_smelts_with_fuel_and_stops_without() {
        let (furnace, _) = furnace();
        let mut entity = BlockEntity::default();
        let mut inventory = Inventory::new(3);
        inventory.slots[FURNACE_INPUT] = stack("core:raw_iron", 3);
        inventory.slots[FURNACE_FUEL] = stack("core:stick", 3);
        entity.caps_mut().inventory = Some(inventory);
        // Un bâton brûle 5 s (100 ticks), une fonte dure 10 s (200 ticks) : 3 bâtons
        // (300 ticks) font un lingot et demi.
        let mut ticks = 0;
        while furnace.step(&mut entity) {
            ticks += 1;
            assert!(ticks < 10_000, "le four ne s'arrête pas");
        }
        let inv = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert_eq!(inv.slots[FURNACE_OUTPUT].as_ref().unwrap().count, 1);
        assert_eq!(
            inv.slots[FURNACE_OUTPUT].as_ref().unwrap().item,
            "core:iron_ingot"
        );
        assert_eq!(inv.slots[FURNACE_INPUT].as_ref().unwrap().count, 2);
        assert!(
            inv.slots[FURNACE_FUEL].is_none(),
            "tout le combustible a brûlé"
        );
        assert!(!FurnaceState::read(&entity).burning());
    }

    #[test]
    fn no_fuel_is_wasted_without_something_to_smelt() {
        let (furnace, _) = furnace();
        let mut entity = BlockEntity::default();
        let mut inventory = Inventory::new(3);
        inventory.slots[FURNACE_FUEL] = stack("core:coal", 4);
        inventory.slots[FURNACE_INPUT] = stack("core:stone", 1); // la pierre ne fond pas
        entity.caps_mut().inventory = Some(inventory);
        assert!(!furnace.step(&mut entity));
        let inv = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert_eq!(inv.slots[FURNACE_FUEL].as_ref().unwrap().count, 4);
    }

    fn stand() -> BrewingStand {
        let blocks = BlockRegistry::builtin();
        let items = Arc::new(ItemRegistry::builtin(&blocks).unwrap());
        let brewing = Arc::new(Brewing::builtin(&items).unwrap());
        BrewingStand::new(items, brewing)
    }

    fn stocked(input: Option<ItemStack>, reagent: Option<ItemStack>) -> BlockEntity {
        let mut entity = BlockEntity::default();
        let mut inventory = Inventory::new(3);
        inventory.slots[BREW_INPUT] = input;
        inventory.slots[BREW_REAGENT] = reagent;
        entity.caps_mut().inventory = Some(inventory);
        entity
    }

    #[test]
    fn a_brewing_stand_turns_water_and_an_apple_into_a_potion() {
        let stand = stand();
        let mut entity = stocked(stack("core:water_flask", 2), stack("core:apple", 1));
        let mut ticks = 0;
        while stand.step(&mut entity) {
            ticks += 1;
            assert!(ticks < 10_000, "l'alambic ne s'arrête pas");
            if ticks == 100 {
                let (done, total) = brew_progress(&entity);
                assert_eq!((done, total), (100, 300), "avancement lisible");
            }
        }
        assert_eq!(ticks, 300, "15 secondes");
        let inv = entity.caps.as_ref().unwrap().inventory.as_ref().unwrap();
        assert_eq!(
            inv.slots[BREW_OUTPUT].as_ref().unwrap().item,
            "core:potion_healing"
        );
        assert_eq!(
            inv.slots[BREW_INPUT].as_ref().unwrap().count,
            1,
            "une fiole reste"
        );
        assert!(inv.slots[BREW_REAGENT].is_none(), "la pomme est consommée");
    }

    #[test]
    fn nothing_brews_without_a_recipe_or_with_a_full_output() {
        let stand = stand();
        // Une pierre ne donne rien.
        let mut entity = stocked(stack("core:water_flask", 1), stack("core:stone", 1));
        assert!(!stand.step(&mut entity));
        assert_eq!(brew_progress(&entity), (0, 0));
        // La sortie contient autre chose : on attend qu'elle se libère.
        let mut entity = stocked(stack("core:water_flask", 1), stack("core:apple", 1));
        entity
            .caps
            .as_mut()
            .unwrap()
            .inventory
            .as_mut()
            .unwrap()
            .slots[BREW_OUTPUT] = stack("core:potion_speed", 1);
        assert!(!stand.step(&mut entity));
    }
}
