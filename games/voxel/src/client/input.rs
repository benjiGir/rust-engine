//! Le côté qui lit les entrées : la souris, les touches de la barre d'objets, manger, jeter.
//!
//! Cette chaîne ne touche jamais au monde. Elle lit les boutons et fabrique des intentions
//! (`net/protocol.rs`) dans `self.outbox` ; `update` vide la file à la fin de la chaîne, dans la
//! même image : le simulateur (`server/intents.rs`) les applique sans aucun délai.
//!
//! Ce qui reste ici, du côté client, est ce que le joueur ressent tout de suite : la fissure qui
//! avance, le bras qui frappe, le son d'un coup. Le minage est **prédit** : la fissure grandit
//! chez le client, le simulateur compte son propre temps et vérifie `Dig Finish`.

use crate::key_codes::KeyCodes;
use crate::*;
use voxel_sim::net::protocol::{ClientMsg, DigPhase};

impl VoxelGame {
    /// Direction voulue d'après les touches. Horizontale en marche ; en vol,
    /// Espace et Maj ajoutent la verticale.
    pub(crate) fn wish_direction(&self, ctx: &Context, vertical: bool) -> Vec3 {
        let input = &ctx.input;
        let keys = &self.settings.keys;
        let mut bindings = vec![
            (keys.key(Action::Forward), self.client.camera.forward_flat()),
            (keys.key(Action::Back), -self.client.camera.forward_flat()),
            (keys.key(Action::Right), self.client.camera.right()),
            (keys.key(Action::Left), -self.client.camera.right()),
        ];
        if vertical {
            bindings.push((keys.key(Action::Jump), Vec3::Y));
            bindings.push((keys.key(Action::Descend), -Vec3::Y));
        }
        // Chaque touche ajoute une direction ; on somme puis on normalise.
        // `normalize_or_zero` évite une division par zéro quand aucune touche n'est tenue.
        bindings
            .iter()
            .filter(|(key, _)| input.key_held(*key))
            .map(|(_, dir)| *dir)
            .sum::<Vec3>()
            .normalize_or_zero()
    }

    /// Renvoie `true` si ce clic a servi à capturer la souris (il ne casse donc rien).
    pub(crate) fn handle_mouse_capture(&mut self, ctx: &mut Context) -> bool {
        let input = &ctx.input;
        if !self.client.mouse_captured && input.mouse_pressed(MouseButton::Left) {
            self.client.mouse_captured = true;
            ctx.set_cursor_captured(true);
            return true;
        }
        // Échap ne quitte jamais le jeu (on quitte par le menu) : elle rend seulement la souris.
        if input.key_pressed(KeyCode::Escape) && self.client.mouse_captured {
            self.client.mouse_captured = false;
            ctx.set_cursor_captured(false);
        }
        false
    }

    pub(crate) fn look(&mut self, delta: Vec2) {
        let sensitivity = MOUSE_SENSITIVITY * self.settings.mouse_sensitivity;
        self.client.camera.yaw += delta.x * sensitivity;
        self.client.camera.pitch = (self.client.camera.pitch - delta.y * sensitivity)
            .clamp(-Camera::MAX_PITCH, Camera::MAX_PITCH);
    }

