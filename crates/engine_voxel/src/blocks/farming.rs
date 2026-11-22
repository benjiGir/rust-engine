//! L'agriculture : la terre labourée et son irrigation.
//!
//! - **Labourer** : la houe change l'herbe ou la terre en terre labourée, si rien n'est
//!   posé dessus.
//! - **Irriguer** : de l'eau à 4 blocs au plus (à l'horizontale, au même niveau ou un
//!   au-dessus) garde la terre labourée humide. Sans eau, elle sèche ; sèche et sans
//!   rien de planté, elle redevient de la terre.
//! - Un bloc opaque posé dessus l'étouffe : elle redevient de la terre.
//!
//! La terre sèche et la terre humide sont deux blocs (`core:farmland`,
//! `core:farmland_wet`), comme la lampe éteinte et la lampe allumée : un mod ajoute sa
//! propre terre en lui donnant le comportement `core:farmland`.

use glam::IVec3;

use crate::block::BlockId;
use crate::grid::GridPos;
use crate::registry::BlockRegistry;
use crate::tech::machines::{BlockBehavior, MachineContext};
use crate::world::World;

/// La portée de l'irrigation, à l'horizontale.
pub const IRRIGATION_REACH: i32 = 4;

/// Les blocs de l'agriculture, résolus une fois.
#[derive(Debug, Clone, Copy)]
pub struct FarmBlocks {
    pub grass: BlockId,
    pub dirt: BlockId,
    pub dry: BlockId,
    pub wet: BlockId,
}

impl FarmBlocks {
    pub fn resolve(registry: &BlockRegistry) -> Option<Self> {
        Some(Self {
            grass: registry.id("core:grass")?,
            dirt: registry.id("core:dirt")?,
            dry: registry.id("core:farmland")?,
            wet: registry.id("core:farmland_wet")?,
        })
    }
}

/// Un bloc de terre labourée (sèche ou humide, ou celle d'un mod) ?
pub fn is_farmland(registry: &BlockRegistry, block: BlockId) -> bool {
    registry.def(block).behavior.as_deref() == Some("core:farmland")
}

/// De l'eau assez près de `pos` pour l'irriguer ?
pub fn is_irrigated(world: &World, registry: &BlockRegistry, pos: IVec3) -> bool {
    let r = IRRIGATION_REACH;
    (0..=1).any(|dy| {
        (-r..=r).any(|dx| {
            (-r..=r).any(|dz| {
                let block = world.block(pos + IVec3::new(dx, dy, dz));
                registry.def(block).fluid.as_deref() == Some("core:water")
            })
        })
    })
}

/// Ce que devient le bloc en `pos` sous la houe : de la terre labourée (déjà humide
/// s'il y a de l'eau près), ou `None` si on ne peut pas labourer ici.
pub fn tilled(world: &World, registry: &BlockRegistry, pos: IVec3) -> Option<BlockId> {
    let blocks = FarmBlocks::resolve(registry)?;
    let block = world.block(pos);
    if block != blocks.grass && block != blocks.dirt {
        return None;
    }
    // Rien dessus : ni bloc, ni plante.
    if !world.block(pos + IVec3::Y).is_air() {
        return None;
    }
    Some(if is_irrigated(world, registry, pos) {
        blocks.wet
    } else {
        blocks.dry
    })
}

pub struct Farmland {
    blocks: FarmBlocks,
}

impl Farmland {
    pub fn new(registry: &BlockRegistry) -> Option<Self> {
        Some(Self {
            blocks: FarmBlocks::resolve(registry)?,
        })
    }
}

impl BlockBehavior for Farmland {
    fn random_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let Some(registry) = ctx.registry() else {
            return;
        };
        for &at in positions {
            let block = ctx.block(at);
            let above = ctx.block(at.offset(IVec3::Y));
            let above_def = registry.def(above);
            // Étouffée sous un bloc opaque.
            if !above.is_air() && !above_def.transparent {
                ctx.set_block(at, self.blocks.dirt);
                continue;
            }
            let irrigated = is_irrigated(ctx.grids.world().borrow().world(), &registry, at.pos);
            if irrigated {
                if block != self.blocks.wet {
                    ctx.set_block(at, self.blocks.wet);
                }
            } else if block == self.blocks.wet {
                ctx.set_block(at, self.blocks.dry);
            } else if above.is_air() && ctx.chance(3) {
                // Sèche et sans rien de planté : la terre reprend ses droits.
                ctx.set_block(at, self.blocks.dirt);
            }
        }
    }
}

