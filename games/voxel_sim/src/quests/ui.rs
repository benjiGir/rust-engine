//! La part serveur de `quest_ui.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `quest_ui.rs`).

use crate::net::protocol::QuestAct;

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// `ScreenAction::Quest` : le serveur applique un geste du livre au joueur courant.
    pub fn quest_act(&mut self, act: QuestAct) {
        if act == QuestAct::Open {
            self.open_quests();
            self.refresh_quests();
            if let Some(book) = self.players.me().quest_book.quests() {
                let id = self.players.current_id();
                self.replies
                    .push((id, crate::net::protocol::ServerMsg::QuestBook(book)));
            }
            return;
        }
        let Some(quests) = self.players.me().quest_book.quests() else {
            return;
        };
        let count = quests.quests.len();
        let quest_ok = |q: u32| (q as usize) < count;
        match act {
            QuestAct::LastChapter(id) => {
                if let Ok(progress) = self.players.me_mut().quest_book.progress_mut() {
                    progress.set_last_chapter(&id);
                }
            }
            QuestAct::Claim(quest) if quest_ok(quest) => match self.claim_quest(quest as usize) {
                Ok(gave) if gave.is_empty() => self.toast("Quête récupérée"),
                Ok(gave) => self.toast(&format!("Récupéré : {gave}")),
                Err(err) => self.toast(&err.to_string()),
            },
            QuestAct::ClaimAll => {
                let claimed = self.claim_all_quests();
                self.toast(&if claimed == 0 {
                    "Rien à récupérer (ou un choix à faire)".to_string()
                } else {
                    format!("{claimed} quête(s) récupérée(s)")
                });
            }
            QuestAct::Check(quest, task) if quest_ok(quest) => {
                if let Ok(progress) = self.players.me_mut().quest_book.progress_mut() {
                    progress.check(&quests, quest as usize, task as usize);
                }
                self.refresh_quests();
            }
            QuestAct::HandIn(quest, task) if quest_ok(quest) => {
                // Pas de bannière : la fiche montre le compteur, et la quête finie s'annonce.
                self.hand_in_quest(quest as usize, task as usize);
            }
            QuestAct::Choose(quest, reward, option) if quest_ok(quest) => {
                if let Ok(progress) = self.players.me_mut().quest_book.progress_mut() {
                    progress.choose(&quests, quest as usize, reward as usize, option as usize);
                }
            }
            QuestAct::Pin(quest) if quest_ok(quest) => {
                let id = quests.quests[quest as usize].id.clone();
                let changed = self
                    .players
                    .me_mut()
                    .quest_book
                    .progress_mut()
                    .is_ok_and(|p| p.toggle_pin(&id));
                if !changed {
                    self.toast("Trois quêtes épinglées au plus");
                }
            }
            act => log::warn!("geste du livre de quêtes refusé : {act:?}"),
        }
    }
}
