//! Contrôle animé des vrais transferts et du rendu des conduits, sans fenêtre ni sauvegarde.
#[path = "../src/assets.rs"]
#[allow(dead_code)]
mod assets;
#[allow(dead_code)]
mod common;
#[path = "../src/conduit_fx.rs"]
#[allow(dead_code)]
mod conduit_fx;
#[path = "../../../crates/engine_voxel/tests/support/transit.rs"]
mod fixture;
#[path = "../src/item_models.rs"]
mod item_models;
#[path = "../src/skin.rs"]
#[allow(dead_code)]
mod skin;
#[path = "../../voxel_sim/src/tone.rs"]
#[allow(dead_code)]
mod tone;

use anyhow::{Context, Result};
use engine_render::{
    CubeInstance,
    egui::{self, Color32, TextureId},
};
use engine_voxel::{BlockAccess, ItemRegistry, conduit_shell};
use glam::Vec3;
use image::RgbaImage;
use std::collections::HashMap;

fn image_scene(parts: &[CubeInstance], textures: &[Vec<u8>]) -> RgbaImage {
    let (w, h) = (288, 208);
    let direction = Vec3::new(0.7, -0.65, 1.0).normalize();
    let right = Vec3::Y.cross(-direction).normalize();
    let up = right.cross(direction).normalize();
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb = |c: f32| {
        let c = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0) as u8
    };
    let mut out = RgbaImage::new(w, h);
    let mut hits = Vec::new();
    for y in 0..h {
        for x in 0..w {
            hits.clear();
            let origin = -direction * 20.0
                + right * ((x as f32 + 0.5 - w as f32 * 0.5) / 48.0)
                + up * ((h as f32 * 0.5 - y as f32 - 0.5) / 48.0);
            for part in parts {
                let center = Vec3::from_array(part.center) - Vec3::new(1.5, 0.5, 1.0);
                let size = Vec3::from_array(part.size);
                let min = center - size * 0.5;
                let max = center + size * 0.5;
                let a = (min - origin) / direction;
                let b = (max - origin) / direction;
                let near = a.min(b);
                let far = a.max(b);
                let t = near.max_element();
                let exit = far.min_element();
                if t < 0.0 || t > exit {
                    continue;
                }
                for depth in [t, exit] {
                    if depth == exit && part.opacity >= 1.0 {
                        continue;
                    }
                    let p = (origin + direction * depth - min) / size;
                    let uv = Vec3::from_array(part.uv_min)
                        + p * (Vec3::from_array(part.uv_max) - Vec3::from_array(part.uv_min));
                    let axis = if near.x >= near.y && near.x >= near.z {
                        0
                    } else if near.y >= near.z {
                        1
                    } else {
                        2
                    };
                    let (coords, face, shade) = match axis {
                        1 => ([uv.x, uv.z], 0, 1.0),
                        0 => ([uv.z, 1.0 - uv.y], 1, 0.82),
                        _ => ([uv.x, 1.0 - uv.y], 1, 0.64),
                    };
                    let texel = part.textures[face]
                        .checked_sub(1)
                        .and_then(|i| textures.get(i as usize))
                        .map_or([255; 4], |pixels| {
                            let tx = (coords[0].rem_euclid(1.0) * 16.0) as usize;
                            let ty = (coords[1].rem_euclid(1.0) * 16.0) as usize;
                            pixels[(tx + ty * 16) * 4..(tx + ty * 16) * 4 + 4]
                                .try_into()
                                .unwrap()
                        });
                    if part.cutout != 0 && texel[3] < 128 {
                        continue;
                    }
                    let color = Vec3::new(
                        linear(texel[0]) * part.color[0],
                        linear(texel[1]) * part.color[1],
                        linear(texel[2]) * part.color[2],
                    ) * shade;
                    hits.push((depth, color, part.opacity * texel[3] as f32 / 255.0));
                }
            }
            hits.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut color = Vec3::new(linear(20), linear(28), linear(31));
            for &(_, c, alpha) in &hits {
                color = c * alpha + color * (1.0 - alpha);
            }
            out.put_pixel(
                x,
                y,
                image::Rgba([srgb(color.x), srgb(color.y), srgb(color.z), 255]),
            );
        }
    }
    out
}

