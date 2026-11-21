//! La part serveur de `shaping_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `shaping_ui.rs`).

use crate::inventory::{ItemStack, Slot};
use crate::shape_materials;
use engine_voxel::shaped::{self, PieceKind};

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Refait des blocs pleins avec les pièces de `slot` : un bloc, ou tous ceux que la pile permet.
    pub fn rebuild(&mut self, slot: Slot, all: bool) -> Result<u32, String> {
        let Some(stack) = *self.players.me_mut().inventory.slot_mut(slot) else {
            return Err("rien à reconstituer".into());
        };
        let kind = shape_materials::kind_of(self.content.items.get(stack.item))
            .ok_or("ce ne sont pas des pièces")?;
        let material = shape_materials::key_of(stack.damage).ok_or("pièces sans matière")?;
        let item = self
            .content
            .items
            .id(&material)
            .ok_or_else(|| format!("« {material} » n'existe plus"))?;
        let possible = stack.count / kind.per_block();
        if possible == 0 {
            return Err(format!("il faut {} pièces pour un bloc", kind.per_block()));
        }
        let blocks = if all { possible } else { 1 };
        let left = stack.count - blocks * kind.per_block();
        *self.players.me_mut().inventory.slot_mut(slot) = (left > 0).then_some(ItemStack {
            count: left,
            ..stack
        });
        self.give_or_drop(ItemStack::new(item, blocks));
        Ok(blocks)
    }

    /// Taille des blocs de `slot` en pièces de la sorte `kind` : un bloc, ou toute la pile. `Err`
    /// dit pourquoi rien n'est fait.
    pub fn carve(&mut self, slot: Slot, kind: PieceKind, all: bool) -> Result<u32, String> {
        let Some(stack) = *self.players.me_mut().inventory.slot_mut(slot) else {
            return Err("rien à tailler".into());
        };
        let def = self.content.items.get(stack.item);
        let key = def.key.clone();
        let block = def
            .block
            .ok_or_else(|| format!("« {} » n'est pas un bloc", def.name))?;
        if let Some(reason) = shaped::why_not_shapeable(self.registry.def(block)) {
            return Err(format!("« {} » ne se taille pas : {reason}", def.name));
        }
        let piece = self.content.items.id(kind.key()).ok_or("pièce inconnue")?;
        let blocks = if all { stack.count } else { 1 };
        let made = ItemStack {
            item: piece,
            count: blocks * kind.per_block(),
            damage: shape_materials::number_of(&key),
        };
        *self.players.me_mut().inventory.slot_mut(slot) =
            (stack.count > blocks).then_some(ItemStack {
                count: stack.count - blocks,
                ..stack
            });
        self.give_or_drop(made);
        self.sound("core:place_hard");
        Ok(made.count)
    }
}
