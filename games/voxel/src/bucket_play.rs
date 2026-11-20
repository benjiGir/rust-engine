//! Le seau, côté jeu : prendre un fluide, le reverser. (Sorti de `main.rs`, qui approche la limite
//! de taille des fichiers.)

use engine_voxel::ItemId;

use crate::{VoxelGame, audio, inventory};

impl VoxelGame {
    /// Le seau : vide, il prend le fluide de la première cellule visée ; plein, il le
    /// reverse dans la cellule devant le bloc visé. La quantité transférée est celle de
    /// la cellule (jusqu'à un bloc) : elle est gardée dans l'objet (son champ `damage`).
    pub(crate) fn use_bucket(&mut self, item: ItemId) {
        let items = self.content.items.clone();
        let key = items.get(item).key.clone();
        let slot = self.players.me_mut().inventory.selected;
        if key == "core:bucket" {
            // Le long du rayon, la première cellule de fluide.
            let found = {
                let reach = self.players.me().gear.reach();
                let aim = self.aim();
                let streamer = self.streamer();
                (1..(reach * 10.0) as i32)
                    .map(|k| {
                        (aim.eye + aim.forward * (k as f32 * 0.1))
                            .floor()
                            .as_ivec3()
                    })
                    .find(|&p| self.fluids.kind_of(streamer.world().block(p)).is_some())
            };
            let Some(pos) = found else { return };
            let Some((kind, amount)) = self.fluids.take(&mut self.world.borrow_mut(), pos) else {
                return;
            };
            let fluid = self.fluids.kinds()[kind].key.clone();
            let Some(full) = items.id(&format!("{fluid}_bucket")) else {
                return;
            };
            self.players.me_mut().inventory.consume_selected();
            let filled = inventory::ItemStack {
                item: full,
                count: 1,
                damage: amount as u32,
            };
            if self.players.me_mut().inventory.slots[slot].is_none() {
                self.players.me_mut().inventory.slots[slot] = Some(filled);
            } else if self.players.me_mut().inventory.add_stack(filled, &items) > 0 {
                self.drop_stack(pos, filled);
            }
            self.sound(audio::SPLASH);
            self.punch();
        } else {
            let Some(hit) = self.aim().hit else { return };
            let Some(stack) = self.players.me_mut().inventory.selected_stack() else {
                return;
            };
            let fluid_key = key.trim_end_matches("_bucket");
            let Some(kind) = self.fluids.kinds().iter().position(|k| k.key == fluid_key) else {
                return;
            };
            let pos = hit.block_pos + hit.normal;
            let poured =
                self.fluids
                    .pour(&mut self.world.borrow_mut(), pos, kind, stack.damage as i32);
            if poured == 0 {
                return;
            }
            let left = stack.damage as i32 - poured;
            self.players.me_mut().inventory.slots[slot] = Some(if left > 0 {
                inventory::ItemStack {
                    damage: left as u32,
                    ..stack
                }
            } else {
                inventory::ItemStack::new(items.id("core:bucket").unwrap_or(item), 1)
            });
            self.sound(audio::SPLASH);
            self.punch();
        }
    }
}
