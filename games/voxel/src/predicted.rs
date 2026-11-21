//! Le corps du joueur de la fenêtre, **prédit** : la fenêtre fait tourner elle-même la physique de son
//! joueur ([`PlayerBody`], la même que le serveur) sur sa réplique du monde, et envoie le résultat à
//! chaque pas (`ClientMsg::Move`). Le serveur ne simule plus ce joueur (le système `physics` saute
//! les entités marquées [`crate::components::Predicted`]) : il range la position reçue, la contrôle si
//! `VOXEL_CHECK_MOVES=1`, et répond par un `ServerMsg::Correct` quand il déplace le joueur lui-même
//! (réapparition, téléportation, mouvement refusé) ou quand le joueur est **porté** (wagon, structure
//! mobile) : là c'est le serveur qui simule, et la fenêtre suit. Un coup reçu arrive en
//! `ServerMsg::Push` : le recul s'applique au corps prédit.
//!
//! La caméra se place d'après ce corps, interpolé entre ses deux derniers pas : le joueur se voit
//! bouger sans attendre le serveur, ce qui comptera quand celui-ci sera à l'autre bout d'un réseau.

use engine_voxel::{ChunkStreamer, PLAYER_EYE_HEIGHT, PlayerBody};
use glam::Vec3;

use crate::player::MoveMode;

/// Le corps prédit du joueur de la fenêtre.
#[derive(Debug, Clone, Copy)]
pub struct PredictedBody {
    /// La physique : position des pieds, vitesse, sol, eau.
    pub body: PlayerBody,
    /// Les pieds au pas précédent, pour interpoler la caméra.
    pub previous: Vec3,
    pub mode: MoveMode,
    /// Le terrain autour n'est pas encore arrivé : le corps attend sans bouger.
    pub frozen: bool,
    /// Un wagon ou une structure porte le joueur : le serveur le simule, la fenêtre suit.
    pub carried: bool,
}

impl PredictedBody {
    pub fn new(feet: Vec3, mode: MoveMode) -> Self {
        Self {
            body: PlayerBody::new(feet),
            previous: feet,
            mode,
            frozen: false,
            carried: false,
        }
    }

    pub fn feet(&self) -> Vec3 {
        self.body.position
    }

    /// Les yeux à afficher : interpolés en marche (`alpha` entre le pas précédent et le dernier),
    /// tels quels en vol (le vol avance à chaque image).
    pub fn eye(&self, alpha: f32) -> Vec3 {
        let feet = if self.mode == MoveMode::Walk || self.carried {
            self.previous.lerp(self.body.position, alpha)
        } else {
            self.body.position
        };
        feet + Vec3::Y * PLAYER_EYE_HEIGHT
    }

    /// Un pas de physique en marche, sur le monde que voit la fenêtre.
    pub fn step(&mut self, world: &ChunkStreamer, walk: Vec3, jump: bool, gravity: f32, dt: f32) {
        self.previous = self.body.position;
        if self.mode == MoveMode::Fly || self.carried {
            return;
        }
        self.body.gravity = gravity;
        self.frozen = !crate::mobs::surroundings_ready(world, &self.body);
        if self.frozen {
            self.body.velocity = Vec3::ZERO;
            return;
        }
        self.body.step(world.world(), walk, jump, dt);
    }

    /// Vol libre : déplacement direct, sans collision, à chaque image.
    pub fn fly(&mut self, direction: Vec3, speed: f32, dt: f32) {
        self.body.position += direction.normalize_or_zero() * speed * dt;
    }

    /// Place les pieds, sans interpoler depuis l'ancienne position.
    pub fn place(&mut self, feet: Vec3) {
        self.body.position = feet;
        self.previous = feet;
    }

    /// Marche ou vol : on repart d'une vitesse nulle.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            MoveMode::Walk => MoveMode::Fly,
            MoveMode::Fly => MoveMode::Walk,
        };
        self.previous = self.body.position;
        self.body.velocity = Vec3::ZERO;
    }

    /// Le serveur place le joueur. Porté, le corps glisse vers la nouvelle position (la caméra
    /// interpole) et garde la vitesse du serveur ; sinon il y saute, arrêté.
    pub fn correct(&mut self, position: Vec3, velocity: Vec3, carried: bool) {
        if carried {
            self.previous = self.body.position;
            self.body.position = position;
            self.body.velocity = velocity;
        } else {
            self.place(position);
            self.body.velocity = Vec3::ZERO;
            self.body.impulse = Vec3::ZERO;
        }
        self.carried = carried;
    }

    /// Un coup reçu : un petit bond et une poussée qui s'amortit (voir `PlayerBody::impulse`).
    pub fn push(&mut self, up: f32, impulse: Vec3) {
        self.body.velocity.y = up;
        self.body.impulse = impulse;
    }
}

impl crate::server_side::VoxelServer {
    /// Le joueur est porté : un wagon ou une structure mobile le déplace, le serveur le simule.
    pub(crate) fn carried(&self, entity: engine_ecs::Entity) -> bool {
        self.ecs.has::<crate::components::OnGrid>(entity)
            || self.ecs.has::<crate::carts::Riding>(entity)
    }

