//! Les enchantements en données : `assets/core/enchantments.ron`, puis ceux des mods (qui
//! **ajoutent**). Chaque enchantement a un numéro stable (celui qu'on sauvegarde dans les objets),
//! des niveaux plafonnés selon l'Eterna de la table, ce qui peut le porter, et ses effets, exprimés
//! en bonus d'attributs par niveau (voir `attributes.rs`). `enchanting.rs` s'en sert pour les
//! libellés et les offres, `gear.rs` pour calculer les attributs.
//!
//! Les numéros de 1 à 63 sont ceux du jeu de base, de 64 à 127 ceux des mods : le plus grand reste
//! sous 128, parce qu'une pile simple le range dans les bits 24 à 30 de `damage` (voir
//! `enchanting.rs`).

use anyhow::{Context as _, Result, bail};
use engine_voxel::{ArmorSlot, ItemDef, ToolKind};
use serde::Deserialize;

use crate::adventure::data::AdventureData;
use crate::player::attributes::Attribute;

/// Ce qui peut porter un enchantement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Target {
    /// Une épée ou une hache.
    Weapon,
    /// Une pioche, une hache, une pelle, une houe ou des cisailles.
    Digger,
    /// Toute pièce d'armure.
    Armor,
    /// Tout outil ou toute armure.
    Durable,
}

impl Target {
    pub fn fits(self, item: &ItemDef) -> bool {
        match self {
            Self::Weapon => item
                .tool
                .is_some_and(|t| matches!(t.kind, ToolKind::Sword | ToolKind::Axe)),
            Self::Digger => item.tool.is_some_and(|t| {
                matches!(
                    t.kind,
                    ToolKind::Pickaxe
                        | ToolKind::Axe
                        | ToolKind::Shovel
                        | ToolKind::Hoe
                        | ToolKind::Shears
                )
            }),
            Self::Armor => item.armor.is_some(),
            Self::Durable => {
                item.durability.is_some() && (item.tool.is_some() || item.armor.is_some())
            }
        }
    }
}

/// Quand l'effet d'un enchantement compte : objet en main ou pièce portée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Use {
    Held,
    Worn,
}

/// Ce que chaque niveau d'un enchantement ajoute à un attribut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnchantEffect {
    pub attribute: Attribute,
    pub flat: f32,
    pub percent: f32,
    pub on: Use,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnchantDef {
    pub id: String,
    pub number: u8,
    pub name: String,
    pub max_level: u8,
    pub hard_max: u8,
    pub weight: u32,
    pub treasure: bool,
    pub targets: Vec<Target>,
    pub armor_slots: Vec<ArmorSlot>,
    pub effects: Vec<EnchantEffect>,
}

impl EnchantDef {
    /// Cet enchantement convient-il à cet objet ?
    pub fn applies_to(&self, item: &ItemDef) -> bool {
        if !self.targets.iter().any(|t| t.fits(item)) {
            return false;
        }
        self.armor_slots.is_empty()
            || item
                .armor
                .is_some_and(|a| self.armor_slots.contains(&a.slot))
    }

