//! L'attaque à distance (phase O5) : une créature qui a `ranged` dans `entities.ron`
//! s'arrête à portée, si elle voit le joueur, et lui lance des projectiles.
//!
//! Un projectile est une petite entité : il file, tombe un peu (la gravité), s'arrête au
//! premier bloc plein, et blesse le joueur qu'il touche (un coup ordinaire : armure,
//! invulnérabilité, recul ; voir `player/survival.rs`). La créature vise un peu au-dessus pour
//! compenser la chute.

use std::sync::Arc;

use engine_ecs::{Entity, World};
use glam::Vec3;
use serde::Deserialize;

use crate::components::{Collider, Flying, Mob, PreviousPosition, Transform, Wander};
use crate::events::{self, PlayerAttacked};
use crate::mobs::{FixedDt, MobRegistry, PlayerSpot};
use crate::player::players::PlayerId;
use crate::shared::StreamerExt as _;

/// La gravité des projectiles (blocs par seconde²), plus faible que celle des corps.
const GRAVITY: f32 = 12.0;
/// Un projectile qui n'a rien touché disparaît après ce temps.
const LIFETIME: f32 = 5.0;
/// L'attaque à distance d'une créature (`ranged` dans `entities.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangedDef {
    /// La portée : elle s'arrête un peu avant et tire.
    pub range: f32,
    /// Secondes entre deux tirs.
    pub interval: f32,
    /// Vitesse du projectile (blocs par seconde).
    pub speed: f32,
    pub damage: f32,
    /// Couleur du projectile.
    #[serde(default = "default_color")]
    pub color: (f32, f32, f32),
}

fn default_color() -> (f32, f32, f32) {
    (0.35, 0.3, 0.25)
}

/// Un projectile en vol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projectile {
    pub velocity: Vec3,
    pub damage: f32,
    pub age: f32,
    pub color: Vec3,
}

/// Rien ne bloque la vue entre `from` et `to` ?
pub fn line_of_sight(world: &engine_voxel::World, from: Vec3, to: Vec3) -> bool {
    let distance = from.distance(to);
    engine_voxel::raycast(world, from, to - from, distance).is_none()
}

/// Les yeux d'une créature (pour viser et voir).
pub fn eye_of(position: Vec3, collider: &Collider) -> Vec3 {
    position + Vec3::Y * collider.height * 0.8
}

/// La vitesse de départ pour toucher `to` depuis `from` : droit dessus, un peu plus haut
/// pour compenser la chute pendant le vol.
pub fn aim(from: Vec3, to: Vec3, speed: f32) -> Vec3 {
    let time = from.distance(to) / speed.max(0.1);
    let lifted = to + Vec3::Y * (0.5 * GRAVITY * time * time);
    (lifted - from).normalize_or_zero() * speed
}

/// Les créatures qui tirent : à portée, si elles voient le joueur, et rechargées.
pub fn shoot(world: &mut World) {
    // On ne tire pas sur qui vole : seuls les joueurs au sol sont des cibles.
    let spots: Vec<PlayerSpot> = crate::mobs::player_spots(world)
        .into_iter()
        .filter(|s| !world.has::<Flying>(s.entity))
        .collect();
    if spots.is_empty() {
        return;
    }
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut shots = Vec::new();
    world
        .query::<(&Transform, &Collider, &mut Wander, &Mob)>()
        .for_each(|_, (transform, collider, wander, mob)| {
            let Some(ranged) = registry.get(mob.kind).and_then(|d| d.ranged) else {
                return;
            };
            // Le joueur le plus proche, visé au torse.
            let Some(target) = crate::mobs::nearest_position(&spots, transform.position) else {
                return;
            };
            let target = target + Vec3::Y * 1.2;
            let eye = eye_of(transform.position, collider);
            if wander.attack_cooldown > 0.0
                || eye.distance(target) > ranged.range
                || !line_of_sight(streamer.world(), eye, target)
            {
                return;
            }
            wander.attack_cooldown = ranged.interval;
            let velocity = aim(eye, target, ranged.speed);
            // Le départ, un peu devant la créature (pas dans sa propre boîte).
            let start = eye + velocity.normalize_or_zero() * (collider.width * 0.5 + 0.3);
            let (r, g, b) = ranged.color;
            shots.push((start, velocity, ranged.damage, Vec3::new(r, g, b)));
        });
    drop(streamer);
    for (start, velocity, damage, color) in shots {
        launch(world, start, velocity, damage, color);
    }
}

