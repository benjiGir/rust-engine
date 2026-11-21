//! Les entités vues par un client : ce que le serveur en dit, et le suivi qui le dit.
//!
//! Le serveur (l'ECS) possède les créatures, objets au sol, wagons, projectiles, autres joueurs et
//! structures mobiles. Le client ne les lit jamais : il en tient une **réplique** (voir
//! `client_entities.rs`) faite de ce que le rendu demande, que le serveur lui envoie par messages
//! ([`ServerMsg`]) :
//!
//! - `Spawn` : une entité entre dans la portée du joueur, avec sa sorte ([`EntityKind`]), sa pose
//!   ([`Pose`]) et son apparence ([`Look`]) ;
//! - `Moves` : les poses de ce qui a bougé, toutes les `interval` pas fixes ;
//! - `EntityState` : un changement d'apparence (une créature blessée, un petit qui grandit) ;
//! - `Despawn` : elle sort de la portée, ou elle meurt ;
//! - `GridBlocks` : les blocs d'une structure mobile, quand ils changent.
//!
//! Chaque entité répliquée porte un [`NetId`], numéro du serveur jamais réutilisé dans une partie : le
//! client désigne ce qu'il voit par ce numéro (un coup, un wagon ouvert) et le serveur retrouve l'entité
//! et revalide ([`NetIds::entity`]).
//!
//! Un [`EntityTracker`] par joueur garde ce qu'il a reçu. Une entité est vue si elle est dans une
//! colonne que le joueur a reçue (voir `engine_voxel::ChunkSender::has_column`) et à moins de N blocs de
//! lui, N dépendant de la sorte.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use engine_ecs::{Entity, World};
use engine_voxel::{CHUNK_SIZE, GridId, SubGrid, SubGrids};
use glam::{IVec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::carts::{Cart, CartKind};
use crate::components::{Baby, Collider, Hurt, Mob, MobAnim, Player, Transform};
use crate::drops::DroppedItem;
use crate::inventory::ItemStack;
use crate::players::PlayerId;
use crate::projectiles::Projectile;
use crate::protocol::ServerMsg;

/// Jusqu'où le joueur voit chaque sorte d'entité, en blocs.
pub const MOB_RANGE: f32 = 80.0;
pub const ITEM_RANGE: f32 = 48.0;
pub const PROJECTILE_RANGE: f32 = 64.0;
pub const CART_RANGE: f32 = 128.0;
pub const GRID_RANGE: f32 = 128.0;
pub const PLAYER_RANGE: f32 = 128.0;
/// Une entité déjà vue n'est reprise qu'au-delà de sa portée plus cette marge : une créature à la
/// limite n'apparaît pas et ne disparaît pas à chaque pas.
pub const LEAVE_MARGIN: f32 = 4.0;

/// Le numéro d'une entité répliquée, donné par le serveur et jamais réutilisé dans une partie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NetId(pub u32);

/// Ce qu'est une entité : ce qui ne change pas pendant sa vie.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntityKind {
    /// Une créature : son numéro dans le registre, et le nom d'une élite ou d'un envahisseur.
    Mob {
        kind: u32,
        title: Option<MobTitle>,
    },
    /// Un objet au sol.
    Item {
        stack: ItemStack,
    },
    Cart {
        kind: CartKind,
    },
    Projectile {
        color: Vec3,
    },
    /// Une structure mobile (bateau, contraption) : ses blocs viennent dans `GridBlocks`.
    Grid {
        id: GridId,
    },
    /// Un autre joueur.
    Player {
        name: String,
    },
}

/// Où est l'entité et comment elle bouge, à un pas du serveur. Une créature envoie sa phase de marche,
/// l'ampleur de ses pas et le tour de sa tête ; un wagon, son lacet et son inclinaison ; une
/// structure, son lacet.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Pose {
    /// Les pieds (le pivot d'une structure).
    pub position: Vec3,
    pub yaw: f32,
    /// L'inclinaison d'un wagon.
    pub pitch: f32,
    pub phase: f32,
    pub stride: f32,
    /// Le tour de la tête d'une créature.
    pub look: f32,
}

