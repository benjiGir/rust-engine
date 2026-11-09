//! Les systèmes : le code qui agit sur le monde, regroupé par étapes.
//!
//! Un système est n'importe quel type qui implémente [`System`]. Le planning garde des
//! `Box<dyn System>` : des objets traits, de types différents (une closure, une struct
//! avec un état...) rangés dans la même liste. L'appel `system.run(world)` passe par une
//! table de méthodes (« dispatch dynamique ») : un tout petit coût, une grande souplesse.
//! C'est ce qui permettra aux mods d'ajouter leurs propres systèmes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::world::World;

/// Quand un système tourne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// À pas fixe (60 Hz) : physique, IA, simulation.
    FixedUpdate,
    /// Une fois par image : entrées, animations, ce qui dépend du temps réel.
    Update,
}

pub trait System {
    fn name(&self) -> &str;
    fn run(&mut self, world: &mut World);
}

/// Une fonction (ou une closure) avec un nom devient un système.
struct FnSystem<F> {
    name: String,
    f: F,
}

impl<F: FnMut(&mut World)> System for FnSystem<F> {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, world: &mut World) {
        (self.f)(world);
    }
}

/// Le temps passé par un système lors de son dernier passage (pour l'inspecteur).
#[derive(Debug, Clone)]
pub struct SystemTiming {
    pub name: String,
    pub stage: Stage,
    pub duration: Duration,
}

#[derive(Default)]
pub struct Schedule {
    stages: HashMap<Stage, Vec<Box<dyn System>>>,
    timings: Vec<SystemTiming>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un système écrit comme une fonction : `schedule.add(Stage::Update, "gravité", gravity)`.
    pub fn add(
        &mut self,
        stage: Stage,
        name: &str,
        f: impl FnMut(&mut World) + 'static,
    ) -> &mut Self {
        self.add_system(
            stage,
            Box::new(FnSystem {
                name: name.to_string(),
                f,
            }),
        )
    }

    pub fn add_system(&mut self, stage: Stage, system: Box<dyn System>) -> &mut Self {
        self.stages.entry(stage).or_default().push(system);
        self
    }

    /// Lance les systèmes d'une étape, dans l'ordre d'ajout. Les commandes différées de
    /// chaque système sont appliquées avant le suivant : il voit leurs effets.
    pub fn run(&mut self, stage: Stage, world: &mut World) {
        let Some(systems) = self.stages.get_mut(&stage) else {
            return;
        };
        self.timings.retain(|t| t.stage != stage);
        for system in systems {
            let _span = engine_trace::span_dyn(system.name());
            let start = Instant::now();
            system.run(world);
            world.apply_commands();
            self.timings.push(SystemTiming {
                name: system.name().to_string(),
                stage,
                duration: start.elapsed(),
            });
        }
    }

    pub fn timings(&self) -> &[SystemTiming] {
        &self.timings
    }
}
