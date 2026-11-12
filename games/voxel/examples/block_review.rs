//! Planche CPU avant/après : formes réelles, matériaux et éclairages de contrôle.
//! block_review <racine des assets> <lot> <sortie.png> [audit.tsv]
mod common;
use anyhow::{Context as _, Result};
use engine_render::{
    CubeInstance,
    egui::{self, Color32, TextureId},
};
use engine_voxel::{BlockBox, BlockRegistry, Shape};
use glam::Vec3;
use std::{collections::HashMap, path::Path};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 3,
        "block_review <racine> <lot> <sortie.png> [audit.tsv]"
    );
    let root = Path::new(&args[0]);
    let registry = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
    if let Some(path) = args.get(3) {
        let mut audit = String::new();
        for (id, def) in registry.iter() {
            let mut normalized = def.clone();
            normalized.color = [0.0; 3];
            normalized.face_textures = [0; 3];
            normalized.boxes.clear();
            normalized.connections = None;
            normalized.shape = Shape::Cube;
            normalized.transparent = true;
            normalized.source.clear();
            let mut hash = 0xcbf29ce484222325u64;
            for b in format!("{normalized:?}").bytes() {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
            }
            audit.push_str(&format!("{}\t{}\t{hash:016x}\n", id.0, def.key));
        }
        std::fs::write(path, audit)?;
    }
    let keys: Vec<_> = registry
        .iter()
        .filter(|(_, d)| {
            if d.base.is_some() {
                return false;
            }
            match args[1].as_str() {
                "conteneurs" => [
                    "core:chest",
                    "core:furnace",
                    "core:workbench",
                    "core:bed",
                    "core:enchanting_table",
                    "core:brewing_stand",
                    "entrepot:vault",
                    "forge:anvil",
                    "forge:alloy_furnace",
                    "magie:rune_table",
                ]
                .contains(&d.key.as_str()),
                "machines" => {
                    (d.behavior.is_some()
                        || d.conduit.is_some()
                        || [
                            "core:piston_head",
                            "core:propeller",
                            "core:helm",
                            "core:balloon",
                            "mecanique:gear",
                            "mecanique:belt",
                        ]
                        .contains(&d.key.as_str()))
                        && ![
                            "core:furnace",
                            "core:chest",
                            "core:brewing_stand",
                            "core:crop",
                            "core:farmland",
                        ]
                        .contains(&d.behavior.as_deref().unwrap_or(""))
                        && d.shape != Shape::Cross
                        && d.key != "core:grass"
                        && !d.key.ends_with("_leaves")
                }
                _ => [
                    "core:cactus",
                    "core:torch",
                    "core:farmland",
                    "core:farmland_wet",
                    "core:oak_log",
                    "core:birch_log",
                    "core:spruce_log",
                    "core:acacia_log",
                    "core:jungle_log",
                    "core:stone",
                    "core:sand",
                    "core:snow",
                    "core:ice",
                    "core:iron_ore",
                    "core:oak_leaves",
                ]
                .contains(&d.key.as_str()),
            }
        })
        .map(|(id, _)| id)
        .collect();
    let textures: Vec<_> = registry
        .textures()
        .iter()
        .map(|key| {
            let path = registry
                .asset_path(key, "textures", "png")
                .context("texture")?;
            Ok(image::open(path)?.into_rgba8().into_raw())
        })
        .collect::<Result<_>>()?;
    let ctx = egui::Context::default();
    let mut images = Vec::new();
    let views = [
        Vec3::new(0.04, -0.10, 1.0),
        Vec3::new(1.0, -0.10, 0.04),
        Vec3::new(0.04, -1.0, 0.10),
        Vec3::new(0.7, -0.45, 1.0),
    ];
    for &id in &keys {
        let d = registry.def(id);
        let flat = [BlockBox {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 0.02, 1.0),
        }];
        let boxes = if d.shape == Shape::Flat {
            &flat[..]
        } else if d.boxes.is_empty() {
            &[BlockBox::FULL][..]
        } else {
            &d.boxes
        };
        let parts: Vec<_> = boxes
            .iter()
            .map(|b| {
                CubeInstance::new(
                    (b.min + b.max) * 0.5 - Vec3::splat(0.5),
                    b.max - b.min,
                    if d.face_textures == [0; 3] {
                        Vec3::from_array(d.color)
                    } else {
                        Vec3::ONE
                    },
                )
                .with_surface(d.face_textures, b.min, b.max, d.cutout, d.alpha)
            })
            .collect();
        let mut row = Vec::new();
        for (v, direction) in views.into_iter().chain([views[3]]).enumerate() {
            let pixels = common::model_image(
                &parts,
                &textures,
                direction,
                if v == 4 { 0.14 } else { 1.0 },
            );
            row.push(ctx.load_texture(
                format!("{}:{v}", d.key),
                egui::ColorImage::from_rgba_unmultiplied([64, 64], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        for wall in [false, true] {
            let mut scene = Vec::new();
            if wall {
                scene.extend(parts.iter().copied());
                let stone = registry.def(registry.id("core:stone").context("pierre")?);
                scene.push(
                    CubeInstance::new(
                        Vec3::new(0.0, 0.0, 1.0),
                        Vec3::new(2.0, 1.0, 1.0),
                        Vec3::ONE,
                    )
                    .with_surface(
                        stone.face_textures,
                        Vec3::ZERO,
                        Vec3::new(2.0, 1.0, 1.0),
                        false,
                        1.0,
                    ),
                );
            } else {
                for shift in [-0.5, 0.5] {
                    for part in &parts {
                        let mut copy = *part;
                        copy.center[0] += shift;
                        scene.push(copy);
                    }
                }
            }
            let pixels = common::scene_image(&scene, &textures, views[3], 1.0, 25.0);
            row.push(ctx.load_texture(
                format!("{}:scene:{wall}", d.key),
                egui::ColorImage::from_rgba_unmultiplied([64, 64], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        images.push(row);
    }
    let width = 1200;
    let height = 70 + keys.len() * 94;
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
                    egui::pos2(16.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} · contrôle CPU (pas une capture de partie)", args[1]),
                    egui::FontId::proportional(18.0),
                    Color32::from_rgb(235, 225, 205),
                );
                for (col, label) in ["face", "côté", "dessus", "volume", "nuit", "voisins", "mur"]
                    .iter()
                    .enumerate()
                {
                    p.text(
                        egui::pos2(260.0 + col as f32 * 130.0, 42.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(14.0),
                        Color32::LIGHT_GRAY,
                    );
                }
                for (r, &id) in keys.iter().enumerate() {
                    let y = 70.0 + r as f32 * 94.0;
                    p.text(
                        egui::pos2(12.0, y + 28.0),
                        egui::Align2::LEFT_CENTER,
                        &registry.def(id).key,
                        egui::FontId::monospace(13.0),
                        Color32::WHITE,
                    );
                    for (c, tex) in images[r].iter().enumerate() {
                        p.image(
                            tex.id(),
                            egui::Rect::from_min_size(
                                egui::pos2(220.0 + c as f32 * 130.0, y),
                                egui::vec2(80.0, 80.0),
                            ),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
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
