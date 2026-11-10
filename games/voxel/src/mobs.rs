//! Les créatures : leur description (chargée depuis des fichiers, comme les blocs) et
//! les systèmes ECS qui les font vivre.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::shared::StreamerExt;
use anyhow::{Context as _, Result, bail};
use engine_ecs::{Entity, World};
use engine_render::CubeInstance;
use engine_voxel::{BlockRegistry, ChunkStreamer, PlayerBody, ray_aabb};
use glam::{IVec3, Vec3};
use serde::Deserialize;

use crate::components::{
    Collider, Flying, Health, Hurt, Mob, MoveIntent, PreviousPosition, Transform, Velocity, Wander,
};
use crate::events::{self, EntityDied, EntitySpawned, PlayerAttacked, Tick};

/// Au plus ce nombre de créatures autour du joueur.
/// Distance d'apparition (entre les deux) et de disparition, en blocs.
const DESPAWN_DISTANCE: f32 = 96.0;
/// Distance à laquelle une créature hostile repère le joueur.
const CHASE_DISTANCE: f32 = 14.0;

// ----- La description des créatures -----

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobPart {
    pub offset: (f32, f32, f32),
    pub size: (f32, f32, f32),
    pub color: (f32, f32, f32),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MobFileEntry {
    id: String,
    name: String,
    health: f32,
    speed: f32,
    size: (f32, f32),
    #[serde(default)]
    hostile: bool,
    #[serde(default)]
    damage: f32,
    #[serde(default)]
    hops: bool,
    spawn_on: Vec<String>,
    parts: Vec<MobPart>,
    #[serde(default)]
    drops: Vec<MobDrop>,
    #[serde(default)]
    xp: (u32, u32),
    // L'apparition (voir `spawning.rs`) : tout est facultatif.
    #[serde(default)]
    light: Option<(u8, u8)>,
    #[serde(default)]
    when: crate::spawning::When,
    #[serde(default)]
    biomes: Vec<String>,
    #[serde(default)]
    group: Option<(u32, u32)>,
    #[serde(default)]
    weight: Option<u32>,
    #[serde(default)]
    dissolves: Option<bool>,
}

/// Ce que lâche une créature à sa mort : un objet, un nombre (min, max), une chance.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobDrop {
    pub item: String,
    pub count: (u32, u32),
    #[serde(default = "always")]
    pub chance: f32,
}

fn always() -> f32 {
    1.0
}

#[derive(Debug, Clone)]
pub struct MobDef {
    pub key: String,
    pub name: String,
    pub health: f32,
    pub speed: f32,
    pub width: f32,
    pub height: f32,
    pub hostile: bool,
    pub damage: f32,
    pub hops: bool,
    /// Où et quand elle apparaît (voir `spawning.rs`).
    pub spawn: crate::spawning::SpawnRules,
    pub parts: Vec<MobPart>,
    /// Son butin (les objets sont vérifiés à la mort : ils peuvent venir d'un mod).
    pub drops: Vec<MobDrop>,
    /// L'expérience qu'elle rapporte (min, max).
    pub xp: (u32, u32),
}

/// Toutes les créatures connues. Leur numéro (`Mob::kind`) est leur place dans la liste.
#[derive(Debug, Default)]
pub struct MobRegistry {
    defs: Vec<MobDef>,
}

impl MobRegistry {
    /// `assets/core/entities.ron`, puis `mods/<nom>/entities.ron` par ordre alphabétique.
    /// `assets/core/entities.ron`, puis celui de chaque mod dans l'ordre de chargement.
    pub fn load(assets: &Path, mods: &[(String, PathBuf)], blocks: &BlockRegistry) -> Result<Self> {
        let mut registry = Self::default();
        let core = assets.join("core").join("entities.ron");
        if core.exists() {
            registry.add_file("core", &core, blocks)?;
        }
        for (namespace, dir) in mods {
            let file = dir.join("entities.ron");
            if file.exists() {
                registry.add_file(namespace, &file, blocks)?;
            }
        }
        Ok(registry)
    }

