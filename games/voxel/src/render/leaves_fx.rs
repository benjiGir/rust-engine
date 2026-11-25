//! Les feuilles qui tombent (lot N2 de `docs/plan-nature.md`) : des feuilles se détachent du dessous du
//! feuillage proche du joueur, tombent en tournoyant, poussées par le vent (`Weather::wind`), se posent
//! à plat sur le sol et s'effacent. Les cerisiers et les arbres en fleurs lâchent des **pétales** (lot
//! N4) : plus petits, plus nombreux, plus lents.
//!
//! Tout est décoratif et calculé par la fenêtre seule, comme les éclaboussures de l'eau : de petites
//! boîtes plates dessinées avec les créatures, sans effet sur la simulation ni sur le réseau.

use std::collections::HashMap;

use engine_render::CubeInstance;
use engine_voxel::{BlockId, BlockRegistry, CHUNK_SIZE, Tint, World};
use glam::{IVec3, Vec2, Vec3};

use crate::VoxelGame;
use voxel_sim::world::weather::Wind;

/// Au plus ce nombre de feuilles à la fois.
const MAX_LEAVES: usize = 320;
/// Les feuilles naissent dans ce rayon autour de la caméra (horizontal, puis en hauteur, de part
/// et d'autre)...
const SPAWN_RADIUS: i32 = 18;
const SPAWN_HEIGHT: i32 = 12;
/// ... et disparaissent au-delà de celui-ci.
const KEEP_RADIUS: f32 = 28.0;
/// Colonnes examinées par seconde (on y cherche le dessous du feuillage).
const SAMPLES_PER_SECOND: f32 = 200.0;
/// La chance qu'une feuille se détache du dessous du feuillage d'une colonne examinée, à vent
/// faible ; le vent la multiplie (jusqu'à × 5,7 sous l'orage).
const DETACH_CHANCE: f32 = 0.16;
/// La vitesse de chute d'une feuille, et la part du vent qu'elle prend.
const FALL_SPEED: f32 = 0.75;
const WIND_CARRY: f32 = 3.2;
/// Une feuille vit au plus ce temps en l'air, puis ce temps posée.
const MAX_FLIGHT: f32 = 16.0;
const REST_TIME: f32 = 4.0;
/// La taille d'une feuille (longueur, épaisseur, largeur), et celle d'un pétale.
const LEAF_SIZE: Vec3 = Vec3::new(0.2, 0.015, 0.14);
const PETAL_SIZE: Vec3 = Vec3::new(0.12, 0.012, 0.1);

/// Une essence de feuillage et ce qu'elle lâche.
#[derive(Debug, Clone, Copy)]
struct LeafKind {
    color: [f32; 3],
    tint: Tint,
    /// Combien elle en lâche, par rapport à un feuillage ordinaire.
    shedding: f32,
    /// Ce sont des pétales (cerisier, arbre en fleurs).
    petals: bool,
}

struct Leaf {
    position: Vec3,
    velocity: Vec3,
    color: Vec3,
    /// Sa phase de balancement et de rotation, propre à chaque feuille.
    phase: f32,
    spin: f32,
    age: f32,
    /// Posée au sol depuis ce temps.
    landed: Option<f32>,
    /// Un pétale : plus petit, il tombe moins vite.
    petal: bool,
}

#[derive(Default)]
pub struct LeavesFx {
    leaves: Vec<Leaf>,
    /// Les blocs de feuillage du registre (retrouvés une fois).
    kinds: Option<HashMap<BlockId, LeafKind>>,
    /// Des examens en retard (une fraction d'examen par image).
    pending: f32,
    rng: u64,
    time: f32,
}

/// Les blocs de feuillage : ceux qui ont le comportement des feuilles. Les érables et les arbres en
/// fleurs en lâchent plus.
fn leaf_kinds(registry: &BlockRegistry) -> HashMap<BlockId, LeafKind> {
    registry
        .iter()
        .filter(|(_, def)| def.behavior.as_deref() == Some("core:leaves"))
        .map(|(id, def)| {
            let (shedding, petals) = match def.key.as_str() {
                "core:maple_leaves"
                | "core:maple_yellow_leaves"
                | "core:maple_gold_leaves"
                | "core:maple_red_leaves"
                | "core:maple_crimson_leaves" => (2.5, false),
                "core:blossom_leaves" => (3.0, true),
                "core:cherry_leaves" => (4.5, true),
                _ => (1.0, false),
            };
            (
                id,
                LeafKind {
                    color: def.color,
                    tint: def.tint,
                    shedding,
                    petals,
                },
            )
        })
        .collect()
}

