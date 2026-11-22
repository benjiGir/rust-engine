//! Les deux tables du butin : la **récupération** et la **réforge**. La logique est ici (pure et
//! testée) ; les écrans sont dans `forge_ui.rs`.
//!
//! Comme la table d'enchantement, elles travaillent sur l'objet que le joueur **tient à la
//! souris** dans leur écran, et prennent leurs matériaux dans son inventaire : aucun objet ne reste
//! posé dans le bloc, donc rien ne se perd si l'écran se ferme.
//!
//! # La table de récupération
//!
//! Elle détruit un objet à affixes et rend des **matériaux de rareté** : des fragments de la
//! rareté de l'objet (`shard` de `rarities.ron`, en nombre tiré dans `salvage`), parfois de la
//! poussière de gemme, et toutes les gemmes qu'il portait (elles ne sont jamais détruites). Un
//! objet sans affixe n'a pas de rareté : la table le refuse.
//!
//! # La table de réforge
//!
//! Un objet **de base** (sans affixe) devient un objet à affixes de la rareté choisie, contre des
//! fragments de cette rareté, de la poussière de gemme et des niveaux d'expérience (le coût est
//! dans `reforge` de `rarities.ron`). On voit **trois tirages** (des fiches différentes, de la même
//! rareté) et on en choisit un ; l'usure et les enchantements de l'objet sont gardés. Les tirages ne
//! changent pas tant qu'on ne choisit pas : la graine de la table avance après chaque réforge.

use engine_voxel::{ItemDef, ItemRegistry, Rng64};

use crate::adventure::affixes;
use crate::adventure::data::AdventureData;
use crate::player::item_data::{self, ItemData};

/// Ce que la récupération rend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Salvaged {
    /// Le matériau de rareté (clé d'objet) et sa quantité.
    pub shard: String,
    pub shards: u32,
    pub dust: u32,
    /// Les gemmes qui étaient serties dans l'objet.
    pub gems: Vec<String>,
}

/// Un intervalle de quantités (min, max).
pub type Range = (u32, u32);

/// Ce que rendra la récupération, en intervalles : (matériau, fragments min–max, poussière
/// min–max). `None` si l'objet n'a pas de rareté.
pub fn salvage_preview(data: &AdventureData, damage: u32) -> Option<(String, Range, Range)> {
    let rarity = data.rarity(affixes::rarity_of(damage)?)?;
    Some((
        rarity.shard.clone(),
        rarity.salvage.shards,
        rarity.salvage.dust,
    ))
}

/// Détruit un objet à affixes (la fiche est libérée) et dit ce qu'il rend. `None` (rien n'est
/// détruit) si l'objet n'a pas de rareté.
pub fn salvage(data: &AdventureData, damage: u32, rng: &mut Rng64) -> Option<Salvaged> {
    let rarity = data.rarity(affixes::rarity_of(damage)?)?;
    let gems = crate::adventure::gems::gems_of(damage);
    let between = |rng: &mut Rng64, (min, max): (u32, u32)| {
        if max <= min {
            min
        } else {
            min + rng.below(max - min + 1)
        }
    };
    let out = Salvaged {
        shard: rarity.shard.clone(),
        shards: between(rng, rarity.salvage.shards),
        dust: between(rng, rarity.salvage.dust),
        gems,
    };
    crate::enchanting::release(damage);
    Some(out)
}

/// Le coût d'un tirage de réforge pour cette rareté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReforgeCost {
    pub shard: String,
    pub shards: u32,
    pub dust: u32,
    pub levels: u32,
}

pub fn reforge_cost(data: &AdventureData, rarity: u8) -> Option<ReforgeCost> {
    let r = data.rarity(rarity)?;
    Some(ReforgeCost {
        shard: r.shard.clone(),
        shards: r.reforge.shards,
        dust: r.reforge.dust,
        levels: r.reforge.levels,
    })
}

/// Cet objet peut-il être réforgé ? Un objet de base seulement : qui porte des affixes, sans en
/// avoir déjà.
pub fn can_reforge(def: &ItemDef, damage: u32) -> bool {
    affixes::can_carry(def) && affixes::rarity_of(damage).is_none()
}

