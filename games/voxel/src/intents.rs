//! Le côté qui agit : `VoxelGame::handle(joueur, message)` applique une intention du client.
//!
//! Toute action sur le monde passe par ici : le minage, les coups, le clic droit, les clics de
//! cases, la console, la réapparition (voir `protocol.rs` pour les messages). La fenêtre ne fait
//! que lire les entrées et fabriquer des messages (`play_input.rs`) ; en solo ils sont appliqués
//! dans la même image, sans délai.
//!
//! - **La visée est refaite ici.** Un message d'action porte l'œil et le regard du client
//!   ([`Aim`]) ; le simulateur lance son propre rayon dans son monde (bloc et créature visés) et ne
//!   croit jamais la cible que le client dit viser. Pendant le traitement, cette visée est
//!   [`ActingAim`] : les fonctions d'action la lisent par [`VoxelGame::aim`] au lieu de la caméra
//!   et du bloc visé de la fenêtre (qui restent la visée quand rien n'est en cours).
//! - **Le joueur courant** est celui du message : `players.me()` désigne ce joueur tant que `handle`
//!   tourne (voir `players.rs`), jamais le joueur de la fenêtre en dur.
//! - **Le minage** est prédit côté client (la fissure et le son avancent sans attendre) ; le
//!   simulateur compte son propre temps en pas fixes et n'accepte `Dig Finish` que si au moins
//!   70 % du temps de minage théorique s'est écoulé.

use super::*;
use crate::entities::{NetId, NetIds};
use crate::gear::Strike;
use crate::movement::{MoveLimits, MoveRefusal, MoveSample, movement_check};
use crate::players::{DigState, OpenContainer, PlayerId};
use crate::protocol::{Aim, ClickButton, ClientMsg, DigPhase, ServerMsg, SlotTarget};
use crate::ui::{Screen, SlotRef};

/// De combien l'œil annoncé par le client peut s'écarter de l'œil du joueur côté simulateur.
const EYE_TOLERANCE: f32 = 1.5;
/// La part du temps de minage théorique qui doit s'être écoulée avant `Dig Finish`.
const DIG_MIN_PROGRESS: f32 = 0.7;
/// Ce que `Dig Finish` tolère en plus de la portée : le bloc se mesure depuis son centre.
const DIG_REACH_MARGIN: f32 = 1.0;

/// La visée d'une action : l'œil, le regard, et ce que le rayon touche dans le monde du joueur.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActingAim {
    pub player: PlayerId,
    pub eye: Vec3,
    pub forward: Vec3,
    /// Le bloc touché dans la portée du joueur.
    pub hit: Option<RayHit>,
    /// La créature touchée, si elle est plus près que le bloc.
    pub mob: Option<Entity>,
}

/// Ce que le dernier message a produit : pour `ctl`, qui répond avec (voir `control.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum Outcome {
    #[default]
    Nothing,
    /// Le clic droit a servi (porte, conteneur, lit, machine...).
    Used,
    /// Le clic droit a posé un bloc.
    Placed(IVec3),
    /// Un coup a touché une créature.
    Struck(Strike),
    /// Un coup de poing a touché un wagon.
    Punched,
}

/// La réponse du dernier message `Command` : le texte de la console, et le JSON de `ctl`.
pub(crate) struct CommandReply {
    pub text: String,
    pub json: Option<Result<String>>,
}

/// L'œil annoncé par le client est-il celui du joueur (à la tolérance près) ?
pub(crate) fn eye_is_plausible(claimed: Vec3, server: Vec3) -> bool {
    claimed.is_finite() && claimed.distance(server) <= EYE_TOLERANCE
}

/// Le lancer de rayon du simulateur : le bloc touché dans `reach`, et la créature touchée si elle
/// est plus près que ce bloc.
pub(crate) fn cast_aim(
    world: &engine_voxel::World,
    ecs: &engine_ecs::World,
    player: PlayerId,
    aim: Aim,
    reach: f32,
) -> Option<ActingAim> {
    let forward = aim.forward.normalize_or_zero();
    if forward == Vec3::ZERO || !aim.eye.is_finite() {
        return None;
    }
    let hit = raycast(world, aim.eye, forward, reach);
    let block_distance = hit.map_or(reach, |h| h.distance);
    let mob = mobs::hit_mob(ecs, aim.eye, forward, block_distance).map(|(mob, _)| mob);
    Some(ActingAim {
        player,
        eye: aim.eye,
        forward,
        hit,
        mob,
    })
}

/// Le début d'un minage vu du simulateur : `time` est le temps de minage théorique (0 :
/// instantané), `first` la durée de la première image, déjà comptée chez le client.
pub(crate) fn start_dig(pos: IVec3, block: BlockId, time: f32, first: f32) -> DigState {
    DigState {
        pos,
        block,
        progress: if time <= 0.0 {
            1.0
        } else {
            (first / time).min(1.0)
        },
    }
}

