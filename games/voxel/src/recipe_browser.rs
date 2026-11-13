//! L'explorateur de recettes, façon JEI (« Just Enough Items », le mod de Minecraft).
//!
//! Quand un écran d'inventaire est ouvert :
//! - à **droite**, la liste de tous les objets du jeu et des mods, par pages, avec une
//!   recherche en bas (`@forge` : les objets d'un mod ; plusieurs mots : tous doivent
//!   correspondre) ;
//! - à **gauche**, les **favoris** (touche A sur un objet survolé) ;
//! - sur n'importe quel objet survolé (liste, inventaire, coffre, recette) : **R** montre ses
//!   recettes, **U** ses usages. Dans la liste, un clic fait de même (gauche : recettes,
//!   droit : usages).
//!
//! La fenêtre des recettes range les résultats par onglets (artisanat, four, combustible,
//! alambic, machines des mods) ; un clic sur un ingrédient y navigue, Retour arrière revient
//! en arrière. Les groupes (`#planks`) font défiler leurs membres. Devant l'établi (ou la
//! grille de l'inventaire pour une petite recette), « Fabriquer » fabrique la recette avec ce
//! qu'on a (Maj : autant que possible).
//!
//! Le mode **triche** (bouton en bas de la liste) donne les objets d'un clic, comme `/give`.
//!
//! Les recettes elles-mêmes viennent de [`engine_voxel::RecipeIndex`], construit au
//! chargement du contenu.

use engine_render::egui::{self, RichText};
use engine_voxel::{Choice, ItemDef, ItemId, Recipe, RecipeCategory, RecipeIndex, RecipeLayout};

use crate::VoxelGame;
use crate::skin;
use crate::ui::Screen;

/// Ce qu'on cherche sur un objet : comment le fabriquer, ou à quoi il sert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Recipes,
    Uses,
}

/// La fenêtre des recettes : l'objet, l'onglet et la page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Focus {
    pub item: ItemId,
    pub lookup: Lookup,
    pub tab: usize,
    pub page: usize,
}

/// L'état de l'explorateur (dans `GameUi`).
#[derive(Debug, Default)]
pub struct BrowserState {
    /// Le texte de la recherche.
    pub search: String,
    /// La page de la liste d'objets.
    pub page: usize,
    /// La fenêtre des recettes, si elle est ouverte.
    pub view: Option<Focus>,
    /// Les vues précédentes (Retour arrière).
    history: Vec<Focus>,
    /// Le mode triche : un clic dans la liste donne l'objet.
    pub cheat: bool,
    /// L'objet sous la souris à cette image (pour les touches R, U et A).
    pub hovered: Option<ItemId>,
}

/// Au-delà, on oublie les vues les plus anciennes.
const HISTORY: usize = 32;
/// Recettes par page dans la fenêtre des recettes.
const PER_PAGE: usize = 3;
/// La taille des cases de la fenêtre des recettes.
const CELL: f32 = 34.0;

impl BrowserState {
    /// Ouvre les recettes ou les usages d'un objet ; la vue actuelle va dans l'historique.
    pub fn open(&mut self, item: ItemId, lookup: Lookup) {
        if let Some(current) = self.view.take()
            && (current.item, current.lookup) != (item, lookup)
        {
            self.history.push(current);
            if self.history.len() > HISTORY {
                self.history.remove(0);
            }
        }
        self.view = Some(Focus {
            item,
            lookup,
            tab: 0,
            page: 0,
        });
    }

    /// Revient à la vue précédente (ou ferme la fenêtre s'il n'y en a pas).
    pub fn back(&mut self) {
        self.view = self.history.pop();
    }

    pub fn close(&mut self) {
        self.view = None;
        self.history.clear();
    }
}

/// Un objet correspond-il à la recherche ? Chaque mot doit correspondre : au nom ou à
/// l'identifiant (sans tenir compte des accents ni des majuscules), ou au mod (`@forge`).
pub fn matches(def: &ItemDef, query: &str) -> bool {
    let name = fold(&def.name);
    let key = fold(&def.key);
    query
        .split_whitespace()
        .all(|term| match term.strip_prefix('@') {
            Some(wanted) => {
                let wanted = fold(wanted);
                let namespace = key.split(':').next().unwrap_or_default();
                namespace.starts_with(&wanted)
                    || fold(&crate::ui_info::origin(&def.key)).starts_with(&wanted)
            }
            None => {
                let term = fold(term);
                name.contains(&term) || key.contains(&term)
            }
        })
}

