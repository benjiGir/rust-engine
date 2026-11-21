//! Le conteneur ouvert, tel que la fenêtre le voit : coffre, four, alambic, sac à dos, wagon.
//!
//! Le serveur sait ce que chaque joueur a ouvert (`PlayerState::open`). À la fin de chaque image, il
//! décrit ce conteneur dans une [`ContainerView`] : ses cases, leur disposition (améliorations,
//! filtres), les améliorations coupées d'un sac, les jauges d'un four ou d'un alambic, la sorte et le
//! feu d'un wagon. Comme les autres vues (`player_view.rs`), elle n'est envoyée que si elle a changé
//! (`ServerMsg::Container`), et `None` quand plus rien n'est ouvert.
//!
//! Les écrans dessinent cette vue ; leurs gestes partent en messages : les clics de cases
//! (`SlotClick`, lot 2), les boutons Déposer, Compléter, Tout prendre et Trier (`ContainerOp`), les
//! améliorations d'un sac coupées d'un clic (`ToggleUpgrade`), l'artisanat (`Craft`).
//!
//! Le contrôleur de stockage, qui réunit plusieurs coffres, n'a pas encore de vue : son écran lit le
//! serveur (sous-lot 3d-2).

use engine_voxel::{FurnaceState, ItemStack as StoredStack, Layout};
use serde::{Deserialize, Serialize};

use crate::VoxelGame;
use crate::carts::{Cart, CartKind};
use crate::inventory::ItemStack;
use crate::players::OpenContainer;
use crate::storage_ui::Target;

/// Ce que la fenêtre montre du conteneur ouvert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerView {
    /// Toutes les cases, objets par identifiant texte (comme dans l'entité du bloc).
    pub raw: Vec<Option<StoredStack>>,
    pub layout: Layout,
    /// Les améliorations coupées d'un sac (un bit par emplacement).
    pub disabled: u32,
    /// Un conteneur à paliers ou un sac : bandeau d'améliorations et filtres.
    pub tiered: bool,
    /// Un four : le feu restant et la cuisson (0 à 1).
    pub furnace: [f32; 2],
    /// Un alambic : l'avancement (0 à 1).
    pub brewing: f32,
    /// Un wagon : sa sorte et son feu (une locomotive).
    pub cart: Option<(CartKind, f32)>,
}

impl ContainerView {
    /// La pile d'une case, en pile du jeu (`None` si vide, ou objet inconnu : mod absent).
    pub fn stack(&self, items: &engine_voxel::ItemRegistry, index: usize) -> Option<ItemStack> {
        let stored = self.raw.get(index)?.as_ref()?;
        Some(ItemStack {
            item: items.id(&stored.item)?,
            count: stored.count,
            damage: stored.damage,
        })
    }
}

fn ratio(done: f32, total: f32) -> f32 {
    if total > 0.0 {
        (done / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

impl VoxelGame {
    /// Le conteneur ouvert par le joueur courant, décrit pour sa fenêtre.
    pub(crate) fn open_container_view(&mut self) -> Option<ContainerView> {
        let (target, furnace, brewing, cart) = match self.players.me().open? {
            OpenContainer::Chest(pos) => (Target::Block(pos), [0.0; 2], 0.0, None),
            OpenContainer::Machine(pos) => {
                let (furnace, brewing) = {
                    let streamer = self.streamer();
                    let entity = streamer.block_entity(pos)?;
                    let state = FurnaceState::read(entity);
                    let (done, total) = engine_voxel::brew_progress(entity);
                    (
                        [
                            ratio(state.burn_left as f32, state.burn_total as f32),
                            ratio(state.progress as f32, state.cook_total as f32),
                        ],
                        ratio(f32::from(done), f32::from(total)),
                    )
                };
                (Target::Block(pos), furnace, brewing, None)
            }
            OpenContainer::Cart(entity) => {
                let cart = self.ecs.get::<Cart>(entity).map(|c| (c.kind, c.burn))?;
                (Target::Cart(entity), [0.0; 2], 0.0, Some(cart))
            }
            OpenContainer::Bag(number) => (Target::Bag(number), [0.0; 2], 0.0, None),
            OpenContainer::Controller(_) => return None,
        };
        let shown = self.open_contents(target)?;
        Some(ContainerView {
            raw: shown.0,
            layout: shown.1,
            disabled: shown.2,
            tiered: shown.3,
            furnace,
            brewing,
            cart,
        })
    }

    /// `Craft` : fabriquer avec la grille du joueur courant, ou une recette du livre de recettes.
    pub(crate) fn craft(&mut self, size: u8, recipe: Option<u32>) {
        let size = usize::from(size).clamp(2, 3);
        let items = self.content.items.clone();
        let made = match recipe {
            None => {
                let grid = self.players.me_mut().inventory.craft_grid(size);
                let Some(recipe) = self.content.recipes.find(&grid, size).cloned() else {
                    return;
                };
                self.players
                    .me_mut()
                    .inventory
                    .take_craft_result(&recipe, size, &items)
                    .then(|| recipe.result())
            }
            Some(index) => {
                let Some(recipe) = self.content.recipes.iter().nth(index as usize).cloned() else {
                    log::warn!("recette inconnue ({index}) : ignorée");
                    return;
                };
                if recipe.grid_size() > size {
                    return;
                }
                self.players
                    .me_mut()
                    .inventory
                    .craft_from_book(&recipe, &items)
                    .map(|_| recipe.result())
            }
        };
        if let Some((item, count)) = made {
            self.players.me_mut().quest_book.note_crafted(item, count);
        }
    }

    /// La vue du conteneur ouvert que la fenêtre a reçue.
    pub(crate) fn shown_container(&self) -> Option<&ContainerView> {
        self.view.me.container.as_ref()
    }

    /// La pile d'une case du conteneur ouvert, telle que la fenêtre la voit.
    pub(crate) fn shown_stack(&self, index: usize) -> Option<ItemStack> {
        self.shown_container()?.stack(&self.content.items, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(raw: Vec<Option<StoredStack>>) -> ContainerView {
        ContainerView {
            layout: Layout::new(raw.len(), 0, 0),
            raw,
            disabled: 0,
            tiered: false,
            furnace: [0.5, 0.25],
            brewing: 0.0,
            cart: None,
        }
    }

    #[test]
    fn a_container_view_survives_postcard() {
        let view = ContainerView {
            cart: Some((CartKind::Locomotive, 12.5)),
            ..view(vec![
                Some(StoredStack {
                    item: "core:stone".into(),
                    count: 12,
                    damage: 0,
                }),
                None,
            ])
        };
        let bytes = postcard::to_allocvec(&view).unwrap();
        assert_eq!(postcard::from_bytes::<ContainerView>(&bytes).unwrap(), view);
    }

    #[test]
    fn gauges_are_ratios_and_an_empty_total_is_zero() {
        assert_eq!(ratio(5.0, 10.0), 0.5);
        assert_eq!(ratio(5.0, 0.0), 0.0);
        assert_eq!(ratio(20.0, 10.0), 1.0);
    }
}
