//! La dynamique d'une structure mobile : gravité, flottaison, ballons, hélices, barre, et
//! collisions avec le terrain.
//!
//! # Un corps rigide, mais simple
//!
//! Le plan d'origine parlait de `rapier3d`. Une structure de ce jeu n'a pourtant besoin que de
//! trois choses : tomber ou flotter, avancer, tourner autour de la verticale (elle ne se retourne
//! pas). On calcule donc nous-mêmes un corps à quatre degrés de liberté (la position et le
//! lacet), ce qui reste lisible et sans dépendance, et se règle en quelques constantes. Rapier
//! deviendrait utile le jour où les structures devront tanguer, se heurter entre elles ou
//! rouler.
//!
//! # Les forces
//!
//! Chaque bloc pèse 1. À chaque pas :
//! - **gravité** : tire vers le bas ;
//! - **flottaison** : chaque bloc dont le centre est dans l'eau déplace 1,6 de masse d'eau
//!   (comme un bloc de bois qui flotte à moitié enfoncé) ;
//! - **ballons** : chacun ajoute une portance de 4 ;
//! - **hélices** : la poussée suit la vitesse de rotation reçue par leur réseau
//!   (`Kinetic::speed`), dans le sens où elles regardent. Une hélice décentrée fait aussi
//!   tourner la structure (le couple autour du pivot) ;
//! - **la barre** : un couple de lacet choisi par le pilote ;
//! - **frottements** : faibles dans l'air, forts dans l'eau.
//!
//! # Collisions
//!
//! Chaque bloc plein est une boîte (celle de son cube tourné, englobée par une boîte alignée).
//! On avance axe par axe : un déplacement qui ferait entrer un bloc dans un bloc plein du monde
//! est refusé et la vitesse sur cet axe s'annule. Même chose pour le lacet.

use glam::{IVec3, Vec3};

use crate::subgrid::{SubGrid, rotate_y};
use crate::world::World;

pub const GRAVITY: f32 = 24.0;
/// La masse d'eau déplacée par un bloc immergé (un bloc pèse 1).
pub const WATER_DISPLACEMENT: f32 = 1.6;
/// La portance d'un ballon.
pub const BALLOON_LIFT: f32 = 4.0;
/// La poussée d'une hélice à 64 tr/min.
pub const THRUST_AT_64: f32 = 60.0;
/// L'accélération angulaire de la barre à fond, en rad/s².
pub const STEER_ACCEL: f32 = 1.2;
const AIR_DRAG: f32 = 0.25;
const WATER_DRAG: f32 = 1.8;
const YAW_DRAG: f32 = 1.4;
const MAX_SPEED: f32 = 24.0;
/// Les boîtes se rétrécissent un peu pour qu'un bloc posé sur le sol ne le « touche » pas.
const SKIN: f32 = 0.02;

/// Ce que fait le pilote.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ShipInput {
    /// De -1 (à gauche) à 1 (à droite).
    pub steer: f32,
}

/// Ce qui s'est passé pendant un pas.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StepReport {
    /// Le déplacement de la structure et sa rotation pendant ce pas.
    pub moved: Vec3,
    pub turned: f32,
    /// Combien de blocs sont dans l'eau.
    pub submerged: usize,
    /// La structure est posée sur quelque chose (elle ne tombe plus).
    pub grounded: bool,
}

/// Le bloc est-il de l'eau ?
fn is_water(world: &World, pos: IVec3) -> bool {
    world.registry().def(world.block(pos)).fluid.as_deref() == Some("core:water")
}

/// Une case de la structure vue par la dynamique.
struct Part {
    /// Le centre de la case dans le repère local, relatif au pivot.
    offset: Vec3,
    solid: bool,
    lift: f32,
    /// La poussée locale d'une hélice (sens × vitesse), sinon zéro.
    thrust: Vec3,
}

