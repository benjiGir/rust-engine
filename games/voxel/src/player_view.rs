//! Ce que la fenêtre sait de **son** joueur, et les retours que le serveur lui adresse.
//!
//! Le serveur tient l'état de chaque joueur (`PlayerState`, la vie dans l'ECS). À la fin de chaque
//! image, il en fait deux vues :
//! - [`VitalsView`] : vie, faim, air, feu, coups reçus, expérience, effets en cours ;
//! - [`InventoryView`] : les cases, l'armure, la grille d'artisanat, la case choisie, la pile tenue.
//!
//! Chacune n'est envoyée que si elle a changé depuis le dernier envoi à ce joueur. Le client les range
//! dans [`ClientPlayer`] : le HUD (barre d'objets, vie, faim, air, expérience, effets) et la main ne
//! lisent plus que lui.
//!
//! Les **retours** d'une action ([`Feedback`]) suivent le même chemin : un son, un message à l'écran,
//! le bras qui frappe. Le code du serveur les adresse au joueur qui agit (`VoxelGame::sound`,
//! `toast`, `punch`), ou aux joueurs proches d'un point (`sound_near`).
//!
//! Le minage et le repas en cours restent une **prédiction** du client (la barre sous le réticule) :
//! ils sont calculés par la fenêtre elle-même, comme aujourd'hui.

use std::collections::HashMap;
use std::time::Instant;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::VoxelGame;
use crate::audio::BlockSound;
use crate::container_view::ContainerView;
use crate::inventory::{Inventory, ItemStack, SLOTS, Slot};
use crate::players::{PlayerId, PlayerState};
use crate::potions::Effect;
use crate::protocol::ServerMsg;

/// La distance (en blocs) jusqu'à laquelle un joueur entend un son placé dans le monde.
pub const HEARING: f32 = 24.0;

/// Un effet en cours, tel que le HUD l'affiche.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EffectView {
    pub effect: Effect,
    /// Les secondes restantes, arrondies : la vue ne change qu'une fois par seconde.
    pub seconds: u32,
}

/// Les jauges du joueur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VitalsView {
    /// Vie actuelle et maximale.
    pub health: [f32; 2],
    pub food: f32,
    /// Secondes d'air restantes.
    pub air: f32,
    pub burning: bool,
    /// Le nombre de coups reçus : quand il change, l'écran rougit un instant.
    pub hits: u32,
    /// Niveau et avancement vers le suivant (0 à 1).
    pub level: u32,
    pub progress: f32,
    /// Les points d'expérience (le prix des lots de la table d'enchantement).
    pub points: u32,
    /// La graine des lots de la table d'enchantement : le client calcule les mêmes lots que le
    /// serveur.
    pub enchant_seed: u32,
    /// La rareté choisie à la table de réforge, et la graine de ses tirages.
    pub forge_rarity: u8,
    pub forge_seed: u64,
    /// L'aventure : le palier choisi, les paliers débloqués et ce qui les débloque.
    pub tiers: crate::tiers::Progress,
    /// La portée de la main (l'équipement l'allonge).
    pub reach: f32,
    pub effects: Vec<EffectView>,
}

/// L'inventaire du joueur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InventoryView {
    pub slots: Vec<Option<ItemStack>>,
    pub armor: [Option<ItemStack>; 4],
    pub craft: [Option<ItemStack>; 9],
    pub selected: u8,
    pub cursor: Option<ItemStack>,
}

impl InventoryView {
    pub fn of(inventory: &Inventory) -> Self {
        Self {
            slots: inventory.slots.to_vec(),
            armor: inventory.armor,
            craft: inventory.craft,
            selected: inventory.selected as u8,
            cursor: inventory.cursor,
        }
    }

