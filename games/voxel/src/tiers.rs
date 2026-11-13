//! Les paliers du monde : ce qui les débloque, celui qu'on a choisi, et ce qu'ils changent.
//!
//! Cinq paliers (`tiers.ron`, d'« Havre » à « Apogée ») : plus le palier est haut, plus les
//! créatures hostiles ont de vie et font de dégâts, plus les élites sont nombreuses, et plus les
//! raretés hautes sortent au tirage. On choisit son palier dans un écran (touche « Paliers du
//! monde » des options, voir `tiers_ui.rs`), parmi ceux qu'on a débloqués.
//!
//! # Le déblocage
//!
//! Chaque palier liste des **critères** (`Kills`, `Bosses`, `Rarity`, `Depth`) qu'il faut tous
//! remplir. La [`Progress`] compte ce qui s'est passé dans le monde : hostiles tuées, boss vaincus,
//! plus haute rareté obtenue, plus grande profondeur. Un palier se débloque dans l'ordre (le
//! deuxième avant le troisième) et **reste** débloqué, même si les données changent ensuite.
//! Changer de palier n'est jamais définitif : on peut redescendre.
//!
//! # La sauvegarde
//!
//! Tout tient dans `AdventureSave` (voir `engine_voxel::save`), un champ de `world.ron`. Un monde
//! plus ancien, sans ce champ, se charge au palier 0 sans progression. Un numéro de palier
//! sauvegardé qui n'existe plus (un mod retiré) est ramené au plus haut palier connu.
//!
//! # Ce que le palier change
//!
//! [`TierEffects`] est la ressource de l'ECS que la simulation lit (`mobs.rs`, `spawning.rs`) :
//! multiplicateurs de vie et de dégâts, chance d'élite, rythme des envahisseurs. Le jeu la tient à
//! jour à chaque pas depuis le palier choisi. Sans cette ressource (les tests de la simulation), les
//! créatures sont inchangées.

use engine_voxel::AdventureSave;

use crate::adventure_data::{Criterion, TierDef};

/// Ce que la simulation lit du palier du monde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierEffects {
    /// Le numéro du palier (0 : Havre).
    pub tier: u8,
    /// La vie et les dégâts des hostiles, multipliés.
    pub health: f32,
    pub damage: f32,
    /// La part des hostiles qui naissent élites.
    pub elite_chance: f32,
    /// Le temps entre deux envahisseurs, en secondes (min, max) ; (0, 0) : aucun.
    pub invader: (f32, f32),
}

impl Default for TierEffects {
    fn default() -> Self {
        Self {
            tier: 0,
            health: 1.0,
            damage: 1.0,
            elite_chance: 0.0,
            invader: (0.0, 0.0),
        }
    }
}

impl TierEffects {
    pub fn of(tier: u8, def: &TierDef) -> Self {
        Self {
            tier,
            health: def.health,
            damage: def.damage,
            elite_chance: def.elite_chance,
            invader: (def.invader.0 as f32, def.invader.1 as f32),
        }
    }
}

/// Ce qui s'est passé dans le monde, et le palier choisi.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Le palier choisi.
    pub tier: u8,
    /// Le palier le plus haut débloqué.
    pub unlocked: u8,
    pub kills: u32,
    pub bosses: u32,
    /// La plus haute rareté d'objet obtenue.
    pub best_rarity: Option<u8>,
    /// La plus grande profondeur atteinte, en blocs sous le niveau de la mer.
    pub depth: i32,
}

impl Progress {
    /// Ce critère est-il rempli ?
    pub fn meets(&self, criterion: Criterion) -> bool {
        match criterion {
            Criterion::Kills(n) => self.kills >= n,
            Criterion::Bosses(n) => self.bosses >= n,
            Criterion::Rarity(r) => self.best_rarity.is_some_and(|best| best >= r),
            Criterion::Depth(d) => self.depth >= d,
        }
    }

    /// Les critères d'un palier qui ne sont pas encore remplis.
    pub fn missing(&self, tier: &TierDef) -> Vec<Criterion> {
        tier.unlock
            .iter()
            .copied()
            .filter(|c| !self.meets(*c))
            .collect()
    }

    /// Le palier le plus haut qu'on peut débloquer, dans l'ordre : on s'arrête au premier dont un
    /// critère manque.
    pub fn reachable(&self, tiers: &[TierDef]) -> u8 {
        let mut reached = 0;
        for (i, tier) in tiers.iter().enumerate().skip(1) {
            if self.missing(tier).is_empty() {
                reached = i;
            } else {
                break;
            }
        }
        reached as u8
    }

    /// Débloque ce que la progression permet ; renvoie le nouveau palier le plus haut s'il y en a un.
    pub fn refresh_unlocked(&mut self, tiers: &[TierDef]) -> Option<u8> {
        let reached = self.reachable(tiers);
        (reached > self.unlocked).then(|| {
            self.unlocked = reached;
            reached
        })
    }

    /// Ce palier est-il choisissable ?
    pub fn is_unlocked(&self, tier: u8) -> bool {
        tier <= self.unlocked
    }

