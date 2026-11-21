//! Les charmes de potion : un objet fait d'une potion, qui garde son effet actif tant qu'il est dans
//! l'inventaire et qu'on l'a activé d'un clic droit.
//!
//! # Comment c'est rangé
//!
//! Un charme est un objet ordinaire (un par potion, voir `charms.ron`), dont la **durabilité** est sa
//! charge en secondes : l'usure monte d'un point par seconde tant qu'il est actif, et une barre
//! s'affiche dans la case. L'activation tient dans un bit de `damage` ([`ACTIVE`], le bit 16, que
//! l'enchantement simple n'a de raison de lire que comme un niveau sans sorte : un charme ne reçoit
//! jamais d'enchantement). Aucune fiche, donc aucun format de sauvegarde ne change.
//!
//! # Les gestes
//!
//! - **clic droit** avec le charme en main : l'active ou le désactive (refusé s'il est vide) ;
//! - **recharge** : la même potion au curseur, un clic sur le charme (l'usure revient à zéro, la
//!   fiole vide est rendue) ;
//! - **chaque seconde**, `VoxelGame::update_charms` (voir `charm_play.rs`) relit les cases de l'inventaire : l'effet de la
//!   potion est renouvelé pour trois secondes (il s'éteint donc un instant après qu'on a posé le
//!   charme), et une potion de soin rend de la vie toutes les dix secondes.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::adventure_data::AdventureData;
use crate::enchanting;
use crate::tone::Tone;

/// Le bit de `damage` qui dit « actif ».
pub const ACTIVE: u32 = 1 << 16;

/// Un charme : l'objet, et la potion qui le fait et le recharge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharmDef {
    pub item: String,
    pub potion: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    item: String,
    potion: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    charms: Vec<Entry>,
}

/// Ajoute un fichier `charms.ron` aux données.
pub fn add_source(
    data: &mut AdventureData,
    namespace: &str,
    text: &str,
    origin: &str,
) -> Result<()> {
    let file: File =
        ron::from_str(text).with_context(|| format!("{origin} : fichier RON invalide"))?;
    for entry in file.charms {
        if !entry.item.starts_with(&format!("{namespace}:")) {
            bail!(
                "{origin} : le charme « {} » doit commencer par « {namespace}: »",
                entry.item
            );
        }
        if data.charms.iter().any(|c| c.item == entry.item) {
            bail!(
                "{origin} : le charme « {} » est défini deux fois",
                entry.item
            );
        }
        data.charms.push(CharmDef {
            item: entry.item,
            potion: entry.potion,
        });
    }
    Ok(())
}

/// Le charme est-il actif ?
pub fn is_active(damage: u32) -> bool {
    damage & ACTIVE != 0
}

/// Le même `damage`, activé ou désactivé.
pub fn with_active(damage: u32, active: bool) -> u32 {
    if active {
        damage | ACTIVE
    } else {
        damage & !ACTIVE
    }
}

/// La charge restante, de 0 à 1.
pub fn charge(damage: u32, durability: u32) -> f32 {
    let wear = damage & enchanting::WEAR_MASK;
    1.0 - (wear as f32 / durability.max(1) as f32).min(1.0)
}

/// Le charme est-il vide ?
pub fn is_empty(damage: u32, durability: u32) -> bool {
    damage & enchanting::WEAR_MASK >= durability
}

/// Un charme qui s'use d'une seconde : renvoie `true` s'il vient de se vider (il se désactive).
pub fn wear_one(damage: &mut u32, durability: u32) -> bool {
    let wear = (*damage & enchanting::WEAR_MASK) + 1;
    *damage = (*damage & !enchanting::WEAR_MASK) | wear.min(enchanting::WEAR_MASK);
    if wear >= durability {
        *damage = with_active(*damage, false);
        return true;
    }
    false
}

