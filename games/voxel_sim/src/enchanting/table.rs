//! Les stats d'une table d'enchantement et ce qu'elle propose.
//!
//! # Les stats
//!
//! La table lit **32 positions** autour d'elle : deux anneaux (à sa hauteur et un bloc au-dessus) de
//! 16 cases chacun, à deux blocs de distance (le carré de 5 × 5 moins l'intérieur de 3 × 3). Une
//! étagère ne compte que s'il y a de **l'air** entre la table et elle, à la même hauteur : la case
//! du milieu doit être libre. Chaque bloc lu apporte ce que dit son champ `enchanting` de `blocks.ron`
//! (voir `engine_voxel::EnchantingStats`), et la table elle-même donne sa base :
//!
//! - **Eterna** (0 à 100) : la somme des contributions, plafonnée par le plus haut `max_eterna` des
//!   blocs présents : un mur d'étagères de chêne plafonne à 15, il faut des matériaux plus rares
//!   pour monter plus haut ;
//! - **Quanta** et **Arcana** (0 à 100 %) : la somme des contributions ;
//! - **Indices** : le nombre d'enchantements montrés à l'avance (la partie entière du total) ;
//! - **Stabilité** (0 à 100 %) : le hasard de Quanta ne fait plus que du bien ;
//! - **Trésor** : un seul bloc suffit, la table propose alors aussi les enchantements rares.
//!
//! # Les offres
//!
//! Trois lots, du moins cher au plus cher ([`offers`]). Le **niveau de table** d'un lot va de 1 à 100 :
//! un tiers, deux tiers, la totalité de l'Eterna (au moins 1, 2 et 3), puis Quanta le fait varier
//! (de ± la moitié de Quanta, vers le haut seulement si la stabilité est pleine). Il donne le niveau
//! de chaque enchantement, entre 1 et son plafond pour cette Eterna (voir `EnchantDef::cap_for`) ;
//! il coûte `4 × niveau` **points d'expérience** (pas des niveaux) et un à trois saphirs. Arcana
//! ajoute des enchantements au lot (jusqu'à deux) et favorise les rares. Les indices disent combien
//! d'enchantements du lot sont montrés avant l'achat ; les autres s'affichent « ? ».
//!
//! Un tirage ne dépend que de la graine de la table, de l'objet et des stats : le même lot revient
//! tant qu'on n'a pas enchanté (la graine change alors).

use engine_voxel::{EnchantingStats, ItemDef, Rng64};
use glam::IVec3;

use crate::adventure::data::AdventureData;
use crate::enchanting::books::BookKind;
use crate::enchanting::{self, Enchantment};

/// Ce que lit la table, en positions relatives : (la case de l'étagère, la case qui doit être libre).
pub fn shelf_positions() -> Vec<(IVec3, IVec3)> {
    let mut out = Vec::new();
    for dy in 0..2_i32 {
        for dx in -2..=2_i32 {
            for dz in -2..=2_i32 {
                if dx.abs().max(dz.abs()) == 2 {
                    out.push((IVec3::new(dx, dy, dz), IVec3::new(dx / 2, dy, dz / 2)));
                }
            }
        }
    }
    out
}

/// Les stats lues autour d'une table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableStats {
    /// L'Eterna retenue (0 à 100).
    pub eterna: f32,
    /// La somme des contributions, avant le plafond.
    pub raw_eterna: f32,
    /// Le plafond : le plus haut `max_eterna` des blocs présents.
    pub max_eterna: f32,
    pub quanta: f32,
    pub arcana: f32,
    pub clues: u8,
    pub stability: f32,
    pub treasure: bool,
    /// Combien d'étagères comptent.
    pub shelves: u32,
}

