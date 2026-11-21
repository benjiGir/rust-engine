//! La part serveur de `death.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `death.rs`).

use crate::components::{Health, Mob, Transform};
use crate::player;
use crate::survival::{Hunger, Vitals};
use engine_voxel::bed;
use glam::{IVec3, Vec3};

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// À chaque pas : la vie à 0, c'est la mort.
    pub(crate) fn check_death(&mut self) {
        // Appelé seulement pendant la partie (voir `fixed_update`).
        if self.players.me().dead || player::health(&self.ecs, self.players.entity()).current > 0.0
        {
            return;
        }
        let message = self
            .players
            .me_mut()
            .vitals
            .last_cause
            .map_or("Vous êtes mort.", |cause| cause.death_message());
        // Ce qu'il tenait à la souris et la grille d'artisanat reviennent dans l'inventaire, puis
        // tout tombe là où le joueur est mort. La fenêtre ferme son écran en recevant `Died`.
        self.close_menu();
        let feet = player::body(&self.ecs, self.players.entity())
            .position
            .floor()
            .as_ivec3();
        let stacks = self.players.me_mut().inventory.take_all();
        let dropped: u32 = stacks.iter().map(|s| s.count).sum();
        for stack in stacks {
            self.drop_stack(feet, stack);
        }
        self.players.me_mut().hunger.chewing = None;
        // L'expérience est perdue.
        let (level, _) = self.players.me_mut().experience.level();
        self.players.me_mut().experience = crate::survival::Experience::default();
        log::info!("{message} ({dropped} objets lâchés en {feet}, niveau {level} perdu)");
        self.players.me_mut().dead = true;
        let id = self.players.current_id();
        self.replies.push((
            id,
            crate::protocol::ServerMsg::Died {
                message: message.to_string(),
            },
        ));
    }

    /// Une créature hostile à moins de [`MONSTER_RADIUS`] blocs ?
    fn monster_nearby(&self, center: Vec3) -> bool {
        let registry = self
            .ecs
            .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
            .clone();
        let mut found = false;
        self.ecs
            .query::<(&Transform, &Mob)>()
            .for_each(|_, (transform, mob)| {
                let hostile = registry.get(mob.kind).is_some_and(|d| d.hostile);
                found |= hostile && transform.position.distance(center) < MONSTER_RADIUS;
            });
        found
    }

    /// Un clic droit sur un lit : le point de réapparition, et dormir la nuit.
    /// Renvoie `false` si `pos` n'est pas un lit.
    pub(crate) fn use_bed(&mut self, pos: IVec3) -> bool {
        let head = bed::head_at(self.streamer().world(), pos);
        let Some(head) = head else {
            return false;
        };
        self.players.me_mut().bed = Some(head);
        if !self.day.is_night() {
            self.toast("Point de réapparition fixé");
            return true;
        }
        if self.monster_nearby(head.as_vec3()) {
            self.toast("Impossible de dormir : des créatures rôdent");
            return true;
        }
        self.day.time = MORNING;
        self.toast("Vous avez dormi jusqu'au matin");
        true
    }

    /// Où réapparaître : sur le lit (s'il n'a pas été cassé), sinon au point de départ.
    fn respawn_point(&mut self) -> Vec3 {
        if let Some(bed) = self.players.me_mut().bed {
            let head = bed::head_at(self.streamer().world(), bed);
            if let Some(head) = head {
                return head.as_vec3() + Vec3::new(0.5, 1.0, 0.5);
            }
            self.players.me_mut().bed = None;
            self.toast("Votre lit a disparu : retour au point de départ");
        }
        let spawn = self.spawn;
        Vec3::new(
            spawn.x as f32 + 0.5,
            spawn.y as f32 + 1.4,
            spawn.z as f32 + 0.5,
        )
    }

    /// Réapparaître : au lit s'il existe encore, sinon au départ du monde.
    pub(crate) fn respawn(&mut self) {
        let feet = self.respawn_point();
        player::set_eye(
            &self.ecs,
            self.players.entity(),
            feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
        );
        if let Some(mut health) = self.ecs.get_mut::<Health>(self.players.entity()) {
            health.current = health.max;
        }
        self.players.me_mut().vitals = Vitals::new(&self.survival);
        self.players.me_mut().hunger = Hunger::new(&self.hunger_rules);
        self.players.me_mut().effects.clear();
        self.players.me_mut().dead = false;
        let id = self.players.current_id();
        self.replies
            .push((id, crate::protocol::ServerMsg::Respawned));
        // Le jeu a déplacé le joueur : son prochain mouvement n'est pas à comparer à l'ancien.
        self.forget_last_move();
        log::info!("réapparition en {feet}");
    }
}

/// Une créature hostile plus proche que ça empêche de dormir.
pub(crate) const MONSTER_RADIUS: f32 = 8.0;

/// L'heure du réveil (le soleil vient de se lever).
pub(crate) const MORNING: f32 = 0.27;
