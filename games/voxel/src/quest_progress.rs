//! L'avancement du joueur dans le livre de quêtes : états, compteurs, récompenses, sauvegarde.
//!
//! Tout est pur : ce module ne connaît ni l'ECS ni l'affichage. Le jeu (`quest_play.rs`) lui donne
//! un **instantané** du joueur ([`Snapshot`] : objets portés, dimension, biome, hauteur...) et la
//! liste des **événements** survenus ([`Event`] : blocs cassés ou posés, créatures tuées, objets
//! fabriqués), et reçoit la liste des quêtes qui viennent de se terminer.
//!
//! # États d'une quête
//!
//! [`Status`] : `Locked` (dépendances non finies), `Available` (rien de fait), `Started` (au moins une
//! tâche avance), `Done` (tâches finies, récompenses à récupérer), `Claimed` (tout est récupéré ; une
//! quête sans récompense passe directement ici).
//!
//! # Les compteurs
//!
//! Chaque tâche a un compteur, et la tâche est finie quand il atteint [`Task::target`]. Il ne
//! redescend jamais : une tâche d'objet finie le reste si on jette l'objet. En mode `Linear`, rien
//! n'avance tant que les dépendances ne sont pas finies ; en mode `Flexible`, les compteurs avancent
//! toujours, mais la quête ne se termine que quand ses dépendances le sont.
//!
//! # La sauvegarde
//!
//! `quests.ron` à la racine du monde. Les identifiants inconnus (un mod retiré) sont **conservés**
//! tels quels pour retrouver la progression si le mod revient. Un monde sans ce fichier part de
//! zéro : les tâches d'objet se valident au premier passage.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context as _, Result};
use engine_voxel::ItemId;
use serde::{Deserialize, Serialize};

use crate::quests::{ItemSpec, Progression, Quests, Requires, Reward, Task};

/// Le nom du fichier, à la racine du monde.
pub const FILE: &str = "quests.ron";
/// La version du format du fichier.
const VERSION: u32 = 1;
/// Le nombre de quêtes qu'on peut épingler.
pub const MAX_PINNED: usize = 3;

/// Ce que le joueur est et porte à cet instant.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// Les objets portés (barre, réserve, armure, main) : pas le contenu des sacs.
    pub items: HashMap<ItemId, u32>,
    pub dimension: String,
    pub biome: String,
    /// Les structures où le joueur se trouve.
    pub structures: Vec<String>,
    pub y: f32,
    pub level: u32,
    /// Le palier du monde le plus haut débloqué.
    pub tier: u8,
}

impl Snapshot {
    /// Combien d'objets correspondent à `spec`.
    pub fn count(&self, spec: &ItemSpec) -> u32 {
        self.items
            .iter()
            .filter(|(item, _)| spec.matches(**item))
            .map(|(_, n)| *n)
            .sum()
    }
}

/// Ce qui s'est passé dans le monde.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Un bloc cassé (identifiant complet, `core:stone`).
    Broken(String),
    Placed(String),
    /// Une créature tuée près du joueur.
    Killed {
        kind: String,
        hostile: bool,
    },
    /// Un objet fabriqué (grille d'artisanat ou livre de recettes).
    Crafted(ItemId, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Locked,
    Available,
    Started,
    Done,
    Claimed,
}

/// Ce qu'une récupération donne au joueur (les tirages et choix sont déjà faits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    Item(ItemId, u32),
    Xp(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    Unknown,
    NotDone,
    AlreadyClaimed,
    /// Une récompense `Choice` (son numéro) attend le choix du joueur.
    NeedsChoice(usize),
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "quête inconnue"),
            Self::NotDone => write!(f, "quête pas terminée"),
            Self::AlreadyClaimed => write!(f, "récompenses déjà récupérées"),
            Self::NeedsChoice(i) => write!(f, "la récompense {i} demande un choix (quest choose)"),
        }
    }
}

/// Ce que le monde retient d'une quête.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Le compteur de chaque tâche.
    #[serde(default)]
    pub counters: Vec<u32>,
    /// Les tâches sont finies (et les dépendances aussi).
    #[serde(default)]
    pub done: bool,
    /// Les récompenses sont récupérées.
    #[serde(default)]
    pub claimed: bool,
    /// Le choix fait pour chaque récompense `Choice` : numéro de récompense → numéro d'option.
    #[serde(default)]
    pub choices: BTreeMap<usize, usize>,
}

/// La progression d'un monde.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    records: BTreeMap<String, Record>,
    /// Les quêtes épinglées à l'écran (`MAX_PINNED` au plus).
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Le dernier chapitre ouvert dans le livre.
    #[serde(default)]
    pub last_chapter: Option<String>,
    #[serde(skip)]
    dirty: bool,
}

fn options() -> ron::Options {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
}