    fn add_file(&mut self, namespace: &str, path: &Path, blocks: &BlockRegistry) -> Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.add_source(namespace, &text, &path.display().to_string(), blocks)
    }

    pub fn add_source(
        &mut self,
        namespace: &str,
        text: &str,
        origin: &str,
        blocks: &BlockRegistry,
    ) -> Result<()> {
        let entries: Vec<MobFileEntry> =
            // `IMPLICIT_SOME` : un champ facultatif s'écrit `weight: 30`, sans `Some(...)`.
            ron::Options::default()
                .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
                .from_str(text)
                .with_context(|| format!("{origin} : fichier RON invalide"))?;
        for entry in entries {
            if !entry.id.starts_with(&format!("{namespace}:")) {
                bail!(
                    "{origin} : « {} » doit commencer par « {namespace}: »",
                    entry.id
                );
            }
            if self.defs.iter().any(|d| d.key == entry.id) {
                bail!(
                    "{origin} : la créature « {} » est définie deux fois",
                    entry.id
                );
            }
            let spawn_on = entry
                .spawn_on
                .iter()
                .map(|key| {
                    blocks
                        .id(key)
                        .with_context(|| format!("{origin} : bloc « {key} » inconnu"))
                })
                .collect::<Result<Vec<_>>>()?;
            self.defs.push(MobDef {
                key: entry.id,
                name: entry.name,
                health: entry.health,
                speed: entry.speed,
                width: entry.size.0,
                height: entry.size.1,
                hostile: entry.hostile,
                damage: entry.damage,
                hops: entry.hops,
                spawn: crate::spawning::SpawnRules {
                    on: spawn_on,
                    // Par défaut : les hostiles dans le noir, les autres au jour.
                    light: entry
                        .light
                        .unwrap_or(if entry.hostile { (0, 7) } else { (9, 15) }),
                    when: entry.when,
                    biomes: entry.biomes,
                    group: entry.group.unwrap_or((1, 1)),
                    weight: entry.weight.unwrap_or(10),
                    dissolves: entry.dissolves.unwrap_or(entry.hostile),
                },
                parts: entry.parts,
                drops: entry.drops,
                xp: entry.xp,
            });
        }
        Ok(())
    }

    pub fn by_key(&self, key: &str) -> Option<&MobDef> {
        self.defs.iter().find(|d| d.key == key)
    }

    pub fn get(&self, kind: u32) -> Option<&MobDef> {
        self.defs.get(kind as usize)
    }

    pub fn id(&self, key: &str) -> Option<u32> {
        self.defs
            .iter()
            .position(|d| d.key == key)
            .map(|i| i as u32)
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }
}

// ----- Les ressources partagées par les systèmes -----

/// La durée d'un pas de simulation (1/60 s).
#[derive(Debug, Clone, Copy)]
pub struct FixedDt(pub f32);

/// L'entité du joueur.
#[derive(Debug, Clone, Copy)]
pub struct PlayerEntity(pub Entity);

/// Un générateur pseudo-aléatoire pour l'IA et l'apparition (xorshift).
#[derive(Debug, Clone, Copy)]
pub struct GameRng(pub u64);

