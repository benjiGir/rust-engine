//! Comment lancer un monde, et ce que le jeu demande à l'application : changer de monde ou quitter.

use std::path::PathBuf;

use crate::{env_parse, persistence};

/// Comment lancer un monde : depuis quel dossier, avec quelle graine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launch {
    /// Le dossier de sauvegarde (`None` : un monde jetable, rien n'est écrit).
    pub world: Option<PathBuf>,
    /// La graine d'un nouveau monde (un monde existant garde la sienne).
    pub seed: Option<u32>,
    /// Le monde de fond du menu principal : survol de caméra, sans joueur.
    pub title: bool,
    /// La difficulté d'un nouveau monde (0 paisible, 1 normale, 2 difficile).
    pub difficulty: Option<u8>,
    /// La dimension où arriver (`None` : celle que le monde sauvegardé indique, sinon celle de départ).
    pub dimension: Option<String>,
    /// Rejoindre la partie d'un hôte (« adresse:port ») au lieu d'ouvrir un monde (voir `net_client.rs`).
    pub join: Option<String>,
}

impl Launch {
    /// Les modes automatiques (captures, démo, vol, test de charge) et `VOXEL_WORLD`
    /// lancent un monde directement, sans menu, comme avant.
    pub fn from_env() -> Option<Self> {
        let automatic = [
            "VOXEL_DEMO",
            "VOXEL_AUTOFLY",
            "VOXEL_SCREENSHOTS",
            "VOXEL_STRESS",
            "VOXEL_BENCH",
        ]
        .iter()
        .any(|v| std::env::var(v).is_ok());
        let named = std::env::var("VOXEL_WORLD").is_ok();
        let join = std::env::var("VOXEL_JOIN").ok().filter(|a| !a.is_empty());
        if !automatic && !named && join.is_none() {
            return None;
        }
        Some(Self {
            // Les modes automatiques n'écrivent pas dans le monde du joueur, sauf demande explicite.
            world: named.then(|| persistence::world_dir(&crate::paths::get().saves)),
            seed: env_parse("VOXEL_SEED"),
            title: false,
            difficulty: None,
            dimension: None,
            join,
        })
    }

    /// Le monde de fond du menu principal.
    pub fn title() -> Self {
        Self {
            world: None,
            seed: Some(TITLE_SEED),
            title: true,
            difficulty: None,
            dimension: None,
            join: None,
        }
    }
}

/// La graine du monde de fond du menu.
const TITLE_SEED: u32 = 2024;

/// Ce que le jeu demande à l'application : changer de monde ou quitter.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Play(Launch),
    /// Retour au menu principal (le monde est sauvegardé et déchargé).
    Title,
    Quit,
}