/// En minuscules et sans accents : « Établi » et « etabli » se valent.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Le nom d'un onglet.
pub fn category_name(category: &RecipeCategory) -> String {
    match category {
        RecipeCategory::Crafting => "Artisanat".into(),
        RecipeCategory::Smelting => "Four".into(),
        RecipeCategory::Fuel => "Combustible".into(),
        RecipeCategory::Brewing => "Alambic".into(),
        RecipeCategory::Machine(kind) => {
            let (namespace, name) = kind.split_once(':').unwrap_or(("", kind));
            let name = name.replace('_', " ");
            format!(
                "{} · {name}",
                crate::ui_info::origin(&format!("{namespace}:"))
            )
        }
    }
}

/// Un bouton pressé sur une case de l'explorateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    Primary,
    Secondary,
}

/// Ce que la fenêtre des recettes demande, appliqué après le dessin.
enum Action {
    Open(ItemId, Lookup),
    Tab(usize),
    Page(usize),
    Back,
    Close,
    Craft(Recipe, bool),
}

impl VoxelGame {
    /// L'explorateur autour de l'écran d'inventaire, dont `inventory` est le cadre.
    pub(crate) fn draw_recipe_browser(&mut self, ctx: &egui::Context, inventory: egui::Rect) {
        self.draw_item_list(ctx, inventory);
        self.draw_bookmarks(ctx, inventory);
        self.draw_recipe_view(ctx, inventory);
        self.browser_keys(ctx);
    }

    /// Les objets qui correspondent à la recherche, dans l'ordre du registre.
    fn browser_items(&self) -> Vec<ItemId> {
        let query = self.ui.browser.search.trim();
        self.content
            .items
            .iter()
            .filter(|(_, def)| query.is_empty() || matches(def, query))
            .map(|(id, _)| id)
            .collect()
    }

