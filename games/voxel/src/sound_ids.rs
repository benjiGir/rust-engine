//! Les sons que le serveur désigne aux joueurs (la simulation ne joue rien : la fenêtre les joue
//! avec `audio.rs`).

/// Le type de son d'un bloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlockSound {
    Step,
    Break,
    Place,
}
