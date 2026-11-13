//! Les interactions du joueur avec le monde : levier, bouton, houe, engrais, animaux et
//! habitants, fioles, et ce qui s'y rattache (potions en cours, musique, flèche d'orientation).

use super::*;

impl VoxelGame {
    /// Maj + clic droit avec un conduit : poser contre une machine sans ouvrir son menu.
    pub(crate) fn place_conduit_first(&mut self) -> bool {
        let Some(block) = self
            .inventory
            .selected_item()
            .and_then(|item| self.content.items.get(item).block)
        else {
            return false;
        };
        if self.registry.def(block).connections.is_none() {
            return false;
        }
        self.place_at_target();
        true
    }
    pub(crate) fn handle_keys(&mut self, ctx: &Context) {
        let input = &ctx.input;
        if self.mouse_captured
            && !self.ui.inventory_open
            && input.key_pressed(self.settings.keys.key(Action::Drop))
        {
            self.throw_selected();
        }
        // Le sac à dos : la même touche l'ouvre (le premier sac de l'inventaire) et le referme.
        if input.key_pressed(self.settings.keys.key(Action::Backpack))
            && !self.ui.ctx.egui_wants_keyboard_input()
        {
            if self.ui.inventory_open && matches!(self.ui.screen, ui::Screen::Bag(_)) {
                self.close_screen();
            } else if self.mouse_captured && !self.ui.inventory_open {
                self.open_first_bag();
            }
        }
        if input.key_pressed(self.settings.keys.key(Action::Time)) {
            self.day.cycle_speed();
            log::info!("temps {} ({})", self.day.speed.name(), self.day.clock());
        }
        if input.key_pressed(self.settings.keys.key(Action::Fly)) {
            player::toggle_mode(&mut self.ecs, self.player);
            log::info!("mode {}", player::mode(&self.ecs, self.player).name());
        }
        for (i, key) in HOTBAR_KEYS.iter().enumerate() {
            if input.key_pressed(*key) {
                self.inventory.selected = i;
            }
        }
        if input.key_pressed(KeyCode::F3) {
            self.ui.inspector_open = !self.ui.inspector_open;
        }
        if input.key_pressed(KeyCode::F4) {
            self.hand.visible = !self.hand.visible;
        }
        if input.key_pressed(KeyCode::F1) {
            let mesher = self.streamer().config().mesher.other();
            log::info!("maillage {} : on remaille tout", mesher.name());
            self.streamer_mut().set_mesher(mesher);
        }
        if input.key_pressed(KeyCode::F6) {
            self.toggle_cave_culling();
        }
        if input.key_pressed(KeyCode::F2) {
            self.renderer.frustum_culling = !self.renderer.frustum_culling;
            let state = if self.renderer.frustum_culling {
                "activé"
            } else {
                "désactivé"
            };
            log::info!("frustum culling {state}");
        }
    }

    /// Jette un exemplaire en conservant son usure et son enchantement.
    pub(crate) fn throw_selected(&mut self) {
        let Some(stack) = self.inventory.selected_stack() else {
            return;
        };
        let direction = self.camera.forward();
        let center = self.camera.position + direction * 0.45;
        if self.streamer().world().is_solid(center.floor().as_ivec3()) {
            return;
        }
        self.inventory.consume_selected();
        let seed = self.next_random();
        let entity = drops::spawn(
            &mut self.ecs,
            inventory::ItemStack { count: 1, ..stack },
            center,
            seed,
        );
        if let Some(mut velocity) = self.ecs.get_mut::<components::Velocity>(entity) {
            velocity.0 = direction * 3.5 + Vec3::Y;
        }
        self.hand.punch();
    }

    /// Pose le bloc de la case choisie contre la face visée. Renvoie sa position si c'est fait.
    /// Une flèche sur le bloc visé s'il est orienté (piston, inverseur, répéteur) : une barre
    /// claire dans sa direction, avec un bout plus vif devant.
    pub(crate) fn direction_hint(&self, out: &mut Vec<engine_render::CubeInstance>) {
        let Some(hit) = self.target else { return };
        let streamer = self.streamer();
        let block = streamer.world().block(hit.block_pos);
        let directional = matches!(
            self.registry.def(block).behavior.as_deref(),
            Some(
                "core:piston"
                    | "core:sticky_piston"
                    | "core:inverter"
                    | "core:repeater"
                    | "core:propeller"
            )
        );
        if !directional {
            return;
        }
        let facing = engine_voxel::facing_of(streamer.block_entity(hit.block_pos));
        let center = hit.block_pos.as_vec3() + Vec3::splat(0.5) + hit.normal.as_vec3() * 0.52;
        let along = facing.as_vec3();
        let bar = Vec3::ONE * 0.12 + along.abs() * 0.5;
        out.push(engine_render::CubeInstance::new(
            center,
            bar,
            Vec3::new(1.0, 0.9, 0.3),
        ));
        out.push(engine_render::CubeInstance::new(
            center + along * 0.3,
            Vec3::splat(0.2),
            Vec3::new(1.0, 0.3, 0.1),
        ));
    }

