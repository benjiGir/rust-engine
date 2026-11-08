//! L'interface, avec egui : barre d'objets, inventaire (E) et inspecteur de debug (F3).
//!
//! egui est une interface « en mode immédiat » : à chaque image, on décrit toute
//! l'interface depuis zéro (« un bouton ici, un texte là »), et egui s'occupe de
//! l'état (survol, clics). Pas d'objets « bouton » à créer et à garder à jour.

use std::collections::HashMap;
use std::sync::Arc;

use engine_core::winit::window::Window;
use engine_ecs::{Entity, World};
use engine_inspect::Inspect;
use engine_render::egui::{self, Color32, TextureHandle, TextureOptions};
use engine_render::{TEXTURE_SIZE, UiFrame};
use engine_voxel::{BlockId, BlockRegistry, ChunkStreamer};

use crate::VoxelGame;
use crate::components::{
    Collider, Health, Hurt, Mob, MoveIntent, Player, PreviousPosition, Transform, Velocity, Wander,
};
use crate::inventory::HOTBAR;
use crate::mobs::MobRegistry;
use crate::player;

const SLOT_SIZE: f32 = 44.0;

pub struct GameUi {
    pub ctx: egui::Context,
    state: egui_winit::State,
    pub inventory_open: bool,
    pub inspector_open: bool,
    /// Une icône par bloc, créée au premier affichage.
    icons: HashMap<BlockId, TextureHandle>,
}

impl GameUi {
    pub fn new(window: &Window) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Self {
            ctx,
            state,
            inventory_open: false,
            inspector_open: false,
            icons: HashMap::new(),
        }
    }

    /// Un événement de la fenêtre. `true` si egui le garde pour lui (un clic sur un
    /// bouton) : le jeu ne doit alors pas casser de bloc.
    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &engine_core::winit::event::WindowEvent,
    ) -> bool {
        let response = self.state.on_window_event(window, event);
        // La souris capturée (vue FPS) : l'interface ne reçoit rien.
        response.consumed && (self.inventory_open || self.inspector_open)
    }

    pub fn take_input(&mut self, window: &Window) -> egui::RawInput {
        self.state.take_egui_input(window)
    }

    pub fn handle_output(&mut self, window: &Window, output: egui::PlatformOutput) {
        self.state.handle_platform_output(window, output);
    }

    /// Les textures des blocs ont changé (rechargement) : on refera les icônes.
    pub fn clear_icons(&mut self) {
        self.icons.clear();
    }

    /// L'icône d'un bloc : sa texture de côté, ou sa couleur s'il n'en a pas.
    fn icon(
        &mut self,
        block: BlockId,
        registry: &BlockRegistry,
        textures: &[Vec<u8>],
    ) -> egui::TextureId {
        let ctx = self.ctx.clone();
        self.icons
            .entry(block)
            .or_insert_with(|| {
                let def = registry.def(block);
                let size = TEXTURE_SIZE as usize;
                let layer = def.face_textures[1] as usize;
                let pixels = match layer.checked_sub(1).and_then(|i| textures.get(i)) {
                    Some(pixels) => pixels.clone(),
                    None => {
                        let [r, g, b] = def.color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
                        [r, g, b, 255].repeat(size * size)
                    }
                };
                let image = egui::ColorImage::from_rgba_unmultiplied([size, size], &pixels);
                // `NEAREST` : des pixels nets, le style pixel art.
                ctx.load_texture(def.key.clone(), image, TextureOptions::NEAREST)
            })
            .id()
    }
}

/// Les données de debug affichées par F3, mesurées ailleurs.
#[derive(Debug, Clone, Default)]
pub struct DebugInfo {
    pub fps: f32,
    pub frame_ms: f32,
    pub max_frame_ms: f32,
}

