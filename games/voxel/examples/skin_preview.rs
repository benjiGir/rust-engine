//! Planche de contrôle du thème et des icônes, sans fenêtre ni sauvegarde de monde.
//! `cargo run -p voxel --example skin_preview -- /tmp/voxel-skin.png`

#[path = "../src/assets.rs"]
#[allow(dead_code)]
mod assets;
mod common;
#[path = "../src/item_art.rs"]
mod item_art;
#[path = "../src/item_models.rs"]
mod item_models;
#[path = "../src/skin.rs"]
#[allow(dead_code)]
mod skin;

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use engine_render::egui::{self, Color32};
use engine_voxel::{BlockRegistry, Shape};

const WIDTH: usize = 1040;
const HEIGHT: usize = 760;

#[cfg(test)]
fn test_registry() -> BlockRegistry {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap()
}

fn main() -> Result<()> {
    let output = std::env::args()
        .nth(1)
        .context("indiquer le chemin du PNG de contrôle")?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
    let textures: Vec<_> = registry
        .textures()
        .iter()
        .map(|key| {
            let path = registry
                .asset_path(key, "textures", "png")
                .context("chemin de texture")?;
            Ok(image::open(path)?.into_rgba8().into_raw())
        })
        .collect::<Result<_>>()?;
    let ctx = egui::Context::default();
    skin::apply(&ctx);
    let items = engine_voxel::ItemRegistry::builtin(&registry)?;
    let mut models = Vec::new();
    for key in [
        "core:grass",
        "core:dirt",
        "example:ruby_slab",
        "core:chest",
        "core:iron_pickaxe",
        "core:iron_sword",
        "core:iron_helmet",
        "core:bucket",
        "core:wheat",
    ] {
        let parts = item_models::build(
            items.id(key).context("objet de la planche")?,
            &items,
            &registry,
        );
        let pixels =
            common::model_image(&parts, &textures, glam::Vec3::new(-0.7, -0.45, -1.0), 1.0);
        models.push(ctx.load_texture(
            format!("model:{key}"),
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &pixels),
            egui::TextureOptions::NEAREST,
        ));
    }
    let blocks = [
        "core:grass",
        "core:stone",
        "core:oak_log",
        "core:oak_planks",
        "core:glass",
        "core:copper_ore",
        "core:furnace",
        "core:chest",
        "mecanique:gear",
    ];
    let mut icons = Vec::new();
    for key in blocks {
        let block = registry.id(key).context("bloc de la planche")?;
        let def = registry.def(block);
        anyhow::ensure!(
            matches!(def.shape, Shape::Cube | Shape::Model),
            "modèle attendu"
        );
        let pixels = item_art::block_icon(&registry, block, &textures);
        icons.push(ctx.load_texture(
            key,
            egui::ColorImage::from_rgba_unmultiplied([32, 32], &pixels),
            egui::TextureOptions::NEAREST,
        ));
    }
    for name in [
        "wooden_pickaxe",
        "stone_axe",
        "copper_shovel",
        "iron_sword",
        "golden_hoe",
        "sapphire_pickaxe",
        "bread",
        "carrot",
        "pear",
        "iron_ingot",
        "copper_ingot",
        "sapphire",
        "apple",
        "cabbage",
        "melon_slice",
        "water_flask",
        "potion_healing",
        "iron_helmet",
    ] {
        let path = root.join(format!("assets/core/items/{name}.png"));
        let pixels = image::open(path)?.into_rgba8().into_raw();
        icons.push(ctx.load_texture(
            name,
            egui::ColorImage::from_rgba_unmultiplied([16, 16], &pixels),
            egui::TextureOptions::NEAREST,
        ));
    }
    let mut atlas = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH as f32, HEIGHT as f32),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let p = ui.ctx().layer_painter(egui::LayerId::background());
            p.rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(20, 28, 31));
            p.text(
                egui::pos2(40.0, 22.0),
                egui::Align2::LEFT_TOP,
                "TERRE / ARDOISE / CUIVRE",
                egui::FontId::monospace(14.0),
                skin::MUTED,
            );
            egui::Window::new("menu")
                .fixed_pos(egui::pos2(40.0, 60.0))
                .frame(egui::Frame::NONE)
                .title_bar(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.set_width(300.0);
                        skin::logo(ui);
                        ui.label(
                            egui::RichText::new("TERRES SAUVAGES · ATELIERS · AVENTURES")
                                .size(10.0)
                                .color(skin::MUTED),
                        );
                        ui.add_space(16.0);
                        for text in ["Jouer", "Nouveau monde", "Options", "Quitter"] {
                            skin::menu_button(ui, text);
                        }
                        ui.add_space(8.0);
                    });
                });
            egui::Window::new("inventaire")
                .fixed_pos(egui::pos2(410.0, 60.0))
                .frame(skin::inventory_panel())
                .title_bar(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    skin::inventory_style(ui);
                    ui.heading("Inventaire");
                    ui.label(
                        egui::RichText::new("Matières · outils · provisions").color(skin::INK),
                    );
                    ui.add_space(12.0);
                    for row in 0..3 {
                        ui.horizontal(|ui| {
                            for col in 0..9 {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(48.0, 48.0),
                                    egui::Sense::hover(),
                                );
                                skin::slot(ui.painter(), rect, row == 1 && col == 3, false);
                                ui.painter().image(
                                    icons[row * 9 + col].id(),
                                    skin::icon_rect(rect),
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                        });
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label("Quantité");
                        let mut value = 16;
                        ui.add(egui::Slider::new(&mut value, 1..=64));
                    });
                    ui.horizontal(|ui| {
                        let mut checked = true;
                        ui.checkbox(&mut checked, "Afficher les recettes");
                    });
                    ui.add(
                        egui::ProgressBar::new(0.62)
                            .corner_radius(0)
                            .fill(skin::COPPER)
                            .text("Fonte en cours"),
                    );
                });
            egui::Area::new(egui::Id::unique("palette"))
                .fixed_pos(egui::pos2(42.0, 590.0))
                .show(ui.ctx(), |ui| {
                    ui.label(
                        egui::RichText::new("PALETTE COMMUNE")
                            .color(skin::MUTED)
                            .size(11.0),
                    );
                    ui.horizontal(|ui| {
                        for color in [
                            skin::PANEL,
                            skin::BORDER,
                            skin::COPPER,
                            skin::SAGE,
                            skin::GOLD,
                            skin::TEXT,
                        ] {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(41.0, 28.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 0, color);
                        }
                    });
                });
            egui::Area::new(egui::Id::unique("hud"))
                .fixed_pos(egui::pos2(290.0, 650.0))
                .show(ui.ctx(), |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(444.0, 18.0), egui::Sense::hover());
                    for i in 0..10 {
                        for (offset, color, seed) in
                            [(0.0, skin::RED, false), (284.0, skin::GOLD, true)]
                        {
                            let cell = egui::Rect::from_min_size(
                                rect.min + egui::vec2(offset + i as f32 * 16.0, 0.0),
                                egui::vec2(13.0, 13.0),
                            );
                            skin::meter(
                                ui.painter(),
                                cell,
                                if i < 8 {
                                    1.0
                                } else if i == 8 {
                                    0.5
                                } else {
                                    0.0
                                },
                                color,
                                seed,
                            );
                        }
                    }
                    ui.add(
                        egui::ProgressBar::new(0.45)
                            .corner_radius(0)
                            .fill(skin::SAGE)
                            .desired_width(444.0)
                            .desired_height(5.0),
                    );
                    ui.horizontal(|ui| {
                        for (i, icon) in icons.iter().take(9).enumerate() {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::hover());
                            skin::slot(ui.painter(), rect, i == 0, false);
                            ui.painter().image(
                                icon.id(),
                                skin::icon_rect(rect),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                    });
                });
            egui::Window::new("modèles au sol")
                .fixed_pos(egui::pos2(40.0, 445.0))
                .frame(skin::panel())
                .title_bar(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("OBJETS AU SOL · VOLUMES ET TEXTURES");
                    ui.horizontal(|ui| {
                        for icon in &models {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(88.0, 80.0), egui::Sense::hover());
                            ui.painter().image(
                                icon.id(),
                                rect.shrink2(egui::vec2(12.0, 8.0)),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                    });
                });
        });
        for (&id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                let egui::ImageData::Color(image) = &delta.image;
                if let Some([x, y]) = delta.pos {
                    let stored: &mut egui::ColorImage =
                        atlas.get_mut(&id).context("atlas partiel sans image")?;
                    for row in 0..image.height() {
                        let dest = (y + row) * stored.width() + x;
                        stored.pixels[dest..dest + image.width()].copy_from_slice(
                            &image.pixels[row * image.width()..(row + 1) * image.width()],
                        );
                    }
                } else {
                    atlas.insert(id, image.as_ref().clone());
                }
            }
        }
        output.textures_delta.clear();
        shapes = output.shapes;
    }
    let primitives = ctx.tessellate(shapes, 1.0);
    let rgba = common::rasterize(&primitives, &atlas, WIDTH, HEIGHT);
    image::save_buffer(
        &output,
        &rgba,
        WIDTH as u32,
        HEIGHT as u32,
        image::ColorType::Rgba8,
    )?;
    println!("{output}");
    Ok(())
}
