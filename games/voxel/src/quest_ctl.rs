//! Les commandes `ctl` du livre de quêtes, pour les essais sans clavier.
//!
//! - `quests [chapitre|quête]` : l'état des quêtes (état, compteur de chaque tâche) ;
//! - `quest done id` : termine la quête d'office (compteurs au maximum, dépendances ignorées) ;
//! - `quest claim id` : récupère ses récompenses ; `quest claim all` : celles de toutes les quêtes
//!   finies (sauf les choix) ;
//! - `quest check id [tâche]` : coche la case d'une tâche `Check` ;
//! - `quest handin id [tâche]` : remet les objets d'une tâche `consume` ;
//! - `quest choose id récompense option` : choisit l'objet d'une récompense `Choice` ;
//! - `quest scan` : refait la revue tout de suite (au lieu d'attendre la seconde suivante) ;
//! - `quest reset [id]` : efface la progression d'une quête, ou de toutes ;
//! - `quest pin id` ou `questpin id` : épingle une quête (trois au plus), ou la désépingle.
//!
//! Un identifiant sans espace de noms est cherché dans `core` (`steps/log` → `core:steps/log`).

use std::sync::Arc;

use anyhow::{Context as _, Result, bail};

use crate::VoxelGame;
use crate::control::json_str;
use crate::quest_progress::Status;
use crate::quests::{ItemSpec, Quests, Task};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    List(Option<String>),
    Done(String),
    /// `None` : toutes les quêtes finies.
    Claim(Option<String>),
    Check(String, Option<usize>),
    HandIn(String, Option<usize>),
    Choose(String, usize, usize),
    Scan,
    Reset(Option<String>),
    Pin(String),
}

fn qualified(key: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("core:{key}")
    }
}

/// Lit les commandes des quêtes ; `None` si ce n'est pas l'une d'elles.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    let number = |word: &str, what: &str| -> Result<usize> {
        word.parse()
            .with_context(|| format!("{what} : un numéro, pas « {word} »"))
    };
    Some(match (name, args) {
        ("quests", []) => Ok(Command::List(None)),
        ("quests", [id]) => Ok(Command::List(Some(qualified(id)))),
        ("questpin", [id]) => Ok(Command::Pin(qualified(id))),
        ("quest", ["done", id]) => Ok(Command::Done(qualified(id))),
        ("quest", ["claim", "all"]) => Ok(Command::Claim(None)),
        ("quest", ["claim", id]) => Ok(Command::Claim(Some(qualified(id)))),
        ("quest", ["check", id]) => Ok(Command::Check(qualified(id), None)),
        ("quest", ["check", id, task]) => {
            number(task, "quest check").map(|t| Command::Check(qualified(id), Some(t)))
        }
        ("quest", ["handin", id]) => Ok(Command::HandIn(qualified(id), None)),
        ("quest", ["handin", id, task]) => {
            number(task, "quest handin").map(|t| Command::HandIn(qualified(id), Some(t)))
        }
        ("quest", ["choose", id, reward, option]) => (|| {
            Ok(Command::Choose(
                qualified(id),
                number(reward, "quest choose")?,
                number(option, "quest choose")?,
            ))
        })(),
        ("quest", ["scan"]) => Ok(Command::Scan),
        ("quest", ["reset"]) => Ok(Command::Reset(None)),
        ("quest", ["reset", id]) => Ok(Command::Reset(Some(qualified(id)))),
        ("quest", ["pin", id]) => Ok(Command::Pin(qualified(id))),
        ("quest", _) => Err(anyhow::anyhow!(
            "quest : done id | claim id|all | check id [tâche] | handin id [tâche] | choose id récompense option | scan | reset [id] | pin id"
        )),
        _ => return None,
    })
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Locked => "locked",
        Status::Available => "available",
        Status::Started => "started",
        Status::Done => "done",
        Status::Claimed => "claimed",
    }
}

/// Ce qu'une tâche demande, en quelques mots (pour `quests`).
fn describe(task: &Task, items: &engine_voxel::ItemRegistry) -> (&'static str, String) {
    match task {
        Task::Item { spec, .. } => (
            "Item",
            match spec {
                ItemSpec::Item(id) => items.get(*id).key.clone(),
                ItemSpec::Group { name, .. } => format!("#{name}"),
            },
        ),
        Task::Check => ("Check", String::new()),
        Task::Break { block, .. } => ("Break", block.clone()),
        Task::Place { block, .. } => ("Place", block.clone()),
        Task::Kill { creature, .. } => {
            ("Kill", creature.clone().unwrap_or_else(|| "hostile".into()))
        }
        Task::Dimension(key) => ("Dimension", key.clone()),
        Task::Biome(key) => ("Biome", key.clone()),
        Task::Structure(key) => ("Structure", key.clone()),
        Task::Depth(y) => ("Depth", y.to_string()),
        Task::Level(n) => ("Level", n.to_string()),
        Task::Tier(n) => ("Tier", n.to_string()),
    }
}

impl VoxelGame {
    /// Le livre chargé et le numéro d'une quête.
    fn quest_lookup(&mut self, id: &str) -> Result<(Arc<Quests>, usize)> {
        self.open_quests();
        let quests = self.quest_book.quests().context("quêtes pas chargées")?;
        let index = quests
            .index(id)
            .with_context(|| format!("quête inconnue « {id} » (voir `quests`)"))?;
        Ok((quests, index))
    }

