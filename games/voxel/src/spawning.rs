//! L'apparition des créatures (phase O5) : où, quand, combien.
//!
//! Chaque créature dit dans `entities.ron` où elle peut apparaître :
//! - `spawn_on` : les blocs du sol ;
//! - `light` : la lumière effective (0 à 15) de la case où elle apparaît. Par défaut,
//!   les hostiles dans le noir (0 à 7 : la nuit, les grottes), les autres au jour (9 à 15) ;
//! - `when` : `Any` (défaut), `Day` ou `Night` ;
//! - `biomes` : les biomes permis (vide : tous) ;
//! - `group` : combien d'un coup (min, max) ; `weight` : sa part au tirage (10) ;
//! - `dissolves` : se dissout au soleil (par défaut : les hostiles).
//!
//! La lumière effective d'une case : la plus forte de la lumière des blocs (torches) et
//! de celle du ciel multipliée par l'heure. La nuit, le ciel tombe à 1 ou 2 : les
//! créatures de la nuit sortent partout dehors, et à toute heure au fond des grottes.
//!
//! Deux limites : au plus [`HOSTILE_CAP`] hostiles et [`PASSIVE_CAP`] autres dans la zone
//! de simulation. Au-delà de cette zone ([`SIM_DISTANCE`]), une créature ne pense ni ne
//! bouge : elle attend que le joueur revienne, ou disparaît s'il s'éloigne trop (l'idée
//! vient d'Horizon : sept fois moins d'entités à faire tourner).

use std::sync::Arc;

use engine_ecs::World;
use engine_voxel::BlockId;
use glam::{IVec3, Vec3};
use serde::Deserialize;

use crate::components::{Mob, Transform};
use crate::mobs::{self, FixedDt, GameRng, MobRegistry, PlayerEntity, SimClock};
use crate::shared::StreamerExt as _;

/// Au-delà (à l'horizontale), une créature ne pense ni ne bouge.
pub const SIM_DISTANCE: f32 = 48.0;
/// Les limites, dans la zone de simulation.
pub const HOSTILE_CAP: usize = 16;
pub const PASSIVE_CAP: usize = 10;
/// Les bancs de poissons ne prennent pas la place des animaux terrestres.
pub const AQUATIC_CAP: usize = 12;
/// L'anneau où elles apparaissent autour du joueur (ni sous son nez, ni trop loin).
const SPAWN_MIN: f32 = 16.0;
const SPAWN_MAX: f32 = 44.0;
/// Essais par passage, et temps entre deux passages.
const ATTEMPTS: usize = 4;
const SPAWN_INTERVAL: f32 = 0.5;
/// Lumière effective au-dessus de laquelle une créature qui `dissolves` peut disparaître,
/// et sa chance de disparaître à chaque seconde.
const SUNLIGHT: u8 = 12;
const DISSOLVE_CHANCE: f32 = 0.25;

/// Le moment de la journée où une créature peut apparaître.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum When {
    #[default]
    Any,
    Day,
    Night,
}

/// Les règles d'apparition d'une créature (voir l'en-tête du module).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnRules {
    pub on: Vec<BlockId>,
    pub light: (u8, u8),
    pub when: When,
    pub biomes: Vec<String>,
    pub group: (u32, u32),
    pub weight: u32,
    pub dissolves: bool,
}

/// Le ciel du moment, posé par le jeu à chaque image : l'ECS ne connaît pas l'heure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    /// Le facteur de la lumière du ciel (de 0,1 la nuit à 1 à midi).
    pub daylight: f32,
    pub night: bool,
}

impl Default for Sky {
    fn default() -> Self {
        Self {
            daylight: 1.0,
            night: false,
        }
    }
}

/// La lumière effective d'une case : la plus forte des torches et du ciel à cette heure.
pub fn effective_light(sky: u8, block: u8, daylight: f32) -> u8 {
    block.max((f32::from(sky) * daylight).round().clamp(0.0, 15.0) as u8)
}

/// Une case où une créature pourrait apparaître.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot<'a> {
    pub floor: BlockId,
    pub light: u8,
    pub night: bool,
    pub biome: &'a str,
    pub in_water: bool,
}

/// Cette créature peut-elle apparaître ici ?
pub fn allowed(rules: &SpawnRules, spot: &Spot) -> bool {
    let when = match rules.when {
        When::Any => true,
        When::Day => !spot.night,
        When::Night => spot.night,
    };
    when && rules.on.contains(&spot.floor)
        && (rules.light.0..=rules.light.1).contains(&spot.light)
        && (rules.biomes.is_empty() || rules.biomes.iter().any(|b| b == spot.biome))
}