fn parts(grid: &SubGrid) -> Vec<Part> {
    let registry = grid.world().registry();
    grid.cells()
        .map(|cell| {
            let block = grid.world().block(cell);
            let def = registry.def(block);
            let mut part = Part {
                offset: cell.as_vec3() + Vec3::splat(0.5) - grid.pivot,
                solid: def.solid,
                lift: 0.0,
                thrust: Vec3::ZERO,
            };
            if def.key == "core:balloon" {
                part.lift = BALLOON_LIFT;
            }
            if def.behavior.as_deref() == Some("core:propeller") {
                let entity = {
                    let (chunk, [x, y, z]) = World::split(cell);
                    grid.world()
                        .chunk(chunk)
                        .and_then(|c| c.block_entity(x, y, z))
                };
                let speed = entity
                    .and_then(|e| e.caps.as_ref()?.kinetic.as_ref().map(|k| k.speed))
                    .unwrap_or(0.0);
                let facing = crate::circuits::facing_of(entity);
                part.thrust = facing.as_vec3() * (speed / 64.0) * THRUST_AT_64;
            }
            part
        })
        .collect()
}

/// La boîte entière du cube, tournée de `yaw` et englobée : sa demi-largeur horizontale.
fn half_extent(yaw: f32) -> f32 {
    0.5 * (yaw.cos().abs() + yaw.sin().abs())
}