/// Un pas fixe de minage : avec l'outil tenu à ce moment (`time`, `None` si le bloc est
/// devenu incassable).
pub(crate) fn advance_dig(dig: &mut DigState, time: Option<f32>, dt: f32) {
    dig.progress = match time {
        Some(time) if time > 0.0 => (dig.progress + dt / time).min(1.0),
        _ => 1.0,
    };
}

/// Pourquoi `Dig Finish` est refusé, s'il l'est : ce n'est pas le bloc commencé, le bloc a changé,
/// il est hors de portée, ou le temps de minage n'a pas pu s'écouler.
pub(crate) fn dig_refusal(
    dig: &DigState,
    pos: IVec3,
    block_now: BlockId,
    eye: Vec3,
    reach: f32,
) -> Option<&'static str> {
    let center = pos.as_vec3() + Vec3::splat(0.5);
    if dig.pos != pos {
        Some("ce n'est pas le bloc commencé")
    } else if block_now != dig.block {
        Some("le bloc a changé entre-temps")
    } else if eye.distance(center) > reach + DIG_REACH_MARGIN {
        Some("hors de portée")
    } else if dig.progress < DIG_MIN_PROGRESS {
        Some("trop tôt")
    } else {
        None
    }
}

/// La case que désigne un message, si ce joueur peut la toucher : une case de son inventaire, ou
/// d'un conteneur qu'il a lui-même ouvert.
pub(crate) fn slot_ref_for(
    state: &crate::players::PlayerState,
    slot: SlotTarget,
    entity_of: &dyn Fn(NetId) -> Option<Entity>,
) -> Option<SlotRef> {
    let open = state.open;
    match slot {
        SlotTarget::Player(slot) => {
            let in_range = match slot {
                inventory::Slot::Main(i) => i < state.inventory.slots.len(),
                inventory::Slot::Armor(i) => i < state.inventory.armor.len(),
                inventory::Slot::Craft(i) => i < state.inventory.craft.len(),
            };
            in_range.then_some(SlotRef::Player(slot))
        }
        SlotTarget::Block { pos, index } => matches!(
            open,
            Some(OpenContainer::Chest(p) | OpenContainer::Machine(p) | OpenContainer::Controller(p))
                if p == pos
        )
        .then_some(SlotRef::Container(pos, index)),
        SlotTarget::Cart { cart, index } => {
            // Un numéro inconnu (ou d'une entité morte) ne désigne rien.
            let cart = entity_of(cart)?;
            (open == Some(OpenContainer::Cart(cart))).then_some(SlotRef::Cart(cart, index))
        }
        SlotTarget::Bag { number, index } => {
            (open == Some(OpenContainer::Bag(number))).then_some(SlotRef::Bag(number, index))
        }
    }
}

impl VoxelGame {
    /// La visée de l'action en cours ; hors d'un message, celle de la fenêtre (le bloc visé de
    /// l'image et la caméra).
    pub(crate) fn aim(&self) -> ActingAim {
        if let Some(aim) = self.acting {
            return aim;
        }
        let eye = self.camera.position;
        let forward = self.camera.forward();
        ActingAim {
            player: self.players.current_id(),
            eye,
            forward,
            hit: self.target,
            // La fenêtre choisit ce qu'elle frappe dans sa réplique (voir `window_entity`) ; le simulateur
            // refait sa propre visée à chaque message.
            mob: None,
        }
    }

    /// L'entité que retrouve le serveur sous ce numéro, si elle existe encore.
    pub(crate) fn server_entity(&self, id: NetId) -> Option<Entity> {
        self.ecs.get_resource::<NetIds>()?.entity(&self.ecs, id)
    }

    /// La visée de la fenêtre, telle qu'elle part dans un message.
    pub(crate) fn window_aim(&self) -> Aim {
        Aim {
            eye: self.camera.position,
            forward: self.camera.forward(),
        }
    }

    /// La visée d'une commande `ctl` : l'œil vient du corps (la caméra peut retarder d'une image
    /// après un `goto`), le regard de la caméra.
    pub(crate) fn body_aim(&self) -> Aim {
        Aim {
            eye: player::eye(&self.ecs, self.players.entity(), 1.0),
            forward: self.camera.forward(),
        }
    }

    /// Met un message dans la file du client : il sera appliqué à la fin de la chaîne d'entrées de
    /// l'image (voir `play_input.rs`).
    pub(crate) fn send(&mut self, msg: ClientMsg) {
        self.outbox.push(msg);
    }

    /// Envoie et applique tout de suite (les écrans et `ctl`, qui ne sont pas dans la chaîne
    /// d'entrées).
    pub(crate) fn send_now(&mut self, msg: ClientMsg) {
        self.outbox.push(msg);
        self.flush_outbox();
    }

