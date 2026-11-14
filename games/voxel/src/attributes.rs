//! Les attributs du joueur : des valeurs calculées (dégâts, armure, vitesse...) à partir de
//! sources (objet en main, armure portée, potions). Un module **pur** : il ne connaît ni
//! l'inventaire ni le monde, seulement des [`Source`] de [`Modifier`] et des formules. Le
//! rassemblement des sources est dans `gear.rs`, l'affichage dans `attributes_ui.rs`.
//!
//! # Le calcul
//!
//! Chaque attribut a une valeur de base (`attributes` dans `assets/core/player.ron`, voir
//! [`BaseAttributes`]) ; chaque source y ajoute un bonus **fixe** (`flat`) et/ou une **part**
//! (`percent`, 0,25 = +25 %). La valeur est
//! `(base + somme des fixes) × (1 + somme des parts)`, bornée par [`Attribute::bounds`]. Les
//! attributs de vitesse, de minage, de dégâts critiques et d'expérience sont des **facteurs**
//! (base 1) ; les chances (critique, butin, vol de vie, perforation en %) sont des fractions
//! entre 0 et 1.
//!
//! # Les formules de combat
//!
//! - **Armure** : chaque point d'armure efficace retire `per_point` des dégâts, jusqu'à `max`
//!   (c'est la seule réduction, [`damage_after_armor`]).
//! - **Perforation** : elle diminue l'armure du défenseur avant ce calcul ([`effective_armor`]) :
//!   d'abord la part en % (l'armure est multipliée par `1 - part`), puis les points fixes sont
//!   retirés. Elle ne soigne jamais : l'armure efficace ne descend pas sous 0.
//! - **Critique** : un coup tiré sous la chance multiplie les dégâts ([`roll_hit`]).
//! - **Vol de vie** : une fraction des dégâts infligés rend de la vie ([`life_steal`]).
//! - **Butin** : la chance de butin ajoute une fraction du nombre d'objets lâchés, la partie
//!   entière à coup sûr et le reste au tirage ([`bonus_count`]).

use std::path::Path;

use serde::Deserialize;

/// Un attribut. Les numéros sont **stables** (ils sont sauvegardés dans les affixes des fiches
/// d'objets) : on n'en change jamais un, on en ajoute à la fin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attribute {
    Damage = 0,
    CritChance = 1,
    CritDamage = 2,
    LifeSteal = 3,
    ArmorPierce = 4,
    ArmorPiercePercent = 5,
    Armor = 6,
    MaxHealth = 7,
    Regen = 8,
    MoveSpeed = 9,
    MiningSpeed = 10,
    LootChance = 11,
    Reach = 12,
    XpGain = 13,
    /// Fortune : les minerais lâchent jusqu'à ce nombre de fois plus (voir `enchant_effects.rs`).
    Fortune = 14,
    /// Épines : les dégâts renvoyés à la créature qui frappe le joueur.
    Thorns = 15,
    /// Légèreté : la part des dégâts de chute évitée (0 à 1).
    FallResist = 16,
    /// Toucher de soie : au moins 1, un bloc lâche le bloc lui-même.
    SilkTouch = 17,
    /// Aspiration : au moins 1, le butin va directement dans l'inventaire.
    Magnet = 18,
}

/// Le nombre d'attributs.
pub const COUNT: usize = 19;

impl Attribute {
    pub const ALL: [Attribute; COUNT] = [
        Self::Damage,
        Self::CritChance,
        Self::CritDamage,
        Self::LifeSteal,
        Self::ArmorPierce,
        Self::ArmorPiercePercent,
        Self::Armor,
        Self::MaxHealth,
        Self::Regen,
        Self::MoveSpeed,
        Self::MiningSpeed,
        Self::LootChance,
        Self::Reach,
        Self::XpGain,
        Self::Fortune,
        Self::Thorns,
        Self::FallResist,
        Self::SilkTouch,
        Self::Magnet,
    ];

