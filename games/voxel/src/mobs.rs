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
    Collider, Flying, Health, Hurt, Impulse, Mob, MoveIntent, PreviousPosition, Transform,
    Velocity, Wander,
};
use crate::events::{self, EntityDied, EntitySpawned, PlayerAttacked, Tick};
use crate::navigation::{DangerBlocks, PathFollow, SEARCHES_PER_STEP, cell_of};
use crate::survival::{Senses, SurvivalRules, Vitals};

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
    #[serde(default)]
    ranged: Option<crate::projectiles::RangedDef>,
    #[serde(default)]
    breeds: Option<crate::breeding::BreedingDef>,
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
    /// Une attaque à distance (voir `projectiles.rs`).
    pub ranged: Option<crate::projectiles::RangedDef>,
    /// Comment elle se reproduit (voir `breeding.rs`) ; `None` : elle ne le fait pas.
    pub breeds: Option<crate::breeding::BreedingDef>,
}

/// Toutes les créatures connues. Leur numéro (`Mob::kind`) est leur place dans la liste.
#[derive(Debug, Default, Clone)]
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
                ranged: entry.ranged,
                breeds: entry.breeds,
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
                body.impulse = world.get::<Impulse>(entity).map_or(Vec3::ZERO, |i| i.0);
                collider.frozen = !surroundings_ready(&streamer, &body);
                if collider.frozen {
                    velocity.0 = Vec3::ZERO;
                    return;
                }
                body.step(streamer.world(), intent.walk, intent.jump, dt);
                transform.position = body.position;
                velocity.0 = body.velocity;
                if let Some(mut impulse) = world.get_mut::<Impulse>(entity) {
                    impulse.0 = body.impulse;
                }
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
    // À part : un « bundle » prend au plus 8 composants.
    world.insert(entity, PathFollow::default());
    let rules = world.resource::<Arc<SurvivalRules>>().clone();
    world.insert(entity, MobVitals(Vitals::new(&rules)));
    events::send(
        world,
        EntitySpawned {
            entity,
            kind: def.key,
        },
    );
    Some(entity)
}

