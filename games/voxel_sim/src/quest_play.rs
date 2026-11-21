//! Le livre de quêtes dans le jeu vivant : charge les données et la progression du monde, observe le
//! joueur et les événements, annonce les quêtes finies et donne les récompenses.
//!
//! La logique est ailleurs (`quests.rs`, `quest_progress.rs`) ; ce module ne fait que la brancher.
//!
//! # Quand ça tourne
//!
//! - Au premier pas de simulation d'un monde, [`VoxelGame::update_quests`] charge `quests.ron` de
//!   core et des mods actifs (confrontés aux objets, blocs, créatures, biomes, dimensions et
//!   structures), puis la progression du monde (`quests.ron` à la racine du monde).
//! - Les événements (blocs cassés et posés, créatures tuées près du joueur, objets fabriqués) sont
//!   rassemblés à chaque pas ; un instantané du joueur (objets portés, dimension, biome, hauteur,
//!   niveau, palier) est pris **une fois par seconde**. Dès qu'il y a l'un ou l'autre, la progression
//!   se met à jour.
//! - Une quête qui se termine fait un message (et la cloche) ; les récompenses attendent d'être
//!   récupérées (touche des quêtes, ou `ctl quest claim`), sauf pour une quête `autoclaim`.
//! - La progression est écrite avec le monde (sauvegarde automatique et fermeture). Passer dans une
//!   autre dimension recharge le monde : la progression y est donc relue, et la tâche `Dimension` se
//!   valide à l'arrivée.
//!
//! Le contenu est facultatif : un fichier de mod invalide n'empêche pas de jouer, il est journalisé
//! et seuls les chapitres du jeu de base restent.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use engine_ecs::{EventReader, Events};
use engine_voxel::{BiomeRegistry, BlockRegistry, DimensionRegistry, ItemId, Rng64};
use serde::Deserialize;

use crate::events::{BlockBroken, BlockPlaced, EntityDied};
use crate::inventory::ItemStack;
use crate::quest_progress::{ClaimError, Event, Grant, Progress, Snapshot};
use crate::quests::{Catalog, ItemSpec, Quests, Task};

/// Les créatures tuées à moins de cette distance du joueur comptent pour lui.
const KILL_RADIUS: f32 = 16.0;
/// Le temps entre deux instantanés du joueur, en secondes.
const SCAN_PERIOD: f32 = 1.0;

/// Les noms que les quêtes peuvent citer, lus dans les fichiers de données.
#[derive(Debug, Clone, Default)]
pub struct Names {
    pub creatures: HashSet<String>,
    pub dimensions: HashSet<String>,
    pub biomes: HashSet<String>,
    pub structures: HashSet<String>,
}

#[derive(Deserialize)]
struct StructureNames {
    #[serde(default)]
    structures: Vec<StructureName>,
}

#[derive(Deserialize)]
struct StructureName {
    id: String,
}

impl Names {
    /// Lit les registres de créatures, biomes, dimensions et structures du jeu et des mods.
    pub fn load(assets: &Path, mods: &[(String, PathBuf)], blocks: &BlockRegistry) -> Result<Self> {
        let creatures = crate::mobs::MobRegistry::load(assets, mods, blocks)
            .context("créatures")?
            .keys()
            .into_iter()
            .collect();
        let biomes = BiomeRegistry::load_mods(blocks, mods).context("biomes")?;
        let mut dimensions: HashSet<String> = HashSet::new();
        dimensions.insert(engine_voxel::OVERWORLD.to_string());
        if let Ok(registry) = DimensionRegistry::load_mods(blocks, &biomes, mods) {
            dimensions.extend(registry.iter().map(|d| d.key.clone()));
        }
        let mut structures = HashSet::new();
        let mut dirs = vec![assets.join("core")];
        dirs.extend(mods.iter().map(|(_, dir)| dir.clone()));
        for dir in dirs {
            let path = dir.join("structures.ron");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let file: StructureNames = ron::Options::default()
                .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
                .from_str(&text)
                .with_context(|| format!("{} : RON invalide", path.display()))?;
            structures.extend(file.structures.into_iter().map(|s| s.id));
        }
        Ok(Self {
            creatures,
            dimensions,
            biomes: biomes.iter().map(|(_, b)| b.key.clone()).collect(),
            structures,
        })
    }
}

