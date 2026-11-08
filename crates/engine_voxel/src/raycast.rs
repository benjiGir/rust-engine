//! Lancer de rayon dans la grille de blocs : quel bloc le joueur vise-t-il ?
//!
//! Algorithme DDA d'Amanatides et Woo (1987). Plutôt que d'avancer le long du rayon
//! par petits pas (lent et imprécis), on saute directement d'une frontière de bloc
//! à la suivante. À chaque étape, on traverse la frontière la plus proche parmi
//! les trois axes : on visite ainsi exactement les blocs que le rayon traverse,
//! dans l'ordre, sans en sauter un seul.

use glam::{IVec3, Vec3};

use crate::world::World;

/// Le bloc touché par un rayon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Position monde du bloc touché.
    pub block_pos: IVec3,
    /// Normale de la face touchée (ex. `IVec3::Y` pour le dessus).
    /// `block_pos + normal` est la case où poser un nouveau bloc.
    /// Vaut zéro si le rayon part de l'intérieur d'un bloc plein.
    pub normal: IVec3,
    /// Distance parcourue depuis l'origine jusqu'à la face touchée.
    pub distance: f32,
}

/// Le premier bloc plein touché par le rayon `origin + t · dir`, avec `t <= max_distance`.
pub fn raycast(world: &World, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<RayHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }

    let mut block = origin.floor().as_ivec3();
    // Sens de parcours sur chaque axe : -1, 0 ou +1.
    // Attention : `f32::signum(0.0)` vaut 1.0 en Rust, d'où ce calcul à la main.
    let step = IVec3::new(sign(dir.x), sign(dir.y), sign(dir.z));
    // Distance (le long du rayon) pour traverser un bloc entier sur chaque axe.
    // Un axe que le rayon ne parcourt pas ne sera jamais choisi : distance infinie.
    let t_delta = Vec3::new(inverse_abs(dir.x), inverse_abs(dir.y), inverse_abs(dir.z));
    // Distance jusqu'à la première frontière sur chaque axe.
    let mut t_max = Vec3::ZERO;
    for axis in 0..3 {
        t_max[axis] = if step[axis] > 0 {
            (block[axis] as f32 + 1.0 - origin[axis]) * t_delta[axis]
        } else if step[axis] < 0 {
            (origin[axis] - block[axis] as f32) * t_delta[axis]
        } else {
            f32::INFINITY
        };
    }

    let mut normal = IVec3::ZERO;
    let mut distance = 0.0;
    loop {
        // Seuls les blocs « sélectionnables » arrêtent le rayon (pas l'air, par exemple).
        if world.is_selectable(block) {
            return Some(RayHit {
                block_pos: block,
                normal,
                distance,
            });
        }
        // L'axe dont la prochaine frontière est la plus proche.
        let axis = if t_max.x < t_max.y {
            if t_max.x < t_max.z { 0 } else { 2 }
        } else if t_max.y < t_max.z {
            1
        } else {
            2
        };
        distance = t_max[axis];
        if distance > max_distance {
            return None;
        }
        block[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        // On est entré par la face opposée au sens de marche.
        normal = IVec3::ZERO;
        normal[axis] = -step[axis];
    }
}

/// La distance à laquelle le rayon `origin + t · dir` entre dans la boîte `[min, max]`,
/// ou `None` s'il la manque (ou si elle est derrière). Sert à viser les créatures.
///
/// Méthode des « tranches » (slabs) : sur chaque axe, le rayon est entre les deux plans
/// de la boîte pendant un intervalle de `t`. Il est dans la boîte là où les trois
/// intervalles se chevauchent.
pub fn ray_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut t_enter = 0.0f32;
    let mut t_exit = f32::INFINITY;
    for axis in 0..3 {
        if dir[axis].abs() < 1e-8 {
            // Parallèle à ces plans : il faut déjà être entre les deux.
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let t1 = (min[axis] - origin[axis]) / dir[axis];
        let t2 = (max[axis] - origin[axis]) / dir[axis];
        t_enter = t_enter.max(t1.min(t2));
        t_exit = t_exit.min(t1.max(t2));
        if t_enter > t_exit {
            return None;
        }
    }
    Some(t_enter)
}

fn sign(x: f32) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

