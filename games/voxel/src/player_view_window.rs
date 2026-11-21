//! La part fenêtre de `player_view.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `player_view.rs`).

use crate::VoxelGame;
#[allow(unused_imports)]
use crate::player_view::*;
use std::time::Instant;

impl VoxelGame {
    /// La fenêtre applique un retour du serveur.
    pub(crate) fn apply_feedback(&mut self, feedback: Feedback) {
        match feedback {
            Feedback::Sound(Sound::Key(key)) => {
                if let Some(audio) = &mut self.client.audio {
                    audio.play(&key);
                }
            }
            Feedback::Sound(Sound::Block { block, kind }) => {
                if let Some(audio) = &mut self.client.audio {
                    audio.play_block(&self.client.registry, engine_voxel::BlockId(block), kind);
                }
            }
            Feedback::Toast(text) => {
                self.client.ui.toast = Some((text, Instant::now()));
            }
            Feedback::Punch => self.client.hand.punch(),
            Feedback::TierUnlocked(name) => {
                let key = self
                    .settings
                    .keys
                    .name(crate::settings::Action::Tiers)
                    .to_string();
                let text = format!("Palier débloqué : {name} (touche {key})");
                log::info!("{text}");
                self.client.ui.toast = Some((text, Instant::now()));
            }
        }
    }
}