impl GameRng {
    pub fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Compteurs de la simulation.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimClock {
    pub tick: u64,
    pub spawn_timer: f32,
    pub dissolve_timer: f32,
}

// ----- Les systèmes -----

/// Le terrain autour de la boîte est-il chargé ? Sinon l'entité attend, gelée :
/// le monde n'y contient encore que de l'air et elle tomberait à travers le sol.
pub fn surroundings_ready(streamer: &ChunkStreamer, body: &PlayerBody) -> bool {
    let (min, max) = body.bounds();
    let first = engine_voxel::World::chunk_containing(min - Vec3::ONE);
    let last = engine_voxel::World::chunk_containing(max + Vec3::ONE);
    (first.x..=last.x).all(|x| {
        (first.y..=last.y)
            .all(|y| (first.z..=last.z).all(|z| streamer.is_generated(IVec3::new(x, y, z))))
    })
}

/// La physique de toutes les entités (joueur compris) : gravité, collisions, eau.
pub fn physics(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let player = world.resource::<PlayerEntity>().0;
    let center = world
        .get::<Transform>(player)
        .map(|t| t.position)
        .unwrap_or_default();
    let shared = world.streamer();
    let streamer = shared.borrow();
    world
        .query::<(
            &mut Transform,
            &mut PreviousPosition,
            &mut Velocity,
            &mut Collider,
            &MoveIntent,
        )>()
        .for_each(
            |entity, (transform, previous, velocity, collider, intent)| {
                previous.0 = transform.position;
                if world.has::<Flying>(entity)
                    || crate::spawning::asleep(world, entity, transform.position, center)
                {
                    return;
                }
                let mut body =
                    PlayerBody::with_size(transform.position, collider.width, collider.height);
                body.velocity = velocity.0;
                body.on_ground = collider.on_ground;
                collider.frozen = !surroundings_ready(&streamer, &body);
                if collider.frozen {
                    velocity.0 = Vec3::ZERO;
                    return;
                }
                body.step(streamer.world(), intent.walk, intent.jump, dt);
                transform.position = body.position;
                velocity.0 = body.velocity;
                collider.on_ground = body.on_ground;
                collider.in_liquid = body.in_liquid;
            },
        );
}

/// Crée une créature et prévient les abonnés (`EntitySpawned`).
pub fn spawn_mob(world: &mut World, kind: u32, position: Vec3) -> Option<Entity> {
    let def = world.resource::<Arc<MobRegistry>>().get(kind)?.clone();
    let entity = world.spawn((
        Transform { position, yaw: 0.0 },
        PreviousPosition(position),
        Velocity::default(),
        Collider::new(def.width, def.height),
        MoveIntent::default(),
        Health::full(def.health),
        Mob { kind },
        Wander::default(),
    ));
    events::send(
        world,
        EntitySpawned {
            entity,
            kind: def.key,
        },
    );
    Some(entity)
}

/// L'IA : errer, sauter les obstacles, poursuivre le joueur si hostile.
pub fn wander(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let player = world.resource::<PlayerEntity>().0;
    let target = world.get::<Transform>(player).map(|t| t.position);
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut rng = world.resource_mut::<GameRng>();
    world
        .query::<(
            &mut Transform,
            &mut Wander,
            &mut MoveIntent,
            &Mob,
            &Collider,
        )>()
        .for_each(|entity, (transform, wander, intent, mob, collider)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            // Loin du joueur : elle ne pense pas (zone de simulation).
            if target.is_some_and(|t| {
                crate::spawning::horizontal_distance(t, transform.position)
                    > crate::spawning::SIM_DISTANCE
            }) {
                intent.walk = Vec3::ZERO;
                intent.jump = false;
                return;
            }
            wander.timer -= dt;
            wander.attack_cooldown = (wander.attack_cooldown - dt).max(0.0);
            let chasing = def.hostile
                && target.is_some_and(|t| t.distance(transform.position) < CHASE_DISTANCE);
            if chasing {
                let to = target.unwrap_or_default() - transform.position;
                wander.direction = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
            } else if wander.timer <= 0.0 {
                // Une nouvelle idée : une direction au hasard, ou rester sur place.
                wander.timer = 2.0 + rng.next_f32() * 3.0;
                wander.direction = if rng.next_f32() < 0.3 {
                    Vec3::ZERO
                } else {
                    let angle = rng.next_f32() * std::f32::consts::TAU;
                    Vec3::new(angle.cos(), 0.0, angle.sin())
                };
            }
            // Un coup reçu : la créature s'arrête un instant.
            let stunned = world.get::<Hurt>(entity).is_some_and(|h| h.timer > 0.2);
            let direction = if stunned {
                Vec3::ZERO
            } else {
                wander.direction
            };
            intent.walk = direction * def.speed;
            if direction != Vec3::ZERO {
                // Se tourner doucement vers sa direction.
                let wanted = direction.z.atan2(direction.x);
                let delta = (wanted - transform.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                transform.yaw += delta * (dt * 8.0).min(1.0);
            }
            // Sauter : par bonds (slime), ou devant un obstacle d'un bloc.
            let ahead =
                transform.position + direction * (collider.width * 0.5 + 0.4) + Vec3::Y * 0.5;
            let blocked = streamer.world().is_solid(ahead.floor().as_ivec3());
            intent.jump = direction != Vec3::ZERO && (def.hops || blocked);
        });
}

/// Les créatures hostiles blessent le joueur au contact.
pub fn attack(world: &mut World) {
    let player = world.resource::<PlayerEntity>().0;
    let Some(target) = world.get::<Transform>(player).map(|t| t.position) else {
        return;
    };
    if world.has::<Flying>(player) {
        return;
    }
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    world
        .query::<(&Transform, &mut Wander, &Mob)>()
        .for_each(|_, (transform, wander, mob)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            let close = transform.position.distance(target) < def.width * 0.5 + 0.6;
            if !def.hostile || !close || wander.attack_cooldown > 0.0 {
                return;
            }
            wander.attack_cooldown = 1.0;
            // Le jeu applique le coup (armure, invulnérabilité) : voir `survival.rs`.
            events::send(world, PlayerAttacked { amount: def.damage });
        });
}