    /// La liste des objets, à droite de l'inventaire.
    fn draw_item_list(&mut self, ctx: &egui::Context, inventory: egui::Rect) {
        let safe = crate::ui_layout::safe_rect(ctx);
        let left = inventory.right() + 10.0;
        let cell = crate::ui_layout::slot_size(ctx).min(36.0);
        let step = cell + crate::ui_layout::SLOT_GAP;
        let margin = skin::panel().total_margin().sum().x;
        let columns = (((safe.right() - left - margin) + crate::ui_layout::SLOT_GAP) / step)
            .floor()
            .min(12.0);
        if columns < 3.0 {
            return; // Pas la place à côté de l'inventaire (fenêtre étroite).
        }
        let columns = columns as usize;
        // La hauteur : l'écran, moins le cadre, la ligne des pages et celle de la recherche.
        let rows = ((safe.height() - margin - 2.0 * 30.0 - 16.0) / step)
            .floor()
            .max(1.0) as usize;
        let per_page = rows * columns;
        let items = self.browser_items();
        let pages = items.len().div_ceil(per_page).max(1);
        self.ui.browser.page = self.ui.browser.page.min(pages - 1);
        let width = columns as f32 * step - crate::ui_layout::SLOT_GAP;
        let mut pressed = None;
        egui::Area::new(egui::Id::unique("recipe_browser_list"))
            .fixed_pos(egui::pos2(left, safe.top()))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                skin::panel().show(ui, |ui| {
                    ui.set_width(width);
                    // Les pages.
                    ui.horizontal(|ui| {
                        if ui.small_button("◀").clicked() {
                            self.ui.browser.page = (self.ui.browser.page + pages - 1) % pages;
                        }
                        ui.label(
                            RichText::new(format!("{} / {pages}", self.ui.browser.page + 1))
                                .color(skin::MUTED),
                        );
                        if ui.small_button("▶").clicked() {
                            self.ui.browser.page = (self.ui.browser.page + 1) % pages;
                        }
                        ui.label(RichText::new(format!("{} objets", items.len())).size(11.0))
                            .on_hover_text(
                                "Clic : recettes · clic droit : usages\nSurvol + R, U : recettes, usages · A : favori",
                            );
                    });
                    // La grille.
                    let start = self.ui.browser.page * per_page;
                    let shown = &items[start.min(items.len())..(start + per_page).min(items.len())];
                    pressed = self.item_grid(ui, shown, columns, cell);
                    // La recherche et la triche.
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.ui.browser.search)
                                .hint_text("Rechercher (@mod)")
                                .desired_width(width - 90.0),
                        );
                        if edit.changed() {
                            self.ui.browser.page = 0;
                        }
                        let label = if self.ui.browser.cheat {
                            RichText::new("Triche").color(skin::GOLD)
                        } else {
                            RichText::new("Triche").color(skin::MUTED)
                        };
                        if ui
                            .selectable_label(self.ui.browser.cheat, label)
                            .on_hover_text(
                                "Mode triche : un clic donne une pile, un clic droit un objet",
                            )
                            .clicked()
                        {
                            self.ui.browser.cheat = !self.ui.browser.cheat;
                        }
                    });
                });
            });
        if let Some((item, press)) = pressed {
            self.list_press(item, press);
        }
    }

    /// Les favoris, à gauche de l'inventaire.
    fn draw_bookmarks(&mut self, ctx: &egui::Context, inventory: egui::Rect) {
        let bookmarks: Vec<ItemId> = self
            .settings
            .bookmarks
            .iter()
            .filter_map(|key| self.content.items.id(key))
            .collect();
        if bookmarks.is_empty() {
            return;
        }
        let safe = crate::ui_layout::safe_rect(ctx);
        let cell = crate::ui_layout::slot_size(ctx).min(36.0);
        let step = cell + crate::ui_layout::SLOT_GAP;
        let margin = skin::panel().total_margin().sum().x;
        let room = inventory.left() - 10.0 - safe.left() - margin;
        let columns = ((room + crate::ui_layout::SLOT_GAP) / step)
            .floor()
            .min(6.0);
        if columns < 1.0 {
            return;
        }
        let columns = (columns as usize).min(bookmarks.len());
        // Assez large pour le titre, même avec un seul favori.
        let width = (columns as f32 * step - crate::ui_layout::SLOT_GAP).max(64.0);
        let mut pressed = None;
        egui::Area::new(egui::Id::unique("recipe_browser_bookmarks"))
            .fixed_pos(egui::pos2(
                inventory.left() - 10.0 - width - margin,
                safe.top(),
            ))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                skin::panel().show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(RichText::new("Favoris").strong())
                        .on_hover_text("A sur un objet survolé : ajouter ou retirer");
                    pressed = self.item_grid(ui, &bookmarks, columns, cell);
                });
            });
        if let Some((item, press)) = pressed {
            self.list_press(item, press);
        }
    }

    /// Une grille de cases d'objets ; renvoie la case pressée.
    fn item_grid(
        &mut self,
        ui: &mut egui::Ui,
        items: &[ItemId],
        columns: usize,
        cell: f32,
    ) -> Option<(ItemId, Press)> {
        let mut pressed = None;
        ui.spacing_mut().item_spacing = egui::Vec2::splat(crate::ui_layout::SLOT_GAP);
        for row in items.chunks(columns) {
            ui.horizontal(|ui| {
                for &item in row {
                    if let Some(press) = self.item_cell(ui, item, 1, cell, None) {
                        pressed = Some((item, press));
                    }
                }
            });
        }
        pressed
    }

    /// Un clic dans la liste ou les favoris : donner (triche) ou ouvrir les recettes.
    fn list_press(&mut self, item: ItemId, press: Press) {
        if self.ui.browser.cheat {
            let count = match press {
                Press::Primary => self.content.items.get(item).max_stack.max(1),
                Press::Secondary => 1,
            };
            let left = self.inventory.add(item, count, &self.content.items);
            if left > 0 {
                self.ui.toast = Some(("Inventaire plein".into(), std::time::Instant::now()));
            }
            return;
        }
        let lookup = match press {
            Press::Primary => Lookup::Recipes,
            Press::Secondary => Lookup::Uses,
        };
        self.ui.browser.open(item, lookup);
    }

    /// Une case d'objet : son icône, son nombre, sa bulle d'aide. `alternatives` : le
    /// nombre d'objets au choix pour un groupe (affiché dans la bulle).
    fn item_cell(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        count: u32,
        size: f32,
        alternatives: Option<usize>,
    ) -> Option<Press> {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
        let painter = ui.painter().clone();
        skin::slot(&painter, rect, false, response.hovered());
        let icon = self.ui.icon(
            item,
            &self.content.items,
            &self.registry,
            &self.block_textures,
        );
        skin::stack(&painter, rect, icon, count, None, false);
        if response.hovered() {
            self.ui.browser.hovered = Some(item);
        }
        let press = if response.clicked() {
            Some(Press::Primary)
        } else if response.secondary_clicked() {
            Some(Press::Secondary)
        } else {
            None
        };
        let mut card = crate::ui_info::item(self.content.items.get(item), count, 0);
        if let Some(n) = alternatives.filter(|&n| n > 1) {
            card.lines
                .insert(0, (format!("Au choix parmi {n} objets"), skin::GOLD));
        }
        card.lines
            .push(("R : recettes · U : usages · A : favori".into(), skin::MUTED));
        crate::ui_info::hover(response, &card);
        press
    }

    /// Une case vide de grille d'artisanat.
    fn empty_cell(ui: &mut egui::Ui, size: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        skin::slot(ui.painter(), rect, false, false);
    }

    /// Une case d'ingrédient : les membres d'un groupe défilent, un par seconde.
    fn choice_cell(&mut self, ui: &mut egui::Ui, choice: &Choice, actions: &mut Vec<Action>) {
        let Some(&first) = choice.items.first() else {
            Self::empty_cell(ui, CELL);
            return;
        };
        let time = ui.input(|i| i.time);
        let item = choice
            .items
            .get(time as usize % choice.items.len())
            .copied()
            .unwrap_or(first);
        if let Some(press) = self.item_cell(ui, item, choice.count, CELL, Some(choice.items.len()))
        {
            actions.push(open_action(item, press));
        }
    }

    /// La fenêtre des recettes ou des usages d'un objet.
    fn draw_recipe_view(&mut self, ctx: &egui::Context, inventory: egui::Rect) {
        let Some(focus) = self.ui.browser.view else {
            return;
        };
        let index = self.content.recipe_index.clone();
        let list = match focus.lookup {
            Lookup::Recipes => index.recipes_for(focus.item),
            Lookup::Uses => index.uses_of(focus.item),
        };
        let groups = RecipeIndex::by_category(&list);
        let name = self.content.items.get(focus.item).name.clone();
        let title = match focus.lookup {
            Lookup::Recipes => format!("Recettes : {name}"),
            Lookup::Uses => format!("Usages : {name}"),
        };
        let has_history = !self.ui.browser.history.is_empty();
        let mut actions: Vec<Action> = Vec::new();
        let safe = crate::ui_layout::safe_rect(ctx);
        // Par-dessus l'inventaire, à sa largeur : la liste et les favoris restent visibles.
        crate::ui_layout::window(ctx, &title, inventory.width())
            .id(egui::Id::unique("recipe_view"))
            .order(egui::Order::Foreground)
            .title_bar(false)
            .fixed_pos(egui::pos2(inventory.left(), safe.top() + 40.0))
            .show(ctx, |ui| {
                // L'en-tête : l'objet, Recettes ↔ Usages, Retour, Fermer.
                ui.horizontal(|ui| {
                    if let Some(press) = self.item_cell(ui, focus.item, 1, CELL, None) {
                        actions.push(open_action(focus.item, press));
                    }
                    ui.label(RichText::new(&title).strong().size(16.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").on_hover_text("Fermer · Échap").clicked() {
                            actions.push(Action::Close);
                        }
                        if has_history
                            && ui
                                .button("←")
                                .on_hover_text("Retour · Retour arrière")
                                .clicked()
                        {
                            actions.push(Action::Back);
                        }
                        let other = match focus.lookup {
                            Lookup::Recipes => ("Usages", Lookup::Uses),
                            Lookup::Uses => ("Recettes", Lookup::Recipes),
                        };
                        if ui.button(other.0).clicked() {
                            actions.push(Action::Open(focus.item, other.1));
                        }
                    });
                });
                ui.separator();
                if groups.is_empty() {
                    ui.label(match focus.lookup {
                        Lookup::Recipes => {
                            "Aucune recette ne fabrique cet objet : il se trouve dans le monde."
                        }
                        Lookup::Uses => "Cet objet ne sert dans aucune recette.",
                    });
                    return;
                }
                // Les onglets.
                let tab = focus.tab.min(groups.len() - 1);
                ui.horizontal_wrapped(|ui| {
                    for (i, (category, views)) in groups.iter().enumerate() {
                        let label = format!("{} ({})", category_name(category), views.len());
                        if ui.selectable_label(i == tab, label).clicked() {
                            actions.push(Action::Tab(i));
                        }
                    }
                });
                ui.separator();
                let views = &groups[tab].1;
                let pages = views.len().div_ceil(PER_PAGE).max(1);
                let page = focus.page.min(pages - 1);
                for view in views.iter().skip(page * PER_PAGE).take(PER_PAGE) {
                    self.draw_recipe(ui, view, &mut actions);
                    ui.separator();
                }
                if pages > 1 {
                    ui.horizontal(|ui| {
                        if ui.small_button("◀").clicked() {
                            actions.push(Action::Page((page + pages - 1) % pages));
                        }
                        ui.label(format!("{} / {pages}", page + 1));
                        if ui.small_button("▶").clicked() {
                            actions.push(Action::Page((page + 1) % pages));
                        }
                    });
                }
            });
        for action in actions {
            self.apply_view_action(action);
        }
    }

    /// Une recette : ses entrées, une flèche, ses produits, et ce qu'il faut savoir.
    fn draw_recipe(
        &mut self,
        ui: &mut egui::Ui,
        view: &engine_voxel::RecipeView,
        actions: &mut Vec<Action>,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::splat(crate::ui_layout::SLOT_GAP);
            let info = match &view.layout {
                RecipeLayout::Grid {
                    width,
                    height,
                    cells,
                    shapeless,
                    grid_size,
                } => {
                    ui.vertical(|ui| {
                        for y in 0..*height {
                            ui.horizontal(|ui| {
                                for x in 0..*width {
                                    match &cells[y * width + x] {
                                        Some(choice) => self.choice_cell(ui, choice, actions),
                                        None => Self::empty_cell(ui, CELL),
                                    }
                                }
                            });
                        }
                    });
                    let place = if *grid_size > 2 {
                        "Établi"
                    } else {
                        "Inventaire ou établi"
                    };
                    if *shapeless {
                        format!("{place}\nSans forme")
                    } else {
                        place.to_string()
                    }
                }
                RecipeLayout::Smelt { input, seconds } => {
                    self.choice_cell(ui, input, actions);
                    ui.label(RichText::new("🔥").size(18.0));
                    format!("Four · {}", duration(*seconds))
                }
                RecipeLayout::Fuel { fuel, seconds } => {
                    self.choice_cell(ui, fuel, actions);
                    ui.label(RichText::new("🔥").size(18.0));
                    format!("Brûle {}", duration(*seconds))
                }
                RecipeLayout::Brew {
                    input,
                    reagent,
                    seconds,
                } => {
                    self.choice_cell(ui, input, actions);
                    ui.label("+");
                    self.choice_cell(ui, reagent, actions);
                    format!("Alambic · {}", duration(*seconds))
                }
                RecipeLayout::Machine {
                    inputs,
                    seconds,
                    energy,
                } => {
                    for choice in inputs {
                        self.choice_cell(ui, choice, actions);
                    }
                    let mut text = category_name(&view.category);
                    if *seconds > 0.0 {
                        text.push_str(&format!("\n{}", duration(*seconds)));
                    }
                    if *energy > 0 {
                        text.push_str(&format!(" · {energy} énergie"));
                    }
                    text
                }
            };
            if !view.outputs.is_empty() {
                ui.label(RichText::new("→").size(20.0));
                for &(item, count) in &view.outputs {
                    if let Some(press) = self.item_cell(ui, item, count, CELL, None) {
                        actions.push(open_action(item, press));
                    }
                }
            }
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(info).size(12.0).color(skin::MUTED));
                if let Some(recipe) = &view.crafting
                    && self.can_use_grid(recipe)
                {
                    let ready = self.inventory.can_craft(recipe);
                    let button = ui
                        .add_enabled(ready, egui::Button::new("Fabriquer"))
                        .on_hover_text("Maj : autant que possible")
                        .on_disabled_hover_text("Il manque des ingrédients");
                    if button.clicked() {
                        let all = ui.input(|i| i.modifiers.shift);
                        actions.push(Action::Craft(recipe.clone(), all));
                    }
                }
            });
        });
    }

    /// L'écran ouvert permet-il cette recette ? (L'établi pour toutes, l'inventaire pour
    /// celles qui tiennent en 2 × 2.)
    fn can_use_grid(&self, recipe: &Recipe) -> bool {
        match self.ui.screen {
            Screen::Workbench => true,
            Screen::Inventory => recipe.grid_size() <= 2,
            _ => false,
        }
    }

    fn apply_view_action(&mut self, action: Action) {
        let browser = &mut self.ui.browser;
        match action {
            Action::Open(item, lookup) => browser.open(item, lookup),
            Action::Tab(tab) => {
                if let Some(view) = &mut browser.view {
                    view.tab = tab;
                    view.page = 0;
                }
            }
            Action::Page(page) => {
                if let Some(view) = &mut browser.view {
                    view.page = page;
                }
            }
            Action::Back => browser.back(),
            Action::Close => browser.close(),
            Action::Craft(recipe, all) => {
                // Autant que possible : jusqu'à une pile du produit.
                let (item, _) = recipe.result();
                let limit = if all {
                    self.content.items.get(item).max_stack.max(1)
                } else {
                    1
                };
                for _ in 0..limit {
                    if self
                        .inventory
                        .craft_from_book(&recipe, &self.content.items)
                        .is_none()
                    {
                        break;
                    }
                }
            }
        }
    }

    /// Les touches de l'explorateur, sur l'objet survolé : R, U, A ; Retour arrière.
    fn browser_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return; // On tape dans la recherche.
        }
        let (recipes, uses, bookmark, back) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::U),
                i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Backspace),
            )
        });
        if let Some(item) = self.ui.browser.hovered {
            if recipes {
                self.ui.browser.open(item, Lookup::Recipes);
            } else if uses {
                self.ui.browser.open(item, Lookup::Uses);
            } else if bookmark {
                self.toggle_bookmark(item);
            }
        }
        if back && self.ui.browser.view.is_some() {
            self.ui.browser.back();
        }
    }

    /// Ajoute ou retire un favori, et l'enregistre dans les options.
    fn toggle_bookmark(&mut self, item: ItemId) {
        let key = self.content.items.get(item).key.clone();
        let bookmarks = &mut self.settings.bookmarks;
        match bookmarks.iter().position(|k| *k == key) {
            Some(at) => {
                bookmarks.remove(at);
            }
            None => bookmarks.push(key),
        }
        self.settings.save(&crate::project_root());
    }
}

