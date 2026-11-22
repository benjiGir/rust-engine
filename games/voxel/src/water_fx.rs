//! L'eau vue du joueur : éclaboussures et rides quand on plonge ou qu'on nage, bulles sous l'eau,
//! et le degré d'immersion de la caméra que le shader utilise (brume bleue, reflets mouvants).
//!
//! Tout est décoratif : de petites boîtes (comme la pluie) dessinées avec les créatures, sans
//! effet sur la simulation.

use engine_render::CubeInstance;
use engine_voxel::{PlayerBody, World};
use glam::{Vec2, Vec3};

use crate::VoxelGame;

/// La gravité d'une gouttelette (blocs par seconde au carré).
const DROP_GRAVITY: f32 = 22.0;
/// Combien de temps vit une ride, en secondes.
const RING_LIFE: f32 = 1.1;
/// Délai entre deux rides quand on nage à la surface.
const WAKE_PERIOD: f32 = 0.32;
/// Combien de temps entre deux bulles sous l'eau.
const BUBBLE_PERIOD: f32 = 0.45;
/// Plus petite vitesse d'entrée qui éclabousse.
const MIN_SPLASH: f32 = 2.5;

struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    surface: f32,
}

struct Ring {
    center: Vec3,
    age: f32,
    delay: f32,
    strength: f32,
}

/// Ce que les effets savent du joueur : ses pieds et sa caméra, la vitesse d'entrée dans l'eau à
/// ce pas (voir `PlayerBody::splash`), sa vitesse horizontale, et s'il est dans un liquide.
pub struct Swimmer {
    pub feet: Vec3,
    pub eye: Vec3,
    pub splash: f32,
    pub speed: f32,
    pub swimming: bool,
}

#[derive(Default)]
pub struct WaterFx {
    drops: Vec<Particle>,
    bubbles: Vec<Particle>,
    rings: Vec<Ring>,
    item_positions: std::collections::HashMap<crate::entities::NetId, Vec3>,
    /// De 0 (dehors) à 1 (la caméra est sous la surface), lissé.
    underwater: f32,
    /// L'altitude de la surface au-dessus de la caméra.
    surface: f32,
    wake_timer: f32,
    diving: bool,
    bubble_timer: f32,
    seed: u32,
}

fn water_at(world: &World, cell: glam::IVec3) -> bool {
    let def = world.registry().def(world.block(cell));
    def.liquid && def.alpha < 1.0 && def.light_emission == 0
}

fn submerged_surface(world: &World, eye: Vec3) -> Option<f32> {
    let mut cell = eye.floor().as_ivec3();
    if !water_at(world, cell) {
        return None;
    }
    while water_at(world, cell + glam::IVec3::Y) {
        cell.y += 1;
    }
    let level = world.fluid_amount(cell).map_or(1., |amount| {
        f32::from(amount) / engine_voxel::FLUID_FULL as f32
    });
    let surface = cell.y as f32 + 0.875 * level.min(1.);
    (eye.y < surface).then_some(surface)
}

impl WaterFx {
    /// Le degré d'immersion de la caméra, et l'altitude de la surface au-dessus d'elle.
    pub fn underwater(&self) -> (f32, f32) {
        (self.underwater, self.surface)
    }

