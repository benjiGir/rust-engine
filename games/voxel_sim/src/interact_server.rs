//! La part serveur de `interact.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `interact.rs`).

use crate::mobs::MobRegistry;
use crate::{breeding, components, inventory};
use glam::IVec3;

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un clic droit sur un levier, un bouton ou un répéteur.
    pub fn use_machine(&mut self) -> bool {
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
    pub fn fill_flask(&mut self, flask: engine_voxel::ItemId) {
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
        self.sound(crate::sound_ids::SPLASH);
        self.punch();
    }

    /// Une houe en main, clic droit sur de l'herbe ou de la terre dégagée : on laboure.
    /// Renvoie `true` si le sol a été labouré.
    pub fn use_hoe(&mut self) -> bool {
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
    pub fn use_fertilizer(&mut self) -> bool {
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
    pub fn trade_with_villager(&mut self) -> bool {
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
        self.open_screen(crate::screens::Screen::Trade(id));
        true
    }

    /// Maj + clic droit avec un conduit : poser contre une machine sans ouvrir son menu.
    pub fn place_conduit_first(&mut self) -> bool {
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
    pub fn mob_key(&self, mob: engine_ecs::Entity) -> Option<String> {
        let kind = self.ecs.get::<components::Mob>(mob)?.kind;
        self.ecs
            .resource::<std::sync::Arc<MobRegistry>>()
            .get(kind)
            .map(|d| d.key.clone())
    }

    /// Un aliment en main, clic droit sur un animal qui l'aime : il entre en rut.
    pub fn feed_mob(&mut self) -> bool {
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
    pub fn heal(&mut self, amount: f32) {
        if let Some(mut health) = self
            .ecs
            .get_mut::<components::Health>(self.players.entity())
        {
            health.current = (health.current + amount).min(health.max);
        }
    }
}