fn open_action(item: ItemId, press: Press) -> Action {
    Action::Open(
        item,
        match press {
            Press::Primary => Lookup::Recipes,
            Press::Secondary => Lookup::Uses,
        },
    )
}

/// « 10 s », « 1,5 s ».
fn duration(seconds: f32) -> String {
    if (seconds - seconds.round()).abs() < 0.05 {
        format!("{} s", seconds.round())
    } else {
        format!("{seconds:.1} s").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> engine_voxel::ItemRegistry {
        let blocks = engine_voxel::BlockRegistry::builtin();
        engine_voxel::ItemRegistry::builtin(&blocks).unwrap()
    }

    #[test]
    fn search_ignores_case_and_accents_and_filters_by_mod() {
        let items = items();
        let def = |key: &str| items.get(items.id(key).unwrap());
        assert!(matches(def("core:workbench"), "etabli"));
        assert!(matches(def("core:workbench"), "ÉTABLI"));
        assert!(matches(def("core:oak_planks"), "planches chêne"));
        assert!(!matches(def("core:oak_planks"), "planches bouleau"));
        assert!(matches(def("core:stone"), "@core"));
        assert!(matches(def("core:stone"), "@jeu"));
        assert!(!matches(def("core:stone"), "@forge"));
        // L'identifiant aussi : « glowstone ».
        assert!(matches(def("core:glowstone"), "glowstone"));
    }

    #[test]
    fn the_view_history_goes_back_and_ignores_repeats() {
        let mut browser = BrowserState::default();
        let (a, b) = (ItemId(1), ItemId(2));
        browser.open(a, Lookup::Recipes);
        browser.open(a, Lookup::Recipes);
        browser.open(b, Lookup::Uses);
        assert_eq!(browser.view.map(|v| v.item), Some(b));
        browser.back();
        assert_eq!(
            browser.view.map(|v| (v.item, v.lookup)),
            Some((a, Lookup::Recipes))
        );
        browser.back();
        assert!(browser.view.is_none());
    }

    #[test]
    fn categories_have_readable_names() {
        assert_eq!(category_name(&RecipeCategory::Smelting), "Four");
        assert_eq!(
            category_name(&RecipeCategory::Machine("forge:anvil_plate".into())),
            "Forge · anvil plate"
        );
        assert_eq!(duration(10.0), "10 s");
        assert_eq!(duration(1.5), "1,5 s");
    }
}