fn background() -> RgbaImage {
    let ctx = egui::Context::default();
    skin::apply(&ctx);
    let mut atlas: HashMap<TextureId, egui::ColorImage> = HashMap::new();
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0, 336.0),
                )),
                ..Default::default()
            },
            |ui| {
                let p = ui.painter();
                p.text(
                    egui::pos2(22.0, 16.0),
                    egui::Align2::LEFT_TOP,
                    "Contenu des conduits · transferts réels",
                    egui::FontId::monospace(21.0),
                    skin::TEXT,
                );
                for (i, label) in [
                    "EAU · flux liquide",
                    "VAPEUR · flux gazeux",
                    "OBJETS · modèles 3D",
                ]
                .into_iter()
                .enumerate()
                {
                    p.text(
                        egui::pos2(160.0 + i as f32 * 320.0, 64.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::monospace(15.0),
                        skin::TEXT,
                    );
                }
                p.text(
                    egui::pos2(22.0, 316.0),
                    egui::Align2::LEFT_CENTER,
                    "Contrôle CPU en mémoire · aucun monde sauvegardé",
                    egui::FontId::monospace(12.0),
                    Color32::GRAY,
                );
            },
        );
        for (&id, deltas) in &output.textures_delta.set {
            for d in deltas {
                let egui::ImageData::Color(im) = &d.image;
                if let Some([x, y]) = d.pos {
                    let a = atlas.get_mut(&id).unwrap();
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
        shapes = ctx.tessellate(output.shapes, output.pixels_per_point);
        output.textures_delta.clear();
    }
    RgbaImage::from_raw(960, 336, common::rasterize(&shapes, &atlas, 960, 336)).unwrap()
}

fn main() -> Result<()> {
    let prefix = std::env::args()
        .nth(1)
        .context("transit_review <préfixe de sortie>")?;
    let mut rig = fixture::Rig::new();
    let items = ItemRegistry::builtin(&rig.blocks)?;
    let textures: Vec<_> = rig
        .blocks
        .textures()
        .iter()
        .map(|key| {
            Ok(image::open(
                rig.blocks
                    .asset_path(key, "textures", "png")
                    .context("texture")?,
            )?
            .into_rgba8()
            .into_raw())
        })
        .collect::<Result<_>>()?;
    let base = background();
    let mut fx = conduit_fx::ConduitFx::default();
    for frame in 0..16 {
        fx.advance(0.1);
        rig.tick();
        rig.tick();
        fx.ingest(rig.networks.take_visual_transfers());
        let mut canvas = base.clone();
        for (index, id) in rig.ids.iter().enumerate() {
            let grid = rig.grids.get(*id).unwrap().borrow();
            let mut parts = Vec::new();
            for p in [
                fixture::SOURCE,
                fixture::ROUTE[0],
                fixture::ROUTE[1],
                fixture::ROUTE[2],
                fixture::ROUTE[3],
                fixture::SINK,
            ] {
                let d = rig.blocks.def(grid.block(p));
                let boxes = grid.world().block_boxes(p);
                let shell = conduit_shell(d, &boxes);
                let (solid, glass) = shell.as_ref().map_or((boxes.as_ref(), &[][..]), |s| {
                    (s.rails.as_slice(), s.windows.as_slice())
                });
                for (b, is_glass) in solid
                    .iter()
                    .map(|b| (b, false))
                    .chain(glass.iter().map(|b| (b, true)))
                {
                    let mut cube = CubeInstance::new(
                        p.as_vec3() + (b.min + b.max) * 0.5,
                        b.max - b.min,
                        if is_glass {
                            Vec3::new(0.35, 0.5, 0.46)
                        } else {
                            Vec3::ONE
                        },
                    );
                    if is_glass {
                        cube.opacity = 0.14;
                    } else {
                        cube = cube.with_surface(d.face_textures, b.min, b.max, d.cutout, d.alpha);
                    }
                    parts.push(cube);
                }
            }
            fx.instances(
                *id,
                &*grid,
                &items,
                &rig.fluids,
                Vec3::ZERO,
                |p| p,
                0.0,
                &mut parts,
                8192,
            );
            let tile = image_scene(&parts, &textures);
            image::imageops::overlay(&mut canvas, &tile, 16 + index as i64 * 320, 88);
        }
        if frame == 10 {
            canvas.save(format!("{prefix}.png"))?;
        }
        canvas.save(format!("{prefix}-{frame:02}.png"))?;
    }
    Ok(())
}
