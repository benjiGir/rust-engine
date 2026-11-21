//! L'eau vue du joueur : éclaboussures et rides quand on plonge ou qu'on nage, bulles sous l'eau,
//! et le degré d'immersion de la caméra que le shader utilise (brume bleue, reflets mouvants).
//!
//! Tout est décoratif : de petites boîtes (comme la pluie) dessinées avec les créatures, sans
//! effet sur la simulation.

use engine_render::CubeInstance;
use engine_voxel::{PlayerBody, World};
use glam::{Vec2, Vec3};

use crate::VoxelGame;
use crate::components::{Collider, Velocity};
use crate::player;

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
    /// De 0 (dehors) à 1 (la caméra est sous la surface), lissé.
    underwater: f32,
    /// L'altitude de la surface au-dessus de la caméra.
    surface: f32,
    wake_timer: f32,
    bubble_timer: f32,
    seed: u32,
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
        let registry = world.registry();
        let in_water = registry.def(world.block(eye.floor().as_ivec3())).liquid;
        let surface = PlayerBody::surface_at(world, eye);
        let submerged = in_water && surface.is_some_and(|s| eye.y < s);
        let target = if submerged { 1.0 } else { 0.0 };
        self.underwater += (target - self.underwater) * (dt * 10.0).min(1.0);
        if let Some(s) = surface.filter(|_| submerged) {
            self.surface = s;
        }

        // Plonger : une gerbe de gouttes et deux rides qui s'élargissent.
        if splash > MIN_SPLASH
            && let Some(top) = PlayerBody::surface_at(world, feet)
        {
            let center = Vec3::new(feet.x, top, feet.z);
            let count = (splash * 2.5).clamp(8.0, 36.0) as usize;
            for _ in 0..count {
                let angle = self.random() * std::f32::consts::TAU;
                let out = 0.3 + self.random() * 2.4;
                let up = 2.0 + self.random() * (1.5 + splash * 0.35).min(6.0);
                self.drops.push(Particle {
                    position: center + Vec3::new(angle.cos() * 0.7, 0.05, angle.sin() * 0.7),
                    velocity: Vec3::new(angle.cos() * out, up, angle.sin() * out),
                    age: 0.0,
                });
            }
            let strength = (splash / 12.0).clamp(0.4, 1.0);
            for delay in [0.0, 0.18] {
                self.rings.push(Ring {
                    center,
                    age: 0.0,
                    delay,
                    strength,
                });
            }
        }

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
                age: 0.0,
            });
        }

        // La vie des particules.
        for d in &mut self.drops {
            d.age += dt;
            d.velocity.y -= DROP_GRAVITY * dt;
            d.position += d.velocity * dt;
        }
        let water_top = PlayerBody::surface_at(world, feet).unwrap_or(f32::MIN);
        self.drops
            .retain(|d| d.age < 1.6 && (d.age < 0.1 || d.position.y > water_top));
        for b in &mut self.bubbles {
            b.age += dt;
            b.position += b.velocity * dt;
            // Une bulle dérive un peu de côté, comme dans un courant.
            b.position.x += (b.age * 3.0 + b.position.y).sin() * 0.15 * dt;
        }
        let surface_limit = self.surface;
        self.bubbles
            .retain(|b| b.age < 6.0 && b.position.y < surface_limit - 0.05);
        for r in &mut self.rings {
            if r.delay > 0.0 {
                r.delay -= dt;
            } else {
                r.age += dt;
            }
        }
        self.rings.retain(|r| r.age < RING_LIFE);
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
        for r in &self.rings {
            if r.delay > 0.0 {
                continue;
            }
            let t = r.age / RING_LIFE;
            let radius = 0.25 + t * 1.6 * (0.6 + 0.4 * r.strength);
            let fade = (1.0 - t) * r.strength;
            // Un cercle de petits morceaux posés à la surface.
            let pieces = (radius * 26.0).clamp(14.0, 52.0) as usize;
            for i in 0..pieces {
                let angle = i as f32 / pieces as f32 * std::f32::consts::TAU;
                let position =
                    r.center + Vec3::new(angle.cos() * radius, 0.03, angle.sin() * radius);
                let mut cube = CubeInstance::new(
                    position,
                    Vec3::new(0.05, 0.01, 0.05),
                    Vec3::new(0.9, 0.96, 1.0),
                );
                cube.opacity = (fade * 0.6).clamp(0.0, 0.6);
                if cube.opacity > 0.02 {
                    out.push(cube);
                }
            }
        }
    }
}

impl VoxelGame {
    /// Met les effets de l'eau à jour avec l'état du joueur.
    pub(crate) fn update_water_fx(&mut self, dt: f32) {
        let body = player::body(&self.server.ecs, self.server.players.entity());
        let (splash, swimming) = self
            .server
            .ecs
            .get::<Collider>(self.server.players.entity())
            .map_or((0.0, false), |c| (c.splash, c.in_liquid));
        let speed = self
            .server
            .ecs
            .get::<Velocity>(self.server.players.entity())
            .map_or(0.0, |v| Vec2::new(v.0.x, v.0.z).length());
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
    }
}