/// Lit les stats d'une table. `table` : sa position ; `is_clear(p)` : le bloc en `p` laisse-t-il
/// passer (de l'air) ? ; `stats_at(p)` : ce que le bloc en `p` apporte, s'il apporte quelque chose.
pub fn read_stats(
    table: IVec3,
    is_clear: impl Fn(IVec3) -> bool,
    stats_at: impl Fn(IVec3) -> Option<EnchantingStats>,
) -> TableStats {
    let mut blocks: Vec<EnchantingStats> = Vec::new();
    let mut shelves = 0;
    // La table elle-même donne sa base.
    blocks.extend(stats_at(table));
    for (offset, middle) in shelf_positions() {
        if !is_clear(table + middle) {
            continue;
        }
        if let Some(stats) = stats_at(table + offset) {
            shelves += 1;
            blocks.push(stats);
        }
    }
    let sum = |f: fn(&EnchantingStats) -> f32| blocks.iter().map(f).sum::<f32>();
    let raw_eterna = sum(|b| b.eterna);
    let max_eterna = blocks.iter().map(|b| b.max_eterna).fold(0.0, f32::max);
    TableStats {
        eterna: raw_eterna.min(max_eterna).clamp(0.0, 100.0),
        raw_eterna,
        max_eterna,
        quanta: sum(|b| b.quanta).clamp(0.0, 100.0),
        arcana: sum(|b| b.arcana).clamp(0.0, 100.0),
        clues: sum(|b| b.clues).floor().clamp(0.0, 5.0) as u8,
        stability: sum(|b| b.stability).clamp(0.0, 100.0),
        treasure: blocks.iter().any(|b| b.treasure),
        shelves,
    }
}

/// Ce que reçoit un lot : un objet à enchanter, ou un livre (ou un tome) à remplir.
#[derive(Debug, Clone, Copy)]
pub enum Subject<'a> {
    Item(&'a ItemDef),
    Book(BookKind),
}

/// Un lot de la table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub enchants: Vec<(Enchantment, u8)>,
    /// Le niveau de table du lot (1 à 100).
    pub level: u32,
    /// Les points d'expérience dépensés.
    pub points: u32,
    /// Les saphirs dépensés.
    pub gems: u32,
    /// Combien d'enchantements sont montrés avant l'achat.
    pub clues: usize,
}