/// Les trois tirages de la table pour cet objet, cette rareté et cette graine : les mêmes à chaque
/// appel.
pub fn draws(data: &AdventureData, def: &ItemDef, rarity: u8, seed: u64) -> Vec<ItemData> {
    (0..3u64)
        .map(|i| {
            let mut rng =
                Rng64::new(seed.wrapping_add(i.wrapping_mul(0x9E37_79B9_7F4A_7C15)) ^ 0xF0_46E);
            affixes::roll_sheet(data, def, rarity, &mut rng)
        })
        .collect()
}

/// Ce que le joueur a en poche pour payer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purse {
    pub shards: u32,
    pub dust: u32,
    pub levels: u32,
}

impl ReforgeCost {
    pub fn affordable(&self, purse: Purse) -> bool {
        purse.shards >= self.shards && purse.dust >= self.dust && purse.levels >= self.levels
    }
}

/// Le matériau et la poussière sont-ils des objets du jeu ? (vérifié au chargement par
/// `AdventureData::validate_items`, ici pour l'écran.)
pub fn dust_item(items: &ItemRegistry) -> Option<engine_voxel::ItemId> {
    items.id("core:gem_dust")
}

/// Applique un tirage à une pile (voir `affixes::apply`). Les emplacements que l'objet avait déjà
/// (un sceau d'emplacement, des gemmes serties) sont gardés ; le tirage ne fait que les compléter.
pub fn apply_draw(damage: &mut u32, mut draw: ItemData) -> bool {
    let mut sockets = item_data::sheet(*damage)
        .map(|s| s.sockets)
        .unwrap_or_default();
    while sockets.len() < draw.sockets.len() {
        sockets.push(None);
    }
    draw.sockets = sockets;
    affixes::apply(damage, draw)
}