    /// Range le mouvement que la fenêtre du joueur a joué. Si le serveur l'a déplacé depuis le dernier
    /// accord (réapparition, téléportation), ce mouvement est périmé : le joueur reçoit sa vraie
    /// place. Porté, seul le serveur le déplace.
    pub(crate) fn store_move(&mut self, player: crate::players::PlayerId, moved: StoredMove) {
        let Some(state) = self.players.get(player) else {
            return;
        };
        let entity = state.entity;
        let synced = state.synced;
        if !self.ecs.has::<crate::components::Predicted>(entity) || self.carried(entity) {
            return;
        }
        let here = crate::player::body(&self.ecs, entity).position;
        if synced.is_some_and(|feet| feet.distance_squared(here) > 1e-8) {
            self.send_correct(player, false);
            return;
        }
        if let Some(mut previous) = self
            .ecs
            .get_mut::<crate::components::PreviousPosition>(entity)
        {
            previous.0 = here;
        }
        if let Some(mut transform) = self.ecs.get_mut::<crate::components::Transform>(entity) {
            transform.position = moved.position;
        }
        if let Some(mut velocity) = self.ecs.get_mut::<crate::components::Velocity>(entity) {
            velocity.0 = moved.velocity;
        }
        if let Some(mut collider) = self.ecs.get_mut::<crate::components::Collider>(entity) {
            collider.on_ground = moved.on_ground;
            collider.in_liquid = moved.in_liquid;
            collider.frozen = moved.frozen;
        }
        match moved.mode {
            MoveMode::Fly => {
                self.ecs.insert(entity, crate::components::Flying);
            }
            MoveMode::Walk => {
                self.ecs.remove::<crate::components::Flying>(entity);
            }
        }
        if let Some(state) = self.players.get_mut(player) {
            state.synced = Some(moved.position);
        }
    }

    /// Dit au joueur où il est vraiment, et en fait le nouvel accord.
    pub(crate) fn send_correct(&mut self, player: crate::players::PlayerId, carried: bool) {
        let Some(state) = self.players.get(player) else {
            return;
        };
        let entity = state.entity;
        let position = crate::player::body(&self.ecs, entity).position;
        let velocity = self
            .ecs
            .get::<crate::components::Velocity>(entity)
            .map_or(Vec3::ZERO, |v| v.0);
        if let Some(state) = self.players.get_mut(player) {
            state.synced = Some(position);
            state.carried = carried;
        }
        self.replies.push((
            player,
            crate::protocol::ServerMsg::Correct {
                position,
                velocity,
                carried,
            },
        ));
    }

    /// Fin de pas : chaque joueur prédit que le serveur a déplacé (ou qui est porté) reçoit sa place.
    pub(crate) fn correct_predicted(&mut self) {
        let players: Vec<_> = self
            .players
            .iter()
            .filter(|p| self.ecs.has::<crate::components::Predicted>(p.entity))
            .map(|p| (p.id, p.entity, p.synced, p.carried))
            .collect();
        for (id, entity, synced, was_carried) in players {
            let carried = self.carried(entity);
            let here = crate::player::body(&self.ecs, entity).position;
            let moved = synced.is_some_and(|feet| feet.distance_squared(here) > 1e-8);
            if moved || carried != was_carried {
                self.send_correct(id, carried);
            }
        }
    }

    /// Un coup : le joueur recule. Prédit (et pas porté), c'est sa fenêtre qui le fait reculer.
    pub(crate) fn push_player(&mut self, up: f32, impulse: Vec3) {
        let entity = self.players.entity();
        if self.ecs.has::<crate::components::Predicted>(entity) && !self.carried(entity) {
            let id = self.players.current_id();
            self.replies
                .push((id, crate::protocol::ServerMsg::Push { up, impulse }));
            return;
        }
        if let Some(mut velocity) = self.ecs.get_mut::<crate::components::Velocity>(entity) {
            velocity.0.y = up;
        }
        self.ecs.insert(entity, crate::components::Impulse(impulse));
    }
}

/// Un mouvement reçu, tel que le serveur le range.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StoredMove {
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub in_liquid: bool,
    pub frozen: bool,
    pub mode: MoveMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_snaps_unless_the_player_is_carried() {
        let mut body = PredictedBody::new(Vec3::ZERO, MoveMode::Walk);
        body.body.velocity = Vec3::X;
        body.correct(Vec3::new(5.0, 0.0, 0.0), Vec3::ONE, false);
        assert_eq!(
            (body.previous, body.feet()),
            (Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0))
        );
        assert_eq!(body.body.velocity, Vec3::ZERO);
        body.correct(Vec3::new(6.0, 0.0, 0.0), Vec3::ONE, true);
        assert_eq!(body.previous, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(body.eye(0.5).x, 5.5);
        assert_eq!(body.body.velocity, Vec3::ONE);
    }

    #[test]
    fn flying_moves_straight_and_toggling_stops_the_body() {
        let mut body = PredictedBody::new(Vec3::new(0.0, 80.0, 0.0), MoveMode::Fly);
        body.fly(Vec3::X * 2.0, 12.0, 0.5);
        assert_eq!(body.feet(), Vec3::new(6.0, 80.0, 0.0));
        assert_eq!(body.eye(0.0), body.feet() + Vec3::Y * PLAYER_EYE_HEIGHT);
        body.body.velocity = Vec3::ONE;
        body.toggle_mode();
        assert_eq!(
            (body.mode, body.body.velocity),
            (MoveMode::Walk, Vec3::ZERO)
        );
        assert_eq!(body.previous, body.feet());
    }

    #[test]
    fn a_push_sets_the_jump_and_the_impulse() {
        let mut body = PredictedBody::new(Vec3::ZERO, MoveMode::Walk);
        body.push(5.0, Vec3::X * 4.0);
        assert_eq!(
            (body.body.velocity.y, body.body.impulse),
            (5.0, Vec3::X * 4.0)
        );
    }
}
