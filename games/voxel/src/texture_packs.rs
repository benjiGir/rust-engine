//! Packs graphiques du menu principal : le choix n'affecte ni données ni sauvegardes.
use crate::settings::TexturePack;
use engine_render::egui::{self, TextureHandle};
use std::{collections::HashMap, path::PathBuf};
const PACKS: [TexturePack; 3] = [
    TexturePack::Classic,
    TexturePack::Earth32,
    TexturePack::Earth64,
];

#[allow(unused_imports)]
pub(crate) use crate::texture_pack_config::{
    Manifest, available, configure, details, folder, manifest,
};
fn previews(ctx: &egui::Context, dirs: &[PathBuf], pack: TexturePack) -> Vec<TextureHandle> {
    let id = egui::Id::unique("texture_pack_previews");
    let key = format!("{dirs:?}:{pack:?}");
    let mut cache = ctx.data_mut(|d| {
        d.get_temp::<HashMap<String, Vec<TextureHandle>>>(id)
            .unwrap_or_default()
    });
    let handles = cache
        .entry(key)
        .or_insert_with(|| {
            let base = details(pack).map_or_else(
                || voxel_sim::paths::get().game.join("core"),
                |(id, _)| folder(dirs, id).join("assets/core"),
            );
            [
                ("textures", "grass_top"),
                ("textures", "stone"),
                ("textures", "oak_planks"),
                ("textures", "copper_ore"),
                ("items", "iron_pickaxe"),
                ("items", "iron_sword"),
            ]
            .into_iter()
            .filter_map(|(kind, name)| {
                let img = image::open(base.join(kind).join(format!("{name}.png")))
                    .ok()?
                    .to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                Some(ctx.load_texture(
                    format!("pack:{pack:?}:{name}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                    egui::TextureOptions::NEAREST,
                ))
            })
            .collect()
        })
        .clone();
    ctx.data_mut(|d| d.insert_temp(id, cache));
    handles
}

pub fn clear_previews(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<HashMap<String, Vec<TextureHandle>>>(egui::Id::unique("texture_pack_previews"))
    });
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Apply,
    Back,
}

