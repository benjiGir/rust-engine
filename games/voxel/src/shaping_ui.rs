//! L'établi de taille : on y met un bloc, on choisit une forme, on récupère des pièces.
//!
//! La case d'entrée est la première case de la grille d'artisanat de l'inventaire : ce qu'elle
//! contient revient dans l'inventaire (ou tombe à terre) à la fermeture de l'écran, comme pour
//! l'établi ordinaire, donc rien ne se perd.
//!
//! - **Tailler** : n'importe quel cube plein (bois, pierres, verre, laine, minerais, blocs de mods : voir
//!   `engine_voxel::shaped::is_shapeable`) donne, au choix, des pièces des vingt-trois formes de
//!   `PieceKind`, rangées par familles dans une liste qui défile (couches, pentes, escaliers, panneaux, formes raccordées). Un clic taille un bloc, Maj + clic toute la pile. Un
//!   bloc refusé dit pourquoi.
//! - **Reconstituer** : quand la case contient assez de pièces d'un même matériau (16 couches, 2 dalles, 8
//!   microblocs...), un bouton les rend en bloc plein.
//!
//! Le matériau d'une pièce est un numéro de la palette du monde (voir `shape_materials.rs`).

use engine_render::egui;
use engine_voxel::shaped::{self, Family, PieceKind};

use crate::VoxelGame;
use crate::inventory::{ItemStack, Slot};
use crate::shape_materials;

/// L'établi de taille.
pub const TABLE: &str = "core:shaping_table";

/// Ce que contient la case d'entrée.
enum Input {
    Empty,
    /// Un bloc à tailler : son identifiant et son nom.
    Block(String, String),
    /// Un bloc qui ne se taille pas, et pourquoi.
    Refused(String, &'static str),
    /// Des pièces : la sorte, le matériau, combien.
    Pieces(PieceKind, String, u32),
}

impl VoxelGame {
    fn shaping_input(&self) -> Input {
        let Some(stack) = self.players.me().inventory.craft[0] else {
            return Input::Empty;
        };
        let def = self.content.items.get(stack.item);
        if let Some(kind) = shape_materials::kind_of(def) {
            return match shape_materials::key_of(stack.damage) {
                Some(material) => Input::Pieces(kind, material, stack.count),
                None => Input::Refused(def.name.clone(), "cette pièce n'a pas de matière"),
            };
        }
        let Some(block) = def.block else {
            return Input::Refused(def.name.clone(), "ce n'est pas un bloc");
        };
        match shaped::why_not_shapeable(self.registry.def(block)) {
            None => Input::Block(def.key.clone(), def.name.clone()),
            Some(reason) => Input::Refused(def.name.clone(), reason),
        }
    }

    /// L'écran de l'établi de taille.
    pub(crate) fn shaping_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Mets un bloc dans la case, puis choisis une forme. Maj + clic : toute la pile.");
        ui.add_space(4.0);
        let input = self.shaping_input();
        ui.horizontal_top(|ui| {
            self.slot_widget(ui, Slot::Craft(0), false);
            ui.vertical(|ui| match &input {
                Input::Empty => {
                    ui.label(
                        egui::RichText::new(
                            "Un bloc de bois, de pierre, de verre, de laine... ou d'un mod.",
                        )
                        .weak(),
                    );
                }
                Input::Block(_, name) => {
                    ui.strong(name);
                    ui.label("Se taille.");
                }
                Input::Refused(name, reason) => {
                    ui.strong(name);
                    ui.label(
                        egui::RichText::new(format!("Ne se taille pas : {reason}."))
                            .color(crate::skin::RED),
                    );
                }
                Input::Pieces(kind, material, count) => {
                    let name = shape_materials::material_name(&self.registry, material);
                    ui.strong(format!("{} × {count}", kind.name()));
                    ui.label(format!("En {}.", name.to_lowercase()));
                }
            });
        });
        ui.add_space(6.0);
        match input {
            Input::Block(key, _) => self.shape_buttons(ui, &key),
            Input::Pieces(kind, material, count) => self.rebuild_button(ui, kind, &material, count),
            _ => {}
        }
    }