    /// Les potions en cours : la régénération soigne, la résistance éteint le feu.
    pub(crate) fn update_effects(&mut self, dt: f32) {
        let healed = self.effects.update(dt);
        if healed > 0.0 {
            self.heal(healed);
        }
        if self.effects.has(potions::Effect::FireResistance) {
            self.vitals.burning = 0.0;
        }
    }

    /// Rend des points de vie, sans dépasser le maximum.
    pub(crate) fn heal(&mut self, amount: f32) {
        if let Some(mut health) = self.ecs.get_mut::<components::Health>(self.player) {
            health.current = (health.current + amount).min(health.max);
        }
    }

    /// Une fiole vide, face à de l'eau : elle se remplit.
    pub(crate) fn fill_flask(&mut self, flask: engine_voxel::ItemId) {
        let items = self.content.items.clone();
        let Some(full) = items.id("core:water_flask") else {
            return;
        };
        let water = {
            let streamer = self.streamer();
            (1..(self.gear.reach() * 10.0) as i32)
                .map(|k| {
                    (self.camera.position + self.camera.forward() * (k as f32 * 0.1))
                        .floor()
                        .as_ivec3()
                })
                .find(|&p| self.fluids.kind_of(streamer.world().block(p)).is_some())
        };
        let Some(pos) = water else { return };
        if self.inventory.selected_item() != Some(flask) {
            return;
        }
        self.inventory.consume_selected();
        let left = self
            .inventory
            .add_stack(inventory::ItemStack::new(full, 1), &items);
        if left > 0 {
            self.drop_stack(pos, inventory::ItemStack::new(full, left));
        }
        if let Some(audio) = &mut self.audio {
            audio.play(audio::SPLASH);
        }
        self.hand.punch();
    }

    /// La musique et les sons d'ambiance, selon l'endroit où se trouve le joueur.
    pub(crate) fn update_soundscape(&mut self, dt: f32) {
        if self.audio.is_none() {
            return;
        }
        let feet = player::body(&self.ecs, self.player).position;
        let place = {
            let streamer = self.streamer();
            let generator = streamer.generator();
            let (x, z) = (feet.x.floor() as i32, feet.z.floor() as i32);
            let column = generator.column(x, z);
            let key = &generator.biome(column.biome).key;
            let (sky, _) = streamer.world().light(feet.floor().as_ivec3() + IVec3::Y);
            let water = ["ocean", "beach", "river", "swamp"]
                .iter()
                .any(|w| key.contains(w));
            soundscape::Place {
                underground: sky <= 2 && feet.y < column.height as f32 - 8.0,
                night: self.day.is_night(),
                raining: self.weather.amount() > 0.2,
                near_water: water,
                green: !["desert", "tundra", "ocean", "beach", "mountain"]
                    .iter()
                    .any(|w| key.contains(w)),
            }
        };
        let playing = self.music_until.is_some_and(|t| Instant::now() < t);
        let cues = self.soundscape.update(dt, place, playing);
        let Some(audio) = &mut self.audio else { return };
        audio.set_environment(place);
        for cue in cues {
            match cue {
                soundscape::Cue::Music(key) => {
                    // Un morceau introuvable : on réessaiera plus tard.
                    let seconds = audio.play_ambient(key).unwrap_or(0.0);
                    self.soundscape.started(seconds);
                    self.music_until =
                        Some(Instant::now() + std::time::Duration::from_secs_f32(seconds));
                }
                soundscape::Cue::Ambient(key) => {
                    audio.play_ambient(key);
                }
            }
        }
    }

    /// Un clic droit sur un habitant : l'écran d'échange.
    pub(crate) fn trade_with_villager(&mut self) -> bool {
        let block_distance = self.target.map_or(self.gear.reach(), |hit| hit.distance);
        let Some((mob, _)) = mobs::hit_mob(
            &self.ecs,
            self.camera.position,
            self.camera.forward(),
            block_distance,
        ) else {
            return false;
        };
        let key = self.mob_key(mob);
        if !key.as_deref().is_some_and(|k| self.trades.trades(k)) {
            return false;
        }
        self.ui.screen = ui::Screen::Trade(mob);
        self.ui.inventory_open = true;
        true
    }

