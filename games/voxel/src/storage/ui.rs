//! L'écran des conteneurs : coffres (de base ou à paliers), wagons coffres, sacs à dos et
//! contrôleur de stockage. Un seul module pour tous : les boutons, le Maj + clic et la colonne des
//! améliorations ne sont écrits qu'une fois.
//!
//! # Un conteneur, quatre façons de le lire
//!
//! [`Target`] désigne le conteneur : un bloc (son inventaire est dans l'entité du bloc), un wagon
//! (`CartCargo`) ou un sac (le magasin de `storage/backpacks.rs`). Chacun se lit et s'écrit comme
//! une liste de cases du moteur ([`engine_voxel::ItemStack`], objets par identifiant texte). Les
//! gestes en gros (Déposer, Compléter, Tout prendre, Trier, Maj + clic) convertissent les cases
//! principales en piles du jeu, appellent la logique pure de `voxel_sim::storage::transfer`, puis
//! réécrivent.
//!
//! # La disposition des cases
//!
//! Un coffre à paliers et un sac rangent leurs cases en blocs : les cases principales, puis les
//! emplacements d'amélioration, puis les filtres (voir `engine_voxel::storage`). L'écran dessine
//! les améliorations dans un bandeau à part en haut, puis les cases principales, qui défilent si
//! elles sont nombreuses (l'inventaire du joueur reste visible). Les filtres sont
//! des cases fantômes : on y pose un exemplaire d'un objet (rien n'est consommé), un clic le
//! retire. Après chaque geste, les améliorations passives (compactage, néant) s'appliquent tout de
//! suite.
//!
//! # Le contrôleur
//!
//! Son écran ne montre pas de cases mais les objets de tous les coffres reliés, regroupés par
//! sorte, avec une recherche. Un clic prend une pile, Maj + clic tout, et poser la pile tenue (ou
//! Déposer) range dans les coffres en complétant d'abord les piles.

use engine_render::egui;
use engine_voxel::{Upgrade, controller_groups, controller_members};
use glam::IVec3;
use voxel_sim::net::protocol::{ClientMsg, ScreenAction, TakeHow};
#[allow(unused_imports)]
use voxel_sim::storage::ui::*;

use crate::VoxelGame;
use crate::ui::{Press, Screen};
use voxel_sim::player::inventory::ItemStack;
use voxel_sim::server::ui::SlotRef;
use voxel_sim::storage::backpacks;

impl VoxelGame {
    // ----- Lire et écrire un conteneur -----

    // ----- Les gestes en gros -----

    /// Le Maj + clic est-il possible ici ? (un conteneur ouvert, la main vide)
    pub(crate) fn quick_enabled(&self) -> bool {
        // D'après ce que voit la fenêtre : la main vide, un écran de conteneur ouvert (pas la
        // case de feu d'une locomotive).
        let locomotive = self
            .shown_container()
            .and_then(|c| c.cart)
            .is_some_and(|(kind, _)| kind == voxel_sim::vehicles::carts::CartKind::Locomotive);
        self.client.view.me.inventory.cursor.is_none()
            && match self.client.ui.screen {
                Screen::Chest(_) | Screen::Bag(_) | Screen::Controller(_) => true,
                Screen::Cart(_) => !locomotive,
                _ => false,
            }
    }

    // ----- Un clic sur une case -----

    // ----- L'écran -----

    /// Le titre de l'écran d'un conteneur.
    pub(crate) fn storage_title(&self) -> String {
        match self.client.ui.screen {
            Screen::Chest(pos) => {
                let block = self.server.streamer().world().block(pos);
                let name = &self.client.registry.def(block).name;
                if engine_voxel::blocks::double_chest::paired(&*self.server.streamer(), pos)
                    .is_some()
                {
                    // « Coffre de fer » devient « Grand coffre de fer ».
                    let mut chars = name.chars();
                    let first = chars.next().map(|c| c.to_lowercase().to_string());
                    format!("Grand {}{}", first.unwrap_or_default(), chars.as_str())
                } else {
                    name.clone()
                }
            }
            Screen::Bag(number) => self
                .server
                .backpacks
                .bag(number)
                .map_or("Sac à dos", |b| {
                    backpacks::BAG_TIERS[usize::from(b.tier)].name
                })
                .to_string(),
            Screen::Controller(_) => "Contrôleur de stockage".to_string(),
            _ => "Conteneur".to_string(),
        }
    }