    /// Le numéro stable de l'attribut.
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// Le nom court, sans accent, des commandes d'essai et des données.
    pub fn key(self) -> &'static str {
        match self {
            Self::Damage => "damage",
            Self::CritChance => "crit_chance",
            Self::CritDamage => "crit_damage",
            Self::LifeSteal => "life_steal",
            Self::ArmorPierce => "armor_pierce",
            Self::ArmorPiercePercent => "armor_pierce_percent",
            Self::Armor => "armor",
            Self::MaxHealth => "max_health",
            Self::Regen => "regen",
            Self::MoveSpeed => "move_speed",
            Self::MiningSpeed => "mining_speed",
            Self::LootChance => "loot_chance",
            Self::Reach => "reach",
            Self::XpGain => "xp_gain",
            Self::Fortune => "fortune",
            Self::Thorns => "thorns",
            Self::FallResist => "fall_resist",
            Self::SilkTouch => "silk_touch",
            Self::Magnet => "magnet",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Damage => "Dégâts",
            Self::CritChance => "Chance critique",
            Self::CritDamage => "Dégâts critiques",
            Self::LifeSteal => "Vol de vie",
            Self::ArmorPierce => "Perforation d'armure",
            Self::ArmorPiercePercent => "Perforation d'armure (%)",
            Self::Armor => "Armure",
            Self::MaxHealth => "Vie max",
            Self::Regen => "Régénération",
            Self::MoveSpeed => "Vitesse de déplacement",
            Self::MiningSpeed => "Vitesse de minage",
            Self::LootChance => "Chance de butin",
            Self::Reach => "Portée",
            Self::XpGain => "Gain d'expérience",
            Self::Fortune => "Fortune",
            Self::Thorns => "Épines",
            Self::FallResist => "Légèreté",
            Self::SilkTouch => "Toucher de soie",
            Self::Magnet => "Aspiration",
        }
    }

    /// Les bornes de la valeur finale.
    pub fn bounds(self) -> (f32, f32) {
        match self {
            Self::Damage => (0.0, 1000.0),
            Self::CritChance | Self::LifeSteal | Self::ArmorPiercePercent => (0.0, 1.0),
            Self::CritDamage => (1.0, 10.0),
            Self::ArmorPierce | Self::Armor => (0.0, 1000.0),
            Self::MaxHealth => (1.0, 1000.0),
            Self::Regen => (0.0, 100.0),
            Self::MoveSpeed => (0.2, 4.0),
            Self::MiningSpeed => (0.1, 20.0),
            Self::LootChance => (0.0, 10.0),
            Self::Reach => (1.0, 12.0),
            Self::XpGain => (0.0, 10.0),
            Self::Fortune | Self::SilkTouch | Self::Magnet => (0.0, 20.0),
            Self::Thorns => (0.0, 100.0),
            Self::FallResist => (0.0, 0.9),
        }
    }

    /// La valeur, écrite pour l'affichage (« 25 % », « ×1,50 »).
    pub fn format(self, value: f32) -> String {
        let text = match self {
            Self::Damage | Self::ArmorPierce | Self::Armor | Self::MaxHealth => {
                format!("{value:.1}")
            }
            Self::CritChance | Self::LifeSteal | Self::ArmorPiercePercent | Self::LootChance => {
                format!("{:.0} %", value * 100.0)
            }
            Self::CritDamage | Self::MoveSpeed | Self::MiningSpeed | Self::XpGain => {
                format!("×{value:.2}")
            }
            Self::Regen => format!("{value:.2} / s"),
            Self::Reach => format!("{value:.1} blocs"),
            Self::Fortune | Self::Thorns => format!("{value:.1}"),
            Self::FallResist => format!("{:.0} %", value * 100.0),
            Self::SilkTouch | Self::Magnet => if value > 0.0 { "oui" } else { "non" }.to_string(),
        };
        text.replace('.', ",")
    }
}

/// Un bonus sur un attribut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modifier {
    pub attribute: Attribute,
    /// Ce qui s'ajoute à la base.
    pub flat: f32,
    /// La part qui multiplie le total (0,25 = +25 %).
    pub percent: f32,
}

impl Modifier {
    pub fn flat(attribute: Attribute, flat: f32) -> Self {
        Self {
            attribute,
            flat,
            percent: 0.0,
        }
    }