impl Progress {
    // ----- États -------------------------------------------------------------------------

    fn is_done(&self, id: &str) -> bool {
        self.records.get(id).is_some_and(|r| r.done)
    }

    /// Les dépendances de la quête sont-elles finies ?
    pub fn deps_met(&self, quests: &Quests, index: usize) -> bool {
        let quest = &quests.quests[index];
        if quest.deps.is_empty() {
            return true;
        }
        let done = |d: &usize| self.is_done(&quests.quests[*d].id);
        match quest.requires {
            Requires::All => quest.deps.iter().all(done),
            Requires::One => quest.deps.iter().any(done),
        }
    }

    #[cfg(test)]
    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }

    /// Le compteur d'une tâche.
    pub fn counter(&self, id: &str, task: usize) -> u32 {
        self.records
            .get(id)
            .and_then(|r| r.counters.get(task))
            .copied()
            .unwrap_or(0)
    }

    /// L'option choisie pour la récompense `Choice` numéro `reward` d'une quête, s'il y en a une.
    pub fn choice(&self, id: &str, reward: usize) -> Option<usize> {
        self.records.get(id)?.choices.get(&reward).copied()
    }

    pub fn status(&self, quests: &Quests, index: usize) -> Status {
        let quest = &quests.quests[index];
        if let Some(record) = self.records.get(&quest.id) {
            if record.done {
                return if record.claimed || quest.rewards.is_empty() {
                    Status::Claimed
                } else {
                    Status::Done
                };
            }
            if !self.deps_met(quests, index) {
                return Status::Locked;
            }
            if record.counters.iter().any(|c| *c > 0) {
                return Status::Started;
            }
            return Status::Available;
        }
        if self.deps_met(quests, index) {
            Status::Available
        } else {
            Status::Locked
        }
    }

    /// La quête est-elle montrée sur le plan ? Une quête `hidden` attend ses dépendances, une
    /// `secret` attend d'être finie.
    pub fn visible(&self, quests: &Quests, index: usize) -> bool {
        let quest = &quests.quests[index];
        if self.is_done(&quest.id) {
            return true;
        }
        if quest.secret {
            return false;
        }
        !quest.hidden || self.deps_met(quests, index)
    }

    /// (quêtes finies, quêtes comptées) d'un chapitre : les quêtes `optional` ne comptent pas.
    pub fn chapter_progress(&self, quests: &Quests, chapter: usize) -> (usize, usize) {
        let counted = quests.chapters[chapter]
            .quests
            .iter()
            .filter(|&&q| !quests.quests[q].optional);
        let total = counted.clone().count();
        let done = counted
            .filter(|&&q| self.is_done(&quests.quests[q].id))
            .count();
        (done, total)
    }

    /// Les quêtes finies dont les récompenses attendent.
    pub fn pending(&self, quests: &Quests) -> Vec<usize> {
        (0..quests.quests.len())
            .filter(|&i| self.status(quests, i) == Status::Done)
            .collect()
    }

    /// Les quêtes de ce chapitre dont les récompenses attendent.
    pub fn pending_in(&self, quests: &Quests, chapter: usize) -> usize {
        quests.chapters[chapter]
            .quests
            .iter()
            .filter(|&&q| self.status(quests, q) == Status::Done)
            .count()
    }

    // ----- Mise à jour -------------------------------------------------------------------

    fn record_mut(&mut self, id: &str, tasks: usize) -> &mut Record {
        self.dirty = true;
        let record = self.records.entry(id.to_string()).or_default();
        if record.counters.len() != tasks {
            record.counters.resize(tasks, 0);
        }
        record
    }

    /// Les tâches de cette quête avancent-elles en ce moment ?
    fn advancing(&self, quests: &Quests, index: usize) -> bool {
        let quest = &quests.quests[index];
        !self.is_done(&quest.id)
            && (quest.progression == Progression::Flexible || self.deps_met(quests, index))
    }

    /// Fait avancer les compteurs avec ce que le joueur est et ce qui s'est passé, termine les
    /// quêtes dont les tâches sont finies, et renvoie leurs numéros (dans l'ordre où elles se
    /// terminent : une quête finie peut en débloquer une autre dans le même passage).
    pub fn update(&mut self, quests: &Quests, snapshot: &Snapshot, events: &[Event]) -> Vec<usize> {
        let mut finished = Vec::new();
        let mut events = events;
        loop {
            let mut changed = false;
            for index in 0..quests.quests.len() {
                if !self.advancing(quests, index) {
                    continue;
                }
                let quest = &quests.quests[index];
                let before = self.records.get(&quest.id).map(|r| r.counters.clone());
                let counters = self.next_counters(quest, before.as_deref(), snapshot, events);
                let complete = quest
                    .tasks
                    .iter()
                    .zip(&counters)
                    .all(|(t, c)| *c >= t.target());
                let moved = before.as_deref() != Some(counters.as_slice());
                if moved && counters.iter().any(|c| *c > 0) {
                    self.record_mut(&quest.id, quest.tasks.len()).counters = counters;
                }
                if complete && self.deps_met(quests, index) {
                    self.finish(quests, index);
                    finished.push(index);
                    changed = true;
                }
            }
            // Les événements ne comptent qu'une fois : le deuxième tour ne fait que propager.
            events = &[];
            if !changed {
                return finished;
            }
        }
    }

    fn finish(&mut self, quests: &Quests, index: usize) {
        let quest = &quests.quests[index];
        let record = self.record_mut(&quest.id, quest.tasks.len());
        record.done = true;
        record.claimed = quest.rewards.is_empty();
    }

    /// Les compteurs de la quête après ce passage.
    fn next_counters(
        &self,
        quest: &crate::quests::Quest,
        before: Option<&[u32]>,
        snapshot: &Snapshot,
        events: &[Event],
    ) -> Vec<u32> {
        let mut counters: Vec<u32> = (0..quest.tasks.len())
            .map(|i| before.and_then(|b| b.get(i)).copied().unwrap_or(0))
            .collect();
        for (task, counter) in quest.tasks.iter().zip(counters.iter_mut()) {
            let target = task.target();
            let seen = match task {
                // On les remet à la main (`hand_in`).
                Task::Item { consume: true, .. } | Task::Check => 0,
                Task::Item {
                    spec,
                    crafted: true,
                    ..
                } => events
                    .iter()
                    .map(|e| match e {
                        Event::Crafted(item, n) if spec.matches(*item) => *n,
                        _ => 0,
                    })
                    .sum::<u32>()
                    .saturating_add(*counter),
                Task::Item { spec, .. } => snapshot.count(spec),
                Task::Break { block, .. } => counter.saturating_add(
                    events
                        .iter()
                        .filter(|e| matches!(e, Event::Broken(b) if b == block))
                        .count() as u32,
                ),
                Task::Place { block, .. } => counter.saturating_add(
                    events
                        .iter()
                        .filter(|e| matches!(e, Event::Placed(b) if b == block))
                        .count() as u32,
                ),
                Task::Kill { creature, .. } => counter.saturating_add(
                    events
                        .iter()
                        .filter(|e| match (e, creature) {
                            (Event::Killed { kind, .. }, Some(wanted)) => kind == wanted,
                            (Event::Killed { hostile, .. }, None) => *hostile,
                            _ => false,
                        })
                        .count() as u32,
                ),
                Task::Dimension(key) => u32::from(snapshot.dimension == *key),
                Task::Biome(key) => u32::from(snapshot.biome == *key),
                Task::Structure(key) => u32::from(snapshot.structures.contains(key)),
                Task::Depth(y) => u32::from(snapshot.y < *y as f32),
                Task::Level(n) => u32::from(snapshot.level >= *n),
                Task::Tier(n) => u32::from(snapshot.tier >= *n),
            };
            *counter = (*counter).max(seen.min(target));
        }
        counters
    }

    /// Coche la case d'une tâche `Check`. `false` si ce n'en est pas une, ou si la quête
    /// n'avance pas (verrouillée en mode `Linear`, ou déjà finie).
    pub fn check(&mut self, quests: &Quests, index: usize, task: usize) -> bool {
        let quest = &quests.quests[index];
        if !matches!(quest.tasks.get(task), Some(Task::Check)) || !self.advancing(quests, index) {
            return false;
        }
        self.record_mut(&quest.id, quest.tasks.len()).counters[task] = 1;
        true
    }

    /// Remet des objets pour une tâche `consume` : `available` est ce que le joueur a sur lui ;
    /// renvoie combien le jeu doit lui retirer.
    pub fn hand_in(&mut self, quests: &Quests, index: usize, task: usize, available: u32) -> u32 {
        let quest = &quests.quests[index];
        let Some(Task::Item {
            consume: true,
            count,
            ..
        }) = quest.tasks.get(task)
        else {
            return 0;
        };
        if !self.advancing(quests, index) {
            return 0;
        }
        let have = self.counter(&quest.id, task);
        let taken = available.min(count.saturating_sub(have));
        if taken > 0 {
            self.record_mut(&quest.id, quest.tasks.len()).counters[task] = have + taken;
        }
        taken
    }

    /// Termine la quête d'office (essais, `ctl`) : compteurs au maximum, dépendances ignorées.
    /// `false` si elle l'était déjà.
    pub fn force_done(&mut self, quests: &Quests, index: usize) -> bool {
        let quest = &quests.quests[index];
        if self.is_done(&quest.id) {
            return false;
        }
        let record = self.record_mut(&quest.id, quest.tasks.len());
        for (counter, task) in record.counters.iter_mut().zip(&quest.tasks) {
            *counter = task.target();
        }
        self.finish(quests, index);
        true
    }

    /// Efface la progression d'une quête, ou de toutes (`None`).
    pub fn reset(&mut self, id: Option<&str>) {
        self.dirty = true;
        match id {
            Some(id) => {
                self.records.remove(id);
            }
            None => self.records.clear(),
        }
    }

    // ----- Récompenses -------------------------------------------------------------------

    /// Choisit l'option `option` de la récompense `Choice` numéro `reward`. `false` si l'un des
    /// numéros ne convient pas, ou si les récompenses sont déjà récupérées.
    pub fn choose(&mut self, quests: &Quests, index: usize, reward: usize, option: usize) -> bool {
        let quest = &quests.quests[index];
        let Some(Reward::Choice(options)) = quest.rewards.get(reward) else {
            return false;
        };
        if option >= options.len() || self.records.get(&quest.id).is_some_and(|r| r.claimed) {
            return false;
        }
        self.record_mut(&quest.id, quest.tasks.len())
            .choices
            .insert(reward, option);
        true
    }

    /// Récupère les récompenses d'une quête finie. Elles ne sont données **qu'une fois** : la
    /// quête est marquée récupérée avant de rendre la liste. `below(n)` est le hasard des tirages
    /// (un nombre de 0 à n - 1).
    pub fn claim(
        &mut self,
        quests: &Quests,
        index: usize,
        below: &mut dyn FnMut(u32) -> u32,
    ) -> Result<Vec<Grant>, ClaimError> {
        let quest = quests.quests.get(index).ok_or(ClaimError::Unknown)?;
        let record = self.records.get(&quest.id).ok_or(ClaimError::NotDone)?;
        if !record.done {
            return Err(ClaimError::NotDone);
        }
        if record.claimed {
            return Err(ClaimError::AlreadyClaimed);
        }
        let mut grants = Vec::new();
        for (i, reward) in quest.rewards.iter().enumerate() {
            match reward {
                Reward::Item(item, n) => grants.push(Grant::Item(*item, *n)),
                Reward::Xp(points) => grants.push(Grant::Xp(*points)),
                Reward::Loot(table) => {
                    if let Some(table) = quests.loot.get(table) {
                        grants.extend(
                            table
                                .roll(below)
                                .into_iter()
                                .map(|(i, n)| Grant::Item(i, n)),
                        );
                    }
                }
                Reward::Choice(options) => {
                    let picked = record
                        .choices
                        .get(&i)
                        .and_then(|o| options.get(*o))
                        .ok_or(ClaimError::NeedsChoice(i))?;
                    grants.push(Grant::Item(picked.0, picked.1));
                }
            }
        }
        let record = self.record_mut(&quest.id, quest.tasks.len());
        record.claimed = true;
        Ok(grants)
    }

    // ----- Épingles ----------------------------------------------------------------------

    /// Épingle la quête, ou la désépingle si elle l'était. `false` s'il y en a déjà
    /// [`MAX_PINNED`].
    pub fn toggle_pin(&mut self, id: &str) -> bool {
        if let Some(at) = self.pinned.iter().position(|p| p == id) {
            self.pinned.remove(at);
        } else if self.pinned.len() >= MAX_PINNED {
            return false;
        } else {
            self.pinned.push(id.to_string());
        }
        self.dirty = true;
        true
    }

    pub fn set_last_chapter(&mut self, id: &str) {
        if self.last_chapter.as_deref() != Some(id) {
            self.last_chapter = Some(id.to_string());
            self.dirty = true;
        }
    }

    // ----- Sauvegarde --------------------------------------------------------------------

    /// Y a-t-il quelque chose à écrire ?
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Les identifiants que la progression connaît mais que `quests` ne connaît plus (un mod
    /// retiré) : ils restent dans le fichier.
    pub fn unknown_ids(&self, quests: &Quests) -> Vec<&str> {
        self.records
            .keys()
            .filter(|id| quests.index(id).is_none())
            .map(String::as_str)
            .collect()
    }

    pub fn to_ron(&self) -> Result<String> {
        options()
            .to_string_pretty(self, ron::ser::PrettyConfig::default())
            .context("encodage des quêtes")
    }

    pub fn from_ron(text: &str) -> Result<Self> {
        options().from_str(text).context("quests.ron invalide")
    }

    /// Écrit `quests.ron` dans le dossier du monde (rien tant que le joueur n'a rien fait).
    pub fn save(&mut self, dir: &Path) -> Result<()> {
        if self.records.is_empty() && self.pinned.is_empty() && self.last_chapter.is_none() {
            self.dirty = false;
            return Ok(());
        }
        self.version = VERSION;
        engine_voxel::write_atomic(&dir.join(FILE), self.to_ron()?.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    /// Relit `quests.ron` (une progression vide s'il n'existe pas : un ancien monde). Un fichier
    /// illisible est mis de côté (`quests.ron.illisible`), jamais écrasé en silence.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match Self::from_ron(&text) {
            Ok(progress) => progress,
            Err(err) => {
                log::error!("{} illisible ({err:#}) : mis de côté", path.display());
                let _ = std::fs::copy(&path, dir.join("quests.ron.illisible"));
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quests::tests::{load, with_catalog};

    /// Un petit livre qui sert à tout : une chaîne, un choix, un tirage, des tâches de chaque sorte.
    const BOOK: &str = r##"(
        settings: (progression: Flexible),
        groups: [(id: "core:g", title: "G", icon: "stick")],
        loot: { "snacks": (rolls: (1, 1), entries: [(item: "bread", count: (2, 2))]) },
        chapters: [(id: "core:c", group: "core:g", title: "C", icon: "stick", quests: [
            (id: "core:c/log", title: "Bûche", tasks: [Item(item: "#logs", count: 2)],
             rewards: [Item(item: "torch", count: 4), Xp(10)]),
            (id: "core:c/bench", title: "Établi", after: ["core:c/log"],
             tasks: [Item(item: "workbench")],
             rewards: [Loot("snacks")]),
            (id: "core:c/pick", title: "Choix", after: ["core:c/bench"], tasks: [Check],
             rewards: [Choice([(item: "apple"), (item: "bread", count: 3)]), Xp(5)]),
            (id: "core:c/alt", title: "Alternative", after: ["core:c/log", "core:c/bench"],
             requires: One, tasks: [Check]),
            (id: "core:c/hidden", title: "Cachée", after: ["core:c/log"], hidden: true, tasks: [Check]),
            (id: "core:c/secret", title: "Secrète", secret: true, tasks: [Check]),
            (id: "core:c/opt", title: "Facultative", optional: true, tasks: [Check]),
            (id: "core:c/lin", title: "Linéaire", after: ["core:c/log"], progression: Linear,
             tasks: [Break(block: "stone", count: 3)]),
            (id: "core:c/place", title: "Poser", tasks: [Place(block: "torch", count: 2)]),
            (id: "core:c/kill", title: "Tuer", tasks: [Kill(count: 2), Kill(creature: "pig", count: 3)]),
            (id: "core:c/world", title: "Monde", tasks: [Dimension("core:overworld"),
              Biome("core:plains"), Depth(20), Level(2), Tier(1), Structure("core:village")]),
            (id: "core:c/craft", title: "Fabriquer", tasks: [Item(item: "stick", count: 4, crafted: true)]),
            (id: "core:c/give", title: "Donner", tasks: [Item(item: "#planks", count: 5, consume: true)],
             rewards: [Xp(1)]),
        ])],
    )"##;

    fn book() -> Quests {
        load(BOOK).unwrap()
    }

    fn id_of(quests: &Quests, key: &str) -> usize {
        quests.index(&format!("core:c/{key}")).unwrap()
    }

    fn item(key: &str) -> ItemId {
        with_catalog(|c| c.items.id(&format!("core:{key}")).unwrap())
    }

    fn holding(items: &[(&str, u32)]) -> Snapshot {
        Snapshot {
            items: items.iter().map(|(k, n)| (item(k), *n)).collect(),
            ..Snapshot::default()
        }
    }

    fn no_roll() -> impl FnMut(u32) -> u32 {
        |_| 0
    }

    #[test]
    fn dependencies_all_and_one() {
        let quests = book();
        let mut progress = Progress::default();
        let (log, bench, alt, pick) = (
            id_of(&quests, "log"),
            id_of(&quests, "bench"),
            id_of(&quests, "alt"),
            id_of(&quests, "pick"),
        );
        assert_eq!(progress.status(&quests, log), Status::Available);
        assert_eq!(progress.status(&quests, bench), Status::Locked);
        assert_eq!(progress.status(&quests, alt), Status::Locked);
        // La première seulement : `One` suffit pour « alt », pas `All` pour « bench » + « pick ».
        let done = progress.update(&quests, &holding(&[("oak_log", 2)]), &[]);
        assert_eq!(done, vec![log]);
        assert_eq!(progress.status(&quests, log), Status::Done);
        assert_eq!(
            progress.status(&quests, alt),
            Status::Available,
            "One : une suffit"
        );
        assert_eq!(progress.status(&quests, bench), Status::Available);
        assert_eq!(progress.status(&quests, pick), Status::Locked);
        // `All` : deux dépendances demandées.
        let text = r#"(groups: [(id: "core:g", title: "G", icon: "stick")],
            chapters: [(id: "core:c", group: "core:g", title: "C", icon: "stick", quests: [
              (id: "core:c/a", title: "A", tasks: [Check]),
              (id: "core:c/b", title: "B", tasks: [Check]),
              (id: "core:c/all", title: "All", after: ["core:c/a", "core:c/b"], tasks: [Check])])])"#;
        let quests = load(text).unwrap();
        let mut progress = Progress::default();
        progress.force_done(&quests, 0);
        assert_eq!(progress.status(&quests, 2), Status::Locked);
        progress.force_done(&quests, 1);
        assert_eq!(progress.status(&quests, 2), Status::Available);
    }

    #[test]
    fn optional_hidden_and_secret_quests() {
        let quests = book();
        let mut progress = Progress::default();
        let (hidden, secret, opt, log) = (
            id_of(&quests, "hidden"),
            id_of(&quests, "secret"),
            id_of(&quests, "opt"),
            id_of(&quests, "log"),
        );
        assert!(!progress.visible(&quests, hidden), "cachée tant que locked");
        assert!(!progress.visible(&quests, secret));
        assert!(progress.visible(&quests, opt));
        progress.force_done(&quests, log);
        assert!(
            progress.visible(&quests, hidden),
            "ses dépendances sont finies"
        );
        assert!(
            !progress.visible(&quests, secret),
            "jamais avant d'être finie"
        );
        progress.force_done(&quests, secret);
        assert!(progress.visible(&quests, secret));
        // `optional` ne compte pas dans le pourcentage du chapitre.
        let (done, total) = progress.chapter_progress(&quests, 0);
        assert_eq!(total, quests.quests.len() - 1);
        assert_eq!(done, 2);
        progress.force_done(&quests, opt);
        assert_eq!(progress.chapter_progress(&quests, 0).0, 2);
    }

    #[test]
    fn linear_waits_for_dependencies_and_flexible_does_not() {
        let quests = book();
        let mut progress = Progress::default();
        let lin = id_of(&quests, "lin");
        let stone = || Event::Broken("core:stone".into());
        // Linear : la quête dépend de « log », pas finie : les blocs cassés ne comptent pas.
        progress.update(&quests, &Snapshot::default(), &[stone(), stone()]);
        assert_eq!(progress.counter("core:c/lin", 0), 0);
        progress.force_done(&quests, id_of(&quests, "log"));
        progress.update(&quests, &Snapshot::default(), &[stone(), stone()]);
        assert_eq!(progress.counter("core:c/lin", 0), 2);
        assert_eq!(progress.status(&quests, lin), Status::Started);
        let done = progress.update(&quests, &Snapshot::default(), &[stone()]);
        assert_eq!(done, vec![lin]);
        // Flexible : « bench » dépend de « log » mais ses compteurs avancent sans lui.
        let mut progress = Progress::default();
        let done = progress.update(&quests, &holding(&[("workbench", 1)]), &[]);
        assert!(
            done.is_empty(),
            "elle ne se termine pas : « log » n'est pas finie"
        );
        assert_eq!(progress.counter("core:c/bench", 0), 1);
        assert_eq!(
            progress.status(&quests, id_of(&quests, "bench")),
            Status::Locked
        );
        // Dès que la dépendance se termine, elle se termine dans le même passage.
        let done = progress.update(&quests, &holding(&[("workbench", 1), ("oak_log", 2)]), &[]);
        assert_eq!(done, vec![id_of(&quests, "log"), id_of(&quests, "bench")]);
    }

    #[test]
    fn item_tasks_use_groups_stay_done_and_can_be_handed_in_or_crafted() {
        let quests = book();
        let mut progress = Progress::default();
        let log = id_of(&quests, "log");
        // Un groupe : n'importe quelle bûche, ici deux essences.
        progress.update(&quests, &holding(&[("oak_log", 1), ("birch_log", 1)]), &[]);
        assert_eq!(progress.status(&quests, log), Status::Done);
        // Jeter l'objet ensuite ne défait rien.
        progress.update(&quests, &holding(&[]), &[]);
        assert_eq!(progress.status(&quests, log), Status::Done);
        // `crafted` : posséder ne suffit pas, il faut fabriquer.
        let craft = id_of(&quests, "craft");
        progress.update(&quests, &holding(&[("stick", 9)]), &[]);
        assert_eq!(progress.counter("core:c/craft", 0), 0);
        progress.update(&quests, &holding(&[]), &[Event::Crafted(item("stick"), 4)]);
        assert_eq!(
            progress.status(&quests, craft),
            Status::Claimed,
            "sans récompense"
        );
        // `consume` : posséder ne suffit pas non plus, on remet à la main (par lots).
        let give = id_of(&quests, "give");
        progress.update(&quests, &holding(&[("oak_planks", 9)]), &[]);
        assert_eq!(progress.counter("core:c/give", 0), 0);
        assert_eq!(progress.hand_in(&quests, give, 0, 3), 3);
        assert_eq!(
            progress.hand_in(&quests, give, 0, 9),
            2,
            "il n'en manquait que deux"
        );
        assert_eq!(progress.hand_in(&quests, give, 0, 9), 0);
        assert_eq!(
            progress.update(&quests, &Snapshot::default(), &[]),
            vec![give]
        );
        // On ne remet rien pour une tâche qui ne consomme pas.
        assert_eq!(progress.hand_in(&quests, log, 0, 5), 0);
    }

    #[test]
    fn events_are_counted_once_and_by_kind() {
        let quests = book();
        let mut progress = Progress::default();
        let torch = || Event::Placed("core:torch".into());
        progress.update(&quests, &Snapshot::default(), &[torch()]);
        assert_eq!(progress.counter("core:c/place", 0), 1);
        // Un bloc cassé n'est pas un bloc posé, ni l'inverse.
        progress.update(
            &quests,
            &Snapshot::default(),
            &[Event::Broken("core:torch".into())],
        );
        assert_eq!(progress.counter("core:c/place", 0), 1);
        let done = progress.update(&quests, &Snapshot::default(), &[torch(), torch()]);
        assert_eq!(done, vec![id_of(&quests, "place")]);
        assert_eq!(
            progress.counter("core:c/place", 0),
            2,
            "plafonné à la cible"
        );
        // `Kill` sans créature : les hostiles ; avec : celle-là.
        let kill = |kind: &str, hostile| Event::Killed {
            kind: kind.into(),
            hostile,
        };
        progress.update(
            &quests,
            &Snapshot::default(),
            &[
                kill("core:zombie", true),
                kill("core:pig", false),
                kill("core:pig", false),
            ],
        );
        assert_eq!(progress.counter("core:c/kill", 0), 1);
        assert_eq!(progress.counter("core:c/kill", 1), 2);
        progress.update(&quests, &Snapshot::default(), &[kill("core:zombie", true)]);
        assert_eq!(progress.counter("core:c/kill", 0), 2);
        assert_eq!(
            progress.status(&quests, id_of(&quests, "kill")),
            Status::Started
        );
    }

    #[test]
    fn world_tasks_read_the_snapshot() {
        let quests = book();
        let world = id_of(&quests, "world");
        let mut progress = Progress::default();
        let mut snapshot = Snapshot {
            dimension: "core:overworld".into(),
            biome: "core:plains".into(),
            y: 64.0,
            level: 1,
            tier: 0,
            ..Snapshot::default()
        };
        progress.update(&quests, &snapshot, &[]);
        assert_eq!(
            (0..6)
                .map(|t| progress.counter("core:c/world", t))
                .collect::<Vec<_>>(),
            vec![1, 1, 0, 0, 0, 0]
        );
        snapshot.y = 19.5;
        snapshot.level = 2;
        snapshot.tier = 1;
        snapshot.structures = vec!["core:village".into()];
        // Quitter la plaine ne retire rien.
        snapshot.biome = "core:mountains".into();
        assert_eq!(progress.update(&quests, &snapshot, &[]), vec![world]);
    }

    #[test]
    fn check_boxes_and_force_done() {
        let quests = book();
        let mut progress = Progress::default();
        let secret = id_of(&quests, "secret");
        assert!(
            !progress.check(&quests, id_of(&quests, "log"), 0),
            "pas une case"
        );
        assert!(progress.check(&quests, secret, 0));
        assert_eq!(
            progress.update(&quests, &Snapshot::default(), &[]),
            vec![secret]
        );
        assert!(!progress.check(&quests, secret, 0), "déjà finie");
        let log = id_of(&quests, "log");
        assert!(progress.force_done(&quests, log));
        assert!(!progress.force_done(&quests, log));
        assert_eq!(progress.counter("core:c/log", 0), 2);
    }

    #[test]
    fn rewards_are_given_once_and_choices_are_required() {
        let quests = book();
        let mut progress = Progress::default();
        let (log, pick) = (id_of(&quests, "log"), id_of(&quests, "pick"));
        assert_eq!(
            progress.claim(&quests, log, &mut no_roll()),
            Err(ClaimError::NotDone)
        );
        progress.force_done(&quests, log);
        let grants = progress.claim(&quests, log, &mut no_roll()).unwrap();
        assert_eq!(grants, vec![Grant::Item(item("torch"), 4), Grant::Xp(10)]);
        assert_eq!(progress.status(&quests, log), Status::Claimed);
        assert_eq!(
            progress.claim(&quests, log, &mut no_roll()),
            Err(ClaimError::AlreadyClaimed),
            "jamais deux fois"
        );
        // Un tirage.
        let bench = id_of(&quests, "bench");
        progress.force_done(&quests, bench);
        let grants = progress.claim(&quests, bench, &mut no_roll()).unwrap();
        assert_eq!(grants, vec![Grant::Item(item("bread"), 2)]);
        // Un choix : refusé tant qu'il n'est pas fait, et rien n'est consommé par le refus.
        progress.force_done(&quests, pick);
        assert_eq!(
            progress.claim(&quests, pick, &mut no_roll()),
            Err(ClaimError::NeedsChoice(0))
        );
        assert_eq!(progress.status(&quests, pick), Status::Done);
        assert!(!progress.choose(&quests, pick, 0, 5), "option inexistante");
        assert!(!progress.choose(&quests, pick, 1, 0), "pas un choix");
        assert!(progress.choose(&quests, pick, 0, 1));
        assert!(
            progress.choose(&quests, pick, 0, 0),
            "on peut changer d'avis"
        );
        assert!(progress.choose(&quests, pick, 0, 1));
        let grants = progress.claim(&quests, pick, &mut no_roll()).unwrap();
        assert_eq!(grants, vec![Grant::Item(item("bread"), 3), Grant::Xp(5)]);
        assert!(!progress.choose(&quests, pick, 0, 0), "trop tard");
        assert!(progress.pending(&quests).is_empty());
    }

    #[test]
    fn pins_are_limited_and_unknown_ids_are_kept_in_the_save() {
        let quests = book();
        let mut progress = Progress::default();
        for id in ["a", "b", "c"] {
            assert!(progress.toggle_pin(id));
        }
        assert!(!progress.toggle_pin("d"), "trois au plus");
        assert!(progress.toggle_pin("a"), "désépingler");
        assert!(progress.toggle_pin("d"));
        progress.force_done(&quests, id_of(&quests, "log"));
        progress.set_last_chapter("core:c");
        // Une quête d'un mod retiré.
        let mut text = progress.to_ron().unwrap();
        text = text.replace(
            "records: {",
            "records: {\n\"gone:q/x\": (counters: [3], done: true, claimed: true),",
        );
        let mut reread = Progress::from_ron(&text).unwrap();
        assert_eq!(reread.unknown_ids(&quests), vec!["gone:q/x"]);
        assert_eq!(reread.pinned, vec!["b", "c", "d"]);
        // Réécrite avec le livre actuel (sans le mod), elle garde l'inconnue.
        reread.update(&quests, &Snapshot::default(), &[]);
        let again = Progress::from_ron(&reread.to_ron().unwrap()).unwrap();
        assert_eq!(again.record("gone:q/x").unwrap().counters, vec![3]);
        assert!(again.record("gone:q/x").unwrap().claimed);
    }

    #[test]
    fn save_round_trips_and_old_worlds_start_empty() {
        let quests = book();
        let dir = std::env::temp_dir().join(format!("voxel-quests-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Un ancien monde : pas de quests.ron, tout part de zéro, et rien n'est écrit pour rien.
        let mut progress = Progress::load(&dir);
        assert_eq!(progress, Progress::default());
        progress.save(&dir).unwrap();
        assert!(!dir.join(FILE).exists());
        // Le joueur avance.
        let log = id_of(&quests, "log");
        progress.update(&quests, &holding(&[("oak_log", 2)]), &[]);
        progress.update(
            &quests,
            &Snapshot::default(),
            &[Event::Broken("core:stone".into())],
        );
        progress.claim(&quests, log, &mut no_roll()).unwrap();
        progress.toggle_pin("core:c/bench");
        assert!(progress.dirty());
        progress.save(&dir).unwrap();
        assert!(!progress.dirty());
        let reread = Progress::load(&dir);
        assert_eq!(reread.status(&quests, log), Status::Claimed);
        assert_eq!(reread.pinned, vec!["core:c/bench"]);
        assert_eq!(reread, progress);
        // Un fichier illisible est mis de côté, pas écrasé en silence.
        std::fs::write(dir.join(FILE), "pas du RON {{{").unwrap();
        assert_eq!(Progress::load(&dir), Progress::default());
        assert!(dir.join("quests.ron.illisible").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_one_or_all() {
        let quests = book();
        let mut progress = Progress::default();
        progress.force_done(&quests, id_of(&quests, "log"));
        progress.force_done(&quests, id_of(&quests, "secret"));
        progress.reset(Some("core:c/log"));
        assert_eq!(
            progress.status(&quests, id_of(&quests, "log")),
            Status::Available
        );
        assert_eq!(
            progress.status(&quests, id_of(&quests, "secret")),
            Status::Claimed
        );
        progress.reset(None);
        assert_eq!(
            progress.status(&quests, id_of(&quests, "secret")),
            Status::Available
        );
    }
}
