//! Les objets à affixes : tirer une rareté, des affixes, des emplacements de gemmes, et composer le
//! nom. Un module **pur** : il lit les données (`adventure_data.rs`) et un générateur aléatoire,
//! jamais l'inventaire ni le monde. Le même tirage (même graine, même objet) donne toujours la même
//! fiche : c'est ce qui rend les coffres de structure stables d'une ouverture à l'autre.
//!
//! # Le tirage
//!
//! 1. **La rareté** : chaque rareté a un poids (`rarities.ron`) ; le palier du monde le multiplie
//!    par `(1 + rarity_boost) ^ numéro` ([`roll_rarity`]) : plus le palier est haut, plus les
//!    raretés hautes sortent. Un butin peut imposer une rareté minimale.
//! 2. **Les affixes** : la rareté donne un nombre (min, max) ; on les tire au poids parmi ceux dont
//!    une catégorie convient à l'objet ([`categories`]) et dont la rareté minimale est atteinte,
//!    sans en répéter un. Chacun reçoit une valeur dans l'intervalle de sa rareté.
//! 3. **Les emplacements** : un nombre (min, max) de gemmes possibles, selon la rareté.
//! 4. **Le nom** : l'objet de base, puis l'adjectif du premier affixe « préfixe » (accordé en genre
//!    et en nombre), puis le complément du premier affixe « suffixe » : « Épée en fer acérée de la
//!    tempête » ([`compose_name`]).
//!
//! # Les catégories d'un objet
//!
//! `weapon` (épée, hache), `tool` (pioche, hache, pelle, houe, cisailles), `armor` et la pièce
//! (`head`, `chest`, `legs`, `feet`). Une pioche n'a donc jamais d'affixe d'arme.
//!
//! # Les coffres
//!
//! Le moteur range dans un coffre de structure un marqueur « à tirer » (voir
//! `item_data::is_pending`) avec une graine : [`resolve_pending`] en fait la fiche, avec le palier
//! du moment.

use engine_voxel::{ArmorSlot, ItemDef, Rng64, ToolKind};

use crate::adventure::data::{AdventureData, Place};
use crate::enchanting;
use crate::player::item_data::{self, Affix, ItemData};

/// Les catégories d'un objet (vide : il ne peut pas porter d'affixe).
pub fn categories(def: &ItemDef) -> Vec<&'static str> {
    let mut out = Vec::new();
    if let Some(tool) = def.tool {
        match tool.kind {
            ToolKind::Sword => out.push("weapon"),
            ToolKind::Axe => out.extend(["weapon", "tool"]),
            ToolKind::Pickaxe | ToolKind::Shovel | ToolKind::Hoe | ToolKind::Shears => {
                out.push("tool")
            }
        }
    }
    if let Some(armor) = def.armor {
        out.push("armor");
        out.push(match armor.slot {
            ArmorSlot::Head => "head",
            ArmorSlot::Chest => "chest",
            ArmorSlot::Legs => "legs",
            ArmorSlot::Feet => "feet",
        });
    }
    out
}

/// Cet objet peut-il porter des affixes ?
pub fn can_carry(def: &ItemDef) -> bool {
    !categories(def).is_empty()
}

/// Tire une rareté. `boost` : la hausse du palier ; `min` : la rareté minimale.
pub fn roll_rarity(data: &AdventureData, boost: f32, min: u8, rng: &mut Rng64) -> u8 {
    let weights: Vec<f32> = data
        .rarities
        .iter()
        .enumerate()
        .map(|(i, r)| {
            if i < usize::from(min) {
                0.0
            } else {
                r.weight * (1.0 + boost.max(0.0)).powi(i as i32)
            }
        })
        .collect();
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return min.min(data.rarities.len().saturating_sub(1) as u8);
    }
    let mut pick = rng.next_f32() * total;
    for (i, w) in weights.iter().enumerate() {
        if pick < *w {
            return i as u8;
        }
        pick -= w;
    }
    (weights.len() - 1) as u8
}

/// Un nombre entier entre `min` et `max` (inclus).
fn between(rng: &mut Rng64, (min, max): (u32, u32)) -> u32 {
    if max <= min {
        min
    } else {
        min + rng.below(max - min + 1)
    }
}

/// Une valeur dans l'intervalle, arrondie à trois décimales.
fn lerp((low, high): (f32, f32), rng: &mut Rng64) -> f32 {
    let value = low + (high - low) * rng.next_f32();
    (value * 1000.0).round() / 1000.0
}

