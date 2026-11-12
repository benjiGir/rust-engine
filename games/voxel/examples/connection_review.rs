//! Rendu CPU des raccords réels dans plusieurs petites scènes, sans monde sauvegardé.
#[allow(dead_code)]
mod common;
use anyhow::{Context as _, Result};
use engine_render::{
    CubeInstance,
    egui::{self, Color32, TextureId},
};
use engine_voxel::{BlockEntity, BlockRegistry, EnergyStorage, FluidTank, Sides, World};
use glam::{IVec3, Vec3};

type Scene = (&'static str, Vec<IVec3>, Option<(&'static str, Sides)>);
use std::{collections::HashMap, path::Path, sync::Arc};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 2, "connection_review <racine> <sortie.png>");
    let root = Path::new(&args[0]);
    let registry = Arc::new(BlockRegistry::load(
        &root.join("assets"),
        Some(&root.join("mods")),
    )?);
    let textures = registry
        .textures()
        .iter()
        .map(|key| {
            Ok(image::open(
                registry
                    .asset_path(key, "textures", "png")
                    .context("texture")?,
            )?
            .into_rgba8()
            .into_raw())
        })
        .collect::<Result<Vec<_>>>()?;
    let cases: Vec<Scene> = vec![
        ("Isolé", vec![IVec3::ZERO], None),
        ("Ligne", (0..4).map(|x| IVec3::new(x, 0, 0)).collect(), None),
        (
            "Coude",
            vec![
                IVec3::ZERO,
                IVec3::X,
                IVec3::new(1, 0, 1),
                IVec3::new(1, 0, 2),
            ],
            None,
        ),
        (
            "Jonction en T",
            vec![IVec3::ZERO, IVec3::X, -IVec3::X, IVec3::Z],
            None,
        ),
        (
            "Montée",
            vec![-IVec3::X, IVec3::ZERO, IVec3::Y, IVec3::Y * 2],
            None,
        ),
        (
            "Six directions",
            vec![
                IVec3::ZERO,
                IVec3::X,
                -IVec3::X,
                IVec3::Y,
                -IVec3::Y,
                IVec3::Z,
                -IVec3::Z,
            ],
            None,
        ),
        (
            "Machine",
            vec![IVec3::ZERO, IVec3::X],
            Some(("compatible", Sides::ALL)),
        ),
        (
            "Face fermée",
            vec![IVec3::ZERO, IVec3::X],
            Some(("compatible", Sides(1))),
        ),
        (
            "Mauvais type",
            vec![IVec3::ZERO, IVec3::X],
            Some(("incompatible", Sides::ALL)),
        ),
        (
            "Contre un mur",
            vec![IVec3::ZERO, IVec3::X],
            Some(("wall", Sides::ALL)),
        ),
    ];
    let ctx = egui::Context::default();
    let mut images = Vec::new();
    for (label, cells, machine) in &cases {
        let mut row = Vec::new();
        for cable in [true, false] {
            let key = if cable { "core:cable" } else { "core:pipe" };
            let mut world = World::with_registry(registry.clone());
            for &p in cells {
                world.set_block(p, registry.id(key).unwrap());
            }
            if let Some((kind, sides)) = machine {
                let tank = if *kind == "compatible" { !cable } else { cable };
                let key = if *kind == "wall" {
                    "core:stone"
                } else if tank {
                    "core:tank"
                } else {
                    "core:battery"
                };
                let p = IVec3::X * 2;
                world.set_block(p, registry.id(key).unwrap());
                if *kind != "wall" {
                    let mut entity = BlockEntity::default();
                    if tank {
                        let mut t = FluidTank::new(16000);
                        t.sides = *sides;
                        entity.caps_mut().tanks.push(t);
                    } else {
                        let mut e = EnergyStorage::new(100000, 2000);
                        e.sides = *sides;
                        entity.caps_mut().energy = Some(e);
                    }
                    world
                        .chunk_mut(IVec3::ZERO)
                        .set_block_entity(2, 0, 0, Some(entity));
                }
            }
            let mut positions = cells.clone();
            if machine.is_some() {
                positions.push(IVec3::X * 2);
            }
            let min = positions.iter().fold(Vec3::MAX, |a, p| a.min(p.as_vec3()));
            let max = positions
                .iter()
                .fold(Vec3::MIN, |a, p| a.max(p.as_vec3() + Vec3::ONE));
            let center = (min + max) * 0.5;
            let mut parts = Vec::new();
            for p in positions {
                let d = registry.def(world.block(p));
                let boxes = world.block_boxes(p);
                let full = [engine_voxel::BlockBox::FULL];
                let boxes = if boxes.is_empty() { &full[..] } else { &boxes };
                for b in boxes {
                    parts.push(
                        CubeInstance::new(
                            p.as_vec3() + (b.min + b.max) * 0.5 - center,
                            b.max - b.min,
                            if d.face_textures == [0; 3] {
                                Vec3::from_array(d.color)
                            } else {
                                Vec3::ONE
                            },
                        )
                        .with_surface(
                            d.face_textures,
                            b.min,
                            b.max,
                            d.cutout,
                            d.alpha,
                        ),
                    );
                }
            }
            let direction = Vec3::new(0.7, -0.65, 1.0).normalize();
            let right = Vec3::Y.cross(-direction).normalize();
            let up = right.cross(direction).normalize();
            let extent = max - min;
            let zoom = (70.0 / extent.dot(right.abs())).min(28.0 / extent.dot(up.abs()));
            let pixels = common::scene_image(&parts, &textures, direction, 1.0, zoom);
            row.push(ctx.load_texture(
                format!("{label}:{key}"),
                egui::ColorImage::from_rgba_unmultiplied([64, 64], &pixels),
                egui::TextureOptions::NEAREST,
            ));
        }
        images.push(row);
    }
    let width = 1000;
    let height = 96 + cases.len() * 134;
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
                    "Câbles et tuyaux · formes du monde · contrôle CPU",
                    egui::FontId::proportional(20.0),
                    Color32::from_rgb(235, 225, 205),
                );
                for (x, label) in [(340.0, "Câble d'énergie"), (740.0, "Tuyau de fluide")] {
                    p.text(
                        egui::pos2(x, 64.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(17.0),
                        Color32::LIGHT_GRAY,
                    );
                }
                for (i, (label, _, _)) in cases.iter().enumerate() {
                    let y = 96.0 + i as f32 * 134.0;
                    p.text(
                        egui::pos2(16.0, y + 54.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(16.0),
                        Color32::WHITE,
                    );
                    for (c, tex) in images[i].iter().enumerate() {
                        p.with_clip_rect(egui::Rect::from_min_size(
                            egui::pos2(190.0 + c as f32 * 400.0, y),
                            egui::vec2(300.0, 128.0),
                        ))
                        .image(
                            tex.id(),
                            egui::Rect::from_min_size(
                                egui::pos2(212.0 + c as f32 * 400.0, y - 64.0),
                                egui::vec2(256.0, 256.0),
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
    image::save_buffer(
        &args[1],
        &common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, width, height),
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
