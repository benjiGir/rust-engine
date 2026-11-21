//! Les entités que voit la fenêtre : une **réplique** de ce que le serveur envoie.
//!
//! Pour chaque [`NetId`] reçu, le client garde la sorte et l'apparence de l'entité, ses **deux derniers
//! échantillons de position** et, pour un autre joueur, une animation déduite de ses déplacements. Il
//! ne lit jamais l'ECS du serveur : le rendu (voir `entity_render.rs`), la visée d'une créature ou d'un
//! wagon et les noms au-dessus des joueurs viennent d'ici.
//!
//! **L'interpolation.** Les échantillons portent le pas fixe du serveur auquel ils appartiennent
//! (`ServerMsg::Tick`). Le rendu se place à `pas - interval + alpha` : `alpha` est la part du pas
//! fixe écoulée depuis le dernier, et `interval` le nombre de pas entre deux envois. Avec `interval = 1`
//! (le lien en mémoire, 60 Hz), c'est exactement l'interpolation entre `PreviousPosition` et la
//! position d'un pas, qu'avait le rendu de l'ECS ; avec 3 (20 Hz, le réseau), on glisse d'un échantillon
//! à l'autre en trois pas, avec un retard de `interval` pas.
//!
//! Une entité qui ne bouge plus n'envoie plus rien : à la réception d'une pose, l'échantillon de départ
//! est la pose précédente reculée d'un intervalle, comme si l'entité l'avait gardée jusque-là.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use engine_render::CubeInstance;
use engine_voxel::{ItemId, SubGrid, ray_aabb};
use glam::Vec3;

use crate::entities::{EntityKind, Look, NetId, Pose};
use crate::protocol::ServerMsg;

/// La vitesse de marche de référence d'un joueur (pour l'ampleur de ses pas), en blocs par seconde.
const PLAYER_WALK_SPEED: f32 = 4.3;
/// La phase de marche avance de tant de radians par bloc parcouru (comme les créatures).
const PHASE_PER_BLOCK: f32 = 5.0;
/// Le pas fixe du serveur, en secondes.
const STEP_SECONDS: f32 = 1.0 / 60.0;
/// Les boîtes d'un wagon (voir `carts::hit_cart`).
const CART_HALF_WIDTH: f32 = 0.5;
const CART_HEIGHT: f32 = 0.85;

/// Une pose à un pas du serveur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub tick: u64,
    pub pose: Pose,
}

/// Ce que le client sait d'une entité.
#[derive(Debug)]
pub struct EntityView {
    pub kind: EntityKind,
    pub look: Look,
    from: Sample,
    to: Sample,
    /// La marche d'un autre joueur, déduite de ses déplacements (le serveur n'envoie que sa pose).
    walk_phase: f32,
    walk_stride: f32,
    /// Les blocs d'une structure mobile, posée à sa dernière pose.
    grid: Option<SubGrid>,
}

impl EntityView {
    /// La dernière pose reçue.
    pub fn latest(&self) -> &Pose {
        &self.to.pose
    }

    /// La pose au temps `now` (voir [`ClientEntities::now`]) : la position, le lacet et l'inclinaison
    /// glissent entre les deux échantillons ; l'animation est celle du dernier.
    pub fn pose_at(&self, now: f32) -> Pose {
        let span = (self.to.tick - self.from.tick) as f32;
        let t = if span <= 0.0 {
            1.0
        } else {
            ((now - self.from.tick as f32) / span).clamp(0.0, 1.0)
        };
        let (a, b) = (&self.from.pose, &self.to.pose);
        let mut pose = self.to.pose;
        pose.position = a.position.lerp(b.position, t);
        pose.yaw = lerp_angle(a.yaw, b.yaw, t);
        pose.pitch = a.pitch + (b.pitch - a.pitch) * t;
        if matches!(self.kind, EntityKind::Player { .. }) {
            // Les pas s'éteignent quand plus rien n'arrive.
            let idle = (now - self.to.tick as f32).max(0.0);
            pose.phase = self.walk_phase;
            pose.stride = self.walk_stride * (1.0 - (idle / 8.0).min(1.0));
        }
        pose
    }