/// Tire les affixes d'un objet de ces catégories et de cette rareté.
pub fn roll_affixes(
    data: &AdventureData,
    categories: &[&str],
    rarity: u8,
    rng: &mut Rng64,
) -> Vec<Affix> {
    let Some(def) = data.rarity(rarity) else {
        return Vec::new();
    };
    let mut pool: Vec<_> = data
        .affixes
        .iter()
        .filter(|a| {
            a.min_rarity <= rarity
                && a.weight > 0
                && a.categories
                    .iter()
                    .any(|c| categories.contains(&c.as_str()))
        })
        .collect();
    let count = (between(rng, def.affixes) as usize).min(pool.len());
    let mut out = Vec::new();
    for _ in 0..count {
        let total: u32 = pool.iter().map(|a| a.weight).sum();
        let mut pick = rng.below(total.max(1));
        let index = pool
            .iter()
            .position(|a| {
                if pick < a.weight {
                    true
                } else {
                    pick -= a.weight;
                    false
                }
            })
            .unwrap_or(0);
        let affix = pool.remove(index);
        out.push(Affix {
            key: affix.id.clone(),
            attribute: affix.attribute.id(),
            flat: affix.flat_range(rarity).map_or(0.0, |r| lerp(r, rng)),
            percent: affix.percent_range(rarity).map_or(0.0, |r| lerp(r, rng)),
        });
    }
    out
}

/// Le nom composé (voir la doc du module).
pub fn compose_name(data: &AdventureData, base: &str, affixes: &[Affix]) -> String {
    let first = base.split_whitespace().next().unwrap_or("");
    let feminine = data.feminine.iter().any(|w| w == first);
    let plural = data.plural.iter().any(|w| w == first);
    let mut name = base.to_string();
    let mut suffix = None;
    let mut prefix_done = false;
    for affix in affixes {
        match data.affix(&affix.key).map(|a| &a.place) {
            Some(Place::Prefix(male, female)) if !prefix_done => {
                prefix_done = true;
                let mut word = if feminine { female } else { male }.clone();
                if plural && !word.ends_with(['s', 'x']) {
                    word.push('s');
                }
                name = format!("{name} {word}");
            }
            Some(Place::Suffix(text)) if suffix.is_none() => suffix = Some(text.clone()),
            _ => {}
        }
    }
    if let Some(suffix) = suffix {
        name = format!("{name} {suffix}");
    }
    name
}

/// Une fiche neuve pour cet objet : rareté, affixes, emplacements vides et nom. L'usure et les
/// enchantements ne sont pas touchés (voir [`apply`]).
pub fn roll_sheet(data: &AdventureData, def: &ItemDef, rarity: u8, rng: &mut Rng64) -> ItemData {
    let cats = categories(def);
    let affixes = roll_affixes(data, &cats, rarity, rng);
    let sockets = data.rarity(rarity).map_or(0, |r| {
        between(rng, r.sockets).min(data.max_sockets) as usize
    });
    ItemData {
        rarity,
        name: Some(compose_name(data, &def.name, &affixes)),
        affixes,
        sockets: vec![None; sockets],
        ..ItemData::default()
    }
}

/// Tire une rareté puis la fiche. `None` si l'objet ne peut pas porter d'affixe.
pub fn generate(
    data: &AdventureData,
    def: &ItemDef,
    boost: f32,
    min_rarity: u8,
    seed: u64,
) -> Option<ItemData> {
    if !can_carry(def) || data.rarities.is_empty() {
        return None;
    }
    let mut rng = Rng64::new(seed);
    let rarity = roll_rarity(data, boost, min_rarity, &mut rng);
    Some(roll_sheet(data, def, rarity, &mut rng))
}

/// Donne cette fiche à une pile : rareté, affixes, emplacements et nom remplacent ceux de la pile ;
/// l'usure et les enchantements qu'elle avait sont gardés (c'est la migration vers une fiche).
/// `false` si le magasin de fiches est plein.
pub fn apply(damage: &mut u32, rolled: ItemData) -> bool {
    enchanting::edit_sheet(damage, |sheet| {
        sheet.rarity = rolled.rarity;
        sheet.affixes = rolled.affixes;
        sheet.sockets = rolled.sockets;
        sheet.name = rolled.name;
    })
}

