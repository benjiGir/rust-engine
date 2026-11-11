//! Les fonctions que le jeu expose aux mods, dans le module d'import « voxel ».
//!
//! Conventions (détaillées dans `docs/modding.md`) :
//! - un bloc se désigne par **4 entiers** : `grid, x, y, z` (`grid = 0` : le monde ; les
//!   structures mobiles auront les leurs) ;
//! - une face est un entier de 0 à 5 (+X, -X, +Y, -Y, +Z, -Z), ou -1 pour « de l'intérieur » ;
//! - un texte se passe en `(pointeur, taille)` dans la mémoire du mod ;
//! - `simulate != 0` : l'opération est seulement simulée (« combien accepterais-tu ? ») ;
//! - en cas d'échec (grille ou bloc inexistant), la valeur renvoyée est 0 ou -1.
//!
//! Une erreur renvoyée par une fonction (`Err`, par exemple un pointeur hors de la
//! mémoire du mod) arrête le mod comme un plantage.

use std::cell::RefCell;

use engine_voxel::{
    BlockAccess, BlockEntity, BlockId, ChunkStreamer, EnergyStorage, Face, FluidTank, GridId,
    GridPos, Inventory, Kinetic, MachineAction, Sides, received_power,
};
use glam::{IVec3, Vec3};
use wasmtime::{Caller, Linker, format_err};

use super::HostState;

/// `wasmtime` a son propre type d'erreur pour les fonctions hôtes.
type HostResult<T> = wasmtime::Result<T>;

fn at(grid: i32, x: i32, y: i32, z: i32) -> GridPos {
    GridPos::new(GridId(grid as u32), IVec3::new(x, y, z))
}

/// La grille désignée par un mod. Seul le monde existe pour l'instant ; une grille
/// inconnue (ou une structure démontée) donne `None`, jamais un plantage.
fn grid(state: &HostState, id: GridId) -> Option<&RefCell<ChunkStreamer>> {
    (id == GridId::WORLD).then_some(&*state.streamer)
}

/// Lit l'état actif d'un bloc avec `f`.
fn read<R>(state: &HostState, at: GridPos, f: impl FnOnce(&BlockEntity) -> R) -> Option<R> {
    let grid = grid(state, at.grid)?.borrow();
    BlockAccess::block_entity(&*grid, at.pos).map(f)
}

/// Modifie l'état actif d'un bloc avec `f`, en le créant s'il n'existe pas.
fn write<R>(state: &HostState, at: GridPos, f: impl FnOnce(&mut BlockEntity) -> R) -> Option<R> {
    let mut grid = grid(state, at.grid)?.borrow_mut();
    grid.ensure_block_entity(at.pos).map(f)
}

fn face(f: i32) -> Option<Face> {
    Face::from_index(f)
}

fn memory(caller: &mut Caller<'_, HostState>) -> HostResult<wasmtime::Memory> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| format_err!("pas de mémoire exportée"))
}

/// Lit `len` octets à `ptr` dans la mémoire du mod.
fn read_guest(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> HostResult<Vec<u8>> {
    let memory = memory(caller)?;
    let data = memory.data(&*caller);
    let (start, len) = (ptr as u32 as usize, len.max(0) as usize);
    data.get(start..start.saturating_add(len))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format_err!("lecture hors de la mémoire du mod"))
}

fn write_guest(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8]) -> HostResult<()> {
    let memory = memory(caller)?;
    memory
        .write(&mut *caller, ptr as u32 as usize, bytes)
        .map_err(|_| format_err!("écriture hors de la mémoire du mod"))
}

fn read_string(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> HostResult<String> {
    String::from_utf8(read_guest(caller, ptr, len)?)
        .map_err(|_| format_err!("texte UTF-8 invalide"))
}

/// Un texte facultatif : une taille nulle veut dire « aucun » (« n'importe quel fluide »).
fn read_optional(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    len: i32,
) -> HostResult<Option<String>> {
    if len <= 0 {
        Ok(None)
    } else {
        read_string(caller, ptr, len).map(Some)
    }
}

/// Toutes les fonctions hôtes. Le linker refuse d'instancier un mod qui en importe une autre.
pub(super) fn define_host_functions(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    define_basics(linker)?;
    define_blocks(linker)?;
    define_capabilities(linker)?;
    define_content(linker)?;
    define_ui(linker)?;
    Ok(())
}

fn define_basics(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "log",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<()> {
            let message = read_string(&mut caller, ptr, len)?;
            log::info!("[mod {}] {message}", caller.data().mod_id);
            Ok(())
        },
    )?;
    linker.func_wrap(
        "voxel",
        "subscribe",
        |mut caller: Caller<'_, HostState>, kind: i32| {
            caller.data_mut().subscriptions.push(kind);
        },
    )?;
    linker.func_wrap(
        "voxel",
        "random",
        |mut caller: Caller<'_, HostState>| -> i32 {
            // xorshift64 : simple, rapide, et reproductible (même graine, même suite).
            let state = caller.data_mut();
            state.rng ^= state.rng << 13;
            state.rng ^= state.rng >> 7;
            state.rng ^= state.rng << 17;
            (state.rng >> 33) as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "give_item",
        |mut caller: Caller<'_, HostState>, id: i32, count: i32| {
            if id > 0 && count > 0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::GiveItem {
                        block: BlockId(id as u16),
                        count: count as u32,
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "spawn_entity",
        |mut caller: Caller<'_, HostState>,
         ptr: i32,
         len: i32,
         x: f32,
         y: f32,
         z: f32|
         -> HostResult<()> {
            let kind = read_string(&mut caller, ptr, len)?;
            caller
                .data_mut()
                .requests
                .actions
                .push(MachineAction::SpawnEntity {
                    kind,
                    position: Vec3::new(x, y, z),
                });
            Ok(())
        },
    )?;
    Ok(())
}