/// Les créatures sans vie meurent ; celles trop loin disparaissent ; les coups s'estompent.
pub fn cleanup(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let player = world.resource::<PlayerEntity>().0;
    let center = world
        .get::<Transform>(player)
        .map(|t| t.position)
        .unwrap_or_default();
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    world.query::<(&mut Hurt,)>().for_each(|entity, (hurt,)| {
        hurt.timer -= dt;
        if hurt.timer <= 0.0 {
            world.commands().remove::<Hurt>(entity);
        }
    });
    world
        .query::<(&Transform, &Health, &Mob)>()
        .for_each(|entity, (transform, health, mob)| {
            if health.current <= 0.0 {
                let kind = registry
                    .get(mob.kind)
                    .map(|d| d.key.clone())
                    .unwrap_or_default();
                events::send(
                    world,
                    EntityDied {
                        entity,
                        kind,
                        position: transform.position,
                    },
                );
                world.commands().despawn(entity);
            } else if transform.position.distance(center) > DESPAWN_DISTANCE {
                world.commands().despawn(entity);
            }
        });
}

/// Compte les pas de simulation et l'annonce (`Tick`).
pub fn tick(world: &mut World) {
    let tick = {
        let mut clock = world.resource_mut::<SimClock>();
        clock.tick += 1;
        clock.tick
    };
    events::send(world, Tick(tick));
}

// ----- Interaction et rendu -----

/// La créature la plus proche touchée par le rayon, avant `max` blocs.
pub fn hit_mob(world: &World, origin: Vec3, dir: Vec3, max: f32) -> Option<(Entity, f32)> {
    let mut best: Option<(Entity, f32)> = None;
    world
        .query::<(&Transform, &Collider, &Mob)>()
        .for_each(|entity, (transform, collider, _)| {
            let half = Vec3::new(collider.width / 2.0, 0.0, collider.width / 2.0);
            let min = transform.position - half;
            let max_corner = transform.position + half + Vec3::Y * collider.height;
            if let Some(t) = ray_aabb(origin, dir, min, max_corner)
                && t <= max
                && best.is_none_or(|(_, b)| t < b)
            {
                best = Some((entity, t));
            }
        });
    best
}

/// Un coup : des dégâts, un petit bond en arrière, un éclair rouge.
pub fn damage_mob(world: &mut World, entity: Entity, amount: f32, from: Vec3) {
    if let Some(mut health) = world.get_mut::<Health>(entity) {
        health.current -= amount;
    }
    if let Some(mut velocity) = world.get_mut::<Velocity>(entity) {
        let push = Vec3::new(from.x, 0.0, from.z).normalize_or_zero() * 3.0;
        velocity.0 = push + Vec3::Y * 5.0;
    }
    world.insert(entity, Hurt { timer: 0.4 });
}

/// Les boîtes à dessiner pour toutes les créatures, interpolées entre deux pas.
pub fn instances(world: &World, alpha: f32) -> Vec<CubeInstance> {
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut out = Vec::new();
    world
        .query::<(&Transform, &PreviousPosition, &Mob)>()
        .for_each(|entity, (transform, previous, mob)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            let position = previous.0.lerp(transform.position, alpha);
            // La lumière au milieu de la créature, pour qu'elle soit sombre dans une grotte.
            let (sky, block) = streamer
                .world()
                .light((position + Vec3::Y * 0.5).floor().as_ivec3());
            let hurt = world.has::<Hurt>(entity);
            let (c, s) = (transform.yaw.cos(), transform.yaw.sin());
            for part in &def.parts {
                let (ox, oy, oz) = part.offset;
                let offset = Vec3::new(ox * c - oz * s, oy, ox * s + oz * c);
                let mut color = Vec3::new(part.color.0, part.color.1, part.color.2);
                if hurt {
                    color = color.lerp(Vec3::new(1.0, 0.1, 0.1), 0.6);
                }
                out.push(
                    CubeInstance::new(
                        position + offset,
                        Vec3::new(part.size.0, part.size.1, part.size.2),
                        color,
                    )
                    .with_yaw(transform.yaw)
                    .with_light(sky as f32 / 15.0, block as f32 / 15.0),
                );
            }
        });
    out
}