/// Une position et un lacet font-ils entrer un bloc plein dans un bloc plein du monde ?
fn collides(parts: &[Part], world: &World, position: Vec3, yaw: f32) -> bool {
    let half = half_extent(yaw) - SKIN;
    let half_y = 0.5 - SKIN;
    for part in parts.iter().filter(|p| p.solid) {
        let center = position + rotate_y(part.offset, yaw);
        let min = (center - Vec3::new(half, half_y, half)).floor().as_ivec3();
        let max = (center + Vec3::new(half, half_y, half)).floor().as_ivec3();
        for y in min.y..=max.y {
            for z in min.z..=max.z {
                for x in min.x..=max.x {
                    if world.is_solid(IVec3::new(x, y, z)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Fait avancer la structure de `dt` secondes dans `world`.
pub fn step(grid: &mut SubGrid, world: &World, input: ShipInput, dt: f32) -> StepReport {
    let parts = parts(grid);
    if parts.is_empty() {
        return StepReport::default();
    }
    let mass = parts.len() as f32;
    let (position, yaw) = (grid.position, grid.yaw);

    // Les forces. `submerged` : les blocs dont le centre est dans l'eau.
    let mut buoyancy = 0.0;
    let mut submerged = 0usize;
    let mut thrust = Vec3::ZERO;
    let mut torque = 0.0;
    let mut inertia = 1.0;
    for part in &parts {
        let rotated = rotate_y(part.offset, yaw);
        let center = position + rotated;
        if is_water(world, center.floor().as_ivec3()) {
            buoyancy += WATER_DISPLACEMENT;
            submerged += 1;
        }
        buoyancy += part.lift;
        inertia += part.offset.x * part.offset.x + part.offset.z * part.offset.z;
        if part.thrust != Vec3::ZERO {
            let force = rotate_y(part.thrust, yaw);
            thrust += force;
            torque += rotated.x * force.z - rotated.z * force.x;
        }
    }
    let in_water = submerged as f32 / mass;
    let accel = Vec3::new(
        thrust.x / mass,
        GRAVITY * (buoyancy / mass - 1.0),
        thrust.z / mass,
    ) + Vec3::Y * thrust.y / mass;
    let drag = AIR_DRAG + (WATER_DRAG - AIR_DRAG) * in_water.min(1.0);

    grid.velocity += accel * dt;
    grid.velocity *= (1.0 - drag * dt).max(0.0);
    // L'eau retient : sans poussée verticale, on ne coule pas plus vite que la gravité.
    grid.velocity = grid
        .velocity
        .clamp(Vec3::splat(-MAX_SPEED), Vec3::splat(MAX_SPEED));
    grid.yaw_rate += (torque / inertia + input.steer * STEER_ACCEL) * dt;
    grid.yaw_rate *= (1.0 - YAW_DRAG * dt).max(0.0);
    grid.yaw_rate = grid.yaw_rate.clamp(-2.0, 2.0);

    // Les déplacements, axe par axe : Y d'abord (on sait si l'on est posé), puis X et Z.
    let mut grounded = false;
    let start = grid.position;
    for axis in [1usize, 0, 2] {
        let mut next = grid.position;
        next[axis] += grid.velocity[axis] * dt;
        if collides(&parts, world, next, grid.yaw) {
            if axis == 1 && grid.velocity.y < 0.0 {
                grounded = true;
            }
            grid.velocity[axis] = 0.0;
        } else {
            grid.position = next;
        }
    }
    // Le lacet : refusé s'il fait entrer la structure dans le terrain.
    let before = grid.yaw;
    let wanted = grid.yaw + grid.yaw_rate * dt;
    if collides(&parts, world, grid.position, wanted) {
        grid.yaw_rate = 0.0;
    } else {
        grid.yaw = wanted;
    }
    StepReport {
        moved: grid.position - start,
        turned: grid.yaw - before,
        submerged,
        grounded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;
    use crate::circuits::placement_state;
    use crate::grid::{BlockAccess, GridId};
    use crate::registry::BlockRegistry;

    fn registry() -> std::sync::Arc<BlockRegistry> {
        BlockRegistry::builtin()
    }

    fn id(world: &World, key: &str) -> BlockId {
        world.registry().id(key).unwrap()
    }

    /// Un monde : un grand sol de pierre à y = 0 (le dessus est à y = 1), et si `water`, un
    /// bassin d'eau de y = 1 à 6.
    fn arena(water: bool) -> World {
        let mut world = World::with_registry(registry());
        let (stone, water_block) = (id(&world, "core:stone"), id(&world, "core:water"));
        for x in -40..40 {
            for z in -40..40 {
                world.set_block(IVec3::new(x, 0, z), stone);
                if water {
                    for y in 1..=6 {
                        world.set_block(IVec3::new(x, y, z), water_block);
                    }
                }
            }
        }
        world
    }

    /// Un bateau : un radeau de planches de 4 × 1 × 3, posé à `y`.
    fn raft(y: f32) -> SubGrid {
        let mut grid = SubGrid::new(GridId(1), registry());
        let planks = grid.registry().id("core:oak_planks").unwrap();
        for x in 0..4 {
            for z in 0..3 {
                grid.set_block(IVec3::new(x, 0, z), planks);
            }
        }
        grid.recenter();
        grid.position = Vec3::new(0.0, y, 0.0);
        grid
    }

    fn run(grid: &mut SubGrid, world: &World, input: ShipInput, seconds: f32) -> StepReport {
        let mut last = StepReport::default();
        for _ in 0..(seconds * 60.0) as usize {
            last = step(grid, world, input, 1.0 / 60.0);
        }
        last
    }

    #[test]
    fn a_raft_falls_and_rests_on_the_ground() {
        let world = arena(false);
        let mut grid = raft(10.5);
        let report = run(&mut grid, &world, ShipInput::default(), 3.0);
        assert!(report.grounded);
        // Le pivot est au centre du bloc (0,5 au-dessus du dessus du sol, qui est à y = 1).
        assert!(
            (grid.position.y - 1.5).abs() < 0.05,
            "y = {}",
            grid.position.y
        );
        assert!(grid.velocity.length() < 0.1);
    }

    #[test]
    fn a_raft_floats_on_water_around_the_waterline() {
        let world = arena(true);
        let mut grid = raft(5.0);
        run(&mut grid, &world, ShipInput::default(), 8.0);
        // Il flotte : ni au fond (y = 1,5), ni sorti de l'eau. Le dessus de l'eau est à y = 7.
        assert!(
            grid.position.y > 5.0 && grid.position.y < 7.2,
            "y = {}",
            grid.position.y
        );
        assert!(grid.velocity.y.abs() < 0.2, "il se stabilise");
    }

    #[test]
    fn balloons_lift_a_heavy_structure() {
        let world = arena(false);
        let mut grid = raft(1.5);
        let balloon = grid.registry().id("core:balloon").unwrap();
        // Un ballon par bloc du dessus : 12 ballons (portance 48) pour 24 blocs (masse 24).
        for x in 0..4 {
            for z in 0..3 {
                grid.set_block(IVec3::new(x, 1, z), balloon);
            }
        }
        grid.recenter();
        grid.position.y = 2.0;
        run(&mut grid, &world, ShipInput::default(), 3.0);
        assert!(grid.position.y > 6.0, "il monte : {}", grid.position.y);
    }

    /// Une hélice qui regarde vers +x, reliée à un « moteur » : on règle sa vitesse à la main.
    fn with_propeller(grid: &mut SubGrid, at: IVec3, speed: f32) {
        let propeller = grid.registry().id("core:propeller").unwrap();
        grid.set_block(at, propeller);
        let mut entity = placement_state("core:propeller", Vec3::X).unwrap();
        entity.caps_mut().kinetic = Some(crate::Kinetic {
            speed,
            stress_capacity: 0.0,
            stress_impact: 96.0,
            sides: crate::Sides::ALL,
        });
        grid.set_block_entity(at, Some(entity));
    }

    #[test]
    fn a_propeller_pushes_forward_and_an_off_centre_one_turns_the_structure() {
        let world = arena(true);
        // Au centre (z = 1) : pousse tout droit, sans tourner.
        let mut straight = raft(6.0);
        with_propeller(&mut straight, IVec3::new(0, 1, 1), 64.0);
        straight.recenter();
        run(&mut straight, &world, ShipInput::default(), 4.0);
        assert!(
            straight.velocity.x > 1.0,
            "avance : {}",
            straight.velocity.x
        );
        assert!(straight.yaw.abs() < 0.2, "tout droit : {}", straight.yaw);
        // Sans rotation reçue : l'hélice ne pousse pas.
        let mut idle = raft(6.0);
        with_propeller(&mut idle, IVec3::new(0, 1, 1), 0.0);
        idle.recenter();
        run(&mut idle, &world, ShipInput::default(), 4.0);
        assert!(idle.velocity.x.abs() < 0.2);
        // Sur le côté (z = 0) : elle fait aussi tourner (le couple autour du pivot).
        let mut side = raft(6.0);
        with_propeller(&mut side, IVec3::new(3, 1, 0), 64.0);
        side.recenter();
        run(&mut side, &world, ShipInput::default(), 3.0);
        assert!(side.yaw.abs() > 0.1, "tourne : {}", side.yaw);
    }

    #[test]
    fn the_helm_turns_the_raft_and_the_hull_keeps_the_sign() {
        let world = arena(true);
        let mut grid = raft(6.0);
        run(&mut grid, &world, ShipInput { steer: 1.0 }, 2.0);
        assert!(grid.yaw > 0.3, "tourne à droite : {}", grid.yaw);
        let mut left = raft(6.0);
        run(&mut left, &world, ShipInput { steer: -1.0 }, 2.0);
        assert!(left.yaw < -0.3, "tourne à gauche : {}", left.yaw);
        // Les frottements arrêtent la rotation quand on lâche la barre.
        run(&mut grid, &world, ShipInput::default(), 5.0);
        assert!(grid.yaw_rate.abs() < 0.05);
    }

    #[test]
    fn terrain_blocks_the_structure_and_stops_its_speed() {
        let mut world = arena(false);
        // Un mur de pierre devant : x = 6, de y = 1 à 6, large.
        let stone = id(&world, "core:stone");
        for y in 1..=6 {
            for z in -10..10 {
                world.set_block(IVec3::new(6, y, z), stone);
            }
        }
        let mut grid = raft(1.5);
        with_propeller(&mut grid, IVec3::new(0, 1, 1), 64.0);
        grid.recenter();
        grid.position.y = 1.5;
        run(&mut grid, &world, ShipInput::default(), 6.0);
        // Le mur est à x = 6 : le dernier bloc (à 3,5 du pivot, demi-bloc de large) s'arrête avant.
        let front = grid.position.x + (4.0 - grid.pivot.x);
        assert!(front <= 6.05, "ne traverse pas : avant = {front}");
        assert!(grid.velocity.x.abs() < 0.5, "arrêté : {}", grid.velocity.x);
    }
}