    /// Choisit un palier. `false` s'il est verrouillé ou n'existe pas.
    pub fn select(&mut self, tier: u8, tiers: &[TierDef]) -> bool {
        if usize::from(tier) >= tiers.len() || !self.is_unlocked(tier) {
            return false;
        }
        self.tier = tier;
        true
    }

    /// Note une hostile tuée (un boss en plus, s'il en est un).
    pub fn note_kill(&mut self, boss: bool) {
        self.kills = self.kills.saturating_add(1);
        if boss {
            self.bosses = self.bosses.saturating_add(1);
        }
    }

    /// Note une rareté obtenue.
    pub fn note_rarity(&mut self, rarity: u8) {
        if self.best_rarity.is_none_or(|best| rarity > best) {
            self.best_rarity = Some(rarity);
        }
    }

    /// Note une profondeur atteinte (la hauteur y du joueur et le niveau de la mer).
    pub fn note_depth(&mut self, y: f32, sea_level: i32) {
        let depth = sea_level - y.floor() as i32;
        self.depth = self.depth.max(depth);
    }

    pub fn to_save(self) -> AdventureSave {
        AdventureSave {
            tier: self.tier,
            unlocked: self.unlocked,
            kills: self.kills,
            bosses: self.bosses,
            best_rarity: self.best_rarity.map_or(0, |r| r.saturating_add(1)),
            depth: self.depth,
        }
    }

    /// Relit la sauvegarde, en ramenant les numéros trop hauts au dernier palier connu.
    pub fn from_save(save: &AdventureSave, tiers: &[TierDef]) -> Self {
        let last = tiers.len().saturating_sub(1) as u8;
        let unlocked = save.unlocked.min(last);
        Self {
            tier: save.tier.min(unlocked),
            unlocked,
            kills: save.kills,
            bosses: save.bosses,
            best_rarity: save.best_rarity.checked_sub(1),
            depth: save.depth,
        }
    }
}

