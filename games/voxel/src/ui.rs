//! L'interface, avec egui : barre d'objets, écrans (inventaire, établi, four, coffre)
//! et inspecteur de debug (F3).
//!
//! egui est une interface « en mode immédiat » : à chaque image, on décrit toute
//! l'interface depuis zéro (« un bouton ici, un texte là »), et egui s'occupe de
//! l'état (survol, clics). Pas d'objets « bouton » à créer et à garder à jour.

#[allow(unused_imports)]
pub(crate) use crate::screens::Screen;
use crate::ui_server::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::skin;
use engine_core::winit::window::Window;
use engine_ecs::{Entity, World};
use engine_inspect::Inspect;
use engine_render::egui::{self, Color32, TextureHandle, TextureOptions};
use engine_render::{TEXTURE_SIZE, UiFrame};
use engine_voxel::{BlockRegistry, Icon, ItemId, ItemRegistry, Recipe};

use crate::VoxelGame;
use crate::components::{
    Collider, Health, Hurt, Mob, MoveIntent, Player, PreviousPosition, Transform, Velocity, Wander,
};
use crate::inventory::{Click, HOTBAR, ItemStack, Slot};
use crate::mobs::MobRegistry;

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
    /// (objet, matériau) : le matériau n'est non nul que pour les pièces taillées.
    icons: HashMap<(ItemId, u32), TextureHandle>,
    /// La fenêtre « Diviser la pile » (Maj + clic droit), si elle est ouverte.
    pub split: Option<Split>,
    /// L'offre choisie dans l'écran d'échange avec un habitant.
    pub trade_selected: usize,
    /// Un message en haut de l'écran (« Point de réapparition fixé ») et son heure.
    pub toast: Option<(String, std::time::Instant)>,
    pub target_bottom: f32,
    /// Le menu d'une machine de mod, s'il est ouvert (`Screen::Mod`).
    pub mod_menu: Option<crate::mod_menu::ModMenuState>,
    /// Les actions du joueur dans ce menu, à transmettre au script : (type, id, valeur).
    pub mod_events: Vec<(i32, i32, i32)>,
    /// Vrai pendant un clic droit : un script peut alors ouvrir son menu.
    /// Les jauges que les mods affichent (voir `player_api.rs`).
    pub mod_hud: crate::player_api::HudBars,
    /// La console : saisie, lignes et commandes validées.
    pub console: crate::console::ConsoleState,
    /// La page d'options d'un mod, ouverte depuis la pause : (mod, menu).
    pub options_menu: Option<(String, engine_mods::ModMenu)>,
    pub options_events: Vec<(i32, i32, i32)>,
    /// Tab est tenu : la liste des joueurs s'affiche.
    pub show_players: bool,
    /// Les clics et touches simulés par `ctl`.
    pub input: crate::ui_input::UiInput,
    /// L'explorateur de recettes (voir `recipe_browser.rs`).
    pub browser: crate::recipe_browser::BrowserState,
    /// La recherche de l'écran du contrôleur de stockage.
    pub storage_search: String,
    /// La fiche des attributs (voir `attributes_ui.rs`).
    pub attributes: crate::attributes_ui::AttributesPanel,
    /// La page ouverte du guide (voir `guide_ui.rs`).
    pub guide: crate::guide_ui::GuideState,
    /// Le livre de quêtes : chapitre, fiche, plan (voir `quest_ui.rs`).
    pub quests: crate::quest_ui::QuestScreen,
}

/// Une pile qu'on s'apprête à diviser, et combien d'objets prendre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub target: SlotRef,
    pub amount: u32,
}

/// Le bouton pressé sur une case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Press {
    Primary,
    Secondary,
    /// Maj + clic : toute la pile passe de l'autre côté (conteneur ouvert).
    Quick,
    /// Maj + clic droit : ouvrir « Diviser la pile ».
    Split,
}

