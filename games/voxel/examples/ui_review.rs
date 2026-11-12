//! Contrôle CPU des fenêtres, cases, infobulles et fiches de visée de production.
#[allow(dead_code)]
mod common;
#[path = "../src/enchanting.rs"]
#[allow(dead_code)]
mod enchanting;
#[path = "../src/item_art.rs"]
mod item_art;
#[path = "../src/settings.rs"]
#[allow(dead_code)]
mod settings;
#[path = "../src/skin.rs"]
#[allow(dead_code)]
mod skin;
#[path = "../src/ui_info.rs"]
#[allow(dead_code)]
mod ui_info;
#[path = "../src/ui_layout.rs"]
#[allow(dead_code)]
mod ui_layout;
#[path = "../src/ui_options.rs"]
mod ui_options;
use anyhow::{Context as _, Result};
use engine_render::egui::{self, Color32, TextureId};
use engine_voxel::{BlockRegistry, ItemRegistry};
use std::{collections::HashMap, path::Path};

fn empty_slot(ui: &mut egui::Ui, size: f32) {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    skin::slot(ui.painter(), rect, false, response.hovered());
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 4,
        "ui_review <menu|pause|options|inventory|hud|before> <largeur> <hauteur> <sortie.png>"
    );
    let w: usize = args[1].parse()?;
    let h: usize = args[2].parse()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let blocks = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods")))?;
    let items = ItemRegistry::builtin(&blocks)?;
    let textures = blocks
        .textures()
        .iter()
        .map(|key| {
            Ok(image::open(
                blocks
                    .asset_path(key, "textures", "png")
                    .context("texture")?,
            )?
            .into_rgba8()
            .into_raw())
        })
        .collect::<Result<Vec<_>>>()?;
    let ctx = egui::Context::default();
    skin::apply(&ctx);
    let keys = [
        "core:grass",
        "core:stone",
        "core:oak_log",
        "core:cable",
        "core:pipe",
        "core:furnace",
        "core:chest",
        "core:iron_pickaxe",
        "core:iron_sword",
    ];
    let mut icons = Vec::new();
    for key in keys {
        let def = items.get(items.id(key).unwrap());
        let (size, pixels) = match &def.icon {
            engine_voxel::Icon::Block(id) => (32, item_art::block_icon(blocks.def(*id), &textures)),
            engine_voxel::Icon::Texture(key) => (
                16,
                image::open(blocks.asset_path(key, "items", "png").unwrap())?
                    .into_rgba8()
                    .into_raw(),
            ),
        };
        icons.push(ctx.load_texture(
            key,
            egui::ColorImage::from_rgba_unmultiplied([size, size], &pixels),
            egui::TextureOptions::NEAREST,
        ));
    }
    let mut settings = settings::Settings::default();
    let mut waiting = None;
    let mut atlas: HashMap<TextureId, egui::ColorImage> = HashMap::new();
    let mut shapes = Vec::new();
    let mut hover = None;
    for frame in 0..12 {
        let events = if frame == 6 {
            hover
                .map(|p| vec![egui::Event::PointerMoved(p)])
                .unwrap_or_default()
        } else {
            vec![]
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(frame as f64 * 0.1),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(w as f32, h as f32),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                ui.painter()
                    .rect_filled(ui.ctx().content_rect(), 0, Color32::from_rgb(20, 29, 31));
                let ctx = ui.ctx();
                match args[0].as_str() {
                    "options" | "before" => {
                        if args[0] == "before" {
                            egui::Window::new("Options")
                                .frame(skin::panel())
                                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                                .resizable(false)
                                .collapsible(false)
                                .title_bar(false)
                                .show(ctx, |ui| {
                                    ui.vertical_centered(|ui| {
                                        skin::mark(ui);
                                        ui.heading("Options");
                                    });
                                    legacy_options(ui, &mut settings);
                                    skin::menu_button(ui, "Retour");
                                });
                        } else {
                            ui_layout::menu(ctx, "Options", 640.0, |ui| {
                                ui_options::panel(ui, &mut settings, &mut waiting);
                            });
                        }
                    }
                    "menu" => ui_layout::menu(ctx, "Voxel", 500.0, |ui| {
                        skin::menu_button(ui, "Jouer");
                        skin::menu_button(ui, "Nouveau monde");
                        ui.add_space(10.0);
                        ui_layout::paired_buttons(ui, "Options…", "Quitter");
                    }),
                    "pause" => ui_layout::menu(ctx, "Pause", 360.0, |ui| {
                        for label in ["Reprendre", "Options", "Sauvegarder et retourner au menu"] {
                            skin::menu_button(ui, label);
                        }
                    }),
                    "inventory" => {
                        let window = ui_layout::inventory_window(
                            ctx,
                            "Inventaire",
                            egui::Id::unique("review_inventory"),
                        )
                        .show(ctx, |ui| {
                            skin::inventory_style(ui);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Inventaire").strong());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let _ = ui.small_button("×");
                                    },
                                );
                            });
                            ui.add_space(8.0);
                            let slot = ui_layout::slot_size(ctx);
                            ui_layout::inventory_sections(ui, |ui, section| match section {
                                ui_layout::InventorySection::Armor => {
                                    ui.label("Armure");
                                    for label in ["Casque", "Plastron", "Jambières", "Bottes"] {
                                        ui.horizontal(|ui| {
                                            empty_slot(ui, slot);
                                            ui.label(egui::RichText::new(label).size(11.0));
                                        });
                                    }
                                    ui.label(egui::RichText::new("Protection : 0").size(11.0));
                                }
                                ui_layout::InventorySection::Craft => {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            for _ in 0..2 {
                                                ui.horizontal(|ui| {
                                                    empty_slot(ui, slot);
                                                    empty_slot(ui, slot);
                                                });
                                            }
                                        });
                                        ui.label(egui::RichText::new("=>").size(22.0));
                                        empty_slot(ui, slot);
                                    });
                                }
                                ui_layout::InventorySection::Recipes => {
                                    ui.label("Rien à fabriquer avec ce que tu as.");
                                }
                            });
                            ui.separator();
                            let slot = ui_layout::slot_size(ctx);
                            for row in 0..4 {
                                if row == 3 {
                                    ui.add_space(4.0);
                                }
                                ui.horizontal(|ui| {
                                    for col in 0..9 {
                                        let i = (row * 9 + col) % keys.len();
                                        let (rect, response) = ui.allocate_exact_size(
                                            egui::vec2(slot, slot),
                                            egui::Sense::click(),
                                        );
                                        skin::slot(ui.painter(), rect, i == 7, response.hovered());
                                        ui.painter().image(
                                            icons[i].id(),
                                            rect.shrink((slot * 0.14).min(6.0)),
                                            egui::Rect::from_min_max(
                                                egui::Pos2::ZERO,
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            Color32::WHITE,
                                        );
                                        if row == 0 && col == 7 {
                                            hover = Some(rect.center());
                                        }
                                        let def = items.get(items.id(keys[i]).unwrap());
                                        let card = ui_info::item(
                                            def,
                                            1,
                                            if i == 7 {
                                                enchanting::with_enchantment(
                                                    12,
                                                    enchanting::Enchantment::Efficiency,
                                                    2,
                                                )
                                            } else {
                                                0
                                            },
                                        );
                                        ui_info::hover(response, &card);
                                    }
                                });
                            }
                            ui.separator();
                            ui.collapsing("Aide · déplacer les objets", |ui| {
                                ui.label("Clic : prendre / poser · Échap : fermer");
                            });
                        });
                        if let Some(window) = window {
                            skin::inventory_border(&window.response);
                        }
                    }
                    "hud" => {
                        let mut card =
                            ui_info::block(blocks.def(blocks.id("core:gold_ore").unwrap()), None);
                        card.lines.truncate(3);
                        egui::Area::new(egui::Id::unique("target_info"))
                            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
                            .show(ctx, |ui| {
                                ui.set_width((w as f32 - 40.0).min(320.0));
                                egui::Frame::new()
                                    .fill(skin::PANEL)
                                    .stroke(egui::Stroke::new(1.0, skin::BORDER))
                                    .inner_margin(8)
                                    .show(ui, |ui| ui_info::draw(ui, &card, Some(icons[1].id())));
                            });
                        egui::Area::new(egui::Id::unique("hud"))
                            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
                            .show(ctx, |ui| {
                                let slot = ui_layout::slot_size(ctx);
                                ui.set_width(9.0 * slot + 16.0);
                                egui::Frame::new()
                                    .fill(skin::INVENTORY)
                                    .stroke(egui::Stroke::new(2.0, skin::DARK))
                                    .inner_margin(3)
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        ui.horizontal(|ui| {
                                            for (i, tex) in icons.iter().enumerate() {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(slot, slot),
                                                    egui::Sense::hover(),
                                                );
                                                skin::slot(ui.painter(), rect, i == 7, false);
                                                ui.painter().image(
                                                    tex.id(),
                                                    rect.shrink(4.0),
                                                    egui::Rect::from_min_max(
                                                        egui::Pos2::ZERO,
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    Color32::WHITE,
                                                );
                                            }
                                        });
                                    });
                            });
                        let c = ctx.content_rect().center();
                        let p = ctx.layer_painter(egui::LayerId::background());
                        p.line_segment(
                            [c - egui::vec2(5.0, 0.0), c + egui::vec2(5.0, 0.0)],
                            egui::Stroke::new(1.0, skin::TEXT),
                        );
                        p.line_segment(
                            [c - egui::vec2(0.0, 5.0), c + egui::vec2(0.0, 5.0)],
                            egui::Stroke::new(1.0, skin::TEXT),
                        );
                    }
                    _ => {}
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
        &args[3],
        &common::rasterize(&ctx.tessellate(shapes, 1.0), &atlas, w, h),
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}

/// Reproduction de l'ancienne pile de réglages, pour comparer les débordements.
fn legacy_options(ui: &mut egui::Ui, s: &mut settings::Settings) {
    egui::Grid::new("legacy_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for (label, value, min, max) in [
                ("Champ de vision", &mut s.fov, 50.0, 110.0),
                (
                    "Sensibilité de la souris",
                    &mut s.mouse_sensitivity,
                    0.2,
                    3.0,
                ),
                ("Volume général", &mut s.volume_master, 0.0, 1.0),
                ("Musique et ambiance", &mut s.volume_music, 0.0, 1.0),
                ("Effets", &mut s.volume_effects, 0.0, 1.0),
            ] {
                ui.label(label);
                ui.add(egui::Slider::new(value, min..=max));
                ui.end_row();
            }
        });
    ui.separator();
    ui.label("Touches");
    for action in settings::Action::ALL {
        ui.horizontal(|ui| {
            ui.label(action.label());
            ui.add_sized([180.0, 22.0], egui::Button::new(s.keys.name(action)));
        });
    }
}