fn define_blocks(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "block_id",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<i32> {
            let key = read_string(&mut caller, ptr, len)?;
            Ok(caller.data().registry.id(&key).map_or(-1, |id| id.0 as i32))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "block_base",
        |caller: Caller<'_, HostState>, id: i32| -> i32 {
            if id < 0 || id as usize >= caller.data().registry.len() {
                return 0;
            }
            caller.data().registry.base(BlockId(id as u16)).0 as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "get_block",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32| -> i32 {
            let at = at(g, x, y, z);
            grid(caller.data(), at.grid)
                .map_or(0, |grid| grid.borrow().world().block(at.pos).0 as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "set_block",
        |mut caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32, id: i32| -> i32 {
            let at = at(g, x, y, z);
            let state = caller.data_mut();
            if id < 0 || id as usize >= state.registry.len() {
                return 0;
            }
            let done = grid(state, at.grid).is_some_and(|grid| {
                grid.borrow_mut()
                    .set_block_quiet(at.pos, BlockId(id as u16))
            });
            if done {
                // Le jeu préviendra les voisins après l'appel (le mod n'a pas à y penser).
                state.requests.notify.push(at);
            }
            done as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "get_power",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32| -> i32 {
            read(caller.data(), at(g, x, y, z), |e| e.power as i32).unwrap_or(0)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "received_power",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32| -> i32 {
            let at = at(g, x, y, z);
            grid(caller.data(), at.grid)
                .map_or(0, |grid| received_power(&*grid.borrow(), at.pos) as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "set_power",
        |mut caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32, power: i32| -> i32 {
            let at = at(g, x, y, z);
            let power = power.clamp(0, 15) as u8;
            let changed = write(caller.data(), at, |e| {
                std::mem::replace(&mut e.power, power) != power
            });
            if changed == Some(true) {
                caller.data_mut().requests.notify.push(at);
            }
            changed.is_some() as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "get_data",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         ptr: i32,
         cap: i32|
         -> HostResult<i32> {
            let Some(data) = read(caller.data(), at(g, x, y, z), |e| e.data.clone()) else {
                return Ok(-1);
            };
            let n = data.len().min(cap.max(0) as usize);
            write_guest(&mut caller, ptr, &data[..n])?;
            Ok(data.len() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "set_data",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         ptr: i32,
         len: i32|
         -> HostResult<i32> {
            let bytes = read_guest(&mut caller, ptr, len)?;
            Ok(write(caller.data(), at(g, x, y, z), |e| e.data = bytes).is_some() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "schedule_tick",
        |mut caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32, delay: i32| {
            caller
                .data_mut()
                .requests
                .schedules
                .push((at(g, x, y, z), delay.max(1) as u32));
        },
    )?;
    Ok(())
}

/// Les capacités : énergie, fluides, objets, rotation (voir `engine_voxel::capabilities`).
fn define_capabilities(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    // --- Déclarer les capacités d'un bloc (à sa pose) ---
    linker.func_wrap(
        "voxel",
        "add_energy",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         capacity: i64,
         rate: i64,
         sides: i32|
         -> i32 {
            write(caller.data(), at(g, x, y, z), |e| {
                let mut storage = EnergyStorage::new(capacity.max(0) as u64, rate.max(0) as u64);
                storage.sides = Sides(sides as u8 & Sides::ALL.0);
                e.caps_mut().energy = Some(storage);
            })
            .is_some() as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "add_tank",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         capacity: i32,
         sides: i32|
         -> i32 {
            write(caller.data(), at(g, x, y, z), |e| {
                let mut tank = FluidTank::new(capacity.max(0) as u32);
                tank.sides = Sides(sides as u8 & Sides::ALL.0);
                e.caps_mut().tanks.push(tank);
            })
            .is_some() as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "add_inventory",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         slots: i32,
         sides: i32|
         -> i32 {
            write(caller.data(), at(g, x, y, z), |e| {
                let mut inventory = Inventory::new(slots.clamp(0, 256) as usize);
                inventory.sides = Sides(sides as u8 & Sides::ALL.0);
                e.caps_mut().inventory = Some(inventory);
            })
            .is_some() as i32
        },
    )?;
    linker.func_wrap(
        "voxel",
        "add_kinetic",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32, sides: i32| -> i32 {
            write(caller.data(), at(g, x, y, z), |e| {
                e.caps_mut().kinetic = Some(Kinetic {
                    sides: Sides(sides as u8 & Sides::ALL.0),
                    ..Kinetic::default()
                });
            })
            .is_some() as i32
        },
    )?;

    // --- Énergie ---
    linker.func_wrap(
        "voxel",
        "energy_stored",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32| -> i64 {
            read(caller.data(), at(g, x, y, z), |e| {
                e.caps.as_ref()?.energy.as_ref().map(|s| s.amount as i64)
            })
            .flatten()
            .unwrap_or(-1)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "energy_insert",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         amount: i64,
         simulate: i32|
         -> i64 {
            write(caller.data(), at(g, x, y, z), |e| {
                let storage = e.caps.as_mut()?.energy.as_mut()?;
                Some(storage.insert(face(f), amount.max(0) as u64, simulate != 0) as i64)
            })
            .flatten()
            .unwrap_or(0)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "energy_extract",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         amount: i64,
         simulate: i32|
         -> i64 {
            write(caller.data(), at(g, x, y, z), |e| {
                let storage = e.caps.as_mut()?.energy.as_mut()?;
                Some(storage.extract(face(f), amount.max(0) as u64, simulate != 0) as i64)
            })
            .flatten()
            .unwrap_or(0)
        },
    )?;

    // --- Fluides (en millibuckets) ---
    linker.func_wrap(
        "voxel",
        "fluid_amount",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         ptr: i32,
         len: i32|
         -> HostResult<i32> {
            let fluid = read_string(&mut caller, ptr, len)?;
            Ok(read(caller.data(), at(g, x, y, z), |e| {
                e.caps.as_ref().map_or(0, |c| c.fluid_amount(&fluid) as i32)
            })
            .unwrap_or(0))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "fluid_fill",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         ptr: i32,
         len: i32,
         amount: i32,
         simulate: i32|
         -> HostResult<i32> {
            let fluid = read_string(&mut caller, ptr, len)?;
            Ok(write(caller.data(), at(g, x, y, z), |e| {
                e.caps.as_mut().map_or(0, |c| {
                    c.fill(face(f), &fluid, amount.max(0) as u32, simulate != 0) as i32
                })
            })
            .unwrap_or(0))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "fluid_drain",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         ptr: i32,
         len: i32,
         amount: i32,
         simulate: i32|
         -> HostResult<i32> {
            let fluid = read_optional(&mut caller, ptr, len)?;
            Ok(write(caller.data(), at(g, x, y, z), |e| {
                e.caps.as_mut().map_or(0, |c| {
                    c.drain(
                        face(f),
                        fluid.as_deref(),
                        amount.max(0) as u32,
                        simulate != 0,
                    ) as i32
                })
            })
            .unwrap_or(0))
        },
    )?;

    // --- Objets ---
    linker.func_wrap(
        "voxel",
        "item_count",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         ptr: i32,
         len: i32|
         -> HostResult<i32> {
            let item = read_string(&mut caller, ptr, len)?;
            Ok(read(caller.data(), at(g, x, y, z), |e| {
                e.caps
                    .as_ref()
                    .and_then(|c| c.inventory.as_ref())
                    .map_or(0, |i| i.count(&item) as i32)
            })
            .unwrap_or(0))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "item_insert",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         ptr: i32,
         len: i32,
         count: i32,
         simulate: i32|
         -> HostResult<i32> {
            let item = read_string(&mut caller, ptr, len)?;
            Ok(write(caller.data(), at(g, x, y, z), |e| {
                let inventory = e.caps.as_mut()?.inventory.as_mut()?;
                Some(inventory.insert(face(f), &item, count.max(0) as u32, simulate != 0) as i32)
            })
            .flatten()
            .unwrap_or(0))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "item_extract",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         f: i32,
         ptr: i32,
         len: i32,
         count: i32,
         simulate: i32|
         -> HostResult<i32> {
            let item = read_optional(&mut caller, ptr, len)?;
            Ok(write(caller.data(), at(g, x, y, z), |e| {
                let inventory = e.caps.as_mut()?.inventory.as_mut()?;
                inventory
                    .extract(face(f), item.as_deref(), count.max(0) as u32, simulate != 0)
                    .map(|stack| stack.count as i32)
            })
            .flatten()
            .unwrap_or(0))
        },
    )?;

    // --- Rotation ---
    linker.func_wrap(
        "voxel",
        "kinetic_speed",
        |caller: Caller<'_, HostState>, g: i32, x: i32, y: i32, z: i32| -> f32 {
            read(caller.data(), at(g, x, y, z), |e| {
                e.caps.as_ref()?.kinetic.as_ref().map(|k| k.speed)
            })
            .flatten()
            .unwrap_or(0.0)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "set_kinetic",
        |caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         speed: f32,
         capacity: f32,
         impact: f32|
         -> i32 {
            write(caller.data(), at(g, x, y, z), |e| {
                let kinetic = e.caps.as_mut()?.kinetic.as_mut()?;
                kinetic.speed = speed;
                kinetic.stress_capacity = capacity;
                kinetic.stress_impact = impact;
                Some(())
            })
            .flatten()
            .is_some() as i32
        },
    )?;
    Ok(())
}

/// Un texte de la forme `core:iron_ingot*2,example:ruby*1` : des (objet, nombre).
fn parse_stacks(state: &HostState, text: &str) -> Vec<(engine_voxel::ItemId, u32)> {
    text.split(',')
        .filter_map(|part| {
            let (key, n) = part.split_once('*').unwrap_or((part, "1"));
            Some((state.content.items.id(key.trim())?, n.trim().parse().ok()?))
        })
        .collect()
}

fn define_ui(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "ui_show",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         ptr: i32,
         len: i32|
         -> HostResult<()> {
            // Un menu trop gros est ignoré sans lire sa mémoire : le jeu le refuserait de toute façon.
            if len as usize > crate::menu::MAX_MENU_BYTES || len < 0 {
                log::warn!("[mod {}] menu trop gros, ignoré", caller.data().mod_id);
                return Ok(());
            }
            let data = read_guest(&mut caller, ptr, len)?;
            let state = caller.data_mut();
            let mod_id = state.mod_id.clone();
            state.requests.actions.push(MachineAction::ShowMenu {
                mod_id,
                at: at(g, x, y, z),
                data,
            });
            Ok(())
        },
    )?;
    linker.func_wrap("voxel", "ui_close", |mut caller: Caller<'_, HostState>| {
        let mod_id = caller.data().mod_id.clone();
        caller
            .data_mut()
            .requests
            .actions
            .push(MachineAction::CloseMenu { mod_id });
    })?;
    Ok(())
}

fn define_content(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "item_id",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<i32> {
            let key = read_string(&mut caller, ptr, len)?;
            Ok(caller
                .data()
                .content
                .items
                .id(&key)
                .map_or(-1, |id| id.0 as i32))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "give_stack",
        |mut caller: Caller<'_, HostState>, id: i32, count: i32| {
            let state = caller.data_mut();
            if id >= 0 && (id as usize) < state.content.items.len() && count > 0 {
                state.requests.actions.push(MachineAction::GiveStack {
                    item: engine_voxel::ItemId(id as u16),
                    count: count as u32,
                });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "recipe_find",
        |mut caller: Caller<'_, HostState>,
         kind_ptr: i32,
         kind_len: i32,
         in_ptr: i32,
         in_len: i32,
         out_ptr: i32,
         out_cap: i32|
         -> HostResult<i32> {
            let kind = read_string(&mut caller, kind_ptr, kind_len)?;
            let inputs = read_string(&mut caller, in_ptr, in_len)?;
            let state = caller.data();
            let available = parse_stacks(state, &inputs);
            let Some(recipe) = state.content.machine_recipes.find(&kind, &available) else {
                return Ok(-1);
            };
            let join = |stacks: Vec<String>| stacks.join(",");
            let needs = join(
                recipe
                    .inputs
                    .iter()
                    .filter_map(|(ing, n)| {
                        // L'objet réellement présent qui satisfait l'ingrédient.
                        let item = available.iter().find(|(i, _)| ing.matches(*i))?.0;
                        Some(format!("{}*{n}", state.content.items.get(item).key))
                    })
                    .collect(),
            );
            let outputs = join(
                recipe
                    .outputs
                    .iter()
                    .map(|(item, n)| format!("{}*{n}", state.content.items.get(*item).key))
                    .collect(),
            );
            let text = format!("{};{};{needs};{outputs}", recipe.ticks, recipe.energy);
            let n = text.len().min(out_cap.max(0) as usize);
            write_guest(&mut caller, out_ptr, &text.as_bytes()[..n])?;
            Ok(text.len() as i32)
        },
    )?;
    Ok(())
}
