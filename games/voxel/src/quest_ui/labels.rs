//! Les textes du livre de quêtes : états, tâches, récompenses, paragraphes. Rien ici ne dessine :
//! ce sont des fonctions de données, testées, que la fiche, l'infobulle et les quêtes épinglées
//! partagent.

use engine_voxel::{BlockRegistry, ItemId, ItemRegistry};

use crate::mobs::MobRegistry;
use crate::quest_progress::{Progress, Status};
use crate::quests::{ItemSpec, Quest, Reward, Task};

/// Ce qu'il faut pour nommer les choses citées par les quêtes.
pub struct Namer<'a> {
    pub items: &'a ItemRegistry,
    pub blocks: &'a BlockRegistry,
    pub creatures: &'a MobRegistry,
}

/// `core:ruined_village` → « Ruined village » : un nom lisible à défaut d'un vrai nom.
pub fn humanize(key: &str) -> String {
    let name = key
        .rsplit(':')
        .next()
        .unwrap_or(key)
        .replace(['_', '/'], " ");
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

pub fn status_label(status: Status) -> &'static str {
    match status {
        Status::Locked => "Verrouillée",
        Status::Available => "Disponible",
        Status::Started => "En cours",
        Status::Done => "Terminée : récompense à récupérer",
        Status::Claimed => "Terminée",
    }
}

/// Les paragraphes d'un texte de quête : séparés par une ligne vide, sans lignes vides.
pub fn paragraphs(text: &str) -> Vec<&str> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// Le nom d'un groupe d'objets (`#logs`).
fn group_name(name: &str) -> String {
    match name {
        "logs" => "Bûches".into(),
        "planks" => "Planches".into(),
        "leaves" => "Feuilles".into(),
        "stones" => "Pierres".into(),
        "coals" => "Charbons".into(),
        "wools" => "Laines".into(),
        other => humanize(other),
    }
}

impl Namer<'_> {
    pub fn item(&self, item: ItemId) -> String {
        self.items.get(item).name.clone()
    }

    /// Un objet, ou « Bûches (au choix) » pour un groupe.
    pub fn spec(&self, spec: &ItemSpec) -> String {
        match spec {
            ItemSpec::Item(id) => self.item(*id),
            ItemSpec::Group { name, .. } => format!("{} (au choix)", group_name(name)),
        }
    }

    fn block(&self, key: &str) -> String {
        self.blocks
            .id(key)
            .map_or_else(|| humanize(key), |b| self.blocks.def(b).name.clone())
    }

    fn creature(&self, key: &str) -> String {
        self.creatures
            .by_key(key)
            .map_or_else(|| humanize(key), |d| d.name.clone())
    }

    /// Ce qu'une tâche demande, en une phrase courte.
    pub fn task(&self, task: &Task) -> String {
        match task {
            Task::Item {
                spec,
                consume,
                crafted,
                ..
            } => {
                let name = self.spec(spec);
                match (consume, crafted) {
                    (true, _) => format!("Remettre : {name}"),
                    (_, true) => format!("Fabriquer : {name}"),
                    _ => format!("Avoir : {name}"),
                }
            }
            Task::Check => "Le faire, puis cocher la case".into(),
            Task::Break { block, .. } => format!("Casser : {}", self.block(block)),
            Task::Place { block, .. } => format!("Poser : {}", self.block(block)),
            Task::Kill {
                creature: Some(key),
                ..
            } => format!("Tuer : {}", self.creature(key)),
            Task::Kill { creature: None, .. } => "Tuer des créatures hostiles".into(),
            Task::Dimension(key) => format!("Entrer dans : {}", humanize(key)),
            Task::Biome(key) => format!("Se rendre dans le biome : {}", humanize(key)),
            Task::Structure(key) => format!("Trouver : {}", humanize(key)),
            Task::Depth(y) => format!("Descendre sous la hauteur {y}"),
            Task::Level(n) => format!("Atteindre le niveau {n}"),
            Task::Tier(n) => format!("Débloquer le palier {n}"),
            Task::Signal(_) => "Le faire en jeu".into(),
        }
    }

    /// L'objet dont on montre l'icône pour une tâche, s'il y en a un.
    pub fn task_icon(&self, task: &Task) -> Option<ItemId> {
        match task {
            Task::Item { spec, .. } => spec.icon(),
            Task::Break { block, .. } | Task::Place { block, .. } => {
                self.blocks.id(block).and_then(|b| self.items.of_block(b))
            }
            _ => None,
        }
    }

    /// Une récompense, en une ligne (les choix : « Un objet au choix »).
    pub fn reward(&self, reward: &Reward) -> String {
        match reward {
            Reward::Item(item, n) if *n > 1 => format!("{n} × {}", self.item(*item)),
            Reward::Item(item, _) => self.item(*item),
            Reward::Xp(points) => format!("{points} points d'expérience"),
            Reward::Loot(_) => "Un tirage surprise".into(),
            Reward::Choice(_) => "Un objet au choix".into(),
        }
    }
}