/// Lance un projectile.
fn launch(world: &mut World, start: Vec3, velocity: Vec3, damage: f32, color: Vec3) {
    world.spawn((
        Transform {
            position: start,
            yaw: 0.0,
        },
        PreviousPosition(start),
        Projectile {
            velocity,
            damage,
            age: 0.0,
            color,
        },
    ));
}

/// Une créature qui a `spit` crache sur le joueur le plus proche quand on la frappe (au plus une
/// fois par `interval`, et seulement s'il est à portée et en vue). Le crachat est un projectile
/// ordinaire, qui blesse peu.
pub fn spit_back(world: &mut World, entity: Entity) {
    let Some(kind) = world.get::<Mob>(entity).map(|m| m.kind) else {
        return;
    };
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let Some(spit) = registry.get(kind).and_then(|d| d.spit) else {
        return;
    };
    let (Some(position), Some(collider)) = (
        world.get::<Transform>(entity).map(|t| t.position),
        world.get::<Collider>(entity).map(|c| *c),
    ) else {
        return;
    };
    if world
        .get::<Wander>(entity)
        .is_none_or(|w| w.attack_cooldown > 0.0)
    {
        return;
    }
    let spots: Vec<PlayerSpot> = crate::mobs::player_spots(world)
        .into_iter()
        .filter(|s| !world.has::<Flying>(s.entity))
        .collect();
    let Some(target) = crate::mobs::nearest_position(&spots, position) else {
        return;
    };
    let target = target + Vec3::Y * 1.2;
    let eye = eye_of(position, &collider);
    let shared = world.streamer();
    let clear =
        eye.distance(target) <= spit.range && line_of_sight(shared.borrow().world(), eye, target);
    if !clear {
        return;
    }
    if let Some(mut wander) = world.get_mut::<Wander>(entity) {
        wander.attack_cooldown = spit.interval;
    }
    let velocity = aim(eye, target, spit.speed);
    let start = eye + velocity.normalize_or_zero() * (collider.width * 0.5 + 0.3);
    let (r, g, b) = spit.color;
    launch(world, start, velocity, spit.damage, Vec3::new(r, g, b));
}

/// Les projectiles en vol : la gravité, puis un bloc (il s'arrête) ou le joueur (il le
/// blesse).
pub fn fly(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    // Les joueurs que les tirs peuvent toucher : ceux qui ne volent pas.
    let targets: Vec<(Option<PlayerId>, Vec3)> = crate::mobs::player_spots(world)
        .into_iter()
        .filter(|s| !world.has::<Flying>(s.entity))
        .map(|s| (world.get::<PlayerId>(s.entity).map(|id| *id), s.position))
        .collect();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut gone = Vec::new();
    let mut hits = Vec::new();
    world
        .query::<(&mut Transform, &mut PreviousPosition, &mut Projectile)>()
        .for_each(|entity, (transform, previous, projectile)| {
            previous.0 = transform.position;
            projectile.age += dt;
            projectile.velocity.y -= GRAVITY * dt;
            let step = projectile.velocity * dt;
            let from = transform.position;
            // Un bloc sur le trajet de ce pas : il s'y arrête.
            if engine_voxel::raycast(streamer.world(), from, step, step.length()).is_some()
                || projectile.age > LIFETIME
            {
                gone.push(entity);
                return;
            }
            transform.position = from + step;
            // Un joueur : sa boîte (0,6 de large, 1,8 de haut), un peu élargie.
            let p = transform.position;
            let touched = targets.iter().find(|(_, feet)| {
                (p.x - feet.x).abs() < 0.45
                    && (p.z - feet.z).abs() < 0.45
                    && p.y > feet.y - 0.1
                    && p.y < feet.y + 1.9
            });
            if let Some((player, _)) = touched {
                hits.push((*player, projectile.damage, from));
                gone.push(entity);
            }
        });
    drop(streamer);
    for (player, amount, from) in hits {
        events::send(
            world,
            PlayerAttacked {
                player,
                amount: amount * crate::mobs::elites::tier_damage(world),
                from,
                pierce: 0.0,
            },
        );
    }
    for entity in gone {
        world.commands().despawn(entity);
    }
}

