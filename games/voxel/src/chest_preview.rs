//! L'aperçu rapide du contenu d'un coffre, dans la fiche de visée : on regarde un conteneur,
//! on voit ce qu'il y a dedans sans l'ouvrir.
//!
//! [`summarize`] regroupe les objets par sorte (leur total, toutes piles confondues) et les trie
//! par quantité ; au plus [`MAX_ICONS`] icônes, puis « +N sortes ». Il marche pour tout bloc qui
//! a la capacité `Inventory` (coffres de tous les paliers, grand coffre d'un mod...) et pour un
//! wagon coffre. Seules les cases principales comptent : les emplacements d'amélioration et les
//! filtres d'un coffre à paliers ne sont pas son contenu.

use engine_render::egui;

use crate::skin;

/// Le plus d'icônes affichées.
pub const MAX_ICONS: usize = 12;
/// La taille d'une icône de l'aperçu.
const CELL: f32 = 28.0;

/// Ce que montre l'aperçu : les sortes les plus abondantes, et combien d'autres restent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preview {
    /// (identifiant de l'objet, total), par quantité décroissante puis identifiant.
    pub kinds: Vec<(String, u32)>,
    /// Le nombre de sortes qui ne tiennent pas dans les icônes.
    pub more: usize,
}

impl Preview {
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

/// Regroupe les `main` premières cases par sorte d'objet.
pub fn summarize(slots: &[Option<engine_voxel::ItemStack>], main: usize) -> Preview {
    let mut totals: Vec<(String, u32)> = Vec::new();
    for stack in slots.iter().take(main).flatten() {
        match totals.iter_mut().find(|(item, _)| *item == stack.item) {
            Some((_, total)) => *total += stack.count,
            None => totals.push((stack.item.clone(), stack.count)),
        }
    }
    totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let more = totals.len().saturating_sub(MAX_ICONS);
    totals.truncate(MAX_ICONS);
    Preview {
        kinds: totals,
        more,
    }
}

/// Dessine l'aperçu : « Vide », ou une grille d'icônes avec leur total, puis « +N sortes ».
/// `icons` donne la texture de chaque icône, dans l'ordre de `preview.kinds`.
pub fn draw(ui: &mut egui::Ui, preview: &Preview, icons: &[Option<egui::TextureId>]) {
    if preview.is_empty() {
        ui.label(egui::RichText::new("Vide").size(13.0).color(skin::MUTED));
        return;
    }
    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
    ui.horizontal_wrapped(|ui| {
        for (n, (_, total)) in preview.kinds.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::hover());
            let painter = ui.painter();
            skin::slot(painter, rect, false, false);
            if let Some(Some(icon)) = icons.get(n) {
                skin::stack(painter, rect, *icon, *total, None, false);
            }
        }
    });
    if preview.more > 0 {
        ui.label(
            egui::RichText::new(format!("+{} sortes", preview.more))
                .size(12.0)
                .color(skin::MUTED),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<engine_voxel::ItemStack> {
        Some(engine_voxel::ItemStack {
            item: item.into(),
            count,
            damage: 0,
        })
    }

    #[test]
    fn an_empty_container_is_empty() {
        assert!(summarize(&[None, None], 2).is_empty());
        assert_eq!(summarize(&[], 0), Preview::default());
    }

    #[test]
    fn kinds_are_grouped_and_sorted_by_quantity() {
        let slots = [
            stack("core:dirt", 10),
            stack("core:stone", 64),
            None,
            stack("core:dirt", 60),
            stack("core:coal", 3),
        ];
        let preview = summarize(&slots, slots.len());
        assert_eq!(
            preview.kinds,
            vec![
                ("core:dirt".into(), 70),
                ("core:stone".into(), 64),
                ("core:coal".into(), 3)
            ]
        );
        assert_eq!(preview.more, 0);
    }

    #[test]
    fn at_most_twelve_icons_then_a_count_of_the_rest() {
        let slots: Vec<_> = (0..20)
            .map(|i| stack(&format!("core:item{i:02}"), 20 - i))
            .collect();
        let preview = summarize(&slots, slots.len());
        assert_eq!(preview.kinds.len(), MAX_ICONS);
        assert_eq!(preview.more, 8);
        assert_eq!(
            preview.kinds[0].0, "core:item00",
            "le plus abondant d'abord"
        );
    }

    #[test]
    fn upgrade_and_filter_slots_are_not_contents() {
        // 3 cases principales, puis une amélioration et un filtre.
        let slots = [
            stack("core:stone", 5),
            None,
            None,
            stack("core:upgrade_void", 1),
            stack("core:coal", 1),
        ];
        assert_eq!(summarize(&slots, 3).kinds, vec![("core:stone".into(), 5)]);
    }
}
