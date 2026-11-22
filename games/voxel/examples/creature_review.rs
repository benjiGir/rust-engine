//! Vrais modèles et articulations des créatures, skins 16/32, sans fenêtre ni sauvegarde.
#[path = "../src/render"]
mod render {
    pub mod mobs;
}
#[path = "../src/assets.rs"]
#[allow(dead_code)]
mod assets;
#[allow(dead_code)]
mod common;
use anyhow::{Context, Result};
use engine_render::{
    CubeInstance,
    egui::{self, Color32},
};
use engine_voxel::BlockRegistry;
use glam::{Mat4, Vec3};
use serde::Deserialize;
use std::{collections::HashMap, path::Path};
use voxel_sim::{mobs, mobs::sheep_colors};

#[derive(Clone, Deserialize)]
struct Creature {
    id: String,
    name: String,
    size: (f32, f32),
    parts: Vec<mobs::model::MobPart>,
}

fn render(parts: &[CubeInstance], textures: &[Vec<u8>], direction: Vec3, height: f32) -> Vec<u8> {
    let (w, h) = (288, 196);
    let direction = direction.normalize();
    let right = Vec3::Y.cross(-direction).normalize();
    let up = right.cross(direction).normalize();
    let zoom = if height <= 1.4 { 81.0 } else { 60.0 };
    let target = Vec3::Y * (height * 0.5);
    let mut pixels = [27, 36, 39, 255].repeat(w * h);
    let models: Vec<_> = parts
        .iter()
        .map(|p| {
            let pivot = Vec3::Y * p.swing[1];
            let model = Mat4::from_translation(Vec3::from_array(p.center))
                * Mat4::from_rotation_y(-p.yaw)
                * Mat4::from_translation(pivot)
                * Mat4::from_rotation_z(p.swing[0])
                * Mat4::from_translation(-pivot)
                * Mat4::from_scale(Vec3::from_array(p.size));
            (p, model, model.inverse())
        })
        .collect();
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb = |c: f32| {
        let s = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (s.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    for y in 0..h {
        for x in 0..w {
            let origin = target - direction * 8.0
                + right * ((x as f32 + 0.5 - w as f32 * 0.5) / zoom)
                + up * ((h as f32 * 0.5 - y as f32 - 0.5) / zoom);
            let ground = origin + direction * (-origin.y / direction.y);
            let at = (x + y * w) * 4;
            if ground.x * ground.x + ground.z * ground.z * 1.5 < 1.0 {
                pixels[at..at + 3].copy_from_slice(&[19, 27, 29]);
            }
            let mut nearest = f32::INFINITY;
            for (part, model, inverse) in &models {
                let o = inverse.transform_point3(origin);
                let d = inverse.transform_vector3(direction);
                let a = (Vec3::splat(-0.5) - o) / d;
                let b = (Vec3::splat(0.5) - o) / d;
                let near = a.min(b);
                let t = near.max_element();
                if t < 0.0 || t > a.max(b).min_element() || t >= nearest {
                    continue;
                }
                let p = o + d * t + Vec3::splat(0.5);
                let axis = if near.x >= near.y && near.x >= near.z {
                    0
                } else if near.y >= near.z {
                    1
                } else {
                    2
                };
                let mut normal = Vec3::ZERO;
                normal[axis] = -d[axis].signum();
                let uv = Vec3::from_array(part.uv_min)
                    + p * (Vec3::from_array(part.uv_max) - Vec3::from_array(part.uv_min));
                let (coords, face) = match axis {
                    0 => ([uv.z, 1.0 - uv.y], 1),
                    1 => ([uv.x, uv.z], if normal.y > 0.0 { 0 } else { 2 }),
                    _ => ([uv.x, 1.0 - uv.y], 1),
                };
                let texel = part.textures[face]
                    .checked_sub(1)
                    .and_then(|i| textures.get(i as usize))
                    .map_or([255; 4], |pixels| {
                        let side = engine_render::texture_side(pixels).unwrap() as usize;
                        let (tx, ty) = (
                            (coords[0].rem_euclid(1.0) * side as f32) as usize,
                            (coords[1].rem_euclid(1.0) * side as f32) as usize,
                        );
                        pixels[(tx + ty * side) * 4..(tx + ty * side) * 4 + 4]
                            .try_into()
                            .unwrap()
                    });
                if texel[3] < 128 {
                    continue;
                }
                let normal = model.transform_vector3(normal).normalize();
                let light =
                    0.66 + 0.34 * normal.dot(Vec3::new(0.5, 0.9, 0.35).normalize()).max(0.0);
                for c in 0..3 {
                    pixels[at + c] = srgb(linear(texel[c]) * part.color[c] * light);
                }
                nearest = t;
            }
        }
    }
    pixels
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        (args.len() == 3
            || (args.len() == 4 && matches!(args[3].as_str(), "--fauna" | "--sheep-colors")))
            && matches!(args[1].as_str(), "16" | "32"),
        "creature_review <racine RON> <16|32> <sortie.png> [--fauna|--sheep-colors]"
    );
    let source = Path::new(&args[0]);
    let sheep_colors = args.get(3).is_some_and(|a| a == "--sheep-colors");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut blocks = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
    if args[1] == "32" {
        blocks.set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)?;
    }
    let mut creatures = Vec::new();
    for path in [
        "assets/core/entities.ron",
        "mods/example/entities.ron",
        "mods/donjons/entities.ron",
    ] {
        let entries: Vec<Creature> = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(&std::fs::read_to_string(source.join(path))?)?;
        for mut c in entries {
            if sheep_colors && c.id != "core:sheep" {
                continue;
            }
            if args.get(3).is_some_and(|a| a == "--fauna")
                && !["core:sheep", "core:chicken", "core:fish"].contains(&c.id.as_str())
            {
                continue;
            }
            if ["core:farmer", "core:smith", "core:merchant"].contains(&c.id.as_str()) {
                continue;
            }
            mobs::model::bind_textures(&mut c.parts, c.id.split_once(':').unwrap().0, &mut blocks);
            creatures.push(c);
        }
    }
    if sheep_colors {
        let original = creatures.pop().context("modèle du mouton")?;
        creatures = sheep_colors::palette()
            .iter()
            .map(|color| {
                let mut sheep = original.clone();
                sheep.id = format!("core:sheep:{}", color.id);
                sheep.name = color.name.clone();
                sheep
            })
            .collect();
    }
    let textures = assets::load_textures(&blocks);
    let ctx = egui::Context::default();
    let mut frames = Vec::new();
    let mut wool_icons = Vec::new();
    for (index, c) in creatures.iter().enumerate() {
        if sheep_colors {
            let path = blocks
                .asset_path(&sheep_colors::palette()[index].item, "items", "png")
                .context("icône de laine")?;
            let pixels = assets::load_png(&path)?;
            let side = engine_render::texture_side(&pixels).unwrap() as usize;
            wool_icons.push(ctx.load_texture(
                format!("laine:{index}"),
                egui::ColorImage::from_rgba_unmultiplied([side, side], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        let mut row = Vec::new();
        for (i, direction) in [
            Vec3::new(-1.0, -0.45, -1.1),
            Vec3::new(-0.02, -0.25, -1.0),
            Vec3::new(1.0, -0.4, -0.9),
            Vec3::new(-1.0, -0.4, -1.0),
        ]
        .into_iter()
        .enumerate()
        .take(if sheep_colors { 1 } else { 4 })
        {
            let mut parts: Vec<_> = crate::render::mobs::instances(
                &c.parts,
                1.0,
                if i == 3 { 0.85 } else { 0.0 },
                if i == 3 { 0.65 } else { 0.0 },
            )
            .collect();
            if sheep_colors {
                let coat = sheep_colors::SheepCoat(index as u8);
                for (part, cube) in c.parts.iter().zip(&mut parts) {
                    cube.color = (Vec3::from_array(cube.color) * coat.tint_part(part)).to_array();
                }
            }
            row.push(ctx.load_texture(
                format!("{}:{i}", c.id),
                egui::ColorImage::from_rgba_unmultiplied(
                    [288, 196],
                    &render(&parts, &textures, direction, c.size.1),
                ),
                egui::TextureOptions::NEAREST,
            ));
        }
        frames.push(row);
    }
    let (w, h) = (
        1232usize,
        if sheep_colors {
            64 + creatures.len().div_ceil(4) * 230
        } else {
            82 + creatures.len() * 226
        },
    );
    let mut atlas = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w as f32, h as f32),
                )),
                ..Default::default()
            },
            |ui| {
                let p = ui.painter();
                p.rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(20, 28, 31));
                p.text(
                    egui::pos2(18.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    if sheep_colors {
                        format!("MOUTONS ET LAINES · {} × {}", args[1], args[1])
                    } else {
                        format!(
                            "BESTIAIRE · {} × {} · modèles et poses du jeu",
                            args[1], args[1]
                        )
                    },
                    egui::FontId::proportional(22.0),
                    Color32::LIGHT_GRAY,
                );
                for (i, label) in [
                    "Trois-quarts avant",
                    "Profil",
                    "Trois-quarts arrière",
                    "Marche / nage et regard",
                ]
                .iter()
                .enumerate()
                .take(if sheep_colors { 0 } else { 4 })
                {
                    p.text(
                        egui::pos2(18.0 + i as f32 * 304.0, 48.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::monospace(14.0),
                        Color32::LIGHT_GRAY,
                    );
                }
                for (i, (c, row)) in creatures.iter().zip(&frames).enumerate() {
                    let y = if sheep_colors {
                        58.0 + (i / 4) as f32 * 230.0
                    } else {
                        80.0 + i as f32 * 226.0
                    };
                    let x = if sheep_colors {
                        18.0 + (i % 4) as f32 * 304.0
                    } else {
                        18.0
                    };
                    p.text(
                        egui::pos2(x, y),
                        egui::Align2::LEFT_TOP,
                        &c.name,
                        egui::FontId::monospace(16.0),
                        Color32::from_rgb(212, 187, 139),
                    );
                    for (j, texture) in row.iter().enumerate() {
                        p.image(
                            texture.id(),
                            egui::Rect::from_min_size(
                                egui::pos2(x + j as f32 * 304.0, y + 23.0),
                                egui::vec2(288.0, 196.0),
                            ),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if sheep_colors {
                        p.image(
                            wool_icons[i].id(),
                            egui::Rect::from_min_size(
                                egui::pos2(x + 231.0, y + 160.0),
                                egui::Vec2::splat(48.0),
                            ),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
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
        &args[2],
        &common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, w, h),
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
