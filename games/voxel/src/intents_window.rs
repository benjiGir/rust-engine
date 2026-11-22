//! La part fenêtre de `intents.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `intents.rs`).

use super::*;
use voxel_sim::net::entities::NetId;
use voxel_sim::net::protocol::{Aim, ClientMsg, ServerMsg, SlotTarget};
#[allow(unused_imports)]
use voxel_sim::server::intents::*;
use voxel_sim::server::screens::Screen;
use voxel_sim::server::ui::SlotRef;

impl VoxelGame {
    /// La visée de la fenêtre, telle qu'elle part dans un message.
    pub(crate) fn window_aim(&self) -> Aim {
        Aim {
            eye: self.client.camera.position,
            forward: self.client.camera.forward(),
        }
    }

    /// La visée d'une commande `ctl` : l'œil vient du corps (la caméra peut retarder d'une image
    /// après un `goto`), le regard de la caméra.
    pub(crate) fn body_aim(&self) -> Aim {
        Aim {
            eye: self.client.body.eye(1.0),
            forward: self.client.camera.forward(),
        }
    }

    /// Met un message dans la file du client : il sera appliqué à la fin de la chaîne d'entrées de
    /// l'image (voir `play_input.rs`).
    pub(crate) fn send(&mut self, msg: ClientMsg) {
        self.client.outbox.push(msg);
    }

    /// Envoie et applique tout de suite (les écrans et `ctl`, qui ne sont pas dans la chaîne
    /// d'entrées).
    pub(crate) fn send_now(&mut self, msg: ClientMsg) {
        self.client.outbox.push(msg);
        self.flush_outbox();
    }

    /// Applique les messages du joueur local dans l'ordre, puis les réponses du simulateur.
    pub(crate) fn flush_outbox(&mut self) {
        let local = self.server.players.local_id();
        // Le regard de la fenêtre, quand il a changé (le serveur le garde pour la sauvegarde).
        let look = (self.client.camera.yaw, self.client.camera.pitch);
        if look != self.client.last_look {
            self.client.last_look = look;
            self.client.outbox.insert(
                0,
                ClientMsg::Look {
                    yaw: look.0,
                    pitch: look.1,
                },
            );
        }
        let mut guard = 0;
        // Un message peut en provoquer d'autres (fermer un écran...) : on vide jusqu'au bout.
        while !self.client.outbox.is_empty() && guard < 64 {
            guard += 1;
            for msg in std::mem::take(&mut self.client.outbox) {
                // Un invité envoie ses messages à l'hôte (voir `net_client.rs`).
                if self.hosting() {
                    self.server.handle(local, msg);
                } else {
                    self.send_remote(&msg);
                }
            }
            if self.hosting() {
                self.run_pending_commands();
            }
        }
        // Les vues du joueur (inventaire, conteneur ouvert) suivent tout de suite : un clic dans un
        // écran se voit dès cette image.
        if self.hosting() && (!self.server.replies.is_empty() || guard > 0) {
            self.server.replicate_player_views();
        }
        self.apply_replies();
    }

    /// Les commandes de console que le serveur a reçues : la fenêtre hôte les exécute (elles passent
    /// par le pilotage `ctl`, voir `handle_command`).
    fn run_pending_commands(&mut self) {
        for line in std::mem::take(&mut self.server.pending_commands) {
            let (name, args) = line.split_once(char::is_whitespace).unwrap_or((&line, ""));
            let reply = self.console_reply(&name.to_lowercase(), args.trim());
            self.server.command_reply = Some(reply);
        }
    }

