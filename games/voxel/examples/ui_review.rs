//! Contrôle CPU des fenêtres, cases, infobulles et fiches de visée de production.
#[path = "../src/adventure_data.rs"]
#[allow(dead_code)]
mod adventure_data;
#[path = "../src/affixes.rs"]
#[allow(dead_code)]
mod affixes;
#[path = "../src/attributes.rs"]
#[allow(dead_code)]
mod attributes;
#[path = "../src/charms.rs"]
#[allow(dead_code)]
mod charms;
#[allow(dead_code)]
mod common;
#[path = "../src/enchant_books.rs"]
#[allow(dead_code)]
mod enchant_books;
#[path = "../src/enchant_data.rs"]
#[allow(dead_code)]
mod enchant_data;
#[path = "../src/enchanting.rs"]
#[allow(dead_code)]
mod enchanting;
#[path = "../src/gems.rs"]
#[allow(dead_code)]
mod gems;
#[path = "../src/item_art.rs"]
mod item_art;
#[path = "../src/item_data.rs"]
#[allow(dead_code)]
mod item_data;
#[path = "../src/settings.rs"]
#[allow(dead_code)]
mod settings;
#[path = "../src/skin.rs"]
#[allow(dead_code)]
mod skin;
#[path = "../src/texture_packs.rs"]
#[allow(dead_code)]
mod texture_packs;
#[path = "../src/ui_info.rs"]
#[allow(dead_code)]
mod ui_info;
#[path = "../src/ui_layout.rs"]
#[allow(dead_code)]
mod ui_layout;
#[path = "../src/ui_options.rs"]
mod ui_options;
#[path = "../src/ui_widgets.rs"]
mod ui_widgets;
use anyhow::{Context as _, Result};
use engine_render::egui::{self, Color32, TextureId};
use engine_voxel::{BlockRegistry, ItemRegistry};
use std::{collections::HashMap, path::Path};

fn empty_slot(ui: &mut egui::Ui, size: f32) {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    skin::slot(ui.painter(), rect, false, response.hovered());
}