/// L'IA : chacune choisit un but (poursuivre le joueur, fuir après un coup, errer vers
/// un point proche, ou rester là), puis suit un chemin vers lui (`navigation.rs`).
pub fn wander(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let player = world.resource::<PlayerEntity>().0;
    let target = world.get::<Transform>(player).map(|t| t.position);
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let danger = world.resource::<Arc<DangerBlocks>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut rng = world.resource_mut::<GameRng>();
    // Au plus tant de recherches de chemin à ce pas, pour toutes les créatures.
    let mut searches = SEARCHES_PER_STEP;
    world
        .query::<(
            &mut Transform,
            &mut Wander,
            &mut MoveIntent,
            &Mob,
            &Collider,
            &mut PathFollow,
        )>()
        .for_each(
            |entity, (transform, wander, intent, mob, collider, follow)| {
                let Some(def) = registry.get(mob.kind) else {
                    return;
                };
                let position = transform.position;
                // Loin du joueur : elle ne pense pas (zone de simulation).
                if target.is_some_and(|t| {
                    crate::spawning::horizontal_distance(t, position)
                        > crate::spawning::SIM_DISTANCE
                }) {
                    intent.walk = Vec3::ZERO;
                    intent.jump = false;
                    return;
                }
                wander.timer -= dt;
                wander.panic = (wander.panic - dt).max(0.0);
                wander.attack_cooldown = (wander.attack_cooldown - dt).max(0.0);
                let chasing =
                    def.hostile && target.is_some_and(|t| t.distance(position) < CHASE_DISTANCE);

                // Le but.
                let goal = if chasing {
                    target.map(cell_of)
                } else if wander.panic > 0.0
                    && let Some(from) = target
                {
                    // Fuir : à l'opposé du joueur, à 8 blocs.
                    let away = Vec3::new(position.x - from.x, 0.0, position.z - from.z)
                        .normalize_or_zero();
                    Some(cell_of(position + away * 8.0))
                } else {
                    // Un habitant reste près de chez lui : ses idées partent de sa maison.
                    let home = world.get::<crate::components::Home>(entity).map(|h| h.0);
                    if wander.timer <= 0.0 {
                        // Une nouvelle idée : un point proche, ou rester sur place.
                        wander.timer = 3.0 + rng.next_f32() * 4.0;
                        follow.wander_goal = if rng.next_f32() < 0.3 {
                            None
                        } else {
                            let angle = rng.next_f32() * std::f32::consts::TAU;
                            let distance = 3.0 + rng.next_f32() * 5.0;
                            let (center, distance) = match home {
                                Some(h) => (h, distance.min(5.0)),
                                None => (position, distance),
                            };
                            Some(cell_of(
                                center + Vec3::new(angle.cos(), 0.0, angle.sin()) * distance,
                            ))
                        };
                    }
                    // Trop loin de chez lui : il rentre.
                    if let Some(h) = home
                        && crate::spawning::horizontal_distance(h, position) > 10.0
                    {
                        follow.wander_goal = Some(cell_of(h));
                    }
                    follow.wander_goal
                };
                match goal {
                    Some(goal) if searches > 0 && follow.wants(goal, position) => {
                        let _span = engine_trace::span("créatures : chemin");
                        follow.plan(streamer.world(), &danger, position, collider.height, goal);
                        searches -= 1;
                    }
                    Some(_) => {}
                    None => follow.clear(),
                }

                // Marcher : le long du chemin ; tout près du joueur, droit sur lui.
                let close = target.filter(|t| chasing && t.distance(position) < 2.0);
                let (mut direction, path_jump) = match (follow.steer(position, dt), close) {
                    (_, Some(t)) => (
                        Vec3::new(t.x - position.x, 0.0, t.z - position.z).normalize_or_zero(),
                        false,
                    ),
                    (Some(step), None) => step,
                    (None, None) => (Vec3::ZERO, false),
                };
                // Un coup reçu : elle s'arrête un instant (le recul).
                if world.get::<Hurt>(entity).is_some_and(|h| h.timer > 0.2) {
                    direction = Vec3::ZERO;
                }
                // Une créature qui tire s'arrête à portée, face au joueur.
                if chasing && let (Some(ranged), Some(t)) = (def.ranged, target) {
                    let eye = crate::projectiles::eye_of(position, collider);
                    let aim_at = t + Vec3::Y * 1.2;
                    if crate::projectiles::holds_position(&ranged, streamer.world(), eye, aim_at) {
                        direction = Vec3::ZERO;
                        let to = aim_at - position;
                        let wanted = to.z.atan2(to.x);
                        let delta = (wanted - transform.yaw + std::f32::consts::PI)
                            .rem_euclid(std::f32::consts::TAU)
                            - std::f32::consts::PI;
                        transform.yaw += delta * (dt * 8.0).min(1.0);
                    }
                }
                let speed = if wander.panic > 0.0 {
                    def.speed * 1.5
                } else {
                    def.speed
                };
                intent.walk = direction * speed;
                wander.direction = direction;
                if direction != Vec3::ZERO {
                    // Se tourner doucement vers sa direction.
                    let wanted = direction.z.atan2(direction.x);
                    let delta = (wanted - transform.yaw + std::f32::consts::PI)
                        .rem_euclid(std::f32::consts::TAU)
                        - std::f32::consts::PI;
                    transform.yaw += delta * (dt * 8.0).min(1.0);
                }
                // Sauter : par bonds (slime), vers une case plus haute, ou devant un obstacle.
                let ahead = position + direction * (collider.width * 0.5 + 0.4) + Vec3::Y * 0.5;
                let blocked = streamer.world().is_solid(ahead.floor().as_ivec3());
                intent.jump = direction != Vec3::ZERO && (def.hops || path_jump || blocked);
            },
        );
}

/// L'état de survie d'une créature : chute, lave, feu (les mêmes règles que le joueur,
/// sans la noyade : les animaux nagent à la surface).
#[derive(Debug, Clone)]
pub struct MobVitals(pub Vitals);