    /// Applique les messages du joueur local dans l'ordre, puis les réponses du simulateur.
    pub(crate) fn flush_outbox(&mut self) {
        let local = self.players.local_id();
        let mut guard = 0;
        // Un message peut en provoquer d'autres (fermer un écran...) : on vide jusqu'au bout.
        while !self.outbox.is_empty() && guard < 64 {
            guard += 1;
            for msg in std::mem::take(&mut self.outbox) {
                self.handle(local, msg);
            }
        }
        self.apply_replies();
    }

    /// Les réponses du simulateur au joueur de la fenêtre. Celles des autres joueurs partiront sur
    /// le réseau (lot 5).
    pub(crate) fn apply_replies(&mut self) {
        let local = self.players.local_id();
        for (player, reply) in std::mem::take(&mut self.replies) {
            if player != local {
                continue;
            }
            match reply {
                ServerMsg::Correct { position } => {
                    let entity = self.players.entity();
                    if let Some(mut transform) = self.ecs.get_mut::<Transform>(entity) {
                        transform.position = position;
                    }
                    if let Some(mut previous) =
                        self.ecs.get_mut::<components::PreviousPosition>(entity)
                    {
                        previous.0 = position;
                    }
                    if let Some(mut velocity) = self.ecs.get_mut::<Velocity>(entity) {
                        velocity.0 = Vec3::ZERO;
                    }
                    self.camera.position = position + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT;
                }
                ServerMsg::Feedback(feedback) => self.apply_feedback(feedback),
                ServerMsg::Died { message } => {
                    // L'écran ouvert se ferme, la souris est rendue au menu de mort.
                    if self.ui.inventory_open {
                        self.close_screen();
                        self.ui.inventory_open = false;
                    }
                    self.mode = Mode::Dead(message);
                }
                ServerMsg::Respawned => self.mode = Mode::Playing,
                // Le monde que voit la fenêtre : la réplique l'applique à la prochaine mise à jour
                // de ses vues (voir `client_world.rs`).
                chunk_msg => self.view.receive(chunk_msg),
            }
        }
        self.view.apply_received();
    }

    /// Applique une intention du client. Tout ce que le simulateur touche pendant ce temps est
    /// l'état de `player`.
    pub(crate) fn handle(&mut self, player: PlayerId, msg: ClientMsg) {
        if self.players.get(player).is_none() {
            log::warn!("message d'un joueur inconnu ({player:?}) : ignoré");
            return;
        }
        let previous_player = self.players.set_acting(Some(player));
        let previous_aim = self.acting.take();
        self.apply(player, msg);
        self.acting = previous_aim;
        self.players.set_acting(previous_player);
    }

    fn apply(&mut self, player: PlayerId, msg: ClientMsg) {
        match msg {
            ClientMsg::Move {
                position,
                on_ground,
                mode,
                step,
                ..
            } => self.handle_move(
                player,
                MoveSample {
                    position,
                    mode,
                    on_ground,
                    step,
                },
            ),
            ClientMsg::Dig {
                aim, phase, pos, ..
            } => self.handle_dig(player, aim, phase, pos),
            ClientMsg::Attack { aim, entity } => {
                // Un numéro que le serveur ne connaît pas (ou d'une entité morte) est refusé.
                match self.server_entity(entity) {
                    Some(claimed) => {
                        self.with_aim(player, aim, |game| game.handle_attack(claimed));
                    }
                    None => log::debug!("coup refusé : {entity:?} est inconnu"),
                }
            }
            ClientMsg::Use { aim, sneaking } => {
                self.with_aim(player, aim, |game| game.handle_use(sneaking));
            }
            ClientMsg::Eat { held } => self.players.me_mut().eating = held,
            ClientMsg::SelectSlot(slot) => {
                let inventory = &mut self.players.me_mut().inventory;
                inventory.selected = usize::from(slot).min(inventory.slots.len() - 1);
            }
            ClientMsg::Drop { aim, all } => {
                self.with_aim(player, aim, |game| game.throw_selected(all));
            }
            ClientMsg::SlotClick {
                slot,
                button,
                shift,
            } => self.handle_slot_click(slot, button, shift),
            ClientMsg::CloseMenu => self.close_menu(),
            ClientMsg::Chat(text) => {
                let name = self.players.me().name.clone();
                self.chat(&format!("<{name}> {text}"));
            }
            ClientMsg::Command(line) => self.handle_command(&line),
            ClientMsg::Respawn => {
                if self.players.current_is_local() && self.players.me().dead {
                    self.respawn();
                } else {
                    log::debug!("réapparition ignorée : le joueur n'est pas mort");
                }
            }
        }
    }

