//! Les objets posés au sol : une entité ECS par pile, avec sa physique et son modèle.
//!
//! Un bloc cassé (ou un coffre détruit) lâche ses objets par terre. Ils tombent avec la
//! même physique que les créatures (une petite boîte), tournent sur eux-mêmes, et le
//! joueur les ramasse en passant à côté. Au bout de 5 minutes, ils disparaissent.

use engine_ecs::{Entity, World};
use engine_render::CubeInstance;
use engine_voxel::ItemRegistry;
use glam::Vec3;

use crate::components::{Collider, MoveIntent, PreviousPosition, Transform, Velocity};
use crate::inventory::ItemStack;
use crate::shared::StreamerExt;

/// On ne ramasse pas un objet tout juste lâché : le temps de le voir tomber.
const PICKUP_DELAY: f32 = 0.6;
/// Distance de ramassage (des pieds du joueur au centre de l'objet).
pub const PICKUP_RADIUS: f32 = 1.6;
/// Durée de vie au sol, en secondes.
const LIFETIME: f32 = 300.0;
const SIZE: f32 = 0.28;

/// Un objet au sol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroppedItem {
    pub stack: ItemStack,
    /// Depuis combien de secondes il est au sol.
    pub age: f32,
}

impl Default for DroppedItem {
    fn default() -> Self {
        Self {
            stack: ItemStack::new(engine_voxel::ItemId(0), 0),
            age: 0.0,
        }
    }
}

/// Lâche une pile au centre du bloc `center`, avec un petit saut au hasard.
pub fn spawn(world: &mut World, stack: ItemStack, center: Vec3, seed: u64) -> Entity {
    let r = |shift: u32| ((seed >> shift) & 0xFF) as f32 / 255.0 - 0.5;
    let velocity = Vec3::new(r(0) * 2.0, 3.0, r(8) * 2.0);
    let position = center - Vec3::Y * (SIZE / 2.0);
    world.spawn((
        Transform {
            position,
            yaw: r(16) * 6.0,
        },
        PreviousPosition(position),
        Velocity(velocity),
        Collider::new(SIZE, SIZE),
        MoveIntent::default(),
        DroppedItem { stack, age: 0.0 },
    ))
}

/// Vieillit les objets au sol ; renvoie ceux assez proches de `feet` pour être ramassés.
/// Ceux trop vieux sont supprimés.
pub fn update(world: &mut World, feet: Vec3, dt: f32) -> Vec<(Entity, ItemStack)> {
    let mut pickable = Vec::new();
    let mut expired = Vec::new();
    world
        .query::<(&Transform, &mut DroppedItem)>()
        .for_each(|entity, (transform, drop)| {
            drop.age += dt;
            if drop.age > LIFETIME {
                expired.push(entity);
            } else if drop.age > PICKUP_DELAY
                && (transform.position + Vec3::Y * SIZE / 2.0).distance(feet + Vec3::Y * 0.5)
                    < PICKUP_RADIUS
            {
                pickable.push((entity, drop.stack));
            }
        });
    for entity in expired {
        world.despawn(entity);
    }
    pickable
}

/// Les modèles des objets au sol : un petit cube de la couleur de l'objet, qui tourne
/// et flotte doucement.
pub fn instances(world: &World, items: &ItemRegistry, alpha: f32, time: f32) -> Vec<CubeInstance> {
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut out = Vec::new();
    world
        .query::<(&Transform, &PreviousPosition, &DroppedItem)>()
        .for_each(|_, (transform, previous, drop)| {
            let position = previous.0.lerp(transform.position, alpha);
            let def = items.get(drop.stack.item);
            let (sky, block) = streamer
                .world()
                .light((position + Vec3::Y * 0.2).floor().as_ivec3());
            let bob = (time * 2.5 + transform.yaw).sin() * 0.05 + 0.08;
            // Une pile de plusieurs objets : un cube un peu plus gros.
            let size = if drop.stack.count > 1 {
                SIZE * 1.15
            } else {
                SIZE
            };
            let [r, g, b] = def.color;
            out.push(
                CubeInstance::new(
                    position + Vec3::Y * (size / 2.0 + bob),
                    Vec3::splat(size),
                    Vec3::new(r, g, b),
                )
                .with_yaw(transform.yaw + time * 1.5)
                .with_light(sky as f32 / 15.0, block as f32 / 15.0),
            );
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_wait_before_being_picked_up_and_expire() {
        let mut world = World::new();
        let stack = ItemStack::new(engine_voxel::ItemId(3), 2);
        let entity = world.spawn((Transform::default(), DroppedItem { stack, age: 0.0 }));
        assert!(update(&mut world, Vec3::ZERO, 0.1).is_empty(), "trop tôt");
        assert_eq!(update(&mut world, Vec3::ZERO, 1.0), vec![(entity, stack)]);
        assert!(
            update(&mut world, Vec3::new(10.0, 0.0, 0.0), 1.0).is_empty(),
            "trop loin"
        );
        update(&mut world, Vec3::new(10.0, 0.0, 0.0), LIFETIME);
        assert!(!world.is_alive(entity), "disparu au bout de 5 minutes");
    }
}