/// Ce qui d'une élite se voit : sa teinte, et le faisceau d'un envahisseur.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EliteLook {
    pub tint: Vec3,
    pub invader: bool,
}

/// Le nom composé d'une élite ou d'un envahisseur, pour la fiche de la cible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobTitle {
    pub name: String,
    pub augmentations: u8,
}

/// L'apparence qui peut changer pendant la vie de l'entité (`EntityState`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Look {
    /// Elle rougit un instant après un coup (créature ou wagon).
    pub hurt: bool,
    /// Un petit : plus petit.
    pub baby: bool,
    /// La couleur de toison d'un mouton (son numéro dans la palette).
    pub coat: Option<u8>,
    pub elite: Option<EliteLook>,
    /// La boîte de la créature (largeur, hauteur) : le client s'en sert pour la viser. Elle change quand
    /// un petit grandit.
    pub size: [f32; 2],
    /// La vie (actuelle, maximale) d'une créature, au demi-point près, pour la fiche de la cible.
    pub health: [f32; 2],
}

/// Les blocs d'une structure mobile, tels que le serveur les a au moment de l'envoi. Les chunks sont
/// des `Arc` : en mémoire la copie ne coûte presque rien (le transport réseau, au lot 5, enverra les
/// blocs avec le format des sauvegardes). Deux instantanés sont égaux s'ils sont le même.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridSnapshot(#[serde(with = "crate::wire::grid")] pub Arc<SubGrid>);

impl PartialEq for GridSnapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// La table des numéros du serveur : de quoi retrouver l'entité qu'un client désigne.
#[derive(Debug, Default)]
pub struct NetIds {
    next: u32,
    entities: HashMap<NetId, Entity>,
    grids: HashMap<GridId, NetId>,
    /// Le dernier instantané de chaque structure (refait quand ses blocs changent).
    snapshots: HashMap<GridId, (u64, GridSnapshot)>,
}

impl NetIds {
    fn fresh(&mut self) -> NetId {
        self.next += 1;
        NetId(self.next)
    }

    /// L'entité vivante que désigne ce numéro ; `None` pour un numéro inconnu, ou d'une entité morte.
    pub fn entity(&self, world: &World, id: NetId) -> Option<Entity> {
        let entity = *self.entities.get(&id)?;
        world.is_alive(entity).then_some(entity)
    }

    /// Le numéro de la structure, donné à sa première vue.
    fn grid(&mut self, id: GridId) -> NetId {
        if let Some(net) = self.grids.get(&id) {
            return *net;
        }
        let net = self.fresh();
        self.grids.insert(id, net);
        net
    }

    /// Donne un numéro à toute entité répliquée qui n'en a pas, et oublie celles qui ne sont plus.
    pub fn assign(world: &mut World) {
        if world.get_resource::<NetIds>().is_none() {
            world.insert_resource(NetIds::default());
        }
        let mut missing = Vec::new();
        fn gather<T: 'static>(world: &World, out: &mut Vec<Entity>) {
            world.query::<(&T,)>().for_each(|entity, _| {
                if !world.has::<NetId>(entity) {
                    out.push(entity);
                }
            });
        }
        gather::<Mob>(world, &mut missing);
        gather::<DroppedItem>(world, &mut missing);
        gather::<Cart>(world, &mut missing);
        gather::<Projectile>(world, &mut missing);
        gather::<PlayerId>(world, &mut missing);
        if !missing.is_empty() {
            let mut ids = world.resource_mut::<NetIds>();
            let numbered: Vec<(Entity, NetId)> =
                missing.into_iter().map(|e| (e, ids.fresh())).collect();
            for &(entity, id) in &numbered {
                ids.entities.insert(id, entity);
            }
            drop(ids);
            for (entity, id) in numbered {
                world.insert(entity, id);
            }
        }
        // Les morts sortent de la table (un numéro n'est jamais redonné : seule la place est rendue).
        let dead: Vec<NetId> = world
            .resource::<NetIds>()
            .entities
            .iter()
            .filter(|(_, e)| !world.is_alive(**e))
            .map(|(id, _)| *id)
            .collect();
        if !dead.is_empty() {
            let mut ids = world.resource_mut::<NetIds>();
            for id in dead {
                ids.entities.remove(&id);
            }
        }
    }
}

