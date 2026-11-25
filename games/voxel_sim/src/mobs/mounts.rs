//! Monter à cheval : apprivoiser, seller, monter, diriger, descendre.
//!
//! Une créature qui a `mount` dans `entities.ron` (le cheval) peut porter un joueur :
//!
//! ```ron
//! mount: (speed: (6.0, 10.5), jump: (1.3, 2.6), tries: (2, 5), seat: (-0.1, 1.15)),
//! ```
//!
//! - `speed` : sa vitesse au galop, en blocs par seconde (min, max) ; chaque cheval tire la sienne ;
//! - `jump` : la hauteur de son saut, en blocs (min, max) ; elle aussi tirée à la naissance ;
//! - `tries` : combien d'essais un cheval sauvage exige avant d'accepter qu'on le monte (min, max) ;
//! - `seat` : où s'assoit le cavalier, par rapport aux pieds du cheval (vers l'avant, hauteur).
//!
//! # Le déroulé
//!
//! 1. **Apprivoiser** : un clic droit à mains nues sur un cheval sauvage. Il se cabre et jette le
//!    joueur par terre ([`Mounting::Bucked`]), jusqu'à ce que le compte d'essais soit atteint : il
//!    devient alors [`Tamed`] ([`Mounting::Tamed`]).
//! 2. **Seller** : une [`core:saddle`] en main sur un cheval apprivoisé le rend [`Saddled`]. Sans selle,
//!    on ne le monte pas ([`Mounting::NeedsSaddle`]).
//! 3. **Monter** : un clic droit sur un cheval sellé. Le joueur reçoit le composant `Riding` (le même
//!    que pour un wagonnet : la fenêtre le voit « porté », le serveur le simule) et le cheval reçoit
//!    [`Ridden`].
//! 4. **Diriger** : [`steer`] lit les touches du cavalier (son `MoveIntent`) et en fait celles du
//!    cheval. Il tourne doucement vers la direction demandée et avance dans le sens où il regarde ; la
//!    marche donne un trot, la course un galop ; l'espace le fait sauter selon sa [`Gait::jump`].
//! 5. **Descendre** : Maj ([`dismount`]) : le joueur se retrouve à côté du cheval.
//!
//! Le serveur fait foi : tout est calculé ici, la fenêtre ne fait que suivre (comme pour les wagons).
//! Les créatures ne sont pas sauvegardées : un cheval apprivoisé le reste tant que la partie dure.

use engine_ecs::{Entity, World};
use glam::Vec3;
use serde::Deserialize;
use std::sync::Arc;

use crate::components::{Baby, Collider, Mob, MoveIntent, Transform, Velocity, Wander};
use crate::mobs::{FixedDt, MobRegistry};
use crate::shared::StreamerExt as _;
use crate::vehicles::carts::Riding;

/// Ce que `mount` écrit dans `entities.ron`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountDef {
    /// La vitesse de galop (min, max), en blocs par seconde.
    pub speed: (f32, f32),
    /// La hauteur de saut (min, max), en blocs.
    pub jump: (f32, f32),
    /// Les essais nécessaires pour l'apprivoiser (min, max).
    pub tries: (u8, u8),
    /// Le siège : décalage vers l'avant et hauteur, depuis les pieds.
    pub seat: (f32, f32),
}

/// Le trot (le joueur marche) est cette part du galop.
const TROT: f32 = 0.6;
/// Au-delà de cette vitesse de marche voulue (blocs par seconde), le joueur court : le cheval galope.
const GALLOP_FROM: f32 = crate::player::WALK_SPEED * 1.05;
/// Un cheval tourne à cette vitesse (radians par seconde).
const TURN_RATE: f32 = 4.0;
/// Un poulain tient la moyenne de ses parents, à ce quart de l'écart (min, max) près.
const FOAL_SPREAD: f32 = 0.25;

/// Ce qui fait un bon cheval : sa vitesse de galop et la hauteur de son saut. Chaque cheval a la sienne.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gait {
    /// Le galop, en blocs par seconde.
    pub speed: f32,
    /// La hauteur de son saut, en blocs.
    pub jump: f32,
}

fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t.clamp(0.0, 1.0)
}