fn inverse_abs(x: f32) -> f32 {
    if x == 0.0 {
        f32::INFINITY
    } else {
        1.0 / x.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::stone;

    fn world_with(blocks: &[IVec3]) -> World {
        let mut world = World::new();
        for &p in blocks {
            world.set_block(p, stone());
        }
        world
    }

    #[test]
    fn hits_along_each_axis() {
        let world = world_with(&[
            IVec3::new(5, 0, 0),
            IVec3::new(0, -3, 0),
            IVec3::new(0, 0, 4),
        ]);
        let origin = Vec3::splat(0.5);

        let hit = raycast(&world, origin, Vec3::X, 10.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal),
            (IVec3::new(5, 0, 0), IVec3::NEG_X)
        );
        assert!((hit.distance - 4.5).abs() < 1e-5);

        let hit = raycast(&world, origin, Vec3::NEG_Y, 10.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal),
            (IVec3::new(0, -3, 0), IVec3::Y)
        );
        assert!((hit.distance - 2.5).abs() < 1e-5);

        let hit = raycast(&world, origin, Vec3::Z, 10.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal),
            (IVec3::new(0, 0, 4), IVec3::NEG_Z)
        );
    }

    #[test]
    fn hits_along_a_diagonal() {
        let world = world_with(&[IVec3::new(3, 3, 0)]);
        // Rayon un peu décalé pour ne pas passer exactement par les arêtes.
        let hit = raycast(
            &world,
            Vec3::new(0.5, 0.4, 0.5),
            Vec3::new(1.0, 1.0, 0.0),
            10.0,
        )
        .unwrap();
        assert_eq!(hit.block_pos, IVec3::new(3, 3, 0));
        // Le rayon monte un peu moins vite qu'il n'avance : il entre par la face -X ou -Y.
        assert!(hit.normal == IVec3::NEG_X || hit.normal == IVec3::NEG_Y);
    }

    #[test]
    fn adjacent_block_is_hit_right_away() {
        let world = world_with(&[IVec3::new(1, 0, 0)]);
        let hit = raycast(&world, Vec3::new(0.9, 0.5, 0.5), Vec3::X, 6.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal),
            (IVec3::new(1, 0, 0), IVec3::NEG_X)
        );
        assert!((hit.distance - 0.1).abs() < 1e-5);
        // La case où poser un bloc est celle de l'origine.
        assert_eq!(hit.block_pos + hit.normal, IVec3::ZERO);
    }

    #[test]
    fn nothing_in_range() {
        let world = world_with(&[IVec3::new(10, 0, 0)]);
        assert_eq!(raycast(&world, Vec3::splat(0.5), Vec3::X, 6.0), None);
        assert_eq!(raycast(&world, Vec3::splat(0.5), Vec3::NEG_X, 100.0), None);
        assert_eq!(raycast(&world, Vec3::splat(0.5), Vec3::ZERO, 100.0), None);
    }

    #[test]
    fn negative_coordinates_work() {
        let world = world_with(&[IVec3::new(-4, -1, -2)]);
        let origin = Vec3::new(-0.5, -0.5, -1.5);
        let hit = raycast(&world, origin, Vec3::NEG_X, 10.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal),
            (IVec3::new(-4, -1, -2), IVec3::X)
        );
    }

    #[test]
    fn ray_hits_and_misses_boxes() {
        let (min, max) = (Vec3::new(4.0, 0.0, -0.5), Vec3::new(5.0, 2.0, 0.5));
        let t = ray_aabb(Vec3::new(0.0, 1.0, 0.0), Vec3::X, min, max).unwrap();
        assert!((t - 4.0).abs() < 1e-6);
        assert_eq!(
            ray_aabb(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_X, min, max),
            None,
            "derrière"
        );
        assert_eq!(
            ray_aabb(Vec3::new(0.0, 3.0, 0.0), Vec3::X, min, max),
            None,
            "au-dessus"
        );
        // Depuis l'intérieur : distance nulle.
        assert_eq!(
            ray_aabb(Vec3::new(4.5, 1.0, 0.0), Vec3::X, min, max),
            Some(0.0)
        );
    }

    #[test]
    fn origin_inside_a_block() {
        let world = world_with(&[IVec3::ZERO]);
        let hit = raycast(&world, Vec3::splat(0.5), Vec3::X, 6.0).unwrap();
        assert_eq!(
            (hit.block_pos, hit.normal, hit.distance),
            (IVec3::ZERO, IVec3::ZERO, 0.0)
        );
    }
}