/// Charge `quests.ron` de core et des mods, confrontés au jeu. Un fichier de mod invalide est
/// journalisé et laissé de côté (le jeu de base reste) ; si même celui-là est faux, le livre est vide.
pub fn load_quests(
    assets: &Path,
    mods: &[(String, PathBuf)],
    blocks: &BlockRegistry,
    content: &crate::content::Content,
    tiers: usize,
) -> Arc<Quests> {
    let names = match Names::load(assets, mods, blocks) {
        Ok(names) => names,
        Err(err) => {
            log::error!("quêtes : noms du jeu illisibles : {err:#}");
            return Arc::new(Quests::default());
        }
    };
    let mut namespaces: HashSet<String> = mods.iter().map(|(ns, _)| ns.clone()).collect();
    namespaces.insert("core".into());
    let catalog = Catalog {
        items: &content.items,
        blocks,
        guide_chapters: content
            .guide
            .chapters
            .iter()
            .map(|c| c.id.clone())
            .collect(),
        creatures: names.creatures,
        dimensions: names.dimensions,
        biomes: names.biomes,
        structures: names.structures,
        tiers,
        namespaces,
    };
    let sources = match crate::content::sources(assets, mods, "quests.ron") {
        Ok(sources) => sources,
        Err(err) => {
            log::error!("quêtes : {err:#}");
            return Arc::new(Quests::default());
        }
    };
    match Quests::load(&sources, &catalog) {
        Ok(quests) => Arc::new(quests),
        Err(err) => {
            log::error!("quêtes : {err:#} : les quêtes des mods sont laissées de côté");
            let core: Vec<_> = sources.into_iter().filter(|s| s.0 == "core").collect();
            Arc::new(Quests::load(&core, &catalog).unwrap_or_else(|err| {
                log::error!("quêtes de core invalides : {err:#}");
                Quests::default()
            }))
        }
    }
}

/// Les curseurs sur les événements du jeu.
struct Readers {
    broken: EventReader,
    placed: EventReader,
    died: EventReader,
}

struct Loaded {
    quests: Arc<Quests>,
    /// Les signaux de mods que des quêtes attendent : les autres sont ignorés.
    signals: HashSet<String>,
    progress: Progress,
    readers: Readers,
}

/// L'état du livre dans un monde ouvert : rien tant que le monde n'a pas fait un pas.
#[derive(Default)]
pub struct QuestBook {
    loaded: Option<Loaded>,
    /// Le temps avant le prochain instantané du joueur.
    timer: f32,
    /// Ce qui s'est passé depuis la dernière mise à jour.
    events: Vec<Event>,
}

/// Au plus ce nombre de signaux en attente entre deux mises à jour (un script qui en envoie
/// trop ne fait pas grossir la file).
const MAX_PENDING_SIGNALS: usize = 256;

impl QuestBook {
    /// Un script de mod a envoyé ce signal (`mod:nom`) : il compte pour les tâches `Signal` qui
    /// l'attendent, au prochain passage. Un signal que nulle quête n'attend, ou déjà en attente,
    /// est ignoré.
    pub fn note_signal(&mut self, id: String) {
        if self
            .loaded
            .as_ref()
            .is_some_and(|l| !l.signals.contains(&id))
        {
            return;
        }
        let pending = self
            .events
            .iter()
            .any(|e| matches!(e, Event::Signal(s) if *s == id));
        if !pending && self.events.len() < MAX_PENDING_SIGNALS {
            self.events.push(Event::Signal(id));
        }
    }

    /// Un objet vient d'être fabriqué (grille d'artisanat ou livre de recettes).
    pub fn note_crafted(&mut self, item: ItemId, count: u32) {
        self.events.push(Event::Crafted(item, count));
    }

