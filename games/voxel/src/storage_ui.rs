//! L'écran des conteneurs : coffres (de base ou à paliers), wagons coffres, sacs à dos et contrôleur
//! de stockage. Un seul module pour tous : les boutons, le Maj + clic et la colonne des
//! améliorations ne sont écrits qu'une fois.
//!
//! # Un conteneur, quatre façons de le lire
//!
//! [`Target`] désigne le conteneur : un bloc (son inventaire est dans l'entité du bloc), un wagon
//! (`CartCargo`) ou un sac (le magasin de `backpacks.rs`). Chacun se lit et s'écrit comme une
//! liste de cases du moteur ([`engine_voxel::ItemStack`], objets par identifiant texte). Les
//! gestes en gros (Déposer, Compléter, Tout prendre, Trier, Maj + clic) convertissent les cases
//! principales en piles du jeu, appellent la logique pure de `transfer.rs`, puis réécrivent.
//!
//! # La disposition des cases
//!
//! Un coffre à paliers et un sac rangent leurs cases en blocs : les cases principales, puis les
//! emplacements d'amélioration, puis les filtres (voir `engine_voxel::storage`). L'écran dessine
//! les améliorations dans un bandeau à part en haut, puis les cases principales, qui défilent si
//! elles sont nombreuses (l'inventaire du joueur reste visible). Les filtres sont
//! des cases fantômes : on y pose un exemplaire d'un objet (rien n'est consommé), un clic le retire.
//! Après chaque geste, les améliorations passives (compactage, néant) s'appliquent tout de suite.
//!
//! # Le contrôleur
//!
//! Son écran ne montre pas de cases mais les objets de tous les coffres reliés, regroupés par
//! sorte, avec une recherche. Un clic prend une pile, Maj + clic tout, et poser la pile tenue (ou
//! Déposer) range dans les coffres en complétant d'abord les piles.

use engine_ecs::Entity;
use engine_render::egui;
use engine_voxel::{
    CHEST_TIERS, CONTROLLER_KEY, Layout, Policy, SlotKind, Upgrade, Voiding, chest_layout,
    chest_tier_of, controller_groups, controller_insert, controller_members, controller_take,
    ensure_layout, kit_target, layout_of, upgrade_chest,
};
use glam::IVec3;

use crate::VoxelGame;
use crate::backpacks;
use crate::inventory::{Click, ItemStack, Slot, click_slot_mult};
use crate::players::OpenContainer;
use crate::transfer::{self, Rules, Voids};
use crate::ui::{Press, Screen, SlotRef};

/// Un conteneur dont l'écran offre les gestes en gros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Block(IVec3),
    Cart(Entity),
    Bag(u32),
}

impl Target {
    fn slot_ref(self, index: usize) -> SlotRef {
        match self {
            Self::Block(pos) => SlotRef::Container(pos, index),
            Self::Cart(cart) => SlotRef::Cart(cart, index),
            Self::Bag(number) => SlotRef::Bag(number, index),
        }
    }
}

/// Un geste sur un conteneur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Op {
    /// Chaque pile de l'inventaire principal va dans le conteneur.
    Deposit,
    /// Pareil, mais seulement pour les objets déjà présents.
    TopUp,
    /// Tout le conteneur va dans l'inventaire.
    TakeAll,
    Sort,
    /// Maj + clic sur une case du joueur (son numéro dans l'inventaire).
    QuickIn(usize),
    /// Maj + clic sur une case du conteneur.
    QuickOut(usize),
}

/// Le conteneur lu : ses cases, leur disposition, et ce qu'en font les améliorations.
struct View {
    raw: Vec<Option<engine_voxel::ItemStack>>,
    layout: Layout,
    disabled: u32,
    /// `true` pour un conteneur à paliers ou un sac (cases d'amélioration et filtres).
    tiered: bool,
}

/// Une pile du moteur, en pile du jeu (`None` si l'objet est inconnu : mod absent).
pub(crate) fn to_game(
    items: &engine_voxel::ItemRegistry,
    s: &engine_voxel::ItemStack,
) -> Option<ItemStack> {
    Some(ItemStack {
        item: items.id(&s.item)?,
        count: s.count,
        damage: s.damage,
    })
}

