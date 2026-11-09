//! Les comportements natifs de la végétation, réveillés par les ticks aléatoires.
//!
//! - **L'herbe** s'étend sur la terre voisine bien éclairée, meurt (redevient de la
//!   terre) sous un bloc opaque, et se couvre de neige dans les biomes froids.
//! - **Les feuilles** loin de tout tronc (plus de 4 pas à travers d'autres feuilles)
//!   se décomposent : un arbre abattu perd ses feuilles peu à peu.
//! - **Les pousses** bien éclairées deviennent des arbres, s'il y a la place.

use std::collections::{HashMap, HashSet, VecDeque};

use glam::IVec3;

use crate::block::BlockId;
use crate::generator::{FREEZING, SEA_LEVEL};
use crate::grid::GridPos;
use crate::machines::{BlockBehavior, MachineContext};
use crate::registry::{BlockRegistry, Shape};
use crate::streaming::ChunkStreamer;
use crate::trees::{self, Part, Rng64, TreeBlocks, TreeKind};
use crate::world::World;

/// La lumière minimale (du ciel ou des blocs) pour que l'herbe s'étende et qu'une pousse grandisse.
const GROWTH_LIGHT: u8 = 9;

/// Lumière (ciel, blocs) au-dessus de `pos`, le bloc au-dessus, et s'il fait froid.
fn surroundings(streamer: &ChunkStreamer, pos: IVec3) -> (BlockId, u8, bool) {
    let world = streamer.world();
    let up = pos + IVec3::Y;
    let (sky, block) = world.light(up);
    (world.block(up), sky.max(block), is_cold(world, pos))
}

/// Il gèle à cet endroit : la température du biome, moins le froid de l'altitude.
pub fn is_cold(world: &World, pos: IVec3) -> bool {
    let Some(biomes) = world.biomes() else {
        return false;
    };
    let (chunk, [x, _, z]) = World::split(pos);
    let Some(id) = world.chunk(chunk).and_then(|c| c.biome(x, z)) else {
        return false;
    };
    if id as usize >= biomes.len() {
        return false;
    }
    let altitude = (pos.y - (SEA_LEVEL + 40)).max(0) as f32;
    biomes.get(crate::biome::BiomeId(id)).temperature - altitude * 0.012 < FREEZING
}

pub struct Grass {
    grass: BlockId,
    dirt: BlockId,
    snow: BlockId,
}

impl Grass {
    pub fn new(registry: &BlockRegistry) -> Option<Self> {
        Some(Self {
            grass: registry.id("core:grass")?,
            dirt: registry.id("core:dirt")?,
            snow: registry.id("core:snow")?,
        })
    }
}

impl BlockBehavior for Grass {
    fn random_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let Some(registry) = ctx.registry() else {
            return;
        };
        for &at in positions {
            let (above, light, cold) = surroundings(&ctx.grids.world().borrow(), at.pos);
            let def = registry.def(above);
            // Étouffée sous un bloc opaque : elle meurt.
            if !def.transparent && !above.is_air() {
                ctx.set_block(at, self.dirt);
                continue;
            }
            // Dans le froid, à ciel ouvert : la neige se pose.
            if cold && above.is_air() && light >= 15 {
                ctx.set_block(at, self.snow);
                continue;
            }
            if light < GROWTH_LIGHT {
                continue;
            }
            // Elle gagne un bloc de terre voisin (jusqu'à 3 plus bas, 1 plus haut).
            let r = ctx.random();
            let offset = IVec3::new(
                (r % 3) as i32 - 1,
                ((r >> 8) % 5) as i32 - 3,
                ((r >> 16) % 3) as i32 - 1,
            );
            let target = at.offset(offset);
            let (block, target_light, above_target) = {
                let streamer = ctx.grids.world().borrow();
                let (above, light, _) = surroundings(&streamer, target.pos);
                (streamer.world().block(target.pos), light, above)
            };
            let open = above_target.is_air()
                || (registry.def(above_target).transparent && !registry.def(above_target).liquid);
            if block == self.dirt && open && target_light >= GROWTH_LIGHT {
                ctx.set_block(target, self.grass);
            }
        }
    }
}

pub struct Leaves {
    logs: HashSet<BlockId>,
    leaves: HashSet<BlockId>,
}

/// Au-delà de cette distance (en pas à travers les feuilles) d'un tronc, les feuilles meurent.
pub const LEAF_REACH: i32 = 4;

impl Leaves {
    pub fn new(registry: &BlockRegistry) -> Self {
        let (mut logs, mut leaves) = (HashSet::new(), HashSet::new());
        for (id, def) in registry.iter() {
            if def.key.ends_with("_log") {
                logs.insert(id);
            }
            if def.behavior.as_deref() == Some("core:leaves") {
                leaves.insert(id);
            }
        }
        Self { logs, leaves }
    }