/// Recharge : l'usure revient à zéro, l'activation est gardée.
pub fn recharged(damage: u32) -> u32 {
    damage & !enchanting::WEAR_MASK
}

/// Les lignes de la bulle d'aide d'un charme.
pub fn card_lines(
    data: &AdventureData,
    key: &str,
    damage: u32,
    durability: Option<u32>,
) -> Vec<(String, Tone)> {
    if data.charm_of(key).is_none() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let durability = durability.unwrap_or(1);
    if is_empty(damage, durability) {
        lines.push(("Vide : à recharger".to_string(), Tone::Red));
    } else if is_active(damage) {
        lines.push((
            "Actif tant qu'il est dans l'inventaire".to_string(),
            Tone::Sage,
        ));
    } else {
        lines.push((
            "Inactif : clic droit pour l'activer".to_string(),
            Tone::Muted,
        ));
    }
    lines.push((
        format!("Charge : {:.0} %", charge(damage, durability) * 100.0),
        Tone::Text,
    ));
    lines.push((
        "Se recharge avec la potion dont il est fait : à la souris, un clic sur le charme"
            .to_string(),
        Tone::Muted,
    ));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn the_core_charms_name_real_items_and_potions() {
        let data = AdventureData::builtin();
        let items = items();
        assert_eq!(data.charms.len(), 5);
        for charm in &data.charms {
            let item = items.id(&charm.item).expect("objet du charme");
            assert!(items.get(item).durability.is_some(), "{}", charm.item);
            assert_eq!(items.get(item).max_stack, 1);
            assert!(items.id(&charm.potion).is_some(), "{}", charm.potion);
        }
    }

    #[test]
    fn activation_wear_and_recharge_keep_the_other_bits() {
        let mut damage = 0;
        assert!(!is_active(damage));
        damage = with_active(damage, true);
        assert!(is_active(damage));
        // Un point d'usure par seconde ; le bit d'activation reste.
        assert!(!wear_one(&mut damage, 3));
        assert!(!wear_one(&mut damage, 3));
        assert_eq!(enchanting::wear(damage), 2);
        assert!(is_active(damage));
        assert!((charge(damage, 3) - 1.0 / 3.0).abs() < 1e-6);
        // Le dernier point vide le charme, qui se désactive.
        assert!(wear_one(&mut damage, 3));
        assert!(is_empty(damage, 3));
        assert!(!is_active(damage));
        // Recharger rend la charge, activation gardée.
        let active = with_active(damage, true);
        let full = recharged(active);
        assert_eq!(enchanting::wear(full), 0);
        assert!(is_active(full));
        // Un charme n'est pas pris pour un objet enchanté.
        assert!(!enchanting::is_enchanted(active));
    }

    #[test]
    fn mods_cannot_define_a_charm_twice_or_outside_their_namespace() {
        let mut data = AdventureData::builtin();
        let text = "(charms: [(item: \"mod:charme\", potion: \"core:potion_speed\")])";
        add_source(&mut data, "mod", text, "mod").unwrap();
        assert!(add_source(&mut data, "mod", text, "mod").is_err());
        let other = "(charms: [(item: \"core:charme\", potion: \"core:potion_speed\")])";
        assert!(add_source(&mut data, "mod", other, "mod").is_err());
    }

    #[test]
    fn the_tooltip_tells_the_state_and_what_recharges_it() {
        let data = AdventureData::builtin();
        let key = "core:charm_speed";
        let lines = |damage| card_lines(&data, key, damage, Some(100));
        assert!(lines(0)[0].0.starts_with("Inactif"));
        assert!(lines(ACTIVE)[0].0.starts_with("Actif"));
        assert!(lines(100)[0].0.starts_with("Vide"));
        assert!(
            lines(0)
                .iter()
                .any(|(t, _)| t.contains("se recharge") || t.contains("recharge"))
        );
        assert!(card_lines(&data, "core:stone", 0, None).is_empty());
    }
}