/// Les créatures se blessent comme le joueur : une chute trop haute, la lave, le feu.
pub fn environment(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let player = world.resource::<PlayerEntity>().0;
    let center = world
        .get::<Transform>(player)
        .map(|t| t.position)
        .unwrap_or_default();
    let rules = world.resource::<Arc<SurvivalRules>>().clone();
    let danger = world.resource::<Arc<DangerBlocks>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut hurt = Vec::new();
    world
        .query::<(&Transform, &Collider, &mut MobVitals)>()
        .for_each(|entity, (transform, collider, vitals)| {
            if crate::spawning::asleep(world, entity, transform.position, center) {
                return;
            }
            let voxels = streamer.world();
            let at = |dy: f32| voxels.block((transform.position + Vec3::Y * dy).floor().as_ivec3());
            let blocks = [at(0.2), at(collider.height * 0.6)];
            let fluid_damage = blocks.iter().map(|b| danger.damage(*b)).fold(0.0, f32::max);
            let in_water = blocks
                .iter()
                .any(|b| voxels.registry().def(*b).liquid && danger.damage(*b) <= 0.0);
            let senses = Senses {
                feet_y: transform.position.y,
                on_ground: collider.on_ground,
                in_liquid: collider.in_liquid,
                in_water,
                head_under: false,
                fluid_damage,
                flying: false,
            };
            for (amount, cause) in vitals.0.step(&rules, senses, dt) {
                if vitals.0.hurt(&rules, amount, cause, 0).is_some() {
                    hurt.push((entity, amount));
                }
            }
        });
    drop(streamer);
    for (entity, amount) in hurt {
        if let Some(mut health) = world.get_mut::<Health>(entity) {
            health.current -= amount;
        }
        world.insert(entity, Hurt { timer: 0.4 });
    }
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
            events::send(
                world,
                PlayerAttacked {
                    amount: def.damage,
                    from: transform.position,
                },
            );
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
/// Renvoie `false` si le coup tombe pendant son invulnérabilité (juste après un autre
/// coup) : cliquer très vite ne tue pas plus vite.
pub fn damage_mob(world: &mut World, entity: Entity, amount: f32, from: Vec3) -> bool {
    if world.get::<Hurt>(entity).is_some_and(|h| h.timer > 0.0) {
        return false;
    }
    if let Some(mut health) = world.get_mut::<Health>(entity) {
        health.current -= amount;
    }
    // Le recul : un petit bond, et une poussée qui s'amortit (voir `PlayerBody::impulse`).
    if let Some(mut velocity) = world.get_mut::<Velocity>(entity) {
        velocity.0.y = 5.0;
    }
    world.insert(
        entity,
        Impulse(Vec3::new(from.x, 0.0, from.z).normalize_or_zero() * 5.0),
    );
    world.insert(entity, Hurt { timer: 0.4 });
    // Un animal fuit ; une créature hostile, elle, ne recule pas.
    let hostile = world
        .get::<Mob>(entity)
        .and_then(|m| {
            world
                .resource::<Arc<MobRegistry>>()
                .get(m.kind)
                .map(|d| d.hostile)
        })
        .unwrap_or(true);
    if !hostile && let Some(mut wander) = world.get_mut::<Wander>(entity) {
        wander.panic = 4.0;
    }
    true
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
            let scale = if world.has::<crate::components::Baby>(entity) {
                crate::breeding::BABY_SCALE
            } else {
                1.0
            };
            let (c, s) = (transform.yaw.cos(), transform.yaw.sin());
            for part in &def.parts {
                let (ox, oy, oz) = part.offset;
                let offset = Vec3::new(ox * c - oz * s, oy, ox * s + oz * c) * scale;
                let mut color = Vec3::new(part.color.0, part.color.1, part.color.2);
                if hurt {
                    color = color.lerp(Vec3::new(1.0, 0.1, 0.1), 0.6);
                }
                out.push(
                    CubeInstance::new(
                        position + offset,
                        Vec3::new(part.size.0, part.size.1, part.size.2) * scale,
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
pub(crate) mod tests {
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
    pub(crate) fn sim() -> World {
        sim_at(Vec3::new(5.0, 40.0, 5.0), 5)
    }

    /// Le même monde, chargé autour de `center`, avec cette graine.
    pub(crate) fn sim_at(center: Vec3, seed: u32) -> World {
        let blocks = blocks();
        let mut streamer = ChunkStreamer::new(
            TerrainGenerator::with_registry(seed, blocks.clone()).unwrap(),
            StreamingConfig {
                view_radius: 2,
                ..StreamingConfig::default()
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            streamer.update(center);
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
        world.insert_resource(Arc::new(DangerBlocks::default()));
        world.insert_resource(Arc::new(SurvivalRules::default()));
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
    fn day_spawns_animals_and_monsters_only_in_the_dark() {
        // (hostiles au jour, hostiles dans le noir, animaux) : la lumière du ciel à
        // l'endroit où chacune est apparue, en plein jour.
        let census = |world: &World| {
            let registry = world.resource::<Arc<MobRegistry>>().clone();
            let shared = world.streamer();
            let streamer = shared.borrow();
            let mut found = (0, 0, 0);
            world
                .query::<(&Transform, &Mob)>()
                .for_each(|_, (transform, mob)| {
                    if registry.get(mob.kind).is_some_and(|d| d.hostile) {
                        let (sky, block) = streamer
                            .world()
                            .light((transform.position + Vec3::Y * 0.5).floor().as_ivec3());
                        if crate::spawning::effective_light(sky, block, 1.0) > 7 {
                            found.0 += 1;
                        } else {
                            found.1 += 1;
                        }
                    } else {
                        found.2 += 1;
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
            let (lit, dark, passives) = census(&world);
            if night {
                assert!(
                    lit + dark > 0 && passives == 0,
                    "nuit : {lit} + {dark} / {passives}"
                );
            } else {
                // Le jour : des animaux ; des hostiles seulement dans le noir (les grottes).
                assert!(
                    passives > 0 && lit == 0,
                    "jour : {lit} + {dark} / {passives}"
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

    /// Un slime, de l'autre côté d'un mur trop haut pour sauter : il en fait le tour
    /// pour atteindre le joueur (en ligne droite, il resterait collé au mur).
    #[test]
    fn hostile_mobs_walk_around_walls_to_reach_the_player() {
        let mut world = sim();
        let stone = blocks().id("core:stone").unwrap();
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
            // Une plateforme dans le ciel, et un mur de 3 blocs au milieu (z de -3 à 3).
            for x in -8..9 {
                for z in -8..9 {
                    streamer.set_block_raw(IVec3::new(x, 150, z), stone);
                }
            }
            for z in -3..4 {
                for y in 151..154 {
                    streamer.set_block_raw(IVec3::new(0, y, z), stone);
                }
            }
        }
        let player = world.resource::<PlayerEntity>().0;
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(4.5, 151.0, 0.5);
        let slime = world
            .resource::<Arc<MobRegistry>>()
            .id("example:slime")
            .unwrap();
        let mob = spawn_mob(&mut world, slime, Vec3::new(-3.5, 151.0, 0.5)).unwrap();
        let mut planned = false;
        for _ in 0..(20 * 60) {
            wander(&mut world);
            physics(&mut world);
            planned |= !world.get::<PathFollow>(mob).unwrap().remaining().is_empty();
            let at = world.get::<Transform>(mob).unwrap().position;
            if at.distance(Vec3::new(4.5, 151.0, 0.5)) < 2.0 {
                return;
            }
        }
        let at = world.get::<Transform>(mob).unwrap().position;
        panic!("pas arrivé (chemin trouvé : {planned}), il est en {at}");
    }

    #[test]
    fn mobs_get_hurt_by_falls_and_lava() {
        let mut world = sim();
        let blocks = blocks();
        let (stone, lava) = (
            blocks.id("core:stone").unwrap(),
            blocks.id("core:lava").unwrap(),
        );
        let mut danger = vec![0.0; blocks.len()];
        danger[lava.0 as usize] = 4.0;
        world.insert_resource(Arc::new(DangerBlocks(danger)));
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
            // Deux dalles dans le ciel ; sur la seconde, de la lave.
            for x in 0..3 {
                for z in 0..3 {
                    streamer.set_block_raw(IVec3::new(x, 140, z), stone);
                    streamer.set_block_raw(IVec3::new(x + 10, 140, z), stone);
                    streamer.set_block_raw(IVec3::new(x + 10, 141, z), lava);
                }
            }
        }
        let player = world.resource::<PlayerEntity>().0;
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(5.0, 141.0, 1.0);
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        // Lâché de 20 blocs : 17 de trop, plus que ses 10 points de vie.
        let falling = spawn_mob(&mut world, pig, Vec3::new(1.5, 161.0, 1.5)).unwrap();
        let burning = spawn_mob(&mut world, pig, Vec3::new(11.5, 141.0, 1.5)).unwrap();
        for _ in 0..(3 * 60) {
            physics(&mut world);
            environment(&mut world);
            world.apply_commands();
        }
        let health = |e| world.get::<Health>(e).unwrap().current;
        assert!(
            health(falling) <= 0.0,
            "la chute l'a tué : {}",
            health(falling)
        );
        assert!(
            health(burning) < 10.0,
            "la lave l'a brûlé : {}",
            health(burning)
        );
    }

    /// Un archer (une créature de test) s'arrête à portée et touche le joueur.
    #[test]
    fn ranged_mobs_stop_at_range_and_hit_the_player() {
        let mut world = sim();
        let stone = blocks().id("core:stone").unwrap();
        let mut registry = world.resource::<Arc<MobRegistry>>().as_ref().clone();
        registry
            .add_source(
                "core",
                r#"[(id: "core:archer", name: "Archer", health: 10.0, speed: 2.0, size: (0.6, 1.8),
                    hostile: true, spawn_on: [], parts: [],
                    ranged: (range: 12.0, interval: 1.0, speed: 16.0, damage: 3.0))]"#,
                "test",
                &blocks(),
            )
            .unwrap();
        let archer = registry.id("core:archer").unwrap();
        world.insert_resource(Arc::new(registry));
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
            for x in -10..10 {
                for z in -3..4 {
                    streamer.set_block_raw(IVec3::new(x, 150, z), stone);
                }
            }
        }
        // Le joueur au sol (pas en vol : on ne tire pas sur qui vole).
        let player = world.resource::<PlayerEntity>().0;
        world.remove::<Flying>(player);
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(6.5, 151.0, 0.5);
        // À 11 blocs : il poursuit (moins de 14) et il est à portée (12).
        let mob = spawn_mob(&mut world, archer, Vec3::new(-4.5, 151.0, 0.5)).unwrap();
        let mut reader = world.resource::<Events<PlayerAttacked>>().reader();
        let mut hits = 0;
        for _ in 0..(6 * 60) {
            wander(&mut world);
            physics(&mut world);
            crate::projectiles::shoot(&mut world);
            crate::projectiles::fly(&mut world);
            world.apply_commands();
            hits += reader
                .read(&world.resource::<Events<PlayerAttacked>>())
                .count();
            world.resource_mut::<Events<PlayerAttacked>>().update();
        }
        let at = world.get::<Transform>(mob).unwrap().position;
        assert!(hits >= 2, "au moins deux projectiles au but : {hits}");
        assert!(at.x < 6.5 - 3.0, "il garde ses distances : {at}");
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
        // Un coup, puis un second aussitôt : ignoré (invulnérable un instant).
        assert!(damage_mob(&mut world, mob, 4.0, Vec3::X));
        assert!(!damage_mob(&mut world, mob, 4.0, Vec3::X));
        // Trois coups, l'invulnérabilité passée entre chacun : 12 > 10 points de vie.
        for _ in 0..2 {
            world.remove::<Hurt>(mob);
            assert!(damage_mob(&mut world, mob, 4.0, Vec3::X));
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