impl GameUi {
    pub fn new(window: &Window) -> Self {
        let ctx = egui::Context::default();
        skin::apply(&ctx);
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
            split: None,
            trade_selected: 0,
            toast: None,
            target_bottom: 12.0,
            mod_menu: None,
            mod_events: Vec::new(),
            mod_hud: Default::default(),
            console: Default::default(),
            options_menu: None,
            show_players: false,
            options_events: Vec::new(),
            input: Default::default(),
            browser: Default::default(),
            storage_search: String::new(),
            attributes: Default::default(),
            guide: Default::default(),
            quests: Default::default(),
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

    pub fn take_input(&mut self, window: &Window, scale: f32) -> egui::RawInput {
        self.ctx.set_zoom_factor(if scale.is_finite() {
            scale.clamp(0.75, 1.5)
        } else {
            1.0
        });
        let mut raw = self.state.take_egui_input(window);
        raw.events.extend(self.input.next_frame());
        raw
    }

    pub fn handle_output(&mut self, window: &Window, output: egui::PlatformOutput) {
        self.state.handle_platform_output(window, output);
    }

    /// Les textures des blocs ont changé (rechargement) : on refera les icônes.
    pub fn clear_icons(&mut self) {
        self.icons.clear();
        crate::texture_packs::clear_previews(&self.ctx);
    }

    /// L'icône d'un objet : la texture de côté de son bloc, ou son image à lui
    /// (`items/<nom>.png`), ou à défaut sa couleur.
    pub(super) fn icon(
        &mut self,
        item: ItemId,
        items: &ItemRegistry,
        blocks: &BlockRegistry,
        textures: &[Vec<u8>],
    ) -> egui::TextureId {
        self.icon_of(item, 0, items, blocks, textures)
    }

    /// Comme [`GameUi::icon`], pour une pile : une pièce taillée est dessinée avec les textures de
    /// son matériau (`damage`), mise en cache par (pièce, matériau).
    pub(super) fn icon_of(
        &mut self,
        item: ItemId,
        damage: u32,
        items: &ItemRegistry,
        blocks: &BlockRegistry,
        textures: &[Vec<u8>],
    ) -> egui::TextureId {
        let ctx = self.ctx.clone();
        let piece = crate::shape_materials::kind_of(items.get(item))
            .zip(crate::shape_materials::key_of(damage).and_then(|key| blocks.id(&key)));
        let variant = if piece.is_some() { damage } else { 0 };
        self.icons
            .entry((item, variant))
            .or_insert_with(|| {
                let def = items.get(item);
                let mut size = TEXTURE_SIZE as usize;
                let plain = |color: [f32; 3]| {
                    let [r, g, b] = color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
                    [r, g, b, 255].repeat(size * size)
                };
                let pixels = match (&def.icon, piece) {
                    (_, Some((kind, material))) => {
                        size = crate::item_art::ICON_SIZE;
                        crate::item_art::piece_icon(blocks, material, kind, textures)
                    }
                    (Icon::Block(block), None) => {
                        let id = *block;
                        let block = blocks.def(id);
                        if matches!(
                            block.shape,
                            engine_voxel::Shape::Cube | engine_voxel::Shape::Model
                        ) {
                            size = crate::item_art::ICON_SIZE;
                            crate::item_art::block_icon(blocks, id, textures)
                        } else {
                            let layer = block.face_textures[1] as usize;
                            match layer.checked_sub(1).and_then(|i| textures.get(i)) {
                                Some(pixels) => pixels.clone(),
                                None => plain(block.color),
                            }
                        }
                    }
                    (Icon::Texture(key), None) => blocks
                        .asset_path(key, "items", "png")
                        .and_then(|path| crate::assets::load_png(&path).ok())
                        .unwrap_or_else(|| plain(def.color)),
                };
                let size = engine_render::texture_side(&pixels).unwrap_or(size as u32) as usize;
                let image = egui::ColorImage::from_rgba_unmultiplied([size, size], &pixels);
                // `NEAREST` : des pixels nets, le style pixel art.
                ctx.load_texture(
                    format!("{}#{variant}", def.key),
                    image,
                    TextureOptions::NEAREST,
                )
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
        let _span = engine_trace::span("interface");
        // `Context` est un `Arc` : le cloner ne copie rien, et libère `self` pour la closure.
        let ctx = self.client.ui.ctx.clone();
        let output = ctx.run_ui(raw, |ui| {
            // Au menu principal et à l'écran de mort, ni barre d'objets ni vie.
            if self.mode == crate::Mode::Playing && !self.client.ui.inventory_open {
                self.draw_hud(ui.ctx());
                self.draw_name_tags(ui.ctx());
                self.draw_player_list(ui.ctx());
                self.draw_pinned_quests(ui.ctx());
            }
            self.draw_toast(ui.ctx());
            self.draw_menus(ui.ctx());
            self.draw_chat(ui.ctx());
            self.draw_mod_hud(ui.ctx());
            if self.client.ui.inventory_open {
                if self.client.ui.screen == Screen::Console {
                    self.draw_console(ui.ctx());
                } else if self.client.ui.screen == Screen::Quests {
                    self.draw_quests_screen(ui.ctx());
                } else {
                    self.draw_screen(ui.ctx());
                }
            }
            if self.client.ui.inspector_open {
                self.draw_inspector(ui.ctx());
            }
        });
        self.client.ui.input.capture(&output.shapes);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        let frame = UiFrame {
            primitives,
            textures: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
        };
        (frame, output.platform_output)
    }

    /// Dessine une case et la pile qu'elle contient. Renvoie `true` si on a cliqué.
    pub(crate) fn stack_widget(
        &mut self,
        ui: &mut egui::Ui,
        stack: Option<ItemStack>,
        highlighted: bool,
    ) -> Option<Press> {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(
                crate::ui_layout::slot_size(ui.ctx()),
                crate::ui_layout::slot_size(ui.ctx()),
            ),
            egui::Sense::click(),
        );
        let painter = ui.painter().clone();
        skin::slot(&painter, rect, highlighted, response.hovered());
        if let Some(stack) = stack {
            self.paint_stack(&painter, rect, stack);
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let press = if response.clicked() {
            Some(if shift && self.quick_enabled() {
                Press::Quick
            } else {
                Press::Primary
            })
        } else if response.secondary_clicked() {
            Some(if shift {
                Press::Split
            } else {
                Press::Secondary
            })
        } else {
            None
        };
        if let Some(stack) = stack {
            if response.hovered() {
                // Pour les touches R, U et A de l'explorateur de recettes.
                self.client.ui.browser.hovered = Some(stack.item);
            }
            let def = self.client.content.items.get(stack.item);
            let mut card = crate::ui_info::item(def, stack.count, stack.damage);
            if let Some(name) =
                crate::shape_materials::piece_name(&self.client.registry, def, stack.damage)
            {
                card.name = name;
            }
            card.lines.extend(
                crate::backpacks::card_lines(&def.key, stack.damage)
                    .into_iter()
                    .map(|(text, tone)| (text, crate::skin::tone(tone))),
            );
            crate::ui_info::hover(response, &card);
        }
        press
    }

    /// L'icône, la quantité et la barre d'usure d'une pile.
    fn paint_stack(&mut self, painter: &egui::Painter, rect: egui::Rect, stack: ItemStack) {
        let icon = self.client.ui.icon_of(
            stack.item,
            stack.damage,
            &self.client.content.items,
            &self.client.registry,
            &self.client.block_textures,
        );
        let durability = self
            .client
            .content
            .items
            .get(stack.item)
            .durability
            .and_then(|max| {
                let wear = crate::enchanting::wear(stack.damage);
                (wear > 0).then(|| (1.0 - wear as f32 / max.max(1) as f32).clamp(0.0, 1.0))
            });
        // Le liseré : la couleur de la rareté, sinon celle de l'enchantement.
        let border = crate::affixes::rarity_of(stack.damage)
            .map(|r| crate::forge_ui::rarity_color(&crate::adventure_data::current(), r))
            .or_else(|| crate::enchanting::is_enchanted(stack.damage).then_some(skin::MAGIC));
        skin::stack(painter, rect, icon, stack.count, durability, border);
    }

    /// Une case de l'inventaire du joueur (ou de son armure, de sa grille).
    pub(crate) fn slot_widget(&mut self, ui: &mut egui::Ui, slot: Slot, highlighted: bool) {
        // L'inventaire tel que le serveur l'a envoyé (voir `player_view.rs`).
        let stack = self.client.view.me.inventory.slot(slot);
        if let Some(press) = self.stack_widget(ui, stack, highlighted) {
            self.press_slot(SlotRef::Player(slot), stack, press);
        }
    }

    /// Un bouton pressé sur une case : un clic de case, ou l'ouverture de « Diviser la pile ».
    /// Le clic part au simulateur (`SlotClick`, voir `intents.rs`) et s'applique tout de suite.
    /// Renvoie `true` si la case a pu changer.
    pub(crate) fn press_slot(
        &mut self,
        target: SlotRef,
        stack: Option<ItemStack>,
        press: Press,
    ) -> bool {
        // Maj + clic : l'autre côté. Une case qui n'est pas concernée (amélioration, filtre,
        // machine) se comporte comme un clic ordinaire (c'est le simulateur qui le décide).
        let (click, shift) = match press {
            Press::Primary => (Click::Primary, false),
            Press::Quick => (Click::Primary, true),
            Press::Secondary => (Click::Secondary, false),
            // Une pile en main : Maj + clic droit pose un objet, comme un clic droit.
            Press::Split if self.client.view.me.inventory.cursor.is_some() => {
                (Click::Secondary, false)
            }
            Press::Split => {
                if let Some(stack) = stack.filter(|s| s.count > 1) {
                    self.client.ui.split = Some(Split {
                        target,
                        amount: stack.count / 2,
                    });
                }
                return false;
            }
        };
        self.send_slot_click(target, click, shift);
        true
    }

    /// Envoie un clic de case au simulateur.
    fn send_slot_click(&mut self, target: SlotRef, click: Click, shift: bool) {
        self.send_now(crate::protocol::ClientMsg::SlotClick {
            slot: self.slot_target_of(target),
            button: click.into(),
            shift,
        });
    }

    /// La pile d'une case.
    fn stack_at(&self, target: SlotRef) -> Option<ItemStack> {
        match target {
            SlotRef::Player(slot) => self.client.view.me.inventory.slot(slot),
            // Une case du conteneur ouvert : sa vue (voir `container_view.rs`).
            SlotRef::Open(index)
            | SlotRef::Container(_, index)
            | SlotRef::Cart(_, index)
            | SlotRef::Bag(_, index) => self.shown_stack(index),
        }
    }

    /// La fenêtre « Diviser la pile » : choisir combien d'objets prendre en main.
    fn split_window(&mut self, ctx: &egui::Context) {
        let Some(mut split) = self.client.ui.split else {
            return;
        };
        // La pile a disparu, ou on tient déjà quelque chose : on referme.
        let Some(stack) = self.stack_at(split.target).filter(|s| s.count > 1) else {
            self.client.ui.split = None;
            return;
        };
        if self.client.view.me.inventory.cursor.is_some() {
            self.client.ui.split = None;
            return;
        }
        let name = self.client.content.items.get(stack.item).name.clone();
        let mut action = None;
        crate::ui_layout::window(ctx, "Diviser la pile", 360.0)
            .order(egui::Order::Foreground)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!("{name} : {} au total", stack.count));
                ui.horizontal(|ui| {
                    ui.label("Prendre");
                    ui.add(
                        egui::DragValue::new(&mut split.amount)
                            .range(1..=stack.count)
                            .speed(0.2),
                    );
                    for (label, amount) in [("1", 1), ("½", stack.count / 2), ("Tout", stack.count)]
                    {
                        if ui.button(label).clicked() {
                            split.amount = amount.max(1);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Prendre").clicked() || enter {
                        action = Some(true);
                    }
                    if ui.button("Annuler").clicked() {
                        action = Some(false);
                    }
                });
            });
        match action {
            Some(true) => {
                self.client.ui.split = None;
                self.send_slot_click(split.target, Click::Take(split.amount), false);
            }
            Some(false) => self.client.ui.split = None,
            None => self.client.ui.split = Some(split),
        }
    }

    /// La barre d'objets en bas de l'écran, la vie au-dessus, et l'avancement du minage.
    fn draw_hud(&mut self, ctx: &egui::Context) {
        // Les jauges et l'inventaire tels que le serveur les a envoyés (voir `player_view.rs`) ;
        // la barre de minage et de repas est la prédiction de la fenêtre.
        let Some(vitals) = self.client.view.me.vitals.clone() else {
            return;
        };
        let since_hit = self.client.view.me.since_hit();
        let hotbar: Vec<_> = self.client.view.me.inventory.slots[..HOTBAR].to_vec();
        let selected_slot = usize::from(self.client.view.me.inventory.selected);
        let held_stack = self.client.view.me.inventory.selected_stack();
        // La barre sous le réticule : le minage, ou le repas en cours.
        if let Some(progress) = self.client.mining.map(|(_, p)| p).or(self
            .server
            .players
            .me_mut()
            .eat_progress)
        {
            egui::Area::new(egui::Id::unique("mining"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 26.0))
                .show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(60.0, 6.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, skin::INSET);
                    let mut fill = rect.shrink(1.0);
                    fill.set_width(fill.width() * progress.clamp(0.0, 1.0));
                    ui.painter().rect_filled(fill, 0.0, skin::COPPER);
                });
        }
        let target_bottom = self.draw_target_info(ctx);
        self.client.ui.target_bottom = target_bottom;
        // Les potions en cours, en haut à droite, avec le temps restant.
        if !vitals.effects.is_empty() {
            egui::Area::new(egui::Id::unique("effects"))
                .anchor(
                    egui::Align2::RIGHT_TOP,
                    egui::vec2(
                        -12.0,
                        if ctx.content_rect().width() < 760.0 {
                            target_bottom
                                + if self.client.ui.toast.is_some() {
                                    96.0
                                } else {
                                    8.0
                                }
                        } else {
                            12.0
                        },
                    ),
                )
                .show(ctx, |ui| {
                    for active in &vitals.effects {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {}:{:02}",
                                active.effect.name(),
                                active.seconds / 60,
                                active.seconds % 60
                            ))
                            .color(skin::MAGIC)
                            .size(15.0),
                        );
                    }
                });
        }
        // Un coup reçu : l'écran rougit un instant.
        let flash = (1.0 - since_hit / 0.35).clamp(0.0, 1.0);
        if flash > 0.0 {
            ctx.layer_painter(egui::LayerId::background()).rect_filled(
                ctx.content_rect(),
                0.0,
                Color32::from_rgba_unmultiplied(200, 0, 0, (flash * 70.0) as u8),
            );
        }
        let air = vitals.air / self.server.survival.air.max(0.01);
        let burning = vitals.burning;
        let [health_now, health_max] = vitals.health;
        egui::Area::new(egui::Id::unique("hud"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    // L'air, seulement quand il manque : dix bulles bleues.
                    if air < 1.0 {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(10.0 * 16.0, 12.0),
                            egui::Sense::hover(),
                        );
                        let full = air * 10.0;
                        for i in 0..10 {
                            let center = rect.min + egui::vec2(i as f32 * 16.0 + 6.5, 6.0);
                            let fill = (full - i as f32).clamp(0.0, 1.0);
                            ui.painter()
                                .circle_filled(center, 5.5, Color32::from_black_alpha(140));
                            if fill > 0.0 {
                                ui.painter().circle_filled(center, 4.5 * fill, skin::WATER);
                            }
                        }
                    }
                    // La vitalité : un losange par tranche de 2 points ; la réserve, des grains.
                    // Orange quand le joueur brûle.
                    let heart = if burning { skin::GOLD } else { skin::RED };
                    // À droite, la réserve de nourriture, couleur blé.
                    let squares = (health_max / 2.0).ceil().clamp(1.0, 20.0) as usize;
                    let food_squares = (self.server.hunger_rules.max_food / 2.0)
                        .ceil()
                        .clamp(1.0, 20.0) as usize;
                    let hotbar_width = HOTBAR as f32 * crate::ui_layout::slot_size(ctx)
                        + (HOTBAR - 1) as f32 * 2.0;
                    let step = ((hotbar_width - 24.0) / (squares + food_squares) as f32).min(16.0);
                    let gap = hotbar_width - (squares + food_squares) as f32 * step;
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(hotbar_width, 14.0), egui::Sense::hover());
                    let food = skin::GOLD;
                    let rows = [
                        (
                            0.0,
                            squares,
                            health_now / health_max.max(1.0) * squares as f32,
                            heart,
                        ),
                        (
                            squares as f32 * step + gap,
                            food_squares,
                            vitals.food / self.server.hunger_rules.max_food.max(1.0)
                                * food_squares as f32,
                            food,
                        ),
                    ];
                    for (x, count, value, color) in rows {
                        for i in 0..count {
                            let fill = (value - i as f32).clamp(0.0, 1.0);
                            let cell = egui::Rect::from_min_size(
                                rect.min + egui::vec2(x + i as f32 * step, 0.0),
                                egui::vec2((step * 0.8).min(13.0), 13.0),
                            );
                            skin::meter(ui.painter(), cell, fill, color, x > 0.0);
                        }
                    }
                    // L'expérience : une barre verte, le niveau par-dessus.
                    let level = vitals.level;
                    let (bar, _) =
                        ui.allocate_exact_size(egui::vec2(rect.width(), 7.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 0.0, skin::INSET);
                    let mut fill = bar.shrink(1.0);
                    fill.set_width(fill.width() * vitals.progress);
                    ui.painter().rect_filled(fill, 0.0, skin::SAGE);
                    if level > 0 {
                        // Dans l'espace entre la vie et la faim, juste au-dessus de la barre.
                        // Un contour sombre (le texte décalé dans 4 directions), puis le
                        // chiffre : lisible sur le décor.
                        let at = egui::pos2(bar.center().x, rect.center().y);
                        let font = egui::FontId::proportional(14.0);
                        let text = level.to_string();
                        for offset in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                            ui.painter().text(
                                at + egui::vec2(offset.0, offset.1),
                                egui::Align2::CENTER_CENTER,
                                &text,
                                font.clone(),
                                Color32::BLACK,
                            );
                        }
                        ui.painter()
                            .text(at, egui::Align2::CENTER_CENTER, text, font, skin::SAGE);
                    }
                    egui::Frame::new()
                        .fill(skin::INVENTORY)
                        .stroke(egui::Stroke::new(2.0, skin::DARK))
                        .inner_margin(3)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.horizontal(|ui| {
                                for (i, &stack) in hotbar.iter().enumerate() {
                                    let selected = i == selected_slot;
                                    if self.stack_widget(ui, stack, selected)
                                        == Some(Press::Primary)
                                    {
                                        self.send_now(crate::protocol::ClientMsg::SelectSlot(
                                            i as u8,
                                        ));
                                    }
                                }
                            });
                        });
                    if let Some(held) = held_stack {
                        // Un objet à affixes : son nom composé, dans la couleur de sa rareté.
                        let data = crate::adventure_data::current();
                        let sheet = crate::item_data::sheet(held.damage);
                        let color = sheet
                            .as_ref()
                            .and_then(crate::affixes::display_rarity)
                            .map_or(skin::TEXT, |r| crate::forge_ui::rarity_color(&data, r));
                        let item = self.client.content.items.get(held.item);
                        let name = sheet
                            .and_then(|s| s.name)
                            .or_else(|| {
                                crate::shape_materials::piece_name(
                                    &self.client.registry,
                                    item,
                                    held.damage,
                                )
                            })
                            .unwrap_or_else(|| item.name.clone());
                        ui.label(egui::RichText::new(name).color(color));
                    }
                });
            });
    }

    /// La réserve (3 rangées) et la barre d'objets du joueur.
    fn player_slots(&mut self, ui: &mut egui::Ui) {
        crate::ui_layout::player_inventory(ui, |ui, index| {
            self.slot_widget(
                ui,
                Slot::Main(index),
                index == usize::from(self.client.view.me.inventory.selected),
            );
        });
    }

    /// La grille d'artisanat (`size` × `size`) et sa case de résultat.
    fn craft_grid(&mut self, ui: &mut egui::Ui, size: usize) {
        let grid = self
            .client
            .view
            .me
            .inventory
            .as_inventory()
            .craft_grid(size);
        let recipe = self.client.content.recipes.find(&grid, size).cloned();
        crate::ui_layout::crafting(ui, size, |ui, index| {
            if let Some(index) = index {
                self.slot_widget(ui, Slot::Craft(index / size * 3 + index % size), false);
            } else {
                let result = recipe.as_ref().map(|r| {
                    let (item, count) = r.result();
                    ItemStack::new(item, count)
                });
                let clicked =
                    self.stack_widget(ui, result, recipe.is_some()) == Some(Press::Primary);
                if clicked && recipe.is_some() {
                    self.send_now(crate::protocol::ClientMsg::Craft {
                        size: size as u8,
                        recipe: None,
                    });
                }
            }
        });
    }

    /// Le livre de recettes : ce qu'on peut fabriquer avec ce qu'on a. Un clic fabrique.
    fn recipe_book(&mut self, ui: &mut egui::Ui, size: usize) {
        ui.label(egui::RichText::new("Livre de recettes").strong());
        let inventory = self.client.view.me.inventory.as_inventory();
        // Le numéro de chaque recette dans la liste : c'est lui qui part au serveur.
        let craftable: Vec<(u32, Recipe)> = self
            .client
            .content
            .recipes
            .iter()
            .enumerate()
            .filter(|(_, r)| r.grid_size() <= size && inventory.can_craft(r))
            .map(|(i, r)| (i as u32, r.clone()))
            .collect();
        if craftable.is_empty() {
            ui.label("Rien à fabriquer avec ce que tu as.");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height((ui.ctx().content_rect().height() * 0.3).min(240.0))
            .min_scrolled_height(60.0)
            .show(ui, |ui| {
                for (index, recipe) in craftable {
                    let (item, count) = recipe.result();
                    ui.horizontal(|ui| {
                        let clicked =
                            self.stack_widget(ui, Some(ItemStack::new(item, count)), false)
                                == Some(Press::Primary);
                        if clicked {
                            self.send_now(crate::protocol::ClientMsg::Craft {
                                size: size as u8,
                                recipe: Some(index),
                            });
                        }
                        let names: Vec<String> = recipe
                            .ingredients()
                            .iter()
                            .filter_map(|i| i.example())
                            .map(|i| self.client.content.items.get(i).name.clone())
                            .collect();
                        ui.label(format!(
                            "{}\n{}",
                            self.client.content.items.get(item).name,
                            compact(&names)
                        ));
                    });
                }
            });
    }

    /// L'écran ouvert, et la pile tenue qui suit la souris.
    fn draw_screen(&mut self, ctx: &egui::Context) {
        self.client.ui.browser.hovered = None;
        let title = match self.client.ui.screen {
            Screen::Inventory => "Inventaire".to_string(),
            Screen::Workbench => "Établi".to_string(),
            Screen::Furnace(_) => "Four".to_string(),
            Screen::Brewing(_) => "Alambic".to_string(),
            Screen::Chest(_) | Screen::Bag(_) | Screen::Controller(_) => self.storage_title(),
            Screen::Cart(_) => self.cart_title(),
            Screen::Trade(_) => "Échange".to_string(),
            Screen::Enchant(_) => "Table d'enchantement".to_string(),
            Screen::Anvil => "Enclume simple".to_string(),
            Screen::Library(_) => "Bibliothèque d'enchantements".to_string(),
            Screen::Salvage => "Table de récupération".to_string(),
            Screen::Reforge => "Table de réforge".to_string(),
            Screen::Shaping => "Établi de taille".to_string(),
            Screen::Tiers => "Paliers du monde".to_string(),
            Screen::Guide => "Guide du voyageur".to_string(),
            Screen::Console | Screen::Quests => String::new(),
            Screen::Mod(_) => self
                .client
                .ui
                .mod_menu
                .as_ref()
                .map_or_else(|| "Machine".to_string(), |m| m.menu.title.clone()),
        };
        let mut open = true;
        let window = crate::ui_layout::inventory_window(ctx, &title, egui::Id::unique((
            "inventory_screen",
            std::mem::discriminant(&self.client.ui.screen),
        )))
            .show(ctx, |ui| {
                crate::ui_layout::inventory_body(ui);
                if crate::ui_layout::inventory_header(ui, &title) { open = false; }
                ui.add_space(6.0);
                match self.client.ui.screen {
                    Screen::Inventory => self.inventory_panel(ui),
                    Screen::Workbench => {
                        if ui.available_width() >= 600.0 {
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(
                                        4.0 * crate::ui_layout::slot_size(ui.ctx()) + 50.0,
                                    );
                                    self.craft_grid(ui, 3);
                                });
                                ui.separator();
                                ui.vertical(|ui| self.recipe_book(ui, 3));
                            });
                        } else {
                            self.craft_grid(ui, 3);
                            ui.collapsing("Livre de recettes", |ui| self.recipe_book(ui, 3));
                        }
                    }
                    Screen::Furnace(_) => self.furnace_panel(ui),
                    Screen::Brewing(_) => self.brewing_panel(ui),
                    Screen::Chest(_) => self.chest_panel(ui),
                    Screen::Bag(_) => {
                        self.storage_panel(ui, true)
                    }
                    Screen::Controller(pos) => self.controller_panel(ui, pos),
                    Screen::Cart(_) => self.cart_panel(ui),
                    Screen::Trade(_) => self.trade_panel(ui),
                    Screen::Enchant(pos) => self.enchant_panel(ui, pos),
                    Screen::Anvil => self.anvil_panel(ui),
                    Screen::Library(pos) => self.library_panel(ui, pos),
                    Screen::Salvage => self.salvage_panel(ui),
                    Screen::Reforge => self.reforge_panel(ui),
                    Screen::Shaping => self.shaping_panel(ui),
                    Screen::Tiers => self.tiers_panel(ui),
                    Screen::Guide => self.guide_panel(ui),
                    Screen::Mod(_) => self.mod_panel(ui),
                    Screen::Console | Screen::Quests => {}
                }
                if !matches!(self.client.ui.screen, Screen::Tiers | Screen::Guide) {
                    ui.add_space(8.0);
                    ui.separator();
                    if self.client.ui.screen != Screen::Inventory { ui.label("Inventaire"); }
                    self.player_slots(ui);
                    ui.add_space(6.0);
                    ui.collapsing("Aide · déplacer les objets", |ui| {
                        ui.label("Clic : prendre ou poser. Clic droit : moitié ou un objet. Maj + clic : envoyer la pile de l'autre côté (coffre, sac). Maj + clic droit : choisir la quantité. Une gemme ou un sceau au curseur, cliqué sur un objet : sertir, retirer, ajouter un emplacement. Échap : fermer.");
                    });
                }
            });
        // Le cadre de l'inventaire, centré (voir `ui_layout::inventory_window`) : l'explorateur
        // de recettes se place de part et d'autre.
        let frame = window.as_ref().map(|w| {
            let safe = crate::ui_layout::safe_rect(ctx);
            let half = crate::ui_layout::inventory_width(ctx) * 0.5;
            egui::Rect::from_x_y_ranges(
                (safe.center().x - half)..=(safe.center().x + half),
                w.response.rect.y_range(),
            )
        });
        if let Some(window) = window {
            skin::inventory_border(&window.response);
        }
        if !open {
            self.close_screen();
            return;
        }
        self.split_window(ctx);
        self.attributes_window(ctx, frame);
        if let Some(frame) = frame {
            self.draw_recipe_browser(ctx, frame);
        }
        // La pile tenue suit la souris.
        if let (Some(held), Some(pointer)) = (
            self.client.view.me.inventory.cursor,
            ctx.pointer_hover_pos(),
        ) {
            let rect = egui::Rect::from_center_size(
                pointer,
                egui::Vec2::splat(crate::ui_layout::slot_size(ctx)),
            );
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::unique("cursor"),
            ));
            self.paint_stack(&painter, rect, held);
        }
    }

    fn armor_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Armure");
        for (i, slot) in engine_voxel::ArmorSlot::ALL.iter().enumerate() {
            ui.horizontal(|ui| {
                self.slot_widget(ui, Slot::Armor(i), false);
                ui.label(egui::RichText::new(slot.name()).size(11.0));
            });
        }
        ui.label(
            egui::RichText::new(format!(
                "Protection : {}",
                self.client
                    .view
                    .me
                    .inventory
                    .protection(&self.client.content.items)
            ))
            .size(11.0),
        );
        self.attributes_button(ui);
    }

    fn inventory_panel(&mut self, ui: &mut egui::Ui) {
        use crate::ui_layout::{InventorySection, inventory_sections};
        inventory_sections(ui, |ui, section| match section {
            InventorySection::Armor => self.armor_panel(ui),
            InventorySection::Craft => {
                self.craft_grid(ui, 2);
                self.quests_button(ui);
            }
            InventorySection::Recipes => self.recipe_book(ui, 2),
        });
    }

    /// Le menu d'une machine de mod : les éléments décrits par son script, dessinés ici.
    fn mod_panel(&mut self, ui: &mut egui::Ui) {
        // Le menu est sorti de l'état pendant le dessin (les cases ont besoin de `self`) ;
        // les actions sont mises en file, jamais appliquées pendant le dessin.
        let Some(mut state) = self.client.ui.mod_menu.take() else {
            return;
        };
        let mut events = Vec::new();
        crate::ui_widgets::draw_widgets(
            ui,
            &mut state.menu.widgets,
            &mut events,
            &mut |ui, first, count, columns| {
                let columns = columns.min(crate::ui_layout::slot_columns(ui)).max(1);
                // Un clic de case réveille la machine côté serveur (`SlotClick`).
                crate::ui_layout::slots(ui, count, columns, |ui, index| {
                    self.container_slot(ui, first + index);
                });
            },
        );
        self.client.ui.mod_menu = Some(state);
        self.client.ui.mod_events.extend(events);
    }

    /// Une case d'un conteneur (coffre, four) : ses objets sont dans l'entité du bloc.
    fn container_slot(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let stack = self.shown_stack(index);
        match self.stack_widget(ui, stack, false) {
            Some(press) => self.press_slot(SlotRef::Open(index), stack, press),
            None => false,
        }
    }

    /// Le titre de l'écran d'un wagon (et sa réserve de combustible, pour une locomotive).
    fn cart_title(&self) -> String {
        match self.shown_container().and_then(|c| c.cart) {
            Some((crate::carts::CartKind::Locomotive, burn)) => {
                format!(
                    "{} · {:.0} s de feu",
                    crate::carts::CartKind::Locomotive.title(),
                    burn
                )
            }
            Some((kind, _)) => kind.title().to_string(),
            None => "Wagon".to_string(),
        }
    }

    /// L'inventaire d'un wagon : 27 cases pour un coffre ou une trémie, la case de combustible
    /// d'une locomotive.
    fn cart_panel(&mut self, ui: &mut egui::Ui) {
        let Some(kind) = self
            .shown_container()
            .and_then(|c| c.cart)
            .map(|(kind, _)| kind)
        else {
            // Le wagon n'existe plus (cassé, déchargé) : on revient à l'inventaire.
            self.client.ui.screen = Screen::Inventory;
            return;
        };
        if kind == crate::carts::CartKind::Locomotive {
            ui.label("Combustible : charbon, bois...");
            crate::ui_layout::slots(ui, 1, 1, |ui, index| {
                self.cart_slot(ui, index);
            });
        } else {
            self.storage_panel(ui, false);
        }
    }

    fn cart_slot(&mut self, ui: &mut egui::Ui, index: usize) {
        let stack = self.shown_stack(index);
        if let Some(press) = self.stack_widget(ui, stack, false) {
            self.press_slot(SlotRef::Open(index), stack, press);
        }
    }

    /// Les échanges d'un habitant : une ligne par échange, un bouton pour le faire.
    /// L'échange avec un habitant, à la manière de Minecraft : à gauche la liste des offres, chacune
    /// avec ses objets dessinés (prix, flèche, produit), grisée si l'on ne peut pas payer et marquée
    /// si l'habitant n'en a plus ; à droite l'offre choisie en grandes cases, ce qui manque, et les
    /// boutons d'échange.
    fn trade_panel(&mut self, ui: &mut egui::Ui) {
        // L'habitant, d'après la vue d'écran du serveur ; l'inventaire, d'après la vue du joueur.
        let Some(crate::screen_actions::ScreenView::Trade { key, name, uses }) =
            self.shown_screen().cloned()
        else {
            // L'habitant a disparu (tué, ou trop loin) : on ferme.
            self.close_screen();
            self.client.ui.inventory_open = false;
            return;
        };
        let inventory = self.client.view.me.inventory.as_inventory();
        let trades = self.server.trades.for_mob(&key).to_vec();
        let items = self.client.content.items.clone();
        let item_name = |item: engine_voxel::ItemId| items.get(item).name.clone();
        ui.label(egui::RichText::new(name).strong().size(18.0));
        if trades.is_empty() {
            ui.label("Cet habitant n'a rien à échanger.");
            return;
        }
        let selected = self.client.ui.trade_selected.min(trades.len() - 1);
        let slot = crate::ui_layout::slot_size(ui.ctx()).max(30.0);
        let row_height = slot + 8.0;
        let list_width = slot * 4.6;
        let mut chosen = selected;
        let mut deal = None;
        ui.horizontal_top(|ui| {
            // La liste des offres.
            ui.vertical(|ui| {
                ui.set_width(list_width);
                ui.label(egui::RichText::new("Offres").strong());
                egui::ScrollArea::vertical()
                    .id_salt("trade_offers")
                    .max_height(row_height * 5.0)
                    .show(ui, |ui| {
                        for (i, trade) in trades.iter().enumerate() {
                            let left = uses.get(i).copied().unwrap_or(0);
                            let can = crate::trading::can_trade(&inventory, trade, left);
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(list_width - 4.0, row_height),
                                egui::Sense::click(),
                            );
                            let painter = ui.painter().clone();
                            let fill = if i == selected {
                                egui::Color32::from_rgb(152, 157, 140)
                            } else if response.hovered() {
                                egui::Color32::from_rgb(140, 145, 130)
                            } else {
                                egui::Color32::from_rgb(124, 129, 116)
                            };
                            painter.rect_filled(rect, 0, fill);
                            if i == selected {
                                painter.rect_stroke(
                                    rect,
                                    0,
                                    egui::Stroke::new(2.0, skin::TEXT),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let cell = |x: f32| {
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.left() + x, rect.top() + 4.0),
                                    egui::vec2(slot, slot),
                                )
                            };
                            let (give, get) = (cell(4.0), cell(slot * 2.4));
                            skin::slot(&painter, give, false, false);
                            skin::slot(&painter, get, false, false);
                            self.paint_stack(
                                &painter,
                                give,
                                ItemStack::new(trade.give.0, trade.give.1),
                            );
                            self.paint_stack(
                                &painter,
                                get,
                                ItemStack::new(trade.get.0, trade.get.1),
                            );
                            trade_arrow(
                                &painter,
                                give.right() + slot * 0.2,
                                get.left() - slot * 0.2,
                                give.center().y,
                                if can { skin::INK } else { skin::BORDER },
                            );
                            // Grisée si l'on ne peut pas payer ; une croix rouge si c'est épuisé.
                            if !can {
                                painter.rect_filled(rect, 0, egui::Color32::from_black_alpha(70));
                            }
                            if left == 0 {
                                let at = egui::pos2(rect.right() - slot * 0.45, rect.center().y);
                                let r = slot * 0.22;
                                let stroke =
                                    egui::Stroke::new(3.0, egui::Color32::from_rgb(170, 40, 30));
                                painter.line_segment(
                                    [at - egui::vec2(r, r), at + egui::vec2(r, r)],
                                    stroke,
                                );
                                painter.line_segment(
                                    [at + egui::vec2(-r, r), at + egui::vec2(r, -r)],
                                    stroke,
                                );
                            }
                            let response = response.on_hover_text(format!(
                                "{} × {}  →  {} × {}{}",
                                trade.give.1,
                                item_name(trade.give.0),
                                trade.get.1,
                                item_name(trade.get.0),
                                if left == 0 { "\nÉpuisé" } else { "" }
                            ));
                            if response.clicked() {
                                chosen = i;
                            }
                        }
                    });
            });
            // Un simple espace : un séparateur vertical prendrait toute la hauteur de la fenêtre.
            ui.add_space(12.0);
            // L'offre choisie.
            ui.vertical(|ui| {
                let trade = &trades[selected];
                let left = uses.get(selected).copied().unwrap_or(0);
                let have = inventory.count(trade.give.0);
                let can = crate::trading::can_trade(&inventory, trade, left);
                let big = slot * 1.3;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(big * 3.6, big + 8.0), egui::Sense::hover());
                let painter = ui.painter().clone();
                let give = egui::Rect::from_min_size(
                    rect.min + egui::vec2(0.0, 4.0),
                    egui::vec2(big, big),
                );
                let get = egui::Rect::from_min_size(
                    rect.min + egui::vec2(big * 2.4, 4.0),
                    egui::vec2(big, big),
                );
                skin::slot(&painter, give, false, false);
                skin::slot(&painter, get, can, false);
                self.paint_stack(&painter, give, ItemStack::new(trade.give.0, trade.give.1));
                self.paint_stack(&painter, get, ItemStack::new(trade.get.0, trade.get.1));
                trade_arrow(
                    &painter,
                    give.right() + big * 0.25,
                    get.left() - big * 0.25,
                    give.center().y,
                    if can { skin::INK } else { skin::BORDER },
                );
                ui.add_space(4.0);
                ui.label(format!(
                    "Prix : {} × {}",
                    trade.give.1,
                    item_name(trade.give.0)
                ));
                ui.label(egui::RichText::new(format!("Vous en avez {have}")).color(
                    if have >= trade.give.1 {
                        skin::INK
                    } else {
                        egui::Color32::from_rgb(150, 40, 30)
                    },
                ));
                ui.label(format!(
                    "Contre : {} × {}",
                    trade.get.1,
                    item_name(trade.get.0)
                ));
                ui.label(if left == 0 {
                    egui::RichText::new("Épuisé : l'habitant n'en a plus pour l'instant.")
                        .color(egui::Color32::from_rgb(150, 40, 30))
                } else {
                    egui::RichText::new(format!(
                        "{left} échange{} possible{}",
                        if left > 1 { "s" } else { "" },
                        if left > 1 { "s" } else { "" }
                    ))
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            can,
                            egui::Button::new(egui::RichText::new("Échanger").strong()),
                        )
                        .clicked()
                    {
                        deal = Some((selected, 1));
                    }
                    let most = (have / trade.give.1.max(1)).min(left);
                    if ui
                        .add_enabled(
                            most > 1,
                            egui::Button::new(format!("Tout échanger ({most})")),
                        )
                        .clicked()
                    {
                        deal = Some((selected, most));
                    }
                });
            });
        });
        self.client.ui.trade_selected = chosen;
        if let Some((i, times)) = deal {
            self.send_now(crate::protocol::ClientMsg::Screen(
                crate::protocol::ScreenAction::Trade {
                    offer: i as u8,
                    times,
                },
            ));
        }
    }

    fn chest_panel(&mut self, ui: &mut egui::Ui) {
        self.storage_panel(ui, false);
    }

    fn furnace_panel(&mut self, ui: &mut egui::Ui) {
        // Les jauges et les cases viennent de la vue du conteneur ouvert (`container_view.rs`) ; le
        // serveur réveille le four quand une case change.
        let [burn, cook] = self.shown_container().map_or([0.0; 2], |c| c.furnace);
        crate::ui_layout::furnace(ui, burn, cook, |ui, index| {
            self.container_slot(ui, index);
        });
    }

    /// L'alambic : la fiole d'eau, l'ingrédient, l'avancement et la potion obtenue.
    fn brewing_panel(&mut self, ui: &mut egui::Ui) {
        let progress = self.shown_container().map_or(0.0, |c| c.brewing);
        crate::ui_layout::brewing(ui, progress, |ui, index| {
            self.container_slot(ui, index);
        });
        ui.label("Pomme : soin. Spores : régénération. Carotte : vitesse. Poussière d'ombre : force. Cinabre : résistance au feu.");
    }

    /// L'inspecteur (F3) : mesures, et les entités avec leurs composants modifiables.
    fn draw_inspector(&mut self, ctx: &egui::Context) {
        let eye = self.client.body.eye(1.0);
        let chunk = engine_voxel::World::chunk_containing(eye);
        let streamer_stats = self.stream_stats();
        let target_light = self
            .client
            .target
            .map(|hit| self.view_world().light(hit.block_pos + hit.normal));
        let render = self.client.last_render;
        let debug = self.client.debug.clone();
        crate::ui_layout::window(ctx, "Inspecteur (F3)", 520.0)
            .hscroll(true)
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
                    ui.label(self.server.ecs.len().to_string());
                    ui.end_row();
                    ui.label("Créatures");
                    let mobs = self.server.ecs.resource::<Arc<MobRegistry>>().clone();
                    let mut census: Vec<String> = crate::mobs::census(&self.server.ecs)
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
                    ui.label(self.client.day.clock());
                    ui.end_row();
                });
                ui.collapsing("Systèmes", |ui| {
                    for timing in self.server.schedule.timings() {
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
                        let mobs = self.server.ecs.resource::<Arc<MobRegistry>>().clone();
                        let mut entities: Vec<Entity> = self.server.ecs.entities().collect();
                        entities.sort();
                        for entity in entities {
                            let name = entity_name(&self.server.ecs, entity, &mobs);
                            ui.collapsing(format!("{name} {entity}"), |ui| {
                                inspect_components(&self.server.ecs, entity, ui);
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

/// La flèche d'une offre d'échange (du prix au produit), dessinée : la police n'a pas le glyphe.
fn trade_arrow(painter: &egui::Painter, from: f32, to: f32, y: f32, color: egui::Color32) {
    let head = ((to - from) * 0.4).min(10.0);
    let stroke = egui::Stroke::new(3.0, color);
    painter.line_segment(
        [egui::pos2(from, y), egui::pos2(to - head * 0.5, y)],
        stroke,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(to, y),
            egui::pos2(to - head, y - head * 0.8),
            egui::pos2(to - head, y + head * 0.8),
        ],
        color,
        egui::Stroke::NONE,
    ));
}