    pub fn percent(attribute: Attribute, percent: f32) -> Self {
        Self {
            attribute,
            flat: 0.0,
            percent,
        }
    }
}

/// D'où viennent des bonus : un nom lisible (« Épée de fer », « Force ») et ses modificateurs.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub label: String,
    pub modifiers: Vec<Modifier>,
}

/// Les valeurs de base du joueur (`attributes` dans `assets/core/player.ron`). Tout champ absent
/// prend sa valeur par défaut : un ancien fichier sans cette section reste valide.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default)]
pub struct BaseAttributes {
    /// Les dégâts d'un coup de poing (un outil en main les remplace s'il est meilleur).
    pub damage: f32,
    pub crit_chance: f32,
    /// Le facteur d'un coup critique.
    pub crit_damage: f32,
    pub life_steal: f32,
    pub armor_pierce: f32,
    pub armor_pierce_percent: f32,
    pub armor: f32,
    pub max_health: f32,
    /// Points de vie rendus par seconde, en plus de la faim et des potions.
    pub regen: f32,
    pub move_speed: f32,
    pub mining_speed: f32,
    pub loot_chance: f32,
    /// La portée du bras, en blocs.
    pub reach: f32,
    pub xp_gain: f32,
}

impl Default for BaseAttributes {
    fn default() -> Self {
        Self {
            damage: 4.0,
            crit_chance: 0.0,
            crit_damage: 1.5,
            life_steal: 0.0,
            armor_pierce: 0.0,
            armor_pierce_percent: 0.0,
            armor: 0.0,
            max_health: 20.0,
            regen: 0.0,
            move_speed: 1.0,
            mining_speed: 1.0,
            loot_chance: 0.0,
            reach: 6.0,
            xp_gain: 1.0,
        }
    }
}