fn review_slot(ui: &mut egui::Ui, icon: TextureId, count: u32, selected: bool) -> egui::Response {
    let size = ui_layout::slot_size(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
    skin::slot(ui.painter(), rect, selected, response.hovered());
    skin::stack(
        ui.painter(),
        rect,
        icon,
        count,
        selected.then_some(0.62),
        selected.then_some(skin::MAGIC),
    );
    response
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 4,
        "ui_review <menu|packs|pause|options|inventory|tooltip|chest|furnace|brewing|workbench|mod|hud|before> <largeur> <hauteur> <sortie.png>"
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
        "core:iron_ore",
        "core:coal",
        "core:iron_ingot",
        "core:water_flask",
        "core:apple",
        "core:potion_healing",
    ];
    let mut icons = Vec::new();
    for key in keys {
        let def = items.get(items.id(key).unwrap());
        let (size, pixels) = match &def.icon {
            engine_voxel::Icon::Block(id) => (32, item_art::block_icon(&blocks, *id, &textures)),
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
                        skin::menu_button(ui, "Packs de textures…");
                        ui.add_space(10.0);
                        ui_layout::paired_buttons(ui, "Options…", "Quitter");
                    }),
                    "packs" => ui_layout::menu(ctx, "Packs de textures", 640.0, |ui| {
                        let mut selected = settings.texture_pack;
                        texture_packs::panel(ui, &root, &mut selected, settings::TexturePack::Classic);
                    }),
                    "pause" => ui_layout::menu(ctx, "Pause", 360.0, |ui| {
                        for label in ["Reprendre", "Options", "Sauvegarder et retourner au menu"] {
                            skin::menu_button(ui, label);
                        }
                    }),
                    "inventory" | "tooltip" | "chest" | "furnace" | "brewing" | "workbench" | "mod" => {
                        let title = match args[0].as_str() {
                            "chest" => "Coffre", "furnace" => "Four", "brewing" => "Alambic",
                            "workbench" => "Établi", "mod" => "Four à alliages", _ => "Inventaire",
                        };
                        let window = ui_layout::inventory_window(
                            ctx, title, egui::Id::unique("review_inventory"),
                        ).show(ctx, |ui| {
                            ui_layout::inventory_body(ui);
                            ui_layout::inventory_header(ui, title);
                            ui.add_space(6.0);
                            match args[0].as_str() {
                                "inventory" | "tooltip" => {
                                    ui_layout::inventory_sections(ui, |ui, section| match section {
                                        ui_layout::InventorySection::Armor => {
                                            ui.label("Armure");
                                            for label in ["Casque", "Plastron", "Jambières", "Bottes"] {
                                                ui.horizontal(|ui| {
                                                    empty_slot(ui, ui_layout::slot_size(ui.ctx()));
                                                    ui.label(egui::RichText::new(label).size(11.0));
                                                });
                                            }
                                            ui.label(egui::RichText::new("Protection : 0").size(11.0));
                                        }
                                        ui_layout::InventorySection::Craft => ui_layout::crafting(ui, 2, |ui, _| {
                                            empty_slot(ui, ui_layout::slot_size(ui.ctx()));
                                        }),
                                        ui_layout::InventorySection::Recipes => { ui.label("Rien à fabriquer avec ce que tu as."); }
                                    });
                                }
                                "chest" => { ui_layout::slots(ui, 27, 9, |ui, i| { let item = i % keys.len();
                                        let count = items.get(items.id(keys[item]).unwrap()).max_stack.min(64);
                                        review_slot(ui, icons[item].id(), count, false); }); }
                                "furnace" => ui_layout::furnace(ui, 0.72, 0.46, |ui, i| {
                                    review_slot(ui, icons[[9, 10, 11][i]].id(), [12, 32, 6][i], false);
                                }),
                                "brewing" => {
                                    ui_layout::brewing(ui, 0.6, |ui, i| { review_slot(ui, icons[[12, 13, 14][i]].id(), 1, false); });
                                    ui.label("Pomme : soin. Spores : régénération. Carotte : vitesse. Poussière d'ombre : force. Cinabre : résistance au feu.");
                                }
                                "workbench" => {
                                    ui_layout::crafting(ui, 3, |ui, index| {
                                        if index.is_some() { empty_slot(ui, ui_layout::slot_size(ui.ctx())); }
                                        else { review_slot(ui, icons[7].id(), 1, false); }
                                    });
                                    ui.collapsing("Livre de recettes", |ui| { ui.label("Rien à fabriquer avec ce que tu as."); });
                                }
                                "mod" => {
                                    use engine_mods::Widget::*;
                                    let mut widgets = vec![
                                        Bar { text: "Allumé".into(), value: 0.72 }, RowBegin,
                                        Slots { first: 0, count: 3, columns: 3 },
                                        Bar { text: "Alliage : fer + charbon, ou cuivre + malachite".into(), value: 0.46 },
                                        Slots { first: 3, count: 1, columns: 1 }, RowEnd,
                                        Label("Combustible (charbon, bâton) :".into()),
                                        Slots { first: 4, count: 1, columns: 1 },
                                    ];
                                    ui_widgets::draw_widgets(ui, &mut widgets, &mut vec![], &mut |ui, first, count, columns| {
                                        ui_layout::slots(ui, count, columns, |ui, i| { review_slot(ui, icons[[11, 10, 11, 7, 10][first + i]].id(), 8, false); });
                                    });
                                }
                                _ => {}
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            if title != "Inventaire" { ui.label("Inventaire"); }
                            ui_layout::player_inventory(ui, |ui, index| {
                                let i = index % 9;
                                let response = review_slot(ui, icons[i].id(), if i < 7 { 64 } else { 1 }, i == 7);
                                if index == 16 { hover = Some(response.rect.center()); }
                                if args[0] == "tooltip" {
                                    let def = items.get(items.id(keys[i]).unwrap());
                                    ui_info::hover(response, &ui_info::item(def, 1, 0));
                                }
                            });
                            ui.add_space(6.0);
                            ui.collapsing("Aide · déplacer les objets", |ui| {
                                ui.label("Clic : prendre / poser · Échap : fermer");
                            });
                        });
                        if let Some(window) = window { skin::inventory_border(&window.response); }
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
                                            for (i, tex) in icons.iter().take(9).enumerate() {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(slot, slot),
                                                    egui::Sense::hover(),
                                                );
                                                skin::slot(ui.painter(), rect, i == 7, false);
                                                ui.painter().image(
                                                    tex.id(),
                                                    skin::icon_rect(rect),
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
