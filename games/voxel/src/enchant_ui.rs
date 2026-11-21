//! Les écrans de l'enchantement avancé : la table (stats, indices, lots payés en points d'expérience),
//! l'enclume simple et la bibliothèque. Les règles sont dans `enchant_table.rs`, `enchant_books.rs`
//! et `enchant_library.rs` ; ici, la lecture du monde autour de la table et le dessin.

use std::collections::HashMap;

use engine_render::egui;
use engine_voxel::ItemStack as BlockStack;
use glam::IVec3;

use crate::VoxelGame;
use crate::enchant_books::{self, BookKind, ENCHANTED_BOOK};
use crate::enchant_library::{self, Library};
use crate::enchant_table::{self, Offer, Subject, TableStats};
use crate::enchanting::{self, Enchantment};
use crate::inventory::ItemStack;
use crate::protocol::ScreenAction;

/// La couleur d'un nom d'enchantement sur le cadre clair de l'inventaire.
const ENCHANT_TEXT: egui::Color32 = egui::Color32::from_rgb(98, 54, 158);

/// Ce que gardent les écrans d'une image à l'autre.
#[derive(Default)]
pub struct EnchantUi {
    /// Le temps écoulé depuis le dernier pas des charmes, et le nombre de pas (voir `charms.rs`).
    pub charm_clock: f32,
    pub charm_beats: u32,
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
    /// Les stats de la table posée en `pos` (voir `enchant_table.rs`).
    pub(crate) fn table_stats(&self, pos: IVec3) -> TableStats {
        table_stats_in(&self.registry, self.streamer().world(), pos)
    }

    /// Les stats de la table telles que la fenêtre les voit (la réplique du monde).
    fn shown_table_stats(&self, pos: IVec3) -> TableStats {
        table_stats_in(&self.registry, self.view_world(), pos)
    }
}

/// Les stats d'une table, lues dans `world` (celui du serveur, ou la réplique de la fenêtre).
fn table_stats_in(
    registry: &engine_voxel::BlockRegistry,
    world: &engine_voxel::World,
    pos: IVec3,
) -> TableStats {
    enchant_table::read_stats(
        pos,
        |p| !registry.def(world.block(p)).solid,
        |p| {
            registry
                .def(world.block(p))
                .enchanting
                .clone()
                .filter(|s| s.library_level == 0)
        },
    )
}

