//! Les écrans de l'enchantement avancé : la table (stats, indices, lots payés en points d'expérience),
//! l'enclume simple et la bibliothèque. Les règles sont dans `enchant_table.rs`, `enchant_books.rs`
//! et `enchant_library.rs` ; ici, la lecture du monde autour de la table et le dessin.

#[allow(unused_imports)]
use crate::enchant_ui_server::*;
use std::collections::HashMap;

use engine_render::egui;
use engine_voxel::ItemStack as BlockStack;
use glam::IVec3;

use crate::VoxelGame;
use crate::enchant_books::{self, ENCHANTED_BOOK};
use crate::enchant_library::{self, Library};
use crate::enchant_table::{self, TableStats};
use crate::enchanting::{self, Enchantment};
use crate::protocol::ScreenAction;

/// La couleur d'un nom d'enchantement sur le cadre clair de l'inventaire.
const ENCHANT_TEXT: egui::Color32 = egui::Color32::from_rgb(98, 54, 158);

/// Ce que gardent les écrans d'une image à l'autre.
#[derive(Default)]
pub struct EnchantUi {
    /// La recherche de la bibliothèque, son filtre « ce qui va sur l'objet en main », et le niveau
    /// choisi pour chaque sorte.
    pub search: String,
    pub only_fitting: bool,
    pub picks: HashMap<u8, u8>,
}

/// Un pourcentage : « 15 % ».
fn percent(value: f32) -> String {
    format!("{value:.0} %")
}

impl VoxelGame {
    /// Les stats de la table telles que la fenêtre les voit (la réplique du monde).
    fn shown_table_stats(&self, pos: IVec3) -> TableStats {
        table_stats_in(&self.client.registry, self.view_world(), pos)
    }
}

impl VoxelGame {
    /// La table d'enchantement : ses stats, et trois lots pour l'objet (ou le livre) en main, payés
    /// en points d'expérience et en saphirs.
    pub(crate) fn enchant_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        // Ce que la fenêtre voit : la réplique (étagères) et les vues du joueur ; le serveur
        // recalcule les lots quand on en achète un (`ScreenAction::EnchantBuy`).
        let stats = self.shown_table_stats(pos);
        stats_grid(ui, &stats);
        ui.add_space(4.0);
        ui.separator();
        // Le reste défile : l'inventaire du joueur, dessous, reste toujours visible.
        egui::ScrollArea::vertical()
            .id_salt("enchant_table")
            .max_height(scroll_height(ui))
            .show(ui, |ui| self.enchant_body(ui, &stats));
        self.book_help(ui);
    }

    /// L'objet en main et les lots proposés, avec leur prix.
    fn enchant_body(&mut self, ui: &mut egui::Ui, stats: &TableStats) {
        let Some(vitals) = self.client.view.me.vitals.clone() else {
            return;
        };
        let Some(stack) = self.client.view.me.inventory.selected_stack() else {
            ui.label("Tiens un outil, une armure, un livre ou un tome dans la barre d'objets, puis reviens.");
            return;
        };
        let items = self.client.content.items.clone();
        let def = items.get(stack.item);
        let owned = enchanting::labels(stack.damage);
        if !owned.is_empty() {
            ui.label(format!("Enchantements : {}", owned.join(", ")));
        }
        let Some(subject) = crate::enchant_ui_server::table_subject(&items, &stack) else {
            ui.label("Un livre enchanté s'applique sur un objet : voir l'aide plus bas.");
            return;
        };
        let data = crate::adventure_data::current();
        let Some(list) = enchant_table::offers(
            &data,
            subject,
            &def.key,
            stack.damage,
            vitals.enchant_seed,
            stats,
        ) else {
            ui.label(if owned.is_empty() {
                "Cet objet ne peut pas être enchanté."
            } else {
                "Cet objet ne peut plus recevoir d'enchantement."
            });
            return;
        };
        let gem = items.id("core:sapphire");
        let have_gems = gem.map_or(0, |g| self.client.view.me.inventory.as_inventory().count(g));
        let points = vitals.points;
        ui.label(format!(
            "{} · {points} pts d'XP (niveau {}) · {}",
            def.name,
            vitals.level,
            sapphires(have_gems)
        ));
        let mut chosen = None;
        for (index, offer) in list.into_iter().enumerate() {
            let affordable = points >= offer.points && have_gems >= offer.gems;
            let text = format!(
                "Niv. {} · {} · {} pts, {}",
                offer.level,
                compact_preview(&offer.preview()),
                offer.points,
                sapphires(offer.gems)
            );
            if ui
                .add_enabled(affordable, egui::Button::new(text).wrap())
                .clicked()
            {
                chosen = Some(index);
            }
        }
        if let Some(index) = chosen {
            self.send_now(crate::protocol::ClientMsg::Screen(
                crate::protocol::ScreenAction::EnchantBuy { offer: index as u8 },
            ));
        }
    }

    /// L'aide des livres et des indices, repliée sous la table pour laisser la place à
    /// l'inventaire.
    fn book_help(&self, ui: &mut egui::Ui) {
        ui.collapsing("Aide · livres, tomes et indices", |ui| {
            ui.label(
                "Un livre enchanté ou un tome d'extraction pris à la souris, puis un clic sur un objet de l'inventaire : \
                 le livre apporte ses enchantements, le tome d'extraction les retire sur un livre vierge. \
                 Les indices disent combien d'enchantements chaque lot montre ; les autres sont des « ? ».",
            );
        });
    }

    /// L'enclume simple : le geste du livre et du tome d'extraction, sans stats.
    pub(crate) fn anvil_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("Prends à la souris un livre enchanté ou un tome d'extraction (clic sur sa case, dans l'inventaire), puis clique sur l'objet à enchanter ou à vider.");
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Livre enchanté : chaque enchantement qui convient est ajouté, ou monte d'un niveau s'il est déjà au même niveau. \
                 Tome d'extraction : retire les enchantements de l'objet sur un livre vierge pris dans l'inventaire.",
            )
            .weak(),
        );
        if let Some(held) = self.client.view.me.inventory.cursor {
            let items = self.client.content.items.clone();
            ui.separator();
            ui.label(format!("À la souris : {}", items.get(held.item).name));
            for label in enchanting::labels(held.damage) {
                ui.label(egui::RichText::new(label).color(ENCHANT_TEXT));
            }
        }
    }

    // ----- La bibliothèque -----

    /// La bibliothèque telle que la fenêtre la voit (la réplique).
    fn shown_library(&self, pos: IVec3) -> Option<(Library, u8)> {
        library_in(&self.client.registry, self.view_streamer(), pos)
    }
}

