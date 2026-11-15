//! Même donnée sRGB que le jeu ; seuls les champs graphiques sont nécessaires ici.
use serde::Deserialize;
use std::sync::LazyLock;
#[derive(Deserialize)]
pub struct Color {
    pub item: String,
    pub rgb: (u8, u8, u8),
}
pub fn colors() -> &'static [Color] {
    static COLORS: LazyLock<Vec<Color>> = LazyLock::new(|| {
        ron::from_str(include_str!("../../../assets/core/wool_colors.ron"))
            .expect("palette de laine invalide")
    });
    &COLORS
}
pub fn find(name: &str) -> Option<&'static Color> {
    colors()
        .iter()
        .find(|c| c.item.strip_prefix("core:") == Some(name))
}
