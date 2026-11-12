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
    define_calls(linker)?;
    define_commands(linker)?;
    define_mobs(linker)?;
    define_dimensions(linker)?;
    define_player(linker)?;
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
        "slot_get",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         slot: i32,
         ptr: i32,
         cap: i32|
         -> HostResult<i32> {
            let text = read(caller.data(), at(g, x, y, z), |e| {
                let stack = e
                    .caps
                    .as_ref()?
                    .inventory
                    .as_ref()?
                    .slots
                    .get(usize::try_from(slot).ok()?)?
                    .as_ref()?;
                Some(format!("{}*{}", stack.item, stack.count))
            })
            .flatten();
            let Some(text) = text else { return Ok(-1) };
            let n = text.len().min(cap.max(0) as usize);
            write_guest(&mut caller, ptr, &text.as_bytes()[..n])?;
            Ok(text.len() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "slot_set",
        |mut caller: Caller<'_, HostState>,
         g: i32,
         x: i32,
         y: i32,
         z: i32,
         slot: i32,
         ptr: i32,
         len: i32|
         -> HostResult<i32> {
            let text = read_string(&mut caller, ptr, len)?;
            let state = caller.data();
            let new = if text.trim().is_empty() {
                None
            } else {
                // Un objet inconnu est refusé : la case ne doit pas contenir de fantôme.
                match parse_stacks(state, &text).as_slice() {
                    [(item, count)] if *count > 0 => Some(engine_voxel::ItemStack {
                        item: state.content.items.get(*item).key.clone(),
                        count: (*count).min(engine_voxel::MAX_STACK),
                        damage: 0,
                    }),
                    _ => return Ok(0),
                }
            };
            let Ok(index) = usize::try_from(slot) else {
                return Ok(0);
            };
            let done = write(state, at(g, x, y, z), |e| {
                let target = e.caps.as_mut()?.inventory.as_mut()?.slots.get_mut(index)?;
                *target = new;
                Some(())
            })
            .flatten();
            Ok(done.is_some() as i32)
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
    linker.func_wrap(
        "voxel",
        "options_show",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<()> {
            if len as usize > crate::menu::MAX_MENU_BYTES || len < 0 {
                log::warn!(
                    "[mod {}] page d'options trop grosse, ignorée",
                    caller.data().mod_id
                );
                return Ok(());
            }
            let data = read_guest(&mut caller, ptr, len)?;
            let state = caller.data_mut();
            let mod_id = state.mod_id.clone();
            state
                .requests
                .actions
                .push(MachineAction::ShowOptions { mod_id, data });
            Ok(())
        },
    )?;
    linker.func_wrap(
        "voxel",
        "setting_get",
        |mut caller: Caller<'_, HostState>,
         key_ptr: i32,
         key_len: i32,
         out_ptr: i32,
         cap: i32|
         -> HostResult<i32> {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            let state = caller.data();
            let value = state
                .settings
                .borrow()
                .get(&state.mod_id, &key)
                .map(str::to_owned);
            let Some(value) = value else { return Ok(-1) };
            let n = value.len().min(cap.max(0) as usize);
            write_guest(&mut caller, out_ptr, &value.as_bytes()[..n])?;
            Ok(value.len() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "setting_set",
        |mut caller: Caller<'_, HostState>,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> HostResult<i32> {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            let value = read_string(&mut caller, val_ptr, val_len)?;
            let state = caller.data();
            let done = state.settings.borrow_mut().set(&state.mod_id, &key, &value);
            Ok(done as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "world_get",
        |mut caller: Caller<'_, HostState>,
         key_ptr: i32,
         key_len: i32,
         out_ptr: i32,
         cap: i32|
         -> HostResult<i32> {
            let key = read_string(&mut caller, key_ptr, key_len)?;
            let state = caller.data();
            let value = state
                .world_data
                .borrow()
                .get(&state.mod_id, &key)
                .map(str::to_owned);
            let Some(value) = value else { return Ok(-1) };
            let n = value.len().min(cap.max(0) as usize);
            write_guest(&mut caller, out_ptr, &value.as_bytes()[..n])?;
            Ok(value.len() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "world_set",
        |mut caller: Caller<'_, HostState>,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> HostResult<i32> {
            // Une valeur trop grande est refusée sans lire la mémoire du mod.
            if val_len < 0 || val_len as usize > crate::settings::MAX_WORLD_VALUE {
                return Ok(0);
            }
            let key = read_string(&mut caller, key_ptr, key_len)?;
            let value = read_string(&mut caller, val_ptr, val_len)?;
            let state = caller.data();
            let done = state
                .world_data
                .borrow_mut()
                .set(&state.mod_id, &key, &value);
            Ok(done as i32)
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

/// Les appels entre mods. Un script en cours d'appel est emprunté en écriture : l'appeler de
/// nouveau (un cycle A → B → A, ou un mod qui s'appelle lui-même) échoue au lieu de se
/// bloquer, et la profondeur est donc bornée par le nombre de mods.
/// Les commandes `/nom` : déclarées dans `init`, validées ici (le jeu ne voit que des noms sains).
/// Les créatures à comportement : le type par son nom, et les ordres (rangés pour le jeu, qui les
/// applique aux créatures du lot en cours).
fn define_mobs(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "mob_kind",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<i32> {
            let key = read_string(&mut caller, ptr, len)?;
            Ok(caller
                .data()
                .content
                .mob_keys
                .iter()
                .position(|k| *k == key)
                .map_or(-1, |i| i as i32))
        },
    )?;
    linker.func_wrap(
        "voxel",
        "mob_move",
        |mut caller: Caller<'_, HostState>, mob: i32, dx: f32, dz: f32, speed: f32| {
            if dx.is_finite() && dz.is_finite() && speed.is_finite() {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::MobMove {
                        mob: mob as u32,
                        dx,
                        dz,
                        speed,
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "mob_jump",
        |mut caller: Caller<'_, HostState>, mob: i32| {
            caller
                .data_mut()
                .requests
                .actions
                .push(MachineAction::MobJump { mob: mob as u32 });
        },
    )?;
    linker.func_wrap(
        "voxel",
        "mob_attack",
        |mut caller: Caller<'_, HostState>, mob: i32, damage: f32| {
            if damage.is_finite() && damage > 0.0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::MobAttack {
                        mob: mob as u32,
                        damage,
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "mob_state",
        |mut caller: Caller<'_, HostState>, mob: i32, value: i32| {
            caller
                .data_mut()
                .requests
                .actions
                .push(MachineAction::MobState {
                    mob: mob as u32,
                    value: value as u32,
                });
        },
    )?;
    Ok(())
}

/// Les dimensions : savoir où l'on est, et y emmener le joueur.
/// Le joueur : ce qu'on en sait (un instantané), et ce qu'un script peut lui faire.
fn define_player(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "player_info",
        |mut caller: Caller<'_, HostState>, out_ptr: i32, cap: i32| -> HostResult<i32> {
            let floats = caller.data().player.get().to_floats();
            let n = floats.len().min(cap.max(0) as usize / 4);
            let bytes: Vec<u8> = floats[..n].iter().flat_map(|f| f.to_le_bytes()).collect();
            write_guest(&mut caller, out_ptr, &bytes)?;
            Ok(n as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "player_target",
        |mut caller: Caller<'_, HostState>, max_dist: f32, out_ptr: i32| -> HostResult<i32> {
            let player = caller.data().player.get();
            let max = max_dist.clamp(0.0, 128.0);
            let hit = {
                let streamer = caller.data().streamer.borrow();
                engine_voxel::raycast(streamer.world(), player.eye, player.look, max)
            };
            let Some(hit) = hit else { return Ok(0) };
            let point = player.eye + player.look.normalize_or_zero() * hit.distance;
            let bytes: Vec<u8> = point
                .to_array()
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect();
            write_guest(&mut caller, out_ptr, &bytes)?;
            Ok(1)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "player_heal",
        |mut caller: Caller<'_, HostState>, amount: f32| {
            if amount.is_finite() && amount > 0.0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::PlayerHeal {
                        amount: amount.min(100.0),
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "player_teleport",
        |mut caller: Caller<'_, HostState>, x: f32, y: f32, z: f32| {
            if x.is_finite() && y.is_finite() && z.is_finite() {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::PlayerTeleport {
                        position: Vec3::new(x, y, z),
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "player_effect",
        |mut caller: Caller<'_, HostState>,
         ptr: i32,
         len: i32,
         seconds: f32,
         power: f32|
         -> HostResult<()> {
            if len > 32 || !seconds.is_finite() || !power.is_finite() {
                return Ok(());
            }
            let name = read_string(&mut caller, ptr, len)?;
            if crate::player::EFFECTS.contains(&name.as_str()) {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::PlayerEffect {
                        name,
                        seconds: seconds.clamp(0.0, 600.0),
                        power: power.clamp(0.0, 10.0),
                    });
            }
            Ok(())
        },
    )?;
    linker.func_wrap(
        "voxel",
        "hurt_area",
        |mut caller: Caller<'_, HostState>, x: f32, y: f32, z: f32, radius: f32, damage: f32| {
            if [x, y, z, radius, damage].iter().all(|v| v.is_finite()) && damage > 0.0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::HurtArea {
                        center: Vec3::new(x, y, z),
                        radius: radius.clamp(0.0, 16.0),
                        damage: damage.min(200.0),
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "hurt_player",
        |mut caller: Caller<'_, HostState>, amount: f32| {
            if amount.is_finite() && amount > 0.0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::PlayerHurt {
                        amount: amount.min(100.0),
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "say",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<()> {
            if len > 256 {
                return Ok(());
            }
            let text = read_string(&mut caller, ptr, len)?;
            caller
                .data_mut()
                .requests
                .actions
                .push(MachineAction::Say { text });
            Ok(())
        },
    )?;
    linker.func_wrap(
        "voxel",
        "consume_held",
        |mut caller: Caller<'_, HostState>, count: i32| {
            if count > 0 {
                caller
                    .data_mut()
                    .requests
                    .actions
                    .push(MachineAction::ConsumeHeld {
                        count: (count as u32).min(64),
                    });
            }
        },
    )?;
    linker.func_wrap(
        "voxel",
        "hud_bar",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         fraction: f32,
         r: f32,
         g: f32,
         b: f32,
         ptr: i32,
         len: i32|
         -> HostResult<()> {
            if len > 64 || ![fraction, r, g, b].iter().all(|v| v.is_finite()) {
                return Ok(());
            }
            let label = read_string(&mut caller, ptr, len)?;
            let state = caller.data_mut();
            let mod_id = state.mod_id.clone();
            state.requests.actions.push(MachineAction::HudBar {
                mod_id,
                id: id as u32,
                fraction: fraction.clamp(0.0, 1.0),
                color: [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)],
                label,
            });
            Ok(())
        },
    )?;
    linker.func_wrap(
        "voxel",
        "hud_clear",
        |mut caller: Caller<'_, HostState>, id: i32| {
            let state = caller.data_mut();
            let mod_id = state.mod_id.clone();
            state.requests.actions.push(MachineAction::HudClear {
                mod_id,
                id: id as u32,
            });
        },
    )?;
    Ok(())
}

fn define_dimensions(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "travel_to",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<i32> {
            if len > 64 {
                return Ok(0);
            }
            let dimension = read_string(&mut caller, ptr, len)?;
            // Le jeu vérifie que la dimension existe et que le monde est sauvegardé.
            caller
                .data_mut()
                .requests
                .actions
                .push(MachineAction::Travel { dimension });
            Ok(1)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "dimension",
        |mut caller: Caller<'_, HostState>, out_ptr: i32, cap: i32| -> HostResult<i32> {
            let key = caller.data().content.dimension.to_string();
            let n = key.len().min(cap.max(0) as usize);
            write_guest(&mut caller, out_ptr, &key.as_bytes()[..n])?;
            Ok(key.len() as i32)
        },
    )?;
    Ok(())
}

fn define_commands(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "register_command",
        |mut caller: Caller<'_, HostState>,
         name_ptr: i32,
         name_len: i32,
         help_ptr: i32,
         help_len: i32|
         -> HostResult<()> {
            if name_len > 32 || help_len > 200 {
                return Ok(());
            }
            let name = read_string(&mut caller, name_ptr, name_len)?;
            let help = read_string(&mut caller, help_ptr, help_len)?;
            let valid = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            let state = caller.data_mut();
            if !valid
                || state.commands.len() >= 32
                || state.commands.iter().any(|(n, _)| *n == name)
            {
                log::warn!("[mod {}] commande « {name} » refusée", state.mod_id);
                return Ok(());
            }
            state.commands.push((name, help));
            Ok(())
        },
    )?;
    Ok(())
}

fn define_calls(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "voxel",
        "call_mod",
        |mut caller: Caller<'_, HostState>,
         mod_ptr: i32,
         mod_len: i32,
         name_ptr: i32,
         name_len: i32,
         arg_ptr: i32,
         arg_len: i32,
         out_ptr: i32,
         cap: i32|
         -> HostResult<i32> {
            if arg_len < 0 || arg_len as usize > super::MAX_CALL_BYTES || name_len > 64 {
                return Ok(-1);
            }
            let target = read_string(&mut caller, mod_ptr, mod_len)?;
            let name = read_string(&mut caller, name_ptr, name_len)?;
            let args = read_guest(&mut caller, arg_ptr, arg_len)?;
            let me = caller.data().mod_id.clone();
            if target == me {
                return Ok(-1);
            }
            let peer = caller
                .data()
                .peers
                .borrow()
                .iter()
                .find(|(id, _)| *id == target)
                .map(|(_, script)| script.clone());
            let Some(peer) = peer else { return Ok(-1) };
            // Occupé (cycle) : refusé, sans attendre.
            let Ok(mut script) = peer.try_borrow_mut() else {
                log::warn!("[mod {me}] appel de « {target} » refusé : déjà en cours (cycle ?)");
                return Ok(-1);
            };
            let result = script.call(&name, &args);
            drop(script);
            let (reply, requests) = match result {
                Ok(done) => done,
                Err(_) => return Ok(-1),
            };
            // Ce que le mod appelé a demandé au jeu est fait comme si l'appelant l'avait demandé.
            caller.data_mut().requests.append(requests);
            let Some(reply) = reply else { return Ok(-1) };
            let n = reply.len().min(cap.max(0) as usize);
            write_guest(&mut caller, out_ptr, &reply[..n])?;
            Ok(reply.len() as i32)
        },
    )?;
    linker.func_wrap(
        "voxel",
        "mod_loaded",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> HostResult<i32> {
            let id = read_string(&mut caller, ptr, len)?;
            let state = caller.data();
            let found = state
                .peers
                .borrow()
                .iter()
                .find(|(m, _)| *m == id)
                .map(|(_, s)| s.clone());
            // Un script occupé (en cours d'appel) est chargé, et actif tant qu'il n'a pas planté.
            Ok(
                found.is_some_and(|s| s.try_borrow().map_or(true, |s| s.disabled().is_none()))
                    as i32,
            )
        },
    )?;
    Ok(())
}
