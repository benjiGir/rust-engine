//! Les machines de base qui se branchent sur les réseaux (voir `tech/networks.rs`) : de quoi
//! produire, stocker et consommer de l'énergie, des fluides et de la rotation. Les mods
//! techniques écrivent les leurs avec les mêmes capacités ; celles-ci servent d'exemple et de
//! base au jeu.
//!
//! | Machine | Rôle | Ce qu'elle fait |
//! |---|---|---|
//! | Panneau solaire | énergie, `Out` | 4 par tick à ciel ouvert |
//! | Batterie | énergie, `Both` | stocke 100 000 |
//! | Lampe électrique | énergie, `In` | s'allume en consommant 4 par tick |
//! | Pompe | fluide, `Out` | tire de l'eau du bloc d'eau dessous |
//! | Réservoir | fluide, `Both` | stocke 16 000 mB |
//! | Remplisseur | fluide, `In` | verse un bloc d'eau au-dessus quand il a 1 000 mB |
//! | Moteur | énergie `In` + rotation | 8 d'énergie par tick pour 64 tr/min, 512 de contrainte |
//! | Ventilateur | rotation | consomme 128 de contrainte ; tourne quand il reçoit de la vitesse |

pub(crate) mod capabilities;
pub(crate) mod circuits;
pub(crate) mod conduit_shell;
pub(crate) mod containers;
pub(crate) mod crafting;
pub(crate) mod machines;
pub(crate) mod network_visual;
pub(crate) mod networks;
pub(crate) mod recipe_index;
pub(crate) mod storage;

use std::rc::Rc;

use crate::block::BlockId;
use crate::grid::{GridId, GridPos};
use crate::registry::BlockRegistry;
use crate::tech::capabilities::{Capabilities, EnergyStorage, FluidTank, Io, Kinetic, Sides};
use crate::tech::circuits::restyle;
use crate::tech::machines::{BlockBehavior, MachineContext};

const SOLAR_PER_SECOND: u64 = 80;
const LAMP_COST: u64 = 4;
const PUMP_AMOUNT: u32 = 500;
const MOTOR_COST: u64 = 8;
pub const MOTOR_SPEED: f32 = 64.0;
pub const MOTOR_CAPACITY: f32 = 512.0;
pub const FAN_IMPACT: f32 = 128.0;
pub const PROPELLER_IMPACT: f32 = 96.0;

/// Crée les capacités d'un bloc s'il n'en a pas encore (un rechargement garde son contenu).
fn ensure_caps(ctx: &mut MachineContext, at: GridPos, make: impl FnOnce(&mut Capabilities)) {
    ctx.grids.write_or_create(at, |e| {
        if e.caps.is_none() {
            make(e.caps_mut());
        }
    });
}

/// Prend exactement `n` d'énergie, ou rien (on ne perd pas un fond de stock).
fn take(store: &mut EnergyStorage, n: u64) -> bool {
    store.extract(None, n, true) == n && store.extract(None, n, false) == n
}

fn energy(capacity: u64, rate: u64, io: Io) -> EnergyStorage {
    let mut storage = EnergyStorage::new(capacity, rate);
    storage.sides = Sides::ALL.with_io(io);
    storage
}

fn tank(capacity: u32, io: Io) -> FluidTank {
    let mut tank = FluidTank::new(capacity);
    tank.sides = Sides::ALL.with_io(io);
    tank
}

// ----- Énergie -----

struct Solar;

impl BlockBehavior for Solar {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| c.energy = Some(energy(2_000, 400, Io::Out)));
            ctx.schedule(at, 20);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            // Le ciel doit être dégagé : dans le monde, la lumière du ciel au-dessus est au
            // maximum ; sur une structure mobile (qui n'a pas de lumière à elle), rien de plein
            // au-dessus dans la structure.
            let open = if at.grid == GridId::WORLD {
                ctx.grids
                    .world()
                    .borrow()
                    .world()
                    .light(at.pos + glam::IVec3::Y)
                    .0
                    >= 15
            } else {
                let registry = ctx.registry();
                (1..=16).all(|d| {
                    let b = ctx.block(at.offset(glam::IVec3::Y * d));
                    b.is_air() || registry.as_ref().is_some_and(|r| !r.is_solid(b))
                })
            };
            if open {
                ctx.grids.write(at, |e| {
                    if let Some(store) = e.caps.as_mut().and_then(|c| c.energy.as_mut()) {
                        store.insert(None, SOLAR_PER_SECOND, false);
                    }
                });
            }
            ctx.schedule(at, 20);
        }
    }
}

struct Battery;

impl BlockBehavior for Battery {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| {
                c.energy = Some(energy(100_000, 2_000, Io::Both))
            });
        }
    }
}

/// Une lampe qui s'allume tant qu'elle reçoit de l'énergie.
struct PowerLamp {
    off: BlockId,
    on: BlockId,
}

impl BlockBehavior for PowerLamp {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| c.energy = Some(energy(200, 50, Io::In)));
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let lit = ctx
                .grids
                .write(at, |e| {
                    e.caps
                        .as_mut()
                        .and_then(|c| c.energy.as_mut())
                        .is_some_and(|s| take(s, LAMP_COST))
                })
                .unwrap_or(false);
            let wanted = if lit { self.on } else { self.off };
            if ctx.block(at) != wanted {
                restyle(ctx, at, wanted, |_| {});
            }
            // Allumée, elle consomme à chaque tick ; éteinte, le réseau la réveillera.
            if lit {
                ctx.schedule(at, 1);
            }
        }
    }
}

// ----- Fluides -----

