//! Les interactions du joueur avec le monde : levier, bouton, houe, engrais, animaux et
//! habitants, fioles, et ce qui s'y rattache (potions en cours, musique, flèche d'orientation).

use super::*;

impl VoxelGame {
    /// Pose le bloc de la case choisie contre la face visée. Renvoie sa position si c'est fait.
    /// Une flèche sur le bloc visé s'il est orienté (piston, inverseur, répéteur) : une barre
    /// claire dans sa direction, avec un bout plus vif devant.
    pub(crate) fn direction_hint(&self, out: &mut Vec<engine_render::CubeInstance>) {
        let Some(hit) = self.client.target else {
            return;
        };
        let streamer = self.server.streamer();
        let block = streamer.world().block(hit.block_pos);
        let directional = matches!(
            self.server.registry.def(block).behavior.as_deref(),
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
        let healed = self.server.players.me_mut().effects.update(dt);
        if healed > 0.0 {
            self.server.heal(healed);
        }
        if self
            .server
            .players
            .me_mut()
            .effects
            .has(potions::Effect::FireResistance)
        {
            self.server.players.me_mut().vitals.burning = 0.0;
        }
    }

    /// La musique et les sons d'ambiance, selon l'endroit où se trouve le joueur.
    pub(crate) fn update_soundscape(&mut self, dt: f32) {
        if self.client.audio.is_none() {
            return;
        }
        let feet = player::body(&self.server.ecs, self.server.players.entity()).position;
        let place = {
            let streamer = self.server.streamer();
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
                night: self.server.day.is_night(),
                raining: self.server.weather.amount() > 0.2,
                near_water: water,
                green: !["desert", "tundra", "ocean", "beach", "mountain"]
                    .iter()
                    .any(|w| key.contains(w)),
            }
        };
        let playing = self.client.music_until.is_some_and(|t| Instant::now() < t);
        let cues = self.client.soundscape.update(dt, place, playing);
        let Some(audio) = &mut self.client.audio else {
            return;
        };
        audio.set_environment(place);
        for cue in cues {
            match cue {
                soundscape::Cue::Music(key) => {
                    // Un morceau introuvable : on réessaiera plus tard.
                    let seconds = audio.play_ambient(key).unwrap_or(0.0);
                    self.client.soundscape.started(seconds);
                    self.client.music_until =
                        Some(Instant::now() + std::time::Duration::from_secs_f32(seconds));
                }
                soundscape::Cue::Ambient(key) => {
                    audio.play_ambient(key);
                }
            }
        }
    }

