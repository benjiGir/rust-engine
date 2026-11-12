//! Les objets posés au sol : une entité ECS par pile, avec sa physique et son modèle.
//!
//! Un bloc cassé (ou un coffre détruit) lâche ses objets par terre. Ils tombent avec la
//! même physique que les créatures (une petite boîte), tournent sur eux-mêmes, et le
//! joueur les ramasse en passant à côté. Au bout de 5 minutes, ils disparaissent.

use engine_ecs::{Entity, World};
use engine_render::CubeInstance;
use engine_voxel::ItemRegistry;
use glam::Vec3;
use std::{cell::RefCell, collections::HashMap};

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

#[derive(Default)]
struct Models(RefCell<HashMap<engine_voxel::ItemId, Vec<CubeInstance>>>);

pub fn clear_models(world: &World) {
    if let Some(models) = world.get_resource::<Models>() {
        models.0.borrow_mut().clear();
    }
}

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
    if world.get_resource::<Models>().is_none() {
        world.insert_resource(Models::default());
    }
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

/// Les vrais modèles texturés, qui tournent et flottent doucement.
pub fn instances(
    world: &World,
    items: &ItemRegistry,
    blocks: &engine_voxel::BlockRegistry,
    alpha: f32,
    time: f32,
) -> Vec<CubeInstance> {
    let Some(models) = world.get_resource::<Models>() else {
        return Vec::new();
    };
    let mut models = models.0.borrow_mut();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut out = Vec::new();
    world
        .query::<(&Transform, &PreviousPosition, &DroppedItem)>()
        .for_each(|_, (transform, previous, drop)| {
            let position = previous.0.lerp(transform.position, alpha);
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
            let yaw = transform.yaw + time * 1.5;
            let center = position + Vec3::Y * (size / 2.0 + bob);
            let parts = models
                .entry(drop.stack.item)
                .or_insert_with(|| crate::item_models::build(drop.stack.item, items, blocks));
            for part in parts {
                let mut instance = *part;
                instance.center = (center
                    + engine_voxel::rotate_y(Vec3::from_array(part.center) * size, yaw))
                .to_array();
                instance.size = (Vec3::from_array(part.size) * size).to_array();
                instance.yaw = yaw;
                instance.light = [sky as f32 / 15.0, block as f32 / 15.0];
                out.push(instance);
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_drops_use_their_material_and_cache_is_invalidated() {
        use engine_voxel::{BlockRegistry, ChunkStreamer, StreamingConfig, TerrainGenerator};
        use std::rc::Rc;
        let registry = BlockRegistry::builtin();
        let items = ItemRegistry::builtin(&registry).unwrap();
        let mut world = World::new();
        world.insert_resource(crate::shared::SharedStreamer(Rc::new(RefCell::new(
            ChunkStreamer::new(TerrainGenerator::new(7), StreamingConfig::default()),
        ))));
        let stack = ItemStack {
            item: items.id("core:grass").unwrap(),
            count: 3,
            damage: 7,
        };
        let entity = spawn(&mut world, stack, Vec3::new(1.0, 20.0, 1.0), 42);
        let models = instances(&world, &items, &registry, 0.5, 1.0);
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].textures,
            registry
                .def(registry.id("core:grass").unwrap())
                .face_textures
        );
        assert_eq!(world.get::<DroppedItem>(entity).unwrap().stack, stack);
        assert!(!world.resource::<Models>().0.borrow().is_empty());
        clear_models(&world);
        assert!(world.resource::<Models>().0.borrow().is_empty());
        assert_eq!(
            instances(&world, &items, &registry, 0.5, 1.0)[0].textures,
            models[0].textures
        );
    }

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
