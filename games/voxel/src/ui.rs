//! L'interface, avec egui : barre d'objets, écrans (inventaire, établi, four, coffre)
//! et inspecteur de debug (F3).
//!
//! egui est une interface « en mode immédiat » : à chaque image, on décrit toute
//! l'interface depuis zéro (« un bouton ici, un texte là »), et egui s'occupe de
//! l'état (survol, clics). Pas d'objets « bouton » à créer et à garder à jour.

use std::collections::HashMap;
use std::sync::Arc;

use crate::shared::StreamerExt;
use engine_core::winit::window::Window;
use engine_ecs::{Entity, World};
use engine_inspect::Inspect;
use engine_render::egui::{self, Color32, TextureHandle, TextureOptions};
use engine_render::{TEXTURE_SIZE, UiFrame};
use engine_voxel::{BlockRegistry, FurnaceState, Icon, ItemId, ItemRegistry, Recipe};
use glam::IVec3;

use crate::VoxelGame;
use crate::components::{
    Collider, Health, Hurt, Mob, MoveIntent, Player, PreviousPosition, Transform, Velocity, Wander,
};
use crate::inventory::{HOTBAR, ItemStack, Slot, click_slot};
use crate::mobs::MobRegistry;
use crate::player;

const SLOT_SIZE: f32 = 44.0;

pub struct GameUi {
    pub ctx: egui::Context,
    state: egui_winit::State,
    /// Un écran est ouvert (la souris est libre) : lequel est dans `screen`.
    pub inventory_open: bool,
    pub screen: Screen,
    pub inspector_open: bool,
    /// Un menu (principal, pause) est ouvert : egui reçoit les clics.
    pub menu_open: bool,
    /// Une icône par objet, créée au premier affichage.
    icons: HashMap<ItemId, TextureHandle>,
}

