//! La toison : une créature qui a `fleece` dans `entities.ron` (le mouton, l'alpaga) porte une
//! laine de sa couleur. Le joueur la **tond** d'un clic droit avec des cisailles : la laine tombe, et
//! l'animal reste nu jusqu'à ce qu'il ait **brouté** assez longtemps pour qu'elle repousse.
//!
//! Tout se décrit en données :
//!
//! ```ron
//! fleece: (coats: Alpaca, count: (1, 3), regrow: 90.0, skin: (0.72, 0.58, 0.5)),
//! ```
//!
//! - `coats` : l'ensemble des couleurs naturelles de l'espèce (`Sheep` par défaut, `Alpaca`) ;
//! - `count` : le nombre de laines d'une tonte (min, max) ;
//! - `regrow` : les secondes de pâture avant que la laine revienne ;
//! - `skin` : la couleur de la peau qu'on voit sous la laine.
//!
//! Les boîtes qui forment la toison sont celles dont la matière finit par `_wool` (voir
//! `sheep_colors::is_fleece_texture`) : une fois tondu, le client les dessine amincies, couleur peau.

use std::sync::Arc;

use engine_ecs::{Entity, World};
use serde::Deserialize;

use crate::components::{Baby, Mob, Transform, Velocity};
use crate::mobs::sheep_colors::{CoatSet, SheepCoat, drop_key};
use crate::mobs::{FixedDt, MobRegistry};
use crate::shared::StreamerExt as _;

/// La toison d'une créature (`fleece` dans `entities.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleeceDef {
    #[serde(default)]
    pub coats: CoatSet,
    /// Le nombre de laines d'une tonte (min, max).
    pub count: (u32, u32),
    /// Les secondes de pâture avant que la laine repousse.
    pub regrow: f32,
    /// La peau sous la laine.
    pub skin: (f32, f32, f32),
}

/// Un animal tondu : il lui reste `left` secondes de pâture avant que sa laine repousse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shorn {
    pub left: f32,
}

/// Un animal qui broute avance à ce rythme quand il marche (1 quand il s'arrête pour brouter).
const GRAZE_WHILE_WALKING: f32 = 0.25;
/// En dessous de cette vitesse (blocs par seconde) il est à l'arrêt.
const STILL_SPEED: f32 = 0.2;

/// Tond `entity` : renvoie l'objet de laine (de la couleur de l'animal) et le nombre, ou `None` si
/// elle n'a pas de toison, est un petit, ou a déjà été tondue. `roll` dans [0, 1[ tire le nombre.
pub fn shear(world: &mut World, entity: Entity, roll: f32) -> Option<(String, u32)> {
    let kind = world.get::<Mob>(entity)?.kind;
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let fleece = registry.get(kind)?.fleece?;
    if world.has::<Baby>(entity) || world.has::<Shorn>(entity) {
        return None;
    }
    let coat = world.get::<SheepCoat>(entity).map(|c| *c);
    let item = drop_key("core:wool", coat).to_owned();
    let (min, max) = fleece.count;
    let span = max.saturating_sub(min) + 1;
    let count = min + ((roll.clamp(0.0, 0.999_999) * span as f32) as u32).min(span - 1);
    world.insert(
        entity,
        Shorn {
            left: fleece.regrow,
        },
    );
    Some((item, count))
}

