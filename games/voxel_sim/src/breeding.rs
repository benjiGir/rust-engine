//! L'élevage : nourrir deux animaux du même type les met « en rut » ; ils se rejoignent
//! et un petit naît, qui grandit en quelques minutes.
//!
//! Tout est décrit en données (`breeds` dans `entities.ron`) : l'aliment, la pause entre
//! deux naissances, le temps de croissance.

use std::sync::Arc;

use engine_ecs::{Entity, World};
use glam::Vec3;
use serde::Deserialize;

use crate::components::{Baby, BreedCooldown, Collider, Love, Mob, Transform, Wander};
use crate::mobs::{self, FixedDt, MobRegistry};
use crate::navigation::{PathFollow, cell_of};

/// Combien de temps un animal nourri cherche un partenaire.
const LOVE_SECONDS: f32 = 30.0;
/// Deux partenaires qui se touchent (en blocs) font un petit.
const MEETING_DISTANCE: f32 = 1.4;
/// On ne cherche un partenaire qu'à cette distance.
const SEARCH_DISTANCE: f32 = 12.0;
/// Au-delà de ce nombre d'animaux du même type, plus de naissance (le monde reste léger).
const MAX_PER_KIND: usize = 24;
/// Un petit mesure la moitié d'un adulte.
pub const BABY_SCALE: f32 = 0.5;

fn default_cooldown() -> f32 {
    300.0
}

fn default_growth() -> f32 {
    600.0
}

/// Comment une créature se reproduit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreedingDef {
    /// L'objet qui la met en rut (`core:wheat`).
    pub food: String,
    /// Secondes avant que les parents puissent recommencer (300).
    #[serde(default = "default_cooldown")]
    pub cooldown: f32,
    /// Secondes avant que le petit soit adulte (600).
    #[serde(default = "default_growth")]
    pub growth: f32,
}

/// Le joueur tend `item_key` à `entity` : si elle l'aime et peut se reproduire, elle entre
/// en rut. Renvoie `true` si l'aliment est consommé.
pub fn feed(world: &mut World, entity: Entity, item_key: &str) -> bool {
    let Some(kind) = world.get::<Mob>(entity).map(|m| m.kind) else {
        return false;
    };
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let likes = registry
        .get(kind)
        .and_then(|d| d.breeds.as_ref())
        .is_some_and(|b| b.food == item_key);
    if !likes
        || world.has::<Baby>(entity)
        || world.has::<Love>(entity)
        || world.has::<BreedCooldown>(entity)
    {
        return false;
    }
    world.insert(
        entity,
        Love {
            timer: LOVE_SECONDS,
        },
    );
    true
}