    /// L'identifiant d'une créature (`core:farmer`).
    pub(crate) fn mob_key(&self, mob: engine_ecs::Entity) -> Option<String> {
        let kind = self.ecs.get::<components::Mob>(mob)?.kind;
        self.ecs
            .resource::<std::sync::Arc<MobRegistry>>()
            .get(kind)
            .map(|d| d.key.clone())
    }

    /// Ferme l'écran ouvert : la grille d'artisanat et la main retournent dans
    /// l'inventaire (ce qui n'y tient pas tombe par terre).
    pub(crate) fn close_screen(&mut self) {
        self.forget_mod_menu();
        self.ui.browser.close();
        self.ui.split = None;
        let loose = self.inventory.put_away(&self.content.items);
        let at = player::body(&self.ecs, self.player)
            .position
            .floor()
            .as_ivec3()
            + IVec3::Y;
        for stack in loose {
            self.drop_stack(at, stack);
        }
        self.ui.inventory_open = false;
        self.ui.screen = ui::Screen::Inventory;
    }

    /// Un clic droit sur un levier, un bouton ou un répéteur.
    pub(crate) fn use_machine(&mut self) -> bool {
        let Some(hit) = self.target else {
            return false;
        };
        let block = self.streamer().world().block(hit.block_pos);
        // Les circuits du jeu, et les machines des mods (leur script décide s'il prend le clic).
        let behavior = self.registry.def(block).behavior.as_deref();
        let clickable = match behavior {
            Some("core:lever" | "core:button" | "core:repeater") => true,
            Some(b) => !b.starts_with("core:"),
            None => false,
        };
        if !clickable {
            return false;
        }
        let (used, actions) = self.mods.use_block(hit.block_pos);
        // Le script peut ouvrir son menu, mais seulement en réponse à ce clic.
        self.ui.allow_mod_menu = true;
        self.apply_machine_actions(actions);
        self.ui.allow_mod_menu = false;
        if used {
            self.hand.punch();
            if let Some(audio) = &mut self.audio {
                audio.play("core:place_hard");
            }
        }
        used
    }

    /// Un aliment en main, clic droit sur un animal qui l'aime : il entre en rut.
    pub(crate) fn feed_mob(&mut self) -> bool {
        let Some(item) = self.inventory.selected_item() else {
            return false;
        };
        let block_distance = self.target.map_or(self.gear.reach(), |hit| hit.distance);
        let Some((mob, _)) = mobs::hit_mob(
            &self.ecs,
            self.camera.position,
            self.camera.forward(),
            block_distance,
        ) else {
            return false;
        };
        let key = self.content.items.get(item).key.clone();
        if !breeding::feed(&mut self.ecs, mob, &key) {
            return false;
        }
        self.inventory.consume_selected();
        self.hand.punch();
        if let Some(audio) = &mut self.audio {
            audio.play("core:place_soft");
        }
        true
    }

    /// L'engrais en main, clic droit sur une culture : elle avance de deux stades.
    pub(crate) fn use_fertilizer(&mut self) -> bool {
        let Some(item) = self.inventory.selected_item() else {
            return false;
        };
        if self.content.items.get(item).key != "core:fertilizer" {
            return false;
        }
        let Some(hit) = self.target else {
            return false;
        };
        let block = self.streamer().world().block(hit.block_pos);
        let Some(next) = engine_voxel::fertilized(&self.registry, block) else {
            return false;
        };
        if !self.edit(hit.block_pos, next) {
            return false;
        }
        self.inventory.consume_selected();
        self.hand.punch();
        if let Some(audio) = &mut self.audio {
            audio.play("core:place_soft");
        }
        true
    }

    /// Une houe en main, clic droit sur de l'herbe ou de la terre dégagée : on laboure.
    /// Renvoie `true` si le sol a été labouré.
    pub(crate) fn use_hoe(&mut self) -> bool {
        if !self
            .held_tool()
            .is_some_and(|t| t.kind == engine_voxel::ToolKind::Hoe)
        {
            return false;
        }
        let Some(hit) = self.target else {
            return false;
        };
        // On ne laboure que par le dessus du bloc.
        if hit.normal != IVec3::Y {
            return false;
        }
        let farmland = engine_voxel::tilled(self.streamer().world(), &self.registry, hit.block_pos);
        let Some(farmland) = farmland else {
            return false;
        };
        if !self.edit(hit.block_pos, farmland) {
            return false;
        }
        self.wear_held();
        self.hand.punch();
        if let Some(audio) = &mut self.audio {
            audio.play("core:place_soft");
        }
        true
    }
}