    /// Les formes, rangées par familles, dessinées avec le matériau, chacune avec son rendement. La
    /// liste défile quand la fenêtre est petite : les entrées passent à la ligne, donc 320 px de large
    /// suffisent.
    fn shape_buttons(&mut self, ui: &mut egui::Ui, material: &str) {
        let number = shape_materials::number_of(material);
        let height = (crate::ui_layout::safe_rect(ui.ctx()).height() * 0.42).clamp(120.0, 420.0);
        egui::ScrollArea::vertical()
            .id_salt("shaping_shapes")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for family in Family::ALL {
                    ui.label(egui::RichText::new(family.name()).strong());
                    ui.horizontal_wrapped(|ui| {
                        for kind in PieceKind::of_family(family) {
                            self.shape_entry(ui, kind, number);
                        }
                    });
                    ui.add_space(4.0);
                }
            });
    }

    /// Une entrée de la grille : l'icône de la pièce et son rendement ; un clic taille.
    fn shape_entry(&mut self, ui: &mut egui::Ui, kind: PieceKind, number: u32) {
        let Some(item) = self.content.items.id(kind.key()) else {
            return;
        };
        let preview = ItemStack {
            item,
            count: kind.per_block().min(999),
            damage: number,
        };
        ui.vertical(|ui| {
            ui.set_width(crate::ui_layout::slot_size(ui.ctx()) + 30.0);
            if self.stack_widget(ui, Some(preview), false).is_some() {
                let all = ui.input(|i| i.modifiers.shift);
                if let Err(why) = self.carve(Slot::Craft(0), kind, all) {
                    self.toast(&why);
                }
            }
            ui.label(
                egui::RichText::new(format!("{}\n×{}", kind.name(), kind.per_block())).small(),
            );
        });
    }

    /// « Reconstituer » : des pièces d'un même matériau rendent le bloc plein.
    fn rebuild_button(&mut self, ui: &mut egui::Ui, kind: PieceKind, material: &str, count: u32) {
        let blocks = count / kind.per_block();
        ui.label("Reconstituer");
        if blocks == 0 {
            ui.label(
                egui::RichText::new(format!(
                    "Il en faut {} pour refaire un bloc ({count} ici).",
                    kind.per_block()
                ))
                .weak(),
            );
            return;
        }
        let name = shape_materials::material_name(&self.registry, material);
        let all = ui.input(|i| i.modifiers.shift);
        let wanted = if all { blocks } else { 1 };
        let clicked = ui
            .button(format!("{} bloc(s) : {}", wanted, name.to_lowercase()))
            .clicked();
        if clicked && let Err(why) = self.rebuild(Slot::Craft(0), all) {
            self.toast(&why);
        }
    }

    /// Taille des blocs de `slot` en pièces de la sorte `kind` : un bloc, ou toute la pile. `Err`
    /// dit pourquoi rien n'est fait.
    pub(crate) fn carve(&mut self, slot: Slot, kind: PieceKind, all: bool) -> Result<u32, String> {
        let Some(stack) = *self.players.me_mut().inventory.slot_mut(slot) else {
            return Err("rien à tailler".into());
        };
        let def = self.content.items.get(stack.item);
        let key = def.key.clone();
        let block = def
            .block
            .ok_or_else(|| format!("« {} » n'est pas un bloc", def.name))?;
        if let Some(reason) = shaped::why_not_shapeable(self.registry.def(block)) {
            return Err(format!("« {} » ne se taille pas : {reason}", def.name));
        }
        let piece = self.content.items.id(kind.key()).ok_or("pièce inconnue")?;
        let blocks = if all { stack.count } else { 1 };
        let made = ItemStack {
            item: piece,
            count: blocks * kind.per_block(),
            damage: shape_materials::number_of(&key),
        };
        *self.players.me_mut().inventory.slot_mut(slot) =
            (stack.count > blocks).then_some(ItemStack {
                count: stack.count - blocks,
                ..stack
            });
        self.give_or_drop(made);
        if let Some(audio) = &mut self.audio {
            audio.play("core:place_hard");
        }
        Ok(made.count)
    }

    /// Refait des blocs pleins avec les pièces de `slot` : un bloc, ou tous ceux que la pile permet.
    pub(crate) fn rebuild(&mut self, slot: Slot, all: bool) -> Result<u32, String> {
        let Some(stack) = *self.players.me_mut().inventory.slot_mut(slot) else {
            return Err("rien à reconstituer".into());
        };
        let kind = shape_materials::kind_of(self.content.items.get(stack.item))
            .ok_or("ce ne sont pas des pièces")?;
        let material = shape_materials::key_of(stack.damage).ok_or("pièces sans matière")?;
        let item = self
            .content
            .items
            .id(&material)
            .ok_or_else(|| format!("« {material} » n'existe plus"))?;
        let possible = stack.count / kind.per_block();
        if possible == 0 {
            return Err(format!("il faut {} pièces pour un bloc", kind.per_block()));
        }
        let blocks = if all { possible } else { 1 };
        let left = stack.count - blocks * kind.per_block();
        *self.players.me_mut().inventory.slot_mut(slot) = (left > 0).then_some(ItemStack {
            count: left,
            ..stack
        });
        self.give_or_drop(ItemStack::new(item, blocks));
        Ok(blocks)
    }
}