/// « 3 / 16 » pour une tâche qui compte, vide pour une tâche « oui ou non ».
pub fn progress_text(task: &Task, have: u32) -> String {
    let need = task.target();
    if need > 1 {
        format!("{} / {need}", have.min(need))
    } else {
        String::new()
    }
}

/// Une tâche est-elle finie avec ce compteur ?
pub fn task_done(task: &Task, have: u32) -> bool {
    have >= task.target()
}

/// Les tâches qui restent à faire : (phrase, compteur, objectif).
pub fn remaining(quest: &Quest, progress: &Progress, namer: &Namer) -> Vec<(String, u32, u32)> {
    quest
        .tasks
        .iter()
        .enumerate()
        .filter_map(|(t, task)| {
            let have = progress.counter(&quest.id, t);
            (!task_done(task, have)).then(|| (namer.task(task), have, task.target()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_turns_keys_into_names() {
        assert_eq!(humanize("core:ruined_village"), "Ruined village");
        assert_eq!(humanize("deep_mine"), "Deep mine");
        assert_eq!(humanize("a/b"), "A b");
        assert_eq!(humanize(""), "");
    }

    #[test]
    fn paragraphs_skip_blank_ones() {
        assert_eq!(paragraphs("a\n\n\n\n b \n\nc"), vec!["a", "b", "c"]);
        assert!(paragraphs("  \n\n ").is_empty());
        assert_eq!(paragraphs("une seule"), vec!["une seule"]);
    }

    #[test]
    fn progress_text_counts_only_what_counts() {
        let many = Task::Break {
            block: "core:stone".into(),
            count: 16,
        };
        assert_eq!(progress_text(&many, 3), "3 / 16");
        // Un compteur ne déborde jamais de l'objectif.
        assert_eq!(progress_text(&many, 99), "16 / 16");
        assert_eq!(progress_text(&Task::Check, 0), "");
        assert!(task_done(&many, 16));
        assert!(!task_done(&many, 15));
        assert!(task_done(&Task::Check, 1));
    }

    #[test]
    fn status_labels_are_distinct() {
        let all = [
            Status::Locked,
            Status::Available,
            Status::Started,
            Status::Done,
            Status::Claimed,
        ];
        let labels: std::collections::HashSet<_> = all.iter().map(|s| status_label(*s)).collect();
        assert_eq!(labels.len(), all.len());
    }

    /// Les textes des tâches et des récompenses des quêtes livrées se lisent bien : jamais de
    /// clé technique (`core:`) ni de texte vide, et le reste à faire d'une quête neuve est complet.
    #[test]
    fn shipped_quests_have_readable_labels() {
        let (blocks, content, quests) = crate::quest_play::tests::shipped();
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let order = crate::modding::resolve_order(&assets);
        let mods = crate::modding::mod_dirs(&order);
        let mobs = crate::mobs::MobRegistry::load(&assets, &mods, &blocks).unwrap();
        let namer = Namer {
            items: &content.items,
            blocks: &blocks,
            creatures: &mobs,
        };
        let progress = Progress::default();
        for quest in &quests.quests {
            for task in &quest.tasks {
                let text = namer.task(task);
                assert!(
                    !text.trim().is_empty() && !text.contains("core:"),
                    "{}: {text}",
                    quest.id
                );
            }
            for reward in &quest.rewards {
                let text = namer.reward(reward);
                assert!(
                    !text.trim().is_empty() && !text.contains("core:"),
                    "{}: {text}",
                    quest.id
                );
            }
            assert_eq!(
                remaining(quest, &progress, &namer).len(),
                quest.tasks.len(),
                "{}",
                quest.id
            );
        }
    }
}