/// Une entité telle que le serveur la décrit à ce pas, avant de savoir qui la voit.
#[derive(Debug, Clone)]
pub struct Replicated {
    pub id: NetId,
    pub entity: Option<Entity>,
    pub kind: EntityKind,
    pub pose: Pose,
    pub look: Look,
    pub range: f32,
    /// Pour une structure : sa révision et ses blocs.
    pub blocks: Option<(u64, GridSnapshot)>,
}

/// Décrit toutes les entités répliquées (`NetIds::assign` doit avoir tourné). `name` donne le nom d'un
/// joueur d'après son entité.
pub fn collect(
    world: &World,
    subs: &SubGrids,
    name: &dyn Fn(Entity) -> Option<String>,
) -> Vec<Replicated> {
    let mut out = Vec::new();
    world
        .query::<(&Transform, &Mob, &NetId)>()
        .for_each(|entity, (transform, mob, id)| {
            let size = world
                .get::<Collider>(entity)
                .map_or([0.6, 1.0], |c| [c.width, c.height]);
            let anim = world
                .get::<MobAnim>(entity)
                .map_or_else(MobAnim::default, |a| *a);
            out.push(Replicated {
                id: *id,
                entity: Some(entity),
                kind: EntityKind::Mob {
                    kind: mob.kind,
                    title: world.get::<crate::elites::Elite>(entity).map(|e| MobTitle {
                        name: e.name.clone(),
                        augmentations: e.augmentations.len().min(255) as u8,
                    }),
                },
                pose: Pose {
                    position: transform.position,
                    yaw: transform.yaw,
                    phase: anim.phase,
                    stride: anim.stride,
                    look: anim.look,
                    ..Pose::default()
                },
                look: Look {
                    hurt: world.has::<Hurt>(entity),
                    baby: world.has::<Baby>(entity),
                    coat: world
                        .get::<crate::sheep_colors::SheepCoat>(entity)
                        .map(|c| c.0),
                    elite: world
                        .get::<crate::elites::Elite>(entity)
                        .map(|e| EliteLook {
                            tint: e.tint,
                            invader: e.invader,
                        }),
                    size,
                    health: world
                        .get::<crate::components::Health>(entity)
                        .map_or([0.0, 0.0], |h| {
                            [(h.current.max(0.0) * 2.0).round() / 2.0, h.max]
                        }),
                },
                range: MOB_RANGE,
                blocks: None,
            });
        });
    world
        .query::<(&Transform, &DroppedItem, &NetId)>()
        .for_each(|entity, (transform, drop, id)| {
            out.push(Replicated {
                id: *id,
                entity: Some(entity),
                kind: EntityKind::Item { stack: drop.stack },
                pose: Pose {
                    position: transform.position,
                    yaw: transform.yaw,
                    ..Pose::default()
                },
                look: Look::default(),
                range: ITEM_RANGE,
                blocks: None,
            });
        });
    world
        .query::<(&Transform, &Cart, &NetId)>()
        .for_each(|entity, (transform, cart, id)| {
            out.push(Replicated {
                id: *id,
                entity: Some(entity),
                kind: EntityKind::Cart { kind: cart.kind },
                pose: Pose {
                    position: transform.position,
                    yaw: cart.yaw,
                    pitch: cart.pitch,
                    ..Pose::default()
                },
                look: Look {
                    hurt: cart.hurt > 0.0,
                    ..Look::default()
                },
                range: CART_RANGE,
                blocks: None,
            });
        });
    world.query::<(&Transform, &Projectile, &NetId)>().for_each(
        |entity, (transform, projectile, id)| {
            out.push(Replicated {
                id: *id,
                entity: Some(entity),
                kind: EntityKind::Projectile {
                    color: projectile.color,
                },
                pose: Pose {
                    position: transform.position,
                    ..Pose::default()
                },
                look: Look::default(),
                range: PROJECTILE_RANGE,
                blocks: None,
            });
        },
    );
    world
        .query::<(&Transform, &PlayerId, &Player, &NetId)>()
        .for_each(|entity, (transform, _, _, id)| {
            out.push(Replicated {
                id: *id,
                entity: Some(entity),
                kind: EntityKind::Player {
                    name: name(entity).unwrap_or_default(),
                },
                pose: Pose {
                    position: transform.position,
                    yaw: transform.yaw,
                    ..Pose::default()
                },
                look: Look::default(),
                range: PLAYER_RANGE,
                blocks: None,
            });
        });
    let mut ids = world.resource_mut::<NetIds>();
    for grid_id in subs.ids() {
        let net = ids.grid(grid_id);
        let grid = subs.get(grid_id).expect("identifiant connu").borrow();
        let revision = grid.revision();
        let fresh = ids
            .snapshots
            .get(&grid_id)
            .is_none_or(|(known, _)| *known != revision);
        if fresh {
            ids.snapshots
                .insert(grid_id, (revision, GridSnapshot(Arc::new(grid.clone()))));
        }
        let snapshot = ids.snapshots[&grid_id].clone();
        out.push(Replicated {
            id: net,
            entity: None,
            kind: EntityKind::Grid { id: grid_id },
            pose: Pose {
                position: grid.position,
                yaw: grid.yaw,
                ..Pose::default()
            },
            look: Look::default(),
            range: GRID_RANGE,
            blocks: Some(snapshot),
        });
    }
    let live: Vec<GridId> = subs.ids();
    ids.snapshots.retain(|id, _| live.contains(id));
    out.sort_by_key(|r| r.id);
    out
}