impl Gait {
    /// Une allure tirée au sort : `roll_speed` et `roll_jump` dans [0, 1[.
    pub fn roll(def: &MountDef, roll_speed: f32, roll_jump: f32) -> Self {
        Self {
            speed: lerp(def.speed, roll_speed),
            jump: lerp(def.jump, roll_jump),
        }
    }

    /// Celle d'un poulain : la moyenne de ses parents, un peu déviée au hasard (`spread_*` dans
    /// [0, 1[ : 0,5 ne change rien). Les bons chevaux font de bons poulains, sans jamais sortir des
    /// bornes du jeu.
    pub fn foal(def: &MountDef, a: Self, b: Self, spread_speed: f32, spread_jump: f32) -> Self {
        let mix = |mean: f32, range: (f32, f32), spread: f32| {
            let drift = (spread - 0.5) * 2.0 * FOAL_SPREAD * (range.1 - range.0);
            (mean + drift).clamp(range.0, range.1)
        };
        Self {
            speed: mix((a.speed + b.speed) * 0.5, def.speed, spread_speed),
            jump: mix((a.jump + b.jump) * 0.5, def.jump, spread_jump),
        }
    }

    /// La vitesse verticale qui fait monter ce cheval de `jump` blocs sous cette gravité (v² = 2 g h).
    pub fn jump_speed(self, gravity: f32) -> f32 {
        (2.0 * gravity * self.jump).sqrt()
    }
}

/// Un cheval encore sauvage : il se laisse monter au bout de `needed` essais.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Temper {
    pub tries: u8,
    pub needed: u8,
}

impl Temper {
    pub fn roll(def: &MountDef, roll: f32) -> Self {
        let (min, max) = def.tries;
        let span = max.saturating_sub(min) as f32 + 1.0;
        Self {
            tries: 0,
            needed: min + ((roll.clamp(0.0, 0.999_999) * span) as u8).min(max.saturating_sub(min)),
        }
    }
}

/// Un cheval apprivoisé : il accepte le joueur (et ne disparaît plus quand on s'éloigne).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tamed;

/// Un cheval sellé : on peut le monter et le diriger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saddled;

/// Le cavalier d'un cheval (l'entité du joueur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ridden(pub Entity);

/// Ce qui arrive à un joueur qui tente de monter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mounting {
    /// Il est en selle.
    Mounted,
    /// Le cheval sauvage se cabre : `tries` essais faits sur `needed`.
    Bucked { tries: u8, needed: u8 },
    /// Le cheval accepte le joueur : il est apprivoisé.
    Tamed,
    /// La selle est posée (le serveur la retire de la main du joueur).
    Saddled,
    /// Apprivoisé, mais sans selle : pas de monte.
    NeedsSaddle,
    /// Un poulain ne porte personne.
    TooYoung,
    /// Un autre joueur le monte déjà.
    Taken,
    /// Ce n'est pas une monture.
    NotAMount,
}

/// La monture décrite pour cette créature, si elle en est une.
pub fn def_of(world: &World, entity: Entity) -> Option<MountDef> {
    let kind = world.get::<Mob>(entity)?.kind;
    let registry = world.resource::<Arc<MobRegistry>>();
    registry.get(kind)?.mount
}

/// Donne à une créature qui naît son allure et son caractère (`def.mount`). `born_of` : l'allure de
/// ses deux parents, pour un poulain.
pub fn init(world: &mut World, entity: Entity, born_of: Option<(Gait, Gait)>) {
    let Some(def) = def_of(world, entity) else {
        return;
    };
    let rolls = {
        let mut rng = world.resource_mut::<crate::mobs::GameRng>();
        [rng.next_f32(), rng.next_f32(), rng.next_f32()]
    };
    let gait = match born_of {
        Some((a, b)) => Gait::foal(&def, a, b, rolls[0], rolls[1]),
        None => Gait::roll(&def, rolls[0], rolls[1]),
    };
    world.insert(entity, gait);
    world.insert(entity, Temper::roll(&def, rolls[2]));
}

