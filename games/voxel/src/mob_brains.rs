//! Les créatures à comportement : celles dont l'`entities.ron` dit `brain: true` n'ont pas l'IA du
//! jeu. Quatre fois par seconde, le jeu confie au script de leur mod **toutes** celles qui sont
//! près du joueur en un seul appel (un appel du jeu vers le script coûte cher, un ordre du script
//! vers le jeu presque rien), puis applique ses ordres : marcher, sauter, frapper, se souvenir.
//!
//! Le script ne commande que les créatures du lot qu'il vient de recevoir : un ordre pour une
//! autre est ignoré. La physique, les dégâts reçus, la mort, le butin et le rendu restent ceux
//! du jeu.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use engine_ecs::Entity;
use engine_mods::mobs::{self, MobSnapshot};
use engine_voxel::MachineAction;
use glam::Vec3;

use crate::components::{
    BrainGoal, BrainState, Collider, Health, Hurt, Mob, MoveIntent, Transform, Wander,
};
use crate::events::{self, PlayerAttacked};
use crate::mobs::MobRegistry;
use crate::navigation::{DangerBlocks, PathFollow, SEARCHES_PER_STEP, cell_of};
use crate::player;
use crate::shared::StreamerExt;

/// Au bout de cette distance (blocs), le joueur est à portée de bras d'une créature qui frappe.
const REACH: f32 = 3.0;
/// Le temps entre deux coups d'une même créature (secondes).
const ATTACK_COOLDOWN: f32 = 1.0;
/// Le temps entre deux lots (secondes) : 4 fois par seconde.
const BATCH_SECONDS: f32 = 0.25;