impl BaseAttributes {
    pub fn get(&self, attribute: Attribute) -> f32 {
        match attribute {
            Attribute::Damage => self.damage,
            Attribute::CritChance => self.crit_chance,
            Attribute::CritDamage => self.crit_damage,
            Attribute::LifeSteal => self.life_steal,
            Attribute::ArmorPierce => self.armor_pierce,
            Attribute::ArmorPiercePercent => self.armor_pierce_percent,
            Attribute::Armor => self.armor,
            Attribute::MaxHealth => self.max_health,
            Attribute::Regen => self.regen,
            Attribute::MoveSpeed => self.move_speed,
            Attribute::MiningSpeed => self.mining_speed,
            Attribute::LootChance => self.loot_chance,
            Attribute::Reach => self.reach,
            Attribute::XpGain => self.xp_gain,
            // Les effets d'enchantement (phase 3) n'ont pas de valeur de base.
            Attribute::Fortune
            | Attribute::Thorns
            | Attribute::FallResist
            | Attribute::SilkTouch
            | Attribute::Magnet => 0.0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlayerFile {
    #[serde(default)]
    attributes: BaseAttributes,
}

/// Lit la section `attributes` de `player.ron` (les valeurs par défaut s'il manque, est absente
/// ou invalide).
pub fn load_base(assets: &Path) -> BaseAttributes {
    let path = assets.join("core").join("player.ron");
    std::fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| parse_base(&text))
        .unwrap_or_else(|err| {
            log::warn!("{} : {err:#} (attributs par défaut)", path.display());
            BaseAttributes::default()
        })
}

/// Lit les attributs de base dans le texte de `player.ron`.
pub fn parse_base(text: &str) -> anyhow::Result<BaseAttributes> {
    Ok(ron::from_str::<PlayerFile>(text)?.attributes)
}

/// Ce qu'une source apporte à un attribut (pour le détail au survol).
#[derive(Debug, Clone, PartialEq)]
pub struct Contribution {
    pub label: String,
    pub flat: f32,
    pub percent: f32,
}

/// Les valeurs calculées, et d'où elles viennent.
#[derive(Debug, Clone, PartialEq)]
pub struct Attributes {
    base: BaseAttributes,
    values: [f32; COUNT],
    sources: Vec<Source>,
}

impl Attributes {
    /// Le calcul complet depuis la base et les sources.
    pub fn compute(base: BaseAttributes, sources: Vec<Source>) -> Self {
        let mut values = [0.0; COUNT];
        for attribute in Attribute::ALL {
            let (flat, percent) = totals(&sources, attribute);
            let (low, high) = attribute.bounds();
            values[attribute as usize] =
                ((base.get(attribute) + flat) * (1.0 + percent)).clamp(low, high);
        }
        Self {
            base,
            values,
            sources,
        }
    }

    /// Sans aucune source : les valeurs de base.
    pub fn plain(base: BaseAttributes) -> Self {
        Self::compute(base, Vec::new())
    }

    pub fn value(&self, attribute: Attribute) -> f32 {
        self.values[attribute as usize]
    }

    pub fn base(&self, attribute: Attribute) -> f32 {
        self.base.get(attribute)
    }

    /// La valeur diffère-t-elle de la base ?
    pub fn is_modified(&self, attribute: Attribute) -> bool {
        (self.value(attribute) - self.base(attribute)).abs() > 1e-4
    }

    /// Ce que chaque source apporte à cet attribut (celles qui n'y touchent pas sont omises).
    pub fn contributions(&self, attribute: Attribute) -> Vec<Contribution> {
        self.sources
            .iter()
            .filter_map(|source| {
                let (flat, percent) = totals(std::slice::from_ref(source), attribute);
                (flat != 0.0 || percent != 0.0).then(|| Contribution {
                    label: source.label.clone(),
                    flat,
                    percent,
                })
            })
            .collect()
    }

    /// La perforation du joueur, pour [`effective_armor`].
    pub fn pierce(&self) -> Pierce {
        Pierce {
            flat: self.value(Attribute::ArmorPierce),
            percent: self.value(Attribute::ArmorPiercePercent),
        }
    }
}

fn totals(sources: &[Source], attribute: Attribute) -> (f32, f32) {
    sources
        .iter()
        .flat_map(|s| &s.modifiers)
        .filter(|m| m.attribute == attribute)
        .fold((0.0, 0.0), |(f, p), m| (f + m.flat, p + m.percent))
}

/// La perforation d'un attaquant : points d'armure ignorés, et part d'armure ignorée.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pierce {
    pub flat: f32,
    pub percent: f32,
}

impl Pierce {
    pub const NONE: Pierce = Pierce {
        flat: 0.0,
        percent: 0.0,
    };

    pub fn flat(flat: f32) -> Self {
        Self { flat, percent: 0.0 }
    }
}

/// L'armure qui reste face à cette perforation : la part d'abord, puis les points fixes.
pub fn effective_armor(armor: f32, pierce: Pierce) -> f32 {
    (armor * (1.0 - pierce.percent.clamp(0.0, 1.0)) - pierce.flat.max(0.0)).max(0.0)
}

/// La part des dégâts que l'armure retire : `per_point` par point efficace, au plus `max`.
pub fn armor_reduction(effective_armor: f32, per_point: f32, max: f32) -> f32 {
    (effective_armor * per_point).clamp(0.0, max)
}

/// Les dégâts qui passent l'armure (la perforation la contourne en partie).
pub fn damage_after_armor(
    amount: f32,
    armor: f32,
    pierce: Pierce,
    per_point: f32,
    max: f32,
) -> f32 {
    amount * (1.0 - armor_reduction(effective_armor(armor, pierce), per_point, max))
}

/// Un coup : les dégâts, critiques si le tirage `roll` (0 à 1) tombe sous la chance. Renvoie les
/// dégâts et s'il y a eu critique.
pub fn roll_hit(damage: f32, crit_chance: f32, crit_damage: f32, roll: f32) -> (f32, bool) {
    if roll < crit_chance {
        (damage * crit_damage, true)
    } else {
        (damage, false)
    }
}

/// La vie rendue par des dégâts infligés.
pub fn life_steal(dealt: f32, fraction: f32) -> f32 {
    dealt.max(0.0) * fraction.clamp(0.0, 1.0)
}

/// Le nombre d'objets lâchés avec la chance de butin : `count` plus `count × loot` objets, dont
/// la partie fractionnaire se joue au tirage `roll` (0 à 1).
pub fn bonus_count(count: u32, loot: f32, roll: f32) -> u32 {
    let extra = count as f32 * loot.max(0.0);
    let whole = extra.floor();
    count + whole as u32 + u32::from(roll < extra - whole)
}

/// Les points d'expérience avec le gain d'expérience (arrondi).
pub fn scaled_xp(points: u32, gain: f32) -> u32 {
    (points as f32 * gain.max(0.0)).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(label: &str, modifiers: Vec<Modifier>) -> Source {
        Source {
            label: label.into(),
            modifiers,
        }
    }

    #[test]
    fn ids_are_stable_and_complete() {
        for (i, attribute) in Attribute::ALL.into_iter().enumerate() {
            assert_eq!(attribute as usize, i);
            assert_eq!(Attribute::from_id(attribute.id()), Some(attribute));
        }
        assert_eq!(Attribute::from_id(200), None);
        assert_eq!(Attribute::ALL.len(), COUNT);
    }

    #[test]
    fn keys_are_unique_and_named() {
        let mut keys: Vec<&str> = Attribute::ALL.iter().map(|a| a.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), COUNT);
        assert!(Attribute::ALL.iter().all(|a| !a.name().is_empty()));
    }

