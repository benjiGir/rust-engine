//! Notre ECS contre `hecs` : `cargo bench -p engine_ecs`.
//!
//! Trois mesures, sur 10 000 et 100 000 entités :
//! - itérer `(&mut Position, &Velocity)` et avancer chaque position ;
//! - créer toutes les entités ;
//! - les détruire toutes.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

#[derive(Clone, Copy)]
struct Position([f32; 3]);
#[derive(Clone, Copy)]
struct Velocity([f32; 3]);

fn ours(n: usize) -> engine_ecs::World {
    let mut world = engine_ecs::World::new();
    for i in 0..n {
        world.spawn((Position([i as f32, 0.0, 0.0]), Velocity([1.0, 0.5, 0.25])));
    }
    world
}

fn theirs(n: usize) -> hecs::World {
    let mut world = hecs::World::new();
    for i in 0..n {
        world.spawn((Position([i as f32, 0.0, 0.0]), Velocity([1.0, 0.5, 0.25])));
    }
    world
}

fn bench(c: &mut Criterion) {
    for n in [10_000, 100_000] {
        let mut group = c.benchmark_group(format!("{n} entités"));

        let world = ours(n);
        group.bench_function(BenchmarkId::new("itération", "maison"), |b| {
            b.iter(|| {
                world
                    .query::<(&mut Position, &Velocity)>()
                    .for_each(|_, (p, v)| {
                        for k in 0..3 {
                            p.0[k] += v.0[k];
                        }
                    })
            })
        });
        let mut world = theirs(n);
        group.bench_function(BenchmarkId::new("itération", "hecs"), |b| {
            b.iter(|| {
                for (p, v) in world.query_mut::<(&mut Position, &Velocity)>() {
                    for k in 0..3 {
                        p.0[k] += v.0[k];
                    }
                }
            })
        });

        group.bench_function(BenchmarkId::new("création", "maison"), |b| {
            b.iter(|| black_box(ours(n)))
        });
        group.bench_function(BenchmarkId::new("création", "hecs"), |b| {
            b.iter(|| black_box(theirs(n)))
        });

        group.bench_function(BenchmarkId::new("destruction", "maison"), |b| {
            b.iter_batched(
                || {
                    let world = ours(n);
                    let entities: Vec<_> = world.entities().collect();
                    (world, entities)
                },
                |(mut world, entities)| {
                    for e in entities {
                        world.despawn(e);
                    }
                    world
                },
                criterion::BatchSize::LargeInput,
            )
        });
        group.bench_function(BenchmarkId::new("destruction", "hecs"), |b| {
            b.iter_batched(
                || {
                    let world = theirs(n);
                    let entities: Vec<_> = world.iter().map(|e| e.entity()).collect();
                    (world, entities)
                },
                |(mut world, entities)| {
                    for e in entities {
                        let _ = world.despawn(e);
                    }
                    world
                },
                criterion::BatchSize::LargeInput,
            )
        });
        group.finish();
    }
}

/// Le scénario « usine » : des objets qui avancent sur des convoyeurs. Chaque objet a
/// une position, une vitesse et une étiquette ; le système les fait avancer et les
/// renvoie au début du tapis. Un pas de simulation complet.
#[derive(Clone, Copy)]
struct Item {
    belt_length: f32,
}

fn conveyors(c: &mut Criterion) {
    let mut group = c.benchmark_group("convoyeurs");
    for n in [10_000, 50_000] {
        let mut world = engine_ecs::World::new();
        for i in 0..n {
            world.spawn((
                Position([0.0, i as f32, 0.0]),
                Velocity([2.0 / 60.0, 0.0, 0.0]),
                Item {
                    belt_length: 16.0 + (i % 7) as f32,
                },
            ));
        }
        group.bench_function(BenchmarkId::new("pas de simulation", n), |b| {
            b.iter(|| {
                world
                    .query::<(&mut Position, &Velocity, &Item)>()
                    .for_each(|_, (p, v, item)| {
                        p.0[0] += v.0[0];
                        if p.0[0] > item.belt_length {
                            p.0[0] -= item.belt_length;
                        }
                    })
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench, conveyors);
criterion_main!(benches);