    pub fn progress(&self) -> Option<&Progress> {
        self.loaded.as_ref().map(|l| &l.progress)
    }

    pub fn progress_mut(&mut self) -> Result<&mut Progress> {
        self.loaded
            .as_mut()
            .map(|l| &mut l.progress)
            .context("quêtes pas chargées")
    }

    pub fn quests(&self) -> Option<Arc<Quests>> {
        self.loaded.as_ref().map(|l| l.quests.clone())
    }

    /// Les quêtes épinglées, en JSON (pour `ctl`).
    pub fn pinned_json(&self) -> String {
        let ids: Vec<String> = self
            .progress()
            .map(|p| {
                p.pinned
                    .iter()
                    .map(|id| crate::json_text::json_str(id))
                    .collect()
            })
            .unwrap_or_default();
        format!("[{}]", ids.join(","))
    }
}

/// L'identifiant d'un bloc sans ses états (`core:backpack_leather_placed[facing=west]` devient
/// `core:backpack_leather_placed`) : une quête cite le bloc, pas la manière dont il est tourné.
fn plain_key(key: &str) -> String {
    key.split('[').next().unwrap_or(key).to_string()
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Charge le livre et la progression si ce n'est pas fait.
    pub fn open_quests(&mut self) {
        if self.players.me_mut().quest_book.loaded.is_some() {
            return;
        }
        let tiers = crate::adventure_data::current().tiers.len();
        let quests = load_quests(
            &crate::assets_dir(),
            &self.mod_dirs,
            &self.registry,
            &self.content,
            tiers,
        );
        let progress = self
            .quest_dir()
            .map_or_else(Progress::default, |dir| Progress::load(&dir));
        let readers = Readers {
            broken: self.ecs.resource::<Events<BlockBroken>>().reader(),
            placed: self.ecs.resource::<Events<BlockPlaced>>().reader(),
            died: self.ecs.resource::<Events<EntityDied>>().reader(),
        };
        log::info!(
            "{} quêtes dans {} chapitres",
            quests.quests.len(),
            quests.chapters.len()
        );
        self.players.me_mut().quest_book.loaded = Some(Loaded {
            signals: quests.wanted_signals(),
            quests,
            progress,
            readers,
        });
        self.sync_quests_done();
    }

    /// Lit les événements du monde que les quêtes écoutent.
    fn collect_quest_events(&mut self) {
        let player_at = crate::player::body(&self.ecs, self.players.entity()).position;
        // Un événement compte pour le joueur qui l'a causé ; sans auteur (une machine, un script),
        // il compte pour tous.
        let me = self.players.local_id();
        let mine = |player: Option<crate::players::PlayerId>| player.is_none_or(|p| p == me);
        let book = &mut self.players.me_mut().quest_book;
        let Some(loaded) = book.loaded.as_mut() else {
            return;
        };
        let registry = &self.registry;
        let mobs = self.ecs.resource::<Arc<crate::mobs::MobRegistry>>().clone();
        let events = &mut book.events;
        for e in loaded
            .readers
            .broken
            .read(&self.ecs.resource::<Events<BlockBroken>>())
            .filter(|e| mine(e.player))
        {
            events.push(Event::Broken(plain_key(&registry.def(e.block).key)));
        }
        for e in loaded
            .readers
            .placed
            .read(&self.ecs.resource::<Events<BlockPlaced>>())
            .filter(|e| mine(e.player))
        {
            events.push(Event::Placed(plain_key(&registry.def(e.block).key)));
        }
        for e in loaded
            .readers
            .died
            .read(&self.ecs.resource::<Events<EntityDied>>())
            .filter(|e| mine(e.killer))
        {
            if e.position.distance(player_at) <= KILL_RADIUS {
                let hostile = mobs.by_key(&e.kind).is_some_and(|d| d.hostile);
                events.push(Event::Killed {
                    kind: e.kind.clone(),
                    hostile,
                });
            }
        }
    }

    /// À chaque pas de simulation : rassemble les événements, et met la progression à jour dès
    /// qu'il y en a ou toutes les secondes.
    pub fn update_quests(&mut self, dt: f32) {
        self.open_quests();
        self.collect_quest_events();
        self.players.me_mut().quest_book.timer -= dt;
        let due = self.players.me_mut().quest_book.timer <= 0.0;
        if !due && self.players.me_mut().quest_book.events.is_empty() {
            return;
        }
        if due {
            self.players.me_mut().quest_book.timer = SCAN_PERIOD;
        }
        self.refresh_quests();
    }

    /// Le dossier où vit `quests.ron` du joueur local : la racine du monde (voir [`crate::players::PlayerState::quest_dir`]).
    fn quest_dir(&self) -> Option<PathBuf> {
        self.dims
            .meta_store
            .clone()
            .or_else(|| self.store.clone())
            .map(|s| self.players.me().quest_dir(s.dir(), true))
    }

    /// Écrit la progression dans le monde (si quelque chose a changé).
    pub fn save_quests(&mut self) {
        let Some(dir) = self.quest_dir() else {
            return;
        };
        let Some(loaded) = self.players.me_mut().quest_book.loaded.as_mut() else {
            return;
        };
        if !loaded.progress.dirty() {
            return;
        }
        if let Err(err) = loaded.progress.save(&dir) {
            log::error!("sauvegarde des quêtes : {err:#}");
        }
    }

    /// Dit aux scripts quelles quêtes sont finies (`quest_done`).
    pub fn sync_quests_done(&mut self) {
        let done: Vec<String> = self
            .players
            .me_mut()
            .quest_book
            .progress()
            .map(|p| p.done_ids().map(str::to_string).collect())
            .unwrap_or_default();
        self.mods.set_quests_done(done);
    }

    /// L'instantané du joueur : ce qu'il porte, où il est.
    pub fn quest_snapshot(&self, wanted_structures: &[&str]) -> Snapshot {
        let mut items: HashMap<ItemId, u32> = HashMap::new();
        let inventory = &self.players.me().inventory;
        for stack in inventory
            .slots
            .iter()
            .chain(inventory.armor.iter())
            .chain(std::iter::once(&inventory.cursor))
            .flatten()
        {
            *items.entry(stack.item).or_default() += stack.count;
        }
        let feet = crate::player::body(&self.ecs, self.players.entity()).position;
        let (x, z) = (feet.x.floor() as i32, feet.z.floor() as i32);
        let streamer = self.streamer();
        let generator = streamer.generator();
        let biome = generator.biome(generator.column(x, z).biome).key.clone();
        let mut structures = Vec::new();
        if let Some(set) = generator.structures() {
            for key in wanted_structures {
                let Some(def) = set.defs().iter().position(|d| d.key == *key) else {
                    continue;
                };
                let radius = set.defs()[def].radius;
                if generator
                    .structure_sites_near(x, z, radius)
                    .iter()
                    .any(|site| site.def == def)
                {
                    structures.push((*key).to_string());
                }
            }
        }
        Snapshot {
            items,
            dimension: self.dims.current.key.clone(),
            biome,
            structures,
            y: feet.y,
            level: self.players.me().experience.level().0,
            tier: self.players.me().adventure.progress.unlocked,
        }
    }

    /// Un message pour les quêtes qui viennent de se terminer ; celles qui se récupèrent
    /// d'elles-mêmes (`autoclaim`) donnent tout de suite leurs récompenses.
    pub fn announce_quests(&mut self, quests: &Arc<Quests>, finished: &[usize]) {
        let Some(&first) = finished.first() else {
            return;
        };
        self.sync_quests_done();
        // Les scripts abonnés l'apprennent (une seule fois : une quête ne se termine qu'une fois).
        for &index in finished {
            let quest = quests.quests[index].id.clone();
            let actions = self
                .mods
                .dispatch(&engine_mods::ModEvent::QuestCompleted { quest });
            self.apply_machine_actions(actions);
        }
        let mut text = format!("Quête terminée : {}", quests.quests[first].title);
        if finished.len() > 1 {
            text.push_str(&format!(" (+{})", finished.len() - 1));
        }
        for &index in finished {
            let quest = &quests.quests[index];
            if quest.autoclaim && self.claim_quest(index).is_ok() && finished.len() == 1 {
                text.push_str(" : récompenses reçues");
            }
        }
        self.toast(&text);
        self.bell();
    }

    /// Met la progression à jour avec ce qui s'est passé, et annonce les quêtes finies.
    pub fn refresh_quests(&mut self) {
        let Some(quests) = self
            .players
            .me_mut()
            .quest_book
            .loaded
            .as_ref()
            .map(|l| l.quests.clone())
        else {
            return;
        };
        if quests.is_empty() {
            self.players.me_mut().quest_book.events.clear();
            return;
        }
        let snapshot = self.quest_snapshot(&quests.wanted_structures());
        let events = std::mem::take(&mut self.players.me_mut().quest_book.events);
        let finished = match self.players.me_mut().quest_book.loaded.as_mut() {
            Some(loaded) => loaded.progress.update(&quests, &snapshot, &events),
            None => return,
        };
        self.announce_quests(&quests, &finished);
    }

    /// Remet les objets d'une tâche `consume` : prend ce qu'il faut dans l'inventaire (barre et
    /// réserve) et avance le compteur. Renvoie combien d'objets ont été pris.
    pub fn hand_in_quest(&mut self, index: usize, task: usize) -> u32 {
        let Some(quests) = self.players.me_mut().quest_book.quests() else {
            return 0;
        };
        let Some(Task::Item {
            spec: spec @ (ItemSpec::Item(_) | ItemSpec::Group { .. }),
            consume: true,
            ..
        }) = quests.quests[index].tasks.get(task)
        else {
            return 0;
        };
        // Ce que le joueur a sur lui (barre et réserve : seules ces cases se vident).
        let have: u32 = self
            .players
            .me_mut()
            .inventory
            .slots
            .iter()
            .flatten()
            .filter(|s| spec.matches(s.item))
            .map(|s| s.count)
            .sum();
        let Ok(progress) = self.players.me_mut().quest_book.progress_mut() else {
            return 0;
        };
        let mut left = progress.hand_in(&quests, index, task, have);
        let taken = left;
        let members: Vec<ItemId> = match spec {
            ItemSpec::Item(id) => vec![*id],
            ItemSpec::Group { members, .. } => members.clone(),
        };
        for item in members {
            let n = self.players.me_mut().inventory.count(item).min(left);
            if n > 0 && self.players.me_mut().inventory.remove(item, n) {
                left -= n;
            }
        }
        self.refresh_quests();
        taken
    }

    /// Récupère les récompenses d'une quête finie : objets (ce qui ne tient pas tombe aux pieds du
    /// joueur) et expérience. Renvoie ce qui a été donné, en une ligne.
    pub fn claim_quest(&mut self, index: usize) -> Result<String, ClaimError> {
        let quests = self
            .players
            .me_mut()
            .quest_book
            .loaded
            .as_ref()
            .map(|l| l.quests.clone())
            .ok_or(ClaimError::Unknown)?;
        let mut rng = Rng64::new(self.next_random());
        let grants = {
            let loaded = self
                .players
                .me_mut()
                .quest_book
                .loaded
                .as_mut()
                .ok_or(ClaimError::Unknown)?;
            loaded
                .progress
                .claim(&quests, index, &mut |n| rng.below(n))?
        };
        let mut parts = Vec::new();
        for grant in grants {
            match grant {
                Grant::Item(item, count) => {
                    let name = self.content.items.get(item).name.clone();
                    parts.push(format!("{count} × {name}"));
                    self.give_or_drop(ItemStack::new(item, count));
                }
                Grant::Xp(points) => {
                    parts.push(format!("{points} points d'expérience"));
                    self.grant_xp(points);
                }
            }
        }
        Ok(parts.join(", "))
    }

    /// Récupère les récompenses de toutes les quêtes finies, sauf celles qui demandent un choix.
    /// Renvoie le nombre de quêtes récupérées.
    pub fn claim_all_quests(&mut self) -> usize {
        let pending = self
            .players
            .me_mut()
            .quest_book
            .loaded
            .as_ref()
            .map(|l| l.progress.pending(&l.quests))
            .unwrap_or_default();
        pending
            .into_iter()
            .filter(|&index| self.claim_quest(index).is_ok())
            .count()
    }
}