/// Nombre de créatures par type (pour l'inspecteur).
pub fn census(world: &World) -> HashMap<u32, usize> {
    let mut counts = HashMap::new();
    world.query::<(&Mob,)>().for_each(|_, (mob,)| {
        *counts.entry(mob.kind).or_default() += 1;
    });
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ecs::{EventReader, Events};
    use engine_voxel::{StreamingConfig, TerrainGenerator};

    fn blocks() -> Arc<BlockRegistry> {
        Arc::new(crate::test_registry())
    }

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn core_and_mod_mobs_load() {
        let registry = MobRegistry::load(
            &repo().join("assets"),
            &[("example".into(), repo().join("mods/example"))],
            &blocks(),
        )
        .unwrap();
        let pig = registry.get(registry.id("core:pig").unwrap()).unwrap();
        assert!(!pig.hostile && pig.parts.len() > 3);
        let slime = registry.get(registry.id("example:slime").unwrap()).unwrap();
        assert!(slime.hostile && slime.hops);
    }

    #[test]
    fn duplicate_or_foreign_mobs_are_refused() {
        let blocks = blocks();
        let mut registry = MobRegistry::default();
        let entry = |id: &str| {
            format!(
                r#"[(id: "{id}", name: "X", health: 1.0, speed: 1.0, size: (1.0, 1.0), spawn_on: [], parts: [])]"#
            )
        };
        registry
            .add_source("test", &entry("test:a"), "t", &blocks)
            .unwrap();
        assert!(
            registry
                .add_source("test", &entry("test:a"), "t", &blocks)
                .is_err()
        );
        assert!(
            registry
                .add_source("test", &entry("core:b"), "t", &blocks)
                .is_err()
        );
    }

    /// Un petit monde ECS avec un terrain chargé autour de l'origine.
    fn sim() -> World {
        let blocks = blocks();
        let mut streamer = ChunkStreamer::new(
            TerrainGenerator::with_registry(5, blocks.clone()).unwrap(),
            StreamingConfig {
                view_radius: 2,
                ..StreamingConfig::default()
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            streamer.update(Vec3::new(5.0, 40.0, 5.0));
            while streamer.next_event().is_some() {}
            if streamer.is_idle() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut world = World::new();
        let mobs = MobRegistry::load(
            &repo().join("assets"),
            &[("example".into(), repo().join("mods/example"))],
            &blocks,
        )
        .unwrap();
        world.insert_resource(crate::shared::SharedStreamer(std::rc::Rc::new(
            std::cell::RefCell::new(streamer),
        )));
        world.insert_resource(Arc::new(mobs));
        world.insert_resource(FixedDt(1.0 / 60.0));
        world.insert_resource(GameRng(7));
        world.insert_resource(SimClock::default());
        world.insert_resource(crate::spawning::Sky::default());
        events::register(&mut world);
        let player = world.spawn((Transform::default(), Flying));
        world.insert_resource(PlayerEntity(player));
        world
    }

    #[test]
    fn a_mob_falls_and_lands_without_going_through_the_ground() {
        let mut world = sim();
        // Une colonne de terre ferme près de l'origine (pas un lac : le cochon y flotterait).
        let (x, z) = world
            .streamer()
            .borrow()
            .generator()
            .find_column(5, 5, 2, 48, |c| c.height > engine_voxel::SEA_LEVEL + 1)
            .expect("de la terre près de l'origine");
        let ground = world.streamer().borrow().generator().column(x, z).height;
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let mob = spawn_mob(
            &mut world,
            pig,
            Vec3::new(x as f32 + 0.5, ground as f32 + 10.0, z as f32 + 0.5),
        )
        .unwrap();
        for _ in 0..240 {
            physics(&mut world);
        }
        let position = world.get::<Transform>(mob).unwrap().position;
        let collider = *world.get::<Collider>(mob).unwrap();
        assert!(collider.on_ground);
        // Les grottes peuvent percer la surface : le sol est au plus à la hauteur du terrain.
        assert!(
            position.y <= ground as f32 + 1.01 && position.y > 1.0,
            "y = {}",
            position.y
        );
        assert!(
            world
                .streamer()
                .borrow()
                .world()
                .is_solid((position - Vec3::Y * 0.5).floor().as_ivec3())
        );
    }

    #[test]
    fn far_mobs_sleep_outside_the_simulation_zone() {
        let mut world = sim();
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        // Le joueur à l'origine ; le cochon à 60 blocs, dans le terrain chargé, en l'air.
        let mob = spawn_mob(&mut world, pig, Vec3::new(60.5, 120.0, 5.5)).unwrap();
        for _ in 0..30 {
            physics(&mut world);
        }
        assert_eq!(
            world.get::<Transform>(mob).unwrap().position.y,
            120.0,
            "endormi"
        );
        // Le joueur s'approche : il tombe.
        let player = world.resource::<PlayerEntity>().0;
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(50.0, 80.0, 5.0);
        for _ in 0..30 {
            physics(&mut world);
        }
        assert!(
            world.get::<Transform>(mob).unwrap().position.y < 119.0,
            "réveillé"
        );
    }

    /// Le jour, des cochons ; la nuit, des slimes (le mod d'exemple), jamais l'inverse.
    #[test]
    fn day_spawns_animals_and_night_spawns_monsters() {
        let census = |world: &World| {
            let registry = world.resource::<Arc<MobRegistry>>().clone();
            let mut found = (0, 0);
            world.query::<(&Mob,)>().for_each(|_, (mob,)| {
                if registry.get(mob.kind).is_some_and(|d| d.hostile) {
                    found.0 += 1;
                } else {
                    found.1 += 1;
                }
            });
            found
        };
        for night in [false, true] {
            let mut world = sim();
            let player = world.resource::<PlayerEntity>().0;
            world.get_mut::<Transform>(player).unwrap().position = Vec3::new(5.0, 70.0, 5.0);
            *world.resource_mut::<crate::spawning::Sky>() = crate::spawning::Sky {
                daylight: if night { 0.1 } else { 1.0 },
                night,
            };
            for _ in 0..400 {
                world.resource_mut::<SimClock>().spawn_timer = 0.0;
                crate::spawning::spawn(&mut world);
                world.apply_commands();
            }
            let (hostiles, passives) = census(&world);
            if night {
                assert!(
                    hostiles > 0 && passives == 0,
                    "nuit : {hostiles} / {passives}"
                );
            } else {
                assert!(
                    passives > 0 && hostiles == 0,
                    "jour : {hostiles} / {passives}"
                );
            }
        }
    }

    #[test]
    fn night_creatures_dissolve_in_the_sun() {
        let mut world = sim();
        let registry = world.resource::<Arc<MobRegistry>>().clone();
        let slime = registry.id("example:slime").unwrap();
        let pig = registry.id("core:pig").unwrap();
        // Dehors, en plein jour : au-dessus de la plus haute colonne, la lumière du ciel.
        let monster = spawn_mob(&mut world, slime, Vec3::new(5.5, 150.0, 5.5)).unwrap();
        let animal = spawn_mob(&mut world, pig, Vec3::new(6.5, 150.0, 5.5)).unwrap();
        for _ in 0..40 {
            world.resource_mut::<SimClock>().dissolve_timer = 0.0;
            crate::spawning::dissolve(&mut world);
            world.apply_commands();
        }
        assert!(
            world.get::<Transform>(monster).is_none(),
            "le slime s'est dissous"
        );
        assert!(world.get::<Transform>(animal).is_some(), "le cochon reste");
    }

    #[test]
    fn entities_wait_frozen_until_the_terrain_is_loaded() {
        let mut world = sim();
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        // Loin du terrain chargé (rayon de 2 chunks autour de l'origine), mais près du
        // joueur : dans la zone de simulation, elle devrait bouger.
        let player = world.resource::<PlayerEntity>().0;
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(5002.0, 80.0, 5.0);
        let mob = spawn_mob(&mut world, pig, Vec3::new(5000.5, 80.0, 5.5)).unwrap();
        for _ in 0..60 {
            physics(&mut world);
        }
        assert!(world.get::<Collider>(mob).unwrap().frozen);
        assert_eq!(
            world.get::<Transform>(mob).unwrap().position.y,
            80.0,
            "n'est pas tombé"
        );
    }

    #[test]
    fn hitting_a_mob_kills_it_and_announces_its_death() {
        let mut world = sim();
        let mut reader: EventReader = world.resource::<Events<EntityDied>>().reader();
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let mob = spawn_mob(&mut world, pig, Vec3::new(5.5, 60.0, 5.5)).unwrap();
        // Le rayon le touche.
        let (hit, _) = hit_mob(&world, Vec3::new(0.0, 60.4, 5.5), Vec3::X, 10.0).unwrap();
        assert_eq!(hit, mob);
        for _ in 0..3 {
            damage_mob(&mut world, mob, 4.0, Vec3::X);
        }
        cleanup(&mut world);
        world.apply_commands();
        assert!(!world.is_alive(mob));
        let deaths: Vec<_> = reader
            .read(&world.resource::<Events<EntityDied>>())
            .cloned()
            .collect();
        assert_eq!(
            deaths,
            [EntityDied {
                entity: mob,
                kind: "core:pig".into(),
                position: deaths[0].position,
            }]
        );
    }
}
