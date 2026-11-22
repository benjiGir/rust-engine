//! La part serveur de `adventure/forge_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du
//! plan multijoueur ; la fenêtre reste dans `adventure/forge_ui.rs`).

use crate::adventure::forge_tables::{self, Purse};
use crate::player::inventory::ItemStack;
use engine_voxel::Rng64;

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un objet qui a bougé de place : un message qui dit quoi.
    fn give_back(&mut self, stack: ItemStack) {
        let left = self.collect_stack(stack);
        if left > 0 {
            let at = crate::player::body(&self.ecs, self.players.entity())
                .position
                .floor()
                .as_ivec3();
            self.drop_stack(
                at,
                ItemStack {
                    count: left,
                    ..stack
                },
            );
        }
    }

    /// `Salvage` : le serveur détruit l'objet tenu à la souris et rend ses matériaux.
    pub fn salvage_cursor(&mut self) {
        let data = crate::adventure::data::current();
        let items = self.content.items.clone();
        let Some(held) = self.players.me().inventory.cursor else {
            return;
        };
        if !forge_tables::can_salvage(held.damage) {
            return;
        }
        let mut rng = Rng64::new(self.next_random());
        let Some(got) = forge_tables::salvage(&data, held.damage, &mut rng) else {
            return;
        };
        self.players.me_mut().inventory.cursor = None;
        let shard_name = items
            .id(&got.shard)
            .map_or(got.shard.clone(), |id| items.get(id).name.clone());
        let mut stacks = Vec::new();
        if let Some(id) = items.id(&got.shard) {
            stacks.push(ItemStack::new(id, got.shards));
        }
        if got.dust > 0
            && let Some(id) = forge_tables::dust_item(&items)
        {
            stacks.push(ItemStack::new(id, got.dust));
        }
        for key in &got.gems {
            if let Some(id) = items.id(key) {
                stacks.push(ItemStack::new(id, 1));
            }
        }
        for stack in stacks {
            self.give_back(stack);
        }
        self.toast(&format!("Récupéré : {} × {shard_name}", got.shards));
        self.sound("core:break_glass");
    }

    /// `ReforgeChoose` : le serveur refait les tirages de l'objet tenu, encaisse le prix et
    /// applique celui-ci.
    pub fn reforge_choose(&mut self, index: usize) {
        let data = crate::adventure::data::current();
        let items = self.content.items.clone();
        let Some(held) = self.players.me().inventory.cursor else {
            return;
        };
        let def = items.get(held.item);
        if !forge_tables::can_reforge(def, held.damage) {
            return;
        }
        let rarity = self.players.me().adventure.forge_rarity;
        let Some(cost) = forge_tables::reforge_cost(&data, rarity) else {
            return;
        };
        let Some(shard_id) = items.id(&cost.shard) else {
            return;
        };
        let dust = forge_tables::dust_item(&items);
        let me = self.players.me();
        let purse = Purse {
            shards: me.inventory.count(shard_id),
            dust: dust.map_or(0, |id| me.inventory.count(id)),
            levels: me.experience.level().0,
        };
        if !cost.affordable(purse) {
            return;
        }
        let draws = forge_tables::draws(
            &data,
            def,
            rarity,
            me.adventure.forge_seed ^ u64::from(held.item.0),
        );
        let Some(draw) = draws.get(index).cloned() else {
            return;
        };
        if !self.players.me_mut().experience.spend_levels(cost.levels) {
            return;
        }
        let me = self.players.me_mut();
        me.inventory.remove(shard_id, cost.shards);
        if let Some(dust) = dust {
            me.inventory.remove(dust, cost.dust);
        }
        let name = draw.name.clone();
        if let Some(stack) = me.inventory.cursor.as_mut() {
            forge_tables::apply_draw(&mut stack.damage, draw);
        }
        me.adventure.forge_seed = me.adventure.forge_seed.wrapping_add(1);
        self.toast(name.as_deref().unwrap_or("Objet réforgé"));
        self.sound("core:enchant");
    }
}
