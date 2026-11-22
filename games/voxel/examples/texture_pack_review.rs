//! Comparaison du pack 16/32, raccords et modèles réels, sans fenêtre ni sauvegarde.
#[path = "../src/render"]
mod render {
    pub mod item_models;
}
#[path = "../src/ui"]
mod ui {
    #[allow(dead_code)]
    pub mod skin;
}
#[path = "../src/assets.rs"]
#[allow(dead_code)]
mod assets;
#[allow(dead_code)]
mod common;
use anyhow::{Context, Result};
use engine_render::egui::{self, Color32};
use engine_voxel::{BlockRegistry, ItemRegistry};
use std::{collections::HashMap, path::Path};
#[cfg(test)]
fn test_registry() -> BlockRegistry {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap()
}
fn main() -> Result<()> {
    let output = std::env::args().nth(1).context("sortie.png")?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ctx = egui::Context::default();
    crate::ui::skin::apply(&ctx);
    let mut boards = Vec::new();
    for packed in [false, true] {
        let mut registry = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
        if packed {
            registry
                .set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)?;
        }
        let textures = assets::load_textures(&registry);
        let mut tiles = Vec::new();
        for key in [
            "grass_top",
            "dirt",
            "stone",
            "oak_planks",
            "copper_ore",
            "oak_log",
            "furnace_front",
            "oak_leaves",
        ] {
            let pixels = assets::load_png(
                &registry
                    .asset_path(&format!("core:{key}"), "textures", "png")
                    .unwrap(),
            )?;
            let side = engine_render::texture_side(&pixels).unwrap() as usize;
            tiles.push(ctx.load_texture(
                format!("{packed}:{key}"),
                egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        let items = ItemRegistry::builtin(&registry)?;
        let mut models = Vec::new();
        for key in [
            "core:grass",
            "core:chest",
            "core:furnace",
            "core:bed",
            "core:iron_pickaxe",
            "core:iron_sword",
            "core:water_flask",
            "mecanique:gear",
        ] {
            let parts = crate::render::item_models::build(
                items.id(key).context("objet")?,
                &items,
                &registry,
            );
            let image =
                common::model_image(&parts, &textures, glam::Vec3::new(-0.7, -0.45, -1.0), 1.0);
            models.push(ctx.load_texture(
                format!("{packed}:model:{key}"),
                egui::ColorImage::from_rgba_unmultiplied([64, 64], &image),
                egui::TextureOptions::NEAREST,
            ));
        }
        boards.push((tiles, models));
    }
    let (w, h) = (1280usize, 940usize);
    let mut atlas = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..4 {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w as f32, h as f32),
                )),
                ..Default::default()
            },
            |ui| {
                ui.painter()
                    .rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(20, 29, 31));
                for (column, (tiles, models)) in boards.iter().enumerate() {
                    egui::Area::new(egui::Id::unique(("pack_board", column)))
                        .fixed_pos(egui::pos2(28.0 + column as f32 * 632.0, 22.0))
                        .show(ui.ctx(), |ui| {
                            ui.set_width(590.0);
                            ui.heading(if column == 0 {
                                "CLASSIQUE · 16 × 16"
                            } else {
                                "TERRE, ARDOISE ET CUIVRE · 32 × 32"
                            });
                            ui.add_space(12.0);
                            for row in 0..2 {
                                ui.horizontal(|ui| {
                                    for (i, key) in [
                                        "Herbe", "Terre", "Pierre", "Planches", "Cuivre", "Écorce",
                                        "Four", "Feuilles",
                                    ]
                                    .iter()
                                    .enumerate()
                                    .skip(row * 4)
                                    .take(4)
                                    {
                                        ui.vertical(|ui| {
                                            ui.label(*key);
                                            ui.add(egui::Image::new((
                                                tiles[i].id(),
                                                egui::Vec2::splat(128.0),
                                            )));
                                        });
                                    }
                                });
                            }
                            ui.add_space(16.0);
                            ui.label("RACCORDS · 4 × 4 BLOCS");
                            ui.horizontal(|ui| {
                                for index in [0, 1, 3] {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(176.0),
                                        egui::Sense::hover(),
                                    );
                                    for y in 0..4 {
                                        for x in 0..4 {
                                            ui.painter().image(
                                                tiles[index].id(),
                                                egui::Rect::from_min_size(
                                                    rect.min
                                                        + egui::vec2(
                                                            x as f32 * 44.0,
                                                            y as f32 * 44.0,
                                                        ),
                                                    egui::Vec2::splat(44.0),
                                                ),
                                                egui::Rect::from_min_max(
                                                    egui::Pos2::ZERO,
                                                    egui::pos2(1.0, 1.0),
                                                ),
                                                Color32::WHITE,
                                            );
                                        }
                                    }
                                }
                            });
                            ui.add_space(16.0);
                            ui.label("BLOCS ET OBJETS · MÊMES FORMES ET DIMENSIONS");
                            for row in 0..2 {
                                ui.horizontal(|ui| {
                                    for model in models.iter().skip(row * 4).take(4) {
                                        ui.add(egui::Image::new((
                                            model.id(),
                                            egui::Vec2::splat(120.0),
                                        )));
                                    }
                                });
                            }
                        });
                }
            },
        );
        for (&id, deltas) in &out.textures_delta.set {
            for d in deltas {
                let egui::ImageData::Color(im) = &d.image;
                if let Some([x, y]) = d.pos {
                    let a: &mut egui::ColorImage = atlas.get_mut(&id).context("atlas")?;
                    for row in 0..im.height() {
                        let start = (y + row) * a.width() + x;
                        a.pixels[start..start + im.width()]
                            .copy_from_slice(&im.pixels[row * im.width()..(row + 1) * im.width()]);
                    }
                } else {
                    atlas.insert(id, im.as_ref().clone());
                }
            }
        }
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    image::save_buffer(
        &output,
        &common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, w, h),
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