/// La lumière minimale pour qu'une culture pousse.
const CROP_LIGHT: u8 = 9;

/// Le stade suivant d'une culture : `core:wheat_1` → `core:wheat_2`. Les stades sont des
/// blocs numérotés (`_0` la graine, le plus grand est mûr) ; `None` pour le dernier.
pub fn next_stage(registry: &BlockRegistry, block: BlockId) -> Option<BlockId> {
    let def = registry.def(block);
    if def.behavior.as_deref() != Some("core:crop") {
        return None;
    }
    let (stem, stage) = def.key.rsplit_once('_')?;
    let stage: u32 = stage.parse().ok()?;
    registry.id(&format!("{stem}_{}", stage + 1))
}

/// Une culture mûre : c'est un bloc de culture sans stade suivant.
pub fn is_ripe(registry: &BlockRegistry, block: BlockId) -> bool {
    registry.def(block).behavior.as_deref() == Some("core:crop")
        && next_stage(registry, block).is_none()
}

/// L'engrais : fait avancer la culture de deux stades au plus. `None` si ce n'est pas une
/// culture, ou si elle est déjà mûre.
pub fn fertilized(registry: &BlockRegistry, block: BlockId) -> Option<BlockId> {
    let first = next_stage(registry, block)?;
    Some(next_stage(registry, first).unwrap_or(first))
}

/// Les cultures : elles grandissent d'un stade à chaque tirage réussi, à la lumière et
/// sur de la terre labourée ; deux fois plus vite sur la terre humide.
pub struct Crop;

