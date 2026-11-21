//! Le livre de quêtes, côté données : le format de `quests.ron`, son chargement (core puis mods) et
//! ses vérifications.
//!
//! Ce module ne dépend ni de l'ECS ni de l'affichage : il lit du texte RON et le confronte aux
//! registres (objets, blocs) et à des listes de noms ([`Catalog`]). L'avancement du joueur est dans
//! `quest_progress.rs` ; le branchement sur le jeu vivant dans `quest_play.rs`.
//!
//! # Le fichier
//!
//! `assets/core/quests.ron`, puis le `quests.ron` de chaque mod (voir `docs/modding.md`) :
//! - `settings` (core seulement) : la progression et la forme par défaut ;
//! - `groups` : les groupes de chapitres (« Les bases »...) ;
//! - `loot` : des tables de tirage pour les récompenses `Loot` ;
//! - `chapters` : des chapitres, avec leurs quêtes (tâches, récompenses, dépendances).
//!
//! Les identifiants sont `espace:nom` ; un mod n'ajoute que des chapitres, groupes et quêtes de son
//! espace de noms, mais peut ranger un chapitre dans le groupe d'un autre espace et faire dépendre
//! une quête d'une quête d'un autre espace. Si cet autre espace n'est pas chargé (mod désactivé),
//! la dépendance est ignorée avec un avertissement.
//!
//! Les identifiants d'objets, de blocs, de créatures... sans espace de noms prennent celui du
//! fichier (`stick` → `core:stick`) ; `#logs` désigne un groupe d'objets de `items.ron`.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context as _, Result, bail};
use engine_voxel::{BlockRegistry, ItemId, ItemRegistry};
use serde::{Deserialize, Serialize};

/// Comment les tâches avancent par rapport aux dépendances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Progression {
    /// Les tâches avancent toujours ; la quête se termine quand ses dépendances le sont.
    #[default]
    Flexible,
    /// Une tâche n'avance pas tant que les dépendances ne sont pas finies.
    Linear,
}

/// La forme d'une quête sur le plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Shape {
    #[default]
    Circle,
    Square,
    RoundSquare,
    Hexagon,
    Diamond,
    Pentagon,
}

/// Faut-il toutes les dépendances, ou une seule ?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Requires {
    #[default]
    All,
    One,
}

fn one() -> u32 {
    1
}

fn size_one() -> f32 {
    1.0
}

fn rolls_one() -> (u32, u32) {
    (1, 1)
}

fn weight_one() -> u32 {
    1
}

