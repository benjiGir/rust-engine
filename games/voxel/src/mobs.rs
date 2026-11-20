//! Les créatures : leur description (chargée depuis des fichiers, comme les blocs) et
//! les systèmes ECS qui les font vivre.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::shared::StreamerExt;
use anyhow::{Context as _, Result, bail};
use engine_ecs::{Entity, World};
use engine_voxel::{BlockRegistry, ChunkStreamer, PlayerBody, ray_aabb};
use glam::{IVec3, Vec2, Vec3};
use serde::Deserialize;

use crate::attributes::Pierce;
use crate::components::{
    Collider, Flying, Health, Hurt, Impulse, Mob, MobAnim, MoveIntent, PreviousPosition, Transform,
    Velocity, Wander,
};
use crate::events::{self, EntityDied, EntitySpawned, PlayerAttacked, Tick};
use crate::navigation::{DangerBlocks, PathFollow, SEARCHES_PER_STEP, cell_of};
use crate::players::PlayerId;
use crate::survival::{Senses, SurvivalRules, Vitals};
use crate::tiers::TierEffects;

/// Au plus ce nombre de créatures autour du joueur.
/// Distance d'apparition (entre les deux) et de disparition, en blocs.
const DESPAWN_DISTANCE: f32 = 96.0;
/// Distance à laquelle une créature hostile repère le joueur.
const CHASE_DISTANCE: f32 = 14.0;

// ----- La description des créatures -----

pub use crate::mob_model::MobPart;

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
    #[serde(default)]
    aquatic: bool,
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
    /// Le script de son mod la dirige (`Mod::mobs`) au lieu de l'IA du jeu.
    #[serde(default)]
    brain: bool,
    #[serde(default)]
    armor: f32,
    #[serde(default)]
    armor_pierce: f32,
    /// Un boss : il ne devient jamais élite, et le vaincre compte pour les paliers du monde.
    #[serde(default)]
    boss: bool,
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
    /// Ses points d'armure : ils retirent une part des dégâts des coups du joueur (voir
    /// `attributes.rs`). 0 par défaut.
    pub armor: f32,
    /// Les points d'armure du joueur que ses coups ignorent.
    pub armor_pierce: f32,
    pub hops: bool,
    pub aquatic: bool,
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
    /// Dirigée par le script de son mod (celui de l'espace de noms de son identifiant).
    pub brain: bool,
    /// Un boss (voir `elites.rs` et `tiers.rs`).
    pub boss: bool,
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
                armor: entry.armor,
                armor_pierce: entry.armor_pierce,
                hops: entry.hops,
                aquatic: entry.aquatic,
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
                brain: entry.brain,
                boss: entry.boss,
            });
        }
        Ok(())
    }

    /// Ajoute les surfaces des créatures après celles des blocs, pour le pack actif.
    pub fn bind_textures(&mut self, blocks: &mut BlockRegistry) {
        for def in &mut self.defs {
            let namespace = def.key.split_once(':').unwrap().0;
            crate::mob_model::bind_textures(&mut def.parts, namespace, blocks);
        }
    }

    /// Les identifiants des types de créatures, dans l'ordre de leur numéro.
    pub fn keys(&self) -> Vec<String> {
        self.defs.iter().map(|d| d.key.clone()).collect()
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

/// Les entités de tous les joueurs, dans l'ordre d'arrivée : le joueur local est le premier. Le
/// jeu la tient à jour (voir `players.rs`). Les systèmes qui visent « le joueur » prennent le plus
/// proche (voir [`player_spots`] et [`nearest_spot`]).
#[derive(Debug, Clone, Default)]
pub struct PlayerEntities(pub Vec<Entity>);

impl PlayerEntities {
    /// Le joueur local, celui de la fenêtre.
    pub fn local(&self) -> Option<Entity> {
        self.0.first().copied()
    }
}

/// Où est un joueur, pour les systèmes de créatures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSpot {
    pub entity: Entity,
    /// La position de ses pieds.
    pub position: Vec3,
    /// Il lui reste de la vie : un joueur mort n'est plus visé tant qu'un autre vit.
    pub alive: bool,
}