/// Composition commune au menu et à la capture de contrôle.
pub fn panel(
    ui: &mut egui::Ui,
    dirs: &[PathBuf],
    draft: &mut TexturePack,
    active: TexturePack,
) -> Action {
    let width = ui.available_width();
    let mut action = Action::None;
    ui.scope_builder(
        egui::UiBuilder::new().layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(width);
            let compact = ui.ctx().content_rect().height() < 300.0;
            egui::ScrollArea::vertical()
                .id_salt("texture_packs_body")
                .max_height(
                    (ui.ctx().content_rect().height() - if compact { 130.0 } else { 170.0 })
                        .max(30.0),
                )
                .min_scrolled_height(0.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if !compact {
                        ui.label("Choisis l'apparence des blocs et des objets.");
                        ui.label(
                            egui::RichText::new(
                                "Le pack choisi sera utilisé au lancement des mondes.",
                            )
                            .color(crate::skin::MUTED),
                        );
                        ui.add_space(8.0);
                    }
                    for pack in PACKS {
                        let enabled = available(dirs, pack);
                        let selected = *draft == pack;
                        let (title, description) = match details(pack) {
                            None => (
                                "Classique · 16 × 16".to_owned(),
                                "Les dessins d'origine, pour le rendu le plus léger.".to_owned(),
                            ),
                            Some((id, size)) => {
                                let info = manifest(&folder(dirs, id));
                                let (name, description) = info.map_or_else(
                                    || ("Matières détaillées".into(), String::new()),
                                    |p| (p.name, p.description),
                                );
                                (format!("{name} · {size} × {size}"), description)
                            }
                        };
                        let card_width = ui.available_width();
                        egui::Frame::new()
                            .fill(crate::skin::PANEL)
                            .stroke(egui::Stroke::new(
                                2.0,
                                if selected {
                                    crate::skin::SAGE
                                } else {
                                    crate::skin::BORDER
                                },
                            ))
                            .inner_margin(if compact { 4 } else { 10 })
                            .show(ui, |ui| {
                                ui.set_width(
                                    (card_width - if compact { 12.0 } else { 24.0 }).max(1.0),
                                );
                                ui.add_enabled_ui(enabled, |ui| {
                                    ui.radio_value(
                                        draft,
                                        pack,
                                        egui::RichText::new(title).strong(),
                                    );
                                });
                                if !compact {
                                    ui.label(
                                        egui::RichText::new(description)
                                            .size(12.0)
                                            .color(crate::skin::MUTED),
                                    );
                                    let icons = previews(ui.ctx(), dirs, pack);
                                    let side =
                                        ((ui.available_width() - 20.0) / 6.0).clamp(12.0, 48.0);
                                    ui.horizontal(|ui| {
                                        for icon in icons {
                                            ui.add(egui::Image::new((
                                                icon.id(),
                                                egui::Vec2::splat(side),
                                            )));
                                        }
                                    });
                                    if !enabled {
                                        ui.label(
                                            egui::RichText::new("Pack indisponible")
                                                .color(crate::skin::RED),
                                        );
                                    } else if pack == active {
                                        ui.label(
                                            egui::RichText::new("Choix enregistré")
                                                .size(11.0)
                                                .color(crate::skin::SAGE),
                                        );
                                    }
                                }
                            });
                        ui.add_space(if compact { 4.0 } else { 8.0 });
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let width = ui.available_width().min(320.0);
                ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
                let half = (width - ui.spacing().item_spacing.x) * 0.5;
                if ui
                    .add_enabled_ui(available(dirs, *draft), |ui| {
                        crate::skin::menu_button_sized(ui, "Appliquer", half)
                    })
                    .inner
                {
                    action = Action::Apply;
                }
                if crate::skin::menu_button_sized(ui, "Retour", half) {
                    action = Action::Back;
                }
            });
        },
    );
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::BlockRegistry;
    use std::path::PathBuf;
    fn root() -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }
    #[test]
    fn a_click_selects_the_optional_pack_and_apply_returns_the_selection() {
        click_and_apply(TexturePack::Earth32, "Terre, ardoise");
        click_and_apply(TexturePack::Earth64, "Sous-bois et forge");
    }

    fn click_and_apply(expected: TexturePack, label: &str) {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let mut draft = TexturePack::Classic;
        let (mut target, mut apply) = (None, None);
        let mut action = Action::None;
        for frame in 0..10 {
            let click = match frame {
                3 | 4 => target,
                6 | 7 => apply,
                _ => None,
            };
            let mut events = Vec::new();
            if let Some(pos) = click {
                events.push(egui::Event::PointerMoved(pos));
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(frame, 3 | 6),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 720.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::ui_layout::menu(ui.ctx(), "Packs de textures", 640.0, |ui| {
                        let result = panel(
                            ui,
                            &[root().join("texture_packs")],
                            &mut draft,
                            TexturePack::Classic,
                        );
                        if result != Action::None {
                            action = result;
                        }
                    });
                },
            );
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.text().starts_with(label) {
                        target = Some(text.visual_bounding_rect().center());
                    }
                    if text.galley.text() == "Appliquer" {
                        apply = Some(text.visual_bounding_rect().center());
                    }
                }
            }
            output.textures_delta.clear();
        }
        assert_eq!(draft, expected);
        assert_eq!(action, Action::Apply);
    }

    #[test]
    fn header_and_actions_remain_visible_on_small_windows() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        for size in [
            egui::vec2(1280.0, 720.0),
            egui::vec2(640.0, 360.0),
            egui::vec2(320.0, 240.0),
        ] {
            for frame in 0..8 {
                let mut draft = TexturePack::Classic;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui_layout::menu(ui.ctx(), "Packs de textures", 640.0, |ui| {
                            panel(
                                ui,
                                &[root().join("texture_packs")],
                                &mut draft,
                                TexturePack::Classic,
                            );
                        });
                    },
                );
                let visible = |name: &str| {
                    output.shapes.iter().any(|s| match &s.shape {
                        egui::Shape::Text(t) => {
                            t.galley.text() == name
                                && s.clip_rect.contains_rect(t.visual_bounding_rect())
                                && egui::Rect::from_min_size(egui::Pos2::ZERO, size)
                                    .contains_rect(t.visual_bounding_rect())
                        }
                        _ => false,
                    })
                };
                let valid =
                    visible("Packs de textures") && visible("Appliquer") && visible("Retour");
                output.textures_delta.clear();
                if frame > 1 {
                    assert!(valid, "actions ou titre hors du viewport : {size:?}");
                }
            }
        }
    }
    #[test]
    fn optional_pack_changes_graphics_but_not_ids_and_keeps_default_fallbacks() {
        let root = root();
        let mut registry =
            BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap();
        let keys: Vec<_> = registry.iter().map(|(id, d)| (id, d.key.clone())).collect();
        let sounds = registry.asset_path("core:step_soft", "sounds", "wav");
        configure(
            &mut registry,
            TexturePack::Earth32,
            &[root.join("texture_packs")],
        );
        assert_eq!(registry.texture_size(), 32);
        assert!(
            registry
                .asset_path("core:grass_top", "textures", "png")
                .unwrap()
                .starts_with(root.join("texture_packs/earth-copper-32"))
        );
        assert_eq!(
            registry.asset_path("core:step_soft", "sounds", "wav"),
            sounds
        );
        assert_eq!(
            registry
                .iter()
                .map(|(id, d)| (id, d.key.clone()))
                .collect::<Vec<_>>(),
            keys
        );
        configure(
            &mut registry,
            TexturePack::Classic,
            &[root.join("texture_packs")],
        );
        assert_eq!(registry.texture_size(), 16);
        assert!(
            registry
                .asset_path("core:grass_top", "textures", "png")
                .unwrap()
                .starts_with(root.join("assets/core"))
        );
        configure(
            &mut registry,
            TexturePack::Earth32,
            &[root.join("missing_pack_test")],
        );
        assert_eq!(registry.texture_size(), 16);
    }
}
