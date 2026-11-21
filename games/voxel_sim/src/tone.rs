//! Le ton d'une ligne de texte que la simulation prépare (bulle d'aide d'un objet) : la fenêtre en
//! fait une couleur (`skin::tone`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code, reason = "tous les tons ne servent pas encore")]
pub enum Tone {
    Text,
    Muted,
    Red,
    Sage,
    Gold,
    Rgb(u8, u8, u8),
}