#[cfg(any(test, feature = "testing"))]
#[allow(unused_imports)]
pub mod tests {
    use super::*;
    use crate::quest_progress::{Progress, Snapshot, Status};

    #[test]
    fn block_states_are_dropped_from_event_keys() {
        assert_eq!(
            plain_key("core:backpack_leather_placed[facing=west]"),
            "core:backpack_leather_placed"
        );
        assert_eq!(plain_key("core:torch"), "core:torch");
    }

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    /// Le jeu de base et tous les mods, comme au lancement.
    pub fn shipped() -> (Arc<BlockRegistry>, crate::content::Content, Arc<Quests>) {
        let assets = assets();
        let order = crate::modding::resolve_order(&assets);
        let mods = crate::modding::mod_dirs(&order);
        let blocks = Arc::new(crate::test_registry());
        let content = crate::content::Content::load(&blocks, &assets, &mods).unwrap();
        let names = Names::load(&assets, &mods, &blocks).unwrap();
        let mut namespaces: HashSet<String> = mods.iter().map(|(ns, _)| ns.clone()).collect();
        namespaces.insert("core".into());
        let tiers = crate::adventure_data::AdventureData::load(&assets, &mods)
            .unwrap()
            .tiers
            .len();
        let catalog = Catalog {
            items: &content.items,
            blocks: &blocks,
            guide_chapters: content
                .guide
                .chapters
                .iter()
                .map(|c| c.id.clone())
                .collect(),
            creatures: names.creatures,
            dimensions: names.dimensions,
            biomes: names.biomes,
            structures: names.structures,
            tiers,
            namespaces,
        };
        let sources = crate::content::sources(&assets, &mods, "quests.ron").unwrap();
        let quests = Quests::load(&sources, &catalog)
            .unwrap_or_else(|err| panic!("quests.ron livrés invalides : {err:#}"));
        (blocks, content, Arc::new(quests))
    }