    /// Refait la visée du joueur dans son monde, puis lance `action` avec elle. Un œil trop loin de
    /// celui du joueur refuse l'action.
    fn with_aim(&mut self, player: PlayerId, aim: Aim, action: impl FnOnce(&mut Self)) -> bool {
        let Some(acting) = self.resolve_aim(player, aim) else {
            return false;
        };
        debug_assert_eq!(acting.player, self.players.current_id());
        let previous = self.acting.replace(acting);
        action(self);
        self.acting = previous;
        true
    }

    /// Le lancer de rayon du simulateur, depuis l'œil et le regard du client.
    pub(crate) fn resolve_aim(&self, player: PlayerId, aim: Aim) -> Option<ActingAim> {
        let state = self.players.get(player)?;
        let server_eye = player::eye(&self.ecs, state.entity, 1.0);
        if !eye_is_plausible(aim.eye, server_eye) {
            log::warn!(
                "visée refusée : l'œil annoncé {:?} est loin de {:?}",
                aim.eye,
                server_eye
            );
            return None;
        }
        cast_aim(
            self.streamer().world(),
            &self.ecs,
            player,
            aim,
            state.gear.reach(),
        )
    }

    // ----- Le mouvement -----

    /// Contrôle un mouvement que le client a déjà joué. Un refus ramène le joueur à sa dernière
    /// position acceptée.
    fn handle_move(&mut self, player: PlayerId, next: MoveSample) {
        let Some(state) = self.players.get(player) else {
            return;
        };
        let Some(previous) = state.last_move else {
            // Le premier mouvement n'a rien à quoi se comparer.
            self.players.me_mut().last_move = Some(next);
            return;
        };
        let limits = MoveLimits::new(next.mode, state.gear.move_speed());
        let result = {
            let streamer = self.streamer();
            let world = streamer.world();
            movement_check(&previous, &next, limits, |cell| {
                world.is_solid(cell) && world.collision_boxes(cell).is_some_and(|b| b.is_empty())
            })
        };
        match result {
            Ok(()) => self.players.me_mut().last_move = Some(next),
            Err(refusal) => {
                log::warn!(
                    "mouvement refusé ({refusal:?}) : retour en {}",
                    previous.position
                );
                self.replies.push((
                    player,
                    ServerMsg::Correct {
                        position: previous.position,
                    },
                ));
                if let MoveRefusal::Invalid = refusal {
                    self.players.me_mut().last_move = None;
                }
            }
        }
    }

    /// Le joueur a été déplacé par le jeu (réapparition, téléportation) : sa prochaine position
    /// n'est pas à comparer avec l'ancienne.
    pub(crate) fn forget_last_move(&mut self) {
        self.players.local_mut().last_move = None;
    }

    // ----- Le minage -----

    /// Le temps que prend le joueur courant à casser `block` en `pos` avec ce qu'il tient. `None` :
    /// incassable. 0 : instantané.
    pub(crate) fn mining_time(&self, pos: IVec3, block: BlockId) -> Option<f32> {
        let tool = self.held_tool();
        let time = engine_voxel::break_time(&self.mining_def(pos, block), tool.as_ref())?;
        // La vitesse de minage (Efficacité, affixes) est un attribut.
        Some(
            time / self
                .players
                .me()
                .gear
                .value(attributes::Attribute::MiningSpeed),
        )
    }

    fn handle_dig(&mut self, player: PlayerId, aim: Aim, phase: DigPhase, pos: IVec3) {
        match phase {
            DigPhase::Abort => self.players.me_mut().dig = None,
            DigPhase::Start => {
                self.with_aim(player, aim, |game| {
                    let aimed = game.aim().hit.map(|hit| hit.block_pos);
                    if aimed != Some(pos) {
                        log::debug!(
                            "minage refusé : le simulateur ne voit pas {pos} dans la visée"
                        );
                        game.players.me_mut().dig = None;
                        return;
                    }
                    let block = game.streamer().world().block(pos);
                    // La première image est créditée comme chez le client (qui y a déjà avancé).
                    let first = game.frame_dt;
                    game.players.me_mut().dig = game
                        .mining_time(pos, block)
                        .map(|time| start_dig(pos, block, time, first));
                });
            }
            DigPhase::Finish => self.finish_dig(player, aim, pos),
        }
    }

    /// `Dig Finish` : le même bloc qu'au début, à portée, et assez de temps écoulé.
    fn finish_dig(&mut self, player: PlayerId, aim: Aim, pos: IVec3) {
        let Some(dig) = self.players.me_mut().dig.take() else {
            log::warn!("minage refusé : {pos} n'a pas été commencé");
            return;
        };
        let reach = self.players.me().gear.reach();
        let block_now = self.streamer().world().block(pos);
        if let Some(why) = dig_refusal(&dig, pos, block_now, aim.eye, reach) {
            // Au lot 3, le simulateur renverra l'état du bloc au client ; ici, un journal.
            log::warn!(
                "minage refusé en {pos} : {why} ({:.0} % du temps)",
                dig.progress * 100.0
            );
            return;
        }
        self.with_aim(player, aim, |game| {
            game.break_block(pos);
        });
    }