/// Qui regarde : le joueur, où il est, et les colonnes que son client a reçues.
pub struct Viewer<'a> {
    /// Son entité : il ne se voit pas lui-même.
    pub entity: Entity,
    pub position: Vec3,
    /// Le client a-t-il la colonne `(x, z)` du monde (en chunks) ?
    pub knows: &'a dyn Fn(IVec2) -> bool,
}

/// Ce qu'un joueur a reçu des entités.
#[derive(Debug)]
pub struct EntityTracker {
    interval: u32,
    seen: BTreeMap<NetId, Seen>,
}

#[derive(Debug)]
struct Seen {
    pose: Pose,
    look: Look,
    /// La révision des blocs d'une structure, la dernière envoyée.
    revision: Option<u64>,
}

impl EntityTracker {
    /// `interval` : un `Moves` tous les tant de pas fixes. 1 sur le lien en mémoire (60 Hz, comme
    /// l'ECS) ; 3 (20 Hz) sera celui du réseau.
    pub fn new(interval: u32) -> Self {
        Self {
            interval: interval.max(1),
            seen: BTreeMap::new(),
        }
    }

    /// Le nombre d'entités que ce joueur voit.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Ce joueur voit cette entité.
    #[cfg(test)]
    pub fn has(&self, id: NetId) -> bool {
        self.seen.contains_key(&id)
    }