    /// Une copie de l'inventaire, pour les lectures de l'interface (ce qu'on peut fabriquer, une
    /// case...). La modifier ne change rien : les gestes partent en messages.
    pub fn as_inventory(&self) -> Inventory {
        let mut inventory = Inventory::default();
        for (to, from) in inventory.slots.iter_mut().zip(&self.slots) {
            *to = *from;
        }
        inventory.armor = self.armor;
        inventory.craft = self.craft;
        inventory.selected = usize::from(self.selected);
        inventory.cursor = self.cursor;
        inventory
    }

    /// La pile d'une case du joueur.
    pub fn slot(&self, slot: Slot) -> Option<ItemStack> {
        match slot {
            Slot::Main(i) => self.slots.get(i).copied().flatten(),
            Slot::Armor(i) => self.armor.get(i).copied().flatten(),
            Slot::Craft(i) => self.craft.get(i).copied().flatten(),
        }
    }

    /// La pile de la case choisie.
    pub fn selected_stack(&self) -> Option<ItemStack> {
        self.slots
            .get(usize::from(self.selected))
            .copied()
            .flatten()
    }
}

impl Default for InventoryView {
    fn default() -> Self {
        Self {
            slots: vec![None; SLOTS],
            armor: [None; 4],
            craft: [None; 9],
            selected: 0,
            cursor: None,
        }
    }
}

/// Un son. Les sons de blocs portent le numéro du bloc : le client choisit la matière (bois, pierre...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Sound {
    Key(String),
    Block { block: u16, kind: BlockSound },
}

impl Sound {
    pub fn key(key: &str) -> Self {
        Self::Key(key.to_string())
    }
}

/// Un retour du serveur au joueur, après une action ou un événement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Feedback {
    Sound(Sound),
    /// Un message en bas de l'écran.
    Toast(String),
    /// Le bras du joueur frappe (un geste réussi : poser, utiliser, frapper).
    Punch,
}

/// Ce que la fenêtre a reçu de son joueur.
#[derive(Debug, Default)]
pub struct ClientPlayer {
    pub vitals: Option<VitalsView>,
    pub inventory: InventoryView,
    /// Le conteneur ouvert (voir `container_view.rs`).
    pub container: Option<ContainerView>,
    /// Ce que montre l'écran ouvert, quand le serveur seul le sait (voir `screen_actions.rs`).
    pub screen: Option<crate::screen_actions::ScreenView>,
    /// La progression des quêtes du joueur (le livre et les quêtes épinglées la lisent).
    pub quests: Option<crate::quest_progress::Progress>,
    /// Le moment du dernier coup reçu (quand `hits` a changé).
    hit_at: Option<Instant>,
}

impl ClientPlayer {
    /// Range une vue reçue.
    pub fn receive_vitals(&mut self, vitals: VitalsView) {
        if self
            .vitals
            .as_ref()
            .is_some_and(|before| before.hits != vitals.hits)
        {
            self.hit_at = Some(Instant::now());
        }
        self.vitals = Some(vitals);
    }

    /// Les secondes depuis le dernier coup reçu (l'infini s'il n'y en a pas eu).
    pub fn since_hit(&self) -> f32 {
        self.hit_at
            .map_or(f32::INFINITY, |at| at.elapsed().as_secs_f32())
    }
}

/// Ce que le serveur a envoyé en dernier à chaque joueur, pour n'envoyer que ce qui change.
#[derive(Debug, Default)]
pub struct PlayerViews {
    vitals: HashMap<PlayerId, VitalsView>,
    inventories: HashMap<PlayerId, InventoryView>,
    containers: HashMap<PlayerId, Option<ContainerView>>,
    screens: HashMap<PlayerId, Option<crate::screen_actions::ScreenView>>,
    quests: HashMap<PlayerId, Option<crate::quest_progress::Progress>>,
}

/// Les vues d'un joueur à une fin d'image, avant de voir ce qui a changé.
pub struct PlayerUpdate<'a> {
    pub vitals: VitalsView,
    pub inventory: InventoryView,
    pub container: Option<ContainerView>,
    pub screen: Option<crate::screen_actions::ScreenView>,
    pub quests: Option<&'a crate::quest_progress::Progress>,
}

