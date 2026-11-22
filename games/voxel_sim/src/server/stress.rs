//! Le mode `VOXEL_STRESS=N` : un test de charge dans le vrai jeu.
//!
//! Dès que le terrain autour du joueur est chargé, on construit :
//! - **N horloges** dans le ciel, au rythme le plus rapide (un basculement par tick de
//!   machines, le pire cas) : elles mesurent le coût des machines ;
//! - **N objets sur des tapis roulants**, des entités ECS (position, vitesse, tapis)
//!   qu'un système fait avancer à chaque pas : ils mesurent le coût de l'ECS.
//!
//! Toutes les 5 secondes, les temps moyens et maximaux sont écrits dans les logs.

use std::time::{Duration, Instant};

use engine_ecs::World;
use engine_inspect::Inspect;
use glam::Vec3;

use crate::components::{Transform, Velocity};
use crate::modding::{ModSystem, build_stress_factory, ms};

/// Un objet posé sur un tapis : il avance puis revient au début.
#[derive(Debug, Clone, Copy, PartialEq, Default, Inspect)]
pub struct ConveyorItem {
    pub start: f32,
    pub length: f32,
}

/// Le système des tapis : un pas de simulation pour tous les objets.
pub fn conveyors(world: &mut World) {
    world
        .query::<(&mut Transform, &Velocity, &ConveyorItem)>()
        .for_each(|_, (transform, velocity, item)| {
            transform.position += velocity.0;
            if transform.position.x > item.start + item.length {
                transform.position.x -= item.length;
            }
        });
}

pub struct StressTest {
    count: usize,
    built: bool,
    machine_ticks: Vec<Duration>,
    ecs_steps: Vec<Duration>,
    last_report: Instant,
}

impl StressTest {
    pub fn from_env() -> Option<Self> {
        let count: usize = std::env::var("VOXEL_STRESS").ok()?.parse().ok()?;
        log::info!("test de charge : {count} machines et {count} objets sur tapis");
        Some(Self {
            count,
            built: false,
            machine_ticks: Vec::new(),
            ecs_steps: Vec::new(),
            last_report: Instant::now(),
        })
    }

    /// Construit l'usine une fois le terrain prêt (`ready`).
    pub fn build_if_ready(
        &mut self,
        ready: bool,
        ecs: &mut World,
        mods: &mut ModSystem,
        around: Vec3,
    ) {
        if self.built || !ready {
            return;
        }
        self.built = true;
        let start = Instant::now();
        let placed = build_stress_factory(mods, self.count, around.floor().as_ivec3());
        let machines = start.elapsed();
        let start = Instant::now();
        for i in 0..self.count {
            let lane = (i / 100) as f32;
            ecs.spawn((
                Transform {
                    position: Vec3::new(
                        around.x + (i % 100) as f32 * 0.3,
                        around.y + 40.0,
                        around.z + lane,
                    ),
                    yaw: 0.0,
                },
                Velocity(Vec3::new(2.0 / 60.0, 0.0, 0.0)),
                ConveyorItem {
                    start: around.x,
                    length: 30.0,
                },
            ));
        }
        log::info!(
            "usine construite : {placed} horloges en {:.1} ms, {} objets en {:.1} ms",
            ms(machines),
            self.count,
            ms(start.elapsed())
        );
    }

    /// Note les temps d'un pas : celui du tick de machines (s'il y en a eu un) et celui
    /// du système des tapis.
    pub fn record(&mut self, machine_tick: Option<Duration>, ecs_step: Duration) {
        if !self.built {
            return;
        }
        self.machine_ticks.extend(machine_tick);
        self.ecs_steps.push(ecs_step);
        if self.last_report.elapsed() < Duration::from_secs(5) {
            return;
        }
        let stats = |samples: &[Duration]| {
            let max = samples.iter().max().copied().unwrap_or_default();
            let mean = samples.iter().sum::<Duration>() / samples.len().max(1) as u32;
            (ms(mean), ms(max))
        };
        let (machine_mean, machine_max) = stats(&self.machine_ticks);
        let (ecs_mean, ecs_max) = stats(&self.ecs_steps);
        log::info!(
            "charge ({} machines, {} objets) : tick machines {machine_mean:.2} ms (max {machine_max:.2}), \
             pas des tapis {ecs_mean:.3} ms (max {ecs_max:.3})",
            self.count,
            self.count
        );
        self.machine_ticks.clear();
        self.ecs_steps.clear();
        self.last_report = Instant::now();
    }
}