    #[test]
    fn without_sources_the_values_are_the_base() {
        let attrs = Attributes::plain(BaseAttributes::default());
        assert_eq!(attrs.value(Attribute::Damage), 4.0);
        assert_eq!(attrs.value(Attribute::MaxHealth), 20.0);
        assert_eq!(attrs.value(Attribute::MoveSpeed), 1.0);
        assert_eq!(attrs.value(Attribute::Reach), 6.0);
        for attribute in Attribute::ALL {
            assert!(!attrs.is_modified(attribute));
            assert!(attrs.contributions(attribute).is_empty());
        }
    }

    #[test]
    fn flat_bonuses_add_then_percentages_multiply() {
        let attrs = Attributes::compute(
            BaseAttributes::default(),
            vec![
                source(
                    "Épée",
                    vec![
                        Modifier::flat(Attribute::Damage, 4.0),
                        Modifier::percent(Attribute::MiningSpeed, 0.3),
                    ],
                ),
                source(
                    "Force",
                    vec![
                        Modifier::flat(Attribute::Damage, 2.0),
                        Modifier::percent(Attribute::Damage, 0.5),
                        Modifier::percent(Attribute::MiningSpeed, 0.3),
                    ],
                ),
            ],
        );
        // (4 de base + 4 + 2) × (1 + 0,5)
        assert_eq!(attrs.value(Attribute::Damage), 15.0);
        // 1 × (1 + 0,3 + 0,3)
        assert!((attrs.value(Attribute::MiningSpeed) - 1.6).abs() < 1e-6);
        assert!(attrs.is_modified(Attribute::Damage));
        assert!(!attrs.is_modified(Attribute::Armor));
        let detail = attrs.contributions(Attribute::Damage);
        assert_eq!(detail.len(), 2);
        assert_eq!(detail[0].label, "Épée");
        assert_eq!(detail[1].percent, 0.5);
    }

    #[test]
    fn values_stay_inside_their_bounds() {
        let attrs = Attributes::compute(
            BaseAttributes::default(),
            vec![source(
                "Excès",
                vec![
                    Modifier::flat(Attribute::CritChance, 3.0),
                    Modifier::flat(Attribute::MoveSpeed, -5.0),
                    Modifier::flat(Attribute::MaxHealth, -100.0),
                ],
            )],
        );
        assert_eq!(attrs.value(Attribute::CritChance), 1.0);
        assert_eq!(attrs.value(Attribute::MoveSpeed), 0.2);
        assert_eq!(attrs.value(Attribute::MaxHealth), 1.0);
    }