#[cfg(test)]
impl PlayerUpdate<'_> {
    /// Les jauges et l'inventaire, rien d'ouvert, pas de quêtes.
    pub fn new(vitals: VitalsView, inventory: InventoryView) -> Self {
        Self {
            vitals,
            inventory,
            container: None,
            screen: None,
            quests: None,
        }
    }
}

impl PlayerViews {
    /// Les messages d'un joueur : seulement ses vues qui ont changé.
    pub fn step(&mut self, id: PlayerId, update: PlayerUpdate<'_>, out: &mut Vec<ServerMsg>) {
        let PlayerUpdate {
            vitals,
            inventory,
            container,
            screen,
            quests,
        } = update;
        let known = self.quests.get(&id).and_then(Option::as_ref);
        if known != quests {
            let quests = quests.cloned();
            self.quests.insert(id, quests.clone());
            out.push(ServerMsg::Quests(quests));
        }
        let before = self.screens.get(&id).cloned().flatten();
        if before != screen {
            self.screens.insert(id, screen.clone());
            out.push(ServerMsg::ScreenView(screen));
        }
        if self.vitals.get(&id) != Some(&vitals) {
            self.vitals.insert(id, vitals.clone());
            out.push(ServerMsg::Vitals(vitals));
        }
        if self.inventories.get(&id) != Some(&inventory) {
            self.inventories.insert(id, inventory.clone());
            out.push(ServerMsg::Inventory(inventory));
        }
        // Rien d'ouvert la première fois : rien à envoyer.
        let before = self.containers.get(&id).cloned().flatten();
        if before != container {
            self.containers.insert(id, container.clone());
            out.push(ServerMsg::Container(container));
        }
    }

    pub fn forget(&mut self, id: PlayerId) {
        self.vitals.remove(&id);
        self.inventories.remove(&id);
        self.containers.remove(&id);
        self.screens.remove(&id);
        self.quests.remove(&id);
    }
}

/// Les jauges d'un joueur. `health` vient de l'ECS (le composant `Health` de son entité).
pub fn vitals_of(state: &PlayerState, health: [f32; 2], enchant_seed: u32) -> VitalsView {
    let (level, _) = state.experience.level();
    VitalsView {
        health,
        food: state.hunger.food,
        air: state.vitals.air,
        burning: state.vitals.burning > 0.0,
        hits: state.vitals.hits,
        level,
        progress: state.experience.progress(),
        points: state.experience.points,
        enchant_seed,
        forge_rarity: state.adventure.forge_rarity,
        forge_seed: state.adventure.forge_seed,
        tiers: state.adventure.progress,
        reach: state.gear.reach(),
        effects: state
            .effects
            .list()
            .iter()
            .map(|active| EffectView {
                effect: active.effect,
                seconds: active.left.max(0.0).ceil() as u32,
            })
            .collect(),
    }
}

