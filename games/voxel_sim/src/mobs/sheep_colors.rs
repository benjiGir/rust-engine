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
    pub weight: u32,
}

pub fn palette() -> &'static [WoolColor] {
    static COLORS: LazyLock<Vec<WoolColor>> = LazyLock::new(|| {
        let colors: Vec<WoolColor> =
            ron::from_str(include_str!("../../../../assets/core/wool_colors.ron"))
                .expect("palette de laine intégrée invalide");
        assert!(!colors.is_empty() && colors.len() <= 256 && colors[0].item == "core:wool");
        assert!(colors.iter().all(|c| c.weight > 0));
        colors
    });
    &COLORS
}

/// La couleur appartient au mouton, pas à son espèce ni à une image temporaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SheepCoat(pub u8);

impl SheepCoat {
    pub fn data(self) -> &'static WoolColor {
        palette().get(self.0 as usize).unwrap_or(&palette()[0])
    }
    pub fn choose(roll: f32) -> Self {
        let total: u32 = palette().iter().map(|c| c.weight).sum();
        let mut pick = (roll.clamp(0.0, 0.999_999) * total as f32) as u32;
        for (index, color) in palette().iter().enumerate() {
            if pick < color.weight {
                return Self(index as u8);
            }
            pick -= color.weight;
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
    pub fn tint_part(self, part: &MobPart) -> Vec3 {
        if part.texture.as_deref() == Some("core:mob_sheep_wool") {
            self.tint()
        } else {
            Vec3::ONE
        }
    }
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
        assert_eq!(colors.len(), 16);
        let ids: std::collections::HashSet<_> = colors.iter().map(|c| &c.id).collect();
        let items: std::collections::HashSet<_> = colors.iter().map(|c| &c.item).collect();
        assert_eq!(ids.len(), colors.len());
        assert_eq!(items.len(), colors.len());
        let mut counts = vec![0; colors.len()];
        for i in 0..10000 {
            counts[SheepCoat::choose((i as f32 + 0.5) / 10000.0).0 as usize] += 1;
        }
        assert!(counts.iter().all(|&n| n > 0));
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
}