struct Pump {
    water_blocks: Vec<BlockId>,
}

impl BlockBehavior for Pump {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| c.tanks.push(tank(4_000, Io::Out)));
            ctx.schedule(at, 10);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let below = ctx.block(at.offset(-glam::IVec3::Y));
            if self.water_blocks.contains(&below) {
                ctx.grids.write(at, |e| {
                    if let Some(caps) = e.caps.as_mut() {
                        caps.fill(None, "core:water", PUMP_AMOUNT, false);
                    }
                });
            }
            ctx.schedule(at, 10);
        }
    }
}

struct Tank;

impl BlockBehavior for Tank {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| c.tanks.push(tank(16_000, Io::Both)));
        }
    }
}

/// Verse un bloc d'eau au-dessus de lui chaque fois qu'il a 1 000 mB.
struct Filler {
    water: BlockId,
}

impl BlockBehavior for Filler {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| c.tanks.push(tank(2_000, Io::In)));
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let above = at.offset(glam::IVec3::Y);
            if !ctx.block(above).is_air() {
                continue;
            }
            let poured = ctx
                .grids
                .write(at, |e| {
                    e.caps.as_mut().is_some_and(|c| {
                        // Seulement s'il y a un bloc entier : on ne perd pas un fond d'eau.
                        c.drain(None, Some("core:water"), 1_000, true) == 1_000
                            && c.drain(None, Some("core:water"), 1_000, false) == 1_000
                    })
                })
                .unwrap_or(false);
            if poured {
                ctx.set_block(above, self.water);
            }
        }
    }
}

// ----- Rotation -----

/// Un moteur électrique : de l'énergie en entrée, de la rotation en sortie.
struct Motor;

impl BlockBehavior for Motor {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| {
                c.energy = Some(energy(400, 100, Io::In));
                c.kinetic = Some(Kinetic {
                    speed: 0.0,
                    stress_capacity: MOTOR_CAPACITY,
                    stress_impact: 0.0,
                    sides: Sides::ALL,
                });
            });
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let running = ctx
                .grids
                .write(at, |e| {
                    let caps = e.caps.as_mut()?;
                    let powered = caps.energy.as_mut().is_some_and(|s| take(s, MOTOR_COST));
                    caps.kinetic.as_mut()?.speed = if powered { MOTOR_SPEED } else { 0.0 };
                    Some(powered)
                })
                .flatten()
                .unwrap_or(false);
            if running {
                ctx.schedule(at, 1);
            }
        }
    }
}

/// Un ventilateur : il tourne (et change d'aspect) quand le réseau lui donne de la vitesse.
struct Fan {
    idle: BlockId,
    spinning: BlockId,
}

impl BlockBehavior for Fan {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| {
                c.kinetic = Some(Kinetic {
                    speed: 0.0,
                    stress_capacity: 0.0,
                    stress_impact: FAN_IMPACT,
                    sides: Sides::ALL,
                });
            });
            ctx.schedule(at, 1);
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let speed = ctx
                .grids
                .read(at, |e| e.caps.as_ref()?.kinetic.as_ref().map(|k| k.speed))
                .flatten()
                .unwrap_or(0.0);
            let wanted = if speed != 0.0 {
                self.spinning
            } else {
                self.idle
            };
            if ctx.block(at) != wanted {
                restyle(ctx, at, wanted, |_| {});
            }
        }
    }
}

/// Une hélice : elle consomme de la rotation (un arbre la relie à un moteur) et pousse dans
/// le sens où elle regarde, d'autant plus que sa vitesse est grande. La poussée est appliquée par
/// `motion/ship.rs` à la structure qui la porte ; ici, elle ne fait que recevoir de la rotation.
struct Propeller;

impl BlockBehavior for Propeller {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ensure_caps(ctx, at, |c| {
                c.kinetic = Some(Kinetic {
                    speed: 0.0,
                    stress_capacity: 0.0,
                    stress_impact: PROPELLER_IMPACT,
                    sides: Sides::ALL,
                });
            });
        }
    }
}

/// Tous les comportements de machines de base, par nom (`core:solar_panel`...). Un
/// comportement dont un bloc manque au registre est simplement omis.
pub fn natives(registry: &BlockRegistry) -> Vec<(&'static str, Rc<dyn BlockBehavior>)> {
    let mut out: Vec<(&'static str, Rc<dyn BlockBehavior>)> = Vec::new();
    let id = |key: &str| registry.id(key);
    out.push(("core:solar_panel", Rc::new(Solar)));
    out.push(("core:battery", Rc::new(Battery)));
    out.push(("core:tank", Rc::new(Tank)));
    out.push(("core:motor", Rc::new(Motor)));
    out.push(("core:propeller", Rc::new(Propeller)));
    if let (Some(off), Some(on)) = (id("core:power_lamp"), id("core:power_lamp_on")) {
        out.push(("core:power_lamp", Rc::new(PowerLamp { off, on })));
    }
    if let (Some(idle), Some(spinning)) = (id("core:fan"), id("core:fan_on")) {
        out.push(("core:fan", Rc::new(Fan { idle, spinning })));
    }
    // Les blocs qui portent de l'eau : le fluide « core:water » dans `fluid`.
    let water_blocks: Vec<BlockId> = registry
        .iter()
        .filter(|(_, def)| def.fluid.as_deref() == Some("core:water"))
        .map(|(id, _)| id)
        .collect();
    out.push(("core:pump", Rc::new(Pump { water_blocks })));
    if let Some(water) = id("core:water") {
        out.push(("core:filler", Rc::new(Filler { water })));
    }
    out
}