/// Un critère écrit pour l'écran : « 25 hostiles tuées (12) ».
pub fn describe(
    criterion: Criterion,
    progress: &Progress,
    rarities: &[crate::adventure_data::Rarity],
) -> String {
    match criterion {
        Criterion::Kills(n) => format!("{n} hostiles tuées ({})", progress.kills.min(n)),
        Criterion::Bosses(n) => {
            format!("{n} boss de crypte vaincu(s) ({})", progress.bosses.min(n))
        }
        Criterion::Rarity(r) => format!(
            "Obtenir un objet {} ou mieux",
            rarities
                .get(usize::from(r))
                .map_or("?".to_string(), |x| x.name.to_lowercase())
        ),
        Criterion::Depth(d) => format!(
            "Descendre à {d} blocs sous la mer (record : {})",
            progress.depth.max(0).min(d)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adventure_data::AdventureData;

    fn tiers() -> Vec<TierDef> {
        AdventureData::builtin().tiers
    }

    #[test]
    fn a_new_world_is_at_the_first_tier_with_nothing_unlocked() {
        let tiers = tiers();
        let mut progress = Progress::default();
        assert_eq!(progress.reachable(&tiers), 0);
        assert_eq!(progress.refresh_unlocked(&tiers), None);
        assert!(progress.is_unlocked(0) && !progress.is_unlocked(1));
        assert!(!progress.select(1, &tiers), "verrouillé");
        assert!(progress.select(0, &tiers));
    }

    #[test]
    fn criteria_unlock_in_order_and_all_must_be_met() {
        let tiers = tiers();
        let mut progress = Progress::default();
        for _ in 0..25 {
            progress.note_kill(false);
        }
        assert_eq!(
            progress.refresh_unlocked(&tiers),
            Some(1),
            "25 hostiles : Frontière"
        );
        // Ascension demande 100 hostiles ET 20 blocs de profondeur.
        for _ in 0..80 {
            progress.note_kill(false);
        }
        assert_eq!(
            progress.refresh_unlocked(&tiers),
            None,
            "il manque la profondeur"
        );
        assert_eq!(progress.missing(&tiers[2]), vec![Criterion::Depth(20)]);
        progress.note_depth(30.0, 62);
        assert_eq!(progress.depth, 32);
        assert_eq!(progress.refresh_unlocked(&tiers), Some(2));
        // Le Sommet : un boss de plus. Les boss comptent aussi comme des hostiles.
        for _ in 0..150 {
            progress.note_kill(false);
        }
        assert_eq!(
            progress.refresh_unlocked(&tiers),
            None,
            "pas de boss encore"
        );
        progress.note_kill(true);
        assert_eq!(progress.bosses, 1);
        assert_eq!(progress.refresh_unlocked(&tiers), Some(3));
        // L'Apogée : un objet épique et 40 blocs de profondeur, 500 hostiles.
        for _ in 0..300 {
            progress.note_kill(false);
        }
        progress.note_depth(10.0, 62);
        assert_eq!(
            progress.refresh_unlocked(&tiers),
            None,
            "pas d'objet épique"
        );
        progress.note_rarity(2);
        assert_eq!(progress.best_rarity, Some(2));
        progress.note_rarity(3);
        progress.note_rarity(1);
        assert_eq!(progress.best_rarity, Some(3), "la plus haute seulement");
        assert_eq!(progress.refresh_unlocked(&tiers), Some(4));
        assert!(progress.select(4, &tiers));
        assert!(progress.select(0, &tiers), "on peut redescendre");
        assert!(!progress.select(9, &tiers));
    }

    #[test]
    fn a_tier_stays_unlocked_when_the_requirements_change() {
        let mut tiers = tiers();
        let mut progress = Progress {
            kills: 30,
            ..Progress::default()
        };
        progress.refresh_unlocked(&tiers);
        assert_eq!(progress.unlocked, 1);
        // Les données exigent désormais bien plus : le palier reste acquis.
        tiers[1].unlock = vec![Criterion::Kills(10_000)];
        assert_eq!(progress.refresh_unlocked(&tiers), None);
        assert_eq!(progress.unlocked, 1);
        assert!(progress.select(1, &tiers));
    }

    #[test]
    fn higher_tiers_strengthen_the_creatures() {
        let tiers = tiers();
        let effects: Vec<TierEffects> = tiers
            .iter()
            .enumerate()
            .map(|(i, t)| TierEffects::of(i as u8, t))
            .collect();
        assert_eq!(effects[0].health, 1.0);
        assert_eq!(effects[0].damage, 1.0);
        for pair in effects.windows(2) {
            assert!(pair[1].health > pair[0].health);
            assert!(pair[1].damage > pair[0].damage);
            assert!(pair[1].elite_chance > pair[0].elite_chance);
        }
        // Les envahisseurs commencent au palier 2.
        assert_eq!(effects[1].invader, (0.0, 0.0));
        assert!(effects[2].invader.0 > 0.0);
        assert!(
            effects[4].invader.1 < effects[2].invader.1,
            "de plus en plus souvent"
        );
        assert_eq!(TierEffects::default().health, 1.0);
    }

    #[test]
    fn progress_survives_a_save_and_an_old_world_loads_at_tier_zero() {
        let tiers = tiers();
        let progress = Progress {
            tier: 2,
            unlocked: 3,
            kills: 141,
            bosses: 1,
            best_rarity: Some(0),
            depth: 33,
        };
        let back = Progress::from_save(&progress.to_save(), &tiers);
        assert_eq!(
            back, progress,
            "la rareté 0 (commune) se distingue de « aucune »"
        );
        assert_eq!(Progress::default().to_save().best_rarity, 0);
        assert_eq!(
            Progress::from_save(&Progress::default().to_save(), &tiers).best_rarity,
            None
        );
        // Un ancien world.ron : pas de champ « adventure ».
        let old: engine_voxel::WorldMeta = ron::from_str(
            "(format_version: 4, seed: 3, time_of_day: 0.5, player: None, mods: [\"core\"])",
        )
        .unwrap();
        let loaded = Progress::from_save(&old.adventure, &tiers);
        assert_eq!(loaded, Progress::default());
        // Un numéro de palier trop haut (mod retiré) est ramené au dernier connu.
        let saved = AdventureSave {
            tier: 9,
            unlocked: 9,
            ..AdventureSave::default()
        };
        let clamped = Progress::from_save(&saved, &tiers);
        assert_eq!((clamped.tier, clamped.unlocked), (4, 4));
        // Un palier choisi ne dépasse jamais le palier débloqué.
        let cheat = AdventureSave {
            tier: 3,
            unlocked: 1,
            ..AdventureSave::default()
        };
        assert_eq!(Progress::from_save(&cheat, &tiers).tier, 1);
    }

    #[test]
    fn the_tier_is_written_in_world_ron_and_read_back() {
        let dir = std::env::temp_dir().join(format!("voxel-tiers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = engine_voxel::SaveStore::open(dir.clone()).unwrap();
        let mut meta: engine_voxel::WorldMeta =
            ron::from_str("(format_version: 4, seed: 3, time_of_day: 0.5, player: None, mods: [])")
                .unwrap();
        let progress = Progress {
            tier: 1,
            unlocked: 2,
            kills: 7,
            ..Progress::default()
        };
        meta.adventure = progress.to_save();
        store.save_meta(&meta).unwrap();
        let text = std::fs::read_to_string(dir.join("world.ron")).unwrap();
        assert!(text.contains("adventure"), "{text}");
        let loaded = store.load_meta().unwrap().unwrap();
        assert_eq!(Progress::from_save(&loaded.adventure, &tiers()), progress);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn criteria_are_described_for_the_screen() {
        let data = AdventureData::builtin();
        let progress = Progress {
            kills: 12,
            ..Progress::default()
        };
        assert_eq!(
            describe(Criterion::Kills(25), &progress, &data.rarities),
            "25 hostiles tuées (12)"
        );
        assert!(describe(Criterion::Rarity(3), &progress, &data.rarities).contains("épique"));
    }
}