    /// Un pas fixe (`tick`) : les messages à envoyer à ce joueur d'après les entités décrites par le
    /// serveur (`all`). Ils sont ajoutés à `out`, `Tick` d'abord.
    pub fn step(
        &mut self,
        tick: u64,
        viewer: &Viewer,
        all: &[Replicated],
        out: &mut Vec<ServerMsg>,
    ) {
        out.push(ServerMsg::Tick(tick));
        let send_moves = tick.is_multiple_of(u64::from(self.interval));
        let mut visible = Vec::with_capacity(all.len());
        let mut moves = Vec::new();
        for r in all {
            if r.entity == Some(viewer.entity) {
                continue;
            }
            let known = self.seen.contains_key(&r.id);
            let limit = if known {
                r.range + LEAVE_MARGIN
            } else {
                r.range
            };
            if r.pose.position.distance(viewer.position) > limit {
                continue;
            }
            // Une structure est vue de loin sans qu'on ait sa colonne : ses blocs sont à elle.
            if r.blocks.is_none() && !(viewer.knows)(column_of(r.pose.position)) {
                continue;
            }
            visible.push(r.id);
            match self.seen.get_mut(&r.id) {
                None => {
                    out.push(ServerMsg::Spawn {
                        id: r.id,
                        kind: r.kind.clone(),
                        pose: r.pose,
                        look: r.look,
                    });
                    let revision = r.blocks.as_ref().map(|(revision, blocks)| {
                        out.push(ServerMsg::GridBlocks {
                            id: r.id,
                            grid: blocks.clone(),
                        });
                        *revision
                    });
                    self.seen.insert(
                        r.id,
                        Seen {
                            pose: r.pose,
                            look: r.look,
                            revision,
                        },
                    );
                }
                Some(seen) => {
                    if seen.look != r.look {
                        seen.look = r.look;
                        out.push(ServerMsg::EntityState {
                            id: r.id,
                            look: r.look,
                        });
                    }
                    if let Some((revision, blocks)) = &r.blocks
                        && seen.revision != Some(*revision)
                    {
                        seen.revision = Some(*revision);
                        out.push(ServerMsg::GridBlocks {
                            id: r.id,
                            grid: blocks.clone(),
                        });
                    }
                    if send_moves && seen.pose != r.pose {
                        seen.pose = r.pose;
                        moves.push((r.id, r.pose));
                    }
                }
            }
        }
        // Ce qui n'est plus dans la portée, ou n'existe plus.
        let gone: Vec<NetId> = self
            .seen
            .keys()
            .filter(|id| visible.binary_search(id).is_err())
            .copied()
            .collect();
        for id in gone {
            self.seen.remove(&id);
            out.push(ServerMsg::Despawn(id));
        }
        if !moves.is_empty() {
            out.push(ServerMsg::Moves(moves));
        }
    }
}

