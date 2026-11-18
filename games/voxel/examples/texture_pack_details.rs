//! Comparaison directe des dessins d'équipement, plantes et mods, sans GPU.
#[allow(dead_code)]
mod common;
use anyhow::{Context, Result};
use engine_render::egui::{self, Color32};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

fn main() -> Result<()> {
    let output = std::env::args().nth(1).context("sortie.png")?;
    let root = std::env::args().nth(3).map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );
    let ctx = egui::Context::default();
    let names = [
        ("core", "items", "iron_pickaxe"),
        ("core", "items", "iron_axe"),
        ("core", "items", "iron_shovel"),
        ("core", "items", "iron_sword"),
        ("core", "items", "golden_hoe"),
        ("forge", "items", "bronze_hammer"),
        ("core", "items", "iron_helmet"),
        ("core", "items", "iron_chestplate"),
        ("core", "items", "iron_leggings"),
        ("core", "items", "iron_boots"),
        ("core", "items", "sapphire_pickaxe"),
        ("forge", "items", "steel_axe"),
        ("core", "items", "water_flask"),
        ("core", "items", "copper_ingot"),
        ("core", "items", "gem_amethyst_3"),
        ("core", "items", "guide"),
        ("core", "items", "backpack_copper"),
        ("core", "items", "carrot"),
        ("core", "textures", "poppy"),
        ("core", "textures", "wheat_3"),
        ("core", "textures", "cabbage_3"),
        ("core", "textures", "carrot_3"),
        ("core", "textures", "oak_sapling"),
        ("core", "textures", "red_mushroom"),
        ("core", "textures", "oak_log_top"),
        ("core", "textures", "birch_log"),
        ("core", "textures", "spruce_planks"),
        ("core", "textures", "sand"),
        ("core", "textures", "ice"),
        ("core", "textures", "cactus_side"),
        ("core", "textures", "workbench_top"),
        ("core", "textures", "chest_top"),
        ("core", "textures", "bed_top"),
        ("core", "textures", "solar_panel"),
        ("entrepot", "textures", "vault_side"),
        ("forge", "textures", "anvil_top"),
    ];
    let mode = std::env::args().nth(2);
    let names: Vec<_> = if mode.as_deref() == Some("--accessories") {
        vec![
            ("magie", "items", "wand"),
            ("magie", "items", "rune_flame"),
            ("magie", "items", "rune_heal"),
            ("magie", "items", "rune_blink"),
            ("magie", "items", "rune_power"),
            ("magie", "items", "rune_reach"),
            ("magie", "textures", "rune_table"),
            ("donjons", "items", "warden_sigil"),
            ("forge", "items", "pickaxe_head"),
            ("forge", "items", "axe_head"),
            ("forge", "items", "sword_blade"),
            ("core", "items", "shears"),
            ("core", "items", "enchanted_book"),
            ("core", "items", "charm_fire_resistance"),
            ("core", "items", "charm_healing"),
            ("core", "items", "charm_regeneration"),
            ("core", "items", "charm_speed"),
            ("core", "items", "charm_strength"),
            ("core", "items", "seal_socket"),
            ("core", "items", "seal_unsocket"),
            ("core", "items", "backpack_leather"),
            ("core", "items", "backpack_copper"),
            ("core", "items", "backpack_iron"),
            ("core", "items", "backpack_sapphire"),
        ]
    } else if mode.as_deref() == Some("--fauna") {
        [
            "wool",
            "feather",
            "raw_mutton",
            "cooked_mutton",
            "raw_chicken",
            "cooked_chicken",
            "raw_fish",
            "cooked_fish",
        ]
        .into_iter()
        .map(|name| ("core", "items", name))
        .collect()
    } else {
        names.to_vec()
    };
    let mut pairs = Vec::new();
    for (namespace, kind, name) in names {
        let original = if namespace == "core" {
            root.join("assets/core")
        } else {
            root.join("mods").join(namespace)
        }
        .join(kind)
        .join(format!("{name}.png"));
        let packed = root
            .join("texture_packs/earth-copper-32/assets")
            .join(namespace)
            .join(kind)
            .join(format!("{name}.png"));
        let mut textures = Vec::new();
        for path in [original, packed] {
            let pixels = image::open(&path)?.into_rgba8();
            textures.push(ctx.load_texture(
                path.to_string_lossy(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [pixels.width() as usize, pixels.height() as usize],
                    pixels.as_raw(),
                ),
                egui::TextureOptions::NEAREST,
            ));
        }
        pairs.push((name, textures));
    }
    let (w, h) = (1280usize, 66 + pairs.len().div_ceil(6) * 134);
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
                p.rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(20, 29, 31));
                p.text(
                    egui::pos2(18.0, 12.0),
                    egui::Align2::LEFT_TOP,
                    "DESSINS · Dans chaque paire : classique 16 à gauche, nouveau 32 à droite",
                    egui::FontId::proportional(20.0),
                    Color32::from_rgb(230, 226, 211),
                );
                for (i, (name, pair)) in pairs.iter().enumerate() {
                    let origin =
                        egui::pos2(18.0 + (i % 6) as f32 * 210.0, 55.0 + (i / 6) as f32 * 134.0);
                    p.text(
                        origin,
                        egui::Align2::LEFT_TOP,
                        *name,
                        egui::FontId::monospace(12.0),
                        Color32::LIGHT_GRAY,
                    );
                    for (j, tex) in pair.iter().enumerate() {
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(j as f32 * 100.0, 22.0),
                            egui::Vec2::splat(96.0),
                        );
                        p.rect_filled(rect, 0, Color32::from_rgb(30, 40, 42));
                        p.image(
                            tex.id(),
                            rect,
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
        output,
        &common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, w, h),
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}