    /// Ferme l'écran ouvert : la grille d'artisanat et la main retournent dans
    /// l'inventaire (ce qui n'y tient pas tombe par terre).
    pub(crate) fn close_screen(&mut self) {
        // Le serveur prévient le mod (`CloseMenu`, voir `close_menu`).
        self.client.ui.mod_menu = None;
        self.client.ui.browser.close();
        self.client.ui.split = None;
        // Le simulateur range la main et la grille d'artisanat (voir `intents.rs`).
        self.send_now(crate::protocol::ClientMsg::CloseMenu);
        self.client.ui.inventory_open = false;
        self.client.ui.screen = ui::Screen::Inventory;
    }
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un clic droit sur un levier, un bouton ou un répéteur.
    pub(crate) fn use_machine(&mut self) -> bool {
        let Some(hit) = self.aim().hit else {
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
        self.allow_mod_menu = true;
        self.apply_machine_actions(actions);
        self.allow_mod_menu = false;
        if used {
            self.punch();
            self.sound("core:place_hard");
        }
        used
    }

    /// Une fiole vide, face à de l'eau : elle se remplit.
    pub(crate) fn fill_flask(&mut self, flask: engine_voxel::ItemId) {
        let items = self.content.items.clone();
        let Some(full) = items.id("core:water_flask") else {
            return;
        };
        let aim = self.aim();
        let water = {
            let streamer = self.streamer();
            (1..(self.players.me().gear.reach() * 10.0) as i32)
                .map(|k| {
                    (aim.eye + aim.forward * (k as f32 * 0.1))
                        .floor()
                        .as_ivec3()
                })
                .find(|&p| self.fluids.kind_of(streamer.world().block(p)).is_some())
        };
        let Some(pos) = water else { return };
        if self.players.me_mut().inventory.selected_item() != Some(flask) {
            return;
        }
        self.players.me_mut().inventory.consume_selected();
        let left = self
            .players
            .me_mut()
            .inventory
            .add_stack(inventory::ItemStack::new(full, 1), &items);
        if left > 0 {
            self.drop_stack(pos, inventory::ItemStack::new(full, left));
        }
        self.sound(audio::SPLASH);
        self.punch();
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
        let Some(hit) = self.aim().hit else {
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
        self.punch();
        self.sound("core:place_soft");
        true
    }

    /// L'engrais en main, clic droit sur une culture : elle avance de deux stades.
    pub(crate) fn use_fertilizer(&mut self) -> bool {
        let Some(item) = self.players.me_mut().inventory.selected_item() else {
            return false;
        };
        if self.content.items.get(item).key != "core:fertilizer" {
            return false;
        }
        let Some(hit) = self.aim().hit else {
            return false;
        };
        let block = self.streamer().world().block(hit.block_pos);
        let Some(next) = engine_voxel::fertilized(&self.registry, block) else {
            return false;
        };
        if !self.edit(hit.block_pos, next) {
            return false;
        }
        self.players.me_mut().inventory.consume_selected();
        self.punch();
        self.sound("core:place_soft");
        true
    }

    /// Un clic droit sur un habitant : l'écran d'échange.
    pub(crate) fn trade_with_villager(&mut self) -> bool {
        let Some(mob) = self.aim().mob else {
            return false;
        };
        let key = self.mob_key(mob);
        if !key.as_deref().is_some_and(|k| self.trades.trades(k)) {
            return false;
        }
        let Some(id) = self.ecs.get::<crate::entities::NetId>(mob).map(|id| *id) else {
            return false;
        };
        self.open_screen(ui::Screen::Trade(id));
        true
    }

    /// Maj + clic droit avec un conduit : poser contre une machine sans ouvrir son menu.
    pub(crate) fn place_conduit_first(&mut self) -> bool {
        // Un sac à dos en main : Maj + clic droit le pose au sol (voir `backpack_block.rs`).
        if self.place_backpack() {
            return true;
        }
        let Some(block) = self
            .players
            .me_mut()
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

    /// L'identifiant d'une créature (`core:farmer`).
    pub(crate) fn mob_key(&self, mob: engine_ecs::Entity) -> Option<String> {
        let kind = self.ecs.get::<components::Mob>(mob)?.kind;
        self.ecs
            .resource::<std::sync::Arc<MobRegistry>>()
            .get(kind)
            .map(|d| d.key.clone())
    }

    /// Un aliment en main, clic droit sur un animal qui l'aime : il entre en rut.
    pub(crate) fn feed_mob(&mut self) -> bool {
        let Some(item) = self.players.me_mut().inventory.selected_item() else {
            return false;
        };
        let Some(mob) = self.aim().mob else {
            return false;
        };
        let key = self.content.items.get(item).key.clone();
        if !breeding::feed(&mut self.ecs, mob, &key) {
            return false;
        }
        self.players.me_mut().inventory.consume_selected();
        self.punch();
        self.sound("core:place_soft");
        true
    }

    /// Rend des points de vie, sans dépasser le maximum.
    pub(crate) fn heal(&mut self, amount: f32) {
        if let Some(mut health) = self
            .ecs
            .get_mut::<components::Health>(self.players.entity())
        {
            health.current = (health.current + amount).min(health.max);
        }
    }
}
