//! Les gestes des écrans (hors des cases) : le serveur les applique pour le joueur courant, seulement
//! si l'écran qu'ils visent est celui qu'il lui a ouvert (`PlayerState::screen`, voir
//! `intents.rs::open_screen`). La fenêtre dessine ces écrans d'après ses vues et n'envoie que des
//! [`ScreenAction`].

use serde::{Deserialize, Serialize};

use crate::VoxelGame;
use crate::protocol::ScreenAction;
use crate::ui::Screen;

/// Ce qu'un écran montre et que seul le serveur sait (l'état d'un habitant...). Envoyée quand elle
/// change (`ServerMsg::ScreenView`), comme les autres vues du joueur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScreenView {
    /// L'habitant de l'écran d'échange : son identifiant (ses échanges sont dans les données du
    /// jeu), son nom, et combien de fois chaque échange peut encore se faire.
    Trade {
        key: String,
        name: String,
        uses: Vec<u32>,
    },
    /// Le menu d'une machine de mod, tel que son script l'a décrit (les octets de
    /// `engine_mods::menu`).
    Mod {
        owner: String,
        at: glam::IVec3,
        data: Vec<u8>,
    },
}

impl VoxelGame {
    /// La vue de l'écran ouvert par le joueur courant, si cet écran en a une.
    pub(crate) fn screen_view(&self) -> Option<ScreenView> {
        match self.server.players.me().screen? {
            Screen::Trade(id) => {
                let mob = self.server_entity(id)?;
                let key = self.mob_key(mob)?;
                let kind = self.server.ecs.get::<crate::components::Mob>(mob)?.kind;
                let name = self
                    .server
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .get(kind)
                    .map_or_else(|| key.clone(), |d| d.name.clone());
                let uses = self
                    .server
                    .ecs
                    .get::<crate::components::Trader>(mob)
                    .map(|t| t.uses.clone())
                    .unwrap_or_default();
                Some(ScreenView::Trade { key, name, uses })
            }
            Screen::Mod(_) => self
                .server
                .players
                .me()
                .mod_menu
                .as_ref()
                .map(|m| ScreenView::Mod {
                    owner: m.owner.clone(),
                    at: m.at,
                    data: m.data.clone(),
                }),
            _ => None,
        }
    }

    /// La vue d'écran que la fenêtre a reçue.
    pub(crate) fn shown_screen(&self) -> Option<&ScreenView> {
        self.client.view.me.screen.as_ref()
    }
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// `Trade` : faire l'échange `offer` de l'habitant ouvert, `times` fois au plus.
    fn trade(&mut self, id: crate::entities::NetId, offer: usize, times: u32) {
        let Some(mob) = self.server_entity(id) else {
            return;
        };
        let Some(key) = self.mob_key(mob) else {
            return;
        };
        let Some(trade) = self.trades.for_mob(&key).get(offer).cloned() else {
            return;
        };
        let items = self.content.items.clone();
        let mut done = 0;
        for _ in 0..times.min(64) {
            let left = self
                .ecs
                .get::<crate::components::Trader>(mob)
                .and_then(|t| t.uses.get(offer).copied())
                .unwrap_or(0);
            let Some(overflow) =
                crate::trading::execute(&mut self.players.me_mut().inventory, &items, &trade, left)
            else {
                break;
            };
            if let Some(mut trader) = self.ecs.get_mut::<crate::components::Trader>(mob)
                && let Some(uses) = trader.uses.get_mut(offer)
            {
                *uses = left - 1;
            }
            if overflow > 0 {
                let at = crate::player::body(&self.ecs, self.players.entity())
                    .position
                    .floor()
                    .as_ivec3();
                self.drop_stack(at, crate::inventory::ItemStack::new(trade.get.0, overflow));
            }
            done += 1;
        }
        if done > 0 {
            self.sound("core:place_hard");
        }
    }

    /// Applique un geste d'écran du joueur courant.
    pub(crate) fn screen_action(&mut self, action: ScreenAction) {
        let screen = self.players.me().screen;
        match (action, screen) {
            (ScreenAction::EnchantBuy { offer }, Some(Screen::Enchant(pos))) => {
                self.buy_enchant_offer(pos, usize::from(offer));
            }
            (ScreenAction::LibraryDepositCursor, Some(Screen::Library(pos))) => {
                self.library_deposit_cursor(pos);
            }
            (ScreenAction::LibraryDepositAll, Some(Screen::Library(pos))) => {
                self.library_deposit_all(pos);
            }
            (ScreenAction::LibraryExtract { kind, level }, Some(Screen::Library(pos))) => {
                if let Some(kind) = crate::enchanting::Enchantment::from_id(u32::from(kind)) {
                    self.extract_from_library(pos, kind, level);
                }
            }
            (ScreenAction::LibraryTakeOutput, Some(Screen::Library(pos))) => {
                self.library_take_output(pos);
            }
            (ScreenAction::Salvage, Some(Screen::Salvage)) => self.salvage_cursor(),
            (ScreenAction::ReforgeRarity(rarity), Some(Screen::Reforge)) => {
                let max = crate::adventure_data::current()
                    .rarities
                    .len()
                    .saturating_sub(1);
                self.players.me_mut().adventure.forge_rarity = rarity.min(max as u8);
            }
            (ScreenAction::ReforgeChoose(draw), Some(Screen::Reforge)) => {
                self.reforge_choose(usize::from(draw));
            }
            (ScreenAction::Carve { piece, all }, Some(Screen::Shaping)) => {
                let Some(kind) = engine_voxel::shaped::PieceKind::from_key(&piece) else {
                    return;
                };
                if let Err(why) = self.carve(crate::inventory::Slot::Craft(0), kind, all) {
                    self.toast(&why);
                }
            }
            (ScreenAction::Rebuild { all }, Some(Screen::Shaping)) => {
                if let Err(why) = self.rebuild(crate::inventory::Slot::Craft(0), all) {
                    self.toast(&why);
                }
            }
            (ScreenAction::Trade { offer, times }, Some(Screen::Trade(id))) => {
                self.trade(id, usize::from(offer), times);
            }
            // L'écran des paliers s'ouvre d'une touche, sans le serveur : le choix vaut partout.
            (ScreenAction::Tier(index), _) => self.choose_tier(index),
            // Le livre de quêtes s'ouvre d'une touche, sans le serveur : ses gestes valent partout.
            (ScreenAction::Quest(act), _) => self.quest_act(act),
            (ScreenAction::ModEvent { kind, id, value }, Some(Screen::Mod(_))) => {
                self.send_mod_ui(kind, id, value);
            }
            (ScreenAction::ControllerDeposit, Some(Screen::Controller(pos))) => {
                self.controller_deposit(pos, None);
            }
            (ScreenAction::ControllerPut, Some(Screen::Controller(pos))) => {
                self.controller_put(pos);
            }
            (ScreenAction::ControllerTake { item, damage, how }, Some(Screen::Controller(pos))) => {
                self.controller_take_for(pos, &item, damage, how)
            }
            (action, screen) => {
                log::warn!("geste d'écran refusé : {action:?} (écran ouvert : {screen:?})");
            }
        }
    }
}
