//! Contrôle des PNG originaux et de leur répétition, sans GPU.
#[allow(dead_code)]
mod common;
use anyhow::{Context as _, Result};
use engine_render::egui::{self, Color32, TextureId};
use std::{collections::HashMap, path::Path};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "asset_review <racine> <mods|terrain> <sortie.png>"
    );
    let root = Path::new(&args[0]);
    let mut paths = Vec::new();
    if args[1] == "mods" {
        for namespace in [
            "example",
            "mecanique",
            "donjons",
            "entrepot",
            "forge",
            "magie",
        ] {
            for kind in ["textures", "items"] {
                let dir = root.join("mods").join(namespace).join(kind);
                if dir.exists() {
                    for entry in std::fs::read_dir(dir)? {
                        let p = entry?.path();
                        if p.extension().is_some_and(|e| e == "png") {
                            paths.push(p);
                        }
                    }
                }
            }
        }
        paths.sort();
    } else {
        for name in [
            "grass_top",
            "dirt",
            "stone",
            "sand",
            "snow",
            "ice",
            "coal_ore",
            "iron_ore",
            "copper_ore",
            "gold_ore",
            "oak_log",
            "oak_planks",
            "oak_leaves",
            "birch_log",
            "birch_planks",
            "birch_leaves",
            "spruce_log",
            "spruce_planks",
            "spruce_leaves",
            "acacia_log",
            "acacia_planks",
            "acacia_leaves",
            "jungle_log",
            "jungle_planks",
            "jungle_leaves",
            "cactus_side",
            "farmland_dry",
            "farmland_wet",
        ] {
            paths.push(root.join(format!("assets/core/textures/{name}.png")));
        }
    }
    let ctx = egui::Context::default();
    let images = paths
        .iter()
        .map(|p| {
            let rgba = image::open(p)?.into_rgba8();
            Ok(ctx.load_texture(
                p.to_string_lossy(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                ),
                egui::TextureOptions::NEAREST,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let width = 1200;
    let height = 56 + paths.len().div_ceil(4) * 158;
    let mut atlas: HashMap<TextureId, egui::ColorImage> = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width as f32, height as f32),
                )),
                ..Default::default()
            },
            |ui| {
                let p = ui.painter();
                p.rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(25, 34, 37));
                p.text(
                    egui::pos2(16.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} · PNG et répétition 3 × 3", args[1]),
                    egui::FontId::proportional(18.0),
                    Color32::from_rgb(235, 225, 205),
                );
                for (i, (path, tex)) in paths.iter().zip(&images).enumerate() {
                    let x = (i % 4) as f32 * 300.0 + 12.0;
                    let y = (i / 4) as f32 * 158.0 + 56.0;
                    let label = if args[1] == "mods" {
                        format!(
                            "{}:{}",
                            path.parent()
                                .unwrap()
                                .parent()
                                .unwrap()
                                .file_name()
                                .unwrap()
                                .to_string_lossy(),
                            path.file_stem().unwrap().to_string_lossy()
                        )
                    } else {
                        path.file_stem().unwrap().to_string_lossy().to_string()
                    };
                    p.text(
                        egui::pos2(x, y),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::monospace(13.0),
                        Color32::WHITE,
                    );
                    {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(x, y + 24.0),
                            egui::vec2(96.0, 96.0),
                        );
                        p.rect_filled(rect, 0, Color32::from_rgb(46, 54, 55));
                        p.image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    for ty in 0..3 {
                        for tx in 0..3 {
                            p.image(
                                tex.id(),
                                egui::Rect::from_min_size(
                                    egui::pos2(
                                        x + 116.0 + tx as f32 * 32.0,
                                        y + 24.0 + ty as f32 * 32.0,
                                    ),
                                    egui::vec2(32.0, 32.0),
                                ),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                    }
                }
            },
        );
        for (&id, deltas) in &output.textures_delta.set {
            for d in deltas {
                let egui::ImageData::Color(im) = &d.image;
                if let Some([x, y]) = d.pos {
                    let a = atlas.get_mut(&id).context("atlas")?;
                    for r in 0..im.height() {
                        let start = (y + r) * a.width() + x;
                        a.pixels[start..start + im.width()]
                            .copy_from_slice(&im.pixels[r * im.width()..(r + 1) * im.width()]);
                    }
                } else {
                    atlas.insert(id, im.as_ref().clone());
                }
            }
        }
        output.textures_delta.clear();
        shapes = output.shapes;
    }
    let rgba = common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, width, height);
    image::save_buffer(
        &args[2],
        &rgba,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