    /// Les cases, la colonne des améliorations, les filtres et les boutons d'un conteneur.
    pub(crate) fn storage_panel(&mut self, ui: &mut egui::Ui, bag: bool) {
        // La vue que le serveur a envoyée (voir `storage/container_view.rs`) ; un conteneur ancien
        // y a déjà reçu ses emplacements d'amélioration.
        let Some(shown) = self.shown_container().cloned() else {
            ui.label("Ce conteneur n'existe plus.");
            return;
        };
        let view = View {
            raw: shown.raw,
            layout: shown.layout,
            disabled: shown.disabled,
            tiered: shown.tiered,
        };
        let layout = view.layout;
        let ups = engine_voxel::installed_upgrades(&view.raw, layout, 0);
        // Les améliorations d'abord, dans un bandeau à part : on ne les confond pas avec le
        // stockage.
        if layout.upgrades > 0 {
            egui::Frame::new()
                .fill(UPGRADE_BAND)
                .stroke(egui::Stroke::new(1.0, crate::ui::skin::BORDER))
                .corner_radius(4.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Améliorations").strong());
                        ui.add_space(8.0);
                        ui.spacing_mut().item_spacing.x = crate::ui::layout::SLOT_GAP;
                        for n in 0..layout.upgrades {
                            self.storage_slot(ui, layout.main + n);
                        }
                        ui.add_space(8.0);
                        if bag && !ups.is_empty() {
                            // Un sac : chaque amélioration se coupe d'un clic.
                            ui.vertical(|ui| {
                                for (n, slot) in layout.upgrade_range().enumerate() {
                                    let Some(up) = view
                                        .raw
                                        .get(slot)
                                        .and_then(Option::as_ref)
                                        .and_then(|s| Upgrade::from_key(&s.item))
                                    else {
                                        continue;
                                    };
                                    let mut on = view.disabled & (1 << n) == 0;
                                    if ui.checkbox(&mut on, up.name()).changed() {
                                        self.send_now(
                                            voxel_sim::net::protocol::ClientMsg::ToggleUpgrade(
                                                n as u8,
                                            ),
                                        );
                                    }
                                }
                            });
                        } else {
                            let names: Vec<&str> = ups.iter().map(|u| u.name()).collect();
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(if names.is_empty() {
                                        "Aucune posée (Maj + clic depuis l'inventaire)".to_string()
                                    } else {
                                        names.join(" · ")
                                    })
                                    .size(11.0),
                                )
                                .wrap(),
                            );
                        }
                    });
                });
            ui.add_space(6.0);
        }
        // Le stockage : il défile, pour que l'inventaire du joueur reste visible dessous.
        ui.label(
            egui::RichText::new(format!("Stockage · {} cases", layout.main))
                .strong()
                .size(11.0),
        );
        egui::ScrollArea::vertical()
            .id_salt("storage_main")
            .max_height((ui.ctx().content_rect().height() * 0.2).clamp(96.0, 300.0))
            .show(ui, |ui| {
                crate::ui::layout::slots(ui, layout.main, 9, |ui, i| {
                    self.storage_slot(ui, i);
                });
            });
        // Les filtres, seulement quand l'amélioration qui les lit est posée.
        for group in 0..layout.filter_groups() {
            let wanted = if group == 0 {
                Upgrade::Void
            } else {
                Upgrade::Pickup
            };
            if !ups.contains(&wanted) {
                continue;
            }
            let range = layout.filter_range(group);
            ui.label(
                egui::RichText::new(if group == 0 {
                    "Filtre du Néant (vide : détruit le surplus)"
                } else {
                    "Filtre du ramassage (vide : tout)"
                })
                .size(11.0),
            );
            crate::ui::layout::slots(ui, range.len(), range.len(), |ui, i| {
                self.storage_slot(ui, range.start + i);
            });
        }
        let mut op = None;
        ui.horizontal_wrapped(|ui| {
            for (label, action) in [
                ("Déposer", Op::Deposit),
                ("Compléter", Op::TopUp),
                ("Tout prendre", Op::TakeAll),
                ("Trier", Op::Sort),
            ] {
                if ui.button(label).clicked() {
                    op = Some(action);
                }
            }
        });
        if let Some(op) = op {
            self.send_now(voxel_sim::net::protocol::ClientMsg::ContainerOp(op));
        }
    }

    fn storage_slot(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let stack = self.shown_stack(index);
        match self.stack_widget(ui, stack, false) {
            Some(press) => self.press_slot(SlotRef::Open(index), stack, press),
            None => false,
        }
    }

    // ----- Le contrôleur -----

    /// L'écran du contrôleur : les objets réunis, une recherche, un clic pour en prendre.
    pub(crate) fn controller_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        let items = self.client.content.items.clone();
        // Les coffres reliés, tels que la réplique du monde les voit.
        let (members, groups) = {
            let streamer = self.view_streamer();
            let members = controller_members(streamer, pos);
            let groups = controller_groups(streamer, &members);
            (members, groups)
        };
        let total: u32 = groups.iter().map(|g| g.count).sum();
        ui.label(
            egui::RichText::new(format!(
                "{} conteneur(s) relié(s) · {} sorte(s) · {} objet(s)",
                members.len(),
                groups.len(),
                total
            ))
            .size(11.0),
        );
        ui.horizontal(|ui| {
            ui.label("Recherche");
            ui.add(
                egui::TextEdit::singleline(&mut self.client.ui.storage_search)
                    .text_color(crate::ui::skin::TEXT)
                    .desired_width(ui.available_width().min(220.0)),
            );
            if ui.button("Déposer").clicked() {
                self.send_now(ClientMsg::Screen(ScreenAction::ControllerDeposit));
            }
        });
        let query = self.client.ui.storage_search.clone();
        let shown: Vec<(engine_voxel::ItemId, engine_voxel::Group)> = groups
            .into_iter()
            .filter_map(|g| Some((items.id(&g.item)?, g)))
            .filter(|(id, _)| crate::ui::recipe_browser::matches(items.get(*id), &query))
            .collect();
        let mut pressed: Option<(Option<(engine_voxel::ItemId, engine_voxel::Group)>, Press)> =
            None;
        egui::ScrollArea::vertical()
            .max_height((ui.ctx().content_rect().height() * 0.4).clamp(80.0, 320.0))
            .show(ui, |ui| {
                // Une case de plus, vide : poser ici la pile tenue.
                crate::ui::layout::slots(ui, shown.len() + 1, 9, |ui, i| {
                    let group = shown.get(i).cloned();
                    let stack = group.as_ref().map(|(id, g)| ItemStack {
                        item: *id,
                        count: g.count,
                        damage: g.damage,
                    });
                    if let Some(press) = self.stack_widget(ui, stack, false) {
                        pressed = Some((group, press));
                    }
                });
            });
        let Some((group, press)) = pressed else {
            return;
        };
        let action = match (group, self.client.view.me.inventory.cursor) {
            // La pile tenue est rangée dans les coffres.
            (_, Some(_)) => ScreenAction::ControllerPut,
            (Some((_, g)), None) => ScreenAction::ControllerTake {
                item: g.item,
                damage: g.damage,
                how: match press {
                    Press::Quick => TakeHow::All,
                    Press::Secondary | Press::Split => TakeHow::Half,
                    Press::Primary => TakeHow::Stack,
                },
            },
            (None, None) => return,
        };
        self.send_now(ClientMsg::Screen(action));
    }

    // ----- Ouvrir -----
}