/// Une créature qui tire s'arrête à portée si elle voit le joueur (sinon elle approche).
pub fn holds_position(
    ranged: &RangedDef,
    world: &engine_voxel::World,
    eye: Vec3,
    target: Vec3,
) -> bool {
    eye.distance(target) <= ranged.range * 0.75 && line_of_sight(world, eye, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aiming_compensates_the_drop() {
        let from = Vec3::new(0.0, 10.0, 0.0);
        let to = Vec3::new(12.0, 10.0, 0.0);
        let velocity = aim(from, to, 15.0);
        assert!((velocity.length() - 15.0).abs() < 1e-3);
        assert!(velocity.y > 0.0, "vise au-dessus");
        // En suivant la trajectoire, on passe près du but.
        let (mut p, mut v, dt) = (from, velocity, 1.0 / 240.0);
        while p.x < to.x {
            v.y -= GRAVITY * dt;
            p += v * dt;
        }
        assert!((p.y - to.y).abs() < 0.5, "arrive à {p}");
    }

    #[test]
    fn an_alpaca_spits_back_at_whoever_hits_it_but_not_every_time() {
        use crate::components::Transform;
        use crate::events::PlayerAttacked;
        use crate::mobs::tests::{blocks, sim};
        use crate::mobs::{self, PlayerEntities, damage_mob};
        use engine_ecs::Events;
        use glam::IVec3;
        let mut world = sim();
        let stone = blocks().id("core:stone").unwrap();
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
            for x in -10..10 {
                for z in -3..4 {
                    streamer.set_block_raw(IVec3::new(x, 150, z), stone);
                }
            }
        }
        let player = world.resource::<PlayerEntities>().0[0];
        world.remove::<Flying>(player);
        world.get_mut::<Transform>(player).unwrap().position = Vec3::new(4.5, 151.0, 0.5);
        let kind = world
            .resource::<Arc<MobRegistry>>()
            .id("core:alpaca")
            .unwrap();
        let alpaca = mobs::spawn_mob(&mut world, kind, Vec3::new(0.5, 151.0, 0.5)).unwrap();
        let count = |world: &World| {
            let mut n = 0;
            world.query::<(&Projectile,)>().for_each(|_, _| n += 1);
            n
        };
        assert_eq!(count(&world), 0);
        assert!(damage_mob(&mut world, alpaca, 1.0, Vec3::X));
        assert_eq!(count(&world), 1, "il crache");
        // Un second coup aussitôt : l'invulnérabilité passée, mais le crachat est encore à
        // recharger.
        world.remove::<crate::components::Hurt>(alpaca);
        assert!(damage_mob(&mut world, alpaca, 1.0, Vec3::X));
        assert_eq!(count(&world), 1, "pas deux crachats d'affilée");
        // Le crachat arrive sur le joueur et le blesse un peu.
        let mut reader = world.resource::<Events<PlayerAttacked>>().reader();
        let mut amount = 0.0;
        for _ in 0..120 {
            fly(&mut world);
            world.apply_commands();
            amount += reader
                .read(&world.resource::<Events<PlayerAttacked>>())
                .map(|e| e.amount)
                .sum::<f32>();
        }
        assert!(amount > 0.0 && amount < 3.0, "dégâts du crachat : {amount}");
        // Une vache, elle, ne crache pas.
        let cow = world.resource::<Arc<MobRegistry>>().id("core:cow").unwrap();
        let cow = mobs::spawn_mob(&mut world, cow, Vec3::new(1.5, 151.0, 0.5)).unwrap();
        assert!(damage_mob(&mut world, cow, 1.0, Vec3::X));
        assert_eq!(count(&world), 0);
    }
}