/// Une graine qui dépend de l'objet autant que du tirage : deux objets différents d'un même coffre
/// ne tirent pas la même chose.
fn item_seed(key: &str, seed: u64) -> u64 {
    key.bytes().fold(seed ^ 0x51ED_270B, |h, b| {
        h.wrapping_mul(0x100_0000_01B3).wrapping_add(u64::from(b))
    })
}

/// Remplace le marqueur « à tirer » d'un coffre par une vraie fiche (la même à chaque appel pour la
/// même graine et le même palier). Une pile qui n'est pas un marqueur est rendue telle quelle.
pub fn resolve_pending(data: &AdventureData, def: &ItemDef, damage: u32, boost: f32) -> u32 {
    if !item_data::is_pending(damage) {
        return damage;
    }
    let min = ((damage >> 16) & 0xF) as u8;
    let seed = item_seed(&def.key, u64::from(damage & 0xFFFF));
    let mut out = 0;
    match generate(data, def, boost, min, seed) {
        Some(sheet) => {
            if apply(&mut out, sheet) {
                out
            } else {
                0
            }
        }
        None => 0,
    }
}

/// La rareté à montrer d'une fiche : `None` pour une fiche d'enchantements sans affixe (un objet
/// ordinaire).
pub fn display_rarity(sheet: &ItemData) -> Option<u8> {
    (!sheet.affixes.is_empty()).then_some(sheet.rarity)
}

/// La rareté à montrer d'une pile (par son `damage`).
pub fn rarity_of(damage: u32) -> Option<u8> {
    item_data::sheet(damage).and_then(|s| display_rarity(&s))
}

/// Un affixe écrit pour les bulles : « Dégâts +1,8 », « Vitesse de minage +15 % ».
pub fn describe(affix: &Affix) -> Option<String> {
    use crate::player::attributes::Attribute;
    let attribute = Attribute::from_id(affix.attribute)?;
    let mut parts = Vec::new();
    if affix.flat != 0.0 {
        // Un facteur s'écrit « ×0,69 » : en bonus, « +0,69 » (le × n'a pas de sens après le signe).
        let value = attribute.format(affix.flat.abs());
        parts.push(format!(
            "{}{}",
            sign(affix.flat),
            value.trim_start_matches('×')
        ));
    }
    if affix.percent != 0.0 {
        let percent = format!("{:.0}", affix.percent.abs() * 100.0);
        parts.push(format!("{}{percent} %", sign(affix.percent)));
    }
    (!parts.is_empty()).then(|| format!("{} {}", attribute.name(), parts.join(" et ")))
}

