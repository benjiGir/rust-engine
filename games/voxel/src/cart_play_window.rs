//! La part fenêtre de `cart_play.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `cart_play.rs`).

use super::*;
#[allow(unused_imports)]
use crate::cart_play::*;
use crate::carts::{Cart, CartCargo, CartConfig, CartSignals};
use crate::key_codes::KeyCodes;
use engine_voxel::{RailKind, rail_at};

impl VoxelGame {
    /// Chaque image : la vitesse maximale des options, et Maj pour descendre.
    pub(crate) fn update_carts(&mut self, ctx: &Context) {
        if let Some(mut config) = self.server.ecs.get_resource_mut::<CartConfig>() {
            config.max_speed = self
                .settings
                .cart_max_speed
                .clamp(1.0, engine_voxel::SPEED_LIMIT);
        }
        let sneak = ctx
            .input
            .key_pressed(self.settings.keys.key(Action::Descend))
            || ctx.input.key_pressed(KeyCode::ShiftRight);
        if sneak && self.client.mouse_captured && !self.client.ui.inventory_open {
            self.send_now(crate::protocol::ClientMsg::Dismount);
        }
    }

    /// Les wagons autour du joueur, pour la commande `carts`.
    pub(crate) fn carts_json(&self) -> String {
        let mut list = Vec::new();
        for entity in carts::all(&self.server.ecs) {
            let (Some(cart), Some(transform)) = (
                self.server.ecs.get::<Cart>(entity),
                self.server.ecs.get::<Transform>(entity),
            ) else {
                continue;
            };
            let items: u32 = self
                .server
                .ecs
                .get::<CartCargo>(entity)
                .map_or(0, |c| c.0.slots.iter().flatten().map(|s| s.count).sum());
            let rail = cart
                .rail_block()
                .map_or("null".to_string(), |b| format!("[{},{},{}]", b.x, b.y, b.z));
            let p = transform.position;
            list.push(format!(
                "{{\"id\":{},\"kind\":\"{}\",\"pos\":[{:.2},{:.2},{:.2}],\"speed\":{:.2},\"on_rail\":{},\"rail\":{},\"rider\":{},\"links\":{:?},\"items\":{},\"burn\":{:.1},\"frozen\":{}}}",
                cart.id,
                cart.kind.name(),
                p.x,
                p.y,
                p.z,
                cart.speed(),
                cart.on_rail(),
                rail,
                cart.rider.is_some(),
                cart.links,
                items,
                cart.burn,
                cart.frozen,
            ));
        }
        format!("{{\"ok\":true,\"carts\":[{}]}}", list.join(","))
    }

    /// Le wagon de ce numéro.
    pub(crate) fn cart_by_id(&self, id: u32) -> Option<Entity> {
        carts::all(&self.server.ecs)
            .into_iter()
            .find(|&e| self.server.ecs.get::<Cart>(e).is_some_and(|c| c.id == id))
    }

    /// Est-ce un rail détecteur allumé ? (pour les essais)
    #[allow(dead_code)]
    pub(crate) fn detector_lit(&self, pos: IVec3) -> bool {
        rail_at(&*self.server.streamer(), pos).is_some_and(|r| r.kind == RailKind::Detector)
            && self
                .server
                .ecs
                .get_resource::<CartSignals>()
                .is_some_and(|s| s.lit.contains(&pos))
    }
}