/// Vrai si cette pile est un objet à affixes que la récupération peut détruire.
pub fn can_salvage(damage: u32) -> bool {
    !item_data::is_pending(damage) && affixes::rarity_of(damage).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enchanting;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn affixed(
        data: &AdventureData,
        items: &ItemRegistry,
        key: &str,
        rarity: u8,
        seed: u64,
    ) -> u32 {
        let def = items.get(items.id(key).unwrap());
        let mut damage = 0;
        let mut rng = Rng64::new(seed);
        let mut sheet = affixes::roll_sheet(data, def, rarity, &mut rng);
        sheet.sockets = vec![None, None];
        assert!(affixes::apply(&mut damage, sheet));
        damage
    }

    #[test]
    fn salvage_destroys_an_affixed_item_and_returns_rarity_materials() {
        let data = AdventureData::builtin();
        let items = items();
        let damage = affixed(&data, &items, "core:iron_sword", 3, 11);
        assert!(can_salvage(damage));
        let number = item_data::number(damage);
        let (shard, shards, dust) = salvage_preview(&data, damage).unwrap();
        assert_eq!(shard, "core:shard_epic");
        let mut rng = Rng64::new(1);
        let got = salvage(&data, damage, &mut rng).unwrap();
        assert_eq!(got.shard, "core:shard_epic", "un matériau par rareté");
        assert!((shards.0..=shards.1).contains(&got.shards));
        assert!((dust.0..=dust.1).contains(&got.dust));
        assert!(got.gems.is_empty());
        assert!(
            item_data::read(|s| s.get(number).is_none()),
            "la fiche est libérée"
        );
    }

    #[test]
    fn every_rarity_has_its_own_material_and_higher_ones_give_more() {
        let data = AdventureData::builtin();
        let items = items();
        let mut seen = std::collections::HashSet::new();
        let mut best_before = 0;
        for rarity in 0..5u8 {
            let damage = affixed(&data, &items, "core:iron_pickaxe", rarity, 3);
            let (shard, shards, _) = salvage_preview(&data, damage).unwrap();
            assert!(seen.insert(shard.clone()), "{shard} est déjà pris");
            assert!(items.id(&shard).is_some());
            assert!(
                shards.1 >= best_before,
                "la rareté {rarity} rend au moins autant"
            );
            best_before = shards.1;
        }
    }

    #[test]
    fn salvage_gives_the_gems_back_and_refuses_plain_items() {
        let data = AdventureData::builtin();
        let items = items();
        let mut damage = affixed(&data, &items, "core:iron_sword", 2, 5);
        enchanting::edit_sheet(&mut damage, |s| {
            s.sockets[0] = Some("core:gem_ruby_4".into())
        });
        let got = salvage(&data, damage, &mut Rng64::new(2)).unwrap();
        assert_eq!(got.gems, ["core:gem_ruby_4"]);
        // Un objet de base, ou seulement enchanté : refusé, rien n'est détruit.
        assert!(salvage(&data, 0, &mut Rng64::new(2)).is_none());
        let enchanted = enchanting::with_enchantment(0, enchanting::Enchantment::SHARPNESS, 1);
        assert!(!can_salvage(enchanted));
        assert!(salvage_preview(&data, enchanted).is_none());
    }

    #[test]
    fn reforge_offers_three_stable_draws_of_the_chosen_rarity() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let a = draws(&data, sword, 2, 77);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|d| d.rarity == 2 && !d.affixes.is_empty()));
        assert_eq!(
            a,
            draws(&data, sword, 2, 77),
            "les mêmes tant que la graine ne bouge pas"
        );
        assert_ne!(
            a,
            draws(&data, sword, 2, 78),
            "une autre graine, d'autres tirages"
        );
        assert!(a[0] != a[1] || a[1] != a[2], "trois tirages différents");
    }

    #[test]
    fn reforge_costs_follow_the_rarity_and_need_the_whole_purse() {
        let data = AdventureData::builtin();
        let common = reforge_cost(&data, 0).unwrap();
        let mythic = reforge_cost(&data, 4).unwrap();
        assert_eq!(mythic.shard, "core:shard_mythic");
        assert!(mythic.shards > common.shards && mythic.levels > common.levels);
        assert!(reforge_cost(&data, 9).is_none());
        let rich = Purse {
            shards: 99,
            dust: 99,
            levels: 99,
        };
        assert!(mythic.affordable(rich));
        for poor in [
            Purse {
                shards: mythic.shards - 1,
                ..rich
            },
            Purse {
                dust: mythic.dust - 1,
                ..rich
            },
            Purse {
                levels: mythic.levels - 1,
                ..rich
            },
        ] {
            assert!(!mythic.affordable(poor), "{poor:?}");
        }
    }

    #[test]
    fn only_base_items_can_be_reforged_and_wear_and_enchantments_stay() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        assert!(can_reforge(sword, 0));
        assert!(!can_reforge(items.get(items.id("core:coal").unwrap()), 0));
        let done = affixed(&data, &items, "core:iron_sword", 1, 4);
        assert!(!can_reforge(sword, done), "déjà à affixes");
        // Un objet usé et enchanté (un seul enchantement : pile simple).
        let mut damage = enchanting::with_enchantment(9, enchanting::Enchantment::SHARPNESS, 2);
        assert!(can_reforge(sword, damage));
        let draw = draws(&data, sword, 3, 1).remove(1);
        assert!(apply_draw(&mut damage, draw.clone()));
        let sheet = item_data::sheet(damage).unwrap();
        assert_eq!(sheet.wear, 9);
        assert_eq!(
            enchanting::level_of(damage, enchanting::Enchantment::SHARPNESS),
            2
        );
        assert_eq!(sheet.affixes, draw.affixes);
        assert!(!can_reforge(sword, damage), "il est maintenant à affixes");
        // Des gemmes déjà serties (sceau d'emplacement) ne sont pas perdues.
        let mut socketed = 0;
        enchanting::edit_sheet(&mut socketed, |s| {
            s.sockets = vec![Some("core:gem_opal_2".into())]
        });
        let draw = draws(&data, sword, 0, 9).remove(0);
        assert!(can_reforge(sword, socketed));
        assert!(apply_draw(&mut socketed, draw));
        assert_eq!(
            crate::adventure::gems::gems_of(socketed),
            ["core:gem_opal_2"]
        );
    }
}
