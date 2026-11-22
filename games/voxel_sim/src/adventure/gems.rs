//! Les gemmes : les poser dans les emplacements d'un objet, les retirer, ajouter un emplacement.
//!
//! # Les emplacements
//!
//! Un objet à affixes a 0 à 3 emplacements de gemme, tirés avec sa rareté (voir
//! `adventure/affixes.rs`) ; le sceau d'emplacement en ajoute, jusqu'à `max_sockets`
//! (`rarities.ron`). Ils sont rangés dans la fiche de l'objet (`ItemData::sockets`) : vide, ou la
//! **clé de l'objet gemme** posée (`core:gem_ruby_3`). Les données (`gems.ron`) disent à quelle
//! sorte et à quelle qualité cette clé correspond ; elles ne sont donc jamais copiées dans la
//! fiche, et un mod qui retire une gemme laisse un emplacement plein mais sans effet.
//!
//! # Le bonus
//!
//! Il dépend de la **catégorie** de l'objet (`weapon`, `tool`, `armor`...) : un rubis donne des
//! dégâts à une épée, de la vitesse de minage à une pioche et de la vie à une armure. La première
//! ligne de `bonuses` de la sorte dont une catégorie convient s'applique, avec la valeur de la
//! qualité de la gemme. [`modifiers`] en fait des modificateurs d'attributs (voir
//! `player/gear.rs`).
//!
//! # Les gestes
//!
//! Dans l'inventaire, avec un objet au curseur, on clique sur un autre objet :
//!
//! - **une gemme** sur un objet qui a un emplacement libre : elle y entre ;
//! - **un sceau de retrait** : toutes les gemmes de l'objet sortent (elles sont rendues, jamais
//!   détruites) et le sceau est consommé ;
//! - **un sceau d'emplacement** : un emplacement vide de plus (un objet sans fiche en reçoit une).
//!
//! Sur un objet qui n'a aucun emplacement, ou une case vide, le clic reste un clic ordinaire (on
//! pose ou on échange la pile). La logique des gestes est dans `adventure/gem_sockets.rs` ; ce
//! module-ci ne porte que les données (bonus, noms) que lisent aussi les bulles d'aide.

use engine_voxel::ItemDef;

use crate::adventure::affixes;
use crate::adventure::data::AdventureData;
use crate::player::attributes::Modifier;
use crate::player::item_data::{self, ItemData};

/// Le sceau qui rend les gemmes.
pub const SEAL_UNSOCKET: &str = "core:seal_unsocket";
/// Le sceau qui ajoute un emplacement.
pub const SEAL_SOCKET: &str = "core:seal_socket";

/// Les modificateurs que les gemmes d'une fiche donnent à un objet de cette catégorie.
pub fn modifiers(data: &AdventureData, def: &ItemDef, sheet: &ItemData) -> Vec<Modifier> {
    let cats = affixes::categories(def);
    sheet
        .sockets
        .iter()
        .flatten()
        .filter_map(|key| {
            let (kind, quality) = data.gem_of(key)?;
            let bonus = kind
                .bonuses
                .iter()
                .find(|b| b.categories.iter().any(|c| cats.contains(&c.as_str())))?;
            Some(Modifier {
                attribute: bonus.attribute,
                flat: bonus.flat.get(quality).copied().unwrap_or(0.0),
                percent: bonus.percent.get(quality).copied().unwrap_or(0.0),
            })
        })
        .collect()
}

/// Le nom d'une gemme, avec sa qualité (« Rubis parfait ») et sa couleur.
pub fn gem_label(data: &AdventureData, key: &str) -> Option<(String, crate::player::tone::Tone)> {
    let (kind, quality) = data.gem_of(key)?;
    let (male, female) = data.qualities.get(quality)?;
    let (r, g, b) = kind.color;
    Some((
        format!(
            "{} {}",
            kind.name,
            if kind.feminine { female } else { male }
        ),
        crate::player::tone::Tone::Rgb(r, g, b),
    ))
}

