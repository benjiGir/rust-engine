//! Les événements du jeu. Envoyés par les systèmes, lus par qui veut : le son, les
//! statistiques, et les mods (voir `modding.rs`). Chaque type est rangé comme
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
    /// Où elle est morte (pour les mods : un butin, une explosion...).
    pub position: glam::Vec3,
}

/// Une créature frappe le joueur : le jeu en fait un coup (armure, invulnérabilité),
/// puis un [`PlayerDamaged`] s'il porte.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAttacked {
    pub amount: f32,
    /// D'où vient le coup (la créature, le projectile) : le joueur recule à l'opposé.
    pub from: glam::Vec3,
    /// Les points d'armure que ce coup ignore (la perforation, voir `attributes.rs`).
    pub pierce: f32,
}

/// Le joueur a perdu de la vie (après l'armure).
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
    world.insert_resource(Events::<PlayerAttacked>::default());
    world.insert_resource(Events::<PlayerDamaged>::default());
    world.insert_resource(Events::<Tick>::default());
}

/// Fin d'image : chaque file d'événements tourne ses deux tampons.
pub fn update(world: &World) {
    world.resource_mut::<Events<BlockBroken>>().update();
    world.resource_mut::<Events<BlockPlaced>>().update();
    world.resource_mut::<Events<EntitySpawned>>().update();
    world.resource_mut::<Events<EntityDied>>().update();
    world.resource_mut::<Events<PlayerAttacked>>().update();
    world.resource_mut::<Events<PlayerDamaged>>().update();
    world.resource_mut::<Events<Tick>>().update();
}

/// Envoie un événement : `send(&world, BlockBroken { .. })`.
pub fn send<T: 'static>(world: &World, event: T) {
    world.resource_mut::<Events<T>>().send(event);
}