/// L'écran ouvert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// L'inventaire (E) : armure, grille 2 × 2, livre de recettes.
    Inventory,
    /// L'établi : grille 3 × 3.
    Workbench,
    Furnace(IVec3),
    Chest(IVec3),
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
            screen: Screen::Inventory,
            inspector_open: false,
            menu_open: false,
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
        response.consumed && (self.inventory_open || self.inspector_open || self.menu_open)
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

    /// L'icône d'un objet : la texture de côté de son bloc, ou son image à lui
    /// (`items/<nom>.png`), ou à défaut sa couleur.
    fn icon(
        &mut self,
        item: ItemId,
        items: &ItemRegistry,
        blocks: &BlockRegistry,
        textures: &[Vec<u8>],
    ) -> egui::TextureId {
        let ctx = self.ctx.clone();
        self.icons
            .entry(item)
            .or_insert_with(|| {
                let def = items.get(item);
                let size = TEXTURE_SIZE as usize;
                let plain = |color: [f32; 3]| {
                    let [r, g, b] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
                    [r, g, b, 255].repeat(size * size)
                };
                let pixels = match &def.icon {
                    Icon::Block(block) => {
                        let layer = blocks.def(*block).face_textures[1] as usize;
                        match layer.checked_sub(1).and_then(|i| textures.get(i)) {
                            Some(pixels) => pixels.clone(),
                            None => plain(blocks.def(*block).color),
                        }
                    }
                    Icon::Texture(key) => blocks
                        .asset_path(key, "items", "png")
                        .and_then(|path| crate::assets::load_png(&path).ok())
                        .unwrap_or_else(|| plain(def.color)),
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
            // Au menu principal, ni barre d'objets ni vie : seulement le menu.
            if !matches!(self.mode, crate::Mode::Title(_)) {
                self.draw_hud(ui.ctx());
            }
            self.draw_menus(ui.ctx());
            if self.ui.inventory_open {
                self.draw_screen(ui.ctx());
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

    /// Dessine une case et la pile qu'elle contient. Renvoie `true` si on a cliqué.
    fn stack_widget(
        &mut self,
        ui: &mut egui::Ui,
        stack: Option<ItemStack>,
        highlighted: bool,
    ) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(SLOT_SIZE, SLOT_SIZE), egui::Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(150));
        let stroke = if highlighted {
            egui::Stroke::new(3.0, Color32::WHITE)
        } else {
            egui::Stroke::new(1.0, Color32::from_gray(90))
        };
        painter.rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
        if let Some(stack) = stack {
            self.paint_stack(&painter, rect, stack);
        }
        let clicked = response.clicked();
        if let Some(stack) = stack {
            let def = self.content.items.get(stack.item);
            let mut text = def.name.clone();
            if let Some(durability) = def.durability {
                text += &format!("\nUsure : {} / {durability}", stack.damage);
            }
            if let Some(armor) = def.armor {
                text += &format!("\nProtection : {}", armor.protection);
            }
            response.on_hover_text(text);
        }
        clicked
    }

    /// L'icône, la quantité et la barre d'usure d'une pile.
    fn paint_stack(&mut self, painter: &egui::Painter, rect: egui::Rect, stack: ItemStack) {
        let icon = self.ui.icon(
            stack.item,
            &self.content.items,
            &self.registry,
            &self.block_textures,
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(icon, rect.shrink(8.0), uv, Color32::WHITE);
        if stack.count > 1 {
            painter.text(
                rect.right_bottom() - egui::vec2(4.0, 2.0),
                egui::Align2::RIGHT_BOTTOM,
                stack.count.to_string(),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        // L'usure : une barre qui passe du vert au rouge.
        if let Some(durability) = self.content.items.get(stack.item).durability
            && stack.damage > 0
        {
            let left = 1.0 - stack.damage as f32 / durability as f32;
            let bar = egui::Rect::from_min_size(
                rect.left_bottom() + egui::vec2(6.0, -7.0),
                egui::vec2(SLOT_SIZE - 12.0, 3.0),
            );
            painter.rect_filled(bar, 0.0, Color32::BLACK);
            let mut fill = bar;
            fill.set_width(bar.width() * left);
            let color = Color32::from_rgb(((1.0 - left) * 255.0) as u8, (left * 220.0) as u8, 40);
            painter.rect_filled(fill, 0.0, color);
        }
    }

    /// Une case de l'inventaire du joueur (ou de son armure, de sa grille).
    fn slot_widget(&mut self, ui: &mut egui::Ui, slot: Slot, highlighted: bool) {
        let stack = *self.inventory.slot_mut(slot);
        if self.stack_widget(ui, stack, highlighted) {
            self.inventory.click(slot, &self.content.items);
        }
    }

    /// La barre d'objets en bas de l'écran, la vie au-dessus, et l'avancement du minage.
    fn draw_hud(&mut self, ctx: &egui::Context) {
        let health = player::health(&self.ecs, self.player);
        if let Some((_, progress)) = self.mining {
            egui::Area::new(egui::Id::unique("mining"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 26.0))
                .show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(60.0, 6.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, 2.0, Color32::from_black_alpha(160));
                    let mut fill = rect.shrink(1.0);
                    fill.set_width(fill.width() * progress.clamp(0.0, 1.0));
                    ui.painter().rect_filled(fill, 1.0, Color32::from_gray(230));
                });
        }
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
                            if self.stack_widget(ui, self.inventory.slots[i], selected) {
                                self.inventory.selected = i;
                            }
                        }
                    });
                    if let Some(item) = self.inventory.selected_item() {
                        ui.label(
                            egui::RichText::new(&self.content.items.get(item).name)
                                .color(Color32::WHITE),
                        );
                    }
                });
            });
    }

    /// La réserve (3 rangées) et la barre d'objets du joueur.
    fn player_slots(&mut self, ui: &mut egui::Ui) {
        for row in 0..3 {
            ui.horizontal(|ui| {
                for column in 0..HOTBAR {
                    self.slot_widget(ui, Slot::Main(HOTBAR + row * HOTBAR + column), false);
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for index in 0..HOTBAR {
                let selected = index == self.inventory.selected;
                self.slot_widget(ui, Slot::Main(index), selected);
            }
        });
    }

    /// La grille d'artisanat (`size` × `size`) et sa case de résultat.
    fn craft_grid(&mut self, ui: &mut egui::Ui, size: usize) {
        let grid = self.inventory.craft_grid(size);
        let recipe = self.content.recipes.find(&grid, size).cloned();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                for y in 0..size {
                    ui.horizontal(|ui| {
                        for x in 0..size {
                            self.slot_widget(ui, Slot::Craft(y * 3 + x), false);
                        }
                    });
                }
            });
            ui.label(egui::RichText::new("=>").size(22.0));
            let result = recipe.as_ref().map(|r| {
                let (item, count) = r.result();
                ItemStack::new(item, count)
            });
            if self.stack_widget(ui, result, recipe.is_some())
                && let Some(recipe) = &recipe
            {
                self.inventory
                    .take_craft_result(recipe, size, &self.content.items);
            }
        });
    }

    /// Le livre de recettes : ce qu'on peut fabriquer avec ce qu'on a. Un clic fabrique.
    fn recipe_book(&mut self, ui: &mut egui::Ui, size: usize) {
        ui.label(egui::RichText::new("Livre de recettes").strong());
        let craftable: Vec<Recipe> = self
            .content
            .recipes
            .iter()
            .filter(|r| r.grid_size() <= size && self.inventory.can_craft(r))
            .cloned()
            .collect();
        if craftable.is_empty() {
            ui.label("Rien à fabriquer avec ce que tu as.");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .min_scrolled_height(200.0)
            .show(ui, |ui| {
                for recipe in craftable {
                    let (item, count) = recipe.result();
                    ui.horizontal(|ui| {
                        if self.stack_widget(ui, Some(ItemStack::new(item, count)), false) {
                            self.inventory.craft_from_book(&recipe, &self.content.items);
                        }
                        let names: Vec<String> = recipe
                            .ingredients()
                            .iter()
                            .filter_map(|i| i.example())
                            .map(|i| self.content.items.get(i).name.clone())
                            .collect();
                        ui.label(format!(
                            "{}\n{}",
                            self.content.items.get(item).name,
                            compact(&names)
                        ));
                    });
                }
            });
    }

    /// L'écran ouvert, et la pile tenue qui suit la souris.
    fn draw_screen(&mut self, ctx: &egui::Context) {
        let title = match self.ui.screen {
            Screen::Inventory => "Inventaire",
            Screen::Workbench => "Établi",
            Screen::Furnace(_) => "Four",
            Screen::Chest(_) => "Coffre",
        };
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -20.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                match self.ui.screen {
                    Screen::Inventory => {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label("Armure");
                                for (i, slot) in engine_voxel::ArmorSlot::ALL.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        self.slot_widget(ui, Slot::Armor(i), false);
                                        ui.label(slot.name());
                                    });
                                }
                                ui.label(format!(
                                    "Protection : {}",
                                    self.inventory.protection(&self.content.items)
                                ));
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                ui.label("Artisanat");
                                self.craft_grid(ui, 2);
                            });
                            ui.separator();
                            ui.vertical(|ui| self.recipe_book(ui, 2));
                        });
                    }
                    Screen::Workbench => {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| self.craft_grid(ui, 3));
                            ui.separator();
                            ui.vertical(|ui| self.recipe_book(ui, 3));
                        });
                    }
                    Screen::Furnace(pos) => self.furnace_panel(ui, pos),
                    Screen::Chest(pos) => self.chest_panel(ui, pos),
                }
                ui.separator();
                self.player_slots(ui);
                ui.label("Clic : prendre, poser, empiler ou échanger une pile. Échap : fermer.");
            });
        // La pile tenue suit la souris.
        if let (Some(held), Some(pointer)) = (self.inventory.cursor, ctx.pointer_hover_pos()) {
            let rect = egui::Rect::from_center_size(pointer, egui::vec2(SLOT_SIZE, SLOT_SIZE));
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::unique("cursor"),
            ));
            self.paint_stack(&painter, rect, held);
        }
    }

    /// Une case d'un conteneur (coffre, four) : ses objets sont dans l'entité du bloc.
    fn container_slot(&mut self, ui: &mut egui::Ui, pos: IVec3, index: usize) -> bool {
        let stored = self.streamer().block_entity(pos).and_then(|e| {
            e.caps
                .as_ref()?
                .inventory
                .as_ref()?
                .slots
                .get(index)
                .cloned()
                .flatten()
        });
        let stack = stored.as_ref().and_then(|s| {
            Some(ItemStack {
                item: self.content.items.id(&s.item)?,
                count: s.count,
                damage: s.damage,
            })
        });
        if !self.stack_widget(ui, stack, false) {
            return false;
        }
        let mut slot = stack;
        let mut cursor = self.inventory.cursor.take();
        click_slot(&mut slot, &mut cursor, &self.content.items, |_| true);
        self.inventory.cursor = cursor;
        let items = self.content.items.clone();
        let mut streamer = self.streamer_mut();
        if let Some(entity) = streamer.block_entity_mut(pos)
            && let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut())
            && let Some(target) = inventory.slots.get_mut(index)
        {
            *target = slot.map(|s| engine_voxel::ItemStack {
                item: items.get(s.item).key.clone(),
                count: s.count,
                damage: s.damage,
            });
        }
        true
    }

    fn chest_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        for row in 0..3 {
            ui.horizontal(|ui| {
                for column in 0..9 {
                    self.container_slot(ui, pos, row * 9 + column);
                }
            });
        }
    }

    fn furnace_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        let state = self
            .streamer()
            .block_entity(pos)
            .map(FurnaceState::read)
            .unwrap_or_default();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label("Entrée");
                changed |= self.container_slot(ui, pos, engine_voxel::FURNACE_INPUT);
                // La flamme : ce qui reste du combustible en cours.
                let burn = if state.burn_total > 0 {
                    state.burn_left as f32 / state.burn_total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(burn)
                        .desired_width(SLOT_SIZE)
                        .fill(Color32::from_rgb(240, 120, 30)),
                );
                ui.label("Combustible");
                changed |= self.container_slot(ui, pos, engine_voxel::FURNACE_FUEL);
            });
            ui.vertical(|ui| {
                ui.add_space(SLOT_SIZE / 2.0);
                let cook = if state.cook_total > 0 {
                    state.progress as f32 / state.cook_total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(cook)
                        .desired_width(90.0)
                        .show_percentage(),
                );
            });
            ui.vertical(|ui| {
                ui.label("Sortie");
                changed |= self.container_slot(ui, pos, engine_voxel::FURNACE_OUTPUT);
            });
        });
        if changed {
            // Le four dort peut-être : on le réveille au prochain tick.
            self.mods
                .machines
                .schedule(engine_voxel::GridPos::world(pos), 1);
        }
    }

    /// L'inspecteur (F3) : mesures, et les entités avec leurs composants modifiables.
    fn draw_inspector(&mut self, ctx: &egui::Context) {
        let eye = player::eye(&self.ecs, self.player, 1.0);
        let chunk = engine_voxel::World::chunk_containing(eye);
        let streamer_stats = self.ecs.streamer().borrow().stats();
        let target_light = self.target.map(|hit| {
            self.ecs
                .streamer()
                .borrow()
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

/// « Planches de chêne × 3, Bâton » : les ingrédients regroupés.
fn compact(names: &[String]) -> String {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for name in names {
        match groups.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count += 1,
            None => groups.push((name.clone(), 1)),
        }
    }
    groups
        .iter()
        .map(|(n, c)| {
            if *c > 1 {
                format!("{n} × {c}")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}