impl VoxelGame {
    /// Décrit toute l'interface de cette image et la prépare pour le GPU.
    pub(crate) fn build_ui(&mut self, raw: egui::RawInput) -> (UiFrame, egui::PlatformOutput) {
        // `Context` est un `Arc` : le cloner ne copie rien, et libère `self` pour la closure.
        let ctx = self.ui.ctx.clone();
        let output = ctx.run_ui(raw, |ui| {
            self.draw_hud(ui.ctx());
            if self.ui.inventory_open {
                self.draw_inventory(ui.ctx());
            }
            if self.ui.inspector_open {
                self.draw_inspector(ui.ctx());
            }
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let frame = UiFrame {
            primitives,
            textures: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        };
        (frame, output.platform_output)
    }

    /// Une case d'inventaire : l'icône et la quantité. Renvoie `true` si on a cliqué.
    fn slot_widget(&mut self, ui: &mut egui::Ui, index: usize, highlighted: bool) -> bool {
        let stack = self.inventory.slots[index];
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(SLOT_SIZE, SLOT_SIZE), egui::Sense::click());
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(150));
        let stroke = if highlighted {
            egui::Stroke::new(3.0, Color32::WHITE)
        } else {
            egui::Stroke::new(1.0, Color32::from_gray(90))
        };
        painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
        if let Some(stack) = stack {
            let icon = self
                .ui
                .icon(stack.item, &self.registry, &self.block_textures);
            let inner = rect.shrink(8.0);
            ui.painter().image(
                icon,
                inner,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            if stack.count > 1 {
                ui.painter().text(
                    rect.right_bottom() - egui::vec2(4.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    stack.count.to_string(),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            }
        }
        let clicked = response.clicked();
        if let Some(stack) = stack {
            response.on_hover_text(&self.registry.def(stack.item).name);
        }
        clicked
    }

    /// La barre d'objets en bas de l'écran, et la vie au-dessus.
    fn draw_hud(&mut self, ctx: &egui::Context) {
        let health = player::health(&self.ecs, self.player);
        egui::Area::new(egui::Id::unique("hud"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    // La vie : un carré rouge par tranche de 2 points (la police d'egui n'a
                    // pas de cœur ♥ : on dessine des formes au lieu d'écrire un symbole).
                    let squares = (health.max / 2.0).ceil() as usize;
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(squares as f32 * 16.0, 14.0),
                        egui::Sense::hover(),
                    );
                    for i in 0..squares {
                        let fill = ((health.current - i as f32 * 2.0) / 2.0).clamp(0.0, 1.0);
                        let cell = egui::Rect::from_min_size(
                            rect.min + egui::vec2(i as f32 * 16.0, 0.0),
                            egui::vec2(13.0, 13.0),
                        );
                        ui.painter()
                            .rect_filled(cell, 2.0, Color32::from_black_alpha(160));
                        if fill > 0.0 {
                            let mut part = cell.shrink(1.5);
                            part.set_width(part.width() * fill);
                            ui.painter()
                                .rect_filled(part, 1.5, Color32::from_rgb(220, 40, 40));
                        }
                    }
                    ui.horizontal(|ui| {
                        for i in 0..HOTBAR {
                            let selected = i == self.inventory.selected;
                            if self.slot_widget(ui, i, selected) {
                                self.inventory.selected = i;
                            }
                        }
                    });
                    if let Some(item) = self.inventory.selected_item() {
                        ui.label(
                            egui::RichText::new(&self.registry.def(item).name)
                                .color(Color32::WHITE),
                        );
                    }
                });
            });
    }