    #[test]
    fn armor_is_the_only_reduction_and_pierce_gets_around_it() {
        let (per_point, max) = (0.04, 0.8);
        let hit = |armor, pierce| damage_after_armor(10.0, armor, pierce, per_point, max);
        // Sans armure : tous les dégâts.
        assert_eq!(hit(0.0, Pierce::NONE), 10.0);
        // 10 points : 40 % retirés.
        assert!((hit(10.0, Pierce::NONE) - 6.0).abs() < 1e-5);
        // La perforation fixe retire des points d'armure : 10 - 5 = 5 points, 20 %.
        assert!((hit(10.0, Pierce::flat(5.0)) - 8.0).abs() < 1e-5);
        // La perforation en % : 50 % de 10 = 5 points, puis 1 point fixe : 4 points, 16 %.
        let both = Pierce {
            flat: 1.0,
            percent: 0.5,
        };
        assert!((effective_armor(10.0, both) - 4.0).abs() < 1e-5);
        assert!((hit(10.0, both) - 8.4).abs() < 1e-5);
        // Trop de perforation ne rend rien de plus qu'une absence d'armure.
        assert_eq!(hit(10.0, Pierce::flat(99.0)), 10.0);
        assert_eq!(
            effective_armor(
                10.0,
                Pierce {
                    flat: 0.0,
                    percent: 2.0
                }
            ),
            0.0
        );
        // Un plafond : beaucoup d'armure n'arrête jamais plus de 80 %.
        assert!((hit(500.0, Pierce::NONE) - 2.0).abs() < 1e-5);
        // La perforation fait passer plus de dégâts qu'une armure seule.
        assert!(hit(20.0, Pierce::flat(8.0)) > hit(20.0, Pierce::NONE));
    }

    #[test]
    fn critical_hits_life_steal_loot_and_experience() {
        assert_eq!(roll_hit(10.0, 0.25, 1.5, 0.2), (15.0, true));
        assert_eq!(roll_hit(10.0, 0.25, 1.5, 0.3), (10.0, false));
        assert_eq!(roll_hit(10.0, 0.0, 1.5, 0.0), (10.0, false), "0 % : jamais");
        assert_eq!(life_steal(10.0, 0.1), 1.0);
        assert_eq!(life_steal(-4.0, 0.5), 0.0);
        // 3 objets, +50 % : 1 de plus à coup sûr, un deuxième une fois sur deux.
        assert_eq!(bonus_count(3, 0.5, 0.9), 4);
        assert_eq!(bonus_count(3, 0.5, 0.1), 5);
        assert_eq!(
            bonus_count(3, 0.0, 0.0),
            3,
            "sans chance de butin : inchangé"
        );
        assert_eq!(bonus_count(2, 1.0, 0.99), 4);
        assert_eq!(scaled_xp(7, 1.0), 7);
        assert_eq!(scaled_xp(7, 1.5), 11);
        assert_eq!(scaled_xp(7, 0.0), 0);
    }

    #[test]
    fn the_base_comes_from_player_ron_and_old_files_still_load() {
        let text = "(arm: (skin: (0.8, 0.6, 0.5)), attributes: (damage: 5.0, reach: 7.5))";
        let base = parse_base(text).unwrap();
        assert_eq!(base.damage, 5.0);
        assert_eq!(base.reach, 7.5);
        assert_eq!(base.max_health, 20.0, "champ absent : valeur par défaut");
        // Un ancien `player.ron` sans la section `attributes`.
        let old = "(survival: (invulnerable: 0.5), hunger: (max_food: 20.0))";
        assert_eq!(parse_base(old).unwrap(), BaseAttributes::default());
        // Le fichier du jeu se lit, et ses valeurs sont celles du jeu d'avant les attributs.
        let shipped = include_str!("../../../assets/core/player.ron");
        let base = parse_base(shipped).unwrap();
        assert_eq!(base.damage, 4.0);
        assert_eq!(base.max_health, 20.0);
        assert_eq!(base.reach, 6.0);
    }

    #[test]
    fn values_are_written_for_display() {
        assert_eq!(Attribute::CritChance.format(0.25), "25 %");
        assert_eq!(Attribute::MoveSpeed.format(1.2), "×1,20");
        assert_eq!(Attribute::Reach.format(6.0), "6,0 blocs");
        assert_eq!(Attribute::Regen.format(0.5), "0,50 / s");
    }
}