/// Le système : un animal tondu qui broute (debout sur un bloc où il apparaît, d'herbe en général)
/// voit sa laine revenir.
pub fn graze(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut regrown = Vec::new();
    world
        .query::<(&mut Shorn, &Transform, &Velocity, &Mob)>()
        .for_each(|entity, (shorn, transform, velocity, mob)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            let under = (transform.position - glam::Vec3::Y * 0.1)
                .floor()
                .as_ivec3();
            if !def.spawn.on.contains(&streamer.world().block(under)) {
                return;
            }
            let still = velocity.0.x.hypot(velocity.0.z) < STILL_SPEED;
            shorn.left -= dt * if still { 1.0 } else { GRAZE_WHILE_WALKING };
            if shorn.left <= 0.0 {
                regrown.push(entity);
            }
        });
    drop(streamer);
    for entity in regrown {
        world.commands().remove::<Shorn>(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{self, tests::sim};
    use glam::Vec3;

    fn spawn(world: &mut World, key: &str, at: Vec3) -> Entity {
        let kind = world.resource::<Arc<MobRegistry>>().id(key).unwrap();
        mobs::spawn_mob(world, kind, at).unwrap()
    }

    #[test]
    fn shearing_gives_wool_of_the_animals_colour_once() {
        let mut world = sim();
        let alpaca = spawn(&mut world, "core:alpaca", Vec3::new(5.5, 80.0, 5.5));
        world.insert(alpaca, SheepCoat(16));
        let (item, count) = shear(&mut world, alpaca, 0.99).unwrap();
        assert_eq!(item, "core:wool_cream");
        assert_eq!(count, 3);
        assert!(world.has::<Shorn>(alpaca));
        assert_eq!(shear(&mut world, alpaca, 0.0), None, "déjà tondu");
        let pig = spawn(&mut world, "core:pig", Vec3::new(8.5, 80.0, 5.5));
        assert_eq!(shear(&mut world, pig, 0.0), None, "pas de toison");
    }

    #[test]
    fn a_lamb_cannot_be_sheared() {
        let mut world = sim();
        let sheep = spawn(&mut world, "core:sheep", Vec3::new(5.5, 80.0, 5.5));
        world.insert(sheep, Baby { age_left: 100.0 });
        assert_eq!(shear(&mut world, sheep, 0.5), None);
    }

    #[test]
    fn the_fleece_regrows_only_while_grazing_on_grass() {
        let mut world = sim();
        let blocks = crate::mobs::tests::blocks();
        let (grass, stone) = (
            blocks.id("core:grass").unwrap(),
            blocks.id("core:stone").unwrap(),
        );
        let sheep = spawn(&mut world, "core:sheep", Vec3::new(5.5, 150.0, 5.5));
        world.insert(sheep, Shorn { left: 2.0 });
        let floor = glam::IVec3::new(5, 149, 5);
        let streamer = world.streamer();
        // Sur de la pierre, il a beau rester là : rien ne repousse.
        streamer.borrow_mut().set_block_raw(floor, stone);
        for _ in 0..600 {
            graze(&mut world);
            world.apply_commands();
        }
        assert!(world.has::<Shorn>(sheep), "pas d'herbe, pas de laine");
        // Sur l'herbe, immobile : 2 secondes (120 pas) suffisent.
        streamer.borrow_mut().set_block_raw(floor, grass);
        for _ in 0..110 {
            graze(&mut world);
            world.apply_commands();
        }
        assert!(world.has::<Shorn>(sheep), "pas encore");
        for _ in 0..20 {
            graze(&mut world);
            world.apply_commands();
        }
        assert!(!world.has::<Shorn>(sheep), "la laine a repoussé");
    }

    #[test]
    fn alpacas_live_in_the_alpine_biomes_and_have_a_natural_coat() {
        let mut world = sim();
        let registry = world.resource::<Arc<MobRegistry>>().clone();
        let def = registry.by_key("core:alpaca").unwrap();
        for biome in ["core:alpine_meadow", "core:highland", "core:steppe"] {
            assert!(def.spawn.biomes.iter().any(|b| b == biome), "{biome}");
        }
        assert!(def.spawn.group.0 >= 2, "en troupeau");
        let natural = ["white", "cream", "fawn", "brown", "gray", "black"];
        for _ in 0..40 {
            let alpaca = spawn(&mut world, "core:alpaca", Vec3::new(5.5, 90.0, 5.5));
            let coat = world.get::<SheepCoat>(alpaca).unwrap();
            assert!(
                natural.contains(&coat.data().id.as_str()),
                "{}",
                coat.data().id
            );
        }
    }

    #[test]
    fn the_death_of_a_shorn_animal_is_announced_as_shorn() {
        let mut world = sim();
        let sheep = spawn(&mut world, "core:sheep", Vec3::new(5.5, 80.0, 5.5));
        assert_eq!(shear(&mut world, sheep, 0.0).unwrap().1, 1);
        let mut reader = world
            .resource::<engine_ecs::Events<crate::events::EntityDied>>()
            .reader();
        world
            .get_mut::<crate::components::Health>(sheep)
            .unwrap()
            .current = 0.0;
        mobs::cleanup(&mut world);
        world.apply_commands();
        let deaths: Vec<_> = reader
            .read(&world.resource::<engine_ecs::Events<crate::events::EntityDied>>())
            .cloned()
            .collect();
        assert_eq!(deaths.len(), 1);
        assert!(deaths[0].shorn, "tondu à sa mort");
    }
}