impl VoxelGame {
    /// Fin d'image : chaque joueur reçoit ses jauges et son inventaire s'ils ont changé.
    pub(crate) fn replicate_player_views(&mut self) {
        let mut out = Vec::new();
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            // Le conteneur se décrit avec l'état de ce joueur (son `open`).
            let before = self.players.set_acting(Some(id));
            // Une bibliothèque ouverte range ce que les réseaux déposent dans sa case d'entrée.
            if let Some(crate::ui::Screen::Library(pos)) = self.players.me().screen {
                self.absorb_library(pos);
            }
            let container = self.open_container_view();
            let screen = self.screen_view();
            self.players.set_acting(before);
            let Some(player) = self.players.get(id) else {
                continue;
            };
            let health = crate::player::health(&self.ecs, player.entity);
            let vitals = vitals_of(player, [health.current, health.max], self.enchant_seed);
            let inventory = InventoryView::of(&player.inventory);
            let update = PlayerUpdate {
                vitals,
                inventory,
                container,
                screen,
                quests: player.quest_book.progress(),
            };
            self.view.player_views.step(id, update, &mut out);
            self.replies.extend(out.drain(..).map(|msg| (id, msg)));
        }
    }

    /// Un son pour le joueur qui agit (celui du message en cours, sinon celui de la fenêtre).
    pub(crate) fn sound(&mut self, key: &str) {
        let id = self.players.current_id();
        self.replies
            .push((id, ServerMsg::Feedback(Feedback::Sound(Sound::key(key)))));
    }

    /// Un son placé en `at`, pour tous les joueurs à portée d'oreille.
    pub(crate) fn sound_near(&mut self, sound: Sound, at: Vec3) {
        let heard: Vec<PlayerId> = self
            .players
            .iter()
            .filter(|p| {
                crate::player::eye(&self.ecs, p.entity, 1.0).distance_squared(at)
                    <= HEARING * HEARING
            })
            .map(|p| p.id)
            .collect();
        for id in heard {
            self.replies
                .push((id, ServerMsg::Feedback(Feedback::Sound(sound.clone()))));
        }
    }

    /// Le bras du joueur qui agit frappe.
    pub(crate) fn punch(&mut self) {
        let id = self.players.current_id();
        self.replies
            .push((id, ServerMsg::Feedback(Feedback::Punch)));
    }

    /// Un message en bas de l'écran du joueur qui agit.
    pub(crate) fn toast(&mut self, text: &str) {
        log::info!("{text}");
        let id = self.players.current_id();
        self.replies
            .push((id, ServerMsg::Feedback(Feedback::Toast(text.to_string()))));
    }

    /// La fenêtre applique un retour du serveur.
    pub(crate) fn apply_feedback(&mut self, feedback: Feedback) {
        match feedback {
            Feedback::Sound(Sound::Key(key)) => {
                if let Some(audio) = &mut self.audio {
                    audio.play(&key);
                }
            }
            Feedback::Sound(Sound::Block { block, kind }) => {
                if let Some(audio) = &mut self.audio {
                    audio.play_block(&self.registry, engine_voxel::BlockId(block), kind);
                }
            }
            Feedback::Toast(text) => {
                self.ui.toast = Some((text, Instant::now()));
            }
            Feedback::Punch => self.hand.punch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vitals(food: f32, hits: u32) -> VitalsView {
        VitalsView {
            health: [20.0, 20.0],
            food,
            air: 10.0,
            burning: false,
            hits,
            level: 0,
            progress: 0.0,
            points: 0,
            enchant_seed: 0,
            forge_rarity: 0,
            forge_seed: 0,
            tiers: crate::tiers::Progress::default(),
            reach: 4.5,
            effects: Vec::new(),
        }
    }

    #[test]
    fn views_are_sent_only_when_they_change() {
        let mut views = PlayerViews::default();
        let me = PlayerId(0);
        let mut out = Vec::new();
        views.step(
            me,
            PlayerUpdate::new(vitals(20.0, 0), InventoryView::default()),
            &mut out,
        );
        assert_eq!(out.len(), 2, "la première fois : les deux vues");
        out.clear();
        views.step(
            me,
            PlayerUpdate::new(vitals(20.0, 0), InventoryView::default()),
            &mut out,
        );
        assert!(out.is_empty(), "rien n'a changé");
        views.step(
            me,
            PlayerUpdate::new(vitals(19.0, 0), InventoryView::default()),
            &mut out,
        );
        assert!(matches!(out.as_slice(), [ServerMsg::Vitals(v)] if v.food == 19.0));
        out.clear();
        let inventory = InventoryView {
            selected: 3,
            ..InventoryView::default()
        };
        views.step(me, PlayerUpdate::new(vitals(19.0, 0), inventory), &mut out);
        assert!(matches!(out.as_slice(), [ServerMsg::Inventory(i)] if i.selected == 3));
    }

    #[test]
    fn two_players_have_their_own_views() {
        let mut views = PlayerViews::default();
        let mut out = Vec::new();
        views.step(
            PlayerId(0),
            PlayerUpdate::new(vitals(20.0, 0), InventoryView::default()),
            &mut out,
        );
        out.clear();
        views.step(
            PlayerId(1),
            PlayerUpdate::new(vitals(20.0, 0), InventoryView::default()),
            &mut out,
        );
        assert_eq!(
            out.len(),
            2,
            "le second joueur reçoit tout la première fois"
        );
        out.clear();
        views.forget(PlayerId(1));
        views.step(
            PlayerId(1),
            PlayerUpdate::new(vitals(20.0, 0), InventoryView::default()),
            &mut out,
        );
        assert_eq!(out.len(), 2, "un joueur oublié recommence à zéro");
    }

    #[test]
    fn the_open_container_is_sent_when_it_opens_changes_and_closes() {
        use crate::container_view::ContainerView;
        let chest = |count| ContainerView {
            raw: vec![Some(engine_voxel::ItemStack {
                item: "core:stone".into(),
                count,
                damage: 0,
            })],
            layout: engine_voxel::Layout::new(1, 0, 0),
            disabled: 0,
            tiered: false,
            furnace: [0.0; 2],
            brewing: 0.0,
            cart: None,
        };
        let mut views = PlayerViews::default();
        let me = PlayerId(0);
        let mut out = Vec::new();
        let mut send = |views: &mut PlayerViews, container| {
            out.clear();
            views.step(
                me,
                PlayerUpdate {
                    container,
                    ..PlayerUpdate::new(vitals(20.0, 0), InventoryView::default())
                },
                &mut out,
            );
            out.iter()
                .filter_map(|m| match m {
                    ServerMsg::Container(c) => Some(c.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(send(&mut views, None).is_empty(), "rien d'ouvert : rien");
        assert_eq!(send(&mut views, Some(chest(3))), vec![Some(chest(3))]);
        assert!(send(&mut views, Some(chest(3))).is_empty(), "inchangé");
        assert_eq!(send(&mut views, Some(chest(4))), vec![Some(chest(4))]);
        assert_eq!(send(&mut views, None), vec![None], "fermé");
        assert!(send(&mut views, None).is_empty());
    }

    #[test]
    fn a_new_hit_starts_the_red_flash() {
        let mut me = ClientPlayer::default();
        assert_eq!(me.since_hit(), f32::INFINITY);
        me.receive_vitals(vitals(20.0, 0));
        assert_eq!(
            me.since_hit(),
            f32::INFINITY,
            "la première vue n'est pas un coup"
        );
        me.receive_vitals(vitals(20.0, 1));
        assert!(me.since_hit() < 1.0);
    }

    #[test]
    fn the_inventory_view_mirrors_the_inventory() {
        let mut inventory = Inventory::default();
        let stack = ItemStack::new(engine_voxel::ItemId(2), 5);
        inventory.slots[4] = Some(stack);
        inventory.selected = 4;
        let view = InventoryView::of(&inventory);
        assert_eq!(view.slots.len(), SLOTS);
        assert_eq!(view.selected_stack(), Some(stack));
    }

    #[test]
    fn feedback_survives_postcard() {
        for feedback in [
            Feedback::Sound(Sound::key("core:gulp")),
            Feedback::Sound(Sound::Block {
                block: 7,
                kind: BlockSound::Break,
            }),
            Feedback::Toast("Niveau 3".into()),
            Feedback::Punch,
        ] {
            let bytes = postcard::to_allocvec(&feedback).unwrap();
            assert_eq!(postcard::from_bytes::<Feedback>(&bytes).unwrap(), feedback);
        }
        let view = vitals(12.5, 3);
        let bytes = postcard::to_allocvec(&view).unwrap();
        assert_eq!(postcard::from_bytes::<VitalsView>(&bytes).unwrap(), view);
    }
}