    /// Chaque `quests.ron` livré (core et mods) se charge : tout ce qu'il cite existe (objets, blocs,
    /// créatures, biomes, dimensions, structures, paliers, chapitres du guide, tables de tirage),
    /// sans doublon ni cycle.
    #[test]
    fn the_shipped_quests_load_and_cite_only_real_things() {
        let (_, content, quests) = shipped();
        assert!(
            !quests.is_empty(),
            "le jeu de base livre au moins un chapitre"
        );
        assert!(quests.chapter("core:first_steps").is_some());
        // Les textes citent des touches qui existent.
        let keys = crate::settings::Keys::default();
        for quest in &quests.quests {
            let text = crate::guide::expand_keys(&quest.text, &keys);
            assert!(!text.contains("{touche:"), "{} : {text}", quest.id);
            assert!(!quest.title.trim().is_empty(), "{}", quest.id);
            // Une tâche d'objet a bien une icône à montrer.
            for task in &quest.tasks {
                if let Task::Item { spec, .. } = task {
                    assert!(spec.icon().is_some(), "{}", quest.id);
                }
            }
        }
        // Chaque quête a des coordonnées différentes de ses voisines du chapitre (pas de pile).
        for chapter in &quests.chapters {
            let mut seen = HashSet::new();
            for &q in &chapter.quests {
                let quest = &quests.quests[q];
                assert!(
                    seen.insert(((quest.x * 100.0) as i32, (quest.y * 100.0) as i32)),
                    "{} : deux quêtes au même endroit",
                    quest.id
                );
            }
        }
        let _ = content;
    }