/// La teinte du feuillage du biome de la colonne `at`, rapportée à la couleur de référence (comme
/// le maillage), ou blanche si le biome est inconnu.
fn foliage_tint(world: &World, at: IVec3) -> Vec3 {
    let Some(biomes) = world.biomes() else {
        return Vec3::ONE;
    };
    let size = CHUNK_SIZE as i32;
    let chunk = IVec3::new(
        at.x.div_euclid(size),
        at.y.div_euclid(size),
        at.z.div_euclid(size),
    );
    let biome = world.chunk(chunk).and_then(|c| {
        c.biome(
            at.x.rem_euclid(size) as usize,
            at.z.rem_euclid(size) as usize,
        )
    });
    let Some(biome) = biome.filter(|&b| (b as usize) < biomes.len()) else {
        return Vec3::ONE;
    };
    let (_, reference, _) = biomes.tint_reference();
    let color = biomes.get(engine_voxel::BiomeId(biome)).foliage_color;
    Vec3::from_array(color) / Vec3::from_array(reference).max(Vec3::splat(0.01))
}

impl LeavesFx {
    fn random(&mut self) -> f32 {
        // xorshift64*
        if self.rng == 0 {
            self.rng = 0x2545_F491_4F6C_DD1D;
        }
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Combien de feuilles en l'air ou posées (l'état de `ctl`).
    pub fn count(&self) -> usize {
        self.leaves.len()
    }

    /// Un pas : de nouvelles feuilles se détachent près de `camera`, celles qui tombent avancent.
    pub fn update(&mut self, dt: f32, world: &World, camera: Vec3, wind: Wind) {
        let dt = dt.min(0.1);
        self.time += dt;
        let kinds = self
            .kinds
            .get_or_insert_with(|| leaf_kinds(world.registry()))
            .clone();
        self.spawn(dt, world, camera, wind, &kinds);
        self.fly(dt, world, camera, wind);
    }

    fn spawn(
        &mut self,
        dt: f32,
        world: &World,
        camera: Vec3,
        wind: Wind,
        kinds: &HashMap<BlockId, LeafKind>,
    ) {
        if kinds.is_empty() {
            return;
        }
        self.pending += SAMPLES_PER_SECOND * dt;
        let gusty = 0.35 + 1.65 * wind.strength.clamp(0.0, 1.0);
        let center = camera.floor().as_ivec3();
        while self.pending >= 1.0 {
            self.pending -= 1.0;
            if self.leaves.len() >= MAX_LEAVES {
                continue;
            }
            let dx = (self.random() * (2 * SPAWN_RADIUS + 1) as f32) as i32 - SPAWN_RADIUS;
            let dz = (self.random() * (2 * SPAWN_RADIUS + 1) as f32) as i32 - SPAWN_RADIUS;
            // Le dessous du feuillage de cette colonne : une feuille au-dessus du vide (une feuille
            // au cœur de la couronne n'a nulle part où aller).
            let found = (-SPAWN_HEIGHT..=SPAWN_HEIGHT).rev().find_map(|dy| {
                let cell = center + IVec3::new(dx, dy, dz);
                let kind = kinds.get(&world.block(cell))?;
                world
                    .block(cell - IVec3::Y)
                    .is_air()
                    .then_some((cell, *kind))
            });
            let Some((cell, kind)) = found else {
                continue;
            };
            if self.random() >= DETACH_CHANCE * gusty * kind.shedding {
                continue;
            }
            let tint = if kind.tint == Tint::Foliage {
                foliage_tint(world, cell)
            } else {
                Vec3::ONE
            };
            // Chaque feuille a sa nuance : plus claire, plus sombre, un peu jaunie. Un peu plus claire
            // que le bloc en moyenne : seule dans le ciel, elle reçoit la lumière de tous les côtés.
            let shade = 1.15 + 0.45 * self.random();
            let yellow = Vec3::new(0.06, 0.03, -0.02) * self.random();
            let color = if kind.petals {
                // Un pétale : du rose au presque blanc.
                let pale = 0.25 * self.random();
                Vec3::from_array(kind.color).lerp(Vec3::ONE, pale)
            } else {
                (Vec3::from_array(kind.color) * tint * shade + yellow).max(Vec3::ZERO)
            };
            let position = cell.as_vec3() + Vec3::new(self.random(), -0.02, self.random());
            let phase = self.random() * std::f32::consts::TAU;
            let spin = (self.random() - 0.5) * 4.0;
            self.leaves.push(Leaf {
                position,
                velocity: Vec3::new(0.0, -0.2, 0.0),
                color,
                phase,
                spin,
                age: 0.0,
                landed: None,
                petal: kind.petals,
            });
        }
    }