    /// Les blocs d'une structure mobile, avec sa dernière pose.
    pub fn grid(&self) -> Option<&SubGrid> {
        self.grid.as_ref()
    }
}

/// Les entités répliquées.
#[derive(Debug)]
pub struct ClientEntities {
    interval: u32,
    tick: u64,
    views: BTreeMap<NetId, EntityView>,
    /// Le temps d'interpolation de l'image en cours (voir [`ClientEntities::begin_frame`]).
    frame_now: f32,
    /// Les modèles des objets au sol, construits une fois par objet (vidés au rechargement des textures).
    item_models: RefCell<HashMap<ItemId, Vec<CubeInstance>>>,
}

impl ClientEntities {
    pub fn new(interval: u32) -> Self {
        Self {
            interval: interval.max(1),
            tick: 0,
            views: BTreeMap::new(),
            frame_now: 0.0,
            item_models: RefCell::default(),
        }
    }

    /// Début d'image : fixe le temps d'interpolation d'après la part `alpha` du pas fixe écoulée.
    pub fn begin_frame(&mut self, alpha: f32) {
        self.frame_now = self.now(alpha);
    }

    /// Le temps d'interpolation de l'image en cours.
    pub fn frame_now(&self) -> f32 {
        self.frame_now
    }

    /// Les modèles d'objets mis en cache.
    pub fn item_models(&self) -> &RefCell<HashMap<ItemId, Vec<CubeInstance>>> {
        &self.item_models
    }