    /// Tous les chapitres livrés se terminent depuis rien, et chaque récompense se récupère (avec un
    /// choix fait pour celles qui en demandent un) : aucune quête n'est un cul-de-sac.
    #[test]
    fn every_shipped_quest_can_be_finished_and_claimed() {
        use crate::quests::Reward;
        let (_, _, quests) = shipped();
        let mut progress = Progress::default();
        for index in 0..quests.quests.len() {
            progress.force_done(&quests, index);
            for (r, reward) in quests.quests[index].rewards.iter().enumerate() {
                if matches!(reward, Reward::Choice(_)) {
                    assert!(progress.choose(&quests, index, r, 0));
                }
            }
        }
        for c in 0..quests.chapters.len() {
            let (done, total) = progress.chapter_progress(&quests, c);
            assert_eq!(done, total, "{}", quests.chapters[c].id);
        }
        for index in progress.pending(&quests) {
            assert!(
                progress.claim(&quests, index, &mut |_| 0).is_ok(),
                "{}",
                quests.quests[index].id
            );
        }
        assert!(progress.pending(&quests).is_empty());
        // Les quêtes cachées se montrent une fois leurs dépendances finies.
        for index in 0..quests.quests.len() {
            assert!(progress.visible(&quests, index));
        }
    }

    /// Une progression neuve termine, dans l'ordre, tout un chapitre dont on force les tâches ; et
    /// chaque quête du jeu de base finit bien par se débloquer depuis rien.
    #[test]
    fn a_whole_shipped_chapter_can_be_completed() {
        let (_, _, quests) = shipped();
        let chapter = quests
            .chapters
            .iter()
            .position(|c| c.id == "core:first_steps")
            .unwrap();
        let mut progress = Progress::default();
        // Rien n'est fait, rien n'est récupérable, et la première quête est disponible.
        assert!(progress.pending(&quests).is_empty());
        let first = quests.chapters[chapter].quests[0];
        assert_eq!(progress.status(&quests, first), Status::Available);
        for &q in &quests.chapters[chapter].quests.clone() {
            progress.force_done(&quests, q);
        }
        let (done, total) = progress.chapter_progress(&quests, chapter);
        assert_eq!(done, total);
        // Les récompenses se récupèrent une fois chacune, sans choix en attente.
        let mut claimed = 0;
        for q in progress.pending(&quests) {
            assert!(progress.claim(&quests, q, &mut |_| 0).is_ok());
            claimed += 1;
        }
        assert!(claimed > 0);
        assert!(progress.pending(&quests).is_empty());
        let _ = Snapshot::default();
    }