/// `rider` tente de monter `horse` (`saddle` : il tient une selle en main). Voir le déroulé en tête
/// de ce fichier.
pub fn try_mount(world: &mut World, rider: Entity, horse: Entity, saddle: bool) -> Mounting {
    if def_of(world, horse).is_none() || !world.has::<Gait>(horse) {
        return Mounting::NotAMount;
    }
    if world.has::<Baby>(horse) {
        return Mounting::TooYoung;
    }
    if world.has::<Ridden>(horse) {
        return Mounting::Taken;
    }
    if !world.has::<Tamed>(horse) {
        let Some(mut temper) = world.get_mut::<Temper>(horse) else {
            return Mounting::NotAMount;
        };
        temper.tries += 1;
        let (tries, needed) = (temper.tries, temper.needed);
        drop(temper);
        if tries >= needed {
            world.remove::<Temper>(horse);
            world.insert(horse, Tamed);
            return Mounting::Tamed;
        }
        // Il se cabre et s'agite un moment.
        if let Some(mut wander) = world.get_mut::<Wander>(horse) {
            wander.panic = 1.5;
        }
        return Mounting::Bucked { tries, needed };
    }
    if !world.has::<Saddled>(horse) {
        if saddle {
            world.insert(horse, Saddled);
            return Mounting::Saddled;
        }
        return Mounting::NeedsSaddle;
    }
    board(world, rider, horse);
    Mounting::Mounted
}

/// Fait asseoir `rider` sur `horse` : le joueur est « porté », le cheval « monté ».
fn board(world: &mut World, rider: Entity, horse: Entity) {
    // Un joueur déjà assis ailleurs (wagon, autre cheval) en descend d'abord.
    dismount(world, rider);
    crate::vehicles::carts::dismount(world, rider);
    world.insert(rider, Riding(horse));
    world.insert(horse, Ridden(rider));
    if let Some(mut velocity) = world.get_mut::<Velocity>(horse) {
        velocity.0 = Vec3::ZERO;
    }
}

/// Est-ce que `rider` est en selle ?
pub fn is_riding(world: &World, rider: Entity) -> bool {
    world
        .get::<Riding>(rider)
        .is_some_and(|r| world.has::<Ridden>(r.0))
}

/// Où s'assoit le cavalier de `horse` : ses pieds, dans le monde (suit le cap du cheval).
pub fn seat(world: &World, horse: Entity) -> Option<Vec3> {
    let def = def_of(world, horse)?;
    let transform = world.get::<Transform>(horse)?;
    let (forward, up) = def.seat;
    let (sin, cos) = transform.yaw.sin_cos();
    Some(transform.position + Vec3::new(cos * forward, up, sin * forward))
}

/// Le joueur descend : il se pose à côté du cheval, du côté où il y a de la place.
pub fn dismount(world: &mut World, rider: Entity) {
    let Some(Riding(horse)) = world.get::<Riding>(rider).map(|r| *r) else {
        return;
    };
    if !world.has::<Ridden>(horse) {
        // Un wagon : c'est `vehicles/carts.rs` qui le fait descendre.
        return;
    }
    world.remove::<Riding>(rider);
    world.remove::<Ridden>(horse);
    let (at, yaw, width) = match (world.get::<Transform>(horse), world.get::<Collider>(horse)) {
        (Some(t), Some(c)) => (t.position, t.yaw, c.width),
        _ => return,
    };
    let (sin, cos) = yaw.sin_cos();
    // À gauche, à droite, derrière, puis sur place.
    let right = Vec3::new(-sin, 0.0, cos);
    let back = Vec3::new(-cos, 0.0, -sin);
    let reach = width * 0.5 + 0.6;
    let shared = world.streamer();
    let streamer = shared.borrow();
    let free = |p: Vec3| {
        let cell = p.floor().as_ivec3();
        !streamer.world().is_solid(cell) && !streamer.world().is_solid(cell + glam::IVec3::Y)
    };
    let landing = [
        right * reach,
        -right * reach,
        back * (reach + 0.4),
        Vec3::Y * 1.7,
    ]
    .into_iter()
    .map(|d| at + d)
    .find(|&p| free(p))
    .unwrap_or(at + Vec3::Y * 1.7);
    drop(streamer);
    if let Some(mut transform) = world.get_mut::<Transform>(rider) {
        transform.position = landing;
    }
    if let Some(mut previous) = world.get_mut::<crate::components::PreviousPosition>(rider) {
        previous.0 = landing;
    }
    if let Some(mut velocity) = world.get_mut::<Velocity>(rider) {
        velocity.0 = Vec3::ZERO;
    }
    if let Some(mut velocity) = world.get_mut::<Velocity>(horse) {
        velocity.0.x = 0.0;
        velocity.0.z = 0.0;
    }
}

