//! La part fenêtre de `predicted.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `predicted.rs`).

#[allow(unused_imports)]
use crate::predicted::*;
use engine_voxel::PLAYER_EYE_HEIGHT;
use glam::Vec3;

impl crate::VoxelGame {
    /// La fenêtre place son joueur elle-même (vol automatique, démo, `ctl`) : son corps prédit, et le
    /// serveur, d'accord sur cette place (le prochain `Move` ne sera pas pris pour périmé).
    pub(crate) fn place_eye(&mut self, eye: Vec3) {
        let feet = eye - Vec3::Y * PLAYER_EYE_HEIGHT;
        self.client.body.place(feet);
        let entity = self.server.players.entity();
        crate::player::set_eye(&self.server.ecs, entity, eye);
        self.server.players.local_mut().synced = Some(feet);
    }

    /// Marche ou vol, ici et au serveur.
    pub(crate) fn toggle_fly(&mut self) {
        self.client.body.toggle_mode();
        let entity = self.server.players.entity();
        crate::player::toggle_mode(&mut self.server.ecs, entity);
    }

    /// Un pas de la physique du joueur, chez la fenêtre, puis son envoi au serveur.
    pub(crate) fn predict_move(&mut self, walk: Vec3, jump: bool, dt: f32) {
        // La gravité de la dimension (transitoire : lue au serveur, elle viendra d'une vue).
        let gravity = engine_voxel::GRAVITY
            * self
                .server
                .ecs
                .get_resource::<crate::components::GravityScale>()
                .map_or(1.0, |g| g.0);
        self.client
            .body
            .step(self.client.view.replica(), walk, jump, gravity, dt);
        self.send_local_move(walk, jump);
    }
}