/// Tire au sort une créature de la catégorie (hostile ou non) qui peut apparaître ici,
/// selon les poids. `roll` : un nombre entre 0 et 1.
pub fn choose(registry: &MobRegistry, hostile: bool, spot: &Spot, roll: f32) -> Option<u32> {
    let candidates: Vec<(u32, u32)> = (0..registry.len() as u32)
        .filter_map(|kind| {
            let def = registry.get(kind)?;
            (def.hostile == hostile
                && def.aquatic == spot.in_water
                && def.spawn.weight > 0
                && allowed(&def.spawn, spot))
            .then_some((kind, def.spawn.weight))
        })
        .collect();
    let total: u32 = candidates.iter().map(|(_, w)| w).sum();
    if total == 0 {
        return None;
    }
    let mut pick = (roll.clamp(0.0, 0.999_999) * total as f32) as u32;
    for (kind, weight) in candidates {
        if pick < weight {
            return Some(kind);
        }
        pick -= weight;
    }
    None
}

/// Les créatures de la zone de simulation : (hostiles, terrestres passives, aquatiques).
fn census(world: &World, center: Vec3, registry: &MobRegistry) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    world
        .query::<(&Transform, &Mob)>()
        .for_each(|_, (transform, mob)| {
            if horizontal_distance(transform.position, center) > SIM_DISTANCE {
                return;
            }
            match registry.get(mob.kind) {
                Some(def) if def.hostile => counts.0 += 1,
                // Un poids nul : un habitant, qui n'apparaît pas seul et ne compte pas.
                Some(def) if def.spawn.weight == 0 => {}
                Some(def) if def.aquatic => counts.2 += 1,
                _ => counts.1 += 1,
            }
        });
    counts
}

pub fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

/// Hors de la zone de simulation : ni IA ni physique (le joueur, lui, est toujours
/// simulé : ce n'est pas une créature).
pub fn asleep(world: &World, entity: engine_ecs::Entity, position: Vec3, player: Vec3) -> bool {
    world.has::<Mob>(entity) && horizontal_distance(position, player) > SIM_DISTANCE
}

/// Toutes les [`SPAWN_INTERVAL`] secondes, quelques essais d'apparition autour du joueur.
/// La difficulté du monde : 0 paisible (aucune hostile), 1 normale, 2 difficile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Difficulty(pub u8);