    /// À chaque pas fixe : le temps de minage théorique avance pour tous les joueurs qui creusent,
    /// avec l'outil qu'ils tiennent à ce moment.
    pub(crate) fn tick_digs(&mut self, dt: f32) {
        let digging: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| p.dig.is_some())
            .map(|p| p.id)
            .collect();
        for id in digging {
            let previous = self.players.set_acting(Some(id));
            if let Some(dig) = self.players.me().dig {
                let time = self.mining_time(dig.pos, dig.block);
                if let Some(state) = self.players.me_mut().dig.as_mut() {
                    advance_dig(state, time, dt);
                }
            }
            self.players.set_acting(previous);
        }
    }

    /// Le repas : le bouton tenu fait mâcher à chaque pas fixe.
    pub(crate) fn tick_eating(&mut self, dt: f32) {
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            let previous = self.players.set_acting(Some(id));
            let holding = self.players.me().eating;
            let progress = self.update_eating(holding, dt);
            self.players.me_mut().eat_progress = progress;
            self.players.set_acting(previous);
        }
    }

    /// Fabrique le message `Move` du joueur local après la physique d'un pas, et l'applique. Sans
    /// `VOXEL_CHECK_MOVES=1`, rien ne part.
    pub(crate) fn check_local_move(&mut self) {
        if !self.check_moves {
            return;
        }
        let entity = self.players.entity();
        // Les vols automatiques, les wagons et les structures portent le joueur : pas de contrôle.
        if self.autofly.is_some()
            || self.demo.is_some()
            || self.bench.is_some()
            || self.tour.is_some()
            || self.ecs.has::<carts::Riding>(entity)
            || player::frozen(&self.ecs, entity)
        {
            self.players.local_mut().last_move = None;
            return;
        }
        let (Some(transform), Some(collider), Some(velocity)) = (
            self.ecs.get::<Transform>(entity).map(|t| t.position),
            self.ecs.get::<Collider>(entity).map(|c| *c),
            self.ecs.get::<Velocity>(entity).map(|v| v.0),
        ) else {
            return;
        };
        let msg = ClientMsg::Move {
            position: transform,
            velocity,
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            on_ground: collider.on_ground,
            mode: player::mode(&self.ecs, entity),
            step: self.sim_step,
        };
        self.send_now(msg);
    }

    // ----- Les coups -----

    /// Un coup : sur la créature que le simulateur voit sous le regard, sinon sur le wagon. Ce que
    /// le client désigne doit être ce que le simulateur voit.
    fn handle_attack(&mut self, claimed: Entity) {
        let aim = self.aim();
        if aim.mob == Some(claimed) {
            let strike = self.strike_mob(claimed, aim.forward);
            if strike.is_some() {
                self.exert(self.hunger_rules.attack);
            }
            if self.held_tool().is_some() {
                self.wear_held();
            }
            self.outcome = strike.map_or(Outcome::Nothing, Outcome::Struck);
        } else if aim.mob.is_none() && self.aimed_cart() == Some(claimed) {
            if self.punch_cart() {
                self.outcome = Outcome::Punched;
            }
        } else {
            log::debug!("coup refusé : le simulateur ne voit pas {claimed} dans la visée");
        }
    }

    // ----- Le clic droit -----

    /// Un clic droit : les usages s'enchaînent (wagon, structure, porte, lit, conteneur, machine,
    /// habitant, animal, houe, engrais, charme, guide, objet de mod), puis on pose le bloc tenu.
    fn handle_use(&mut self, sneaking: bool) {
        let block_pos = self.aim().hit.map(|hit| hit.block_pos);
        let used = self.use_cart()
            || self.use_ship()
            || (sneaking && self.place_conduit_first())
            || self.use_assembler()
            || block_pos.is_some_and(|pos| self.use_door(pos) || self.use_bed(pos))
            || self.open_container()
            || self.use_machine()
            || self.trade_with_villager()
            || self.feed_mob()
            || self.use_hoe()
            || self.use_fertilizer()
            || self.use_charm()
            || self.use_guide()
            || self.use_mod_item();
        self.outcome = if used {
            Outcome::Used
        } else {
            self.place_at_target()
                .map_or(Outcome::Nothing, Outcome::Placed)
        };
    }

    /// Jette un exemplaire (ou toute la pile) en conservant son usure et son enchantement.
    pub(crate) fn throw_selected(&mut self, all: bool) {
        let Some(stack) = self.players.me().inventory.selected_stack() else {
            return;
        };
        let aim = self.aim();
        let center = aim.eye + aim.forward * 0.45;
        if self.streamer().world().is_solid(center.floor().as_ivec3()) {
            return;
        }
        let count = if all { stack.count } else { 1 };
        for _ in 0..count {
            self.players.me_mut().inventory.consume_selected();
        }
        let seed = self.next_random();
        let entity = drops::spawn(
            &mut self.ecs,
            inventory::ItemStack { count, ..stack },
            center,
            seed,
        );
        if let Some(mut velocity) = self.ecs.get_mut::<components::Velocity>(entity) {
            velocity.0 = aim.forward * 3.5 + Vec3::Y;
        }
        self.punch();
    }

    // ----- Les cases -----

    /// Un clic sur une case : celle du joueur, ou d'un conteneur qu'il a ouvert.
    fn handle_slot_click(&mut self, slot: SlotTarget, button: ClickButton, shift: bool) {
        let Some(target) = self.slot_ref(slot) else {
            log::warn!("clic de case refusé : {slot:?}");
            return;
        };
        // Maj + clic : toute la pile passe de l'autre côté ; une case que ce geste ne concerne pas
        // (amélioration, filtre) se comporte comme un clic ordinaire.
        if shift && self.quick_move(target) {
            return;
        }
        self.click_target(target, button.into());
    }

    /// La case désignée par un message, si le joueur courant peut la toucher.
    fn slot_ref(&self, slot: SlotTarget) -> Option<SlotRef> {
        slot_ref_for(self.players.me(), slot, &|id| self.server_entity(id))
    }

    /// Ouvre l'écran d'un conteneur pour le joueur courant. L'interface ne s'ouvre que pour le
    /// joueur de la fenêtre.
    pub(crate) fn open_container_screen(&mut self, screen: Screen) {
        self.players.me_mut().open = match screen {
            Screen::Chest(pos) => Some(OpenContainer::Chest(pos)),
            Screen::Furnace(pos) | Screen::Brewing(pos) => Some(OpenContainer::Machine(pos)),
            Screen::Controller(pos) => Some(OpenContainer::Controller(pos)),
            Screen::Cart(cart) => Some(OpenContainer::Cart(cart)),
            Screen::Bag(number) => Some(OpenContainer::Bag(number)),
            _ => None,
        };
        if self.players.current_is_local() {
            self.ui.screen = screen;
            self.ui.inventory_open = true;
        }
    }

    /// `CloseMenu` : la main et la grille d'artisanat retournent dans l'inventaire (ce qui n'y tient
    /// pas tombe par terre), et le conteneur n'est plus ouvert.
    pub(crate) fn close_menu(&mut self) {
        self.players.me_mut().open = None;
        let loose = self
            .players
            .me_mut()
            .inventory
            .put_away(&self.content.items);
        let at = player::body(&self.ecs, self.players.entity())
            .position
            .floor()
            .as_ivec3()
            + IVec3::Y;
        for stack in loose {
            self.drop_stack(at, stack);
        }
    }

    // ----- La console -----

    /// Une ligne de console : une commande du jeu ou d'un mod (sans écho ; la réponse est gardée
    /// dans `command_reply`).
    fn handle_command(&mut self, line: &str) {
        let line = line.trim().trim_start_matches('/').trim();
        if line.is_empty() {
            return;
        }
        let (name, args) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        self.command_reply = Some(self.console_reply(&name.to_lowercase(), args.trim()));
    }

    /// La case de l'interface, telle qu'elle part dans un message.
    pub(crate) fn slot_target_of(&self, target: SlotRef) -> SlotTarget {
        match target {
            SlotRef::Player(slot) => SlotTarget::Player(slot),
            SlotRef::Container(pos, index) => SlotTarget::Block { pos, index },
            SlotRef::Cart(cart, index) => SlotTarget::Cart {
                cart: self.ecs.get::<NetId>(cart).map_or(NetId(0), |id| *id),
                index,
            },
            SlotRef::Bag(number, index) => SlotTarget::Bag { number, index },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use engine_ecs::World;

    use super::*;
    use crate::attributes::BaseAttributes;
    use crate::gear::Gear;
    use crate::inventory::Inventory;
    use crate::players::{PlayerIdentity, PlayerState};
    use crate::survival::{HungerRules, SurvivalRules};

    fn stone() -> BlockId {
        crate::test_registry().id("core:stone").unwrap()
    }

    /// Un joueur sans fenêtre, avec ce qu'il faut pour les clics de cases.
    fn guest(id: u32) -> PlayerState {
        PlayerState::new(
            PlayerIdentity {
                id: PlayerId(id),
                name: format!("joueur {id}"),
                uuid: u128::from(id),
            },
            Entity {
                index: id,
                generation: 0,
            },
            Inventory::default(),
            Gear::new(BaseAttributes::default()),
            &SurvivalRules::default(),
            &HungerRules::default(),
        )
    }

    /// Une plateforme de pierre dans le ciel (x de -8 à 12, z de -3 à 3, à y = 150).
    fn platform(world: &World) {
        let shared = crate::shared::StreamerExt::streamer(world);
        let mut streamer = shared.borrow_mut();
        for x in -8..13 {
            for z in -3..4 {
                streamer.set_block_raw(IVec3::new(x, 150, z), stone());
            }
        }
    }

    fn look_down_from(eye: Vec3) -> Aim {
        Aim {
            eye,
            forward: -Vec3::Y,
        }
    }

    #[test]
    fn the_simulator_casts_its_own_ray_for_each_player() {
        let world = crate::mobs::tests::sim_at(Vec3::new(5.0, 150.0, 0.5), 5);
        platform(&world);
        let shared = crate::shared::StreamerExt::streamer(&world);
        let streamer = shared.borrow();
        let reach = 4.5;
        // Deux joueurs sur la plateforme, chacun regarde le sol à ses pieds.
        for (id, x) in [(0, -4.0_f32), (1, 9.0)] {
            let aim = look_down_from(Vec3::new(x + 0.5, 152.0, 0.5));
            let acting = cast_aim(streamer.world(), &world, PlayerId(id), aim, reach).unwrap();
            let hit = acting.hit.expect("la plateforme est sous le regard");
            assert_eq!(hit.block_pos, IVec3::new(x.floor() as i32, 150, 0));
            assert_eq!(hit.normal, IVec3::Y);
            assert_eq!(acting.player, PlayerId(id));
            assert_eq!(acting.mob, None);
        }
        // Hors de portée : rien n'est touché, quoi que le client prétende viser.
        let high = look_down_from(Vec3::new(0.5, 160.0, 0.5));
        let acting = cast_aim(streamer.world(), &world, PlayerId(0), high, reach).unwrap();
        assert!(acting.hit.is_none());
        // Un regard nul ou une position qui n'est pas un nombre n'ont pas de visée.
        let nothing = Aim {
            eye: Vec3::ZERO,
            forward: Vec3::ZERO,
        };
        assert!(cast_aim(streamer.world(), &world, PlayerId(0), nothing, reach).is_none());
        let nan = look_down_from(Vec3::NAN);
        assert!(cast_aim(streamer.world(), &world, PlayerId(0), nan, reach).is_none());
    }

    #[test]
    fn a_creature_in_front_of_the_block_is_the_target() {
        let mut world = crate::mobs::tests::sim_at(Vec3::new(5.0, 150.0, 0.5), 5);
        platform(&world);
        let slime = world
            .resource::<Arc<crate::mobs::MobRegistry>>()
            .id("example:slime")
            .unwrap();
        let mob = crate::mobs::spawn_mob(&mut world, slime, Vec3::new(9.5, 151.0, 0.5)).unwrap();
        let shared = crate::shared::StreamerExt::streamer(&world);
        let streamer = shared.borrow();
        let over_mob = look_down_from(Vec3::new(9.5, 153.0, 0.5));
        let acting = cast_aim(streamer.world(), &world, PlayerId(1), over_mob, 4.5).unwrap();
        assert_eq!(acting.mob, Some(mob));
        assert!(acting.hit.is_some(), "le bloc est derrière la créature");
        // Ailleurs sur la plateforme : pas de créature.
        let elsewhere = look_down_from(Vec3::new(-4.5, 153.0, 0.5));
        let acting = cast_aim(streamer.world(), &world, PlayerId(0), elsewhere, 4.5).unwrap();
        assert_eq!(acting.mob, None);
    }

    #[test]
    fn an_eye_far_from_the_player_is_refused() {
        let server = Vec3::new(10.0, 70.0, 10.0);
        assert!(eye_is_plausible(server, server));
        // Un peu d'avance de la caméra (interpolation) passe.
        assert!(eye_is_plausible(server + Vec3::new(0.4, -0.3, 0.1), server));
        assert!(!eye_is_plausible(server + Vec3::new(3.0, 0.0, 0.0), server));
        assert!(!eye_is_plausible(Vec3::NAN, server));
    }

    fn digging(progress: f32) -> DigState {
        DigState {
            pos: IVec3::new(3, 60, 8),
            block: stone(),
            progress,
        }
    }

    #[test]
    fn finishing_a_dig_too_early_is_refused() {
        let dig = digging(0.5);
        let eye = Vec3::new(3.5, 61.6, 8.5);
        assert_eq!(
            dig_refusal(&dig, dig.pos, dig.block, eye, 4.5),
            Some("trop tôt")
        );
    }

    #[test]
    fn finishing_a_dig_in_time_is_accepted() {
        let eye = Vec3::new(3.5, 61.6, 8.5);
        let dig = digging(DIG_MIN_PROGRESS);
        assert_eq!(dig_refusal(&dig, dig.pos, dig.block, eye, 4.5), None);
        // Le temps s'écoule en pas fixes : 0,7 s de minage sur un bloc qui en demande 1.
        let mut counted = start_dig(dig.pos, dig.block, 1.0, 1.0 / 60.0);
        for _ in 0..42 {
            advance_dig(&mut counted, Some(1.0), 1.0 / 60.0);
        }
        assert!(counted.progress >= DIG_MIN_PROGRESS);
        assert_eq!(dig_refusal(&counted, dig.pos, dig.block, eye, 4.5), None);
    }

    #[test]
    fn instant_digs_pass_at_once() {
        let dig = start_dig(IVec3::new(0, 70, 0), stone(), 0.0, 1.0 / 60.0);
        assert_eq!(dig.progress, 1.0);
        let eye = Vec3::new(0.5, 71.6, 0.5);
        assert_eq!(dig_refusal(&dig, dig.pos, dig.block, eye, 4.5), None);
        // Un outil qui rend le bloc instantané en cours de route.
        let mut slow = start_dig(dig.pos, dig.block, 8.0, 1.0 / 60.0);
        advance_dig(&mut slow, Some(0.0), 1.0 / 60.0);
        assert_eq!(slow.progress, 1.0);
    }

    #[test]
    fn a_dig_on_another_block_or_a_changed_block_is_refused() {
        let dig = digging(1.0);
        let eye = Vec3::new(3.5, 61.6, 8.5);
        assert_eq!(
            dig_refusal(&dig, IVec3::new(4, 60, 8), dig.block, eye, 4.5),
            Some("ce n'est pas le bloc commencé")
        );
        assert_eq!(
            dig_refusal(&dig, dig.pos, BlockId::AIR, eye, 4.5),
            Some("le bloc a changé entre-temps")
        );
    }

    #[test]
    fn a_dig_out_of_reach_is_refused() {
        let dig = digging(1.0);
        let far = Vec3::new(3.5, 71.6, 8.5);
        assert_eq!(
            dig_refusal(&dig, dig.pos, dig.block, far, 4.5),
            Some("hors de portée")
        );
        // À la limite de la portée (le bloc se mesure depuis son centre) : accepté.
        let edge = Vec3::new(3.5, 60.5 + 4.5 + 0.9, 8.5);
        assert_eq!(dig_refusal(&dig, dig.pos, dig.block, edge, 4.5), None);
    }

    #[test]
    fn a_player_only_clicks_their_own_slots_and_their_own_container() {
        let chest = IVec3::new(5, 64, 5);
        let mut a = guest(0);
        let b = guest(1);
        let none = |_: NetId| None;
        a.open = Some(OpenContainer::Chest(chest));
        let on_chest = SlotTarget::Block {
            pos: chest,
            index: 3,
        };
        // A a ouvert le coffre ; B non : le même message ne vaut que pour A.
        assert_eq!(
            slot_ref_for(&a, on_chest, &none),
            Some(SlotRef::Container(chest, 3))
        );
        assert_eq!(slot_ref_for(&b, on_chest, &none), None);
        // Un autre coffre que celui qu'A a ouvert est refusé.
        let elsewhere = SlotTarget::Block {
            pos: IVec3::new(9, 64, 9),
            index: 0,
        };
        assert_eq!(slot_ref_for(&a, elsewhere, &none), None);
        // Chacun clique ses propres cases ; une case qui n'existe pas est refusée.
        let own = SlotTarget::Player(inventory::Slot::Main(4));
        assert_eq!(
            slot_ref_for(&b, own, &none),
            Some(SlotRef::Player(inventory::Slot::Main(4)))
        );
        let beyond = SlotTarget::Player(inventory::Slot::Main(9999));
        assert_eq!(slot_ref_for(&b, beyond, &none), None);
        // Les sacs et les wagons de même.
        a.open = Some(OpenContainer::Bag(12));
        let bag = SlotTarget::Bag {
            number: 12,
            index: 1,
        };
        assert_eq!(slot_ref_for(&a, bag, &none), Some(SlotRef::Bag(12, 1)));
        assert_eq!(slot_ref_for(&b, bag, &none), None);
        let cart = Entity {
            index: 4,
            generation: 2,
        };
        a.open = Some(OpenContainer::Cart(cart));
        let on_cart = SlotTarget::Cart {
            cart: NetId(31),
            index: 0,
        };
        let known = |id: NetId| (id == NetId(31)).then_some(cart);
        assert_eq!(
            slot_ref_for(&a, on_cart, &known),
            Some(SlotRef::Cart(cart, 0))
        );
        assert_eq!(slot_ref_for(&b, on_cart, &known), None);
        // Un numéro que le serveur ne connaît pas ne désigne aucun wagon.
        assert_eq!(slot_ref_for(&a, on_cart, &none), None);
    }
}