/// Le système (pas fixe, avant la physique) : les touches du cavalier deviennent celles du cheval.
pub fn steer(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    let mut pairs: Vec<(Entity, Entity)> = Vec::new();
    world
        .query::<(&Ridden,)>()
        .for_each(|horse, (ridden,)| pairs.push((horse, ridden.0)));
    for (horse, rider) in pairs {
        // Le cavalier a disparu (déconnecté, descendu ailleurs) : le cheval est libre.
        if world.get::<Riding>(rider).is_none_or(|r| r.0 != horse) || !world.is_alive(rider) {
            world.remove::<Ridden>(horse);
            continue;
        }
        let (Some(gait), Some(keys)) = (
            world.get::<Gait>(horse).map(|g| *g),
            world.get::<MoveIntent>(rider).map(|i| *i),
        ) else {
            continue;
        };
        let wish = Vec3::new(keys.walk.x, 0.0, keys.walk.z);
        let wanted = wish.length();
        let (yaw, on_ground, position, width) = {
            let (Some(t), Some(c)) = (world.get::<Transform>(horse), world.get::<Collider>(horse))
            else {
                continue;
            };
            (t.yaw, c.on_ground, t.position, c.width)
        };
        let saddled = world.has::<Saddled>(horse);
        let (mut walk, mut new_yaw) = (Vec3::ZERO, yaw);
        if saddled && wanted > 0.1 {
            // Tourner vers la direction voulue, puis avancer dans le sens où il regarde : plus il
            // est de travers, moins il avance.
            let target = wish.z.atan2(wish.x);
            let delta = wrap(target - yaw);
            new_yaw = yaw + delta.clamp(-TURN_RATE * dt, TURN_RATE * dt);
            let speed = gait.speed * if wanted > GALLOP_FROM { 1.0 } else { TROT };
            let along = delta.cos().max(0.25);
            walk = Vec3::new(new_yaw.cos(), 0.0, new_yaw.sin()) * speed * along;
        }
        // Sauter : l'espace, ou une marche d'un bloc devant lui.
        let ahead = position + walk.normalize_or_zero() * (width * 0.5 + 0.4) + Vec3::Y * 0.5;
        let blocked = {
            let shared = world.streamer();
            let streamer = shared.borrow();
            walk != Vec3::ZERO && streamer.world().is_solid(ahead.floor().as_ivec3())
        };
        if let Some(mut transform) = world.get_mut::<Transform>(horse) {
            transform.yaw = new_yaw;
        }
        if let Some(mut intent) = world.get_mut::<MoveIntent>(horse) {
            intent.walk = walk;
            intent.jump = on_ground && saddled && (keys.jump || blocked);
        }
        // Il ne pense plus à autre chose.
        if let Some(mut wander) = world.get_mut::<Wander>(horse) {
            wander.panic = 0.0;
            wander.direction = walk.normalize_or_zero();
        }
    }
}