/// Le système : les minuteurs, la recherche d'un partenaire, les naissances.
pub fn update(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let registry = world.resource::<Arc<MobRegistry>>().clone();

    // Les minuteurs.
    world.query::<(&mut Love,)>().for_each(|entity, (love,)| {
        love.timer -= dt;
        if love.timer <= 0.0 {
            world.commands().remove::<Love>(entity);
        }
    });
    world
        .query::<(&mut BreedCooldown,)>()
        .for_each(|entity, (cooldown,)| {
            cooldown.timer -= dt;
            if cooldown.timer <= 0.0 {
                world.commands().remove::<BreedCooldown>(entity);
            }
        });
    let mut grown = Vec::new();
    world
        .query::<(&mut Baby, &Mob)>()
        .for_each(|entity, (baby, mob)| {
            baby.age_left -= dt;
            if baby.age_left <= 0.0 {
                grown.push((entity, mob.kind));
                world.commands().remove::<Baby>(entity);
            }
        });
    for (entity, kind) in grown {
        if let (Some(def), Some(mut collider)) =
            (registry.get(kind), world.get_mut::<Collider>(entity))
        {
            collider.width = def.width;
            collider.height = def.height;
        }
    }

    // Les animaux en rut : (qui, type, où).
    let mut lovers: Vec<(Entity, u32, Vec3)> = Vec::new();
    world
        .query::<(&Love, &Mob, &Transform)>()
        .for_each(|entity, (_, mob, transform)| {
            if !world.has::<Baby>(entity) {
                lovers.push((entity, mob.kind, transform.position));
            }
        });
    let mut births: Vec<(Entity, Entity, u32, Vec3)> = Vec::new();
    for &(entity, kind, position) in &lovers {
        let partner = lovers
            .iter()
            .filter(|&&(other, other_kind, _)| other != entity && other_kind == kind)
            .map(|&(other, _, at)| (other, at, at.distance(position)))
            .filter(|&(_, _, d)| d < SEARCH_DISTANCE)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let Some((other, at, distance)) = partner else {
            continue;
        };
        // Aller vers lui (l'IA d'errance suit ce but tant que le minuteur n'a pas expiré).
        if let (Some(mut wander), Some(mut follow)) = (
            world.get_mut::<Wander>(entity),
            world.get_mut::<PathFollow>(entity),
        ) {
            wander.timer = 1.0;
            follow.wander_goal = Some(cell_of(at));
        }
        if distance < MEETING_DISTANCE && entity < other {
            births.push((entity, other, kind, (position + at) * 0.5));
        }
    }

    for (a, b, kind, at) in births {
        let Some(def) = registry.get(kind) else {
            continue;
        };
        let Some(breeds) = def.breeds.clone() else {
            continue;
        };
        // Les parents se reposent, même si le monde est plein.
        for parent in [a, b] {
            world.commands().remove::<Love>(parent);
            world.insert(
                parent,
                BreedCooldown {
                    timer: breeds.cooldown,
                },
            );
        }
        if mobs::census(world).get(&kind).copied().unwrap_or(0) >= MAX_PER_KIND {
            continue;
        }
        let coat_a = world.get::<crate::sheep_colors::SheepCoat>(a).map(|c| *c);
        let coat_b = world.get::<crate::sheep_colors::SheepCoat>(b).map(|c| *c);
        let inherited = if coat_a.is_some() || coat_b.is_some() {
            crate::sheep_colors::SheepCoat::inherit(
                coat_a,
                coat_b,
                world.resource_mut::<crate::mobs::GameRng>().next_f32(),
            )
        } else {
            None
        };
        if let Some(baby) = mobs::spawn_mob_with_coat(world, kind, at, inherited) {
            world.insert(
                baby,
                Baby {
                    age_left: breeds.growth,
                },
            );
            if let Some(mut collider) = world.get_mut::<Collider>(baby) {
                collider.width = def.width * BABY_SCALE;
                collider.height = def.height * BABY_SCALE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::tests::sim;

    fn pig(world: &mut World) -> u32 {
        world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap()
    }

    #[test]
    fn only_the_right_food_works_and_only_once() {
        let mut world = sim();
        let kind = pig(&mut world);
        let mob = mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 80.0, 5.5)).unwrap();
        assert!(!feed(&mut world, mob, "core:stone"));
        assert!(feed(&mut world, mob, "core:carrot"));
        assert!(world.has::<Love>(mob));
        assert!(!feed(&mut world, mob, "core:carrot"), "déjà en rut");
    }

    #[test]
    fn two_fed_animals_make_a_baby_that_grows_up() {
        let mut world = sim();
        let kind = pig(&mut world);
        let a = mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 80.0, 5.5)).unwrap();
        let b = mobs::spawn_mob(&mut world, kind, Vec3::new(6.2, 80.0, 5.5)).unwrap();
        assert!(feed(&mut world, a, "core:carrot") && feed(&mut world, b, "core:carrot"));
        update(&mut world);
        world.apply_commands();
        assert_eq!(mobs::census(&world)[&kind], 3, "un petit est né");
        assert!(world.has::<BreedCooldown>(a) && world.has::<BreedCooldown>(b));
        assert!(!world.has::<Love>(a));
        assert!(!feed(&mut world, a, "core:carrot"), "elle se repose");
        // Le petit : on le retrouve, petit, puis adulte.
        let mut babies = Vec::new();
        world
            .query::<(&Baby,)>()
            .for_each(|entity, _| babies.push(entity));
        assert_eq!(babies.len(), 1);
        let baby = babies[0];
        let adult = world.get::<Collider>(a).unwrap().height;
        assert!(world.get::<Collider>(baby).unwrap().height < adult);
        world.get_mut::<Baby>(baby).unwrap().age_left = 0.0;
        update(&mut world);
        world.apply_commands();
        assert!(!world.has::<Baby>(baby));
        assert_eq!(world.get::<Collider>(baby).unwrap().height, adult);
    }

    #[test]
    fn sheep_and_chickens_breed_with_wheat_and_keep_their_own_species() {
        let mut world = sim();
        for key in ["core:sheep", "core:chicken"] {
            let kind = world.resource::<Arc<MobRegistry>>().id(key).unwrap();
            let a = mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 80.0, 5.5)).unwrap();
            let b = mobs::spawn_mob(&mut world, kind, Vec3::new(6.0, 80.0, 5.5)).unwrap();
            assert!(!feed(&mut world, a, "core:stone"));
            assert!(feed(&mut world, a, "core:wheat") && feed(&mut world, b, "core:wheat"));
            update(&mut world);
            world.apply_commands();
            let mut babies = Vec::new();
            world
                .query::<(&Baby, &Mob, &Collider)>()
                .for_each(|_, (_, mob, c)| {
                    if mob.kind == kind {
                        babies.push(c.height);
                    }
                });
            assert_eq!(babies.len(), 1);
            assert!(babies[0] < world.get::<Collider>(a).unwrap().height);
            assert_eq!(mobs::census(&world)[&kind], 3);
        }
    }

    #[test]
    fn lamb_inherits_a_parent_coat_and_keeps_it_when_growing_up() {
        use crate::sheep_colors::SheepCoat;
        let mut world = sim();
        let kind = world
            .resource::<Arc<MobRegistry>>()
            .id("core:sheep")
            .unwrap();
        let parents = [SheepCoat(3), SheepCoat(15)];
        let a = mobs::spawn_mob_with_coat(
            &mut world,
            kind,
            Vec3::new(5.5, 80.0, 5.5),
            Some(parents[0]),
        )
        .unwrap();
        let b = mobs::spawn_mob_with_coat(
            &mut world,
            kind,
            Vec3::new(6.0, 80.0, 5.5),
            Some(parents[1]),
        )
        .unwrap();
        assert!(feed(&mut world, a, "core:wheat") && feed(&mut world, b, "core:wheat"));
        update(&mut world);
        world.apply_commands();
        let mut lambs = Vec::new();
        world
            .query::<(&Baby, &SheepCoat)>()
            .for_each(|e, (_, coat)| lambs.push((e, *coat)));
        assert_eq!(lambs.len(), 1);
        let (lamb, coat) = lambs[0];
        assert!(parents.contains(&coat));
        world.get_mut::<Baby>(lamb).unwrap().age_left = 0.0;
        update(&mut world);
        world.apply_commands();
        assert!(!world.has::<Baby>(lamb));
        assert_eq!(*world.get::<SheepCoat>(lamb).unwrap(), coat);
    }

    #[test]
    fn a_baby_cannot_breed_and_love_wears_off() {
        let mut world = sim();
        let kind = pig(&mut world);
        let baby = mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 80.0, 5.5)).unwrap();
        world.insert(baby, Baby { age_left: 100.0 });
        assert!(!feed(&mut world, baby, "core:carrot"));
        let adult = mobs::spawn_mob(&mut world, kind, Vec3::new(9.5, 80.0, 5.5)).unwrap();
        assert!(feed(&mut world, adult, "core:carrot"));
        for _ in 0..(LOVE_SECONDS * 60.0) as usize + 2 {
            update(&mut world);
            world.apply_commands();
        }
        assert!(!world.has::<Love>(adult), "plus personne à aimer : fini");
        assert_eq!(mobs::census(&world)[&kind], 2);
    }
}
