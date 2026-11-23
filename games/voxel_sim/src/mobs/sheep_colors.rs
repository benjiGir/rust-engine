//! Couleurs de toison : palette commune aux modèles, aux butins et au générateur d'icônes.
use crate::mobs::model::MobPart;
use glam::Vec3;
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WoolColor {
    pub id: String,
    pub name: String,
    pub item: String,
    pub rgb: (u8, u8, u8),
    /// La chance d'un mouton de naître de cette couleur.
    pub weight: u32,
    /// La même chance pour un alpaga (0 : cette couleur n'apparaît pas chez lui).
    #[serde(default)]
    pub alpaca: u32,
}

/// Les couleurs naturelles d'une espèce à toison : chacune a son propre tirage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum CoatSet {
    #[default]
    Sheep,
    Alpaca,
}

impl CoatSet {
    fn weight(self, color: &WoolColor) -> u32 {
        match self {
            CoatSet::Sheep => color.weight,
            CoatSet::Alpaca => color.alpaca,
        }
    }
}

pub fn palette() -> &'static [WoolColor] {
    static COLORS: LazyLock<Vec<WoolColor>> = LazyLock::new(|| {
        let colors: Vec<WoolColor> =
            ron::from_str(include_str!("../../../../assets/core/wool_colors.ron"))
                .expect("palette de laine intégrée invalide");
        assert!(!colors.is_empty() && colors.len() <= 256 && colors[0].item == "core:wool");
        for set in [CoatSet::Sheep, CoatSet::Alpaca] {
            assert!(
                colors.iter().any(|c| set.weight(c) > 0),
                "{set:?} sans couleur"
            );
        }
        colors
    });
    &COLORS
}

/// La couleur de la toison appartient à l'animal (mouton ou alpaga), pas à son espèce ni à une
/// image temporaire : c'est un numéro dans la palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SheepCoat(pub u8);

impl SheepCoat {
    pub fn data(self) -> &'static WoolColor {
        palette().get(self.0 as usize).unwrap_or(&palette()[0])
    }
    /// La couleur d'un mouton qui naît (`roll` dans [0, 1[).
    pub fn choose(roll: f32) -> Self {
        Self::choose_in(CoatSet::Sheep, roll)
    }
    /// La couleur d'un animal de l'espèce `set` qui naît, selon le poids de chaque couleur.
    pub fn choose_in(set: CoatSet, roll: f32) -> Self {
        let total: u32 = palette().iter().map(|c| set.weight(c)).sum();
        let mut pick = (roll.clamp(0.0, 0.999_999) * total as f32) as u32;
        for (index, color) in palette().iter().enumerate() {
            let weight = set.weight(color);
            if pick < weight {
                return Self(index as u8);
            }
            pick -= weight;
        }
        Self::default()
    }
    pub fn inherit(a: Option<Self>, b: Option<Self>, roll: f32) -> Option<Self> {
        match (a, b) {
            (Some(a), Some(b)) => Some(if roll < 0.5 { a } else { b }),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }
    /// Teinte linéaire relative au blanc du motif, pour conserver ses fibres et ses ombres.
    pub fn tint(self) -> Vec3 {
        let linear = |(r, g, b): (u8, u8, u8)| {
            Vec3::from_array([r, g, b].map(|v| {
                let s = v as f32 / 255.0;
                if s <= 0.04045 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            }))
        };
        linear(self.data().rgb) / linear(palette()[0].rgb)
    }
    /// La toison et le pelage prennent la couleur : les matières dont le nom finit par `_wool` ou
    /// `_fur` (`mob_sheep_wool`, `mob_alpaca_fur`), dessinées en blanc pour être teintées.
    pub fn tint_part(self, part: &MobPart) -> Vec3 {
        if part.texture.as_deref().is_some_and(is_coat_texture) {
            self.tint()
        } else {
            Vec3::ONE
        }
    }
}

/// Cette matière suit la couleur de l'animal ?
pub fn is_coat_texture(name: &str) -> bool {
    name.ends_with("_wool") || name.ends_with("_fur")
}

/// La matière d'une toison, qu'on enlève à la tonte (`_wool` et pas `_fur` : le pelage reste).
pub fn is_fleece_texture(name: &str) -> bool {
    name.ends_with("_wool")
}

/// Le butin conserve la couleur même si le mouton a déjà quitté l'ECS.
pub fn drop_key(base: &str, coat: Option<SheepCoat>) -> &str {
    if base == "core:wool" {
        coat.map_or(base, |c| c.data().item.as_str())
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weighted_colours_are_reachable_and_genetics_never_introduces_an_unrelated_colour() {
        let colors = palette();
        assert_eq!(colors.len(), 18);
        let ids: std::collections::HashSet<_> = colors.iter().map(|c| &c.id).collect();
        let items: std::collections::HashSet<_> = colors.iter().map(|c| &c.item).collect();
        assert_eq!(ids.len(), colors.len());
        assert_eq!(items.len(), colors.len());
        let mut counts = vec![0; colors.len()];
        for i in 0..10000 {
            counts[SheepCoat::choose((i as f32 + 0.5) / 10000.0).0 as usize] += 1;
        }
        assert!(
            counts
                .iter()
                .zip(colors)
                .all(|(&n, c)| (n > 0) == (c.weight > 0))
        );
        assert!(
            counts[0] > 5000,
            "les moutons blancs restent les plus communs"
        );
        for a in 0..colors.len() {
            for b in 0..colors.len() {
                let (a, b) = (SheepCoat(a as u8), SheepCoat(b as u8));
                assert_eq!(SheepCoat::inherit(Some(a), Some(b), 0.0), Some(a));
                assert_eq!(SheepCoat::inherit(Some(a), Some(b), 0.99), Some(b));
                assert_eq!(SheepCoat::inherit(Some(a), Some(a), 0.8), Some(a));
            }
        }
        assert_eq!(SheepCoat::default().tint(), Vec3::ONE);
        assert_eq!(
            drop_key("core:raw_mutton", Some(SheepCoat(3))),
            "core:raw_mutton"
        );
    }

    #[test]
    fn alpacas_get_only_natural_colours_and_sheep_never_get_the_new_ones() {
        let (mut alpaca, mut sheep) = (
            std::collections::HashMap::new(),
            std::collections::HashSet::new(),
        );
        for i in 0..20000 {
            let roll = (i as f32 + 0.5) / 20000.0;
            *alpaca
                .entry(
                    SheepCoat::choose_in(CoatSet::Alpaca, roll)
                        .data()
                        .id
                        .clone(),
                )
                .or_insert(0) += 1;
            sheep.insert(SheepCoat::choose_in(CoatSet::Sheep, roll).data().id.clone());
        }
        let mut found: Vec<_> = alpaca.keys().cloned().collect();
        found.sort();
        assert_eq!(found, ["black", "brown", "cream", "fawn", "gray", "white"]);
        assert!(alpaca["white"] > alpaca["cream"] && alpaca["cream"] > alpaca["black"]);
        assert!(!sheep.contains("cream") && !sheep.contains("fawn"));
        assert_eq!(sheep.len(), 16);
        // La laine crème et fauve existe comme objet.
        assert_eq!(palette()[16].item, "core:wool_cream");
        assert_eq!(palette()[17].item, "core:wool_fawn");
    }
}