impl Offer {
    /// « Efficacité II + Solidité II ».
    pub fn label(&self) -> String {
        self.enchants
            .iter()
            .map(|&(kind, level)| enchanting::label(kind, level))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Ce qu'on voit avant d'acheter : les indices, puis des « ? » pour le reste.
    pub fn preview(&self) -> String {
        self.enchants
            .iter()
            .enumerate()
            .map(|(i, &(kind, level))| {
                if i < self.clues {
                    enchanting::label(kind, level)
                } else {
                    "?".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Le `damage` de la pile une fois ce lot appliqué.
    pub fn apply(&self, damage: u32) -> u32 {
        self.enchants.iter().fold(damage, |d, &(kind, level)| {
            enchanting::with_enchantment(d, kind, level)
        })
    }
}

/// Les points d'expérience d'un lot de ce niveau de table.
pub fn points_for(level: u32) -> u32 {
    4 * level.max(1)
}

/// Un nombre de départ stable pour un nom (la graine d'un lot dépend de l'objet).
fn salt(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// Les enchantements que ce sujet peut recevoir (ceux qu'il porte déjà et, sans étagère « trésor »,
/// les rares, en sont exclus).
fn eligible(
    data: &AdventureData,
    subject: Subject,
    owned: &[(Enchantment, u8)],
    stats: &TableStats,
) -> Vec<Enchantment> {
    data.enchantments
        .iter()
        .filter(|e| match subject {
            Subject::Item(item) => e.applies_to(item),
            Subject::Book(kind) => kind.accepts(e),
        })
        .filter(|e| stats.treasure || !e.treasure)
        .map(|e| Enchantment(e.number))
        .filter(|e| !owned.iter().any(|(k, _)| k == e))
        .collect()
}

/// Les trois lots. `key` : la clé de l'objet (elle entre dans la graine), `damage` : ce qu'il porte
/// déjà, `seed` : la graine de la table. `None` si rien ne peut plus lui être proposé.
pub fn offers(
    data: &AdventureData,
    subject: Subject,
    key: &str,
    damage: u32,
    seed: u32,
    stats: &TableStats,
) -> Option<[Offer; 3]> {
    let owned = enchanting::enchantments_of(damage);
    let room = data.max_enchantments.saturating_sub(owned.len());
    let fitting = eligible(data, subject, &owned, stats);
    if fitting.is_empty() || room == 0 {
        return None;
    }
    let mut rng = Rng64::new(u64::from(seed) ^ salt(key));
    let weight = |kind: Enchantment| {
        let base = data
            .enchant_by_number(kind.id())
            .map_or(1.0, |e| e.weight as f32);
        // Arcana favorise les rares.
        if base <= 4.0 {
            base * (1.0 + stats.arcana / 25.0)
        } else {
            base
        }
    };
    let mut lot = |i: usize| {
        // Le niveau de table : une part de l'Eterna, au moins 1, 2 ou 3, puis le hasard de Quanta.
        let fraction = [0.34, 0.67, 1.0][i];
        let base = (stats.eterna * fraction).round().max(i as f32 + 1.0);
        let mut spread = rng.next_f32() * 2.0 - 1.0;
        if spread < 0.0 {
            spread *= 1.0 - stats.stability / 100.0;
        }
        let level = (base * (1.0 + spread * stats.quanta / 200.0))
            .round()
            .clamp(1.0, 100.0);
        // Le nombre d'enchantements : un, un, deux, plus ceux qu'Arcana ajoute.
        let mut count = [1, 1, 2][i];
        for _ in 0..2 {
            if rng.next_f32() < stats.arcana / 125.0 {
                count += 1;
            }
        }
        let mut pool = fitting.clone();
        let count = count.min(pool.len()).min(room);
        let reference = stats.eterna.max(3.0);
        let enchants = (0..count)
            .map(|_| {
                let total: f32 = pool.iter().map(|k| weight(*k)).sum();
                let mut pick = rng.next_f32() * total;
                let index = pool
                    .iter()
                    .position(|k| {
                        pick -= weight(*k);
                        pick < 0.0
                    })
                    .unwrap_or(pool.len() - 1);
                let kind = pool.remove(index);
                let cap = f32::from(kind.cap_for(stats.eterna));
                let at = (cap * level / reference).round().clamp(1.0, cap);
                (kind, at as u8)
            })
            .collect::<Vec<_>>();
        let level = level as u32;
        Offer {
            clues: usize::from(stats.clues).max(1).min(enchants.len()),
            enchants,
            level,
            points: points_for(level),
            gems: i as u32 + 1,
        }
    };
    Some([lot(0), lot(1), lot(2)])
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn stats(eterna: f32, max: f32) -> EnchantingStats {
        EnchantingStats {
            eterna,
            max_eterna: max,
            ..EnchantingStats::default()
        }
    }

    /// Une table en (0,0,0), avec ces blocs : `(position, stats)`, et rien d'opaque entre elle et
    /// les étagères, sauf aux positions de `blocked`.
    fn read(blocks: &[(IVec3, EnchantingStats)], blocked: &[IVec3]) -> TableStats {
        let map: HashMap<IVec3, EnchantingStats> = blocks.iter().cloned().collect();
        read_stats(
            IVec3::ZERO,
            |p| !blocked.contains(&p) && !map.contains_key(&p),
            |p| map.get(&p).cloned(),
        )
    }

    #[test]
    fn the_table_reads_thirty_two_positions_in_two_rings() {
        let positions = shelf_positions();
        assert_eq!(positions.len(), 32);
        let unique: std::collections::HashSet<_> = positions.iter().map(|(o, _)| *o).collect();
        assert_eq!(unique.len(), 32, "pas de doublon");
        for (offset, middle) in &positions {
            // À deux blocs de la table, à sa hauteur ou un bloc au-dessus.
            assert_eq!(offset.x.abs().max(offset.z.abs()), 2);
            assert!((0..=1).contains(&offset.y));
            // La case du milieu est entre la table et l'étagère, à la même hauteur.
            assert_eq!(middle.y, offset.y);
            assert!(middle.x.abs() <= 1 && middle.z.abs() <= 1);
            assert!(
                (offset.x - middle.x).abs() <= 1 && (offset.z - middle.z).abs() <= 1,
                "{offset} {middle}"
            );
            // Une étagère n'est jamais sa propre case d'air.
            assert_ne!(offset, middle);
        }
        // Seize par anneau.
        assert_eq!(positions.iter().filter(|(o, _)| o.y == 0).count(), 16);
        assert_eq!(positions.iter().filter(|(o, _)| o.y == 1).count(), 16);
    }

    #[test]
    fn shelves_need_air_between_them_and_the_table() {
        let shelf = stats(1.0, 15.0);
        let at = IVec3::new(2, 0, 0);
        assert_eq!(read(&[(at, shelf.clone())], &[]).eterna, 1.0);
        // Un bloc plein entre la table et l'étagère : elle ne compte pas.
        let blocked = read(&[(at, shelf.clone())], &[IVec3::new(1, 0, 0)]);
        assert_eq!(blocked.eterna, 0.0);
        assert_eq!(blocked.shelves, 0);
        // Un bloc au même endroit mais un étage plus haut ne gêne pas.
        assert_eq!(
            read(&[(at, shelf.clone())], &[IVec3::new(1, 1, 0)]).eterna,
            1.0
        );
        // Hors des deux anneaux (trop près, trop loin, trop haut) : rien.
        for far in [
            IVec3::new(1, 0, 0),
            IVec3::new(3, 0, 0),
            IVec3::new(2, 2, 0),
            IVec3::new(2, -1, 0),
        ] {
            assert_eq!(read(&[(far, shelf.clone())], &[]).shelves, 0, "{far}");
        }
        // Les coins comptent aussi.
        assert_eq!(read(&[(IVec3::new(-2, 1, 2), shelf)], &[]).shelves, 1);
    }

    #[test]
    fn eterna_adds_up_and_is_capped_by_the_best_shelf_in_steps() {
        // Des étagères de bois (0,5 d'Eterna, plafond 15), sur les 32 positions.
        let wood = stats(0.5, 15.0);
        let all: Vec<(IVec3, EnchantingStats)> = shelf_positions()
            .into_iter()
            .map(|(o, _)| (o, wood.clone()))
            .collect();
        let full = read(&all, &[]);
        assert_eq!(full.shelves, 32);
        assert_eq!(full.raw_eterna, 16.0);
        assert_eq!(full.eterna, 15.0, "le bois plafonne à 15");
        // Un seul bloc plus rare relève le plafond, et la somme suit.
        let mut mixed = all.clone();
        mixed[0].1 = stats(3.2, 100.0);
        let mixed = read(&mixed, &[]);
        assert!((mixed.raw_eterna - (31.0 * 0.5 + 3.2)).abs() < 1e-4);
        assert!((mixed.eterna - mixed.raw_eterna).abs() < 1e-4);
        assert_eq!(mixed.max_eterna, 100.0);
        // 32 étagères rares : 102,4 d'Eterna, ramené à 100.
        let top: Vec<_> = shelf_positions()
            .into_iter()
            .map(|(o, _)| (o, stats(3.2, 100.0)))
            .collect();
        assert_eq!(read(&top, &[]).eterna, 100.0);
        // Les paliers de plafond : 15, 40, 65, 100 selon la meilleure étagère.
        for (max, eterna) in [(15.0, 15.0), (40.0, 40.0), (65.0, 65.0)] {
            let rows: Vec<_> = shelf_positions()
                .into_iter()
                .map(|(o, _)| (o, stats(3.0, max)))
                .collect();
            assert_eq!(read(&rows, &[]).eterna, eterna);
        }
        // Sans étagère, ni plafond ni Eterna.
        assert_eq!(read(&[], &[]).eterna, 0.0);
    }

    #[test]
    fn secondary_stats_add_up_and_the_table_gives_its_base() {
        let table = EnchantingStats {
            max_eterna: 15.0,
            quanta: 15.0,
            clues: 1.0,
            ..EnchantingStats::default()
        };
        let shelf = EnchantingStats {
            eterna: 1.0,
            max_eterna: 15.0,
            quanta: 10.0,
            arcana: 30.0,
            clues: 0.5,
            stability: 40.0,
            ..EnchantingStats::default()
        };
        let seen = read(
            &[
                (IVec3::ZERO, table),
                (IVec3::new(2, 0, 0), shelf.clone()),
                (IVec3::new(-2, 0, 0), shelf.clone()),
                (IVec3::new(0, 1, 2), shelf.clone()),
            ],
            &[],
        );
        assert_eq!(seen.quanta, 45.0, "15 de la table + 3 × 10");
        assert_eq!(seen.arcana, 90.0);
        assert_eq!(seen.stability, 100.0, "plafonnée à 100");
        assert_eq!(seen.clues, 2, "1 + 1,5 arrondi vers le bas");
        assert!(!seen.treasure);
        let treasure = EnchantingStats {
            treasure: true,
            ..shelf
        };
        assert!(read(&[(IVec3::new(2, 0, 0), treasure)], &[]).treasure);
    }

    fn data() -> AdventureData {
        AdventureData::builtin()
    }

    fn table(eterna: f32) -> TableStats {
        TableStats {
            eterna,
            raw_eterna: eterna,
            max_eterna: eterna.max(15.0),
            quanta: 15.0,
            clues: 1,
            ..TableStats::default()
        }
    }

    #[test]
    fn offers_give_several_distinct_enchantments_and_skip_those_owned() {
        let items = items();
        let data = data();
        let axe = items.get(items.id("core:iron_axe").unwrap());
        let stats = table(60.0);
        let lots = offers(&data, Subject::Item(axe), "core:iron_axe", 0, 11, &stats).unwrap();
        for lot in &lots {
            let mut kinds: Vec<u8> = lot.enchants.iter().map(|(k, _)| k.id()).collect();
            kinds.sort_unstable();
            kinds.dedup();
            assert_eq!(kinds.len(), lot.enchants.len(), "pas de doublon");
            assert!(lot.enchants.iter().all(|(k, _)| k.applies_to(axe)));
            // Rien de rare sans étagère « trésor ».
            assert!(lot.enchants.iter().all(|(k, _)| !k.is_treasure()));
        }
        assert!(
            lots[2].enchants.len() >= 2,
            "le dernier lot en donne plusieurs"
        );
        // Appliquer un lot garde l'usure et ajoute les enchantements.
        let damage = lots[2].apply(5);
        assert_eq!(enchanting::wear(damage), 5);
        assert_eq!(
            enchanting::enchantments_of(damage).len(),
            lots[2].enchants.len()
        );
        // Ce que l'objet a déjà n'est plus proposé.
        let after = offers(
            &data,
            Subject::Item(axe),
            "core:iron_axe",
            damage,
            11,
            &stats,
        );
        if let Some(after) = after {
            for lot in &after {
                assert!(lot.enchants.iter().all(|(k, _)| {
                    !enchanting::enchantments_of(damage)
                        .iter()
                        .any(|(o, _)| o == k)
                }));
            }
        }
        // Un bloc ou un aliment ne reçoit rien.
        let stone = items.get(items.id("core:stone").unwrap());
        assert!(offers(&data, Subject::Item(stone), "core:stone", 0, 1, &stats).is_none());
    }

    #[test]
    fn levels_and_prices_follow_the_eterna_and_stay_under_the_cap() {
        let items = items();
        let data = data();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        for eterna in [0.0, 15.0, 40.0, 100.0] {
            let stats = TableStats {
                quanta: 0.0,
                ..table(eterna)
            };
            let lots = offers(&data, Subject::Item(sword), "s", 0, 7, &stats).unwrap();
            let mut last_points = 0;
            for lot in &lots {
                assert!(
                    lot.points >= last_points,
                    "le prix monte d'un lot à l'autre"
                );
                last_points = lot.points;
                assert_eq!(lot.points, points_for(lot.level));
                for (kind, level) in &lot.enchants {
                    assert!(*level >= 1 && *level <= kind.cap_for(eterna));
                }
            }
            // Sans hasard, le niveau de table est exactement la part de l'Eterna.
            assert_eq!(lots[2].level, eterna.round().max(3.0) as u32);
            assert_eq!(lots[0].gems, 1);
            assert_eq!(lots[2].gems, 3);
        }
        // À 100 d'Eterna, le dernier lot atteint le plafond de 100 niveaux et 400 points.
        let top = TableStats {
            quanta: 0.0,
            ..table(100.0)
        };
        let lots = offers(&data, Subject::Item(sword), "s", 0, 7, &top).unwrap();
        assert_eq!(lots[2].level, 100);
        assert_eq!(lots[2].points, 400);
        let best = lots[2].enchants.iter().map(|(_, l)| *l).max().unwrap();
        assert!(best >= 6, "niveau {best}");
        // Sans étagère, les niveaux restent bas.
        let bare = TableStats {
            quanta: 0.0,
            ..table(0.0)
        };
        let lots = offers(&data, Subject::Item(sword), "s", 0, 7, &bare).unwrap();
        assert!(
            lots.iter()
                .all(|l| l.enchants.iter().all(|(_, level)| *level <= 3))
        );
    }

    #[test]
    fn offers_are_deterministic_and_change_with_the_seed() {
        let items = items();
        let data = data();
        let pick = items.get(items.id("core:iron_pickaxe").unwrap());
        let stats = table(40.0);
        let first = offers(&data, Subject::Item(pick), "p", 0, 5, &stats).unwrap();
        assert_eq!(
            first,
            offers(&data, Subject::Item(pick), "p", 0, 5, &stats).unwrap()
        );
        assert!(
            (0..40)
                .any(|s| offers(&data, Subject::Item(pick), "p", 0, s, &stats).unwrap() != first),
            "la graine change l'offre"
        );
    }

    #[test]
    fn stability_only_lets_luck_help_and_quanta_spreads_the_levels() {
        let items = items();
        let data = data();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let levels = |stats: &TableStats| -> Vec<u32> {
            (0..200)
                .map(|seed| {
                    offers(&data, Subject::Item(sword), "x", 0, seed, stats).unwrap()[2].level
                })
                .collect()
        };
        let wild = TableStats {
            quanta: 80.0,
            ..table(50.0)
        };
        let spread = levels(&wild);
        assert!(spread.iter().any(|&l| l < 50) && spread.iter().any(|&l| l > 50));
        // Stabilité pleine : jamais en dessous de la valeur sans hasard.
        let steady = TableStats {
            stability: 100.0,
            ..wild.clone()
        };
        let stable = levels(&steady);
        assert!(stable.iter().all(|&l| l >= 50), "{:?}", stable.iter().min());
        assert!(stable.iter().any(|&l| l > 50));
        // Sans Quanta, aucun écart.
        let flat = TableStats {
            quanta: 0.0,
            ..wild
        };
        assert!(levels(&flat).iter().all(|&l| l == 50));
    }

    #[test]
    fn treasure_enchantments_need_a_treasure_shelf() {
        let items = items();
        let data = data();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        let seen = |stats: &TableStats| -> Vec<Enchantment> {
            let mut all = Vec::new();
            for seed in 0..300 {
                for lot in offers(&data, Subject::Item(sword), "t", 0, seed, stats).unwrap() {
                    all.extend(lot.enchants.iter().map(|(k, _)| *k));
                }
            }
            all
        };
        let plain = seen(&table(50.0));
        assert!(!plain.contains(&Enchantment::LIFE_STEAL));
        assert!(!plain.contains(&Enchantment::MAGNET));
        let rich = TableStats {
            treasure: true,
            arcana: 60.0,
            ..table(50.0)
        };
        let rare = seen(&rich);
        assert!(rare.contains(&Enchantment::LIFE_STEAL));
        assert!(rare.contains(&Enchantment::MAGNET));
    }

    #[test]
    fn clues_show_the_first_enchantments_and_hide_the_rest() {
        let items = items();
        let data = data();
        let axe = items.get(items.id("core:iron_axe").unwrap());
        let stats = TableStats {
            arcana: 100.0,
            ..table(60.0)
        };
        // Une graine qui donne un gros lot (Arcana au plus haut en fait presque toujours un).
        let seed = (0..50)
            .find(|&seed| {
                offers(&data, Subject::Item(axe), "a", 0, seed, &stats)
                    .unwrap()
                    .iter()
                    .any(|l| l.enchants.len() >= 3)
            })
            .expect("un gros lot");
        let lots = offers(&data, Subject::Item(axe), "a", 0, seed, &stats).unwrap();
        let big = lots.iter().find(|l| l.enchants.len() >= 3).unwrap();
        assert_eq!(big.clues, 1);
        let preview = big.preview();
        assert_eq!(preview.matches('?').count(), big.enchants.len() - 1);
        assert!(preview.starts_with(&enchanting::label(big.enchants[0].0, big.enchants[0].1)));
        // Avec trois indices, trois sont montrés.
        let more = TableStats { clues: 3, ..stats };
        let lots = offers(&data, Subject::Item(axe), "a", 0, seed, &more).unwrap();
        let big = lots.iter().find(|l| l.enchants.len() >= 3).unwrap();
        assert_eq!(big.clues, 3);
        assert_eq!(
            big.preview().matches('?').count(),
            big.enchants.len() - 3,
            "trois montrés, le reste caché"
        );
        assert_eq!(big.label().matches('?').count(), 0);
    }

    #[test]
    fn the_item_limit_caps_the_offered_count() {
        let items = items();
        let data = data();
        let sword = items.get(items.id("core:iron_sword").unwrap());
        // Quatre enchantements déjà posés, et la limite de six : au plus deux de plus.
        let mut damage = 0;
        for kind in [
            Enchantment::SHARPNESS,
            Enchantment::UNBREAKING,
            Enchantment::LOOTING,
            Enchantment::MAGNET,
        ] {
            damage = enchanting::with_enchantment(damage, kind, 1);
        }
        let stats = TableStats {
            arcana: 100.0,
            treasure: true,
            ..table(80.0)
        };
        for seed in 0..50 {
            for lot in offers(&data, Subject::Item(sword), "s", damage, seed, &stats).unwrap() {
                assert!(lot.enchants.len() <= data.max_enchantments - 4);
            }
        }
        // À la limite : plus d'offre.
        let full = enchanting::with_enchantment(
            enchanting::with_enchantment(damage, Enchantment::LIFE_STEAL, 1),
            Enchantment::EFFICIENCY,
            1,
        );
        assert_eq!(
            enchanting::enchantments_of(full).len(),
            data.max_enchantments
        );
        assert!(offers(&data, Subject::Item(sword), "s", full, 1, &stats).is_none());
    }
}
