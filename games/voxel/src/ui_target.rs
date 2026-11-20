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
            .unwrap_or_else(|| self.target.map_or(self.gear.reach(), |h| h.distance));
        let mob = crate::mobs::hit_mob(
            &self.ecs,
            self.camera.position,
            self.camera.forward(),
            block_distance,
        );
        let mut item = None;
        let mut preview = None;
        let fluids = self.streamer().world().fluids().cloned();
        let lines = |block: engine_voxel::BlockId, caps: Option<&engine_voxel::Capabilities>| {
            let main = engine_voxel::layout_of(&self.registry, block).map(|l| l.main);
            caps.map(|caps| ui_info::capabilities(caps, fluids.as_deref(), main))
                .unwrap_or_default()
        };
        // L'aperçu du contenu d'un bloc à inventaire (voir `chest_preview.rs`).
        let want_preview = self.settings.chest_preview;
        let summary = |block: engine_voxel::BlockId, caps: Option<&engine_voxel::Capabilities>| {
            if !want_preview || engine_voxel::is_controller(&self.registry, block) {
                return None;
            }
            let inventory = caps?.inventory.as_ref()?;
            let main = engine_voxel::layout_of(&self.registry, block)
                .map_or(inventory.slots.len(), |l| l.main);
            Some(crate::chest_preview::summarize(&inventory.slots, main))
        };
        let cart = if mob.is_none() {
            self.aimed_cart().filter(|&c| {
                self.ecs
                    .get::<crate::carts::Cart>(c)
                    .is_some_and(|cart| cart.kind.slots() > 1)
            })
        } else {
            None
        };
        let mut card = if let Some(cart) = cart {
            let kind = self.ecs.get::<crate::carts::Cart>(cart).map(|c| c.kind);
            if want_preview {
                preview = self
                    .ecs
                    .get::<crate::carts::CartCargo>(cart)
                    .map(|c| crate::chest_preview::summarize(&c.0.slots, c.0.slots.len()));
            }
            Card {
                name_color: None,
                name: kind.map_or("Wagon", |k| k.title()).to_string(),
                origin: ui_info::origin("core:cart"),
                lines: Vec::new(),
            }
        } else if let Some((entity, _)) = mob {
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
            // Une élite ou un envahisseur porte son nom composé et sa teinte.
            let elite = self
                .ecs
                .get::<crate::elites::Elite>(entity)
                .map(|e| e.clone());
            if let Some(elite) = &elite {
                lines.push((
                    format!("{} augmentation(s)", elite.augmentations.len()),
                    skin::GOLD,
                ));
            }
            Card {
                name_color: elite.as_ref().map(|e| {
                    let c = e.tint;
                    egui::Color32::from_rgb(
                        (c.x * 255.0) as u8,
                        (c.y * 255.0) as u8,
                        (c.z * 255.0) as u8,
                    )
                }),
                name: elite.map_or_else(|| def.name.clone(), |e| e.name),
                origin: ui_info::origin(&def.key),
                lines,
            }
        } else {
            let (block, caps, found) = if let Some((id, hit)) = ship {
                let grids = self.mods.subgrids.borrow();
                let Some(grid) = grids.get(id) else {
                    return 12.0;
                };
                let grid = grid.borrow();
                let world = grid.world();
                let (chunk, [x, y, z]) = engine_voxel::World::split(hit.block_pos);
                let block = world.block(hit.block_pos);
                let caps = world
                    .chunk(chunk)
                    .and_then(|c| c.block_entity(x, y, z))
                    .and_then(|e| e.caps.as_deref());
                (block, lines(block, caps), summary(block, caps))
            } else if let Some(hit) = self.target {
                let s = self.streamer();
                // La moitié droite d'un grand coffre montre l'inventaire de la gauche.
                let pos = engine_voxel::double_chest::storage_pos(&*s, hit.block_pos);
                let block = s.world().block(pos);
                let caps = s.block_entity(pos).and_then(|e| e.caps.as_deref());
                (block, lines(block, caps), summary(block, caps))
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
            preview = found;
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
        let preview_icons: Vec<Option<egui::TextureId>> = preview
            .as_ref()
            .map(|p| {
                p.kinds
                    .iter()
                    .map(|(key, _)| {
                        let id = self.content.items.id(key)?;
                        Some(self.ui.icon(
                            id,
                            &self.content.items,
                            &self.registry,
                            &self.block_textures,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        egui::Area::new(egui::Id::unique("target_info"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 40.0).clamp(100.0, 320.0));
                egui::Frame::new()
                    .fill(skin::PANEL)
                    .stroke(egui::Stroke::new(1.0, skin::BORDER))
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui_info::draw(ui, &card, icon);
                        if let Some(preview) = &preview {
                            ui.add_space(4.0);
                            crate::chest_preview::draw(ui, preview, &preview_icons);
                        }
                    });
            })
            .response
            .rect
            .bottom()
    }
}