/// Les créatures à comportement qui vont quelque part (`mob_move_to`) : un chemin vers leur but
/// (recalculé si elles se coincent), suivi à chaque pas de simulation, entre deux appels du
/// script. Les recherches sont partagées avec le reste des créatures par le même budget par pas.
pub fn follow_goals(world: &mut engine_ecs::World) {
    let dt = world.resource::<crate::mobs::FixedDt>().0;
    let danger = world.resource::<Arc<DangerBlocks>>().clone();
    let shared = world.streamer();
    let streamer = shared.borrow();
    let mut searches = SEARCHES_PER_STEP;
    world
        .query::<(
            &mut Transform,
            &mut MoveIntent,
            &Collider,
            &mut PathFollow,
            &BrainGoal,
        )>()
        .for_each(|_, (transform, intent, collider, follow, goal)| {
            let position = transform.position;
            if searches > 0 && follow.wants(goal.cell, position) {
                follow.plan(
                    streamer.world(),
                    &danger,
                    position,
                    collider.height,
                    goal.cell,
                );
                searches -= 1;
            }
            let (direction, path_jump) = follow.steer(position, dt).unwrap_or((Vec3::ZERO, false));
            intent.walk = direction * goal.speed;
            let ahead = position + direction * (collider.width * 0.5 + 0.4) + Vec3::Y * 0.5;
            let blocked = streamer.world().is_solid(ahead.floor().as_ivec3());
            intent.jump = direction != Vec3::ZERO && (path_jump || blocked);
            if direction != Vec3::ZERO {
                transform.yaw = direction.z.atan2(direction.x);
            }
        });
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Applique les ordres d'un script, seulement aux créatures de son lot.
    fn apply_mob_orders(
        &mut self,
        actions: &[MachineAction],
        batch: &HashMap<u32, Entity>,
        registry: &MobRegistry,
        player_pos: Vec3,
    ) {
        let mut jumped: HashSet<u32> = HashSet::new();
        for action in actions {
            match *action {
                MachineAction::MobMove { mob, dx, dz, speed } => {
                    let Some(&entity) = batch.get(&mob) else {
                        continue;
                    };
                    let Some(kind) = self.ecs.get::<Mob>(entity).map(|m| m.kind) else {
                        continue;
                    };
                    let max = registry.get(kind).map_or(1.0, |d| d.speed) * 2.0;
                    let direction = Vec3::new(dx, 0.0, dz).normalize_or_zero();
                    let walk = direction * speed.clamp(0.0, max);
                    let blocked = {
                        let width = self.ecs.get::<Collider>(entity).map_or(0.6, |c| c.width);
                        let position = self
                            .ecs
                            .get::<Transform>(entity)
                            .map_or(Vec3::ZERO, |t| t.position);
                        let ahead = position + direction * (width * 0.5 + 0.4) + Vec3::Y * 0.5;
                        walk != Vec3::ZERO
                            && self.streamer().world().is_solid(ahead.floor().as_ivec3())
                    };
                    // Un ordre de marche direct annule le trajet en cours.
                    self.ecs.remove::<BrainGoal>(entity);
                    if let Some(mut intent) = self.ecs.get_mut::<MoveIntent>(entity) {
                        intent.walk = walk;
                        // Un obstacle devant : la créature saute (comme celles du jeu).
                        intent.jump = intent.jump || blocked;
                    }
                    if direction != Vec3::ZERO
                        && let Some(mut transform) = self.ecs.get_mut::<Transform>(entity)
                    {
                        transform.yaw = direction.z.atan2(direction.x);
                    }
                }
                MachineAction::MobMoveTo {
                    mob,
                    x,
                    y,
                    z,
                    speed,
                } => {
                    let Some(&entity) = batch.get(&mob) else {
                        continue;
                    };
                    let Some(kind) = self.ecs.get::<Mob>(entity).map(|m| m.kind) else {
                        continue;
                    };
                    let max = registry.get(kind).map_or(1.0, |d| d.speed) * 2.0;
                    self.ecs.insert(
                        entity,
                        BrainGoal {
                            cell: cell_of(Vec3::new(x, y, z)),
                            speed: speed.clamp(0.0, max),
                        },
                    );
                }
                MachineAction::MobJump { mob } => {
                    if let Some(&entity) = batch.get(&mob)
                        && jumped.insert(mob)
                        && let Some(mut intent) = self.ecs.get_mut::<MoveIntent>(entity)
                    {
                        intent.jump = true;
                    }
                }
                MachineAction::MobAttack { mob, damage } => {
                    let Some(&entity) = batch.get(&mob) else {
                        continue;
                    };
                    let position = self
                        .ecs
                        .get::<Transform>(entity)
                        .map_or(Vec3::ZERO, |t| t.position);
                    let ready = self
                        .ecs
                        .get::<Wander>(entity)
                        .is_some_and(|w| w.attack_cooldown <= 0.0);
                    if ready && position.distance(player_pos) <= REACH {
                        if let Some(mut wander) = self.ecs.get_mut::<Wander>(entity) {
                            wander.attack_cooldown = ATTACK_COOLDOWN;
                        }
                        events::send(
                            &self.ecs,
                            PlayerAttacked {
                                player: Some(self.players.local_id()),
                                amount: damage.min(20.0),
                                from: position,
                                pierce: 0.0,
                            },
                        );
                    }
                }
                MachineAction::MobState { mob, value } => {
                    if let Some(&entity) = batch.get(&mob) {
                        self.ecs.insert(entity, BrainState(value));
                    }
                }
                _ => {}
            }
        }
    }

    /// Un tick de machines sur cinq : confie leurs créatures aux scripts, applique leurs ordres.
    pub(crate) fn run_mob_brains(&mut self, machine_tick: bool) {
        if !self.mods.brain_due(machine_tick) {
            return;
        }
        let owners = self.mods.brain_mods();
        if owners.is_empty() {
            return;
        }
        let registry = self.ecs.resource::<Arc<MobRegistry>>().clone();
        let player_pos = player::body(&self.ecs, self.players.entity()).position;
        // Les créatures à comportement proches, par mod : les plus proches d'abord.
        let mut batches: HashMap<String, Vec<(f32, Entity, MobSnapshot)>> = HashMap::new();
        self.ecs
            .query::<(&Transform, &Mob, &Health, &Collider)>()
            .for_each(|entity, (transform, mob, health, collider)| {
                let Some(def) = registry.get(mob.kind).filter(|d| d.brain) else {
                    return;
                };
                let Some(owner) = def
                    .key
                    .split(':')
                    .next()
                    .filter(|o| owners.iter().any(|m| m == o))
                else {
                    return;
                };
                let distance = crate::spawning::horizontal_distance(transform.position, player_pos);
                if distance > crate::spawning::SIM_DISTANCE || collider.frozen {
                    return;
                }
                let to = player_pos - transform.position;
                let mut flags = 0;
                if collider.on_ground {
                    flags |= mobs::FLAG_ON_GROUND;
                }
                if collider.in_liquid {
                    flags |= mobs::FLAG_IN_LIQUID;
                }
                if self.ecs.get::<Hurt>(entity).is_some_and(|h| h.timer > 0.0) {
                    flags |= mobs::FLAG_HURT;
                }
                batches.entry(owner.to_string()).or_default().push((
                    distance,
                    entity,
                    MobSnapshot {
                        id: entity.index,
                        kind: mob.kind,
                        position: transform.position.to_array(),
                        health: health.current,
                        max_health: health.max,
                        to_player: to.to_array(),
                        flags,
                        state: self.ecs.get::<BrainState>(entity).map_or(0, |s| s.0),
                    },
                ));
            });
        // Un mod dont plus aucune créature n'est proche reçoit un lot vide, une fois.
        let present: Vec<String> = batches.keys().cloned().collect();
        for owner in self.mods.take_brain_leavers(&present) {
            let actions = self.mods.mob_brains(&owner, &[], 0);
            self.apply_mob_orders(&actions, &HashMap::new(), &registry, player_pos);
        }
        for (owner, mut batch) in batches {
            batch.sort_by(|a, b| a.0.total_cmp(&b.0));
            batch.truncate(mobs::MAX_PER_CALL);
            let snapshots: Vec<MobSnapshot> = batch.iter().map(|(_, _, s)| *s).collect();
            let entities: HashMap<u32, Entity> = batch.iter().map(|(_, e, s)| (s.id, *e)).collect();
            // Les sauts d'un lot sont refaits par le suivant ; le recul de l'attaque se refroidit.
            for entity in entities.values() {
                if let Some(mut intent) = self.ecs.get_mut::<MoveIntent>(*entity) {
                    intent.jump = false;
                }
                if let Some(mut wander) = self.ecs.get_mut::<Wander>(*entity) {
                    wander.attack_cooldown = (wander.attack_cooldown - BATCH_SECONDS).max(0.0);
                }
            }
            let actions = self
                .mods
                .mob_brains(&owner, &mobs::encode(&snapshots), snapshots.len());
            self.apply_mob_orders(&actions, &entities, &registry, player_pos);
        }
    }
}