    /// Un tronc à moins de `LEAF_REACH` pas, en ne traversant que des feuilles ?
    /// Un parcours en largeur : on explore les feuilles à distance 1, puis 2, etc.
    fn attached(&self, world: &World, start: IVec3) -> bool {
        let mut seen = HashSet::from([start]);
        let mut queue = VecDeque::from([(start, 0)]);
        while let Some((pos, distance)) = queue.pop_front() {
            for offset in [
                IVec3::X,
                IVec3::NEG_X,
                IVec3::Y,
                IVec3::NEG_Y,
                IVec3::Z,
                IVec3::NEG_Z,
            ] {
                let next = pos + offset;
                if !seen.insert(next) {
                    continue;
                }
                let block = world.block(next);
                if self.logs.contains(&block) {
                    return true;
                }
                if distance + 1 < LEAF_REACH && self.leaves.contains(&block) {
                    queue.push_back((next, distance + 1));
                }
            }
        }
        false
    }
}

impl BlockBehavior for Leaves {
    fn random_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        let lonely: Vec<GridPos> = {
            let streamer = ctx.grids.world().borrow();
            positions
                .iter()
                .copied()
                .filter(|at| !self.attached(streamer.world(), at.pos))
                .collect()
        };
        for at in lonely {
            ctx.set_block(at, BlockId::AIR);
        }
    }
}

pub struct Sapling {
    blocks: TreeBlocks,
    kinds: HashMap<BlockId, TreeKind>,
}

impl Sapling {
    pub fn new(registry: &BlockRegistry) -> Option<Self> {
        let kinds = registry
            .iter()
            .filter_map(|(id, def)| Some((id, TreeBlocks::kind_of_sapling(&def.key)?)))
            .collect();
        Some(Self {
            blocks: TreeBlocks::resolve(registry).ok()?,
            kinds,
        })
    }

    /// Fait pousser l'arbre de la pousse en `at`, s'il y a la place. Renvoie `true` si fait.
    pub fn grow(&self, ctx: &mut MachineContext, at: GridPos, kind: TreeKind) -> bool {
        let Some(registry) = ctx.registry() else {
            return false;
        };
        let mut parts: Vec<(IVec3, BlockId, Part)> = Vec::new();
        let mut rng = Rng64::new(ctx.random());
        trees::grow(kind, at.pos, &mut rng, &self.blocks, &mut |p, b, part| {
            parts.push((p, b, part))
        });
        // La place : le tronc ne doit rencontrer que de l'air, des plantes ou des feuilles.
        let fits = {
            let streamer = ctx.grids.world().borrow();
            let world = streamer.world();
            parts.iter().all(|&(p, _, part)| {
                let (chunk, _) = World::split(p);
                if world.chunk(chunk).is_none() && p.y < crate::generator::WORLD_HEIGHT_CHUNKS * 32
                {
                    return false; // Pas chargé : on attendra.
                }
                let existing = world.block(p);
                part == Part::Leaves || p == at.pos || existing.is_air() || {
                    let def = registry.def(existing);
                    def.shape != Shape::Cube || def.cutout
                }
            })
        };
        if !fits {
            return false;
        }
        for (p, block, part) in parts {
            let existing = ctx.block(GridPos::world(p));
            let replace = match part {
                Part::Leaves => existing.is_air(),
                Part::Log => true,
            };
            if replace {
                ctx.set_block(GridPos::new(at.grid, p), block);
            }
        }
        true
    }
}