/// La position de chaque joueur (ceux qui n'ont plus de position sont ignorés).
pub fn player_spots(world: &World) -> Vec<PlayerSpot> {
    world
        .resource::<PlayerEntities>()
        .0
        .iter()
        .filter_map(|&entity| {
            let position = world.get::<Transform>(entity)?.position;
            let alive = world.get::<Health>(entity).is_none_or(|h| h.current > 0.0);
            Some(PlayerSpot {
                entity,
                position,
                alive,
            })
        })
        .collect()
}

/// Le joueur le plus proche de `from` parmi ceux qui vivent ; s'ils sont tous morts, le plus
/// proche tout court (avec un seul joueur, c'est donc toujours lui).
pub fn nearest_spot(spots: &[PlayerSpot], from: Vec3) -> Option<&PlayerSpot> {
    let closest = |alive_only: bool| {
        spots
            .iter()
            .filter(|s| s.alive || !alive_only)
            .min_by(|a, b| {
                let da = a.position.distance_squared(from);
                let db = b.position.distance_squared(from);
                da.total_cmp(&db)
            })
    };
    closest(true).or_else(|| closest(false))
}

/// La position du joueur le plus proche de `from`.
pub fn nearest_position(spots: &[PlayerSpot], from: Vec3) -> Option<Vec3> {
    nearest_spot(spots, from).map(|s| s.position)
}

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
    /// Le nombre de passages d'apparition : il désigne le joueur dont c'est le tour.
    pub spawn_turn: usize,
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
    let centers: Vec<Vec3> = player_spots(world).iter().map(|s| s.position).collect();
    let shared = world.streamer();
    let streamer = shared.borrow();
    // La gravité de la dimension (un multiple de la normale).
    let gravity = engine_voxel::GRAVITY
        * world
            .get_resource::<crate::components::GravityScale>()
            .map_or(1.0, |g| g.0);
    // Les structures mobiles, s'il y en a : une entité qui s'y tient se déplace dans leur repère.
    let subs = world
        .get_resource::<crate::shared::SharedSubGrids>()
        .map(|s| s.0.clone());
    let subs = subs.as_ref().map(|s| s.borrow());
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
                    || crate::spawning::asleep(world, entity, transform.position, &centers)
                {
                    return;
                }
                // Sur une structure : on simule dans son repère (elle peut tourner et avancer).
                let ride = world
                    .get::<crate::components::OnGrid>(entity)
                    .and_then(|g| subs.as_ref()?.get(g.0));
                if let Some(cell) = ride {
                    let grid = cell.borrow();
                    let mut body = PlayerBody::with_size(
                        grid.to_local(transform.position),
                        collider.width,
                        collider.height,
                    );
                    body.velocity = grid.direction_to_local(velocity.0);
                    body.on_ground = collider.on_ground;
                    body.gravity = gravity;
                    body.step(
                        grid.world(),
                        grid.direction_to_local(intent.walk),
                        intent.jump,
                        dt,
                    );
                    transform.position = grid.to_world(body.position);
                    velocity.0 = grid.direction_to_world(body.velocity);
                    collider.on_ground = body.on_ground;
                    collider.in_liquid = false;
                    return;
                }
                let mut body =
                    PlayerBody::with_size(transform.position, collider.width, collider.height);
                body.velocity = velocity.0;
                body.on_ground = collider.on_ground;
                // Pour savoir qu'elle vient d'entrer dans l'eau (éclaboussure, freinage).
                body.in_liquid = collider.in_liquid;
                body.exit_boost = collider.exit_boost;
                body.gravity = gravity;
                body.impulse = world.get::<Impulse>(entity).map_or(Vec3::ZERO, |i| i.0);
                collider.frozen = !surroundings_ready(&streamer, &body);
                if collider.frozen {
                    velocity.0 = Vec3::ZERO;
                    return;
                }
                if !world.has::<crate::components::Aquatic>(entity)
                    || !crate::aquatic::step(&mut body, streamer.world(), intent.walk, dt)
                {
                    body.step(streamer.world(), intent.walk, intent.jump, dt);
                }
                transform.position = body.position;
                velocity.0 = body.velocity;
                if let Some(mut impulse) = world.get_mut::<Impulse>(entity) {
                    impulse.0 = body.impulse;
                }
                collider.on_ground = body.on_ground;
                collider.in_liquid = body.in_liquid;
                collider.splash = body.splash;
                collider.exit_boost = body.exit_boost;
            },
        );
}