    /// L'inventaire (touche E) : la réserve, la barre, et la pile tenue à la souris.
    fn draw_inventory(&mut self, ctx: &egui::Context) {
        egui::Window::new("Inventaire")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Clic : prendre, poser, empiler ou échanger une pile.");
                for row in 0..3 {
                    ui.horizontal(|ui| {
                        for column in 0..HOTBAR {
                            let index = HOTBAR + row * HOTBAR + column;
                            if self.slot_widget(ui, index, false) {
                                self.inventory.click(index);
                            }
                        }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    for index in 0..HOTBAR {
                        let selected = index == self.inventory.selected;
                        if self.slot_widget(ui, index, selected) {
                            self.inventory.click(index);
                        }
                    }
                });
                match self.inventory.cursor {
                    Some(held) => ui.label(format!(
                        "En main : {} × {}",
                        self.registry.def(held.item).name,
                        held.count
                    )),
                    None => ui.label("En main : rien"),
                };
            });
        // La pile tenue suit la souris.
        if let (Some(held), Some(pointer)) = (self.inventory.cursor, ctx.pointer_hover_pos()) {
            let icon = self
                .ui
                .icon(held.item, &self.registry, &self.block_textures);
            let rect = egui::Rect::from_center_size(pointer, egui::vec2(28.0, 28.0));
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::unique("cursor"),
            ))
            .image(
                icon,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    /// L'inspecteur (F3) : mesures, et les entités avec leurs composants modifiables.
    fn draw_inspector(&mut self, ctx: &egui::Context) {
        let eye = player::eye(&self.ecs, self.player, 1.0);
        let chunk = engine_voxel::World::chunk_containing(eye);
        let streamer_stats = self.ecs.resource::<ChunkStreamer>().stats();
        let target_light = self.target.map(|hit| {
            self.ecs
                .resource::<ChunkStreamer>()
                .world()
                .light(hit.block_pos + hit.normal)
        });
        let render = self.last_render;
        let debug = self.debug.clone();
        egui::Window::new("Inspecteur (F3)")
            .default_pos(egui::pos2(12.0, 12.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
                    ui.label("Images");
                    ui.label(format!(
                        "{:.0} FPS, {:.2} ms (max {:.2} ms)",
                        debug.fps, debug.frame_ms, debug.max_frame_ms
                    ));
                    ui.end_row();
                    ui.label("Position");
                    ui.label(format!("{:.1} {:.1} {:.1}", eye.x, eye.y, eye.z));
                    ui.end_row();
                    ui.label("Chunk");
                    ui.label(format!("{} {} {}", chunk.x, chunk.y, chunk.z));
                    ui.end_row();
                    ui.label("Lumière visée");
                    ui.label(match target_light {
                        Some((sky, block)) => format!("ciel {sky}, blocs {block}"),
                        None => "-".into(),
                    });
                    ui.end_row();
                    ui.label("Chunks");
                    ui.label(format!(
                        "{} chargés, {}/{} dessinés",
                        streamer_stats.loaded, render.meshes_drawn, render.meshes_total
                    ));
                    ui.end_row();
                    ui.label("Entités");
                    ui.label(self.ecs.len().to_string());
                    ui.end_row();
                    ui.label("Créatures");
                    let mobs = self.ecs.resource::<Arc<MobRegistry>>().clone();
                    let mut census: Vec<String> = crate::mobs::census(&self.ecs)
                        .into_iter()
                        .filter_map(|(kind, n)| mobs.get(kind).map(|d| format!("{} × {n}", d.name)))
                        .collect();
                    census.sort();
                    ui.label(if census.is_empty() {
                        "aucune".into()
                    } else {
                        census.join(", ")
                    });
                    ui.end_row();
                    ui.label("Heure");
                    ui.label(self.day.clock());
                    ui.end_row();
                });
                ui.collapsing("Systèmes", |ui| {
                    for timing in self.schedule.timings() {
                        ui.label(format!(
                            "{:?} · {} : {:.3} ms",
                            timing.stage,
                            timing.name,
                            timing.duration.as_secs_f64() * 1000.0
                        ));
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        let mobs = self.ecs.resource::<Arc<MobRegistry>>().clone();
                        let mut entities: Vec<Entity> = self.ecs.entities().collect();
                        entities.sort();
                        for entity in entities {
                            let name = entity_name(&self.ecs, entity, &mobs);
                            ui.collapsing(format!("{name} {entity}"), |ui| {
                                inspect_components(&self.ecs, entity, ui);
                            });
                        }
                    });
            });
    }
}

fn entity_name(world: &World, entity: Entity, mobs: &MobRegistry) -> String {
    if world.has::<Player>(entity) {
        return "Joueur".into();
    }
    world
        .get::<Mob>(entity)
        .and_then(|m| mobs.get(m.kind).map(|d| d.name.clone()))
        .unwrap_or_else(|| "Entité".into())
}

/// Affiche (et laisse modifier) un composant de type `T`, si l'entité en a un.
/// Une fonction générique : le compilateur en crée une version par type de composant.
fn inspect_component<T: Inspect + 'static>(
    world: &World,
    entity: Entity,
    ui: &mut egui::Ui,
    name: &str,
) {
    if let Some(mut component) = world.get_mut::<T>(entity) {
        component.inspect(ui, name);
    }
}

/// Tous les composants inspectables du jeu.
fn inspect_components(world: &World, entity: Entity, ui: &mut egui::Ui) {
    inspect_component::<Transform>(world, entity, ui, "Transform");
    inspect_component::<Velocity>(world, entity, ui, "Velocity");
    inspect_component::<Collider>(world, entity, ui, "Collider");
    inspect_component::<MoveIntent>(world, entity, ui, "MoveIntent");
    inspect_component::<Health>(world, entity, ui, "Health");
    inspect_component::<Wander>(world, entity, ui, "Wander");
    inspect_component::<Hurt>(world, entity, ui, "Hurt");
    inspect_component::<PreviousPosition>(world, entity, ui, "PreviousPosition");
    ui.label(format!(
        "Composants : {}",
        world.component_names(entity).len()
    ));
}

/// Une entrée egui sans fenêtre, pour dessiner l'interface dans une capture.
pub fn offscreen_input(width: u32, height: u32) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32, height as f32),
        )),
        ..Default::default()
    }
}
