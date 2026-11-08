//! `engine_ecs` : un ECS (« Entity Component System ») écrit à la main.
//!
//! - Une **entité** est un numéro ([`Entity`]), rien de plus.
//! - Un **composant** est une donnée attachée à une entité : `Position`, `Health`...
//!   N'importe quel type Rust peut en être un.
//! - Un **système** est une fonction qui agit sur toutes les entités ayant certains
//!   composants : « pour chaque (Position, Velocity), avancer ».
//!
//! Comparé aux `MonoBehaviour` de Unity : pas d'héritage, pas d'objet « cochon » qui
//! contient tout. Les données de même type sont rangées ensemble (rapide à parcourir),
//! et le comportement vit dans les systèmes. Ajouter un comportement (un mod !) revient
//! à ajouter un composant et un système, sans toucher au reste.

mod entity;
mod events;
mod query;
mod schedule;
mod storage;
mod world;

pub use entity::{Entities, Entity};
pub use events::{EventReader, Events};
pub use query::{Query, QueryParam};
pub use schedule::{Schedule, Stage, System, SystemTiming};
pub use storage::{ComponentStorage, SparseSet};
pub use world::{Bundle, Commands, World};

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Position(f32, f32);
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Velocity(f32, f32);
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Health(i32);

    fn sample() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        let a = world.spawn((Position(0.0, 0.0), Velocity(1.0, 0.0)));
        let b = world.spawn((Position(5.0, 5.0),));
        let c = world.spawn((Position(1.0, 1.0), Velocity(0.0, 2.0), Health(10)));
        (world, a, b, c)
    }

    #[test]
    fn query_reads_and_writes_matching_entities() {
        let (world, a, b, c) = sample();
        world
            .query::<(&mut Position, &Velocity)>()
            .for_each(|_, (pos, vel)| {
                pos.0 += vel.0;
                pos.1 += vel.1;
            });
        assert_eq!(*world.get::<Position>(a).unwrap(), Position(1.0, 0.0));
        assert_eq!(
            *world.get::<Position>(b).unwrap(),
            Position(5.0, 5.0),
            "sans Velocity"
        );
        assert_eq!(*world.get::<Position>(c).unwrap(), Position(1.0, 3.0));
        assert_eq!(
            world.query::<(&Position, &Velocity, &Health)>().entities(),
            vec![c]
        );
        assert_eq!(world.query::<(&Position,)>().count(), 3);
        // Un type que personne n'a : la requête est simplement vide.
        assert_eq!(world.query::<(&String,)>().count(), 0);
    }

    #[test]
    #[should_panic(expected = "demandé deux fois")]
    fn double_mutable_access_is_refused() {
        let (world, ..) = sample();
        world.query::<(&mut Position, &Position)>();
    }

    #[test]
    fn despawned_entities_disappear_and_stale_ids_fail() {
        let (mut world, a, _, c) = sample();
        assert!(world.despawn(a));
        assert!(!world.is_alive(a));
        assert!(world.get::<Position>(a).is_none());
        // La case de `a` est réutilisée, avec une nouvelle génération.
        let d = world.spawn((Health(3),));
        assert_eq!(d.index, a.index);
        assert!(
            world.get::<Health>(a).is_none(),
            "l'ancien numéro ne voit pas la nouvelle entité"
        );
        assert!(world.insert(a, Health(99)).is_none());
        assert_eq!(*world.get::<Health>(d).unwrap(), Health(3));
        assert_eq!(world.component_names(c).len(), 3);
    }

    #[test]
    fn commands_are_applied_after_the_query() {
        let (mut world, a, b, c) = sample();
        // Pendant la requête, le monde est emprunté : on note ce qu'il faut faire.
        world.query::<(&Position,)>().for_each(|entity, (pos,)| {
            if pos.0 > 2.0 {
                world.commands().despawn(entity);
                world.commands().spawn((Health(1),));
            }
        });
        assert!(world.is_alive(b), "pas encore appliqué");
        world.apply_commands();
        assert!(!world.is_alive(b));
        assert!(world.is_alive(a) && world.is_alive(c));
        assert_eq!(world.query::<(&Health,)>().count(), 2);
    }

    #[test]
    fn resources_and_schedule() {
        let (mut world, ..) = sample();
        world.insert_resource(0u32);
        let mut schedule = Schedule::new();
        schedule
            .add(Stage::FixedUpdate, "compter", |world| {
                let n = world.query::<(&Velocity,)>().count() as u32;
                *world.resource_mut::<u32>() += n;
            })
            .add(Stage::Update, "doubler", |world| {
                *world.resource_mut::<u32>() *= 2;
            });
        schedule.run(Stage::FixedUpdate, &mut world);
        schedule.run(Stage::FixedUpdate, &mut world);
        schedule.run(Stage::Update, &mut world);
        assert_eq!(*world.resource::<u32>(), 8);
        assert_eq!(schedule.timings().len(), 2);
    }

    #[test]
    fn events_live_two_frames_and_readers_see_each_once() {
        let mut events = Events::default();
        let mut reader = events.reader();
        events.send("cassé");
        assert_eq!(reader.read(&events).collect::<Vec<_>>(), [&"cassé"]);
        assert_eq!(reader.read(&events).count(), 0, "déjà lu");

        // Un lecteur en retard d'une image voit encore l'événement.
        let mut late = EventReader::default();
        events.update();
        events.send("posé");
        assert_eq!(late.read(&events).collect::<Vec<_>>(), [&"cassé", &"posé"]);
        // Deux images plus tard, tout a disparu.
        events.update();
        events.update();
        assert!(events.is_empty());
    }
}