/// Le fond du bandeau des améliorations : plus sombre que le panneau, pour le détacher du stockage.
const UPGRADE_BAND: egui::Color32 = egui::Color32::from_rgb(158, 162, 150);

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> engine_voxel::ItemRegistry {
        let blocks = engine_voxel::BlockRegistry::builtin();
        engine_voxel::ItemRegistry::builtin(&blocks).unwrap()
    }

    #[test]
    fn an_upgrade_slot_takes_one_upgrade_and_nothing_else() {
        let items = items();
        let upgrade = items.id(Upgrade::Compact.key()).unwrap();
        let stone = items.id("core:stone").unwrap();
        let fits = |i| Upgrade::from_key(&items.get(i).key).is_some();
        let mut slot = None;
        let mut cursor = Some(ItemStack::new(upgrade, 3));
        upgrade_click(&mut slot, &mut cursor, fits);
        assert_eq!(slot.unwrap().count, 1);
        assert_eq!(cursor.unwrap().count, 2, "une seule est posée");
        // Une pierre n'entre pas, et une pile de plusieurs n'échange pas.
        let mut cursor = Some(ItemStack::new(stone, 1));
        upgrade_click(&mut slot, &mut cursor, fits);
        assert_eq!(slot.unwrap().item, upgrade);
        let mut cursor = Some(ItemStack::new(upgrade, 2));
        upgrade_click(&mut slot, &mut cursor, fits);
        assert_eq!(cursor.unwrap().count, 2);
        // Main vide : on reprend l'amélioration.
        let mut cursor = None;
        upgrade_click(&mut slot, &mut cursor, fits);
        assert!(slot.is_none() && cursor.unwrap().item == upgrade);
    }

    #[test]
    fn a_filter_slot_copies_one_item_without_consuming_it() {
        let items = items();
        let stone = items.id("core:stone").unwrap();
        let bag = items.id(backpacks::BAG_TIERS[0].key).unwrap();
        let mut slot = None;
        let mut cursor = Some(ItemStack::new(stone, 40));
        ghost_click(&mut slot, &mut cursor, &items);
        assert_eq!(slot, Some(ItemStack::new(stone, 1)));
        assert_eq!(cursor.unwrap().count, 40, "rien n'est consommé");
        let mut cursor = Some(ItemStack::new(bag, 1));
        ghost_click(&mut slot, &mut cursor, &items);
        assert_eq!(slot.unwrap().item, stone, "un sac ne se filtre pas");
        let mut cursor = None;
        ghost_click(&mut slot, &mut cursor, &items);
        assert!(slot.is_none() && cursor.is_none());
    }
}