    /// Le niveau le plus haut qu'une table d'Eterna `eterna` (0 à 100) propose : de `max_level` à
    /// `hard_max`, en proportion de l'Eterna.
    pub fn cap_for(&self, eterna: f32) -> u8 {
        let extra = f32::from(self.hard_max.saturating_sub(self.max_level));
        self.max_level + (extra * (eterna / 100.0).clamp(0.0, 1.0)).floor() as u8
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectEntry {
    attribute: String,
    #[serde(default)]
    flat: f32,
    #[serde(default)]
    percent: f32,
    on: Use,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    number: u8,
    name: String,
    max_level: u8,
    hard_max: u8,
    weight: u32,
    #[serde(default)]
    treasure: bool,
    targets: Vec<Target>,
    #[serde(default)]
    armor_slots: Vec<ArmorSlot>,
    #[serde(default)]
    effects: Vec<EffectEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    max_per_item: Option<usize>,
    enchantments: Vec<Entry>,
}

/// Ajoute un fichier `enchantments.ron` aux données.
pub fn add_source(
    data: &mut AdventureData,
    namespace: &str,
    text: &str,
    origin: &str,
) -> Result<()> {
    let file: File =
        ron::from_str(text).with_context(|| format!("{origin} : fichier RON invalide"))?;
    if let Some(max) = file.max_per_item {
        data.max_enchantments = max.max(1);
    }
    for entry in file.enchantments {
        if !entry.id.starts_with(&format!("{namespace}:")) {
            bail!(
                "{origin} : l'enchantement « {} » doit commencer par « {namespace}: »",
                entry.id
            );
        }
        let (low, high) = if namespace == "core" {
            (1, 63)
        } else {
            (64, 127)
        };
        if !(low..=high).contains(&entry.number) {
            bail!(
                "{origin} : le numéro de « {} » doit aller de {low} à {high}",
                entry.id
            );
        }
        if data
            .enchantments
            .iter()
            .any(|e| e.id == entry.id || e.number == entry.number)
        {
            bail!(
                "{origin} : « {} » (numéro {}) est défini deux fois",
                entry.id,
                entry.number
            );
        }
        if entry.max_level == 0 || entry.hard_max < entry.max_level || entry.weight == 0 {
            bail!("{origin} : « {} » a des niveaux ou un poids nuls", entry.id);
        }
        if entry.targets.is_empty() {
            bail!("{origin} : « {} » ne convient à rien", entry.id);
        }
        let mut effects = Vec::new();
        for effect in &entry.effects {
            let attribute = Attribute::ALL
                .into_iter()
                .find(|a| a.key() == effect.attribute)
                .with_context(|| {
                    format!(
                        "{origin} : « {} » : attribut inconnu « {} »",
                        entry.id, effect.attribute
                    )
                })?;
            effects.push(EnchantEffect {
                attribute,
                flat: effect.flat,
                percent: effect.percent,
                on: effect.on,
            });
        }
        data.enchantments.push(EnchantDef {
            id: entry.id,
            number: entry.number,
            name: entry.name,
            max_level: entry.max_level,
            hard_max: entry.hard_max,
            weight: entry.weight,
            treasure: entry.treasure,
            targets: entry.targets,
            armor_slots: entry.armor_slots,
            effects,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_core_file_is_valid_and_numbers_stay_under_128() {
        let data = AdventureData::builtin();
        assert_eq!(data.enchantments.len(), 12);
        assert!(
            data.enchantments
                .iter()
                .all(|e| (1..=63).contains(&e.number))
        );
        // Les quatre d'avant la phase 3 gardent leurs numéros (ils sont sauvegardés dans les objets).
        for (key, number) in [
            ("core:efficiency", 1),
            ("core:sharpness", 2),
            ("core:unbreaking", 3),
            ("core:protection", 4),
        ] {
            assert_eq!(data.enchant_by_id(key).unwrap().number, number);
        }
    }

    #[test]
    fn caps_climb_with_the_eterna_in_steps() {
        let data = AdventureData::builtin();
        let sharp = data.enchant_by_id("core:sharpness").unwrap();
        assert_eq!(sharp.cap_for(0.0), 3);
        assert_eq!(sharp.cap_for(15.0), 3);
        assert_eq!(sharp.cap_for(40.0), 5);
        assert_eq!(sharp.cap_for(100.0), 8);
        assert_eq!(sharp.cap_for(500.0), 8, "jamais au-delà de hard_max");
        // Une sorte à un seul niveau reste à un.
        let silk = data.enchant_by_id("core:silk_touch").unwrap();
        assert_eq!(silk.cap_for(100.0), 1);
    }

    #[test]
    fn a_mod_can_add_enchantments_but_not_steal_numbers() {
        let mut data = AdventureData::builtin();
        let text = "(enchantments: [(id: \"mod:vertu\", number: 64, name: \"Vertu\", max_level: 2, \
            hard_max: 4, weight: 3, targets: [Weapon], effects: [(attribute: \"damage\", flat: 1.0, on: Held)])])";
        add_source(&mut data, "mod", text, "mod").unwrap();
        assert_eq!(data.enchant_by_number(64).unwrap().name, "Vertu");
        // Un numéro du jeu de base, ou déjà pris : refusé.
        let steal = text.replace("64", "2");
        assert!(add_source(&mut data, "mod", &steal, "mod").is_err());
        assert!(add_source(&mut data, "mod", text, "mod").is_err());
        // Un autre espace de noms que le sien : refusé.
        let other = text.replace("mod:vertu", "autre:vertu").replace("64", "65");
        assert!(add_source(&mut data, "mod", &other, "mod").is_err());
    }
}