fn sign(value: f32) -> &'static str {
    if value < 0.0 { "-" } else { "+" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn def<'a>(items: &'a ItemRegistry, key: &str) -> &'a ItemDef {
        items.get(items.id(key).unwrap())
    }

    #[test]
    fn items_know_their_categories() {
        let items = items();
        assert_eq!(categories(def(&items, "core:iron_sword")), ["weapon"]);
        assert_eq!(categories(def(&items, "core:iron_axe")), ["weapon", "tool"]);
        assert_eq!(categories(def(&items, "core:iron_pickaxe")), ["tool"]);
        assert_eq!(
            categories(def(&items, "core:iron_boots")),
            ["armor", "feet"]
        );
        assert!(categories(def(&items, "core:stick")).is_empty());
        assert!(!can_carry(def(&items, "core:coal")));
    }

    #[test]
    fn the_same_seed_always_draws_the_same_item() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = def(&items, "core:iron_sword");
        let a = generate(&data, sword, 0.5, 0, 1234).unwrap();
        let b = generate(&data, sword, 0.5, 0, 1234).unwrap();
        assert_eq!(a, b);
        let other = (0..20).map(|s| generate(&data, sword, 0.5, 0, s).unwrap());
        assert!(
            other.clone().any(|s| s != a),
            "les graines changent le tirage"
        );
        // Un objet qui ne porte rien ne tire rien.
        assert!(generate(&data, def(&items, "core:stick"), 0.0, 0, 1).is_none());
    }

    #[test]
    fn rarity_weights_follow_the_data_and_the_tier_raises_them() {
        let data = AdventureData::builtin();
        let histogram = |boost: f32| {
            let mut rng = Rng64::new(99);
            let mut counts = [0u32; 5];
            for _ in 0..20_000 {
                counts[usize::from(roll_rarity(&data, boost, 0, &mut rng))] += 1;
            }
            counts
        };
        let plain = histogram(0.0);
        // Les poids 60/25/10/4/1 : de plus en plus rare.
        assert!(plain.windows(2).all(|w| w[0] > w[1]), "{plain:?}");
        assert!((plain[0] as f32 / 20_000.0 - 0.6).abs() < 0.02);
        // Un palier haut décale la masse vers le haut.
        let boosted = histogram(2.2);
        assert!(boosted[4] > plain[4] * 4, "{boosted:?} contre {plain:?}");
        assert!(boosted[0] < plain[0]);
        // Une rareté minimale l'interdit en dessous.
        let mut rng = Rng64::new(5);
        for _ in 0..500 {
            assert!(roll_rarity(&data, 0.0, 3, &mut rng) >= 3);
        }
    }

    #[test]
    fn affixes_fit_the_category_the_count_and_the_value_ranges() {
        let data = AdventureData::builtin();
        let items = items();
        let pickaxe = def(&items, "core:iron_pickaxe");
        let helmet = def(&items, "core:iron_helmet");
        for seed in 0..200 {
            for rarity in 0..5u8 {
                let mut rng = Rng64::new(seed);
                let sheet = roll_sheet(&data, pickaxe, rarity, &mut rng);
                let rule = data.rarity(rarity).unwrap();
                assert!(sheet.affixes.len() as u32 >= rule.affixes.0.min(1));
                assert!(sheet.affixes.len() as u32 <= rule.affixes.1);
                let mut seen = std::collections::HashSet::new();
                for affix in &sheet.affixes {
                    let def = data.affix(&affix.key).unwrap();
                    assert!(seen.insert(&affix.key), "jamais deux fois le même affixe");
                    assert!(def.categories.iter().any(|c| c == "tool"), "{}", affix.key);
                    assert!(def.min_rarity <= rarity);
                    if let Some((lo, hi)) = def.flat_range(rarity) {
                        assert!(
                            (lo..=hi).contains(&affix.flat),
                            "{} {}",
                            affix.key,
                            affix.flat
                        );
                    }
                    if let Some((lo, hi)) = def.percent_range(rarity) {
                        assert!((lo..=hi).contains(&affix.percent));
                    }
                }
                let sockets = sheet.sockets.len() as u32;
                assert!((rule.sockets.0..=rule.sockets.1).contains(&sockets));
            }
        }
        // Une armure de pieds peut avoir « du vent », jamais « de la tempête ».
        let mut keys = std::collections::HashSet::new();
        for seed in 0..400 {
            let mut rng = Rng64::new(seed);
            for affix in roll_sheet(&data, helmet, 4, &mut rng).affixes {
                keys.insert(affix.key);
            }
        }
        assert!(keys.contains("core:sturdy"));
        assert!(!keys.contains("core:storm") && !keys.contains("core:gale"));
    }

    #[test]
    fn a_higher_rarity_gives_higher_values() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = def(&items, "core:iron_sword");
        let mean = |rarity: u8| {
            let (mut sum, mut n) = (0.0, 0);
            for seed in 0..300 {
                let mut rng = Rng64::new(seed);
                for a in roll_affixes(&data, &categories(sword), rarity, &mut rng) {
                    if a.key == "core:sharp" {
                        sum += a.flat;
                        n += 1;
                    }
                }
            }
            sum / n as f32
        };
        assert!(
            mean(0) < mean(2) && mean(2) < mean(4),
            "{} {} {}",
            mean(0),
            mean(2),
            mean(4)
        );
    }

    #[test]
    fn names_are_composed_with_agreement() {
        let data = AdventureData::builtin();
        let affix = |key: &str| Affix {
            key: key.into(),
            attribute: 0,
            flat: 1.0,
            percent: 0.0,
        };
        let list = [
            affix("core:sharp"),
            affix("core:storm"),
            affix("core:vampire"),
        ];
        assert_eq!(
            compose_name(&data, "Épée en fer", &list),
            "Épée en fer acérée de la tempête",
            "féminin, et un seul suffixe"
        );
        assert_eq!(
            compose_name(
                &data,
                "Casque en fer",
                &[affix("core:sharp"), affix("core:vampire")]
            ),
            "Casque en fer acéré du vampire"
        );
        assert_eq!(
            compose_name(&data, "Bottes en or", &[affix("core:nimble")]),
            "Bottes en or agiles",
            "pluriel"
        );
        assert_eq!(
            compose_name(
                &data,
                "Pioche en fer",
                &[affix("core:efficient"), affix("core:long")]
            ),
            "Pioche en fer efficace",
            "un seul préfixe"
        );
        assert_eq!(compose_name(&data, "Épée en fer", &[]), "Épée en fer");
        // Un affixe inconnu (mod retiré) ne change pas le nom.
        assert_eq!(
            compose_name(&data, "Épée en fer", &[affix("mod:disparu")]),
            "Épée en fer"
        );
    }

    #[test]
    fn apply_keeps_wear_and_enchantments_and_replaces_the_affixes() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = def(&items, "core:iron_sword");
        let mut damage = enchanting::with_enchantment(7, enchanting::Enchantment::SHARPNESS, 2);
        let rolled = generate(&data, sword, 0.0, 2, 42).unwrap();
        assert!(apply(&mut damage, rolled.clone()));
        let sheet = item_data::sheet(damage).unwrap();
        assert_eq!(sheet.wear, 7, "l'usure est gardée");
        assert_eq!(
            sheet.enchantments,
            vec![(enchanting::Enchantment::SHARPNESS.id(), 2)]
        );
        assert_eq!(sheet.affixes, rolled.affixes);
        assert_eq!(sheet.name, rolled.name);
        assert!(rarity_of(damage).is_some());
        // Un nouvel appel remplace les affixes, sans en cumuler.
        let again = generate(&data, sword, 0.0, 4, 43).unwrap();
        assert!(apply(&mut damage, again.clone()));
        assert_eq!(item_data::sheet(damage).unwrap().affixes, again.affixes);
    }

    #[test]
    fn a_chest_marker_resolves_to_the_same_sheet_every_time() {
        let data = AdventureData::builtin();
        let items = items();
        let sword = def(&items, "core:iron_sword");
        let marker = engine_voxel::pending_loot(2, 0x1234);
        assert!(item_data::is_pending(marker) && !item_data::is_extended(marker));
        assert_eq!(
            enchanting::wear(marker),
            0,
            "le marqueur n'est pas de l'usure"
        );
        let a = resolve_pending(&data, sword, marker, 0.0);
        let b = resolve_pending(&data, sword, marker, 0.0);
        assert!(item_data::is_extended(a));
        assert_eq!(item_data::sheet(a), item_data::sheet(b));
        assert!(
            item_data::sheet(a).unwrap().rarity >= 2,
            "la rareté minimale du coffre"
        );
        // Un autre objet de la même graine tire autre chose ; une pile ordinaire ne change pas.
        assert_eq!(resolve_pending(&data, sword, 17, 0.0), 17);
        let helmet = def(&items, "core:iron_helmet");
        assert!(item_data::sheet(resolve_pending(&data, helmet, marker, 0.0)).is_some());
        // Un objet qui ne peut rien porter perd son marqueur.
        assert_eq!(
            resolve_pending(&data, def(&items, "core:coal"), marker, 0.0),
            0
        );
    }

    #[test]
    fn plain_enchanted_items_show_no_rarity() {
        let damage = enchanting::with_enchantment(
            enchanting::with_enchantment(0, enchanting::Enchantment::SHARPNESS, 1),
            enchanting::Enchantment::UNBREAKING,
            1,
        );
        assert!(item_data::is_extended(damage));
        assert_eq!(
            rarity_of(damage),
            None,
            "une fiche sans affixe n'a pas de rareté"
        );
        assert_eq!(rarity_of(0), None);
    }

    #[test]
    fn affixes_are_described_for_the_tooltip() {
        let flat = Affix {
            key: "x".into(),
            attribute: crate::player::attributes::Attribute::Damage.id(),
            flat: 1.8,
            percent: 0.0,
        };
        assert_eq!(describe(&flat).unwrap(), "Dégâts +1,8");
        let both = Affix {
            percent: 0.12,
            ..flat.clone()
        };
        assert_eq!(describe(&both).unwrap(), "Dégâts +1,8 et +12 %");
        let crit = Affix {
            attribute: crate::player::attributes::Attribute::CritChance.id(),
            flat: 0.07,
            ..flat
        };
        assert_eq!(describe(&crit).unwrap(), "Chance critique +7 %");
        let factor = Affix {
            attribute: crate::player::attributes::Attribute::CritDamage.id(),
            flat: 0.69,
            ..both
        };
        assert_eq!(
            describe(&factor).unwrap(),
            "Dégâts critiques +0,69 et +12 %"
        );
    }
}