impl VoxelGame {
    /// La bibliothèque : dépôt, recherche, filtre par objet, extraction au niveau choisi.
    pub(crate) fn library_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        let items = self.client.content.items.clone();
        // La bibliothèque telle que la réplique la voit (le serveur range la case d'entrée tant que
        // l'écran est ouvert) ; l'inventaire, d'après la vue du joueur.
        let Some((library, max)) = self.shown_library(pos) else {
            ui.label("Cette bibliothèque a disparu.");
            return;
        };
        let inventory = self.client.view.me.inventory.as_inventory();
        let mut action = None;
        ui.label(
            egui::RichText::new(format!(
                "Niveau n = 2^(n−1) points · niveau {max} au plus · {} points par sorte au plus.",
                Library::capacity(max)
            ))
            .weak(),
        );
        let book_id = items.id(ENCHANTED_BOOK);
        let blank = items.id(enchant_books::BOOK);
        // Le dépôt : le livre à la souris, ou tous ceux de l'inventaire.
        ui.horizontal_wrapped(|ui| {
            let cursor_book = inventory
                .cursor
                .filter(|s| Some(s.item) == book_id && enchanting::is_enchanted(s.damage));
            if ui
                .add_enabled(
                    cursor_book.is_some(),
                    egui::Button::new("Déposer le livre de la souris"),
                )
                .clicked()
            {
                action = Some(ScreenAction::LibraryDepositCursor);
            }
            if ui.button("Déposer tous les livres enchantés").clicked() {
                action = Some(ScreenAction::LibraryDepositAll);
            }
        });
        // La case de sortie : ce que le bloc a refusé.
        if self.library_output(ui, pos) {
            action = Some(ScreenAction::LibraryTakeOutput);
        }
        ui.separator();
        // La recherche et le filtre par objet.
        ui.horizontal(|ui| {
            ui.label("Recherche");
            ui.add(
                egui::TextEdit::singleline(&mut self.client.enchant_ui.search)
                    .desired_width(160.0)
                    .hint_text("nom de l'enchantement"),
            );
            ui.checkbox(
                &mut self.client.enchant_ui.only_fitting,
                "Va sur l'objet en main",
            );
        });
        let held = inventory.selected_stack();
        let needle = self.client.enchant_ui.search.trim().to_lowercase();
        let mut rows: Vec<(Enchantment, u32, u8)> = library.entries();
        rows.retain(|(kind, _, _)| {
            let name = kind.name().to_lowercase();
            let fits = !self.client.enchant_ui.only_fitting
                || held.is_some_and(|h| kind.applies_to(items.get(h.item)));
            fits && (needle.is_empty() || name.contains(&needle))
        });
        if rows.is_empty() {
            ui.label(if library.entries().is_empty() {
                "La bibliothèque est vide : dépose des livres enchantés."
            } else {
                "Aucun enchantement ne correspond."
            });
        }
        let mut extract = None;
        // La liste défile : l'inventaire du joueur, dessous, reste toujours visible.
        egui::ScrollArea::vertical()
            .id_salt("enchant_library")
            .max_height(scroll_height(ui))
            .show(ui, |ui| {
                for (kind, points, best) in rows {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new(kind.name())
                                .color(ENCHANT_TEXT)
                                .strong(),
                        );
                        ui.label(format!(
                            "{points} pts — niveau déposé le plus haut : {}",
                            enchanting::roman(best)
                        ));
                        let reachable = library.extractable(kind);
                        let pick = self
                            .client
                            .enchant_ui
                            .picks
                            .entry(kind.id())
                            .or_insert(reachable.max(1));
                        *pick = (*pick).clamp(1, best.max(1));
                        ui.add(
                            egui::DragValue::new(pick)
                                .range(1..=best.max(1))
                                .prefix("niv. "),
                        );
                        let cost = enchant_library::points_of_level(*pick);
                        let ready =
                            *pick <= reachable && blank.is_some_and(|b| inventory.count(b) > 0);
                        if ui
                            .add_enabled(ready, egui::Button::new(format!("Extraire ({cost} pts)")))
                            .clicked()
                        {
                            extract = Some((kind, *pick));
                        }
                    });
                }
            });
        if blank.is_some_and(|b| inventory.count(b) == 0) {
            ui.label(
                egui::RichText::new("Il faut un livre vierge dans l'inventaire pour extraire.")
                    .weak(),
            );
        }
        if let Some((kind, level)) = extract {
            action = Some(ScreenAction::LibraryExtract {
                kind: kind.id(),
                level,
            });
        }
        if let Some(action) = action {
            self.send_now(crate::protocol::ClientMsg::Screen(action));
        }
    }

    /// La case de sortie du bloc : un bouton pour la récupérer.
    /// `true` si le bouton est cliqué.
    fn library_output(&mut self, ui: &mut egui::Ui, pos: IVec3) -> bool {
        let out: Option<BlockStack> = self.view_streamer().block_entity(pos).and_then(|e| {
            e.caps
                .as_ref()?
                .inventory
                .as_ref()?
                .slots
                .get(enchant_library::OUTPUT)?
                .clone()
        });
        let Some(out) = out else { return false };
        let items = self.client.content.items.clone();
        let name = items
            .id(&out.item)
            .map_or(out.item.clone(), |id| items.get(id).name.clone());
        ui.button(format!("Récupérer le refus : {} × {}", out.count, name))
            .clicked()
    }

    // ----- Les gestes de la bibliothèque, côté serveur -----
}