    /// Les touches de la partie : la barre d'objets (messages), jeter (message), les écrans, le
    /// vol et les réglages d'affichage.
    pub(crate) fn handle_keys(&mut self, ctx: &Context) {
        let input = &ctx.input;
        if self.client.mouse_captured
            && !self.client.ui.inventory_open
            && input.key_pressed(self.settings.keys.key(Action::Drop))
        {
            let aim = self.window_aim();
            self.send(ClientMsg::Drop { aim, all: false });
        }
        // Le sac à dos : la même touche l'ouvre (le premier sac de l'inventaire) et le referme.
        if input.key_pressed(self.settings.keys.key(Action::Backpack))
            && !self.client.ui.ctx.egui_wants_keyboard_input()
        {
            if self.client.ui.inventory_open && matches!(self.client.ui.screen, ui::Screen::Bag(_))
            {
                self.close_screen();
            } else if self.client.mouse_captured && !self.client.ui.inventory_open {
                self.open_first_bag();
            }
        }
        // Les paliers du monde : la même touche ouvre l'écran et le referme.
        if input.key_pressed(self.settings.keys.key(Action::Tiers))
            && !self.client.ui.ctx.egui_wants_keyboard_input()
        {
            self.toggle_tiers_screen();
        }
        // Le livre de quêtes : la même touche l'ouvre et le referme.
        if input.key_pressed(self.settings.keys.key(Action::Quests))
            && !self.client.ui.ctx.egui_wants_keyboard_input()
        {
            self.toggle_quests_screen();
        }
        if input.key_pressed(self.settings.keys.key(Action::Time)) {
            self.server.day.cycle_speed();
            log::info!(
                "temps {} ({})",
                self.server.day.speed.name(),
                self.server.day.clock()
            );
        }
        if input.key_pressed(self.settings.keys.key(Action::Fly)) {
            self.toggle_fly();
            log::info!("mode {}", self.client.body.mode.name());
        }
        for (i, key) in HOTBAR_KEYS.iter().enumerate() {
            if input.key_pressed(*key) {
                self.send(ClientMsg::SelectSlot(i as u8));
            }
        }
        if input.key_pressed(KeyCode::F3) {
            self.client.ui.inspector_open = !self.client.ui.inspector_open;
        }
        if input.key_pressed(KeyCode::F4) {
            self.client.hand.visible = !self.client.hand.visible;
        }
        if input.key_pressed(KeyCode::F1) {
            let mesher = self.view_streamer().config().mesher.other();
            log::info!("maillage {} : on remaille tout", mesher.name());
            self.client.view.replica_mut().set_mesher(mesher);
        }
        if input.key_pressed(KeyCode::F6) {
            self.toggle_cave_culling();
        }
        if input.key_pressed(KeyCode::F2) {
            self.take_screenshot(ctx);
        }
        if input.key_pressed(KeyCode::F7) {
            self.client.renderer.frustum_culling = !self.client.renderer.frustum_culling;
            let state = if self.client.renderer.frustum_culling {
                "activé"
            } else {
                "désactivé"
            };
            log::info!("frustum culling {state}");
        }
    }