    /// Oublie les modèles d'objets (les textures ont changé).
    pub fn clear_models(&self) {
        self.item_models.borrow_mut().clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn get(&self, id: NetId) -> Option<&EntityView> {
        self.views.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (NetId, &EntityView)> {
        self.views.iter().map(|(id, view)| (*id, view))
    }

    /// Le temps d'interpolation, en pas fixes, pour la part `alpha` du pas en cours.
    pub fn now(&self, alpha: f32) -> f32 {
        self.tick as f32 - self.interval as f32 + alpha
    }

    /// Les structures mobiles, avec leur dernière pose.
    pub fn grids(&self) -> Vec<&SubGrid> {
        self.views.values().filter_map(EntityView::grid).collect()
    }

    /// Range un message d'entité du serveur (les autres messages ne sont pas pour lui).
    pub fn receive(&mut self, msg: &ServerMsg) {
        match msg {
            ServerMsg::Tick(tick) => self.tick = *tick,
            ServerMsg::Spawn {
                id,
                kind,
                pose,
                look,
            } => {
                let sample = Sample {
                    tick: self.tick,
                    pose: *pose,
                };
                self.views.insert(
                    *id,
                    EntityView {
                        kind: kind.clone(),
                        look: *look,
                        from: sample,
                        to: sample,
                        walk_phase: 0.0,
                        walk_stride: 0.0,
                        grid: None,
                    },
                );
            }
            ServerMsg::Despawn(id) => {
                self.views.remove(id);
            }
            ServerMsg::Moves(moves) => {
                for (id, pose) in moves {
                    self.moved(*id, *pose);
                }
            }
            ServerMsg::EntityState { id, look } => {
                if let Some(view) = self.views.get_mut(id) {
                    view.look = *look;
                }
            }
            ServerMsg::GridBlocks { id, grid } => {
                if let Some(view) = self.views.get_mut(id) {
                    let mut blocks = grid.0.as_ref().clone();
                    blocks.position = view.to.pose.position;
                    blocks.yaw = view.to.pose.yaw;
                    view.grid = Some(blocks);
                }
            }
            ServerMsg::Correct { .. }
            | ServerMsg::Push { .. }
            | ServerMsg::Sky(_)
            | ServerMsg::Unload(_)
            | ServerMsg::Vitals(_)
            | ServerMsg::Inventory(_)
            | ServerMsg::Container(_)
            | ServerMsg::ScreenView(_)
            | ServerMsg::Quests(_)
            | ServerMsg::Feedback(_)
            | ServerMsg::Died { .. }
            | ServerMsg::Respawned
            | ServerMsg::OpenScreen(_)
            | ServerMsg::CloseScreen
            | ServerMsg::Chat(_)
            | ServerMsg::ModOptions { .. }
            | ServerMsg::ModHud { .. }
            | ServerMsg::Chunk { .. }
            | ServerMsg::ChunkChanged { .. } => {}
        }
    }

    /// Une nouvelle pose : l'ancienne devient l'échantillon de départ, reculé d'un intervalle si elle
    /// date de plus longtemps (l'entité n'a pas bougé entre-temps).
    fn moved(&mut self, id: NetId, pose: Pose) {
        let Some(view) = self.views.get_mut(&id) else {
            return;
        };
        let tick = self.tick;
        let start = tick.saturating_sub(u64::from(self.interval));
        let previous = view.to;
        if matches!(view.kind, EntityKind::Player { .. }) {
            let travelled = (pose.position - previous.pose.position)
                .with_y(0.0)
                .length();
            let seconds = tick.saturating_sub(previous.tick).max(1) as f32 * STEP_SECONDS;
            let goal = (travelled / seconds / PLAYER_WALK_SPEED).min(1.4);
            view.walk_stride += (goal - view.walk_stride) * 0.5;
            view.walk_phase =
                (view.walk_phase + travelled * PHASE_PER_BLOCK) % std::f32::consts::TAU;
        }
        view.from = if previous.tick >= start {
            previous
        } else {
            Sample {
                tick: start,
                pose: previous.pose,
            }
        };
        view.to = Sample { tick, pose };
        if let Some(grid) = &mut view.grid {
            grid.position = pose.position;
            grid.yaw = pose.yaw;
        }
    }

    /// Ce que le regard touche à moins de `max` blocs : la créature la plus proche, sinon le wagon, comme
    /// avant (une créature masque un wagon derrière elle). Les boîtes sont celles de la dernière pose :
    /// c'est ce que le serveur revérifie.
    pub fn hit(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<NetId> {
        let nearest = |cart: bool| {
            let mut best: Option<(NetId, f32)> = None;
            for (id, view) in self.iter() {
                let (half, height) = match (&view.kind, cart) {
                    (EntityKind::Mob { .. }, false) => (view.look.size[0] / 2.0, view.look.size[1]),
                    (EntityKind::Cart { .. }, true) => (CART_HALF_WIDTH, CART_HEIGHT),
                    _ => continue,
                };
                let at = view.latest().position;
                let half = Vec3::new(half, 0.0, half);
                if let Some(t) = ray_aabb(origin, dir, at - half, at + half + Vec3::Y * height)
                    && t <= max
                    && best.is_none_or(|(_, b)| t < b)
                {
                    best = Some((id, t));
                }
            }
            best.map(|(id, _)| id)
        };
        nearest(false).or_else(|| nearest(true))
    }

    /// Les autres joueurs : leur nom et la position de leurs pieds au temps `now`.
    pub fn players(&self, now: f32) -> Vec<(&str, Vec3)> {
        self.iter()
            .filter_map(|(_, view)| match &view.kind {
                EntityKind::Player { name } => Some((name.as_str(), view.pose_at(now).position)),
                _ => None,
            })
            .collect()
    }
}

/// Interpole un angle par le plus court chemin.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    a + d * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{PreviousPosition, Transform};

    /// Une créature de cette taille (largeur, hauteur) : la taille est dans l'apparence.
    fn mob(width: f32, height: f32) -> (EntityKind, Look) {
        (
            EntityKind::Mob {
                kind: 0,
                title: None,
            },
            Look {
                size: [width, height],
                ..Look::default()
            },
        )
    }

    fn at(x: f32) -> Pose {
        Pose {
            position: Vec3::new(x, 10.0, 0.0),
            ..Pose::default()
        }
    }

    fn spawn(
        entities: &mut ClientEntities,
        id: u32,
        kind: impl Into<Appearance>,
        pose: Pose,
    ) -> NetId {
        let id = NetId(id);
        let Appearance(kind, look) = kind.into();
        entities.receive(&ServerMsg::Spawn {
            id,
            kind,
            pose,
            look,
        });
        id
    }

    /// Une sorte et son apparence de départ.
    struct Appearance(EntityKind, Look);

    impl From<(EntityKind, Look)> for Appearance {
        fn from((kind, look): (EntityKind, Look)) -> Self {
            Self(kind, look)
        }
    }

    impl From<EntityKind> for Appearance {
        fn from(kind: EntityKind) -> Self {
            Self(kind, Look::default())
        }
    }

    #[test]
    fn with_an_interval_of_one_it_is_the_previous_position_interpolation() {
        // Le même mouvement joué par l'ECS (`PreviousPosition` puis `Transform`) et par la réplique.
        let mut entities = ClientEntities::new(1);
        let id = spawn(&mut entities, 1, mob(0.9, 0.9), at(0.0));
        let mut ecs = (
            PreviousPosition(Vec3::new(0.0, 10.0, 0.0)),
            Transform {
                position: Vec3::new(0.0, 10.0, 0.0),
                yaw: 0.0,
            },
        );
        let mut x = 0.0;
        for tick in 1..=30u64 {
            // Un pas : l'ECS garde l'ancienne position, avance ; le serveur envoie la nouvelle.
            x += if tick % 7 == 0 {
                0.0
            } else {
                0.07 * tick as f32
            };
            ecs.0.0 = ecs.1.position;
            ecs.1.position = Vec3::new(x, 10.0, 0.0);
            entities.receive(&ServerMsg::Tick(tick));
            if ecs.0.0 != ecs.1.position {
                entities.receive(&ServerMsg::Moves(vec![(id, at(x))]));
            }
            for alpha in [0.0, 0.25, 0.5, 0.99] {
                let expected = ecs.0.0.lerp(ecs.1.position, alpha);
                let got = entities
                    .get(id)
                    .unwrap()
                    .pose_at(entities.now(alpha))
                    .position;
                assert!(
                    got.distance(expected) < 1e-4,
                    "pas {tick}, alpha {alpha} : {got} au lieu de {expected}"
                );
            }
        }
    }

    #[test]
    fn with_an_interval_of_three_it_is_halfway_at_the_middle() {
        let mut entities = ClientEntities::new(3);
        let id = spawn(&mut entities, 1, mob(0.9, 0.9), at(0.0));
        // Deux envois, aux pas 3 et 6 : 0, puis 3 (3 blocs en 3 pas).
        entities.receive(&ServerMsg::Tick(3));
        entities.receive(&ServerMsg::Moves(vec![(id, at(0.5))]));
        entities.receive(&ServerMsg::Tick(6));
        entities.receive(&ServerMsg::Moves(vec![(id, at(3.5))]));
        let x = |tick: u64, alpha: f32| {
            let mut copy = entities.now(alpha);
            copy += tick as f32 - 6.0;
            entities.get(id).unwrap().pose_at(copy).position.x
        };
        // Au pas 6 (alpha 0) le rendu est à la pose du pas 3 ; une fois et demie plus tard, au milieu.
        assert!((x(6, 0.0) - 0.5).abs() < 1e-4, "{}", x(6, 0.0));
        assert!((x(7, 0.5) - 2.0).abs() < 1e-4, "{}", x(7, 0.5));
        assert!((x(9, 0.0) - 3.5).abs() < 1e-4);
        // Au-delà, il attend le prochain envoi sans dépasser.
        assert!((x(12, 0.0) - 3.5).abs() < 1e-4);
    }

    #[test]
    fn an_entity_that_was_still_starts_moving_from_where_it_stood() {
        let mut entities = ClientEntities::new(1);
        let id = spawn(&mut entities, 1, mob(0.9, 0.9), at(5.0));
        // 100 pas immobile, puis un pas à 6 : le rendu glisse de 5 à 6 sur ce pas, pas depuis 100 pas.
        entities.receive(&ServerMsg::Tick(100));
        entities.receive(&ServerMsg::Moves(vec![(id, at(6.0))]));
        let view = entities.get(id).unwrap();
        for (alpha, expected) in [(0.0, 5.0), (0.5, 5.5), (1.0, 6.0)] {
            let got = view.pose_at(entities.now(alpha)).position.x;
            assert!((got - expected).abs() < 1e-4, "alpha {alpha} : {got}");
        }
    }

    #[test]
    fn despawn_and_state_messages_update_the_view() {
        let mut entities = ClientEntities::new(1);
        let id = spawn(&mut entities, 4, mob(0.6, 1.0), at(0.0));
        entities.receive(&ServerMsg::EntityState {
            id,
            look: Look {
                hurt: true,
                coat: Some(5),
                ..Look::default()
            },
        });
        assert!(entities.get(id).unwrap().look.hurt);
        assert_eq!(entities.get(id).unwrap().look.coat, Some(5));
        entities.receive(&ServerMsg::Moves(vec![(NetId(99), at(1.0))]));
        assert_eq!(entities.len(), 1, "un numéro inconnu est ignoré");
        entities.receive(&ServerMsg::Despawn(id));
        assert_eq!(entities.len(), 0);
    }

    #[test]
    fn the_ray_hits_the_nearest_creature_before_a_cart_behind_it() {
        let mut entities = ClientEntities::new(1);
        let far = spawn(&mut entities, 1, mob(0.9, 0.9), at(8.0));
        let near = spawn(&mut entities, 2, mob(0.9, 0.9), at(4.0));
        let cart = spawn(
            &mut entities,
            3,
            EntityKind::Cart {
                kind: crate::carts::CartKind::Minecart,
            },
            at(2.0),
        );
        let origin = Vec3::new(0.0, 10.4, 0.0);
        // Une créature devant, un wagon plus près : la créature gagne tant qu'elle est à portée.
        assert_eq!(entities.hit(origin, Vec3::X, 10.0), Some(near));
        // Hors de portée des créatures (3 blocs), le wagon.
        assert_eq!(entities.hit(origin, Vec3::X, 3.0), Some(cart));
        assert_eq!(entities.hit(origin, Vec3::X, 0.5), None);
        // Un mur plus près que les créatures les cache, mais pas la plus lointaine si elle seule est visée.
        assert_ne!(entities.hit(origin, Vec3::X, 10.0), Some(far));
        assert_eq!(entities.hit(origin, Vec3::Y, 10.0), None);
    }

    #[test]
    fn another_player_walks_in_place_only_while_moving() {
        let mut entities = ClientEntities::new(1);
        let id = spawn(
            &mut entities,
            1,
            EntityKind::Player {
                name: "Alice".into(),
            },
            at(0.0),
        );
        for tick in 1..=30u64 {
            entities.receive(&ServerMsg::Tick(tick));
            entities.receive(&ServerMsg::Moves(vec![(id, at(tick as f32 * 0.07))]));
        }
        let moving = entities.get(id).unwrap().pose_at(entities.now(0.5));
        assert!(moving.stride > 0.5 && moving.phase > 0.0, "{moving:?}");
        // Plus d'envoi : le pas s'éteint.
        entities.receive(&ServerMsg::Tick(60));
        let still = entities.get(id).unwrap().pose_at(entities.now(0.5));
        assert_eq!(still.stride, 0.0);
        assert_eq!(entities.players(entities.now(0.0))[0].0, "Alice");
    }

    #[test]
    fn a_ship_keeps_its_blocks_and_follows_its_pose() {
        use crate::entities::GridSnapshot;
        let mut entities = ClientEntities::new(1);
        let grid = engine_voxel::GridId(3);
        let id = spawn(&mut entities, 9, EntityKind::Grid { id: grid }, at(10.0));
        assert!(
            entities.grids().is_empty(),
            "pas de blocs, pas de structure"
        );
        let snapshot = SubGrid::new(grid, std::sync::Arc::new(crate::test_registry()));
        entities.receive(&ServerMsg::GridBlocks {
            id,
            grid: GridSnapshot(std::sync::Arc::new(snapshot)),
        });
        assert_eq!(entities.grids()[0].position.x, 10.0);
        entities.receive(&ServerMsg::Tick(1));
        entities.receive(&ServerMsg::Moves(vec![(
            id,
            Pose {
                yaw: 1.5,
                ..at(12.0)
            },
        )]));
        let ship = entities.grids()[0];
        assert_eq!((ship.position.x, ship.yaw), (12.0, 1.5));
        entities.receive(&ServerMsg::Despawn(id));
        assert!(entities.grids().is_empty());
    }

    #[test]
    fn angles_take_the_short_way() {
        let t = lerp_angle(3.0, -3.0, 0.5);
        assert!(t.abs() > 3.0, "passe par pi : {t}");
    }
}