impl BlockBehavior for Crop {
    fn random_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let Some(registry) = ctx.registry() else {
            return;
        };
        for &at in positions {
            let block = ctx.block(at);
            let Some(next) = next_stage(&registry, block) else {
                continue;
            };
            let (light, wet) = {
                let streamer = ctx.grids.world().borrow();
                let world = streamer.world();
                let (sky, lamp) = world.light(at.pos);
                let below = world.block(at.pos - IVec3::Y);
                (
                    sky.max(lamp),
                    registry.def(below).key == "core:farmland_wet",
                )
            };
            if light >= CROP_LIGHT && ctx.chance(if wet { 3 } else { 6 }) {
                ctx.set_block(at, next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use glam::Vec3;

    use super::*;
    use crate::streaming::{ChunkStreamer, StreamingConfig, tests::pump_until_idle};
    use crate::tech::machines::Machines;
    use crate::worldgen::generator::TerrainGenerator;

    fn world() -> RefCell<ChunkStreamer> {
        let config = StreamingConfig {
            view_radius: 2,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(TerrainGenerator::new(7), config);
        pump_until_idle(&mut s, Vec3::new(5.0, 100.0, 5.0));
        RefCell::new(s)
    }

    fn id(s: &RefCell<ChunkStreamer>, key: &str) -> BlockId {
        s.borrow().world().registry().id(key).unwrap()
    }

    /// Une dalle de terre 11 × 11 dans le ciel, loin de toute eau.
    fn dirt_slab(s: &RefCell<ChunkStreamer>) -> IVec3 {
        let center = IVec3::new(8, 120, 8);
        let dirt = id(s, "core:dirt");
        for dx in -5..=5 {
            for dz in -5..=5 {
                assert!(
                    s.borrow_mut()
                        .set_block(center + IVec3::new(dx, 0, dz), dirt)
                );
            }
        }
        center
    }

    fn tick_until(
        s: &RefCell<ChunkStreamer>,
        machines: &mut Machines,
        done: impl Fn(&ChunkStreamer) -> bool,
    ) -> bool {
        for _ in 0..400 {
            machines.run_random_ticks(s, IVec3::ZERO, 1, 4096, &mut Vec::new());
            if done(&s.borrow()) {
                return true;
            }
        }
        false
    }

    #[test]
    fn tilling_needs_dirt_or_grass_with_nothing_above() {
        let s = world();
        let center = dirt_slab(&s);
        let registry = s.borrow().world().registry().clone();
        let tilled_at = |p| tilled(s.borrow().world(), &registry, p);
        assert_eq!(tilled_at(center), Some(id(&s, "core:farmland")));
        let stone = id(&s, "core:stone");
        s.borrow_mut().set_block(center + IVec3::Y, stone);
        assert_eq!(tilled_at(center), None, "un bloc dessus");
        s.borrow_mut().set_block(center + IVec3::X, stone);
        assert_eq!(tilled_at(center + IVec3::X), None, "de la pierre");
    }

    #[test]
    fn water_nearby_tills_wet_and_keeps_farmland_wet() {
        let s = world();
        let center = dirt_slab(&s);
        let water = id(&s, "core:water");
        let (dry, wet) = (id(&s, "core:farmland"), id(&s, "core:farmland_wet"));
        // De l'eau à 4 blocs : à portée.
        s.borrow_mut().set_block(center + IVec3::X * 4, water);
        let registry = s.borrow().world().registry().clone();
        assert_eq!(tilled(s.borrow().world(), &registry, center), Some(wet));
        // Une terre sèche posée à portée s'humidifie.
        let near = center + IVec3::X * 2;
        s.borrow_mut().set_block(near, dry);
        let mut machines = Machines::with_natives(&registry);
        assert!(tick_until(&s, &mut machines, |s| s.world().block(near) == wet));
    }

    #[test]
    fn farmland_far_from_water_dries_then_returns_to_dirt() {
        let s = world();
        let center = dirt_slab(&s);
        let (dirt, wet) = (id(&s, "core:dirt"), id(&s, "core:farmland_wet"));
        s.borrow_mut().set_block(center, wet);
        let registry = s.borrow().world().registry().clone();
        assert!(!is_irrigated(s.borrow().world(), &registry, center));
        let mut machines = Machines::with_natives(&registry);
        assert!(tick_until(&s, &mut machines, |s| s.world().block(center) == dirt));
    }

    #[test]
    fn an_opaque_block_smothers_farmland() {
        let s = world();
        let center = dirt_slab(&s);
        let (dirt, dry) = (id(&s, "core:dirt"), id(&s, "core:farmland"));
        let stone = id(&s, "core:stone");
        s.borrow_mut().set_block(center, dry);
        s.borrow_mut().set_block(center + IVec3::Y, stone);
        let registry = s.borrow().world().registry().clone();
        let mut machines = Machines::with_natives(&registry);
        assert!(tick_until(&s, &mut machines, |s| s.world().block(center) == dirt));
    }

    #[test]
    fn a_crop_ripens_stage_by_stage_on_wet_farmland() {
        let s = world();
        let center = dirt_slab(&s);
        let (wet, seeds, ripe) = (
            id(&s, "core:farmland_wet"),
            id(&s, "core:wheat_0"),
            id(&s, "core:wheat_3"),
        );
        s.borrow_mut().set_block(center, wet);
        // De l'eau juste à côté : la terre reste humide pendant la pousse.
        let water = id(&s, "core:water");
        s.borrow_mut().set_block(center + IVec3::X, water);
        let plant = center + IVec3::Y;
        s.borrow_mut().set_block(plant, seeds);
        let registry = s.borrow().world().registry().clone();
        let mut machines = Machines::with_natives(&registry);
        assert!(tick_until(&s, &mut machines, |s| s.world().block(plant) == ripe));
        assert!(is_ripe(&registry, ripe));
        assert!(!is_ripe(&registry, seeds));
    }

    #[test]
    fn fertilizer_skips_stages_but_not_past_ripe() {
        let s = world();
        let registry = s.borrow().world().registry().clone();
        let ids: Vec<_> = (0..4).map(|i| id(&s, &format!("core:wheat_{i}"))).collect();
        assert_eq!(fertilized(&registry, ids[0]), Some(ids[2]));
        assert_eq!(fertilized(&registry, ids[2]), Some(ids[3]));
        assert_eq!(fertilized(&registry, ids[3]), None);
    }

    #[test]
    fn every_crop_has_four_stages_ending_ripe() {
        let s = world();
        let registry = s.borrow().world().registry().clone();
        for crop in ["wheat", "carrot", "potato", "cabbage", "melon"] {
            let first = id(&s, &format!("core:{crop}_0"));
            let mut block = first;
            let mut stages = 1;
            while let Some(next) = next_stage(&registry, block) {
                block = next;
                stages += 1;
            }
            assert_eq!(stages, 4, "{crop}");
            assert!(is_ripe(&registry, block), "{crop}");
        }
    }
}