    /// F2 : une capture de l'écran, interface comprise, dans `screenshots/` (nommée d'après la
    /// date, en temps universel).
    fn take_screenshot(&mut self, ctx: &Context) {
        let size = ctx.window.inner_size();
        let (width, height) = (size.width.max(16), size.height.max(16));
        let dir = &voxel_sim::paths::get().screenshots;
        let stamp = voxel_sim::logs::stamp(std::time::SystemTime::now());
        let mut path = dir.join(format!("{stamp}.png"));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{stamp}_{n}.png"));
            n += 1;
        }
        let (ui, _) = self.build_ui(crate::ui::offscreen_input(width, height));
        let result = self
            .client
            .renderer
            .capture(&self.client.camera, width, height, Some(ui))
            .and_then(|capture| {
                std::fs::create_dir_all(dir)?;
                crate::control::screenshots::save_png(
                    &path,
                    capture.width,
                    capture.height,
                    &capture.rgba,
                )
            });
        match result {
            Ok(()) => log::info!("capture : {}", path.display()),
            Err(error) => log::error!("capture impossible : {error:#}"),
        }
    }

    /// La souris d'une image : le bloc visé, le minage, les coups, le clic droit et le repas. Tout
    /// ce qui agit sur le monde part en messages, vidés à la fin (même image).
    ///
    /// `click_used` : le clic de cette image a servi à autre chose (capturer la souris, un écran
    /// ouvert) ; il ne casse ni ne pose rien.
    pub(crate) fn play_input(&mut self, ctx: &Context, dt: f32, click_used: bool) {
        // Le bloc visé, depuis la caméra de la dernière image, dans le monde que voit la fenêtre
        // (la réplique) : le serveur refait lui-même son lancer de rayon dans `handle`.
        let target = raycast(
            self.view_world(),
            self.client.camera.position,
            self.client.camera.forward(),
            self.client.view.me.reach(),
        );
        self.client.target = target;
        // `ctl hold` tient un bouton comme si la souris était capturée.
        let hold = self.remote.hold;
        let captured = self.client.mouse_captured || hold.is_some();
        let input = &ctx.input;
        let left_held = input.mouse_held(MouseButton::Left) || hold.is_some_and(|h| h.left);
        let left_pressed =
            input.mouse_pressed(MouseButton::Left) || hold.is_some_and(|h| h.left && h.fresh);
        let right_held = input.mouse_held(MouseButton::Right) || hold.is_some_and(|h| h.right);
        let right_pressed =
            input.mouse_pressed(MouseButton::Right) || hold.is_some_and(|h| h.right && h.fresh);
        if let Some(hold) = &mut self.remote.hold {
            hold.fresh = false;
        }
        let click_used = click_used && hold.is_none();
        // Le minage : le bouton gauche maintenu sur le même bloc.
        let mining = captured && !click_used && left_held;
        self.client.air_swing = false;
        if !mining {
            self.stop_mining();
        }
        if captured && !click_used {
            if mining && !left_pressed {
                self.predict_mining(dt);
            }
            if left_pressed {
                self.press_attack(dt);
            }
            // Rien à frapper ni à casser : le bras donne un coup dans le vide, tant que le bouton
            // est tenu.
            self.client.air_swing = mining && self.client.mining.is_none();
            // Un clic droit : une porte, un lit, un conteneur, sinon poser.
            if right_pressed {
                let sneaking =
                    input.key_held(KeyCode::ShiftLeft) || input.key_held(KeyCode::ShiftRight);
                let aim = self.window_aim();
                self.send(ClientMsg::Use { aim, sneaking });
            }
        }
        // Manger : le bouton droit tenu avec un aliment en main (le message ne part que quand le
        // bouton change ; le simulateur mâche à chaque pas fixe).
        let eating = captured && !click_used && right_held;
        if eating != self.client.eating_sent {
            self.client.eating_sent = eating;
            self.send(ClientMsg::Eat { held: eating });
        }
        self.flush_outbox();
    }

    /// Le bouton gauche vient d'être pressé : une créature ou un wagon devant le bloc visé est
    /// frappé, sinon on commence à casser. La cible est choisie dans la réplique des entités (ce
    /// que le joueur voit) ; le simulateur revérifie.
    fn press_attack(&mut self, dt: f32) {
        let message_aim = self.window_aim();
        if let Some(id) = self.window_entity() {
            self.send(ClientMsg::Attack {
                aim: message_aim,
                entity: id,
            });
            // Les retours immédiats restent ici : le bras frappe, le coup résonne (sur une
            // créature).
            let mob = self.client.view.entities.get(id).is_some_and(|view| {
                matches!(view.kind, voxel_sim::net::entities::EntityKind::Mob { .. })
            });
            if mob {
                self.client.hand.punch();
                if let Some(audio) = &mut self.client.audio {
                    audio.play("core:break_soft");
                }
            }
        } else {
            self.predict_mining(dt);
        }
    }

    /// Le minage côté client, à chaque image où le bouton gauche est tenu : la fissure grandit
    /// selon la dureté du bloc et l'outil en main ; à 1, `Dig Finish` part. Le simulateur compte
    /// de son côté et décide.
    fn predict_mining(&mut self, dt: f32) {
        let Some(hit) = self.client.target else {
            self.stop_mining();
            return;
        };
        let pos = hit.block_pos;
        let block = self.server.streamer().world().block(pos);
        let Some(time) = self.server.mining_time(pos, block) else {
            // Incassable.
            self.stop_mining();
            return;
        };
        let progress = match self.client.mining {
            Some((p, progress)) if p == pos => Some(progress),
            _ => None,
        };
        if progress.is_none() {
            let aim = self.window_aim();
            self.send(ClientMsg::Dig {
                aim,
                phase: DigPhase::Start,
                pos,
                face: hit.normal,
            });
        }
        let progress = if time <= 0.0 {
            1.0
        } else {
            progress.unwrap_or(0.0) + dt / time
        };
        if progress >= 1.0 {
            self.client.mining = None;
            let aim = self.window_aim();
            self.send(ClientMsg::Dig {
                aim,
                phase: DigPhase::Finish,
                pos,
                face: hit.normal,
            });
        } else {
            self.client.mining = Some((pos, progress));
        }
    }

    /// Le minage s'arrête avant la fin (bouton lâché, un menu s'ouvre, plus de bloc visé).
    pub(crate) fn stop_mining(&mut self) {
        if let Some((pos, _)) = self.client.mining.take() {
            let aim = self.window_aim();
            self.send(ClientMsg::Dig {
                aim,
                phase: DigPhase::Abort,
                pos,
                face: IVec3::ZERO,
            });
        }
    }
}