// ---------------------------------------------------------------------------------------------
// Le fichier tel qu'il est écrit.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsEntry {
    #[serde(default)]
    pub progression: Progression,
    #[serde(default)]
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupEntry {
    pub id: String,
    pub title: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LootPick {
    pub item: String,
    #[serde(default = "rolls_one")]
    pub count: (u32, u32),
    #[serde(default = "weight_one")]
    pub weight: u32,
}

/// Une table de tirage : `rolls` tirages, chacun choisit une entrée selon son poids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LootEntry {
    #[serde(default = "rolls_one")]
    pub rolls: (u32, u32),
    pub entries: Vec<LootPick>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceEntry {
    pub item: String,
    #[serde(default = "one")]
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskEntry {
    /// Avoir `count` exemplaires sur soi (`#groupe` accepté).
    Item {
        item: String,
        #[serde(default = "one")]
        count: u32,
        #[serde(default)]
        consume: bool,
        #[serde(default)]
        crafted: bool,
    },
    /// Une case à cocher dans la fiche.
    Check,
    Break {
        block: String,
        #[serde(default = "one")]
        count: u32,
    },
    Place {
        block: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// Tuer `count` créatures ; sans `creature`, n'importe quelle hostile.
    Kill {
        #[serde(default)]
        creature: Option<String>,
        #[serde(default = "one")]
        count: u32,
    },
    Dimension(String),
    Biome(String),
    Structure(String),
    /// Descendre sous cette hauteur (y absolu).
    Depth(i32),
    Level(u32),
    Tier(u8),
    /// Un signal du script du mod (`quest_signal(nom)` donne `<mod>:nom`) ; sans espace de noms,
    /// celui du fichier. Il doit être de l'espace de noms du fichier qui le déclare.
    Signal(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RewardEntry {
    Item {
        item: String,
        #[serde(default = "one")]
        count: u32,
    },
    Xp(u32),
    /// Un tirage dans une table `loot` du fichier.
    Loot(String),
    /// Un objet au choix du joueur.
    Choice(Vec<ChoiceEntry>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub shape: Option<Shape>,
    #[serde(default = "size_one")]
    pub size: f32,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub requires: Requires,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub progression: Option<Progression>,
    pub tasks: Vec<TaskEntry>,
    #[serde(default)]
    pub rewards: Vec<RewardEntry>,
    #[serde(default)]
    pub autoclaim: bool,
    #[serde(default)]
    pub guide: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChapterEntry {
    pub id: String,
    pub group: String,
    pub title: String,
    pub icon: String,
    #[serde(default)]
    pub progression: Option<Progression>,
    pub quests: Vec<QuestEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestsFile {
    #[serde(default)]
    pub settings: Option<SettingsEntry>,
    #[serde(default)]
    pub groups: Vec<GroupEntry>,
    #[serde(default)]
    pub loot: BTreeMap<String, LootEntry>,
    #[serde(default)]
    pub chapters: Vec<ChapterEntry>,
}

/// Lit un fichier `quests.ron` (sans le confronter aux registres).
pub fn parse_file(text: &str, origin: &str) -> Result<QuestsFile> {
    ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
        .with_context(|| format!("{origin} : fichier RON invalide"))
}

// ---------------------------------------------------------------------------------------------
// Les données prêtes à servir.
// ---------------------------------------------------------------------------------------------

/// Un objet demandé : un objet précis, ou un groupe (`#logs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemSpec {
    Item(ItemId),
    Group { name: String, members: Vec<ItemId> },
}

impl ItemSpec {
    pub fn matches(&self, item: ItemId) -> bool {
        match self {
            Self::Item(id) => *id == item,
            Self::Group { members, .. } => members.contains(&item),
        }
    }

    /// L'objet qu'on dessine pour lui (le premier du groupe).
    pub fn icon(&self) -> Option<ItemId> {
        match self {
            Self::Item(id) => Some(*id),
            Self::Group { members, .. } => members.first().copied(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Task {
    Item {
        spec: ItemSpec,
        count: u32,
        consume: bool,
        crafted: bool,
    },
    Check,
    Break {
        block: String,
        count: u32,
    },
    Place {
        block: String,
        count: u32,
    },
    Kill {
        creature: Option<String>,
        count: u32,
    },
    Dimension(String),
    Biome(String),
    Structure(String),
    Depth(i32),
    Level(u32),
    Tier(u8),
    /// Le nom complet du signal (`forge:smelted`).
    Signal(String),
}

impl Task {
    /// Le compteur à atteindre (1 pour les tâches « oui ou non »).
    pub fn target(&self) -> u32 {
        match self {
            Self::Item { count, .. }
            | Self::Break { count, .. }
            | Self::Place { count, .. }
            | Self::Kill { count, .. } => *count,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Reward {
    Item(ItemId, u32),
    Xp(u32),
    /// Le nom complet de la table (`core:snacks`).
    Loot(String),
    Choice(Vec<(ItemId, u32)>),
}

/// Une table de tirage prête à servir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LootTable {
    pub rolls: (u32, u32),
    /// (objet, (min, max), poids)
    pub entries: Vec<(ItemId, (u32, u32), u32)>,
}

impl LootTable {
    /// Un tirage. `below(n)` donne un nombre de 0 à n - 1 (le hasard est celui de l'appelant).
    pub fn roll(&self, below: &mut dyn FnMut(u32) -> u32) -> Vec<(ItemId, u32)> {
        let total: u32 = self.entries.iter().map(|e| e.2).sum();
        let (lo, hi) = self.rolls;
        let rolls = lo + if hi > lo { below(hi - lo + 1) } else { 0 };
        let mut out: Vec<(ItemId, u32)> = Vec::new();
        for _ in 0..rolls {
            if total == 0 {
                break;
            }
            let mut pick = below(total);
            let Some(&(item, (min, max), _)) = self.entries.iter().find(|e| {
                if pick < e.2 {
                    true
                } else {
                    pick -= e.2;
                    false
                }
            }) else {
                continue;
            };
            let count = min + if max > min { below(max - min + 1) } else { 0 };
            match out.iter_mut().find(|(i, _)| *i == item) {
                Some(entry) => entry.1 += count,
                None => out.push((item, count)),
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quest {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub shape: Shape,
    pub size: f32,
    pub icon: ItemId,
    /// Les dépendances connues (celles d'un mod absent sont retirées).
    pub after: Vec<String>,
    /// Leurs numéros dans [`Quests::quests`].
    pub deps: Vec<usize>,
    pub requires: Requires,
    pub optional: bool,
    pub hidden: bool,
    pub secret: bool,
    /// La progression qui s'applique (celle de la quête, sinon du chapitre, sinon du fichier).
    pub progression: Progression,
    pub tasks: Vec<Task>,
    pub rewards: Vec<Reward>,
    pub autoclaim: bool,
    pub guide: Option<String>,
    /// Le numéro du chapitre dans [`Quests::chapters`].
    pub chapter: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub title: String,
    pub icon: ItemId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub group: String,
    pub title: String,
    pub icon: ItemId,
    /// Les numéros de ses quêtes dans [`Quests::quests`].
    pub quests: Vec<usize>,
}

/// Tout ce que le livre contient.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Quests {
    pub default_progression: Progression,
    pub default_shape: Shape,
    pub groups: Vec<Group>,
    pub chapters: Vec<Chapter>,
    pub quests: Vec<Quest>,
    pub loot: HashMap<String, LootTable>,
    by_id: HashMap<String, usize>,
}

/// Ce à quoi on confronte le fichier : les registres, et les noms que d'autres modules possèdent.
pub struct Catalog<'a> {
    pub items: &'a ItemRegistry,
    pub blocks: &'a BlockRegistry,
    /// Les identifiants des chapitres du Guide du voyageur.
    pub guide_chapters: HashSet<String>,
    pub creatures: HashSet<String>,
    pub dimensions: HashSet<String>,
    pub biomes: HashSet<String>,
    pub structures: HashSet<String>,
    /// Le nombre de paliers du monde (le palier 0 compris).
    pub tiers: usize,
    /// Les espaces de noms chargés : `core` et les mods actifs.
    pub namespaces: HashSet<String>,
}

/// `stick` → `core:stick` (dans le fichier de `core`) ; `forge:gear` reste tel quel.
pub fn scoped(key: &str, namespace: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("{namespace}:{key}")
    }
}

/// Le nom d'un signal (sans l'espace de noms) : celui que `quest_signal` accepte.
fn valid_signal(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-./".contains(&b))
}

fn namespace_of(id: &str) -> &str {
    id.split_once(':').map_or("", |(ns, _)| ns)
}

impl Quests {
    /// Charge les fichiers, dans l'ordre (core d'abord), et les vérifie ensemble : une erreur
    /// claire plutôt qu'un livre à moitié faux. Les avertissements (dépendance vers un mod absent)
    /// vont au journal.
    pub fn load(sources: &[(String, String, String)], catalog: &Catalog) -> Result<Self> {
        let mut quests = Self::default();
        let mut deferred: Vec<(String, usize, String)> = Vec::new();
        let mut chapter_groups: Vec<(String, String, String)> = Vec::new();
        for (namespace, text, origin) in sources {
            let file = parse_file(text, origin)?;
            quests.add_file(
                namespace,
                file,
                origin,
                catalog,
                &mut chapter_groups,
                &mut deferred,
            )?;
        }
        for (origin, chapter, group) in &chapter_groups {
            if !quests.groups.iter().any(|g| &g.id == group) {
                bail!(
                    "{origin} : le chapitre « {chapter} » est rangé dans un groupe inconnu « {group} »"
                );
            }
        }
        quests.resolve_dependencies(&deferred, catalog)?;
        quests.check_cycles()?;
        Ok(quests)
    }

    fn add_file(
        &mut self,
        namespace: &str,
        file: QuestsFile,
        origin: &str,
        catalog: &Catalog,
        chapter_groups: &mut Vec<(String, String, String)>,
        deferred: &mut Vec<(String, usize, String)>,
    ) -> Result<()> {
        let own = |id: &str, what: &str| -> Result<()> {
            if namespace_of(id) != namespace {
                bail!("{origin} : {what} « {id} » doit commencer par « {namespace}: »");
            }
            Ok(())
        };
        if let Some(settings) = file.settings {
            if namespace != "core" {
                bail!("{origin} : `settings` n'existe que dans le fichier de core");
            }
            self.default_progression = settings.progression;
            self.default_shape = settings.shape;
        }
        let item = |key: &str, what: &str| -> Result<ItemId> {
            let full = scoped(key, namespace);
            catalog
                .items
                .id(&full)
                .with_context(|| format!("{origin} : {what} : objet inconnu « {full} »"))
        };
        for entry in file.groups {
            own(&entry.id, "le groupe")?;
            if self.groups.iter().any(|g| g.id == entry.id) {
                bail!("{origin} : groupe « {} » en double", entry.id);
            }
            self.groups.push(Group {
                icon: item(&entry.icon, &format!("groupe « {} »", entry.id))?,
                id: entry.id,
                title: entry.title,
            });
        }
        for (name, table) in file.loot {
            let key = scoped(&name, namespace);
            if self.loot.contains_key(&key) {
                bail!("{origin} : table de tirage « {key} » en double");
            }
            if table.entries.is_empty() || table.entries.iter().all(|e| e.weight == 0) {
                bail!("{origin} : la table « {key} » n'a aucune entrée");
            }
            if table.rolls.0 > table.rolls.1 {
                bail!("{origin} : la table « {key} » : rolls (min, max) à l'envers");
            }
            let entries = table
                .entries
                .iter()
                .map(|e| {
                    if e.count.0 == 0 || e.count.0 > e.count.1 {
                        bail!("{origin} : table « {key} » : count (min, max) invalide");
                    }
                    Ok((
                        item(&e.item, &format!("table « {key} »"))?,
                        e.count,
                        e.weight,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            self.loot.insert(
                key,
                LootTable {
                    rolls: table.rolls,
                    entries,
                },
            );
        }
        for chapter in file.chapters {
            let ChapterEntry {
                id: chapter_id,
                group,
                title,
                icon,
                progression,
                quests: entries,
            } = chapter;
            own(&chapter_id, "le chapitre")?;
            if self.chapters.iter().any(|c| c.id == chapter_id) {
                bail!("{origin} : chapitre « {chapter_id} » en double");
            }
            if entries.is_empty() {
                bail!("{origin} : le chapitre « {chapter_id} » n'a aucune quête");
            }
            let group = scoped(&group, namespace);
            chapter_groups.push((origin.to_string(), chapter_id.clone(), group.clone()));
            let chapter_icon = item(&icon, &format!("chapitre « {chapter_id} »"))?;
            let chapter_index = self.chapters.len();
            let mut indexes = Vec::new();
            for entry in entries {
                own(&entry.id, "la quête")?;
                if self.by_id.contains_key(&entry.id) {
                    bail!("{origin} : quête « {} » en double", entry.id);
                }
                let index = self.quests.len();
                let (mut quest, after) = self.build_quest(
                    entry,
                    namespace,
                    origin,
                    catalog,
                    (chapter_icon, progression),
                )?;
                quest.chapter = chapter_index;
                for dependency in after {
                    deferred.push((origin.to_string(), index, dependency));
                }
                self.by_id.insert(quest.id.clone(), index);
                indexes.push(index);
                self.quests.push(quest);
            }
            self.chapters.push(Chapter {
                icon: chapter_icon,
                id: chapter_id,
                group,
                title,
                quests: indexes,
            });
        }
        Ok(())
    }

    /// Une quête vérifiée, et ses dépendances telles qu'écrites (résolues plus tard : elles
    /// peuvent viser une quête d'un fichier suivant).
    fn build_quest(
        &self,
        entry: QuestEntry,
        namespace: &str,
        origin: &str,
        catalog: &Catalog,
        (chapter_icon, chapter_progression): (ItemId, Option<Progression>),
    ) -> Result<(Quest, Vec<String>)> {
        let id = entry.id.clone();
        let ctx = |what: String| format!("{origin} : quête « {id} » : {what}");
        if entry.tasks.is_empty() {
            bail!("{}", ctx("aucune tâche".into()));
        }
        if !entry.x.is_finite()
            || !entry.y.is_finite()
            || !entry.size.is_finite()
            || entry.size <= 0.0
        {
            bail!("{}", ctx("position ou taille invalide".into()));
        }
        let known = |set: &HashSet<String>, key: &str, what: &str| -> Result<String> {
            let full = scoped(key, namespace);
            if set.contains(&full) {
                Ok(full)
            } else {
                bail!("{}", ctx(format!("{what} inconnu « {full} »")))
            }
        };
        let block = |key: &str| -> Result<String> {
            let full = scoped(key, namespace);
            if catalog.blocks.id(&full).is_some() {
                Ok(full)
            } else {
                bail!("{}", ctx(format!("bloc inconnu « {full} »")))
            }
        };
        let item = |key: &str| -> Result<ItemId> {
            let full = scoped(key, namespace);
            catalog
                .items
                .id(&full)
                .with_context(|| ctx(format!("objet inconnu « {full} »")))
        };
        let count = |n: u32| -> Result<u32> {
            if n == 0 {
                bail!("{}", ctx("un nombre doit être d'au moins 1".into()));
            }
            Ok(n)
        };
        let mut tasks = Vec::new();
        for task in entry.tasks {
            tasks.push(match task {
                TaskEntry::Item {
                    item: key,
                    count: n,
                    consume,
                    crafted,
                } => {
                    let spec = match key.strip_prefix('#') {
                        Some(group) => {
                            let members = catalog.items.tag(group);
                            if members.is_empty() {
                                bail!(
                                    "{}",
                                    ctx(format!("groupe d'objets inconnu ou vide « #{group} »"))
                                );
                            }
                            ItemSpec::Group {
                                name: group.to_string(),
                                members: members.to_vec(),
                            }
                        }
                        None => ItemSpec::Item(item(&key)?),
                    };
                    if consume && crafted {
                        bail!(
                            "{}",
                            ctx("`consume` et `crafted` ne vont pas ensemble".into())
                        );
                    }
                    Task::Item {
                        spec,
                        count: count(n)?,
                        consume,
                        crafted,
                    }
                }
                TaskEntry::Check => Task::Check,
                TaskEntry::Break {
                    block: key,
                    count: n,
                } => Task::Break {
                    block: block(&key)?,
                    count: count(n)?,
                },
                TaskEntry::Place {
                    block: key,
                    count: n,
                } => Task::Place {
                    block: block(&key)?,
                    count: count(n)?,
                },
                TaskEntry::Kill { creature, count: n } => Task::Kill {
                    creature: creature
                        .map(|c| known(&catalog.creatures, &c, "créature"))
                        .transpose()?,
                    count: count(n)?,
                },
                TaskEntry::Dimension(key) => {
                    Task::Dimension(known(&catalog.dimensions, &key, "dimension")?)
                }
                TaskEntry::Biome(key) => Task::Biome(known(&catalog.biomes, &key, "biome")?),
                TaskEntry::Structure(key) => {
                    Task::Structure(known(&catalog.structures, &key, "structure")?)
                }
                TaskEntry::Depth(y) => Task::Depth(y),
                TaskEntry::Level(n) => Task::Level(count(n)?),
                TaskEntry::Tier(n) => {
                    if n == 0 || usize::from(n) >= catalog.tiers {
                        bail!(
                            "{}",
                            ctx(format!(
                                "palier {n} inexistant (1 à {})",
                                catalog.tiers.saturating_sub(1)
                            ))
                        );
                    }
                    Task::Tier(n)
                }
                TaskEntry::Signal(name) => {
                    let full = scoped(&name, namespace);
                    // Seul le mod de ce nom peut l'envoyer : un signal d'un autre espace ne se
                    // remplirait jamais.
                    if namespace_of(&full) != namespace || !valid_signal(&full[namespace.len() + 1..]) {
                        bail!(
                            "{}",
                            ctx(format!(
                                "signal « {name} » : il doit s'écrire `{namespace}:nom` (minuscules, chiffres, `_`, `-`, `.`, `/`), l'espace de noms du fichier"
                            ))
                        );
                    }
                    Task::Signal(full)
                }
            });
        }
        let mut rewards = Vec::new();
        for reward in entry.rewards {
            rewards.push(match reward {
                RewardEntry::Item {
                    item: key,
                    count: n,
                } => Reward::Item(item(&key)?, count(n)?),
                RewardEntry::Xp(points) => Reward::Xp(count(points)?),
                RewardEntry::Loot(name) => {
                    let key = scoped(&name, namespace);
                    // Les tables d'un fichier suivant n'existent pas encore : on vérifie à la fin
                    // pour les noms d'un autre espace, tout de suite pour les siens.
                    if namespace_of(&key) == namespace && !self.loot.contains_key(&key) {
                        bail!("{}", ctx(format!("table de tirage inconnue « {key} »")));
                    }
                    Reward::Loot(key)
                }
                RewardEntry::Choice(options) => {
                    if options.len() < 2 {
                        bail!("{}", ctx("`Choice` demande au moins deux objets".into()));
                    }
                    Reward::Choice(
                        options
                            .iter()
                            .map(|o| Ok((item(&o.item)?, count(o.count)?)))
                            .collect::<Result<_>>()?,
                    )
                }
            });
        }
        if let Some(guide) = &entry.guide
            && !catalog.guide_chapters.contains(&scoped(guide, namespace))
        {
            bail!("{}", ctx(format!("chapitre du guide inconnu « {guide} »")));
        }
        let icon = match &entry.icon {
            Some(key) => item(key)?,
            None => Self::default_icon(&tasks, catalog, chapter_icon),
        };
        let progression = entry
            .progression
            .or(chapter_progression)
            .unwrap_or(self.default_progression);
        let after: Vec<String> = entry.after.iter().map(|a| scoped(a, namespace)).collect();
        Ok((
            Quest {
                id: entry.id,
                title: entry.title,
                subtitle: entry.subtitle,
                text: entry.text,
                x: entry.x,
                y: entry.y,
                shape: entry.shape.unwrap_or(self.default_shape),
                size: entry.size,
                icon,
                after: Vec::new(),
                deps: Vec::new(),
                requires: entry.requires,
                optional: entry.optional,
                hidden: entry.hidden,
                secret: entry.secret,
                progression,
                tasks,
                rewards,
                autoclaim: entry.autoclaim,
                guide: entry.guide.map(|g| scoped(&g, namespace)),
                chapter: 0,
            },
            after,
        ))
    }

    /// L'icône d'une quête sans `icon` : l'objet de sa première tâche, sinon celui du chapitre.
    fn default_icon(tasks: &[Task], catalog: &Catalog, chapter_icon: ItemId) -> ItemId {
        let from_task = match tasks.first() {
            Some(Task::Item { spec, .. }) => spec.icon(),
            Some(Task::Break { block, .. } | Task::Place { block, .. }) => catalog
                .blocks
                .id(block)
                .and_then(|b| catalog.items.of_block(b)),
            _ => None,
        };
        from_task.unwrap_or(chapter_icon)
    }

    /// Relie chaque quête à ses dépendances. Un espace de noms absent : avertissement, la
    /// dépendance est ignorée ; un espace chargé mais un identifiant inconnu : erreur.
    fn resolve_dependencies(
        &mut self,
        deferred: &[(String, usize, String)],
        catalog: &Catalog,
    ) -> Result<()> {
        for (origin, quest, dependency) in deferred {
            match self.by_id.get(dependency) {
                Some(&target) => {
                    let q = &mut self.quests[*quest];
                    if target == *quest {
                        bail!("{origin} : la quête « {} » dépend d'elle-même", q.id);
                    }
                    if !q.deps.contains(&target) {
                        q.after.push(dependency.clone());
                        q.deps.push(target);
                    }
                }
                None if !catalog.namespaces.contains(namespace_of(dependency)) => {
                    log::warn!(
                        "{origin} : la quête « {} » dépend de « {dependency} » (mod absent) : dépendance ignorée",
                        self.quests[*quest].id
                    );
                }
                None => bail!(
                    "{origin} : la quête « {} » dépend de « {dependency} », qui n'existe pas",
                    self.quests[*quest].id
                ),
            }
        }
        // Les tables `Loot` d'un autre espace : elles existent maintenant.
        for quest in &self.quests {
            for reward in &quest.rewards {
                if let Reward::Loot(key) = reward
                    && !self.loot.contains_key(key)
                {
                    bail!(
                        "quête « {} » : table de tirage inconnue « {key} »",
                        quest.id
                    );
                }
            }
        }
        Ok(())
    }

    /// Les dépendances ne doivent pas faire de boucle (une quête qui attendrait la fin d'elle-même).
    fn check_cycles(&self) -> Result<()> {
        // 0 : pas vue, 1 : en cours, 2 : finie.
        let mut state = vec![0u8; self.quests.len()];
        fn visit(
            quests: &Quests,
            at: usize,
            state: &mut [u8],
            path: &mut Vec<usize>,
        ) -> Result<()> {
            match state[at] {
                2 => return Ok(()),
                1 => {
                    let from = path.iter().position(|&p| p == at).unwrap_or(0);
                    let cycle: Vec<&str> = path[from..]
                        .iter()
                        .chain(std::iter::once(&at))
                        .map(|&i| quests.quests[i].id.as_str())
                        .collect();
                    bail!("dépendances en boucle : {}", cycle.join(" → "));
                }
                _ => {}
            }
            state[at] = 1;
            path.push(at);
            for &dep in &quests.quests[at].deps {
                visit(quests, dep, state, path)?;
            }
            path.pop();
            state[at] = 2;
            Ok(())
        }
        for start in 0..self.quests.len() {
            visit(self, start, &mut state, &mut Vec::new())?;
        }
        Ok(())
    }

    /// Le numéro d'une quête.
    pub fn index(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    #[cfg(test)]
    pub fn quest(&self, id: &str) -> Option<&Quest> {
        self.index(id).map(|i| &self.quests[i])
    }

    pub fn chapter(&self, id: &str) -> Option<&Chapter> {
        self.chapters.iter().find(|c| c.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.quests.is_empty()
    }

    /// Les signaux que des tâches attendent (le jeu ignore les autres).
    pub fn wanted_signals(&self) -> HashSet<String> {
        self.quests
            .iter()
            .flat_map(|q| &q.tasks)
            .filter_map(|t| match t {
                Task::Signal(id) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// Les clés de structures que des tâches cherchent (le jeu ne cherche qu'elles).
    pub fn wanted_structures(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .quests
            .iter()
            .flat_map(|q| &q.tasks)
            .filter_map(|t| match t {
                Task::Structure(key) => Some(key.as_str()),
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::path::Path;

    pub fn registries() -> (std::sync::Arc<BlockRegistry>, ItemRegistry) {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let blocks = std::sync::Arc::new(crate::test_registry());
        let items = ItemRegistry::load_mods(&blocks, &assets, &[]).unwrap();
        (blocks, items)
    }

    pub fn names(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Un catalogue de test : le jeu de base, avec quelques noms de créatures, biomes...
    pub fn with_catalog<R>(run: impl FnOnce(&Catalog) -> R) -> R {
        let (blocks, items) = registries();
        let catalog = Catalog {
            items: &items,
            blocks: &blocks,
            guide_chapters: names(&["core:first_steps", "core:tools"]),
            creatures: names(&["core:zombie", "core:pig"]),
            dimensions: names(&["core:overworld", "mecanique:foundry"]),
            biomes: names(&["core:plains", "core:mountains"]),
            structures: names(&["core:village"]),
            tiers: 5,
            namespaces: names(&["core", "forge"]),
        };
        run(&catalog)
    }

    pub fn load(text: &str) -> Result<Quests> {
        with_catalog(|c| Quests::load(&[("core".into(), text.into(), "test".into())], c))
    }

    const SAMPLE: &str = r##"(
        settings: (progression: Flexible, shape: Circle),
        groups: [(id: "core:basics", title: "Les bases", icon: "workbench")],
        loot: { "snacks": (rolls: (1, 2), entries: [(item: "bread", count: (1, 2), weight: 3), (item: "apple")]) },
        chapters: [(
            id: "core:wood", group: "core:basics", title: "Bois", icon: "oak_log",
            quests: [
                (id: "core:wood/log", title: "Une bûche", text: "Frappe.", size: 2.0, shape: Hexagon,
                 tasks: [Item(item: "#logs", count: 2)],
                 rewards: [Item(item: "torch", count: 4), Xp(10), Loot("snacks")]),
                (id: "core:wood/bench", title: "Établi", x: 2.0, after: ["core:wood/log"],
                 tasks: [Item(item: "workbench"), Check],
                 rewards: [Choice([(item: "apple"), (item: "bread", count: 2)])],
                 guide: "core:tools"),
            ],
        )],
    )"##;

    #[test]
    fn a_file_loads_with_defaults_and_resolved_links() {
        let quests = load(SAMPLE).unwrap();
        assert_eq!(quests.quests.len(), 2);
        let log = quests.quest("core:wood/log").unwrap();
        assert_eq!(log.shape, Shape::Hexagon);
        assert_eq!(log.size, 2.0);
        assert!(matches!(
            &log.tasks[0],
            Task::Item { spec: ItemSpec::Group { name, members }, count: 2, consume: false, crafted: false }
                if name == "logs" && members.len() == 6
        ));
        let bench = quests.quest("core:wood/bench").unwrap();
        assert_eq!(bench.shape, Shape::Circle, "la forme par défaut");
        assert_eq!(bench.size, 1.0);
        assert_eq!(bench.deps, vec![0]);
        assert_eq!(bench.after, vec!["core:wood/log".to_string()]);
        assert_eq!(bench.progression, Progression::Flexible);
        assert_eq!(bench.guide.as_deref(), Some("core:tools"));
        assert_eq!(quests.chapters[0].quests, vec![0, 1]);
        assert!(quests.loot.contains_key("core:snacks"));
        // L'icône par défaut : le premier objet de la première tâche (une bûche).
        assert!(
            quests
                .quest("core:wood/log")
                .is_some_and(|q| q.icon != ItemId(0))
        );
    }

    #[test]
    fn the_file_format_round_trips() {
        let file = parse_file(SAMPLE, "test").unwrap();
        let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default()).unwrap();
        let again = parse_file(&text, "aller-retour").unwrap();
        assert_eq!(file, again);
    }

    fn error_of(text: &str) -> String {
        format!("{:#}", load(text).unwrap_err())
    }

    fn chapter(quests: &str) -> String {
        format!(
            r#"(groups: [(id: "core:g", title: "G", icon: "stick")],
            chapters: [(id: "core:c", group: "core:g", title: "C", icon: "stick", quests: [{quests}])])"#
        )
    }

    #[test]
    fn unknown_things_are_refused_with_a_clear_message() {
        let q = |tasks: &str| {
            chapter(&format!(
                r#"(id: "core:c/a", title: "A", tasks: [{tasks}])"#
            ))
        };
        assert!(
            error_of(&q(r#"Item(item: "licorne")"#)).contains("objet inconnu « core:licorne »")
        );
        assert!(error_of(&q(r##"Item(item: "#nope")"##)).contains("groupe d'objets inconnu"));
        assert!(error_of(&q(r#"Break(block: "licorne")"#)).contains("bloc inconnu"));
        assert!(error_of(&q(r#"Kill(creature: "licorne")"#)).contains("créature inconnu"));
        assert!(error_of(&q(r#"Biome("core:lune")"#)).contains("biome inconnu"));
        assert!(error_of(&q(r#"Dimension("core:lune")"#)).contains("dimension inconnu"));
        assert!(error_of(&q(r#"Structure("core:lune")"#)).contains("structure inconnu"));
        assert!(error_of(&q("Tier(9)")).contains("palier 9 inexistant"));
        assert!(error_of(&q("Item(item: \"stick\", count: 0)")).contains("au moins 1"));
        assert!(error_of(&q("")).contains("aucune tâche"));
        let rewards = |r: &str| {
            chapter(&format!(
                r#"(id: "core:c/a", title: "A", tasks: [Check], rewards: [{r}])"#
            ))
        };
        assert!(error_of(&rewards(r#"Loot("rien")"#)).contains("table de tirage inconnue"));
        assert!(error_of(&rewards(r#"Choice([(item: "apple")])"#)).contains("au moins deux"));
        let guide = chapter(r#"(id: "core:c/a", title: "A", tasks: [Check], guide: "core:lune")"#);
        assert!(error_of(&guide).contains("chapitre du guide inconnu"));
        // Un chapitre dans un groupe qui n'existe pas, ou sans quête.
        let lost = r#"(chapters: [(id: "core:c", group: "core:x", title: "C", icon: "stick",
            quests: [(id: "core:c/a", title: "A", tasks: [Check])])])"#;
        assert!(error_of(lost).contains("groupe inconnu"));
        let empty = r#"(groups: [(id: "core:g", title: "G", icon: "stick")],
            chapters: [(id: "core:c", group: "core:g", title: "C", icon: "stick", quests: [])])"#;
        assert!(error_of(empty).contains("aucune quête"));
        assert!(error_of("(chapitres: [])").contains("RON invalide"));
    }

    #[test]
    fn duplicates_and_foreign_namespaces_are_refused() {
        let twice = chapter(
            r#"(id: "core:c/a", title: "A", tasks: [Check]), (id: "core:c/a", title: "B", tasks: [Check])"#,
        );
        assert!(error_of(&twice).contains("en double"));
        let foreign = chapter(r#"(id: "autre:c/a", title: "A", tasks: [Check])"#);
        assert!(error_of(&foreign).contains("doit commencer par « core: »"));
        // `settings` n'est permis que dans core.
        with_catalog(|c| {
            let text = r#"(settings: (progression: Linear))"#;
            let err = Quests::load(
                &[("forge".into(), text.into(), "forge/quests.ron".into())],
                c,
            );
            assert!(format!("{:#}", err.unwrap_err()).contains("que dans le fichier de core"));
        });
        // Un groupe ou un chapitre en double entre deux fichiers.
        with_catalog(|c| {
            let a = chapter(r#"(id: "core:c/a", title: "A", tasks: [Check])"#);
            let sources = [
                ("core".to_string(), a.clone(), "a".to_string()),
                ("core".to_string(), a, "b".to_string()),
            ];
            assert!(Quests::load(&sources, c).is_err());
        });
    }

    #[test]
    fn dependencies_are_checked_and_cycles_refused() {
        let missing =
            chapter(r#"(id: "core:c/a", title: "A", after: ["core:c/z"], tasks: [Check])"#);
        assert!(error_of(&missing).contains("qui n'existe pas"));
        let cycle = chapter(
            r#"(id: "core:c/a", title: "A", after: ["core:c/b"], tasks: [Check]),
               (id: "core:c/b", title: "B", after: ["core:c/c"], tasks: [Check]),
               (id: "core:c/c", title: "C", after: ["core:c/a"], tasks: [Check])"#,
        );
        let message = error_of(&cycle);
        assert!(message.contains("en boucle"), "{message}");
        let itself =
            chapter(r#"(id: "core:c/a", title: "A", after: ["core:c/a"], tasks: [Check])"#);
        assert!(error_of(&itself).contains("d'elle-même"));
        // Une dépendance vers un mod absent est ignorée ; vers un mod chargé mais inconnue : erreur.
        let absent =
            chapter(r#"(id: "core:c/a", title: "A", after: ["absent:x"], tasks: [Check])"#);
        let quests = load(&absent).unwrap();
        assert!(quests.quests[0].deps.is_empty());
        let loaded = chapter(r#"(id: "core:c/a", title: "A", after: ["forge:x"], tasks: [Check])"#);
        assert!(error_of(&loaded).contains("qui n'existe pas"));
    }

    #[test]
    fn mods_add_chapters_and_depend_on_core_quests() {
        with_catalog(|c| {
            let core = chapter(r#"(id: "core:c/a", title: "A", tasks: [Check])"#);
            let forge = r#"(chapters: [(id: "forge:f", group: "core:g", title: "Forge", icon: "core:stick",
                quests: [(id: "forge:f/a", title: "FA", after: ["core:c/a"], tasks: [Check])])])"#;
            let sources = [
                ("core".to_string(), core, "core".to_string()),
                ("forge".to_string(), forge.to_string(), "forge".to_string()),
            ];
            let quests = Quests::load(&sources, c).unwrap();
            assert_eq!(quests.quest("forge:f/a").unwrap().deps, vec![0]);
            assert_eq!(quests.chapters[1].group, "core:g");
        });
    }

    /// Un signal est de l'espace de noms du fichier qui le déclare : `forge:smelted`, ou `smelted`
    /// (l'espace du fichier est ajouté). Celui d'un autre mod ne se remplirait jamais.
    #[test]
    fn signals_must_belong_to_the_namespace_of_their_file() {
        with_catalog(|c| {
            let forge = |signal: &str| {
                format!(
                    r#"(chapters: [(id: "forge:f", group: "core:g", title: "Forge", icon: "core:stick",
                    quests: [(id: "forge:f/a", title: "FA", tasks: [Signal("{signal}")])])])"#
                )
            };
            let core = chapter(r#"(id: "core:c/a", title: "A", tasks: [Check])"#);
            let load_forge = |signal: &str| {
                Quests::load(
                    &[
                        ("core".to_string(), core.clone(), "core".to_string()),
                        ("forge".to_string(), forge(signal), "forge".to_string()),
                    ],
                    c,
                )
            };
            for written in ["forge:smelted", "smelted", "ingot/steel-2.x"] {
                let quests = load_forge(written).unwrap();
                let Task::Signal(full) = &quests.quest("forge:f/a").unwrap().tasks[0] else {
                    panic!("une tâche Signal");
                };
                assert_eq!(
                    *full,
                    format!("forge:{}", written.trim_start_matches("forge:"))
                );
                assert_eq!(quests.wanted_signals(), HashSet::from([full.clone()]));
            }
            for refused in ["core:smelted", "autre:smelted", "forge:", "", "Majuscule"] {
                let err = format!("{:#}", load_forge(refused).unwrap_err());
                assert!(err.contains("signal"), "{refused} : {err}");
            }
        });
        // Dans core, un signal sans espace de noms est `core:...`.
        let quests = load(&chapter(
            r#"(id: "core:c/a", title: "A", tasks: [Signal("hello")])"#,
        ))
        .unwrap();
        assert_eq!(quests.quests[0].tasks[0], Task::Signal("core:hello".into()));
    }

    #[test]
    fn progression_is_inherited_from_file_to_chapter_to_quest() {
        let text = r#"(settings: (progression: Linear),
            groups: [(id: "core:g", title: "G", icon: "stick")],
            chapters: [
              (id: "core:c", group: "core:g", title: "C", icon: "stick", quests: [
                (id: "core:c/a", title: "A", tasks: [Check]),
                (id: "core:c/b", title: "B", tasks: [Check], progression: Flexible)]),
              (id: "core:d", group: "core:g", title: "D", icon: "stick", progression: Flexible, quests: [
                (id: "core:d/a", title: "A", tasks: [Check]),
                (id: "core:d/b", title: "B", tasks: [Check], progression: Linear)])])"#;
        let quests = load(text).unwrap();
        let of = |id: &str| quests.quest(id).unwrap().progression;
        assert_eq!(of("core:c/a"), Progression::Linear);
        assert_eq!(of("core:c/b"), Progression::Flexible);
        assert_eq!(of("core:d/a"), Progression::Flexible);
        assert_eq!(of("core:d/b"), Progression::Linear);
    }

    #[test]
    fn a_loot_table_draws_by_weight_and_merges_stacks() {
        let quests = load(SAMPLE).unwrap();
        let table = &quests.loot["core:snacks"];
        // Un hasard qui choisit toujours la première possibilité.
        let drawn = table.roll(&mut |_| 0);
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].1, 1);
        // Deux tirages (n - 1 : le maximum) sur la même entrée : ses lots s'additionnent.
        let drawn = table.roll(&mut |n| n - 1);
        assert_eq!(drawn.len(), 1, "la dernière entrée, deux fois : {drawn:?}");
        assert_eq!(drawn[0].1, 2);
    }
}