pub(crate) fn to_engine(
    items: &engine_voxel::ItemRegistry,
    s: ItemStack,
) -> engine_voxel::ItemStack {
    engine_voxel::ItemStack {
        item: items.get(s.item).key.clone(),
        count: s.count,
        damage: s.damage,
    }
}

impl VoxelGame {
    // ----- Lire et écrire un conteneur -----

    /// Le conteneur en entier (copie), avec sa disposition.
    fn view(&mut self, t: Target) -> Option<View> {
        let (mut raw, layout, disabled) = match t {
            Target::Block(pos) => {
                let block = self.streamer().world().block(pos);
                let raw = self
                    .streamer()
                    .block_entity(pos)?
                    .caps
                    .as_ref()?
                    .inventory
                    .as_ref()?
                    .slots
                    .clone();
                (raw, layout_of(&self.registry, block), 0)
            }
            Target::Cart(cart) => {
                let raw = self
                    .ecs
                    .get::<crate::carts::CartCargo>(cart)?
                    .0
                    .slots
                    .clone();
                (raw, None, 0)
            }
            Target::Bag(number) => {
                let bag = self.backpacks.bag(number)?;
                (bag.slots.clone(), Some(bag.layout()), bag.disabled)
            }
        };
        let tiered = layout.is_some();
        let layout = layout.unwrap_or(Layout::new(raw.len(), 0, 0));
        // Un conteneur d'avant les paliers n'a pas encore ses emplacements d'amélioration.
        if tiered {
            ensure_layout(&mut raw, layout);
        }
        Some(View {
            raw,
            layout,
            disabled,
            tiered,
        })
    }

    /// Réécrit le conteneur, après avoir laissé les améliorations passives travailler.
    fn store(&mut self, t: Target, mut view: View) {
        let items = self.content.items.clone();
        if view.tiered {
            let table = self.content.compact.clone();
            engine_voxel::maintain(
                &mut view.raw,
                view.layout,
                view.disabled,
                Some(&items),
                &table,
            );
        }
        match t {
            Target::Block(pos) => {
                let wakes = view.tiered;
                if let Some(inv) = self
                    .streamer_mut()
                    .block_entity_mut(pos)
                    .and_then(|e| e.caps.as_mut()?.inventory.as_mut())
                {
                    inv.slots = view.raw;
                }
                if wakes {
                    // Les améliorations qui travaillent seules (entonnoir, compactage) se réveillent.
                    self.mods
                        .machines
                        .schedule(engine_voxel::GridPos::world(pos), 1);
                }
            }
            Target::Cart(cart) => {
                if let Some(mut cargo) = self.ecs.get_mut::<crate::carts::CartCargo>(cart) {
                    cargo.0.slots = view.raw;
                }
            }
            Target::Bag(number) => {
                if let Some(bag) = self.backpacks.bag_mut(number) {
                    bag.slots = view.raw;
                }
            }
        }
    }

    /// Le contenu d'un conteneur pour la vue de la fenêtre (cases, disposition, améliorations
    /// coupées, à paliers). Un conteneur ancien (27 cases) reçoit ses emplacements d'amélioration dès
    /// qu'il est ouvert.
    pub(crate) fn open_contents(
        &mut self,
        t: Target,
    ) -> Option<(Vec<Option<engine_voxel::ItemStack>>, Layout, u32, bool)> {
        let mut view = self.view(t)?;
        if view.tiered && self.target_len(t) < Some(view.layout.total()) {
            self.store(
                t,
                View {
                    raw: view.raw.clone(),
                    layout: view.layout,
                    disabled: view.disabled,
                    tiered: true,
                },
            );
            view = self.view(t)?;
        }
        Some((view.raw, view.layout, view.disabled, view.tiered))
    }