    fn random(&mut self) -> f32 {
        // Un petit générateur suffit : l'aléa n'a pas besoin d'être reproductible.
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Avance les effets de `dt` secondes. `splash` : la vitesse d'entrée du joueur à ce pas
    /// (voir `PlayerBody::splash`) ; `feet` et `eye` : ses pieds et sa caméra ; `speed` : sa
    /// vitesse horizontale ; `swimming` : il est dans un liquide.
    pub fn update(&mut self, dt: f32, world: &World, player: Swimmer) {
        let Swimmer {
            feet,
            eye,
            splash,
            speed,
            swimming,
        } = player;
        // La caméra est-elle sous la surface ?
        let surface = submerged_surface(world, eye);
        let submerged = surface.is_some();
        let target = if submerged { 1.0 } else { 0.0 };
        self.underwater += (target - self.underwater) * (dt * 10.0).min(1.0);
        if let Some(s) = surface {
            self.surface = s;
        }

        // Plonger : une gerbe de gouttes et deux rides qui s'élargissent.
        if splash > MIN_SPLASH
            && !self.diving
            && let Some(top) = PlayerBody::surface_at(world, feet)
        {
            let center = Vec3::new(feet.x, top, feet.z);
            if water_at(world, (center - Vec3::Y * 0.01).floor().as_ivec3()) {
                self.impact(center, splash, 0.55);
            }
        }

        self.diving = splash > MIN_SPLASH;

        // Nager à la surface laisse un sillage de petites rides.
        self.wake_timer -= dt;
        if swimming && speed > 0.6 && self.wake_timer <= 0.0 && !submerged {
            self.wake_timer = WAKE_PERIOD;
            if let Some(top) = PlayerBody::surface_at(world, feet) {
                self.rings.push(Ring {
                    center: Vec3::new(feet.x, top, feet.z),
                    age: 0.0,
                    delay: 0.0,
                    strength: 0.35,
                });
            }
        }

        // Sous l'eau : de petites bulles montent depuis le visage.
        self.bubble_timer -= dt;
        if submerged && self.bubble_timer <= 0.0 {
            self.bubble_timer = BUBBLE_PERIOD * (0.6 + self.random());
            let offset = Vec3::new(self.random() - 0.5, -0.25, self.random() - 0.5) * 0.5;
            let rise = 0.9 + self.random() * 0.6;
            self.bubbles.push(Particle {
                position: eye + offset,
                velocity: Vec3::new(0.0, rise, 0.0),
                surface: self.surface,
                age: 0.0,
            });
        }

        // La vie des particules.
        for d in &mut self.drops {
            d.age += dt;
            d.velocity.y -= DROP_GRAVITY * dt;
            d.position += d.velocity * dt;
        }
        self.drops
            .retain(|d| d.age < 1.6 && (d.age < 0.1 || d.position.y > d.surface));
        for b in &mut self.bubbles {
            b.age += dt;
            b.position += b.velocity * dt;
            // Une bulle dérive un peu de côté, comme dans un courant.
            b.position.x += (b.age * 3.0 + b.position.y).sin() * 0.15 * dt;
        }
        self.bubbles
            .retain(|b| b.age < 6.0 && b.position.y < b.surface - 0.03);
        for r in &mut self.rings {
            if r.delay > 0.0 {
                r.delay -= dt;
            } else {
                r.age += dt;
            }
        }
        self.rings.retain(|r| r.age < RING_LIFE);
    }

    fn impact(&mut self, center: Vec3, splash: f32, radius: f32) {
        let count = (splash * 2.5).clamp(3.0, 36.0) as usize;
        for _ in 0..count {
            let angle = self.random() * std::f32::consts::TAU;
            let out = 0.3 + self.random() * 2.4;
            let up = 2.0 + self.random() * (1.5 + splash * 0.35).min(6.0);
            self.drops.push(Particle {
                position: center + Vec3::new(angle.cos() * radius, 0.05, angle.sin() * radius),
                velocity: Vec3::new(angle.cos() * out, up, angle.sin() * out),
                surface: center.y,
                age: 0.0,
            });
        }
        for _ in 0..count / 2 {
            let depth = 0.1 + self.random() * (splash * 0.10).min(1.4);
            let x = (self.random() - 0.5) * radius * 2.;
            let z = (self.random() - 0.5) * radius * 2.;
            let rise = 0.45 + self.random() * 0.8;
            self.bubbles.push(Particle {
                position: center + Vec3::new(x, -depth, z),
                velocity: Vec3::new(x * 0.3, rise, z * 0.3),
                age: 0.,
                surface: center.y,
            });
        }
        let strength = (splash / 12.0).clamp(0.2, 1.0);
        for delay in [0.0, 0.18] {
            self.rings.push(Ring {
                center,
                age: 0.0,
                delay,
                strength,
            });
        }
    }

    fn item_impacts(
        &mut self,
        entities: &crate::client_entities::ClientEntities,
        world: &World,
        eye: Vec3,
        dt: f32,
    ) {
        self.item_positions
            .retain(|id, _| entities.get(*id).is_some());
        for (id, item) in entities.iter() {
            if !matches!(item.kind, crate::entities::EntityKind::Item { .. }) {
                continue;
            }
            let position = item.pose_at(entities.now(1.0)).position;
            let Some(old) = self.item_positions.insert(id, position) else {
                continue;
            };
            if old.y <= position.y || eye.distance_squared(position) > 32. * 32. {
                continue;
            }
            if let Some(top) = PlayerBody::surface_at(world, position)
                && old.y >= top
                && position.y < top
                && water_at(
                    world,
                    Vec3::new(position.x, top - 0.01, position.z)
                        .floor()
                        .as_ivec3(),
                )
            {
                self.impact(
                    Vec3::new(position.x, top, position.z),
                    ((old.y - position.y) / dt.max(0.001) * 0.55).clamp(1.5, 6.),
                    0.12,
                );
            }
        }
    }

    fn surface_rings(&self) -> [[f32; 4]; 8] {
        let mut rings = [[0.; 4]; 8];
        for (entry, r) in rings
            .iter_mut()
            .zip(self.rings.iter().rev().filter(|r| r.delay <= 0.))
        {
            *entry = r
                .center
                .extend(0.10 + r.age / RING_LIFE * (0.8 + r.strength))
                .to_array();
        }
        rings
    }

    /// Les boîtes à dessiner.
    pub fn instances(&self, camera: Vec3, out: &mut Vec<CubeInstance>) {
        // Ce qui est collé à la caméra masquerait tout : on ne le dessine pas.
        let far_enough = |p: Vec3| p.distance_squared(camera) > 1.4 * 1.4;
        for d in self.drops.iter().filter(|d| far_enough(d.position)) {
            out.push(CubeInstance::new(
                d.position,
                Vec3::splat(0.035),
                Vec3::new(0.78, 0.9, 1.0),
            ));
        }
        for b in self.bubbles.iter().filter(|b| far_enough(b.position)) {
            let mut cube =
                CubeInstance::new(b.position, Vec3::splat(0.05), Vec3::new(0.8, 0.95, 1.0));
            cube.opacity = 0.55;
            out.push(cube);
        }
    }
}

impl VoxelGame {
    /// Met les effets de l'eau à jour avec l'état du joueur.
    pub(crate) fn update_water_fx(&mut self, dt: f32) {
        let body = self.client.body.body;
        let (splash, swimming) = (body.splash, body.in_liquid);
        let speed = Vec2::new(body.velocity.x, body.velocity.z).length();
        self.client.water_fx.item_impacts(
            &self.client.view.entities,
            self.client.view.world(),
            body.eye(),
            dt,
        );
        self.client.water_fx.update(
            dt,
            self.client.view.world(),
            Swimmer {
                feet: body.position,
                eye: body.eye(),
                splash,
                speed,
                swimming,
            },
        );
        self.client.renderer.water_rings = self.client.water_fx.surface_rings();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immersion_reaches_the_top_of_a_deep_column() {
        let mut world = World::new();
        let water = world.registry().id("core:water").unwrap();
        for y in 0..12 {
            world.set_block(glam::IVec3::new(0, y, 0), water);
        }
        assert_eq!(submerged_surface(&world, Vec3::splat(0.5)), Some(11.875));
        assert_eq!(submerged_surface(&world, Vec3::new(0.5, 11.9, 0.5)), None);
    }
}