/// Crée une créature et prévient les abonnés (`EntitySpawned`).
pub fn spawn_mob(world: &mut World, kind: u32, position: Vec3) -> Option<Entity> {
    spawn_mob_with_coat(world, kind, position, None)
}

/// Une naissance peut fournir une couleur héritée ; les apparitions ordinaires la tirent au sort.
pub fn spawn_mob_with_coat(
    world: &mut World,
    kind: u32,
    position: Vec3,
    inherited: Option<crate::sheep_colors::SheepCoat>,
) -> Option<Entity> {
    let def = world.resource::<Arc<MobRegistry>>().get(kind)?.clone();
    // Un hostile naît plus robuste aux paliers hauts du monde (voir `tiers.rs`).
    let health = if def.hostile {
        def.health
            * world
                .get_resource::<TierEffects>()
                .map_or(1.0, |t| t.health)
    } else {
        def.health
    };
    let entity = world.spawn((
        Transform { position, yaw: 0.0 },
        PreviousPosition(position),
        Velocity::default(),
        Collider::new(def.width, def.height),
        MoveIntent::default(),
        Health::full(health),
        Mob { kind },
        Wander::default(),
    ));
    // À part : un « bundle » prend au plus 8 composants.
    world.insert(entity, PathFollow::default());
    world.insert(entity, MobAnim::default());
    if def.key == "core:sheep" {
        let coat = inherited.unwrap_or_else(|| {
            crate::sheep_colors::SheepCoat::choose(world.resource_mut::<GameRng>().next_f32())
        });
        log::debug!("mouton : {} ({})", coat.data().name, coat.data().id);
        world.insert(entity, coat);
    }
    if def.aquatic {
        world.insert(entity, crate::components::Aquatic);
    }
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
    let spots = player_spots(world);
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
                // Une créature à comportement est dirigée par son script (`mob_brains.rs`).
                if def.brain {
                    return;
                }
                let position = transform.position;
                // Le joueur le plus proche : sa cible, son regard, ce qu'elle fuit.
                let target = nearest_position(&spots, position);
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
                if def.aquatic {
                    let direction = crate::aquatic::wander(
                        streamer.world(),
                        position,
                        collider,
                        wander,
                        &mut rng,
                        target,
                    );
                    intent.walk =
                        direction * def.speed * if wander.panic > 0.0 { 1.5 } else { 1.0 };
                    intent.jump = false;
                    if direction.x != 0.0 || direction.z != 0.0 {
                        let wanted = direction.z.atan2(direction.x);
                        transform.yaw += wrap_angle(wanted - transform.yaw) * (dt * 5.0).min(1.0);
                    }
                    return;
                }
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
                } * crate::elites::speed_of(world, entity);
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
/// sans la noyade pour les terrestres ; les poissons perdent leur air hors de l'eau).
#[derive(Debug, Clone)]
pub struct MobVitals(pub Vitals);