/// Les stats de la table : la barre d'Eterna, puis les autres stats en ligne (elles passent à la
/// ligne si la fenêtre est étroite).
fn stats_grid(ui: &mut egui::Ui, stats: &TableStats) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new("Eterna")
                .strong()
                .color(egui::Color32::from_rgb(130, 82, 12)),
        );
        ui.add(
            egui::ProgressBar::new(stats.eterna / 100.0)
                .desired_width(240.0)
                .fill(egui::Color32::from_rgb(196, 132, 40))
                .text(format!("{:.1} / {:.0}", stats.eterna, stats.max_eterna).replace('.', ",")),
        );
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for text in [
            format!("Quanta {}", percent(stats.quanta)),
            format!("Arcana {}", percent(stats.arcana)),
            format!("Stabilité {}", percent(stats.stability)),
            format!("Indices {}", stats.clues),
            format!("Trésor : {}", if stats.treasure { "oui" } else { "non" }),
            format!("Étagères {} / 32", stats.shelves),
        ] {
            ui.label(text);
        }
    });
}

/// La hauteur des zones qui défilent dans ces écrans : assez pour quelques lignes, sans jamais
/// pousser l'inventaire du joueur hors de la fenêtre.
fn scroll_height(ui: &egui::Ui) -> f32 {
    (ui.ctx().content_rect().height() * 0.3).clamp(80.0, 260.0)
}

/// « 1 saphir », « 3 saphirs ».
fn sapphires(n: u32) -> String {
    format!("{n} saphir{}", if n > 1 { "s" } else { "" })
}

/// « Butin IV + ? + ? » devient « Butin IV +2 ? » : un lot tient sur une ligne.
fn compact_preview(preview: &str) -> String {
    let hidden = preview.matches(" + ?").count();
    let shown = preview.replace(" + ?", "");
    if hidden == 0 {
        shown
    } else {
        format!("{shown} +{hidden} ?")
    }
}