    /// Chaque signal attendu par une quête livrée est envoyé par le script de son mod
    /// (`quest_signal("nom")` dans `mods/<mod>/script/src`) : aucune quête ne reste sans émetteur.
    #[test]
    fn every_shipped_signal_is_sent_by_its_mod_script() {
        let (_, _, quests) = shipped();
        let signals = quests.wanted_signals();
        assert!(signals.contains("magie:spell_cast"), "{signals:?}");
        for id in signals {
            let (ns, name) = id.split_once(':').unwrap();
            let src = assets().join(format!("../mods/{ns}/script/src"));
            let sent = std::fs::read_dir(&src)
                .unwrap_or_else(|_| panic!("{} : le script de {ns}", src.display()))
                .flatten()
                .filter_map(|f| std::fs::read_to_string(f.path()).ok())
                .any(|code| code.contains(&format!("quest_signal(\"{name}\")")));
            assert!(
                sent,
                "{id} : aucun quest_signal({name:?}) dans le script de {ns}"
            );
        }
    }

    /// Les signaux en attente ne s'empilent pas : un doublon est ignoré, et la file a un plafond.
    #[test]
    fn pending_signals_are_deduplicated_and_bounded() {
        let mut book = QuestBook::default();
        book.note_signal("magie:spell_cast".into());
        book.note_signal("magie:spell_cast".into());
        assert_eq!(book.events.len(), 1);
        for n in 0..1000 {
            book.note_signal(format!("magie:s{n}"));
        }
        assert_eq!(book.events.len(), super::MAX_PENDING_SIGNALS);
    }
}