/// Les créatures se blessent comme le joueur : une chute trop haute, la lave, le feu.
pub fn environment(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let centers: Vec<Vec3> = player_spots(world).iter().map(|s| s.position).collect();
    let rules = world.resource::<Arc<SurvivalRules>>().clone();
    let danger = world.resource::<Arc<DangerBlocks>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut hurt = Vec::new();
    world
        .query::<(&Transform, &Collider, &mut MobVitals)>()
        .for_each(|entity, (transform, collider, vitals)| {
            if crate::spawning::asleep(world, entity, transform.position, &centers) {
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
                head_under: world.has::<crate::components::Aquatic>(entity)
                    && !crate::aquatic::fits(
                        voxels,
                        transform.position,
                        collider.width,
                        collider.height,
                    ),
                fluid_damage,
                flying: false,
            };
            for (amount, cause) in vitals.0.step(&rules, senses, dt) {
                if vitals
                    .0
                    .hurt(&rules, amount, cause, 0.0, Pierce::NONE)
                    .is_some()
                {
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
    // Ceux qui volent ne sont pas touchés.
    let spots: Vec<PlayerSpot> = player_spots(world)
        .into_iter()
        .filter(|s| !world.has::<Flying>(s.entity))
        .collect();
    if spots.is_empty() {
        return;
    }
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    world.query::<(&Transform, &mut Wander, &Mob)>().for_each(
        |entity, (transform, wander, mob)| {
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            if !def.hostile || wander.attack_cooldown > 0.0 {
                return;
            }
            // Elle frappe le joueur le plus proche, s'il est à portée.
            let Some(victim) = nearest_spot(&spots, transform.position) else {
                return;
            };
            if transform.position.distance(victim.position) >= def.width * 0.5 + 0.6 {
                return;
            }
            wander.attack_cooldown = 1.0;
            // Le jeu applique le coup (armure, invulnérabilité) : voir `survival.rs`.
            events::send(
                world,
                PlayerAttacked {
                    player: world.get::<PlayerId>(victim.entity).map(|id| *id),
                    amount: def.damage * crate::elites::damage_of(world, entity),
                    from: transform.position,
                    pierce: def.armor_pierce,
                },
            );
        },
    );
}

/// Les créatures sans vie meurent ; celles trop loin disparaissent ; les coups s'estompent.
pub fn cleanup(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let spots = player_spots(world);
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
                        elite: crate::elites::of(world, entity),
                        coat: world
                            .get::<crate::sheep_colors::SheepCoat>(entity)
                            .map(|c| *c),
                        killer: world.get::<crate::players::LastHitBy>(entity).map(|k| k.0),
                    },
                );
                world.commands().despawn(entity);
            } else if spots
                .iter()
                .all(|s| transform.position.distance(s.position) > DESPAWN_DISTANCE)
            {
                // Loin de tous les joueurs : elle disparaît.
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

/// À quelle distance (blocs) une créature remarque le joueur et peut tourner la tête vers lui ; une
/// hostile le suit de plus loin.
const LOOK_RANGE: f32 = 8.0;
const LOOK_RANGE_HOSTILE: f32 = 16.0;
/// De combien la tête peut se tourner de chaque côté (radians).
const LOOK_LIMIT: f32 = 1.1;
/// La phase avance de tant de radians par bloc parcouru.
const PHASE_PER_BLOCK: f32 = 5.0;

fn wrap_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Anime les créatures : la marche (pattes et bras) suit leur vitesse ; la tête se tourne vers le
/// joueur quand il est près, par moments (une créature ne le fixe pas sans cesse), ou fait un petit
/// coup d'œil au hasard.
pub fn animate(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let spots = player_spots(world);
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let mut rng = world.resource_mut::<GameRng>();
    world
        .query::<(&Transform, &Velocity, &Mob, &mut MobAnim)>()
        .for_each(|_, (transform, velocity, mob, anim)| {
            let target = nearest_position(&spots, transform.position);
            let Some(def) = registry.get(mob.kind) else {
                return;
            };
            let speed = Vec2::new(velocity.0.x, velocity.0.z).length();
            let goal = (speed / def.speed.max(0.5)).min(1.4);
            anim.stride += (goal - anim.stride) * (1.0 - (-10.0 * dt).exp());
            anim.phase = (anim.phase
                + dt * (speed + if def.aquatic { 0.4 } else { 0.0 }) * PHASE_PER_BLOCK)
                % std::f32::consts::TAU;

            // Le regard : près du joueur, la créature s'y intéresse un moment, puis passe à autre chose.
            let to = target.map(|t| t - transform.position);
            let range = if def.hostile {
                LOOK_RANGE_HOSTILE
            } else {
                LOOK_RANGE
            };
            let near = to.is_some_and(|d| Vec2::new(d.x, d.z).length() < range);
            anim.timer -= dt;
            if anim.timer <= 0.0 {
                anim.attending = near && (def.hostile || rng.next_f32() < 0.6);
                anim.timer = if anim.attending {
                    2.0 + rng.next_f32() * 3.0
                } else {
                    1.5 + rng.next_f32() * 3.5
                };
                anim.glance = if rng.next_f32() < 0.5 {
                    (rng.next_f32() - 0.5) * 0.9
                } else {
                    0.0
                };
            }
            let wanted = match to {
                Some(d) if anim.attending && near => {
                    wrap_angle(d.z.atan2(d.x) - transform.yaw).clamp(-LOOK_LIMIT, LOOK_LIMIT)
                }
                _ => anim.glance,
            };
            anim.look += wrap_angle(wanted - anim.look) * (1.0 - (-6.0 * dt).exp());
        });
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
    fn sheep_colours_tint_only_fleece_and_survive_despawning_in_the_death_event() {
        use crate::sheep_colors::{SheepCoat, drop_key, palette};
        let mut world = sim();
        let registry = world.resource::<Arc<MobRegistry>>().clone();
        let kind = registry.id("core:sheep").unwrap();
        let mob = spawn_mob_with_coat(
            &mut world,
            kind,
            Vec3::new(5.5, 80.0, 5.5),
            Some(SheepCoat(0)),
        )
        .unwrap();
        let plain = crate::entity_render::support::boxes(&mut world, 1.0);
        world.insert(mob, SheepCoat(3));
        let dyed = crate::entity_render::support::boxes(&mut world, 1.0);
        for ((part, a), b) in registry
            .get(kind)
            .unwrap()
            .parts
            .iter()
            .zip(&plain)
            .zip(&dyed)
        {
            assert_eq!(a.center, b.center);
            assert_eq!(a.size, b.size);
            assert_eq!(a.textures, b.textures);
            if part.texture.as_deref() == Some("core:mob_sheep_wool") {
                assert_ne!(a.color, b.color);
            } else {
                assert_eq!(
                    a.color, b.color,
                    "peau, yeux et sabots gardent leur matière"
                );
            }
        }
        world.commands().despawn(mob);
        world.apply_commands();
        let items = engine_voxel::ItemRegistry::builtin(&crate::test_registry()).unwrap();
        let mut reader = world.resource::<Events<EntityDied>>().reader();
        for (i, color) in palette().iter().enumerate() {
            let coat = SheepCoat(i as u8);
            let e = spawn_mob_with_coat(&mut world, kind, Vec3::new(5.5, 80.0, 5.5), Some(coat))
                .unwrap();
            world.get_mut::<Health>(e).unwrap().current = 0.0;
            cleanup(&mut world);
            world.apply_commands();
            assert!(!world.is_alive(e));
            let died: Vec<_> = reader
                .read(&world.resource::<Events<EntityDied>>())
                .cloned()
                .collect();
            assert_eq!(died.len(), 1);
            assert_eq!(died[0].coat, Some(coat));
            assert_eq!(drop_key("core:wool", died[0].coat), color.item);
            assert!(items.id(drop_key("core:wool", died[0].coat)).is_some());
        }
    }

    #[test]
    fn every_wool_colour_has_native_icons_and_can_make_a_bed_even_when_mixed() {
        use crate::sheep_colors::palette;
        let blocks = crate::test_registry();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let recipes = engine_voxel::Recipes::builtin(&items).unwrap();
        let plank = items.id("core:oak_planks");
        for (i, color) in palette().iter().enumerate() {
            let wool = items.id(&color.item);
            assert!(wool.is_some());
            let other = items.id(&palette()[(i + 1) % palette().len()].item);
            let grid = [wool, other, wool, plank, plank, plank, None, None, None];
            assert_eq!(
                recipes.find(&grid, 3).unwrap().result(),
                (items.id("core:bed").unwrap(), 1)
            );
            for packed in [false, true] {
                let mut registry = blocks.clone();
                if packed {
                    registry
                        .set_visual_pack(
                            Some(repo().join("texture_packs/earth-copper-32/assets")),
                            32,
                        )
                        .unwrap();
                }
                let pixels = crate::assets::load_png(
                    &registry.asset_path(&color.item, "items", "png").unwrap(),
                )
                .unwrap();
                assert_eq!(pixels.len(), if packed { 32 * 32 * 4 } else { 16 * 16 * 4 });
                let opaque: Vec<_> = pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[3] > 0)
                    .collect();
                assert!(!opaque.is_empty());
                for c in 0..3 {
                    let mean = opaque
                        .iter()
                        .map(|p| {
                            let s = p[c] as f32 / 255.0;
                            if s <= 0.04045 {
                                s / 12.92
                            } else {
                                ((s + 0.055) / 1.055).powf(2.4)
                            }
                        })
                        .sum::<f32>()
                        / opaque.len() as f32;
                    assert!(
                        (mean - items.get(wool.unwrap()).color[c]).abs()
                            < if packed { 0.02 } else { 0.00001 }
                    );
                }
            }
        }
    }

    #[test]
    fn farm_loot_has_icons_food_and_cooking_and_wool_can_make_a_bed() {
        use engine_voxel::{ItemRegistry, Recipes, Smelting};
        let blocks = crate::test_registry();
        let items = ItemRegistry::builtin(&blocks).unwrap();
        let creatures = MobRegistry::load(&repo().join("assets"), &[], &blocks).unwrap();
        for key in ["core:sheep", "core:chicken", "core:fish"] {
            for drop in &creatures.by_key(key).unwrap().drops {
                assert!(
                    items.id(&drop.item).is_some(),
                    "butin inconnu : {}",
                    drop.item
                );
            }
        }
        let cooking = Smelting::builtin(&items).unwrap();
        for name in ["mutton", "chicken", "fish"] {
            let raw = items.id(&format!("core:raw_{name}")).unwrap();
            let cooked = items.id(&format!("core:cooked_{name}")).unwrap();
            assert_eq!(cooking.recipe(raw).unwrap().output, cooked);
            assert!(
                items.get(cooked).food.unwrap().nutrition > items.get(raw).food.unwrap().nutrition
            );
            for packed in [false, true] {
                let base = if packed {
                    repo().join("texture_packs/earth-copper-32/assets/core/items")
                } else {
                    repo().join("assets/core/items")
                };
                for prefix in ["raw", "cooked"] {
                    let image = crate::assets::load_png(&base.join(format!("{prefix}_{name}.png")))
                        .unwrap();
                    assert_eq!(image.len(), if packed { 32 * 32 * 4 } else { 16 * 16 * 4 });
                    assert!(image.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
                    assert!(image.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
                }
            }
        }
        let wool = items.id("core:wool");
        let plank = items.id("core:oak_planks");
        let recipes = Recipes::builtin(&items).unwrap();
        let grid = [wool, wool, wool, plank, plank, plank, None, None, None];
        assert_eq!(
            recipes.find(&grid, 3).unwrap().result(),
            (items.id("core:bed").unwrap(), 1)
        );
    }

    #[test]
    fn creature_skins_use_both_packs_without_changing_block_ids_and_keep_fallback_colours() {
        let root = repo();
        let mods = [
            ("example".into(), root.join("mods/example")),
            ("donjons".into(), root.join("mods/donjons")),
        ];
        for packed in [false, true] {
            let mut blocks = crate::test_registry();
            let ids: Vec<_> = blocks
                .iter()
                .map(|(id, def)| (id, def.key.clone()))
                .collect();
            let original_layers = blocks.textures().to_vec();
            if packed {
                blocks
                    .set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)
                    .unwrap();
            }
            let mut creatures = MobRegistry::load(&root.join("assets"), &mods, &blocks).unwrap();
            creatures.bind_textures(&mut blocks);
            let keys = blocks.textures().to_vec();
            creatures.bind_textures(&mut blocks);
            assert_eq!(
                keys,
                blocks.textures(),
                "le rechargement réutilise les couches"
            );
            assert_eq!(
                ids,
                blocks
                    .iter()
                    .map(|(id, def)| (id, def.key.clone()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(&blocks.textures()[..original_layers.len()], original_layers);
            let textures = crate::assets::load_textures(&blocks);
            for key in [
                "core:pig",
                "core:cow",
                "core:sheep",
                "core:chicken",
                "core:fish",
                "core:prowler",
                "core:spore_spitter",
                "example:slime",
                "example:guard",
                "donjons:warden",
            ] {
                let def = creatures.by_key(key).unwrap();
                assert!(def.parts.iter().any(|p| p.texture.is_some()));
                for part in def.parts.iter().filter(|p| p.texture.is_some()) {
                    assert!(part.texture_layer as usize > original_layers.len());
                    let image = &textures[part.texture_layer as usize - 1];
                    let side = if packed { 32 } else { 16 };
                    assert_eq!(image.len(), side * side * 4);
                    let fallback = [part.color.0, part.color.1, part.color.2];
                    for channel in 0..3 {
                        let mean = image
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|p| {
                                let c = p[channel] as f32 / 255.0;
                                if c <= 0.04045 {
                                    c / 12.92
                                } else {
                                    ((c + 0.055) / 1.055).powf(2.4)
                                }
                            })
                            .sum::<f32>()
                            / (side * side) as f32;
                        assert!(
                            (mean - fallback[channel]).abs() < if packed { 0.02 } else { 0.00001 },
                            "{key} : moyenne de repli"
                        );
                    }
                }
            }
        }
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
        world.insert_resource(crate::spawning::Difficulty(1));
        world.insert_resource(Arc::new(DangerBlocks::default()));
        world.insert_resource(Arc::new(SurvivalRules::default()));
        events::register(&mut world);
        let player = world.spawn((Transform::default(), Flying));
        world.insert(player, PlayerId(0));
        world.insert_resource(PlayerEntities(vec![player]));
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
        // Les grottes peuvent percer la surface, et un arbre ou un buisson peut le couvrir : le
        // cochon est au plus sur le bloc solide le plus haut de la colonne.
        let top = (ground..ground + 32)
            .rev()
            .find(|&y| {
                world
                    .streamer()
                    .borrow()
                    .world()
                    .is_solid(glam::IVec3::new(x, y, z))
            })
            .unwrap_or(ground);
        assert!(
            position.y <= top as f32 + 1.01 && position.y > 1.0,
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
        let player = world.resource::<PlayerEntities>().0[0];
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
    fn day_spawns_land_animals_and_monsters_only_in_the_dark() {
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
                    } else if registry.get(mob.kind).is_some_and(|d| !d.aquatic) {
                        found.2 += 1;
                    }
                });
            found
        };
        for night in [false, true] {
            let mut world = sim();
            let player = world.resource::<PlayerEntities>().0[0];
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
        let player = world.resource::<PlayerEntities>().0[0];
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

    /// Une créature dirigée par un script (`mob_move_to`) contourne un mur pour aller au point
    /// demandé : sans chemin, en ligne droite, elle resterait collée au mur.
    #[test]
    fn a_scripted_mob_walks_around_a_wall_to_its_goal() {
        let mut world = sim();
        let stone = blocks().id("core:stone").unwrap();
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
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
        let slime = world
            .resource::<Arc<MobRegistry>>()
            .id("example:slime")
            .unwrap();
        let mob = spawn_mob(&mut world, slime, Vec3::new(-3.5, 151.0, 0.5)).unwrap();
        let goal = Vec3::new(4.5, 151.0, 0.5);
        world.insert(
            mob,
            crate::components::BrainGoal {
                cell: crate::navigation::cell_of(goal),
                speed: 3.0,
            },
        );
        for _ in 0..(20 * 60) {
            crate::mob_brains::follow_goals(&mut world);
            physics(&mut world);
            if world.get::<Transform>(mob).unwrap().position.distance(goal) < 1.5 {
                return;
            }
        }
        let at = world.get::<Transform>(mob).unwrap().position;
        panic!("pas arrivé, il est en {at}");
    }

    #[test]
    fn legs_swing_with_the_walk_and_rest_when_still() {
        let mut world = sim();
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let mob = spawn_mob(&mut world, pig, Vec3::new(0.5, 150.0, 0.5)).unwrap();
        world.get_mut::<Velocity>(mob).unwrap().0 = Vec3::new(1.6, 0.0, 0.0);
        for _ in 0..60 {
            animate(&mut world);
        }
        let walking = *world.get::<MobAnim>(mob).unwrap();
        assert!(walking.stride > 0.8, "il marche : {}", walking.stride);
        // Les pattes ont un balancement, en diagonale (deux de chaque signe).
        let boxes = crate::entity_render::support::boxes(&mut world, 1.0);
        let swings: Vec<f32> = boxes
            .iter()
            .map(|b| b.swing[0])
            .filter(|a| *a != 0.0)
            .collect();
        assert!(
            swings.len() >= 8,
            "pattes et sabots balancent : {}",
            swings.len()
        );
        assert!(swings.iter().any(|a| *a > 0.05) && swings.iter().any(|a| *a < -0.05));
        // À l'arrêt, les pas s'éteignent et la phase ne bouge plus.
        world.get_mut::<Velocity>(mob).unwrap().0 = Vec3::ZERO;
        for _ in 0..120 {
            animate(&mut world);
        }
        let still = *world.get::<MobAnim>(mob).unwrap();
        assert!(still.stride < 0.02, "{}", still.stride);
        let phase = still.phase;
        animate(&mut world);
        assert_eq!(world.get::<MobAnim>(mob).unwrap().phase, phase);
    }

    #[test]
    fn a_creature_turns_its_head_toward_a_player_nearby_and_looks_away_far_off() {
        let mut world = sim();
        let player = world.resource::<PlayerEntities>().0[0];
        let prowler = world
            .resource::<Arc<MobRegistry>>()
            .id("core:prowler")
            .unwrap();
        let mob = spawn_mob(&mut world, prowler, Vec3::new(0.5, 150.0, 0.5)).unwrap();
        world.get_mut::<Transform>(mob).unwrap().yaw = 0.0;
        // Le joueur à 5 blocs, à 45° sur sa gauche (côté +z) : une hostile le suit des yeux.
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(4.0, 150.0, 4.5);
        for _ in 0..240 {
            animate(&mut world);
        }
        let look = world.get::<MobAnim>(mob).unwrap().look;
        assert!((look - 0.78).abs() < 0.15, "tête tournée de {look}");
        // La tête pivote avec ses boîtes : leur lacet change, pas celui du corps.
        let boxes = crate::entity_render::support::boxes(&mut world, 1.0);
        assert!(
            boxes.iter().any(|b| (b.yaw - look).abs() < 0.01),
            "une boîte de tête tournée"
        );
        assert!(boxes.iter().any(|b| b.yaw == 0.0), "le corps reste droit");
        // Le joueur loin (60 blocs) : elle ne le regarde plus, la tête revient près du repos.
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(60.0, 150.0, 4.5);
        for _ in 0..600 {
            animate(&mut world);
        }
        assert!(world.get::<MobAnim>(mob).unwrap().look.abs() < 0.5);
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
        let player = world.resource::<PlayerEntities>().0[0];
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
        let player = world.resource::<PlayerEntities>().0[0];
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
        let player = world.resource::<PlayerEntities>().0[0];
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
                elite: None,
                coat: None,
                killer: None,
            }]
        );
    }

    /// Un monde paisible : plus d'hostiles, celles qui restent disparaissent.
    #[test]
    fn peaceful_worlds_have_no_hostile_creatures() {
        let mut world = sim();
        world.insert_resource(crate::spawning::Difficulty(0));
        let slime = world
            .resource::<Arc<MobRegistry>>()
            .id("example:slime")
            .unwrap();
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let hostile = spawn_mob(&mut world, slime, Vec3::new(8.5, 90.0, 8.5)).unwrap();
        let friendly = spawn_mob(&mut world, pig, Vec3::new(9.5, 90.0, 8.5)).unwrap();
        // La nuit comme le jour : l'hostile finit par partir (un tirage par seconde).
        world.resource_mut::<crate::spawning::Sky>().night = true;
        for _ in 0..200 {
            world.resource_mut::<SimClock>().dissolve_timer = 0.0;
            crate::spawning::dissolve(&mut world);
            world.apply_commands();
        }
        assert!(world.get::<Mob>(hostile).is_none(), "l'hostile a disparu");
        assert!(world.get::<Mob>(friendly).is_some(), "le cochon reste");
    }
}
