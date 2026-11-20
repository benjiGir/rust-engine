//! Nage des poissons : volume d'eau entier, déplacement 3D, courants et évitement des berges.
use crate::{
    components::{Collider, Wander},
    mobs::GameRng,
};
use engine_voxel::{FLUID_FULL, PlayerBody, World};
use glam::{IVec3, Vec3};

/// La boîte est entièrement sous l'eau, y compris ses bords et la hauteur du fluide.
pub fn fits(world: &World, position: Vec3, width: f32, height: f32) -> bool {
    let min = position - Vec3::new(width * 0.5, 0.0, width * 0.5);
    let max = position + Vec3::new(width * 0.5, height, width * 0.5);
    let first = min.floor().as_ivec3();
    let last = (max - Vec3::splat(0.0001)).floor().as_ivec3();
    for x in first.x..=last.x {
        for y in first.y..=last.y {
            for z in first.z..=last.z {
                let at = IVec3::new(x, y, z);
                let def = world.registry().def(world.block(at));
                if def.solid || def.fluid.as_deref() != Some("core:water") {
                    return false;
                }
                let covered = world
                    .registry()
                    .def(world.block(at + IVec3::Y))
                    .fluid
                    .as_deref()
                    == Some("core:water");
                let level = if covered {
                    1.0
                } else {
                    world
                        .fluid_amount(at)
                        .map_or(1.0, |a| (a as f32 / FLUID_FULL as f32).min(1.0))
                        * 0.875
                };
                if max.y.min(y as f32 + 1.0) > y as f32 + level + 0.0001 {
                    return false;
                }
            }
        }
    }
    true
}

/// Nage sans poussée automatique vers la surface ; chaque petit pas conserve le volume humide.
/// Renvoie false hors de l'eau : la physique normale reprend alors (chute sur la berge).
pub fn step(body: &mut PlayerBody, world: &World, desired: Vec3, dt: f32) -> bool {
    if !fits(world, body.position, body.width, body.height) {
        // Si le niveau a changé, replonger au lieu de flotter comme un humain.
        let wet_below = body.position - Vec3::Y * body.height;
        if body.submersion(world).0 > 0.2 && fits(world, wet_below, body.width, body.height) {
            body.position.y -= (1.5 * dt).min(body.height);
            body.velocity = Vec3::new(0.0, -1.5, 0.0);
            body.in_liquid = true;
            body.on_ground = false;
            body.exit_boost = false;
            body.splash = 0.0;
            return true;
        }
        return false;
    }
    body.in_liquid = true;
    body.submersion = 1.0;
    body.on_ground = false;
    body.exit_boost = false;
    body.splash = 0.0;
    body.velocity = desired + body.current(world) * 0.35 + body.impulse;
    let steps = (body.velocity.length() * dt / 0.08).ceil().max(1.0) as u32;
    for _ in 0..steps {
        for axis in [1, 0, 2] {
            let mut next = body.position;
            next[axis] += body.velocity[axis] * dt / steps as f32;
            if fits(world, next, body.width, body.height) {
                body.position = next;
            } else {
                body.velocity[axis] = 0.0;
            }
        }
    }
    body.impulse *= (1.0 - dt * 3.0).max(0.0);
    true
}

