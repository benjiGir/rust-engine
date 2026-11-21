//! Le pas fixe du serveur (60 par seconde) : les systèmes de l'ECS, puis les machines et les mods,
//! les fluides, la survie, les quêtes, l'aventure, et enfin ce que chaque joueur voit changer parmi
//! les entités. La fenêtre n'y prend aucune part : elle donne l'intention de son joueur avant, et lit
//! les réponses après.

use std::time::Instant;

use engine_ecs::{Events, Stage};
use engine_mods::ModEvent;
use engine_voxel::GridPos;
use glam::{IVec3, Vec3};

use crate::events::{BlockBroken, BlockPlaced, EntityDied, PlayerDamaged};
use crate::server_side::VoxelServer;
use crate::{audio, player};

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
        self.update_portals(dt);
        // Les joueurs prédits que le serveur a déplacés reçoivent leur place (voir `predicted.rs`).
        self.correct_predicted();
        self.send_entities();
    }
}

impl VoxelServer {
    /// Le travail du serveur à chaque image, entre les pas fixes : l'heure, la météo, les habitants des villages, les systèmes de l'image, puis les événements
    /// devenus sons et messages aux mods, et le ciel envoyé aux joueurs.
    pub(crate) fn frame(&mut self, dt: f32) {
        self.day.update(dt);
        let player_position = player::body(&self.ecs, self.players.entity()).position;
        // Le temps qu'il fait ; ce qui se voit et s'entend (gouttes, éclairs, tonnerre) est chez la
        // fenêtre (voir `sky_view.rs`).
        self.weather.update_sky(dt);
        // Les habitants des villages proches.
        {
            let shared = self.world.clone();
            let streamer = shared.borrow();
            self.villages.update(
                dt,
                &mut self.ecs,
                streamer.generator(),
                player_position,
                &self.trades,
            );
        }
        // L'heure, pour l'apparition des créatures (l'ECS ne connaît pas le jour).
        *self.ecs.resource_mut::<crate::spawning::Sky>() = crate::spawning::Sky {
            daylight: self.day.daylight(),
            night: self.day.is_night(),
        };
        self.update_effects(dt);
        self.update_dimension_sky();
        self.update_helm();
        self.schedule.run(Stage::Update, &mut self.ecs);
        self.play_event_sounds();
        self.forward_events_to_mods();
        // Fin de l'image : les événements lus tournent leurs tampons.
        crate::events::update(&self.ecs);
        self.send_sky(dt);
    }

    /// Les potions en cours : la régénération soigne, la résistance éteint le feu.
    fn update_effects(&mut self, dt: f32) {
        let healed = self.players.me_mut().effects.update(dt);
        if healed > 0.0 {
            self.heal(healed);
        }
        if self
            .players
            .me_mut()
            .effects
            .has(crate::potions::Effect::FireResistance)
        {
            self.players.me_mut().vitals.burning = 0.0;
        }
    }

    /// Le monde se charge autour de chaque joueur.
    pub(crate) fn stream_world(&mut self) {
        let views = self.stream_views();
        self.streamer_mut().update_views(&views);
    }

    fn play_event_sounds(&mut self) {
        // Les événements du monde deviennent des sons pour les joueurs qui les entendent.
        use crate::player_view::Sound;
        let mut sounds = Vec::new();
        let centre = |pos: IVec3| pos.as_vec3() + Vec3::splat(0.5);
        for event in self
            .readers
            .broken
            .read(&self.ecs.resource::<Events<BlockBroken>>())
        {
            let kind = audio::BlockSound::Break;
            sounds.push((
                Sound::Block {
                    block: event.block.0,
                    kind,
                },
                centre(event.pos),
            ));
        }
        for event in self
            .readers
            .placed
            .read(&self.ecs.resource::<Events<BlockPlaced>>())
        {
            let kind = audio::BlockSound::Place;
            sounds.push((
                Sound::Block {
                    block: event.block.0,
                    kind,
                },
                centre(event.pos),
            ));
        }
        for event in self
            .readers
            .died
            .read(&self.ecs.resource::<Events<EntityDied>>())
        {
            log::info!("{} est mort", event.kind);
            sounds.push((Sound::key("core:break_soft"), event.position));
        }
        let mut cries = Vec::new();
        for event in self
            .readers
            .damaged
            .read(&self.ecs.resource::<Events<PlayerDamaged>>())
        {
            log::info!("aïe : -{} ({} restants)", event.amount, event.health_left);
            cries.push(event.player.unwrap_or(self.players.local_id()));
        }
        for (sound, at) in sounds {
            self.sound_near(sound, at);
        }
        // Le cri de douleur : seulement pour le joueur touché.
        for player in cries {
            let feedback = crate::player_view::Feedback::Sound(Sound::key("core:place_hard"));
            self.replies
                .push((player, crate::protocol::ServerMsg::Feedback(feedback)));
        }
    }

    /// Transmet les événements du jeu aux scripts des mods.
    fn forward_events_to_mods(&mut self) {
        let _span = engine_trace::span("mods : événements");
        let mut events = Vec::new();
        for e in self
            .mod_readers
            .broken
            .read(&self.ecs.resource::<Events<BlockBroken>>())
        {
            events.push(ModEvent::BlockBroken {
                at: GridPos::world(e.pos),
                block: e.block,
            });
        }
        // La direction du regard du joueur qui a posé, ramenée à l'axe horizontal le plus proche :
        // pour orienter les machines posées (un casseur regarde dans la direction du joueur).
        let facing_of = |look: Vec3| {
            if look.x.abs() > look.z.abs() {
                IVec3::new(look.x.signum() as i32, 0, 0)
            } else {
                IVec3::new(0, 0, look.z.signum() as i32)
            }
        };
        for e in self
            .mod_readers
            .placed
            .read(&self.ecs.resource::<Events<BlockPlaced>>())
        {
            let look = e
                .player
                .and_then(|p| self.players.get(p))
                .unwrap_or_else(|| self.players.local())
                .look;
            events.push(ModEvent::BlockPlaced {
                at: GridPos::world(e.pos),
                block: e.block,
                facing: facing_of(look),
            });
        }
        for e in self
            .mod_readers
            .died
            .read(&self.ecs.resource::<Events<EntityDied>>())
        {
            events.push(ModEvent::EntityDied {
                kind: e.kind.clone(),
                position: e.position,
            });
        }
        for e in self
            .mod_readers
            .damaged
            .read(&self.ecs.resource::<Events<PlayerDamaged>>())
        {
            events.push(ModEvent::PlayerDamaged {
                amount: e.amount,
                health_left: e.health_left,
            });
        }
        for event in events {
            let actions = self.mods.dispatch(&event);
            self.apply_machine_actions(actions);
        }
    }
}