    /// Les réponses du simulateur au joueur de la fenêtre. Celles des autres joueurs partiront sur
    /// le réseau (lot 5).
    pub(crate) fn apply_replies(&mut self) {
        // Celles des joueurs du réseau partent sur leur connexion (voir `net_host.rs`) ; un invité
        // reçoit les siennes de l'hôte.
        let replies = if self.hosting() {
            self.server.route_replies()
        } else {
            self.receive_remote()
        };
        for reply in replies {
            match reply {
                ServerMsg::Correct {
                    position,
                    velocity,
                    carried,
                } => {
                    self.client.body.correct(position, velocity, carried);
                    self.client.camera.position = self.client.body.eye(1.0);
                }
                ServerMsg::Push { up, impulse } => self.client.body.push(up, impulse),
                ServerMsg::Sky(view) => self.receive_sky(view),
                ServerMsg::Turn(turn) => self.client.camera.yaw += turn,
                ServerMsg::Feedback(feedback) => self.apply_feedback(feedback),
                ServerMsg::Chat(line) => self.show_chat(&line),
                ServerMsg::PlayerList(names) => self.client.online = names,
                ServerMsg::ModOptions { mod_id, data } => self.show_mod_options(mod_id, &data),
                ServerMsg::ModHud { mod_id, id, bar } => self.receive_mod_hud(mod_id, id, bar),
                ServerMsg::OpenScreen(screen) => self.show_screen(screen),
                ServerMsg::CloseScreen => self.close_screen(),
                ServerMsg::ScreenView(Some(
                    voxel_sim::server::screen_actions::ScreenView::Mod { owner, at, data },
                )) => {
                    self.receive_mod_menu(owner.clone(), at, data.clone());
                    self.client.view.receive(ServerMsg::ScreenView(Some(
                        voxel_sim::server::screen_actions::ScreenView::Mod { owner, at, data },
                    )));
                }
                ServerMsg::Died { message } => {
                    self.client.mining = None;
                    // L'écran ouvert se ferme, la souris est rendue au menu de mort.
                    if self.client.ui.inventory_open {
                        self.close_screen();
                        self.client.ui.inventory_open = false;
                    }
                    self.mode = Mode::Dead(message);
                }
                ServerMsg::Respawned => self.mode = Mode::Playing,
                // Le monde que voit la fenêtre : la réplique l'applique à la prochaine mise à jour
                // de ses vues (voir `client_world.rs`).
                chunk_msg => self.client.view.receive(chunk_msg),
            }
        }
        self.client.view.apply_received();
    }

    // ----- Le mouvement -----

    // ----- Le minage -----

    /// Envoie au serveur le mouvement que la physique de la fenêtre vient de jouer (voir
    /// `predicted.rs`).
    pub(crate) fn send_local_move(&mut self, walk: Vec3, jump: bool) {
        // Les vols automatiques et les démos déplacent le joueur à leur guise : le contrôle de
        // `VOXEL_CHECK_MOVES=1` repart de zéro à chaque pas.
        if self.client.autofly.is_some()
            || self.client.demo.is_some()
            || self.client.bench.is_some()
            || self.client.tour.is_some()
        {
            self.server.players.local_mut().last_move = None;
        }
        let body = self.client.body;
        let msg = ClientMsg::Move {
            position: body.feet(),
            velocity: body.body.velocity,
            yaw: self.client.camera.yaw,
            pitch: self.client.camera.pitch,
            on_ground: body.body.on_ground,
            in_liquid: body.body.in_liquid,
            walk,
            jump,
            frozen: body.frozen,
            mode: body.mode,
            step: self.server.sim_step,
        };
        self.send_now(msg);
    }

    // ----- Les coups -----

    // ----- Le clic droit -----

    // ----- Les cases -----

    /// La fenêtre ouvre l'écran que le serveur lui a ouvert.
    pub(crate) fn show_screen(&mut self, screen: Screen) {
        match screen {
            Screen::Trade(_) => self.client.ui.trade_selected = 0,
            Screen::Mod(_) if self.client.ui.screen == screen && self.client.ui.inventory_open => {
                return;
            }
            _ => {}
        }
        self.client.ui.screen = screen;
        self.client.ui.inventory_open = true;
    }

    // ----- La console -----

    /// La case de l'interface, telle qu'elle part dans un message.
    pub(crate) fn slot_target_of(&self, target: SlotRef) -> SlotTarget {
        match target {
            SlotRef::Player(slot) => SlotTarget::Player(slot),
            SlotRef::Open(index) => SlotTarget::Open(index),
            SlotRef::Container(pos, index) => SlotTarget::Block { pos, index },
            SlotRef::Cart(cart, index) => SlotTarget::Cart {
                cart: self
                    .server
                    .ecs
                    .get::<NetId>(cart)
                    .map_or(NetId(0), |id| *id),
                index,
            },
            SlotRef::Bag(number, index) => SlotTarget::Bag { number, index },
        }
    }
}