    /// Les règles de rangement d'un conteneur : piles plus grandes, refus des sacs dans un sac,
    /// Néant.
    fn rules_of<'a>(
        &self,
        items: &'a engine_voxel::ItemRegistry,
        view: &View,
        accepts: &'a dyn Fn(engine_voxel::ItemId) -> bool,
    ) -> Rules<'a> {
        let policy = Policy::of(&view.raw, view.layout, view.disabled);
        Rules {
            items,
            mult: policy.mult,
            accepts,
            voids: match &policy.void {
                Voiding::None => Voids::Nothing,
                Voiding::Overflow => Voids::Overflow,
                Voiding::Filter(list) => {
                    Voids::Items(list.iter().filter_map(|k| items.id(k)).collect())
                }
            },
        }
    }

    // ----- Les gestes en gros -----

    /// Applique un geste au conteneur.
    pub(crate) fn storage_transfer(&mut self, t: Target, op: Op) {
        let Some(mut view) = self.view(t) else {
            return;
        };
        let items = self.content.items.clone();
        // Une amélioration qu'on Maj + clique va dans un emplacement d'amélioration libre.
        if let Op::QuickIn(slot) = op
            && self.quick_upgrade(&mut view, slot, matches!(t, Target::Bag(_)))
        {
            self.store(t, view);
            return;
        }
        // Les cases principales, en piles du jeu. Une case d'un objet inconnu n'est jamais touchée.
        let mut cslots: Vec<Option<ItemStack>> = vec![None; view.raw.len()];
        let mut indices = Vec::new();
        for i in view.layout.main_range() {
            match view.raw.get(i).and_then(Option::as_ref) {
                None => indices.push(i),
                Some(s) => {
                    if let Some(stack) = to_game(&items, s) {
                        cslots[i] = Some(stack);
                        indices.push(i);
                    }
                }
            }
        }
        let is_bag = matches!(t, Target::Bag(_));
        let accepts = |item| !is_bag || backpacks::accepts(&items, item);
        let crules = self.rules_of(&items, &view, &accepts);
        let prules = Rules::plain(&items);
        let main: Vec<usize> = (9..36).collect();
        let everywhere: Vec<usize> = (9..36).chain(0..9).collect();
        match op {
            Op::Deposit => {
                transfer::move_all(
                    &mut self.players.me_mut().inventory.slots,
                    &main,
                    &mut cslots,
                    &indices,
                    &crules,
                    false,
                );
            }
            Op::TopUp => {
                transfer::move_all(
                    &mut self.players.me_mut().inventory.slots,
                    &main,
                    &mut cslots,
                    &indices,
                    &crules,
                    true,
                );
            }
            Op::TakeAll => {
                transfer::move_all(
                    &mut cslots,
                    &indices,
                    &mut self.players.me_mut().inventory.slots,
                    &everywhere,
                    &prules,
                    false,
                );
            }
            Op::Sort => transfer::sort(&mut cslots, &indices, &crules),
            Op::QuickIn(slot) => {
                transfer::move_slot(
                    &mut self.players.me_mut().inventory.slots,
                    slot,
                    &mut cslots,
                    &indices,
                    &crules,
                );
            }
            Op::QuickOut(index) => {
                transfer::move_slot(
                    &mut cslots,
                    index,
                    &mut self.players.me_mut().inventory.slots,
                    &everywhere,
                    &prules,
                );
            }
        }
        for &i in &indices {
            view.raw[i] = cslots[i].map(|s| to_engine(&items, s));
        }
        self.store(t, view);
    }

    /// Maj + clic sur une amélioration de l'inventaire : elle va dans le premier emplacement libre
    /// (un seul exemplaire). `false` si ce n'en est pas une, ou si elle ne convient pas ici.
    fn quick_upgrade(&mut self, view: &mut View, slot: usize, is_bag: bool) -> bool {
        let Some(stack) = self.players.me_mut().inventory.slots[slot] else {
            return false;
        };
        let key = self.content.items.get(stack.item).key.clone();
        let fits = Upgrade::from_key(&key).is_some_and(|u| {
            if is_bag {
                u.in_backpack()
            } else {
                u.in_chest()
            }
        });
        let free = view
            .layout
            .upgrade_range()
            .find(|&i| view.raw.get(i).is_some_and(Option::is_none));
        let (true, Some(free)) = (fits, free) else {
            return false;
        };
        view.raw[free] = Some(engine_voxel::ItemStack {
            item: key,
            count: 1,
            damage: 0,
        });
        if let Some(s) = self.players.me_mut().inventory.slots[slot].as_mut() {
            s.count -= 1;
            if s.count == 0 {
                self.players.me_mut().inventory.slots[slot] = None;
            }
        }
        true
    }

    /// Le conteneur de l'écran ouvert, s'il en a un (c'est là que va un Maj + clic).
    pub(crate) fn open_target(&self) -> Option<Target> {
        match self.players.me().open? {
            OpenContainer::Chest(pos) => Some(Target::Block(pos)),
            OpenContainer::Bag(number) => Some(Target::Bag(number)),
            OpenContainer::Cart(cart) => self
                .ecs
                .get::<crate::carts::Cart>(cart)
                .filter(|c| c.kind != crate::carts::CartKind::Locomotive)
                .map(|_| Target::Cart(cart)),
            OpenContainer::Machine(_) | OpenContainer::Controller(_) => None,
        }
    }

    /// Le Maj + clic est-il possible ici ? (un conteneur ouvert, la main vide)
    pub(crate) fn quick_enabled(&self) -> bool {
        self.players.me().inventory.cursor.is_none()
            && (self.open_target().is_some()
                || matches!(self.players.me().open, Some(OpenContainer::Controller(_))))
    }

    /// Maj + clic sur une case : toute la pile passe de l'autre côté. `false` si ce n'est pas
    /// une case que ce geste concerne (amélioration, filtre : un clic ordinaire s'en occupe).
    pub(crate) fn quick_move(&mut self, target: SlotRef) -> bool {
        match (target, self.players.me().open) {
            (SlotRef::Player(Slot::Main(slot)), Some(OpenContainer::Controller(pos))) => {
                self.controller_deposit(pos, Some(slot));
                true
            }
            (SlotRef::Player(Slot::Main(slot)), _) => match self.open_target() {
                Some(t) => {
                    self.storage_transfer(t, Op::QuickIn(slot));
                    true
                }
                None => false,
            },
            (
                SlotRef::Container(_, index) | SlotRef::Cart(_, index) | SlotRef::Bag(_, index),
                _,
            ) => {
                let Some(t) = self.open_target() else {
                    return false;
                };
                let is_main = self
                    .view(t)
                    .is_some_and(|v| v.layout.kind_of(index) == Some(SlotKind::Main));
                if is_main {
                    self.storage_transfer(t, Op::QuickOut(index));
                }
                is_main
            }
            _ => false,
        }
    }

    // ----- Un clic sur une case -----

    /// Un clic sur une case d'un conteneur : une case principale se comporte comme n'importe quelle
    /// case (avec la taille de pile du conteneur), un emplacement d'amélioration n'accepte
    /// qu'une amélioration, un filtre est fantôme.
    pub(crate) fn click_storage(&mut self, t: Target, index: usize, click: Click) -> bool {
        let Some(mut view) = self.view(t) else {
            return false;
        };
        let items = self.content.items.clone();
        let kind = view.layout.kind_of(index).unwrap_or(SlotKind::Main);
        let mut slot = view
            .raw
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|s| to_game(&items, s));
        // Un objet inconnu (mod absent) reste tel quel : on ne l'écrase pas par un clic.
        if slot.is_none() && view.raw.get(index).is_some_and(Option::is_some) {
            return false;
        }
        let mut cursor = self.players.me_mut().inventory.cursor.take();
        match kind {
            SlotKind::Main => {
                let is_bag = matches!(t, Target::Bag(_));
                let accepts = |item| !is_bag || backpacks::accepts(&items, item);
                let mult = self.rules_of(&items, &view, &accepts).mult;
                click_slot_mult(&mut slot, &mut cursor, &items, accepts, click, mult);
            }
            SlotKind::Upgrade => {
                let is_bag = matches!(t, Target::Bag(_));
                let fits = |item: engine_voxel::ItemId| {
                    Upgrade::from_key(&items.get(item).key).is_some_and(|u| {
                        if is_bag {
                            u.in_backpack()
                        } else {
                            u.in_chest()
                        }
                    })
                };
                upgrade_click(&mut slot, &mut cursor, fits);
            }
            SlotKind::Filter(_) => ghost_click(&mut slot, &mut cursor, &items),
        }
        self.players.me_mut().inventory.cursor = cursor;
        if let Some(cell) = view.raw.get_mut(index) {
            *cell = slot.map(|s| to_engine(&items, s));
        }
        self.store(t, view);
        true
    }

    // ----- L'écran -----

    /// Le titre de l'écran d'un conteneur.
    pub(crate) fn storage_title(&self) -> String {
        match self.ui.screen {
            Screen::Chest(pos) => {
                let block = self.streamer().world().block(pos);
                let name = &self.registry.def(block).name;
                if engine_voxel::double_chest::paired(&*self.streamer(), pos).is_some() {
                    // « Coffre de fer » devient « Grand coffre de fer ».
                    let mut chars = name.chars();
                    let first = chars.next().map(|c| c.to_lowercase().to_string());
                    format!("Grand {}{}", first.unwrap_or_default(), chars.as_str())
                } else {
                    name.clone()
                }
            }
            Screen::Bag(number) => self
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
    pub(crate) fn storage_panel(&mut self, ui: &mut egui::Ui, t: Target) {
        // La vue que le serveur a envoyée (voir `container_view.rs`) ; un conteneur ancien y a déjà
        // reçu ses emplacements d'amélioration.
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
        // Les améliorations d'abord, dans un bandeau à part : on ne les confond pas avec le stockage.
        if layout.upgrades > 0 {
            egui::Frame::new()
                .fill(UPGRADE_BAND)
                .stroke(egui::Stroke::new(1.0, crate::skin::BORDER))
                .corner_radius(4.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Améliorations").strong());
                        ui.add_space(8.0);
                        ui.spacing_mut().item_spacing.x = crate::ui_layout::SLOT_GAP;
                        for n in 0..layout.upgrades {
                            self.storage_slot(ui, t, layout.main + n);
                        }
                        ui.add_space(8.0);
                        if let Target::Bag(_) = t
                            && !ups.is_empty()
                        {
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
                                        self.send_now(crate::protocol::ClientMsg::ToggleUpgrade(
                                            n as u8,
                                        ));
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
                crate::ui_layout::slots(ui, layout.main, 9, |ui, i| {
                    self.storage_slot(ui, t, i);
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
            crate::ui_layout::slots(ui, range.len(), range.len(), |ui, i| {
                self.storage_slot(ui, t, range.start + i);
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
            self.send_now(crate::protocol::ClientMsg::ContainerOp(op));
        }
    }

    fn target_len(&self, t: Target) -> Option<usize> {
        match t {
            Target::Block(pos) => self
                .streamer()
                .block_entity(pos)?
                .caps
                .as_ref()?
                .inventory
                .as_ref()
                .map(|i| i.slots.len()),
            Target::Cart(cart) => self
                .ecs
                .get::<crate::carts::CartCargo>(cart)
                .map(|c| c.0.slots.len()),
            Target::Bag(number) => self.backpacks.bag(number).map(|b| b.slots.len()),
        }
    }

    fn storage_slot(&mut self, ui: &mut egui::Ui, t: Target, index: usize) -> bool {
        let stack = self.shown_stack(index);
        match self.stack_widget(ui, stack, false) {
            Some(press) => self.press_slot(t.slot_ref(index), stack, press),
            None => false,
        }
    }

    // ----- Le contrôleur -----

    /// Range une pile de l'inventaire (ou toute la réserve, `None`) dans les coffres reliés.
    pub(crate) fn controller_deposit(&mut self, pos: IVec3, only: Option<usize>) {
        let items = self.content.items.clone();
        let slots: Vec<usize> = match only {
            Some(slot) => vec![slot],
            None => (9..36).collect(),
        };
        let world = self.world.clone();
        let mut streamer = world.borrow_mut();
        let members = controller_members(&*streamer, pos);
        for slot in slots {
            let Some(stack) = self.players.me_mut().inventory.slots[slot] else {
                continue;
            };
            let key = items.get(stack.item).key.clone();
            let stored = controller_insert(
                &mut *streamer,
                &members,
                Some(&items),
                &key,
                stack.damage,
                stack.count,
            );
            if stored >= stack.count {
                self.players.me_mut().inventory.slots[slot] = None;
            } else if let Some(s) = self.players.me_mut().inventory.slots[slot].as_mut() {
                s.count = stack.count - stored;
            }
        }
    }

    /// L'écran du contrôleur : les objets réunis, une recherche, un clic pour en prendre.
    pub(crate) fn controller_panel(&mut self, ui: &mut egui::Ui, pos: IVec3) {
        let items = self.content.items.clone();
        let (members, groups) = {
            let streamer = self.streamer();
            let members = controller_members(&*streamer, pos);
            let groups = controller_groups(&*streamer, &members);
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
                egui::TextEdit::singleline(&mut self.ui.storage_search)
                    .text_color(crate::skin::TEXT)
                    .desired_width(ui.available_width().min(220.0)),
            );
            if ui.button("Déposer").clicked() {
                self.controller_deposit(pos, None);
            }
        });
        let query = self.ui.storage_search.clone();
        let shown: Vec<(engine_voxel::ItemId, engine_voxel::Group)> = groups
            .into_iter()
            .filter_map(|g| Some((items.id(&g.item)?, g)))
            .filter(|(id, _)| crate::recipe_browser::matches(items.get(*id), &query))
            .collect();
        let mut pressed: Option<(Option<(engine_voxel::ItemId, engine_voxel::Group)>, Press)> =
            None;
        egui::ScrollArea::vertical()
            .max_height((ui.ctx().content_rect().height() * 0.4).clamp(80.0, 320.0))
            .show(ui, |ui| {
                // Une case de plus, vide : poser ici la pile tenue.
                crate::ui_layout::slots(ui, shown.len() + 1, 9, |ui, i| {
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
        let world = self.world.clone();
        match (group, self.players.me_mut().inventory.cursor) {
            // La pile tenue est rangée dans les coffres.
            (_, Some(held)) => {
                let key = items.get(held.item).key.clone();
                let stored = controller_insert(
                    &mut *world.borrow_mut(),
                    &members,
                    Some(&items),
                    &key,
                    held.damage,
                    held.count,
                );
                self.players.me_mut().inventory.cursor =
                    (stored < held.count).then_some(ItemStack {
                        count: held.count - stored,
                        ..held
                    });
            }
            (Some((id, g)), None) => {
                let max = items.get(id).max_stack.max(1);
                let wanted = match press {
                    Press::Quick => g.count,
                    Press::Secondary | Press::Split => max.div_ceil(2).min(g.count),
                    Press::Primary => max.min(g.count),
                };
                let taken = controller_take(
                    &mut *world.borrow_mut(),
                    &members,
                    &g.item,
                    g.damage,
                    wanted,
                );
                let stack = ItemStack {
                    item: id,
                    count: taken,
                    damage: g.damage,
                };
                if press == Press::Quick {
                    // Tout, directement dans l'inventaire : ce qui n'y tient pas retourne aux coffres.
                    let left = self.players.me_mut().inventory.add_stack(stack, &items);
                    if left > 0 {
                        controller_insert(
                            &mut *world.borrow_mut(),
                            &members,
                            Some(&items),
                            &g.item,
                            g.damage,
                            left,
                        );
                    }
                } else if taken > 0 {
                    self.players.me_mut().inventory.cursor = Some(stack);
                }
            }
            (None, None) => {}
        }
    }

    // ----- Ouvrir -----

    /// Un clic droit sur un établi, un four, un coffre (de n'importe quel palier) ou le contrôleur
    /// ouvre son écran ; avec un kit ou un sac en main, il monte le coffre de palier ou ouvre le
    /// sac. `true` si c'est fait.
    pub(crate) fn open_container(&mut self) -> bool {
        if self.use_storage_item() {
            return true;
        }
        let Some(hit) = self.aim().hit else {
            return false;
        };
        // Un sac posé : son contenu, comme depuis l'inventaire.
        if let Some(number) = self.placed_bag_number(hit.block_pos) {
            self.open_container_screen(Screen::Bag(number));
            return true;
        }
        // Un grand coffre s'ouvre sur sa moitié gauche, qui garde l'inventaire.
        let opened = engine_voxel::double_chest::storage_pos(&*self.streamer(), hit.block_pos);
        let block = self.streamer().world().block(opened);
        let def = self.registry.def(block);
        let screen = match def.behavior.as_deref() {
            Some(b) if chest_layout(b).is_some() => Screen::Chest(opened),
            Some(CONTROLLER_KEY) => Screen::Controller(hit.block_pos),
            Some("core:furnace") => Screen::Furnace(hit.block_pos),
            Some("core:brewing_stand") => Screen::Brewing(hit.block_pos),
            _ if self.registry.def(self.registry.base(block)).key == "core:workbench" => {
                Screen::Workbench
            }
            Some(crate::enchant_library::BEHAVIOR) => Screen::Library(hit.block_pos),
            _ if def.key == "core:enchanting_table" => Screen::Enchant(hit.block_pos),
            _ if def.key == "core:simple_anvil" => Screen::Anvil,
            _ if def.key == "core:salvage_table" => Screen::Salvage,
            _ if def.key == "core:reforge_table" => Screen::Reforge,
            _ if def.key == crate::shaping_ui::TABLE => Screen::Shaping,
            _ => return false,
        };
        // Un conteneur ancien (ou posé par un mod) n'a peut-être pas encore son inventaire.
        if let Some(slots) = engine_voxel::container_slots(&self.registry, block) {
            let mut streamer = self.streamer_mut();
            if streamer
                .block_entity(opened)
                .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
                .is_none()
            {
                let mut entity = streamer.block_entity(opened).cloned().unwrap_or_default();
                entity.caps_mut().inventory = Some(engine_voxel::Inventory::new(slots));
                streamer.set_block_entity(opened, Some(entity));
            }
        }
        // Les objets à affixes d'un coffre de structure sont tirés à la première ouverture.
        if let Screen::Chest(pos) = screen {
            self.resolve_chest_loot(pos);
        }
        self.open_container_screen(screen);
        true
    }

    /// Le bloc visé s'ouvre-t-il (un clic droit avec un sac ne doit pas le voler) ?
    fn aimed_block_opens(&self) -> bool {
        let Some(hit) = self.aim().hit else {
            return false;
        };
        let block = self.streamer().world().block(hit.block_pos);
        let def = self.registry.def(block);
        matches!(
            def.behavior.as_deref(),
            Some(b) if chest_layout(b).is_some()
                || b == CONTROLLER_KEY
                || b == "core:furnace"
                || b == "core:brewing_stand"
        ) || self.registry.def(self.registry.base(block)).key == "core:workbench"
            || def.key == "core:enchanting_table"
            || def.key == "core:simple_anvil"
            || def.behavior.as_deref() == Some(crate::enchant_library::BEHAVIOR)
            || def.behavior.as_deref() == Some(crate::backpack_block::BEHAVIOR)
            || def.key == "core:salvage_table"
            || def.key == "core:reforge_table"
            || def.key == crate::shaping_ui::TABLE
    }

    /// Le kit en main sur un coffre, ou le sac en main (hors d'un bloc qui s'ouvre).
    fn use_storage_item(&mut self) -> bool {
        let Some(held) = self.players.me_mut().inventory.selected_stack() else {
            return false;
        };
        let key = self.content.items.get(held.item).key.clone();
        if engine_voxel::kit_of(&key).is_some() {
            return self.use_kit(&key);
        }
        if backpacks::is_backpack(&key) && !self.aimed_block_opens() {
            let slot = self.players.me_mut().inventory.selected;
            return self.open_bag_at(slot);
        }
        false
    }

    /// Un kit sur un coffre : le bloc monte de palier, l'entité garde ses objets.
    fn use_kit(&mut self, kit: &str) -> bool {
        let Some(hit) = self.aim().hit else {
            return false;
        };
        let pos = hit.block_pos;
        let block = self.streamer().world().block(pos);
        let Some(tier) = self
            .registry
            .def(block)
            .behavior
            .as_deref()
            .and_then(chest_tier_of)
        else {
            return false;
        };
        let Some(to) = kit_target(kit, tier) else {
            self.toast("Ce kit ne convient pas à ce coffre");
            return true;
        };
        if !upgrade_chest(&mut *self.streamer_mut(), pos, to) {
            return false;
        }
        let actions = self.mods.block_changed(pos);
        self.apply_machine_actions(actions);
        self.players.me_mut().inventory.consume_selected();
        self.target = None;
        self.punch();
        self.sound("core:place_hard");
        self.toast(&format!("{} !", CHEST_TIERS[to].name));
        true
    }

    /// Une moitié de grand coffre qu'on casse ne garde que ses propres objets ; l'autre
    /// redevient un coffre simple avec les siens.
    pub(crate) fn split_chest(&mut self, pos: IVec3) -> Option<IVec3> {
        engine_voxel::double_chest::split(&mut *self.streamer_mut(), pos)
    }

    /// Un coffre posé contre un coffre identique forme un grand coffre.
    pub(crate) fn join_chest(&mut self, pos: IVec3) -> Option<IVec3> {
        let registry = self.registry.clone();
        let streamer = self.streamer();
        let block = streamer.world().block(pos);
        engine_voxel::double_chest::part(&registry, block)?;
        // Un coffre de structure tire ses objets avant de rejoindre l'autre.
        let neighbours: Vec<IVec3> = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
            .iter()
            .map(|d| pos + *d)
            .filter(|&p| streamer.world().block(p) == block)
            .collect();
        drop(streamer);
        for p in neighbours {
            self.resolve_chest_loot(p);
        }
        engine_voxel::double_chest::merge(&mut *self.streamer_mut(), pos)
    }

    /// Ce qui tombe quand le bloc en `pos` change : ses objets, sans les filtres (cases fantômes).
    pub(crate) fn container_contents(&self, pos: IVec3) -> Vec<ItemStack> {
        let streamer = self.streamer();
        let block = streamer.world().block(pos);
        // Un sac posé qu'on casse rend le même sac, numéro compris.
        if let Some(bag) = crate::backpack_block::broken_bag(
            &self.registry,
            block,
            streamer.block_entity(pos),
            &self.content.items,
        ) {
            return vec![bag];
        }
        let Some(inventory) = streamer
            .block_entity(pos)
            .and_then(|e| e.caps.as_ref()?.inventory.clone())
        else {
            return Vec::new();
        };
        let layout = layout_of(&self.registry, block);
        let mut out: Vec<ItemStack> = inventory
            .slots
            .into_iter()
            .enumerate()
            .filter(|(i, _)| {
                layout.is_none_or(|l| !matches!(l.kind_of(*i), Some(SlotKind::Filter(_))))
            })
            .filter_map(|(_, s)| to_game(&self.content.items, &s?))
            .collect();
        // Une bibliothèque cassée rend ses points sous forme de livres enchantés.
        out.extend(crate::enchant_library::broken_books(
            &self.registry,
            block,
            streamer.block_entity(pos),
            &self.content.items,
        ));
        out
    }
}

/// Le fond du bandeau des améliorations : plus sombre que le panneau, pour le détacher du stockage.
const UPGRADE_BAND: egui::Color32 = egui::Color32::from_rgb(158, 162, 150);

/// Un emplacement d'amélioration : une seule pose à la fois, que des améliorations.
fn upgrade_click(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    fits: impl Fn(engine_voxel::ItemId) -> bool,
) {
    match cursor {
        Some(held) if fits(held.item) => {
            let one = ItemStack { count: 1, ..*held };
            if slot.is_none() {
                held.count -= 1;
                if held.count == 0 {
                    *cursor = None;
                }
                *slot = Some(one);
            } else if held.count == 1 {
                std::mem::swap(slot, cursor);
            }
        }
        Some(_) => {}
        None => *cursor = slot.take(),
    }
}

/// Une case filtre : poser en copie un exemplaire (rien n'est consommé), ou la vider.
fn ghost_click(
    slot: &mut Option<ItemStack>,
    cursor: &mut Option<ItemStack>,
    items: &engine_voxel::ItemRegistry,
) {
    match cursor {
        // Un sac ne se filtre pas : son numéro n'a pas de sens ici.
        Some(held) if !backpacks::is_backpack(&items.get(held.item).key) => {
            *slot = Some(ItemStack::new(held.item, 1));
        }
        Some(_) => {}
        None => *slot = None,
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
