//! Vues à la première personne des vraies matrices de la main, sans fenêtre ni sauvegarde.
#[path = "../src/render"]
mod render {
    #[allow(dead_code)]
    pub mod hand_model;
    #[allow(dead_code)]
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
use anyhow::{Context as _, Result};
use engine_render::{
    HandInstance,
    egui::{self, Color32, TextureId},
};
use engine_voxel::{BlockRegistry, ItemRegistry};
use glam::{Mat4, Vec2, Vec3};
use std::{collections::HashMap, path::Path};

#[cfg(test)]
fn test_registry() -> BlockRegistry {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap()
}

fn render(parts: &[HandInstance], textures: &[Vec<u8>], w: usize, h: usize) -> Vec<u8> {
    let mut pixels = [24, 33, 35, 255].repeat(w * h);
    let mut depth = vec![0.0f32; w * h];
    let tan = (70.0f32.to_radians() * 0.5).tan(); // Projection propre au bras du renderer.
    let aspect = w as f32 / h as f32;
    let faces = [
        (
            Vec3::X,
            [
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.5, -0.5, 0.5),
            ],
        ),
        (
            Vec3::NEG_X,
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(-0.5, -0.5, -0.5),
            ],
        ),
        (
            Vec3::Y,
            [
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(-0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.5, 0.5, -0.5),
            ],
        ),
        (
            Vec3::NEG_Y,
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(0.5, -0.5, 0.5),
            ],
        ),
        (
            Vec3::Z,
            [
                Vec3::new(-0.5, -0.5, 0.5),
                Vec3::new(0.5, -0.5, 0.5),
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(-0.5, 0.5, 0.5),
            ],
        ),
        (
            Vec3::NEG_Z,
            [
                Vec3::new(0.5, -0.5, -0.5),
                Vec3::new(-0.5, -0.5, -0.5),
                Vec3::new(-0.5, 0.5, -0.5),
                Vec3::new(0.5, 0.5, -0.5),
            ],
        ),
    ];
    let edge = |a: Vec2, b: Vec2, p: Vec2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let srgb = |c: f32| {
        let c = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0) as u8
    };
    for part in parts {
        let model = Mat4::from_cols_array_2d(&part.model);
        for (face, corners) in faces {
            let normal = model.transform_vector3(face).normalize();
            let shade = 0.55 + 0.45 * normal.dot(Vec3::new(-0.4, 0.8, 0.45).normalize()).max(0.0);
            let face_index = if face.y > 0.0 {
                0
            } else if face.y < 0.0 {
                2
            } else {
                1
            };
            let texture = part.textures[face_index]
                .checked_sub(1)
                .and_then(|i| textures.get(i as usize));
            let side = texture
                .and_then(|t| engine_render::texture_side(t))
                .unwrap_or(16) as usize;
            let lo = Vec3::from_array(part.uv_min);
            let hi = Vec3::from_array(part.uv_max);
            let uv = corners.map(|corner| {
                let q = lo + (hi - lo) * (corner + Vec3::splat(0.5));
                if face.y != 0.0 {
                    Vec2::new(q.x, q.z)
                } else if face.x != 0.0 {
                    Vec2::new(q.z, 1.0 - q.y)
                } else {
                    Vec2::new(q.x, 1.0 - q.y)
                }
            });
            let view = corners.map(|p| model.transform_point3(p));
            if view.iter().any(|p| p.z >= -0.05) {
                continue;
            }
            let projected = view.map(|p| {
                Vec2::new(
                    (p.x / (-p.z * tan * aspect) + 1.0) * w as f32 * 0.5,
                    (1.0 - p.y / (-p.z * tan)) * h as f32 * 0.5,
                )
            });
            for indices in [[0, 1, 2], [0, 2, 3]] {
                let p = indices.map(|i| projected[i]);
                let z = indices.map(|i| -view[i].z);
                let area = edge(p[0], p[1], p[2]);
                if area.abs() < 1e-5 {
                    continue;
                }
                let min = p[0].min(p[1]).min(p[2]).max(Vec2::ZERO);
                let max = p[0].max(p[1]).max(p[2]).min(Vec2::new(w as f32, h as f32));
                for y in min.y.floor() as usize..max.y.ceil() as usize {
                    for x in min.x.floor() as usize..max.x.ceil() as usize {
                        let at = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let b = [
                            edge(p[1], p[2], at) / area,
                            edge(p[2], p[0], at) / area,
                            edge(p[0], p[1], at) / area,
                        ];
                        if b.iter().any(|v| *v < 0.0) {
                            continue;
                        }
                        let inv = b[0] / z[0] + b[1] / z[1] + b[2] / z[2];
                        let index = x + y * w;
                        if inv > depth[index] {
                            let mut texel = [255; 4];
                            if let Some(texture) = texture {
                                let coord = (uv[indices[0]] * (b[0] / z[0])
                                    + uv[indices[1]] * (b[1] / z[1])
                                    + uv[indices[2]] * (b[2] / z[2]))
                                    / inv;
                                let u = (coord.x.clamp(0.0, 0.999) * side as f32) as usize;
                                let v = (coord.y.clamp(0.0, 0.999) * side as f32) as usize;
                                let at = (u + v * side) * 4;
                                texel.copy_from_slice(&texture[at..at + 4]);
                            }
                            if part.cutout != 0 && texel[3] < 128 {
                                continue;
                            }
                            let color: [u8; 3] = std::array::from_fn(|i| {
                                let value = f32::from(texel[i]) / 255.0;
                                let linear = if value <= 0.04045 {
                                    value / 12.92
                                } else {
                                    ((value + 0.055) / 1.055).powf(2.4)
                                };
                                srgb(part.color[i] * linear * shade)
                            });
                            depth[index] = inv;
                            pixels[index * 4..index * 4 + 3].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
    }
    for d in -4i32..=4 {
        for (x, y) in [
            (w as i32 / 2 + d, h as i32 / 2),
            (w as i32 / 2, h as i32 / 2 + d),
        ] {
            let at = (x as usize + y as usize * w) * 4;
            pixels[at..at + 3].copy_from_slice(&[185, 190, 180]);
        }
    }
    pixels
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--pack32"),
        "hand_review <rest|swing|portrait|mods|axes|magic|blocks> <sortie.png> [--pack32]"
    );
    let (view_w, view_h) = if args[0] == "portrait" {
        (216, 288)
    } else {
        (384, 216)
    };
    let board_w = 3 * (view_w + 8) + 8;
    let board_h = 2 * (view_h + 28) + 82;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut blocks = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
    if args.get(2).is_some_and(|arg| arg == "--pack32") {
        blocks.set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)?;
    }
    let mods: Vec<_> = [
        "example",
        "mecanique",
        "magie",
        "donjons",
        "entrepot",
        "forge",
    ]
    .into_iter()
    .map(|ns| (ns.into(), root.join("mods").join(ns)))
    .collect();
    let items = ItemRegistry::load_mods(&blocks, &root.join("assets"), &mods)?;
    let textures = assets::load_textures(&blocks);
    let ctx = egui::Context::default();
    crate::ui::skin::apply(&ctx);
    let keys = if args[0] == "blocks" {
        [
            ("Baguette", "magie:wand"),
            ("Torche", "core:torch"),
            ("Lit", "core:bed"),
            ("Coffre", "core:chest"),
            ("Pousse de chêne", "core:oak_sapling"),
            ("Levier", "core:lever"),
        ]
    } else if args[0] == "magic" {
        [
            ("Baguette", "magie:wand"),
            ("Cristal de mana", "magie:mana_crystal"),
            ("Rune de flamme", "magie:rune_flame"),
            ("Cisailles", "core:shears"),
            ("Tête de pioche", "forge:pickaxe_head"),
            ("Sceau du gardien", "donjons:warden_sigil"),
        ]
    } else if args[0] == "axes" {
        [
            ("Hache en bois", "core:wooden_axe"),
            ("Hache en pierre", "core:stone_axe"),
            ("Hache en fer", "core:iron_axe"),
            ("Hache en or", "core:golden_axe"),
            ("Hache en saphir", "core:sapphire_axe"),
            ("Hache en acier", "forge:steel_axe"),
        ]
    } else if args[0] == "mods" {
        [
            ("Marteau scripté", "forge:bronze_hammer"),
            ("Pioche en acier", "forge:steel_pickaxe"),
            ("Hache en acier", "forge:steel_axe"),
            ("Épée en acier", "forge:steel_sword"),
            ("Épée en bois", "core:wooden_sword"),
            ("Pioche en fer", "core:iron_pickaxe"),
        ]
    } else {
        [
            ("Pioche", "core:iron_pickaxe"),
            ("Pelle", "core:copper_shovel"),
            ("Hache", "core:stone_axe"),
            ("Épée", "core:iron_sword"),
            ("Houe", "core:golden_hoe"),
            ("Cisailles", "core:shears"),
        ]
    };
    let mut images = Vec::new();
    for (label, key) in keys {
        let mut hand = crate::render::hand_model::Hand::new(
            crate::render::hand_model::ArmColors::load(&root.join("assets")),
        );
        hand.visible = true;
        hand.set_aspect_ratio(view_w as f32 / view_h as f32);
        hand.frozen = true;
        if args[0] == "swing" {
            hand.swing = Some(0.42);
        }
        let parts = hand.instances(Some(items.id(key).context("outil")?), &items, &blocks);
        let rgba = render(&parts, &textures, view_w, view_h);
        images.push((
            label,
            ctx.load_texture(
                label,
                egui::ColorImage::from_rgba_unmultiplied([view_w, view_h], &rgba),
                egui::TextureOptions::NEAREST,
            ),
        ));
    }
    let mut atlas: HashMap<TextureId, egui::ColorImage> = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(board_w as f32, board_h as f32),
                )),
                ..Default::default()
            },
            |ui| {
                let p = ui.painter();
                p.text(
                    egui::pos2(16.0, 15.0),
                    egui::Align2::LEFT_TOP,
                    if args[0] == "swing" {
                        "Outils en main · pendant le geste"
                    } else {
                        "Outils en main · pose au repos"
                    },
                    egui::FontId::monospace(22.0),
                    crate::ui::skin::TEXT,
                );
                for (i, (label, texture)) in images.iter().enumerate() {
                    let x = 12.0 + (i % 3) as f32 * (view_w + 8) as f32;
                    let y = 70.0 + (i / 3) as f32 * (view_h + 28) as f32;
                    p.text(
                        egui::pos2(x, y - 19.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::monospace(14.0),
                        crate::ui::skin::TEXT,
                    );
                    p.image(
                        texture.id(),
                        egui::Rect::from_min_size(
                            egui::pos2(x, y),
                            egui::vec2(view_w as f32, view_h as f32),
                        ),
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                p.text(
                    egui::pos2(16.0, board_h as f32 - 17.0),
                    egui::Align2::LEFT_CENTER,
                    "Contrôle CPU · mêmes matrices et projection que le jeu · fond uni",
                    egui::FontId::monospace(12.0),
                    crate::ui::skin::MUTED,
                );
            },
        );
        for (&id, deltas) in &output.textures_delta.set {
            for d in deltas {
                let egui::ImageData::Color(im) = &d.image;
                if let Some([x, y]) = d.pos {
                    let a = atlas.get_mut(&id).unwrap();
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
        shapes = ctx.tessellate(output.shapes, output.pixels_per_point);
        output.textures_delta.clear();
    }
    image::save_buffer(
        &args[1],
        &common::rasterize(&shapes, &atlas, board_w, board_h),
        board_w as u32,
        board_h as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