    pub(crate) fn quest_command(&mut self, command: Command) -> Result<String> {
        self.open_quests();
        match command {
            Command::List(filter) => self.quest_list(filter.as_deref()),
            Command::Done(id) => {
                let (quests, index) = self.quest_lookup(&id)?;
                let forced = self.quest_book.progress_mut()?.force_done(&quests, index);
                if forced {
                    self.announce_quests(&quests, &[index]);
                }
                // Ce qu'elle débloque se termine dans la foulée.
                self.refresh_quests();
                Ok(format!("{{\"ok\":true,\"forced\":{forced}}}"))
            }
            Command::Claim(None) => {
                let claimed = self.claim_all_quests();
                Ok(format!("{{\"ok\":true,\"claimed\":{claimed}}}"))
            }
            Command::Claim(Some(id)) => {
                let (_, index) = self.quest_lookup(&id)?;
                let gave = self
                    .claim_quest(index)
                    .map_err(|e| anyhow::anyhow!("quest claim : {e}"))?;
                Ok(format!("{{\"ok\":true,\"gave\":{}}}", json_str(&gave)))
            }
            Command::Check(id, task) => {
                let (quests, index) = self.quest_lookup(&id)?;
                let task = task
                    .or_else(|| {
                        quests.quests[index]
                            .tasks
                            .iter()
                            .position(|t| matches!(t, Task::Check))
                    })
                    .context("quest check : cette quête n'a pas de case à cocher")?;
                let checked = self.quest_book.progress_mut()?.check(&quests, index, task);
                self.refresh_quests();
                Ok(format!("{{\"ok\":true,\"checked\":{checked}}}"))
            }
            Command::HandIn(id, task) => {
                let (quests, index) = self.quest_lookup(&id)?;
                let task = task
                    .or_else(|| {
                        quests.quests[index]
                            .tasks
                            .iter()
                            .position(|t| matches!(t, Task::Item { consume: true, .. }))
                    })
                    .context("quest handin : cette quête ne demande pas de remettre d'objets")?;
                let taken = self.hand_in_quest(index, task);
                Ok(format!("{{\"ok\":true,\"taken\":{taken}}}"))
            }
            Command::Choose(id, reward, option) => {
                let (quests, index) = self.quest_lookup(&id)?;
                let chosen = self
                    .quest_book
                    .progress_mut()?
                    .choose(&quests, index, reward, option);
                if !chosen {
                    bail!("quest choose : récompense ou option invalide, ou déjà récupérée");
                }
                Ok("{\"ok\":true}".into())
            }
            Command::Scan => {
                self.refresh_quests();
                self.quest_list(None)
            }
            Command::Reset(id) => {
                if let Some(id) = &id {
                    self.quest_lookup(id)?;
                }
                self.quest_book.progress_mut()?.reset(id.as_deref());
                Ok("{\"ok\":true}".into())
            }
            Command::Pin(id) => {
                self.quest_lookup(&id)?;
                let changed = self.quest_book.progress_mut()?.toggle_pin(&id);
                if !changed {
                    bail!("questpin : trois quêtes épinglées au plus");
                }
                Ok(format!(
                    "{{\"ok\":true,\"pinned\":{}}}",
                    self.quest_book.pinned_json()
                ))
            }
        }
    }

    /// L'état des quêtes : celles d'un chapitre, une seule, ou toutes.
    fn quest_list(&mut self, filter: Option<&str>) -> Result<String> {
        let quests = self.quest_book.quests().context("quêtes pas chargées")?;
        let progress = self
            .quest_book
            .progress()
            .context("quêtes pas chargées")?
            .clone();
        let items = self.content.items.clone();
        let chosen: Vec<usize> = match filter {
            None => (0..quests.quests.len()).collect(),
            Some(id) => match quests.chapter(id) {
                Some(chapter) => {
                    // Le dernier chapitre regardé : le livre s'y rouvrira (phase 2).
                    if let Ok(progress) = self.quest_book.progress_mut() {
                        progress.set_last_chapter(id);
                    }
                    chapter.quests.clone()
                }
                None => vec![
                    quests
                        .index(id)
                        .with_context(|| format!("quests : ni chapitre ni quête « {id} »"))?,
                ],
            },
        };
        let rows: Vec<String> = chosen
            .iter()
            .map(|&i| {
                let quest = &quests.quests[i];
                let tasks: Vec<String> = quest
                    .tasks
                    .iter()
                    .enumerate()
                    .map(|(t, task)| {
                        let (kind, what) = describe(task, &items);
                        format!(
                            "{{\"kind\":\"{kind}\",\"what\":{},\"have\":{},\"need\":{}}}",
                            json_str(&what),
                            progress.counter(&quest.id, t),
                            task.target()
                        )
                    })
                    .collect();
                format!(
                    "{{\"id\":{},\"title\":{},\"status\":\"{}\",\"visible\":{},\"tasks\":[{}]}}",
                    json_str(&quest.id),
                    json_str(&quest.title),
                    status_name(progress.status(&quests, i)),
                    progress.visible(&quests, i),
                    tasks.join(",")
                )
            })
            .collect();
        let chapters: Vec<String> = quests
            .chapters
            .iter()
            .enumerate()
            .map(|(c, chapter)| {
                let (done, total) = progress.chapter_progress(&quests, c);
                format!(
                    "{{\"id\":{},\"done\":{done},\"total\":{total},\"pending\":{}}}",
                    json_str(&chapter.id),
                    progress.pending_in(&quests, c)
                )
            })
            .collect();
        let unknown: Vec<String> = progress
            .unknown_ids(&quests)
            .into_iter()
            .map(json_str)
            .collect();
        Ok(format!(
            "{{\"ok\":true,\"pending\":{},\"unknown\":[{}],\"pinned\":{},\"chapters\":[{}],\"quests\":[{}]}}",
            progress.pending(&quests).len(),
            unknown.join(","),
            self.quest_book.pinned_json(),
            chapters.join(","),
            rows.join(",")
        ))
    }
}
