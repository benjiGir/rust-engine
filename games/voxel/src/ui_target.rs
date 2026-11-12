//! Identification du bloc ou de la créature regardée, dans le monde et les structures.
use crate::{
    VoxelGame,
    components::{Health, Mob},
    mobs::MobRegistry,
    skin,
    ui_info::{self, Card},
};
use engine_render::egui::{self};

impl VoxelGame {
    pub(crate) fn draw_target_info(&mut self, ctx: &egui::Context) -> f32 {
        if !self.settings.target_info || !self.mouse_captured {
            return 12.0;
        }
        let ship = self.ship_hit();
        let block_distance = ship
            .map(|(_, h)| h.distance)
            .unwrap_or_else(|| self.target.map_or(crate::REACH, |h| h.distance));
        let mob = crate::mobs::hit_mob(
            &self.ecs,
            self.camera.position,
            self.camera.forward(),
            block_distance,
        );
        let mut item = None;
        let fluids = self.streamer().world().fluids().cloned();
        let lines = |caps: Option<&engine_voxel::Capabilities>| {
            caps.map(|caps| ui_info::capabilities(caps, fluids.as_deref()))
                .unwrap_or_default()
        };
        let mut card = if let Some((entity, _)) = mob {
            let Some(kind) = self.ecs.get::<Mob>(entity).map(|m| m.kind) else {
                return 12.0;
            };
            let registry = self.ecs.resource::<std::sync::Arc<MobRegistry>>();
            let Some(def) = registry.get(kind) else {
                return 12.0;
            };
            let mut lines = Vec::new();
            if let Some(h) = self.ecs.get::<Health>(entity) {
                lines.push((
                    format!("Vie : {:.0} / {:.0}", h.current.max(0.0), h.max),
                    skin::RED,
                ));
            }
            Card {
                name: def.name.clone(),
                origin: ui_info::origin(&def.key),
                lines,
            }
        } else {
            let (block, caps) = if let Some((id, hit)) = ship {
                let grids = self.mods.subgrids.borrow();
                let Some(grid) = grids.get(id) else {
                    return 12.0;
                };
                let grid = grid.borrow();
                let world = grid.world();
                let (chunk, [x, y, z]) = engine_voxel::World::split(hit.block_pos);
                (
                    world.block(hit.block_pos),
                    lines(
                        world
                            .chunk(chunk)
                            .and_then(|c| c.block_entity(x, y, z))
                            .and_then(|e| e.caps.as_deref()),
                    ),
                )
            } else if let Some(hit) = self.target {
                let s = self.streamer();
                (
                    s.world().block(hit.block_pos),
                    lines(
                        s.block_entity(hit.block_pos)
                            .and_then(|e| e.caps.as_deref()),
                    ),
                )
            } else {
                return 12.0;
            };
            let def = self.registry.def(block);
            let held = self
                .inventory
                .selected_item()
                .and_then(|id| self.content.items.get(id).tool);
            let mut card = ui_info::block(def, held.as_ref());
            item = self.content.items.of_block(self.registry.base(block));
            card.lines.extend(caps);
            card
        };
        // Une fiche compacte : le nom et l’origine restent visibles, même sur petit écran.
        card.lines.truncate(if ctx.content_rect().height() < 260.0 {
            0
        } else if ctx.content_rect().height() < 400.0 {
            2
        } else {
            4
        });
        let icon = item.map(|item| {
            self.ui.icon(
                item,
                &self.content.items,
                &self.registry,
                &self.block_textures,
            )
        });
        egui::Area::new(egui::Id::unique("target_info"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 40.0).clamp(100.0, 320.0));
                egui::Frame::new()
                    .fill(skin::PANEL)
                    .stroke(egui::Stroke::new(1.0, skin::BORDER))
                    .inner_margin(8)
                    .show(ui, |ui| ui_info::draw(ui, &card, icon));
            })
            .response
            .rect
            .bottom()
    }
}