fn wrap(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::PreviousPosition;
    use crate::mobs::tests::sim;

    fn def() -> MountDef {
        MountDef {
            speed: (6.0, 10.0),
            jump: (1.0, 3.0),
            tries: (2, 4),
            seat: (0.0, 1.0),
        }
    }

    fn horse_and_rider(world: &mut World) -> (Entity, Entity) {
        let kind = world
            .resource::<Arc<MobRegistry>>()
            .id("core:horse")
            .unwrap();
        let horse = crate::mobs::spawn_mob(world, kind, Vec3::new(5.5, 60.0, 5.5)).unwrap();
        let rider = world.spawn((
            Transform {
                position: Vec3::new(5.5, 60.0, 7.5),
                yaw: 0.0,
            },
            PreviousPosition(Vec3::ZERO),
            Velocity::default(),
            Collider::new(0.6, 1.8),
            MoveIntent::default(),
        ));
        (horse, rider)
    }

    /// Un cheval est apprivoisé après son compte d'essais, puis sellé, puis monté.
    #[test]
    fn a_wild_horse_needs_its_tries_then_a_saddle() {
        let mut world = sim();
        let (horse, rider) = horse_and_rider(&mut world);
        let needed = world.get::<Temper>(horse).unwrap().needed;
        assert!((2..=5).contains(&needed));
        for tries in 1..needed {
            assert_eq!(
                try_mount(&mut world, rider, horse, true),
                Mounting::Bucked { tries, needed },
                "une selle ne sert à rien sur un cheval sauvage"
            );
        }
        assert_eq!(try_mount(&mut world, rider, horse, false), Mounting::Tamed);
        assert!(world.has::<Tamed>(horse) && !world.has::<Temper>(horse));
        assert_eq!(
            try_mount(&mut world, rider, horse, false),
            Mounting::NeedsSaddle
        );
        assert_eq!(try_mount(&mut world, rider, horse, true), Mounting::Saddled);
        assert_eq!(
            try_mount(&mut world, rider, horse, false),
            Mounting::Mounted
        );
        assert!(is_riding(&world, rider));
        assert_eq!(world.get::<Ridden>(horse).unwrap().0, rider);
        // Pris : personne d'autre ne le monte.
        let other = world.spawn((Transform::default(),));
        assert_eq!(try_mount(&mut world, other, horse, false), Mounting::Taken);
    }

    #[test]
    fn a_foal_cannot_be_ridden_and_a_pig_is_no_mount() {
        let mut world = sim();
        let (horse, rider) = horse_and_rider(&mut world);
        world.insert(horse, Baby { age_left: 10.0 });
        assert_eq!(
            try_mount(&mut world, rider, horse, false),
            Mounting::TooYoung
        );
        let pig_kind = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let pig = crate::mobs::spawn_mob(&mut world, pig_kind, Vec3::new(3.5, 60.0, 3.5)).unwrap();
        assert_eq!(
            try_mount(&mut world, rider, pig, false),
            Mounting::NotAMount
        );
    }

    #[test]
    fn every_horse_gets_a_gait_within_the_data_bounds() {
        let mut world = sim();
        let def = def_of_horse(&mut world);
        for _ in 0..40 {
            let (horse, _) = horse_and_rider(&mut world);
            let gait = *world.get::<Gait>(horse).unwrap();
            assert!((def.speed.0..=def.speed.1).contains(&gait.speed));
            assert!((def.jump.0..=def.jump.1).contains(&gait.jump));
        }
    }

    fn def_of_horse(world: &mut World) -> MountDef {
        let (horse, _) = horse_and_rider(world);
        def_of(world, horse).unwrap()
    }

    #[test]
    fn a_foal_is_between_its_parents_and_inside_the_bounds() {
        let def = def();
        let slow = Gait {
            speed: 6.0,
            jump: 1.0,
        };
        let fast = Gait {
            speed: 10.0,
            jump: 3.0,
        };
        let foal = Gait::foal(&def, slow, fast, 0.5, 0.5);
        assert_eq!(
            foal,
            Gait {
                speed: 8.0,
                jump: 2.0
            }
        );
        // Même au pire tirage, il ne sort pas des bornes.
        let best = Gait::foal(&def, fast, fast, 0.999, 0.999);
        assert_eq!(
            best,
            Gait {
                speed: 10.0,
                jump: 3.0
            }
        );
        let worst = Gait::foal(&def, slow, slow, 0.0, 0.0);
        assert_eq!(
            worst,
            Gait {
                speed: 6.0,
                jump: 1.0
            }
        );
    }

    #[test]
    fn jump_speed_reaches_the_asked_height() {
        let gait = Gait {
            speed: 8.0,
            jump: 2.0,
        };
        let v = gait.jump_speed(engine_voxel::GRAVITY);
        let height = v * v / (2.0 * engine_voxel::GRAVITY);
        assert!((height - 2.0).abs() < 1e-4);
    }

    #[test]
    fn temper_stays_in_its_range() {
        let def = def();
        assert_eq!(Temper::roll(&def, 0.0).needed, 2);
        assert_eq!(Temper::roll(&def, 0.999).needed, 4);
    }

    /// Sellé et monté, le cheval avance dans le sens des touches, plus vite en courant ; sans
    /// touche, il s'arrête ; la selle manquante l'empêche d'obéir.
    #[test]
    fn the_rider_steers_the_horse() {
        let mut world = sim();
        let (horse, rider) = horse_and_rider(&mut world);
        world.insert(horse, Tamed);
        world.insert(horse, Saddled);
        world.insert(
            horse,
            Gait {
                speed: 10.0,
                jump: 2.0,
            },
        );
        assert_eq!(
            try_mount(&mut world, rider, horse, false),
            Mounting::Mounted
        );
        world.get_mut::<Transform>(horse).unwrap().yaw = 0.0;
        world.get_mut::<Collider>(horse).unwrap().on_ground = true;
        // Il regarde vers +x et on veut aller vers +x : il trotte.
        world.get_mut::<MoveIntent>(rider).unwrap().walk = Vec3::X * 4.3;
        steer(&mut world);
        let trot = world.get::<MoveIntent>(horse).unwrap().walk;
        assert!((trot.x - 6.0).abs() < 1e-3 && trot.z.abs() < 1e-3, "{trot}");
        // En courant : le galop.
        world.get_mut::<MoveIntent>(rider).unwrap().walk = Vec3::X * 5.6;
        steer(&mut world);
        assert!((world.get::<MoveIntent>(horse).unwrap().walk.x - 10.0).abs() < 1e-3);
        // Une touche de côté : il tourne d'abord (la cap change d'au plus TURN_RATE · dt).
        world.get_mut::<MoveIntent>(rider).unwrap().walk = Vec3::Z * 4.3;
        steer(&mut world);
        let yaw = world.get::<Transform>(horse).unwrap().yaw;
        assert!(yaw > 0.0 && yaw <= TURN_RATE / 60.0 + 1e-5);
        // Plus de touche : il s'arrête.
        world.get_mut::<MoveIntent>(rider).unwrap().walk = Vec3::ZERO;
        steer(&mut world);
        assert_eq!(world.get::<MoveIntent>(horse).unwrap().walk, Vec3::ZERO);
        // L'espace le fait sauter, au sol seulement.
        world.get_mut::<MoveIntent>(rider).unwrap().jump = true;
        steer(&mut world);
        assert!(world.get::<MoveIntent>(horse).unwrap().jump);
        world.get_mut::<Collider>(horse).unwrap().on_ground = false;
        steer(&mut world);
        assert!(!world.get::<MoveIntent>(horse).unwrap().jump);
    }

    #[test]
    fn the_seat_follows_the_horse_heading() {
        let mut world = sim();
        let (horse, _) = horse_and_rider(&mut world);
        let at = world.get::<Transform>(horse).unwrap().position;
        let seat_def = def_of(&world, horse).unwrap().seat;
        world.get_mut::<Transform>(horse).unwrap().yaw = 0.0;
        let s = seat(&world, horse).unwrap();
        assert!((s - (at + Vec3::new(seat_def.0, seat_def.1, 0.0))).length() < 1e-5);
        world.get_mut::<Transform>(horse).unwrap().yaw = std::f32::consts::FRAC_PI_2;
        let s = seat(&world, horse).unwrap();
        assert!((s - (at + Vec3::new(0.0, seat_def.1, seat_def.0))).length() < 1e-5);
    }

    #[test]
    fn dismounting_frees_both_and_puts_the_rider_beside() {
        let mut world = sim();
        let (horse, rider) = horse_and_rider(&mut world);
        world.insert(horse, Tamed);
        world.insert(horse, Saddled);
        assert_eq!(
            try_mount(&mut world, rider, horse, false),
            Mounting::Mounted
        );
        dismount(&mut world, rider);
        assert!(!world.has::<Riding>(rider) && !world.has::<Ridden>(horse));
        let a = world.get::<Transform>(rider).unwrap().position;
        let b = world.get::<Transform>(horse).unwrap().position;
        assert!(a.distance(b) > 0.9, "il descend à côté, pas dedans");
        // Un cavalier disparu libère le cheval.
        world.insert(rider, Riding(horse));
        world.insert(horse, Ridden(rider));
        world.despawn(rider);
        steer(&mut world);
        assert!(!world.has::<Ridden>(horse));
    }
}