pub fn spawn(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    {
        let mut clock = world.resource_mut::<SimClock>();
        clock.spawn_timer -= dt;
        if clock.spawn_timer > 0.0 {
            return;
        }
        clock.spawn_timer = SPAWN_INTERVAL;
    }
    let player = world.resource::<PlayerEntity>().0;
    let Some(center) = world.get::<Transform>(player).map(|t| t.position) else {
        return;
    };
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let sky = *world.resource::<Sky>();
    let (mut hostiles, mut passives, mut aquatics) = census(world, center, &registry);
    for _ in 0..ATTEMPTS {
        let [r_kind, r_angle, r_dist, r_cave, r_y, r_pick, r_group] = {
            let mut rng = world.resource_mut::<GameRng>();
            std::array::from_fn(|_| rng.next_f32())
        };
        let hostile = world.resource::<Difficulty>().0 > 0 && r_kind < 0.5;
        let aquatic = !hostile && r_cave < 0.35;
        if (hostile && hostiles >= HOSTILE_CAP)
            || (aquatic && aquatics >= AQUATIC_CAP)
            || (!hostile && !aquatic && passives >= PASSIVE_CAP)
        {
            continue;
        }
        let angle = r_angle * std::f32::consts::TAU;
        let distance = SPAWN_MIN + r_dist * (SPAWN_MAX - SPAWN_MIN);
        let x = (center.x + angle.cos() * distance).floor() as i32;
        let z = (center.z + angle.sin() * distance).floor() as i32;
        // La moitié des essais en surface, l'autre à une hauteur proche de celle du
        // joueur (les grottes quand il est sous terre).
        let land_height_roll = if hostile {
            r_cave
        } else {
            (r_cave - 0.35) / 0.65
        };
        let start_y = if land_height_roll < 0.5 {
            None
        } else {
            Some(center.y.floor() as i32 + (r_y * 32.0) as i32 - 16)
        };
        let found = {
            let shared = world.streamer();
            let streamer = shared.borrow();
            let generator = streamer.generator();
            let column = generator.column(x, z);
            let voxels = streamer.world();
            let floor = if aquatic {
                let top = center.y.floor() as i32 + 16;
                let wet: Vec<_> = (0..64)
                    .map(|dy| IVec3::new(x, top - dy, z))
                    .filter(|p| streamer.is_lit(engine_voxel::World::split(*p).0))
                    .filter(|p| {
                        crate::aquatic::fits(
                            voxels,
                            p.as_vec3() + Vec3::new(0.5, 0.15, 0.5),
                            0.1,
                            0.1,
                        )
                    })
                    .collect();
                if wet.is_empty() {
                    None
                } else {
                    wet.get((r_y * wet.len() as f32) as usize).copied()
                }
            } else {
                match start_y {
                    None => Some(IVec3::new(x, column.height, z)),
                    Some(y) => (0..12)
                        .map(|dy| IVec3::new(x, y - dy, z))
                        .find(|p| standable(voxels, &registry, *p)),
                }
                .filter(|p| standable(voxels, &registry, *p))
                .filter(|p| streamer.is_lit(engine_voxel::World::split(*p).0))
            };
            floor.and_then(|floor| {
                let (sky_light, block_light) =
                    voxels.light(if aquatic { floor } else { floor + IVec3::Y });
                let biome = &generator.biome(column.biome).key;
                let spot = Spot {
                    floor: voxels.block(floor),
                    light: effective_light(sky_light, block_light, sky.daylight),
                    night: sky.night,
                    biome,
                    in_water: aquatic,
                };
                choose(&registry, hostile, &spot, r_pick).map(|kind| (kind, floor))
            })
        };
        let Some((kind, floor)) = found else { continue };
        let (min, max) = registry.get(kind).map_or((1, 1), |d| d.spawn.group);
        let count = min + ((max.saturating_sub(min) + 1) as f32 * r_group) as u32;
        for i in 0..count.min(max.max(1)) {
            if (hostile && hostiles >= HOSTILE_CAP)
                || (aquatic && aquatics >= AQUATIC_CAP)
                || (!hostile && !aquatic && passives >= PASSIVE_CAP)
            {
                break;
            }
            // Le groupe autour du premier, sur le même sol.
            let offset = IVec3::new((i % 3) as i32 - 1, 0, (i / 3) as i32 - 1);
            let at = if i == 0 { floor } else { floor + offset };
            let position = Vec3::new(
                at.x as f32 + 0.5,
                at.y as f32 + if aquatic { 0.15 } else { 1.0 },
                at.z as f32 + 0.5,
            );
            let ok = if aquatic {
                let shared = world.streamer();
                let streamer = shared.borrow();
                let def = registry.get(kind).unwrap();
                crate::aquatic::fits(streamer.world(), position, def.width, def.height)
            } else {
                i == 0 || {
                    let shared = world.streamer();
                    let streamer = shared.borrow();
                    standable(streamer.world(), &registry, at)
                }
            };
            if ok {
                let spawned = mobs::spawn_mob(world, kind, position);
                // Une part des hostiles naît élite, selon le palier du monde (voir `elites.rs`).
                if let Some(entity) = spawned.filter(|_| hostile) {
                    crate::elites::maybe_promote(world, entity);
                }
                if hostile {
                    hostiles += 1;
                } else if aquatic {
                    aquatics += 1;
                } else {
                    passives += 1;
                }
            }
        }
    }
}

/// Un sol où se tenir : un bloc plein (pas un liquide), deux cases libres au-dessus.
pub(crate) fn standable(
    voxels: &engine_voxel::World,
    _registry: &MobRegistry,
    floor: IVec3,
) -> bool {
    voxels.is_solid(floor)
        && !voxels.is_solid(floor + IVec3::Y)
        && !voxels.is_solid(floor + IVec3::Y * 2)
        && voxels.block(floor + IVec3::Y).is_air()
}