    fn fly(&mut self, dt: f32, world: &World, camera: Vec3, wind: Wind) {
        let dir = Vec2::from_angle(wind.direction);
        let strength = wind.strength.clamp(0.0, 1.0);
        let time = self.time;
        self.leaves.retain_mut(|leaf| {
            leaf.age += dt;
            if let Some(rest) = &mut leaf.landed {
                *rest += dt;
                return *rest < REST_TIME;
            }
            if leaf.age > MAX_FLIGHT || leaf.position.distance(camera) > KEEP_RADIUS {
                return false;
            }
            // Le vent la porte (par bouffées), elle se balance d'un côté à l'autre en tombant.
            let gust = 0.6 + 0.4 * (time * 1.3 + leaf.phase).sin();
            let carry = dir * strength * WIND_CARRY * gust;
            let flutter = (time * 2.3 + leaf.phase).sin();
            let side = Vec2::new(-dir.y, dir.x) * flutter * 0.6;
            let fall = if leaf.petal { 0.6 } else { 1.0 } * FALL_SPEED;
            let target = Vec3::new(
                carry.x + side.x,
                -fall * (0.75 + 0.25 * (time * 3.1 + leaf.phase).cos()),
                carry.y + side.y,
            );
            leaf.velocity += (target - leaf.velocity) * (dt * 2.5).min(1.0);
            let next = leaf.position + leaf.velocity * dt;
            let cell = next.floor().as_ivec3();
            if world.is_solid(cell) || world.registry().def(world.block(cell)).liquid {
                // Posée sur le sol (ou sur l'eau) : à plat, juste au-dessus.
                leaf.position = Vec3::new(next.x, cell.y as f32 + 1.01, next.z);
                leaf.landed = Some(0.0);
            } else {
                leaf.position = next;
            }
            true
        });
    }

    /// Les boîtes à dessiner.
    pub fn instances(&self, world: &World, camera: Vec3, out: &mut Vec<CubeInstance>) {
        for leaf in &self.leaves {
            // Collée à la caméra, elle masquerait tout.
            if leaf.position.distance_squared(camera) < 0.6 * 0.6 {
                continue;
            }
            let (sky, block) = world.light(leaf.position.floor().as_ivec3());
            let size = if leaf.petal { PETAL_SIZE } else { LEAF_SIZE };
            let mut cube = CubeInstance::new(leaf.position, size, leaf.color)
                .with_yaw(leaf.phase + leaf.spin * leaf.age);
            match leaf.landed {
                Some(rest) => cube.opacity = (1.0 - rest / REST_TIME).clamp(0.0, 1.0),
                // En l'air, elle bascule d'avant en arrière.
                None => cube.swing = [0.7 * (leaf.age * 2.3 + leaf.phase).sin(), 0.0],
            }
            cube.light = [f32::from(sky) / 15.0, f32::from(block) / 15.0];
            out.push(cube);
        }
    }
}

impl VoxelGame {
    /// Fait tomber les feuilles autour de la caméra, au vent du moment.
    pub(crate) fn update_leaves_fx(&mut self, dt: f32) {
        let wind = self.client.weather.wind();
        let camera = self.client.camera.position;
        self.client
            .leaves_fx
            .update(dt, self.client.view.world(), camera, wind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un monde avec une plaque de feuilles de chêne à 10 blocs de haut et du sol en dessous.
    fn canopy() -> World {
        let mut world = World::new();
        let leaves = world.registry().id("core:oak_leaves").unwrap();
        let stone = world.registry().id("core:stone").unwrap();
        for x in -12..12 {
            for z in -12..12 {
                world.set_block(IVec3::new(x, 10, z), leaves);
                world.set_block(IVec3::new(x, 0, z), stone);
            }
        }
        world
    }

    fn wind(strength: f32) -> Wind {
        Wind {
            direction: 0.0,
            strength,
        }
    }

    #[test]
    fn leaves_fall_from_the_canopy_drift_with_the_wind_and_settle() {
        let world = canopy();
        let camera = Vec3::new(0.0, 5.0, 0.0);
        let mut fx = LeavesFx::default();
        for _ in 0..60 {
            fx.update(1.0 / 30.0, &world, camera, wind(1.0));
        }
        assert!(fx.count() > 5, "{} feuilles", fx.count());
        let start: Vec<Vec3> = fx.leaves.iter().map(|l| l.position).collect();
        for _ in 0..30 {
            fx.update(1.0 / 30.0, &world, camera, wind(1.0));
        }
        // Elles descendent, et le vent (vers +x) les pousse.
        let first = &fx.leaves[0];
        assert!(first.position.y < start[0].y);
        assert!(first.position.x > start[0].x);
        // Assez tard, elles sont posées sur la pierre (le dessus est à y = 1).
        for _ in 0..600 {
            fx.update(1.0 / 30.0, &world, camera, wind(0.0));
        }
        assert!(
            fx.leaves
                .iter()
                .filter(|l| l.landed.is_some())
                .all(|l| (l.position.y - 1.01).abs() < 1e-3)
        );
    }

    #[test]
    fn the_wind_sheds_more_leaves_and_never_more_than_the_cap() {
        let world = canopy();
        let camera = Vec3::new(0.0, 5.0, 0.0);
        let shed = |strength: f32| {
            let mut fx = LeavesFx::default();
            for _ in 0..90 {
                fx.update(1.0 / 30.0, &world, camera, wind(strength));
            }
            fx.count()
        };
        assert!(shed(1.0) > 2 * shed(0.0));
        let mut fx = LeavesFx::default();
        for _ in 0..3000 {
            fx.update(1.0 / 30.0, &world, camera, wind(1.0));
            assert!(fx.count() <= MAX_LEAVES);
        }
    }
}
