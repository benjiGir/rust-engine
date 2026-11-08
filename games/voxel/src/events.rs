//! Les événements du jeu. Envoyés par les systèmes, lus par qui veut : le son, les
//! statistiques, et plus tard les mods (phase 9b). Chaque type est rangé comme
//! ressource du monde ECS dans un `Events<T>` (double tampon, voir `engine_ecs`).

use engine_ecs::{Entity, Events, World};
use engine_voxel::BlockId;
use glam::IVec3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockBroken {
    pub pos: IVec3,
    pub block: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockPlaced {
    pub pos: IVec3,
    pub block: BlockId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpawned {
    pub entity: Entity,
    /// Identifiant de la créature, par exemple `core:pig`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityDied {
    pub entity: Entity,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerDamaged {
    pub amount: f32,
    pub health_left: f32,
}

/// Un pas de simulation (60 par seconde).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick(pub u64);

/// Range un `Events<T>` vide pour chaque type d'événement.
pub fn register(world: &mut World) {
    world.insert_resource(Events::<BlockBroken>::default());
    world.insert_resource(Events::<BlockPlaced>::default());
    world.insert_resource(Events::<EntitySpawned>::default());
    world.insert_resource(Events::<EntityDied>::default());
    world.insert_resource(Events::<PlayerDamaged>::default());
    world.insert_resource(Events::<Tick>::default());
}

/// Fin d'image : chaque file d'événements tourne ses deux tampons.
pub fn update(world: &World) {
    world.resource_mut::<Events<BlockBroken>>().update();
    world.resource_mut::<Events<BlockPlaced>>().update();
    world.resource_mut::<Events<EntitySpawned>>().update();
    world.resource_mut::<Events<EntityDied>>().update();
    world.resource_mut::<Events<PlayerDamaged>>().update();
    world.resource_mut::<Events<Tick>>().update();
}

/// Envoie un événement : `send(&world, BlockBroken { .. })`.
pub fn send<T: 'static>(world: &World, event: T) {
    world.resource_mut::<Events<T>>().send(event);
}