/// Chaque seconde, les créatures de la nuit exposées au soleil peuvent se dissoudre (elles
/// disparaissent, sans butin).
pub fn dissolve(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    {
        let mut clock = world.resource_mut::<SimClock>();
        clock.dissolve_timer -= dt;
        if clock.dissolve_timer > 0.0 {
            return;
        }
        clock.dissolve_timer = 1.0;
    }
    let sky = *world.resource::<Sky>();
    let peaceful = world.resource::<Difficulty>().0 == 0;
    if sky.night && !peaceful {
        return;
    }
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut gone = Vec::new();
    world
        .query::<(&Transform, &Mob)>()
        .for_each(|entity, (transform, mob)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            // Un monde paisible n'a pas d'hostiles : celles qui restent disparaissent.
            if peaceful && def.hostile {
                gone.push(entity);
                return;
            }
            // Une élite ou un envahisseur reste : on est venu pour elle (voir `elites.rs`).
            if !def.spawn.dissolves || world.has::<crate::elites::Elite>(entity) {
                return;
            }
            let head = (transform.position + Vec3::Y * 0.9).floor().as_ivec3();
            let (sky_light, block_light) = streamer.world().light(head);
            if effective_light(sky_light, block_light, sky.daylight) >= SUNLIGHT {
                gone.push(entity);
            }
        });
    drop(streamer);
    for entity in gone {
        let roll = world.resource_mut::<GameRng>().next_f32();
        if roll < DISSOLVE_CHANCE {
            world.commands().despawn(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(light: (u8, u8), when: When, biomes: &[&str]) -> SpawnRules {
        SpawnRules {
            on: vec![BlockId(3)],
            light,
            when,
            biomes: biomes.iter().map(|b| b.to_string()).collect(),
            group: (1, 1),
            weight: 10,
            dissolves: false,
        }
    }

    fn spot(light: u8, night: bool, biome: &str) -> Spot<'_> {
        Spot {
            floor: BlockId(3),
            light,
            night,
            biome,
            in_water: false,
        }
    }

    #[test]
    fn effective_light_follows_the_hour_and_the_torches() {
        assert_eq!(effective_light(15, 0, 1.0), 15, "midi dehors");
        assert_eq!(effective_light(15, 0, 0.1), 2, "la nuit dehors");
        assert_eq!(
            effective_light(15, 11, 0.1),
            11,
            "la nuit près d'une torche"
        );
        assert_eq!(effective_light(0, 0, 1.0), 0, "au fond d'une grotte");
    }

    #[test]
    fn rules_check_floor_light_time_and_biome() {
        let night_monster = rules((0, 7), When::Any, &[]);
        assert!(allowed(&night_monster, &spot(2, true, "core:plains")));
        assert!(
            allowed(&night_monster, &spot(0, false, "core:plains")),
            "grotte de jour"
        );
        assert!(!allowed(&night_monster, &spot(15, false, "core:plains")));
        let other_floor = Spot {
            floor: BlockId(4),
            ..spot(0, true, "core:plains")
        };
        assert!(!allowed(&night_monster, &other_floor));

        let forest_animal = rules((9, 15), When::Day, &["core:forest"]);
        assert!(allowed(&forest_animal, &spot(15, false, "core:forest")));
        assert!(!allowed(&forest_animal, &spot(15, false, "core:desert")));
        assert!(
            !allowed(&forest_animal, &spot(15, true, "core:forest")),
            "pas la nuit"
        );
    }

    #[test]
    fn weights_decide_the_draw() {
        let mut registry = MobRegistry::default();
        let blocks = engine_voxel::BlockRegistry::builtin();
        let entry = |id: &str, weight: u32| {
            format!(
                r#"[(id: "{id}", name: "X", health: 1.0, speed: 1.0, size: (1.0, 1.0), spawn_on: ["core:grass"], weight: {weight}, parts: [])]"#
            )
        };
        registry
            .add_source("core", &entry("core:a", 30), "t", &blocks)
            .unwrap();
        registry
            .add_source("core", &entry("core:b", 10), "t", &blocks)
            .unwrap();
        let grass = blocks.id("core:grass").unwrap();
        let here = Spot {
            floor: grass,
            ..spot(15, false, "core:plains")
        };
        // 3/4 pour la première, 1/4 pour la seconde.
        assert_eq!(choose(&registry, false, &here, 0.0), Some(0));
        assert_eq!(choose(&registry, false, &here, 0.74), Some(0));
        assert_eq!(choose(&registry, false, &here, 0.76), Some(1));
        assert_eq!(choose(&registry, true, &here, 0.5), None, "aucune hostile");
        let night = Spot { light: 2, ..here };
        assert_eq!(choose(&registry, false, &night, 0.5), None, "trop sombre");
    }

    #[test]
    fn fish_are_chosen_only_in_water_and_land_animals_keep_their_daylight_rules() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let blocks = crate::test_registry();
        let registry = MobRegistry::load(&root.join("assets"), &[], &blocks).unwrap();
        let wet = Spot {
            floor: blocks.id("core:water").unwrap(),
            light: 2,
            night: true,
            biome: "core:river",
            in_water: true,
        };
        for i in 0..100 {
            let kind = choose(&registry, false, &wet, i as f32 / 100.0).unwrap();
            assert_eq!(registry.get(kind).unwrap().key, "core:fish");
        }
        assert!(
            choose(
                &registry,
                false,
                &Spot {
                    in_water: false,
                    ..wet
                },
                0.0
            )
            .is_none()
        );
        assert!(
            choose(
                &registry,
                false,
                &Spot {
                    floor: blocks.id("core:lava").unwrap(),
                    ..wet
                },
                0.0
            )
            .is_none()
        );
        let land = Spot {
            floor: blocks.id("core:grass").unwrap(),
            light: 15,
            night: false,
            biome: "core:plains",
            in_water: false,
        };
        let mut kinds = std::collections::HashSet::new();
        for i in 0..100 {
            let def = registry
                .get(choose(&registry, false, &land, i as f32 / 100.0).unwrap())
                .unwrap();
            assert!(!def.aquatic);
            kinds.insert(def.key.as_str());
        }
        assert!(kinds.contains("core:sheep") && kinds.contains("core:chicken"));
        assert!(
            choose(
                &registry,
                false,
                &Spot {
                    light: 2,
                    night: true,
                    ..land
                },
                0.0
            )
            .is_none()
        );
    }
}
