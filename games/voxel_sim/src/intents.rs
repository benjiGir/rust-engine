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

use crate::entities::{NetId, NetIds};
use crate::gear::Strike;
use crate::movement::{MoveLimits, MoveRefusal, MoveSample, movement_check};
use crate::players::{DigState, OpenContainer, PlayerId};
use crate::protocol::{Aim, ClickButton, ClientMsg, DigPhase, ServerMsg, SlotTarget};
use crate::screens::Screen;
use crate::ui_server::SlotRef;
use crate::{attributes, components, drops, inventory, mobs, player};
use anyhow::Result;
use engine_ecs::Entity;
use engine_voxel::BlockId;
use engine_voxel::RayHit;
use engine_voxel::raycast;
use glam::IVec3;
use glam::Vec3;

/// De combien l'œil annoncé par le client peut s'écarter de l'œil du joueur côté simulateur.
const EYE_TOLERANCE: f32 = 1.5;
/// La part du temps de minage théorique qui doit s'être écoulée avant `Dig Finish`.
const DIG_MIN_PROGRESS: f32 = 0.7;
/// Ce que `Dig Finish` tolère en plus de la portée : le bloc se mesure depuis son centre.
const DIG_REACH_MARGIN: f32 = 1.0;

/// La visée d'une action : l'œil, le regard, et ce que le rayon touche dans le monde du joueur.
#[derive(Debug, Clone, Copy)]
pub struct ActingAim {
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
pub enum Outcome {
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
pub struct CommandReply {
    pub text: String,
    pub json: Option<Result<String>>,
}

/// L'œil annoncé par le client est-il celui du joueur (à la tolérance près) ?
pub fn eye_is_plausible(claimed: Vec3, server: Vec3) -> bool {
    claimed.is_finite() && claimed.distance(server) <= EYE_TOLERANCE
}

/// Le lancer de rayon du simulateur : le bloc touché dans `reach`, et la créature touchée si elle
/// est plus près que ce bloc.
pub fn cast_aim(
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
pub fn start_dig(pos: IVec3, block: BlockId, time: f32, first: f32) -> DigState {
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
pub fn advance_dig(dig: &mut DigState, time: Option<f32>, dt: f32) {
    dig.progress = match time {
        Some(time) if time > 0.0 => (dig.progress + dt / time).min(1.0),
        _ => 1.0,
    };
}

/// Pourquoi `Dig Finish` est refusé, s'il l'est : ce n'est pas le bloc commencé, le bloc a changé,
/// il est hors de portée, ou le temps de minage n'a pas pu s'écouler.
pub fn dig_refusal(
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
pub fn slot_ref_for(
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
        SlotTarget::Open(index) => match open? {
            OpenContainer::Chest(pos) | OpenContainer::Machine(pos) => {
                Some(SlotRef::Container(pos, index))
            }
            OpenContainer::Cart(cart) => Some(SlotRef::Cart(cart, index)),
            OpenContainer::Bag(number) => Some(SlotRef::Bag(number, index)),
            OpenContainer::Controller(_) => None,
        },
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

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Le repas : le bouton tenu fait mâcher à chaque pas fixe.
    pub fn tick_eating(&mut self, dt: f32) {
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            let previous = self.players.set_acting(Some(id));
            let holding = self.players.me().eating;
            let progress = self.update_eating(holding, dt);
            self.players.me_mut().eat_progress = progress;
            self.players.set_acting(previous);
        }
    }

    /// À chaque pas fixe : le temps de minage théorique avance pour tous les joueurs qui creusent,
    /// avec l'outil qu'ils tiennent à ce moment.
    pub fn tick_digs(&mut self, dt: f32) {
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

    /// La visée de l'action en cours ; hors d'un message, celle de la fenêtre (le bloc visé de
    /// l'image et la caméra).
    pub fn aim(&self) -> ActingAim {
        if let Some(aim) = self.acting {
            return aim;
        }
        // Hors d'un message : l'œil et le dernier regard connu du joueur courant, sans bloc visé.
        let me = self.players.me();
        ActingAim {
            player: self.players.current_id(),
            eye: player::eye(&self.ecs, me.entity, 1.0),
            forward: me.look,
            hit: None,
            // La fenêtre choisit ce qu'elle frappe dans sa réplique (voir `window_entity`) ; le simulateur
            // refait sa propre visée à chaque message.
            mob: None,
        }
    }

    /// Ouvre un écran pour le joueur courant : le serveur note ce qu'il a ouvert (un conteneur, une
    /// table...), puis lui envoie `OpenScreen`, que sa fenêtre affiche.
    pub fn open_screen(&mut self, screen: Screen) {
        let open = match screen {
            Screen::Chest(pos) => Some(OpenContainer::Chest(pos)),
            // Une machine de mod garde ses cases dans son entité de bloc, comme un four.
            Screen::Furnace(pos) | Screen::Brewing(pos) | Screen::Mod(pos) => {
                Some(OpenContainer::Machine(pos))
            }
            Screen::Controller(pos) => Some(OpenContainer::Controller(pos)),
            Screen::Cart(cart) => self.server_entity(cart).map(OpenContainer::Cart),
            Screen::Bag(number) => Some(OpenContainer::Bag(number)),
            _ => None,
        };
        let me = self.players.me_mut();
        me.open = open;
        me.screen = Some(screen);
        let id = self.players.current_id();
        self.replies.push((id, ServerMsg::OpenScreen(screen)));
    }

    /// Jette un exemplaire (ou toute la pile) en conservant son usure et son enchantement.
    pub fn throw_selected(&mut self, all: bool) {
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

    /// La case désignée par un message, si le joueur courant peut la toucher.
    fn slot_ref(&self, slot: SlotTarget) -> Option<SlotRef> {
        slot_ref_for(self.players.me(), slot, &|id| self.server_entity(id))
    }

    /// Le lancer de rayon du simulateur, depuis l'œil et le regard du client.
    pub fn resolve_aim(&self, player: PlayerId, aim: Aim) -> Option<ActingAim> {
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

    /// Le temps que prend le joueur courant à casser `block` en `pos` avec ce qu'il tient. `None` :
    /// incassable. 0 : instantané.
    pub fn mining_time(&self, pos: IVec3, block: BlockId) -> Option<f32> {
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

    /// Le joueur a été déplacé par le jeu (réapparition, téléportation) : sa prochaine position
    /// n'est pas à comparer avec l'ancienne.
    pub fn forget_last_move(&mut self) {
        self.players.local_mut().last_move = None;
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

    /// Refait la visée du joueur dans son monde, puis lance `action` avec elle. Un œil trop loin de
    /// celui du joueur refuse l'action.
    fn with_aim(&mut self, player: PlayerId, aim: Aim, action: impl FnOnce(&mut Self)) -> bool {
        let Some(acting) = self.resolve_aim(player, aim) else {
            return false;
        };
        debug_assert_eq!(acting.player, self.players.current_id());
        // Le dernier regard connu du joueur (les scripts de mods le lisent hors d'une action).
        self.players.me_mut().look = acting.forward;
        let previous = self.acting.replace(acting);
        action(self);
        self.acting = previous;
        true
    }

    /// L'entité que retrouve le serveur sous ce numéro, si elle existe encore.
    pub fn server_entity(&self, id: NetId) -> Option<Entity> {
        self.ecs.get_resource::<NetIds>()?.entity(&self.ecs, id)
    }

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
        // Un four ou un alambic dort peut-être : on le réveille au prochain tick.
        if let SlotRef::Container(pos, _) = target {
            self.mods
                .machines
                .schedule(engine_voxel::GridPos::world(pos), 1);
        }
    }

    /// Contrôle un mouvement que le client a déjà joué. Un refus ramène le joueur à sa dernière
    /// position acceptée.
    fn handle_move(&mut self, player: PlayerId, next: MoveSample) -> bool {
        let Some(state) = self.players.get(player) else {
            return false;
        };
        let Some(previous) = state.last_move else {
            // Le premier mouvement n'a rien à quoi se comparer.
            self.players.me_mut().last_move = Some(next);
            return true;
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
            Ok(()) => {
                self.players.me_mut().last_move = Some(next);
                true
            }
            Err(refusal) => {
                log::warn!(
                    "mouvement refusé ({refusal:?}) : retour en {}",
                    previous.position
                );
                // Le mouvement n'est pas rangé : le joueur revient là où le serveur le tient.
                self.send_correct(player, false);
                if let MoveRefusal::Invalid = refusal {
                    self.players.me_mut().last_move = None;
                }
                false
            }
        }
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
                    // La première image est créditée comme chez le client (qui y a déjà avancé) : un
                    // pas fixe, la durée d'image n'étant pas connue du serveur.
                    let first = 1.0 / 60.0;
                    game.players.me_mut().dig = game
                        .mining_time(pos, block)
                        .map(|time| start_dig(pos, block, time, first));
                });
            }
            DigPhase::Finish => self.finish_dig(player, aim, pos),
        }
    }

    /// Une ligne de console : une commande du jeu ou d'un mod (sans écho ; la réponse est gardée
    /// dans `command_reply`).
    fn handle_command(&mut self, line: &str) {
        let line = line.trim().trim_start_matches('/').trim();
        if line.is_empty() {
            return;
        }
        // Les commandes de la fenêtre hôte passent encore par son pilotage (`ctl`) : elle les exécute
        // juste après ce message (`run_pending_commands`). Celles d'un joueur du réseau, le serveur
        // les exécute lui-même (voir `server_commands.rs`).
        if !self.players.current_is_local() {
            let reply = self.run_server_command(line);
            self.chat(&reply);
            return;
        }
        self.pending_commands.push(line.to_string());
    }

    /// `CloseMenu` : la main et la grille d'artisanat retournent dans l'inventaire (ce qui n'y tient
    /// pas tombe par terre), et le conteneur n'est plus ouvert.
    pub fn close_menu(&mut self) {
        self.forget_mod_menu();
        self.players.me_mut().open = None;
        self.players.me_mut().screen = None;
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

    fn apply(&mut self, player: PlayerId, msg: ClientMsg) {
        match msg {
            ClientMsg::Move {
                position,
                velocity,
                on_ground,
                in_liquid,
                walk,
                jump,
                frozen,
                mode,
                step,
                yaw,
                pitch,
            } => {
                // L'intention : la physique du serveur s'en sert quand le joueur est porté, et la
                // fatigue en vient (marche, course, saut).
                let entity = self.players.me().entity;
                crate::player::set_intent(&self.ecs, entity, walk, jump);
                let sprinting = walk.length()
                    > crate::player::WALK_SPEED * self.players.me().gear.move_speed() + 0.01;
                self.exert_movement(walk, jump, sprinting, crate::movement::STEP_SECONDS);
                let (sin_yaw, cos_yaw) = yaw.sin_cos();
                let (sin_pitch, cos_pitch) = pitch.sin_cos();
                self.players.me_mut().look =
                    Vec3::new(cos_yaw * cos_pitch, sin_pitch, sin_yaw * cos_pitch);
                // Contrôlé si `VOXEL_CHECK_MOVES=1`, puis rangé (voir `predicted.rs`).
                let sample = MoveSample {
                    position,
                    mode,
                    on_ground,
                    step,
                };
                if !self.check_moves || self.handle_move(player, sample) {
                    self.store_move(
                        player,
                        crate::predicted::StoredMove {
                            position,
                            velocity,
                            on_ground,
                            in_liquid,
                            frozen,
                            mode,
                        },
                    );
                }
            }
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
            ClientMsg::Steer(steer) => self.steer = steer.clamp(-1.0, 1.0),
            ClientMsg::ModOptionsEvent {
                mod_id,
                kind,
                id,
                value,
            } => {
                let actions = self.mods.options_event(&mod_id, kind, id, value);
                self.apply_machine_actions(actions);
            }
            ClientMsg::Dismount => {
                self.dismount();
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
            ClientMsg::ContainerOp(op) => match self.open_target() {
                Some(target) => self.storage_transfer(target, op),
                None => log::debug!("bouton de conteneur sans conteneur ouvert : ignoré"),
            },
            ClientMsg::ToggleUpgrade(slot) => {
                if let Some(OpenContainer::Bag(number)) = self.players.me().open
                    && let Some(bag) = self.backpacks.bag_mut(number)
                    && slot < 32
                {
                    bag.disabled ^= 1 << slot;
                }
            }
            ClientMsg::Craft { size, recipe } => self.craft(size, recipe),
            ClientMsg::Screen(action) => self.screen_action(action),
            ClientMsg::Look { yaw, pitch } => {
                let (sin_yaw, cos_yaw) = yaw.sin_cos();
                let (sin_pitch, cos_pitch) = pitch.sin_cos();
                let me = self.players.me_mut();
                me.yaw = yaw;
                me.pitch = pitch;
                me.look = Vec3::new(cos_yaw * cos_pitch, sin_pitch, sin_yaw * cos_pitch);
            }
            ClientMsg::Chat(text) => {
                // Le chat va à tous les joueurs.
                let name = self.players.me().name.clone();
                let line = format!("<{name}> {text}");
                let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
                for id in ids {
                    self.replies.push((id, ServerMsg::Chat(line.clone())));
                }
            }
            ClientMsg::Command(line) => self.handle_command(&line),
            ClientMsg::Respawn => {
                if self.players.me().dead {
                    self.respawn();
                } else {
                    log::debug!("réapparition ignorée : le joueur n'est pas mort");
                }
            }
        }
    }

    /// Applique une intention du client. Tout ce que le simulateur touche pendant ce temps est
    /// l'état de `player`.
    pub fn handle(&mut self, player: PlayerId, msg: ClientMsg) {
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
        // Une case « du conteneur ouvert » : le serveur sait lequel, la fenêtre n'a pas à le dire.
        assert_eq!(
            slot_ref_for(&a, SlotTarget::Open(5), &none),
            Some(SlotRef::Cart(cart, 5))
        );
        a.open = Some(OpenContainer::Chest(IVec3::new(1, 2, 3)));
        assert_eq!(
            slot_ref_for(&a, SlotTarget::Open(2), &none),
            Some(SlotRef::Container(IVec3::new(1, 2, 3), 2))
        );
        a.open = Some(OpenContainer::Bag(7));
        assert_eq!(
            slot_ref_for(&a, SlotTarget::Open(0), &none),
            Some(SlotRef::Bag(7, 0))
        );
        // Rien d'ouvert, ou le contrôleur (il n'a pas de cases) : rien.
        a.open = None;
        assert_eq!(slot_ref_for(&a, SlotTarget::Open(0), &none), None);
        a.open = Some(OpenContainer::Controller(IVec3::ZERO));
        assert_eq!(slot_ref_for(&a, SlotTarget::Open(0), &none), None);
    }
}
