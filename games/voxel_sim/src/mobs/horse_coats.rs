//! Les robes du cheval : une couleur de poil (et de crinière) et une marque blanche, tirées à la
//! naissance et gardées toute la vie. 7 couleurs × 5 marques = 35 robes, décrites dans
//! `assets/core/horse_coats.ron`.
//!
//! Il n'y a qu'un seul corps de cheval : le client le **teinte**. Les boîtes dont la matière finit
//! par `_coat` prennent la couleur du poil, `_mane` celle de la crinière ; les boîtes `_blaze`,
//! `_sock` et `_patch` sont blanches et n'existent que si la marque de l'animal les montre
//! (une robe « à balzanes » ne dessine que les `_sock`). Rien à dessiner de plus pour 35 robes.
//!
//! Le numéro de robe (`HorseCoat`) est ce qui voyage sur le réseau : un octet.

use crate::mobs::model::MobPart;
use crate::mobs::sheep_colors::srgb_to_linear;
use glam::Vec3;
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorseColor {
    pub id: String,
    pub name: String,
    pub body: (u8, u8, u8),
    pub mane: (u8, u8, u8),
    pub weight: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorseMark {
    pub id: String,
    pub name: String,
    /// Les matières blanches que cette marque montre (`blaze`, `sock`, `patch`).
    pub shows: Vec<String>,
    pub weight: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorseCoats {
    pub colors: Vec<HorseColor>,
    pub marks: Vec<HorseMark>,
}

/// Le gris (sRGB) dans lequel sont dessinées les matières `mob_horse_*` : la teinte d'une robe est
/// le rapport entre sa couleur et celui-ci, pour garder les poils et les ombres de la texture.
const TEXTURE_BASE: (u8, u8, u8) = (236, 236, 236);

pub fn coats() -> &'static HorseCoats {
    static COATS: LazyLock<HorseCoats> = LazyLock::new(|| {
        let coats: HorseCoats =
            ron::from_str(include_str!("../../../../assets/core/horse_coats.ron"))
                .expect("robes de cheval intégrées invalides");
        assert!(!coats.colors.is_empty() && !coats.marks.is_empty());
        assert!(coats.colors.len() * coats.marks.len() <= 256, "un octet");
        coats
    });
    &COATS
}

/// Le nombre de robes (35 pour le jeu de base).
pub fn count() -> usize {
    coats().colors.len() * coats().marks.len()
}

/// La robe d'un cheval : `couleur × marques + marque`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HorseCoat(pub u8);

/// Choisit un indice selon des poids (`roll` dans [0, 1[).
fn pick(weights: impl Iterator<Item = u32> + Clone, roll: f32) -> usize {
    let total: u32 = weights.clone().sum();
    let mut left = (roll.clamp(0.0, 0.999_999) * total as f32) as u32;
    for (index, weight) in weights.enumerate() {
        if left < weight {
            return index;
        }
        left -= weight;
    }
    0
}

impl HorseCoat {
    pub fn new(color: usize, mark: usize) -> Self {
        Self((color * coats().marks.len() + mark) as u8)
    }
    /// La couleur (indice dans `colors`).
    pub fn color_index(self) -> usize {
        (self.0 as usize / coats().marks.len()).min(coats().colors.len() - 1)
    }
    /// La marque (indice dans `marks`).
    pub fn mark_index(self) -> usize {
        self.0 as usize % coats().marks.len()
    }
    pub fn color(self) -> &'static HorseColor {
        &coats().colors[self.color_index()]
    }
    pub fn mark(self) -> &'static HorseMark {
        &coats().marks[self.mark_index()]
    }
    /// Son nom, par exemple « Bai à liste ».
    pub fn name(self) -> String {
        format!("{} {}", self.color().name, self.mark().name)
    }
    /// Une robe au hasard, selon le poids de chaque couleur et de chaque marque : deux tirages.
    pub fn choose(roll_color: f32, roll_mark: f32) -> Self {
        Self::new(
            pick(coats().colors.iter().map(|c| c.weight), roll_color),
            pick(coats().marks.iter().map(|m| m.weight), roll_mark),
        )
    }
    /// Un petit tient sa couleur d'un parent et sa marque de l'autre, au sort : les robes se
    /// mélangent (`roll` dans [0, 1[ ; les deux bits viennent de deux tirages).
    pub fn inherit(a: Self, b: Self, roll_color: f32, roll_mark: f32) -> Self {
        let color = if roll_color < 0.5 { a } else { b };
        let mark = if roll_mark < 0.5 { a } else { b };
        Self::new(color.color_index(), mark.mark_index())
    }
    /// La robe de `color[:mark]` (identifiants du fichier de données), pour `/summon`.
    pub fn parse(text: &str) -> Option<Self> {
        let (color, mark) = text.split_once(':').unwrap_or((text, "plain"));
        let color = coats().colors.iter().position(|c| c.id == color)?;
        let mark = coats().marks.iter().position(|m| m.id == mark)?;
        Some(Self::new(color, mark))
    }
    /// Comment cette robe teinte une boîte : `None` si la boîte ne doit pas être dessinée (une
    /// marque que la robe n'a pas), sinon la teinte à multiplier à sa couleur.
    pub fn tint_part(self, part: &MobPart) -> Option<Vec3> {
        let name = part.texture.as_deref().unwrap_or("");
        let base = srgb_to_linear(TEXTURE_BASE);
        if name.ends_with("_coat") {
            Some(srgb_to_linear(self.color().body) / base)
        } else if name.ends_with("_mane") {
            Some(srgb_to_linear(self.color().mane) / base)
        } else if let Some(kind) = ["blaze", "sock", "patch"]
            .into_iter()
            .find(|k| name.ends_with(&format!("_{k}")))
        {
            self.mark()
                .shows
                .iter()
                .any(|s| s == kind)
                .then_some(Vec3::ONE)
        } else {
            Some(Vec3::ONE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn there_are_35_distinct_coats() {
        assert_eq!(count(), 35);
        let names: HashSet<_> = (0..35).map(|n| HorseCoat(n).name()).collect();
        assert_eq!(names.len(), 35);
        assert_eq!(HorseCoat::new(3, 2), HorseCoat(17));
        assert_eq!(HorseCoat(17).color().id, "gray");
        assert_eq!(HorseCoat(17).mark().id, "socks");
    }

    #[test]
    fn every_coat_can_be_drawn_by_chance() {
        let mut seen = HashSet::new();
        // Un balayage fin des deux tirages atteint toutes les robes.
        for i in 0..200 {
            for j in 0..200 {
                seen.insert(HorseCoat::choose(i as f32 / 200.0, j as f32 / 200.0));
            }
        }
        assert_eq!(seen.len(), 35);
        assert_eq!(HorseCoat::choose(0.0, 0.0), HorseCoat::new(0, 0));
        assert_eq!(HorseCoat::choose(0.999, 0.999), HorseCoat::new(6, 4));
    }

    #[test]
    fn a_foal_takes_its_colour_from_one_parent_and_its_mark_from_the_other() {
        let (bay_plain, black_pied) = (HorseCoat::new(0, 0), HorseCoat::new(2, 4));
        let foal = HorseCoat::inherit(bay_plain, black_pied, 0.1, 0.9);
        assert_eq!(foal, HorseCoat::new(0, 4), "poil de a, marque de b");
        let foal = HorseCoat::inherit(bay_plain, black_pied, 0.9, 0.1);
        assert_eq!(foal, HorseCoat::new(2, 0));
    }

    #[test]
    fn marks_decide_which_white_boxes_are_drawn() {
        let part = |texture: &str| MobPart {
            offset: (0.0, 0.0, 0.0),
            size: (1.0, 1.0, 1.0),
            color: (1.0, 1.0, 1.0),
            bone: Default::default(),
            texture: Some(texture.to_owned()),
            texture_layer: 0,
        };
        let stockings = HorseCoat::new(0, 3);
        assert!(stockings.tint_part(&part("core:mob_horse_blaze")).is_some());
        assert!(stockings.tint_part(&part("core:mob_horse_sock")).is_some());
        assert!(stockings.tint_part(&part("core:mob_horse_patch")).is_none());
        assert!(
            HorseCoat::new(0, 0)
                .tint_part(&part("core:mob_horse_sock"))
                .is_none()
        );
        // Le poil est teinté, un sabot (autre matière) ne l'est pas.
        let bay = HorseCoat::new(0, 0);
        let tint = bay.tint_part(&part("core:mob_horse_coat")).unwrap();
        assert!(tint.x > tint.z, "un bai est brun-roux");
        assert_eq!(bay.tint_part(&part("core:mob_hoof")), Some(Vec3::ONE));
    }

    #[test]
    fn parse_reads_colour_and_mark_ids() {
        assert_eq!(HorseCoat::parse("gray:pied"), Some(HorseCoat::new(3, 4)));
        assert_eq!(HorseCoat::parse("bay"), Some(HorseCoat::new(0, 0)));
        assert_eq!(HorseCoat::parse("pink"), None);
    }

    #[test]
    fn horses_live_in_open_country_and_are_bred_with_oats_or_apples() {
        use crate::components::Baby;
        use crate::mobs::{self, MobRegistry, breeding, tests::sim};
        use std::sync::Arc;
        let mut world = sim();
        let registry = world.resource::<Arc<MobRegistry>>().clone();
        let kind = registry.id("core:horse").unwrap();
        let def = registry.get(kind).unwrap();
        for biome in ["core:plains", "core:flower_meadow", "core:savanna"] {
            assert!(def.spawn.biomes.iter().any(|b| b == biome), "{biome}");
        }
        assert_eq!(def.spawn.group, (2, 6), "en troupeau de 2 à 6");
        let a = mobs::summon(
            &mut world,
            "core:horse",
            Some("bay:blaze"),
            glam::Vec3::new(5.5, 80.0, 5.5),
        )
        .unwrap();
        let b = mobs::summon(
            &mut world,
            "core:horse",
            Some("gray:pied"),
            glam::Vec3::new(6.0, 80.0, 5.5),
        )
        .unwrap();
        assert_eq!(*world.get::<HorseCoat>(a).unwrap(), HorseCoat::new(0, 1));
        assert!(
            !breeding::feed(&mut world, a, "core:wheat"),
            "le cheval ne mange pas de blé"
        );
        assert!(breeding::feed(&mut world, a, "core:oats"));
        assert!(breeding::feed(&mut world, b, "core:apple"));
        breeding::update(&mut world);
        world.apply_commands();
        let mut foals = Vec::new();
        world
            .query::<(&Baby, &HorseCoat)>()
            .for_each(|_, (_, coat)| foals.push(*coat));
        assert_eq!(foals.len(), 1);
        let (colour, mark) = (foals[0].color().id.as_str(), foals[0].mark().id.as_str());
        assert!(["bay", "gray"].contains(&colour), "{colour}");
        assert!(["blaze", "pied"].contains(&mark), "{mark}");
        assert!(
            mobs::summon(
                &mut world,
                "core:horse",
                Some("mauve"),
                glam::Vec3::new(5.5, 80.0, 5.5)
            )
            .is_err()
        );
    }
}
