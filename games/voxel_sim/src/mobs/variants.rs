//! Les variantes d'une créature : des races (les vaches tempérée, des Highlands, à bosse) qui
//! partagent son comportement mais ont leur propre corps.
//!
//! Dans `entities.ron`, une créature peut lister des `variants` : chacune a un identifiant court,
//! un nom, les biomes où elle apparaît, et ses parties (le corps entier, qui remplace celui de la
//! créature). Une créature apparue dans un de ces biomes est de cette variante ; ailleurs, elle
//! garde le corps de base. Les biomes des variantes s'ajoutent à ceux où la créature apparaît.
//! Un petit prend la variante d'un de ses parents, tirée au sort : les races se croisent.
use serde::Deserialize;

use crate::mobs::{MobDef, MobPart};

/// Une variante telle qu'écrite dans `entities.ron`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VariantEntry {
    pub id: String,
    pub name: String,
    pub biomes: Vec<String>,
    pub parts: Vec<MobPart>,
}

/// Une variante chargée.
#[derive(Debug, Clone)]
pub struct MobVariant {
    pub id: String,
    pub name: String,
    pub biomes: Vec<String>,
    pub parts: Vec<MobPart>,
}

impl From<VariantEntry> for MobVariant {
    fn from(entry: VariantEntry) -> Self {
        MobVariant {
            id: entry.id,
            name: entry.name,
            biomes: entry.biomes,
            parts: entry.parts,
        }
    }
}

/// La variante d'une créature : 0 pour le corps de base, `n` pour `variants[n - 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Variant(pub u8);

impl MobDef {
    /// Le corps d'une variante (celui de base si elle est inconnue).
    pub fn parts_for(&self, variant: u8) -> &[MobPart] {
        variant
            .checked_sub(1)
            .and_then(|i| self.variants.get(i as usize))
            .map_or(&self.parts, |v| &v.parts)
    }

    /// Le numéro de la variante d'identifiant `id` (`None` si elle n'existe pas).
    pub fn variant_id(&self, id: &str) -> Option<u8> {
        self.variants
            .iter()
            .position(|v| v.id == id)
            .map(|i| i as u8 + 1)
    }

    /// La variante d'une créature qui apparaît dans `biome` ; `r` dans [0, 1[ départage les
    /// variantes d'un même biome.
    pub fn variant_for_biome(&self, biome: &str, r: f32) -> u8 {
        let matching: Vec<u8> = self
            .variants
            .iter()
            .enumerate()
            .filter(|(_, v)| v.biomes.iter().any(|b| b == biome))
            .map(|(i, _)| i as u8 + 1)
            .collect();
        if matching.is_empty() {
            0
        } else {
            matching[((r * matching.len() as f32) as usize).min(matching.len() - 1)]
        }
    }
}

/// La variante d'un petit : celle d'un des parents, au sort.
pub fn inherit(a: Variant, b: Variant, r: f32) -> Variant {
    if r < 0.5 { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::MobRegistry;

    fn cow() -> MobDef {
        let tests = crate::mobs::tests::repo();
        let registry =
            MobRegistry::load(&tests.join("assets"), &[], &crate::mobs::tests::blocks()).unwrap();
        registry.by_key("core:cow").unwrap().clone()
    }

    #[test]
    fn a_cow_takes_the_breed_of_its_biome() {
        let cow = cow();
        let highland = cow.variant_id("highland").expect("vache des Highlands");
        let zebu = cow.variant_id("zebu").expect("vache à bosse");
        assert_eq!(cow.variant_for_biome("core:plains", 0.3), 0);
        assert_eq!(cow.variant_for_biome("core:taiga", 0.3), highland);
        assert_eq!(cow.variant_for_biome("core:savanna", 0.9), zebu);
        // Elle apparaît là où vivent ses races.
        for biome in ["core:plains", "core:taiga", "core:savanna"] {
            assert!(cow.spawn.biomes.iter().any(|b| b == biome), "{biome}");
        }
        // Chaque race a son propre corps.
        assert_ne!(cow.parts_for(highland).len(), 0);
        assert_ne!(cow.parts_for(zebu).len(), cow.parts.len());
        assert_eq!(cow.parts_for(99).len(), cow.parts.len());
    }

    #[test]
    fn a_calf_takes_the_breed_of_one_parent() {
        let (a, b) = (Variant(1), Variant(2));
        assert_eq!(inherit(a, b, 0.2), a);
        assert_eq!(inherit(a, b, 0.7), b);
    }
}
