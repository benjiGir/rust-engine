//! Le pas fixe du serveur (60 par seconde) : les systèmes de l'ECS, puis les machines et les mods,
//! les fluides, la survie, les quêtes, l'aventure, et enfin ce que chaque joueur voit changer parmi
//! les entités. La fenêtre n'y prend aucune part : elle donne l'intention de son joueur avant, et lit
//! les réponses après.

use std::time::Instant;

use engine_ecs::Stage;

use crate::player;
use crate::server_side::VoxelServer;

impl VoxelServer {
    /// Un pas fixe du monde.
    pub(crate) fn fixed_step(&mut self, dt: f32) {
        self.schedule.run(Stage::FixedUpdate, &mut self.ecs);
        // Les machines : un tick tous les 3 pas (20 par seconde).
        let eye = player::eye(&self.ecs, self.players.entity(), 1.0);
        self.refresh_gear(dt);
        self.pick_up_drops(dt);
        self.update_backpacks(dt);
        self.update_quests(dt);
        let (actions, machine_tick) = self.mods.fixed_step(eye);
        self.apply_machine_actions(actions);
        let snapshot = self.player_snapshot();
        self.mods.set_player(snapshot);
        self.run_mob_brains(machine_tick.is_some());
        let world_ready = self.world.borrow().is_idle() && self.sends.idle();
        if let Some(stress) = &mut self.stress {
            let eye = player::eye(&self.ecs, self.players.entity(), 1.0);
            stress.build_if_ready(world_ready, &mut self.ecs, &mut self.mods, eye);
            let start = Instant::now();
            crate::stress::conveyors(&mut self.ecs);
            stress.record(machine_tick.map(|t| t.duration), start.elapsed());
        }
        self.step_fluids();
        self.update_cart_blocks();
        self.update_vitals(dt);
        self.update_hunger(dt);
        // Le minage et le repas de chaque joueur (voir `intents.rs`).
        self.tick_digs(dt);
        self.tick_eating(dt);
        self.drop_mob_loot();
        self.update_adventure(dt);
        self.update_charms(dt);
        self.check_death();
        // Fin du pas : chaque joueur reçoit ce qui a changé parmi les entités.
        self.send_entities();
    }
}