impl BlockBehavior for Sapling {
    fn random_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            let block = ctx.block(at);
            let Some(&kind) = self.kinds.get(&block) else {
                continue;
            };
            let (_, light, _) = surroundings(&ctx.grids.world().borrow(), at.pos);
            // Une chance sur 4 à chaque tick aléatoire, à la lumière.
            if light >= GROWTH_LIGHT && ctx.chance(4) {
                self.grow(ctx, at, kind);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use glam::Vec3;

    use super::*;
    use crate::generator::TerrainGenerator;
    use crate::machines::Machines;
    use crate::streaming::{StreamingConfig, tests::pump_until_idle};

    /// Un monde chargé autour de l'origine, dans une plaine (ni froid ni désert).
    fn world() -> RefCell<ChunkStreamer> {
        let config = StreamingConfig {
            view_radius: 2,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(TerrainGenerator::new(7), config);
        pump_until_idle(&mut s, Vec3::new(5.0, 100.0, 5.0));
        // Les tests posent leurs blocs autour de (8, 110, 8) : on y impose une plaine.
        let plains = s.generator().biomes().id("core:plains").unwrap();
        s.world_mut_for_tests()
            .chunk_mut(IVec3::new(0, 3, 0))
            .set_biomes(vec![plains.0; 1024]);
        RefCell::new(s)
    }

    fn id(s: &RefCell<ChunkStreamer>, key: &str) -> BlockId {
        s.borrow().world().registry().id(key).unwrap()
    }

    /// Une dalle de `block` dans le ciel (y = 110, assez bas pour ne pas geler), 9 × 9.
    fn slab(s: &RefCell<ChunkStreamer>, block: BlockId) -> IVec3 {
        let center = IVec3::new(8, 110, 8);
        for dx in -4..=4 {
            for dz in -4..=4 {
                assert!(
                    s.borrow_mut()
                        .set_block(center + IVec3::new(dx, 0, dz), block)
                );
            }
        }
        center
    }

    /// Fait tourner les ticks aléatoires (beaucoup de tirages par chunk) jusqu'à `done`.
    fn tick_until(
        s: &RefCell<ChunkStreamer>,
        machines: &mut Machines,
        done: impl Fn(&ChunkStreamer) -> bool,
    ) -> bool {
        for _ in 0..400 {
            machines.run_random_ticks(s, IVec3::ZERO, 1, 4096, &mut Vec::new());
            machines.run_tick(s, &mut Vec::new());
            if done(&s.borrow()) {
                return true;
            }
        }
        false
    }

    #[test]
    fn grass_spreads_on_lit_dirt() {
        let s = world();
        let (grass, dirt) = (id(&s, "core:grass"), id(&s, "core:dirt"));
        let center = slab(&s, dirt);
        s.borrow_mut().set_block(center, grass);
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        // L'herbe gagne la dalle de proche en proche : au moins la moitié des 81 blocs.
        let count = |s: &ChunkStreamer| {
            (-4..=4)
                .flat_map(|dx| (-4..=4).map(move |dz| (dx, dz)))
                .filter(|&(dx, dz)| s.world().block(center + IVec3::new(dx, 0, dz)) == grass)
                .count()
        };
        assert!(
            tick_until(&s, &mut machines, |s| count(s) >= 40),
            "l'herbe ne s'étend pas"
        );
    }

    #[test]
    fn grass_dies_under_an_opaque_block() {
        let s = world();
        let (grass, dirt, stone) = (
            id(&s, "core:grass"),
            id(&s, "core:dirt"),
            id(&s, "core:stone"),
        );
        let center = slab(&s, grass);
        s.borrow_mut().set_block(center + IVec3::Y, stone);
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        assert!(tick_until(&s, &mut machines, |s| s.world().block(center) == dirt));
    }

    #[test]
    fn lonely_leaves_decay_but_attached_ones_stay() {
        let s = world();
        let (leaves, log) = (id(&s, "core:oak_leaves"), id(&s, "core:oak_log"));
        let lonely = IVec3::new(4, 150, 4);
        s.borrow_mut().set_block(lonely, leaves);
        let attached = IVec3::new(12, 150, 12);
        s.borrow_mut().set_block(attached, log);
        s.borrow_mut().set_block(attached + IVec3::X, leaves);
        s.borrow_mut().set_block(attached + IVec3::X * 2, leaves);
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        assert!(tick_until(&s, &mut machines, |s| s
            .world()
            .block(lonely)
            .is_air()));
        assert_eq!(s.borrow().world().block(attached + IVec3::X * 2), leaves);
    }

    #[test]
    fn a_sapling_grows_into_a_tree() {
        let s = world();
        let (dirt, sapling, log) = (
            id(&s, "core:dirt"),
            id(&s, "core:oak_sapling"),
            id(&s, "core:oak_log"),
        );
        let center = slab(&s, dirt);
        let plant = center + IVec3::Y;
        s.borrow_mut().set_block(plant, sapling);
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        assert!(
            tick_until(&s, &mut machines, |s| s.world().block(plant) == log),
            "la pousse n'a pas grandi"
        );
        // Un vrai tronc : au moins 4 bûches empilées.
        for y in 0..4 {
            assert_eq!(s.borrow().world().block(plant + IVec3::Y * y), log);
        }
    }

    #[test]
    fn snow_settles_in_cold_biomes() {
        let generator = TerrainGenerator::new(42);
        let tundra = generator.biomes().id("core:snowy_tundra").unwrap();
        let (x, z) = generator
            .find_column(0, 0, 32, 20_000, |c| {
                c.biome == tundra && c.height > SEA_LEVEL
            })
            .expect("pas de toundra");
        let config = StreamingConfig {
            view_radius: 1,
            ..StreamingConfig::default()
        };
        let mut streamer = ChunkStreamer::new(generator, config);
        pump_until_idle(&mut streamer, Vec3::new(x as f32, 100.0, z as f32));
        let s = RefCell::new(streamer);
        let (grass, snow) = (id(&s, "core:grass"), id(&s, "core:snow"));
        let at = IVec3::new(x, 150, z);
        s.borrow_mut().set_block(at, grass);
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        let center = World::split(at).0;
        let done = (0..400).any(|_| {
            machines.run_random_ticks(&s, center, 1, 4096, &mut Vec::new());
            s.borrow().world().block(at) == snow
        });
        assert!(done, "pas de neige sur l'herbe de la toundra");
    }
}