/// Ce que la gemme apporte, une ligne par catégorie : « Arme : Dégâts +2,8 ».
pub fn gem_effects(data: &AdventureData, key: &str) -> Vec<String> {
    let Some((kind, quality)) = data.gem_of(key) else {
        return Vec::new();
    };
    let names = [
        ("weapon", "Arme"),
        ("tool", "Outil"),
        ("armor", "Armure"),
        ("head", "Casque"),
        ("chest", "Plastron"),
        ("legs", "Jambières"),
        ("feet", "Bottes"),
    ];
    kind.bonuses
        .iter()
        .filter_map(|bonus| {
            let flat = bonus.flat.get(quality).copied().unwrap_or(0.0);
            let percent = bonus.percent.get(quality).copied().unwrap_or(0.0);
            let text = affixes::describe(&crate::player::item_data::Affix {
                key: String::new(),
                attribute: bonus.attribute.id(),
                flat,
                percent,
            })?;
            let label = bonus
                .categories
                .iter()
                .map(|c| {
                    names
                        .iter()
                        .find(|(k, _)| k == c)
                        .map_or(c.as_str(), |(_, n)| *n)
                })
                .collect::<Vec<_>>()
                .join(" ou ");
            Some(format!("{label} : {text}"))
        })
        .collect()
}

/// Le nombre d'emplacements d'un objet (vides ou pleins).
pub fn socket_count(damage: u32) -> usize {
    item_data::sheet(damage).map_or(0, |s| s.sockets.len())
}

/// Les gemmes posées sur un objet.
pub fn gems_of(damage: u32) -> Vec<String> {
    item_data::sheet(damage)
        .map(|s| s.sockets.into_iter().flatten().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::attributes::Attribute;
    use engine_voxel::ItemRegistry;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn the_bonus_depends_on_the_category_of_the_item() {
        let data = AdventureData::builtin();
        let items = items();
        let sheet = ItemData {
            sockets: vec![Some("core:gem_ruby_5".into())],
            ..ItemData::default()
        };
        let on = |key: &str| modifiers(&data, items.get(items.id(key).unwrap()), &sheet);
        let sword = on("core:iron_sword");
        assert_eq!(sword.len(), 1);
        assert_eq!(sword[0].attribute, Attribute::Damage);
        assert_eq!(sword[0].flat, 4.0, "rubis parfait sur une arme");
        let pickaxe = on("core:iron_pickaxe");
        assert_eq!(pickaxe[0].attribute, Attribute::MiningSpeed);
        assert_eq!(pickaxe[0].percent, 0.45);
        let helmet = on("core:iron_helmet");
        assert_eq!(helmet[0].attribute, Attribute::MaxHealth);
        assert_eq!(helmet[0].flat, 6.0);
        // La qualité compte : un rubis fêlé donne bien moins.
        let weak = ItemData {
            sockets: vec![Some("core:gem_ruby_1".into())],
            ..ItemData::default()
        };
        let sword_def = items.get(items.id("core:iron_sword").unwrap());
        assert!(modifiers(&data, sword_def, &weak)[0].flat < 1.0);
        // Une clé inconnue, un emplacement vide : rien.
        let odd = ItemData {
            sockets: vec![Some("mod:inconnue".into()), None],
            ..ItemData::default()
        };
        assert!(modifiers(&data, sword_def, &odd).is_empty());
    }

    #[test]
    fn gems_are_named_and_described_for_the_tooltip() {
        let data = AdventureData::builtin();
        let (name, _) = gem_label(&data, "core:gem_topaz_4").unwrap();
        assert_eq!(name, "Topaze pure", "féminin");
        assert_eq!(
            gem_label(&data, "core:gem_ruby_5").unwrap().0,
            "Rubis parfait"
        );
        assert!(gem_label(&data, "core:coal").is_none());
        let lines = gem_effects(&data, "core:gem_ruby_5");
        assert_eq!(lines[0], "Arme : Dégâts +4,0");
        assert!(lines[1].starts_with("Outil : Vitesse de minage"));
    }
}