pub fn wander(
    world: &World,
    position: Vec3,
    collider: &Collider,
    wander: &mut Wander,
    rng: &mut GameRng,
    player: Option<Vec3>,
) -> Vec3 {
    if !fits(world, position, collider.width, collider.height) {
        return Vec3::ZERO;
    }
    let blocked = !fits(
        world,
        position + wander.direction * 0.65,
        collider.width,
        collider.height,
    );
    if wander.timer <= 0.0 || blocked || wander.direction == Vec3::ZERO {
        wander.timer = 1.5 + rng.next_f32() * 3.0;
        let away = player
            .filter(|_| wander.panic > 0.0)
            .map(|p| (position - p).normalize_or_zero());
        for attempt in 0..12 {
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let direction = if attempt == 0 { away } else { None }.unwrap_or_else(|| {
                Vec3::new(angle.cos(), (rng.next_f32() - 0.5) * 1.0, angle.sin()).normalize()
            });
            if fits(
                world,
                position + direction * 0.65,
                collider.width,
                collider.height,
            ) {
                wander.direction = direction;
                return direction;
            }
        }
        wander.direction = Vec3::ZERO;
    }
    wander.direction
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pool() -> World {
        let blocks = engine_voxel::BlockRegistry::builtin();
        let mut world = World::with_registry(blocks.clone());
        let water = blocks.id("core:water").unwrap();
        for x in 0..6 {
            for y in 1..5 {
                for z in 0..6 {
                    world.set_block(IVec3::new(x, y, z), water);
                }
            }
        }
        world
    }
    #[test]
    fn fish_swim_at_depth_and_turn_before_air_walls_and_shallow_water() {
        let mut world = pool();
        let mut body = PlayerBody::with_size(Vec3::new(2.5, 2.0, 2.5), 0.5, 0.35);
        for _ in 0..600 {
            assert!(step(&mut body, &world, Vec3::X, 1.0 / 60.0));
        }
        assert!(
            (body.position.y - 2.0).abs() < 0.001,
            "pas de remontée vers la surface"
        );
        assert!(body.position.x < 5.76 && fits(&world, body.position, body.width, body.height));
        assert!(!fits(&world, Vec3::new(2.5, 4.7, 2.5), 0.5, 0.35));
        let mut surface = PlayerBody::with_size(Vec3::new(2.5, 4.65, 2.5), 0.5, 0.35);
        for _ in 0..20 {
            assert!(step(&mut surface, &world, Vec3::ZERO, 1.0 / 60.0));
        }
        assert!(
            fits(&world, surface.position, surface.width, surface.height),
            "il replonge sous la surface"
        );
        let stone = world.registry().id("core:stone").unwrap();
        world.set_block(IVec3::new(3, 2, 2), stone);
        assert!(!fits(&world, Vec3::new(2.9, 2.0, 2.5), 0.5, 0.35));
        let lava = world.registry().id("core:lava").unwrap();
        world.set_block(IVec3::new(2, 2, 2), lava);
        assert!(!fits(&world, Vec3::new(2.5, 2.0, 2.5), 0.5, 0.35));
    }

    #[test]
    fn natural_schools_are_capped_swim_in_water_and_suffocate_only_outside_it() {
        use crate::{
            components::{Aquatic, Health, Mob, Transform},
            mobs,
            shared::StreamerExt,
            spawning,
        };
        use std::sync::Arc;
        let center = Vec3::new(0.5, 78.0, 0.5);
        let mut world = mobs::tests::sim_at(center, 7);
        let player = world.resource::<mobs::PlayerEntities>().0[0];
        world.get_mut::<Transform>(player).unwrap().position = center;
        let water = world
            .streamer()
            .borrow()
            .world()
            .registry()
            .id("core:water")
            .unwrap();
        {
            let shared = world.streamer();
            let mut streamer = shared.borrow_mut();
            for x in -40..=40 {
                for z in -40..=40 {
                    for y in 75..80 {
                        streamer.set_block_raw(IVec3::new(x, y, z), water);
                    }
                }
            }
        }
        for _ in 0..100 {
            world.resource_mut::<mobs::SimClock>().spawn_timer = 0.0;
            spawning::spawn(&mut world);
            world.apply_commands();
        }
        let registry = world.resource::<Arc<mobs::MobRegistry>>().clone();
        let mut fish = Vec::new();
        world.query::<(&Mob, &Transform)>().for_each(|e, (mob, t)| {
            let def = registry.get(mob.kind).unwrap();
            if def.aquatic {
                assert!(fits(
                    world.streamer().borrow().world(),
                    t.position,
                    def.width,
                    def.height
                ));
                fish.push(e);
            }
        });
        assert!(!fish.is_empty() && fish.len() <= spawning::AQUATIC_CAP);
        let one = fish[0];
        assert!(world.has::<Aquatic>(one));
        let health = world.get::<Health>(one).unwrap().current;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for _ in 0..180 {
            mobs::wander(&mut world);
            mobs::physics(&mut world);
            mobs::environment(&mut world);
            let p = world.get::<Transform>(one).unwrap().position;
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
            assert!(fits(world.streamer().borrow().world(), p, 0.55, 0.44));
        }
        assert!(
            max_y - min_y > 0.05,
            "la nage doit aussi changer de profondeur"
        );
        assert_eq!(world.get::<Health>(one).unwrap().current, health);
        world.get_mut::<Transform>(one).unwrap().position = Vec3::new(0.5, 90.0, 0.5);
        for _ in 0..(35 * 60) {
            mobs::environment(&mut world);
        }
        assert!(
            world.get::<Health>(one).unwrap().current <= 0.0,
            "asphyxie sur la berge"
        );
    }
}