impl VoxelGame {
    /// Le sujet de la table : la pile en main, objet à enchanter ou livre à remplir.
    fn table_subject<'a>(
        items: &'a engine_voxel::ItemRegistry,
        stack: &ItemStack,
    ) -> Option<Subject<'a>> {
        let def = items.get(stack.item);
        if def.key == ENCHANTED_BOOK {
            return None;
        }
        Some(match BookKind::of_key(&def.key) {
            Some(kind) => Subject::Book(kind),
            None => Subject::Item(def),
        })
    }

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
        let Some(vitals) = self.view.me.vitals.clone() else {
            return;
        };
        let Some(stack) = self.view.me.inventory.selected_stack() else {
            ui.label("Tiens un outil, une armure, un livre ou un tome dans la barre d'objets, puis reviens.");
            return;
        };
        let items = self.content.items.clone();
        let def = items.get(stack.item);
        let owned = enchanting::labels(stack.damage);
        if !owned.is_empty() {
            ui.label(format!("Enchantements : {}", owned.join(", ")));
        }
        let Some(subject) = Self::table_subject(&items, &stack) else {
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
        let have_gems = gem.map_or(0, |g| self.view.me.inventory.as_inventory().count(g));
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

    /// `EnchantBuy` : le serveur refait la liste des lots de la table ouverte pour l'objet en main,
    /// et achète celui-ci s'il est abordable.
    pub(crate) fn buy_enchant_offer(&mut self, pos: IVec3, index: usize) {
        let stats = self.table_stats(pos);
        let Some(stack) = self.players.me().inventory.selected_stack() else {
            return;
        };
        let items = self.content.items.clone();
        let Some(subject) = Self::table_subject(&items, &stack) else {
            return;
        };
        let data = crate::adventure_data::current();
        let key = items.get(stack.item).key.clone();
        let Some(offer) = enchant_table::offers(
            &data,
            subject,
            &key,
            stack.damage,
            self.enchant_seed,
            &stats,
        )
        .and_then(|list| list.into_iter().nth(index)) else {
            log::warn!("lot d'enchantement inconnu ({index}) : ignoré");
            return;
        };
        self.buy_offer(&offer, items.id("core:sapphire"));
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

    /// Paie un lot (points d'expérience et saphirs) et l'applique à la pile en main : un objet
    /// reçoit les enchantements, un livre ou un tome devient un livre enchanté.
    fn buy_offer(&mut self, offer: &Offer, gem: Option<engine_voxel::ItemId>) {
        let Some(gem) = gem else { return };
        if self.players.me_mut().experience.points < offer.points
            || !self.players.me_mut().inventory.remove(gem, offer.gems)
        {
            return;
        }
        self.players.me_mut().experience.points -= offer.points;
        let selected = self.players.me_mut().inventory.selected;
        let items = self.content.items.clone();
        let Some(held) = self.players.me_mut().inventory.slots[selected] else {
            return;
        };
        if BookKind::of_key(&items.get(held.item).key).is_some() {
            // Un livre vierge : un exemplaire est consommé, et un livre enchanté le remplace.
            if let Some(slot) = self.players.me_mut().inventory.slots[selected].as_mut() {
                slot.count -= 1;
                if slot.count == 0 {
                    self.players.me_mut().inventory.slots[selected] = None;
                }
            }
            if let Some(enchanted) = items.id(ENCHANTED_BOOK) {
                self.give_or_drop(ItemStack {
                    item: enchanted,
                    count: 1,
                    damage: offer.apply(0),
                });
            }
        } else if let Some(slot) = self.players.me_mut().inventory.slots[selected].as_mut() {
            slot.damage = offer.apply(slot.damage);
        }
        // La table propose autre chose la prochaine fois.
        self.enchant_seed = self
            .enchant_seed
            .wrapping_mul(1664525)
            .wrapping_add(1013904223);
        self.toast(&offer.label());
        self.bell();
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
        if let Some(held) = self.view.me.inventory.cursor {
            let items = self.content.items.clone();
            ui.separator();
            ui.label(format!("À la souris : {}", items.get(held.item).name));
            for label in enchanting::labels(held.damage) {
                ui.label(egui::RichText::new(label).color(ENCHANT_TEXT));
            }
        }
    }

    // ----- La bibliothèque -----

    /// Les octets et le plafond de la bibliothèque en `pos`.
    fn library_state(&self, pos: IVec3) -> Option<(Library, u8)> {
        library_in(&self.registry, &self.streamer(), pos)
    }

    /// La bibliothèque telle que la fenêtre la voit (la réplique).
    fn shown_library(&self, pos: IVec3) -> Option<(Library, u8)> {
        library_in(&self.registry, self.view_streamer(), pos)
    }
}

/// Les octets et le plafond d'une bibliothèque, lus dans `streamer`.
fn library_in(
    registry: &engine_voxel::BlockRegistry,
    streamer: &engine_voxel::ChunkStreamer,
    pos: IVec3,
) -> Option<(Library, u8)> {
    let max = enchant_library::max_level_of(
        registry
            .def(streamer.world().block(pos))
            .enchanting
            .as_ref(),
    );
    if max == 0 {
        return None;
    }
    let library = streamer
        .block_entity(pos)
        .map_or_else(Library::default, |e| Library::read(&e.data));
    Some((library, max))
}

impl VoxelGame {
    fn write_library(&mut self, pos: IVec3, library: &Library) {
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos) {
            entity.data = library.write();
        }
    }

    /// Range ce que contient la case d'entrée (les réseaux y déposent) avant d'afficher.
    fn absorb_library_input(&mut self, pos: IVec3, max: u8) {
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos) {
            enchant_library::absorb(entity, max);
        }
    }

    /// Dépose un livre enchanté (une pile) dans la bibliothèque. `Err(texte)` s'il est refusé.
    fn deposit_book(&mut self, pos: IVec3, book: &ItemStack) -> Result<(), String> {
        let Some((mut library, max)) = self.library_state(pos) else {
            return Err("Ce n'est pas une bibliothèque".into());
        };
        let enchants = enchanting::enchantments_of(book.damage);
        library.deposit(&enchants, max).map_err(|r| r.text())?;
        self.write_library(pos, &library);
        enchanting::release(book.damage);
        Ok(())
    }

    /// La bibliothèque : dépôt, recherche, filtre par objet, extraction au niveau choisi.
    pub(crate) fn library_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        let items = self.content.items.clone();
        // La bibliothèque telle que la réplique la voit (le serveur range la case d'entrée tant que
        // l'écran est ouvert) ; l'inventaire, d'après la vue du joueur.
        let Some((library, max)) = self.shown_library(pos) else {
            ui.label("Cette bibliothèque a disparu.");
            return;
        };
        let inventory = self.view.me.inventory.as_inventory();
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
                egui::TextEdit::singleline(&mut self.enchant_ui.search)
                    .desired_width(160.0)
                    .hint_text("nom de l'enchantement"),
            );
            ui.checkbox(&mut self.enchant_ui.only_fitting, "Va sur l'objet en main");
        });
        let held = inventory.selected_stack();
        let needle = self.enchant_ui.search.trim().to_lowercase();
        let mut rows: Vec<(Enchantment, u32, u8)> = library.entries();
        rows.retain(|(kind, _, _)| {
            let name = kind.name().to_lowercase();
            let fits = !self.enchant_ui.only_fitting
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
        let items = self.content.items.clone();
        let name = items
            .id(&out.item)
            .map_or(out.item.clone(), |id| items.get(id).name.clone());
        ui.button(format!("Récupérer le refus : {} × {}", out.count, name))
            .clicked()
    }

    // ----- Les gestes de la bibliothèque, côté serveur -----

    /// Range la case d'entrée de la bibliothèque (les réseaux y déposent), si elle n'est pas vide.
    pub(crate) fn absorb_library(&mut self, pos: IVec3) {
        let Some((_, max)) = self.library_state(pos) else {
            return;
        };
        let waiting = self
            .streamer()
            .block_entity(pos)
            .and_then(|e| {
                e.caps
                    .as_ref()?
                    .inventory
                    .as_ref()?
                    .slots
                    .get(enchant_library::INPUT)
                    .cloned()
                    .flatten()
            })
            .is_some();
        if waiting {
            self.absorb_library_input(pos, max);
        }
    }

    pub(crate) fn library_deposit_cursor(&mut self, pos: IVec3) {
        self.absorb_library(pos);
        let book_id = self.content.items.id(ENCHANTED_BOOK);
        let Some(book) = self
            .players
            .me()
            .inventory
            .cursor
            .filter(|s| Some(s.item) == book_id && enchanting::is_enchanted(s.damage))
        else {
            return;
        };
        match self.deposit_book(pos, &book) {
            Ok(()) => {
                self.players.me_mut().inventory.cursor = None;
                self.toast("Livre rangé dans la bibliothèque");
                self.bell();
            }
            Err(why) => self.toast(&why),
        }
    }

    pub(crate) fn library_deposit_all(&mut self, pos: IVec3) {
        self.absorb_library(pos);
        let book_id = self.content.items.id(ENCHANTED_BOOK);
        let mut stored = 0;
        for i in 0..self.players.me().inventory.slots.len() {
            let Some(stack) = self.players.me().inventory.slots[i] else {
                continue;
            };
            if Some(stack.item) != book_id || !enchanting::is_enchanted(stack.damage) {
                continue;
            }
            if self.deposit_book(pos, &stack).is_ok() {
                self.players.me_mut().inventory.slots[i] = None;
                stored += 1;
            }
        }
        self.toast(&format!("{stored} livre(s) rangé(s)"));
        if stored > 0 {
            self.bell();
        }
    }

    pub(crate) fn library_take_output(&mut self, pos: IVec3) {
        let items = self.content.items.clone();
        let mut taken = None;
        if let Some(entity) = self.streamer_mut().block_entity_mut(pos)
            && let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut())
            && let Some(slot) = inventory.slots.get_mut(enchant_library::OUTPUT)
        {
            taken = slot.take();
        }
        if let Some(out) = taken
            && let Some(id) = items.id(&out.item)
        {
            self.give_or_drop(ItemStack {
                item: id,
                count: out.count,
                damage: out.damage,
            });
        }
    }

    /// Sort un livre enchanté de la bibliothèque, au niveau choisi, et consomme un livre vierge.
    pub(crate) fn extract_from_library(&mut self, pos: IVec3, kind: Enchantment, level: u8) {
        self.absorb_library(pos);
        let items = self.content.items.clone();
        let (Some(blank), Some(enchanted)) =
            (items.id(enchant_books::BOOK), items.id(ENCHANTED_BOOK))
        else {
            return;
        };
        let Some((mut library, _)) = self.library_state(pos) else {
            return;
        };
        if !library.extract(kind, level) || !self.players.me_mut().inventory.remove(blank, 1) {
            return;
        }
        self.write_library(pos, &library);
        self.give_or_drop(ItemStack {
            item: enchanted,
            count: 1,
            damage: enchant_books::book_damage(&[(kind, level)]),
        });
        self.toast(&enchanting::label(kind, level));
        self.bell();
    }
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