/// La colonne de chunks qui contient ce point.
fn column_of(position: Vec3) -> IVec2 {
    let size = CHUNK_SIZE as f32;
    IVec2::new(
        (position.x / size).floor() as i32,
        (position.z / size).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Flying, MoveIntent, PreviousPosition, Velocity};
    use crate::mobs::{self, MobRegistry};

    fn everywhere(_: IVec2) -> bool {
        true
    }

    fn viewer_at(entity: Entity, position: Vec3, knows: &dyn Fn(IVec2) -> bool) -> Viewer<'_> {
        Viewer {
            entity,
            position,
            knows,
        }
    }

    /// Un monde de test avec un joueur en `player_at` ; renvoie aussi la table des structures (vide).
    fn scene(player_at: Vec3) -> (World, Entity, SubGrids) {
        let mut world = mobs::tests::sim_at(player_at, 5);
        let player = world.resource::<mobs::PlayerEntities>().0[0];
        world.get_mut::<Transform>(player).unwrap().position = player_at;
        world.insert(player, PlayerId(0));
        let subs = SubGrids::new(std::sync::Arc::new(crate::test_registry()));
        NetIds::assign(&mut world);
        (world, player, subs)
    }

    fn mob_kind(world: &World, key: &str) -> u32 {
        world.resource::<Arc<MobRegistry>>().id(key).unwrap()
    }

    fn describe(world: &World, subs: &SubGrids) -> Vec<Replicated> {
        collect(world, subs, &|_| Some("vue".to_string()))
    }

    fn step(
        tracker: &mut EntityTracker,
        tick: u64,
        world: &mut World,
        subs: &SubGrids,
        viewer: &Viewer,
    ) -> Vec<ServerMsg> {
        NetIds::assign(world);
        let mut out = Vec::new();
        tracker.step(tick, viewer, &describe(world, subs), &mut out);
        out
    }

    fn spawns(out: &[ServerMsg]) -> usize {
        out.iter()
            .filter(|m| matches!(m, ServerMsg::Spawn { .. }))
            .count()
    }

    fn despawns(out: &[ServerMsg]) -> usize {
        out.iter()
            .filter(|m| matches!(m, ServerMsg::Despawn(_)))
            .count()
    }

    fn moved(out: &[ServerMsg]) -> usize {
        out.iter()
            .map(|m| match m {
                ServerMsg::Moves(moves) => moves.len(),
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn net_ids_are_unique_and_never_reused() {
        let (mut world, _, _) = scene(Vec3::new(0.5, 150.0, 0.5));
        let pig = mob_kind(&world, "core:pig");
        let first = mobs::spawn_mob(&mut world, pig, Vec3::new(3.5, 150.0, 0.5)).unwrap();
        let second = mobs::spawn_mob(&mut world, pig, Vec3::new(5.5, 150.0, 0.5)).unwrap();
        NetIds::assign(&mut world);
        let (a, b) = (
            *world.get::<NetId>(first).unwrap(),
            *world.get::<NetId>(second).unwrap(),
        );
        assert_ne!(a, b);
        // Un numéro retrouve son entité ; un numéro inconnu ou d'une entité morte, rien.
        let ids = world.resource::<NetIds>();
        assert_eq!(ids.entity(&world, a), Some(first));
        assert_eq!(ids.entity(&world, NetId(9_999)), None);
        drop(ids);
        world.despawn(first);
        assert_eq!(world.resource::<NetIds>().entity(&world, a), None);
        // Le suivant n'a pas le numéro du mort.
        let third = mobs::spawn_mob(&mut world, pig, Vec3::new(7.5, 150.0, 0.5)).unwrap();
        NetIds::assign(&mut world);
        let c = *world.get::<NetId>(third).unwrap();
        assert!(c != a && c != b, "{c:?}");
    }

    #[test]
    fn an_entity_spawns_in_range_and_despawns_out_of_it_or_when_it_dies() {
        let eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, player, subs) = scene(eye);
        let pig = mob_kind(&world, "core:pig");
        let near = mobs::spawn_mob(&mut world, pig, Vec3::new(10.5, 150.0, 0.5)).unwrap();
        let far = mobs::spawn_mob(&mut world, pig, Vec3::new(120.5, 150.0, 0.5)).unwrap();
        let mut tracker = EntityTracker::new(1);
        let viewer = viewer_at(player, eye, &everywhere);
        let out = step(&mut tracker, 1, &mut world, &subs, &viewer);
        assert_eq!(spawns(&out), 1, "seule la proche : {out:?}");
        assert_eq!(tracker.len(), 1);
        // Ni elle ni le joueur ne sont renvoyés ensuite.
        let out = step(&mut tracker, 2, &mut world, &subs, &viewer);
        assert_eq!(spawns(&out) + despawns(&out), 0);
        // La lointaine s'approche : elle apparaît à la portée (80).
        world.get_mut::<Transform>(far).unwrap().position.x = eye.x + 79.0;
        let out = step(&mut tracker, 3, &mut world, &subs, &viewer);
        assert_eq!(spawns(&out), 1);
        // Elle s'éloigne un peu au-delà de 80 : la marge la garde ; bien au-delà, elle sort.
        world.get_mut::<Transform>(far).unwrap().position.x = eye.x + 82.0;
        let out = step(&mut tracker, 4, &mut world, &subs, &viewer);
        assert_eq!(despawns(&out), 0, "la marge de sortie la garde");
        world.get_mut::<Transform>(far).unwrap().position.x = eye.x + 90.0;
        let out = step(&mut tracker, 5, &mut world, &subs, &viewer);
        assert_eq!(despawns(&out), 1);
        // La proche meurt.
        world.despawn(near);
        let out = step(&mut tracker, 6, &mut world, &subs, &viewer);
        assert_eq!(despawns(&out), 1);
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn a_hurt_mob_sends_its_health_and_an_elite_its_title() {
        let eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, player, subs) = scene(eye);
        let pig = mob_kind(&world, "core:pig");
        let mob = mobs::spawn_mob(&mut world, pig, Vec3::new(4.5, 150.0, 0.5)).unwrap();
        world.insert(
            mob,
            crate::elites::Elite {
                name: "Cochon colosse".into(),
                augmentations: vec!["a".into(), "b".into()],
                tint: Vec3::ONE,
                damage: 0.0,
                speed: 1.0,
                armor: 0.0,
                regen: 0.0,
                invader: false,
            },
        );
        let mut tracker = EntityTracker::new(1);
        let viewer = viewer_at(player, eye, &everywhere);
        let out = step(&mut tracker, 1, &mut world, &subs, &viewer);
        let spawned = out.iter().find_map(|m| match m {
            ServerMsg::Spawn {
                kind: EntityKind::Mob { title, .. },
                look,
                ..
            } => Some((title.clone(), look.health)),
            _ => None,
        });
        let (title, health) = spawned.expect("la créature apparaît");
        assert_eq!(
            title,
            Some(MobTitle {
                name: "Cochon colosse".into(),
                augmentations: 2
            })
        );
        assert!(health[0] > 0.0 && health[0] == health[1], "{health:?}");
        // Un coup : la vie arrive par `EntityState`, au demi-point près.
        world
            .get_mut::<crate::components::Health>(mob)
            .unwrap()
            .current = 3.3;
        let out = step(&mut tracker, 2, &mut world, &subs, &viewer);
        assert!(
            out.iter().any(|m| matches!(
                m,
                ServerMsg::EntityState { look, .. } if look.health[0] == 3.5
            )),
            "{out:?}"
        );
    }

    #[test]
    fn only_entities_in_a_known_column_are_seen() {
        let eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, player, subs) = scene(eye);
        let pig = mob_kind(&world, "core:pig");
        mobs::spawn_mob(&mut world, pig, Vec3::new(5.5, 150.0, 0.5)).unwrap();
        mobs::spawn_mob(&mut world, pig, Vec3::new(40.5, 150.0, 0.5)).unwrap();
        let mut tracker = EntityTracker::new(1);
        // Le client n'a que la colonne (0, 0).
        let only_home = |c: IVec2| c == IVec2::ZERO;
        let out = step(
            &mut tracker,
            1,
            &mut world,
            &subs,
            &viewer_at(player, eye, &only_home),
        );
        assert_eq!(spawns(&out), 1);
    }

    #[test]
    fn moves_are_sent_only_for_what_moved_and_at_the_cadence() {
        let eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, player, subs) = scene(eye);
        let pig = mob_kind(&world, "core:pig");
        let walker = mobs::spawn_mob(&mut world, pig, Vec3::new(5.5, 150.0, 0.5)).unwrap();
        mobs::spawn_mob(&mut world, pig, Vec3::new(7.5, 150.0, 2.5)).unwrap();
        let viewer = viewer_at(player, eye, &everywhere);
        let walk =
            |world: &mut World| world.get_mut::<Transform>(walker).unwrap().position.x += 0.1;

        // À 1 : chaque pas, et seulement ce qui a bougé.
        let mut fast = EntityTracker::new(1);
        step(&mut fast, 1, &mut world, &subs, &viewer);
        let out = step(&mut fast, 2, &mut world, &subs, &viewer);
        assert_eq!(moved(&out), 0, "rien n'a bougé");
        walk(&mut world);
        let out = step(&mut fast, 3, &mut world, &subs, &viewer);
        assert_eq!(moved(&out), 1);

        // À 3 : une fois tous les trois pas, avec ce qui a bougé entre-temps.
        let mut slow = EntityTracker::new(3);
        step(&mut slow, 3, &mut world, &subs, &viewer);
        let mut sent = Vec::new();
        for tick in 4..=9 {
            walk(&mut world);
            sent.push((
                tick,
                moved(&step(&mut slow, tick, &mut world, &subs, &viewer)),
            ));
        }
        assert_eq!(
            sent,
            [(4, 0), (5, 0), (6, 1), (7, 0), (8, 0), (9, 1)],
            "un envoi tous les 3 pas"
        );
    }

    #[test]
    fn a_change_of_look_is_a_state_message_and_two_players_track_separately() {
        let a_eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, a, subs) = scene(a_eye);
        let b_eye = Vec3::new(300.5, 150.0, 0.5);
        let b = world.spawn((
            Transform {
                position: b_eye,
                yaw: 0.0,
            },
            PreviousPosition(b_eye),
            Velocity::default(),
            Collider::new(0.6, 1.8),
            MoveIntent::default(),
            Player,
            PlayerId(1),
        ));
        let pig = mob_kind(&world, "core:pig");
        let near_a = mobs::spawn_mob(&mut world, pig, Vec3::new(4.5, 150.0, 0.5)).unwrap();
        let near_b = mobs::spawn_mob(&mut world, pig, Vec3::new(304.5, 150.0, 0.5)).unwrap();
        let mut for_a = EntityTracker::new(1);
        let mut for_b = EntityTracker::new(1);
        let (va, vb) = (
            viewer_at(a, a_eye, &everywhere),
            viewer_at(b, b_eye, &everywhere),
        );
        step(&mut for_a, 1, &mut world, &subs, &va);
        step(&mut for_b, 1, &mut world, &subs, &vb);
        let id_a = *world.get::<NetId>(near_a).unwrap();
        let id_b = *world.get::<NetId>(near_b).unwrap();
        assert!(for_a.has(id_a) && !for_a.has(id_b));
        assert!(for_b.has(id_b) && !for_b.has(id_a));
        assert!(!for_a.has(*world.get::<NetId>(b).unwrap()), "trop loin");

        world.insert(near_a, Hurt { timer: 0.4 });
        let out = step(&mut for_a, 2, &mut world, &subs, &va);
        assert!(
            out.iter().any(|m| matches!(
                m,
                ServerMsg::EntityState { id, look } if *id == id_a && look.hurt
            )),
            "{out:?}"
        );
        let out = step(&mut for_b, 2, &mut world, &subs, &vb);
        assert!(
            !out.iter()
                .any(|m| matches!(m, ServerMsg::EntityState { .. })),
            "l'autre joueur ne la voit pas"
        );
        // Un autre joueur proche est vu, avec son nom ; le sien n'est pas renvoyé.
        let c = world.spawn((
            Transform {
                position: a_eye + Vec3::X * 3.0,
                yaw: 1.0,
            },
            Player,
            PlayerId(2),
            Flying,
        ));
        let out = step(&mut for_a, 3, &mut world, &subs, &va);
        let c_id = *world.get::<NetId>(c).unwrap();
        assert!(
            out.iter().any(|m| matches!(
                m,
                ServerMsg::Spawn { id, kind: EntityKind::Player { name }, .. }
                    if *id == c_id && name == "vue"
            )),
            "{out:?}"
        );
    }

    #[test]
    fn a_ship_sends_its_blocks_when_they_change_and_its_pose_when_it_moves() {
        use engine_voxel::BlockAccess;
        let eye = Vec3::new(0.5, 150.0, 0.5);
        let (mut world, player, mut subs) = scene(eye);
        let stone = crate::test_registry().id("core:stone").unwrap();
        let id = subs.create();
        {
            let mut grid = subs.get(id).unwrap().borrow_mut();
            grid.set_block(glam::IVec3::ZERO, stone);
            grid.position = Vec3::new(20.0, 150.0, 0.0);
        }
        let mut tracker = EntityTracker::new(1);
        // Même sans colonne reçue : ses blocs sont à elle.
        let nowhere = |_: IVec2| false;
        let viewer = viewer_at(player, eye, &nowhere);
        let out = step(&mut tracker, 1, &mut world, &subs, &viewer);
        let blocks = |out: &[ServerMsg]| {
            out.iter()
                .filter(|m| matches!(m, ServerMsg::GridBlocks { .. }))
                .count()
        };
        assert_eq!((spawns(&out), blocks(&out)), (1, 1));
        // Rien ne change : rien n'est renvoyé.
        let out = step(&mut tracker, 2, &mut world, &subs, &viewer);
        assert_eq!((spawns(&out), blocks(&out), moved(&out)), (0, 0, 0));
        // Elle avance : une pose, pas de blocs.
        subs.get(id).unwrap().borrow_mut().position.x += 0.5;
        let out = step(&mut tracker, 3, &mut world, &subs, &viewer);
        assert_eq!((blocks(&out), moved(&out)), (0, 1));
        // On casse un bloc : ses blocs repartent (la révision change).
        subs.get(id)
            .unwrap()
            .borrow_mut()
            .set_block(glam::IVec3::X, stone);
        let out = step(&mut tracker, 4, &mut world, &subs, &viewer);
        assert_eq!(blocks(&out), 1);
        // Posée dans le monde, elle disparaît.
        let mut subs = subs;
        subs.remove(id);
        let out = step(&mut tracker, 5, &mut world, &subs, &viewer);
        assert_eq!(despawns(&out), 1);
    }
}
