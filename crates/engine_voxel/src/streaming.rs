//! Le monde infini : charger, éclairer et mailler les chunks autour du joueur, en tâche de fond.
//!
//! # Qui fait quoi
//!
//! - Le **thread principal** possède le [`World`] et l'état de chaque chunk. Il décide
//!   quoi charger, lance des tâches, et reçoit leurs résultats par un canal.
//! - Les **threads de fond** (un pool `rayon` à nous) génèrent les chunks, calculent
//!   la lumière et les maillages. Ils ne touchent jamais au `World`.
//!
//! Le pool laisse deux cœurs libres : un pour le thread principal (rendu, entrées),
//! un pour le système et le pilote graphique. Avec un thread de fond par cœur,
//! le thread principal devait parfois attendre son tour : des images à 40 ms.
//!
//! # Trois étapes, trois rayons
//!
//! 1. **Génération**, chunk par chunk, jusqu'à `rayon de vue + 3`.
//! 2. **Lumière**, colonne par colonne (les 5 chunks d'une verticale) : la lumière du
//!    ciel tombe du haut du monde, il faut donc toute la colonne ; et elle déborde de
//!    15 blocs sur les côtés, il faut donc les 8 colonnes voisines générées.
//!    Chaque colonne voisine (diagonales comprises) d'une colonne visible est éclairée.
//! 3. **Maillage**, chunk par chunk : la lumière douce et l'AO lisent un bloc au-delà
//!    du bord, diagonales comprises, il faut donc les colonnes voisines éclairées.
//!    Jusqu'au rayon de vue.
//!
//! # Priorités, file des maillages et envois (lot 6)
//!
//! - **Ordre de chargement** ([`StreamingConfig::column_order`], [`StreamingConfig::view_bias`]) : on
//!   complète des colonnes entières, de la plus proche à la plus lointaine (la lumière et le maillage
//!   attendent une colonne et ses huit voisines), avec un petit biais pour ce que la caméra regarde
//!   ([`ChunkStreamer::update_view`]). Le biais est borné : aucune colonne n'est affamée, et le sol
//!   sous les pieds n'en a jamais.
//! - **Maillages en attente** : le plus récent de chaque chunk seulement, le plus proche de la caméra
//!   d'abord ; les maillages d'un bloc modifié passent en tête et hors budget. Un chunk modifié
//!   pendant qu'on le maille n'a qu'un remaillage de plus (au lieu d'une tâche par modification, dont
//!   chaque résultat était jeté : un chunk qui changeait à chaque tick ne s'actualisait jamais).
//! - **Envois** ([`ChunkStreamer::next_event_budgeted`]) : sous un budget d'octets et de temps commun
//!   avec le terrain lointain (voir `transfer.rs`) ; l'ancien maillage d'un chunk reste affiché jusqu'à
//!   ce que le nouveau soit envoyé, et une libération en attente est annulée si le chunk revient.
//!
//! # Pourquoi pas de `Mutex` sur le monde ?
//!
//! Un `Mutex<World>` obligerait chaque thread à attendre son tour pour lire un bloc :
//! les threads passeraient leur temps à se bloquer les uns les autres. Ici, chacun
//! travaille sur ses propres données :
//! - une tâche de génération ne reçoit qu'une position et un `Arc<TerrainGenerator>`
//!   (lecture seule), et renvoie un `Chunk` tout neuf ;
//! - une tâche de lumière ou de maillage reçoit des `Arc<Chunk>` en lecture seule
//!   ([`LightRegion`], [`Neighbourhood`]). Le thread principal peut continuer à lire
//!   ces mêmes chunks en même temps, et s'il les modifie, `Arc::make_mut` lui en fait
//!   une copie : la tâche garde l'ancienne version, intacte.
//!
//! # `Send`, `Sync` et canaux
//!
//! - `Send` : une valeur peut être *déplacée* vers un autre thread. `Chunk` (un simple
//!   tableau) l'est ; un `Rc` ne l'est pas, son compteur n'est pas atomique.
//! - `Sync` : une valeur peut être *partagée* (`&T`) entre threads. `TerrainGenerator`
//!   l'est car personne ne le modifie. `Arc<T>` n'est `Send` que si `T: Send + Sync`.
//! - Le compilateur vérifie ces règles : `rayon::spawn` exige une closure `Send`, donc
//!   oublier un `Arc` ou partager une donnée mutable ne compile tout simplement pas.
//! - Un canal (`crossbeam_channel`) transfère la *propriété* du résultat : après
//!   `send(chunk)`, le thread de fond ne peut plus y toucher (il a été « moved »),
//!   et le thread principal en devient l'unique propriétaire. Aucune copie, aucun verrou.

use std::collections::VecDeque;
use std::collections::hash_map::Entry;

// Des tables rapides (FxHash, voir `world.rs`) pour les positions de chunks.
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use glam::{IVec2, IVec3, Vec2, Vec3};

use crate::block::BlockId;
use crate::chunk::{BlockEntity, Chunk};
use crate::generator::TerrainGenerator;
use crate::light::{ColumnLight, LightRegion, apply_column_light, update_light};
use crate::mesh::{ChunkMesh, Mesher, Neighbourhood};
use crate::pipeline_timing::{PipelineReport, PipelineTimings};
use crate::registry::BlockRegistry;
use crate::save::{ChunkToSave, SaveReport, SaveStore, UnknownBlocks};
use crate::transfer::TransferBudget;
use crate::world::World;

/// Les réglages du chargement.
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Rayon (horizontal, en chunks) dans lequel les chunks sont maillés et dessinés.
    pub view_radius: i32,
    /// Hystérésis : un chunk n'est déchargé qu'à `view_radius + 3 + unload_margin`.
    /// Sans marge, faire un pas d'avant en arrière sur la frontière d'un chunk
    /// chargerait et déchargerait sans cesse la même rangée.
    pub unload_margin: i32,
    /// Nombre de threads de fond.
    pub worker_threads: usize,
    /// Nombre maximum de tâches lancées et pas encore terminées.
    /// Garder la file courte permet de suivre le joueur : si on lançait tout d'un coup,
    /// les chunks devenus lointains seraient quand même calculés avant les proches.
    pub max_jobs_in_flight: usize,
    pub mesher: Mesher,
    /// Dessiner opaques les feuilles au cœur des arbres, et retirer les faces contre elles
    /// (moins de triangles, même image).
    pub leaf_culling: bool,
    /// Compléter les **colonnes** de chunks de la plus proche à la plus lointaine : la lumière et le
    /// maillage attendent une colonne entière (et ses huit voisines), donc trier chaque chunk par
    /// distance 3D les finit dans le désordre (le haut d'une colonne proche passe après le bas
    /// d'une colonne plus loin). `false` : l'ancien tri par distance 3D (pour comparer).
    pub column_order: bool,
    /// Préférence (de 0 à 1) pour ce que la caméra regarde : la distance des colonnes derrière elle
    /// compte jusqu'à `1 + view_bias` fois plus. Bornée : une colonne de derrière est chargée avant une
    /// colonne de devant plus de `√(1 + view_bias)` fois plus loin, donc rien n'est affamé. Les
    /// colonnes à moins de 3 chunks (le sol sous les pieds, les collisions) n'ont jamais de biais.
    pub view_bias: f32,
    /// Les chunks **sombres** (aucune lumière du ciel : roche et grottes fermées) ne sont maillés qu'à
    /// moins de ce nombre de couches sous celle du joueur ; ils le sont quand il descend. Tout ce qui
    /// voit le ciel (surface, montagnes, entrées de grottes, fonds peu profonds) est toujours maillé.
    /// Les blocs, eux, restent chargés en entier : la lumière d'une colonne les demande tous.
    /// `None` : tout mailler.
    pub mesh_depth: Option<i32>,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        let worker_threads = cores.saturating_sub(2).max(1);
        Self {
            view_radius: 16,
            unload_margin: 2,
            worker_threads,
            // Deux tâches par thread : aucun cœur n'attend pendant qu'on en relance.
            max_jobs_in_flight: worker_threads * 2,
            mesher: Mesher::Binary,
            leaf_culling: true,
            column_order: true,
            view_bias: 0.6,
            mesh_depth: Some(3),
        }
    }
}

/// Ce que le jeu doit faire côté GPU.
#[derive(Debug)]
pub enum StreamEvent {
    /// Un maillage neuf (ou qui remplace l'ancien) pour ce chunk.
    Mesh(IVec3, ChunkMesh),
    /// Ce chunk est déchargé : libérer son maillage s'il en a un.
    Unload(IVec3),
}

/// Des compteurs pour le titre de la fenêtre et les logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Chunks générés et gardés en mémoire (vides compris).
    pub loaded: usize,
    /// Colonnes dont la lumière est calculée.
    pub lit_columns: usize,
    /// Chunks qui attendent d'être générés.
    pub queued: usize,
    /// Tâches en cours sur les threads de fond.
    pub in_flight: usize,
    /// Maillages prêts, pas encore envoyés au GPU.
    pub meshes_waiting: usize,
    /// Octets de ces maillages (ce que le GPU devra recevoir).
    pub bytes_waiting: usize,
    /// Libérations de maillages en attente.
    pub unloads_waiting: usize,
    /// Colonnes ou chunks prêts à être éclairés ou maillés, pas encore lancés.
    pub candidates: usize,
    /// Tâches de fond terminées depuis le lancement.
    pub jobs_done: u64,
    /// Parmi elles, celles dont le résultat a été jeté (chunk déchargé entre-temps,
    /// ou bloc modifié à côté) : du travail perdu.
    pub jobs_discarded: u64,
}

impl StreamStats {
    /// Tout le travail de chargement en attente ou en cours : de quoi décider si le terrain lointain
    /// doit se faire discret (voir `far::far_allowance`).
    pub fn backlog(&self) -> usize {
        self.queued + self.in_flight + self.meshes_waiting + self.candidates
    }
}

/// Où en est un chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Une tâche de génération tourne.
    Generating,
    /// Les blocs sont prêts (le chunk est dans le `World`, sauf s'il est vide).
    Generated(MeshState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshState {
    Missing,
    /// Une tâche de maillage tourne ; son numéro (« ticket ») permet de reconnaître
    /// et de jeter un résultat périmé.
    Meshing(u32),
    Done,
}

/// Où en est une colonne de chunks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ColumnInfo {
    /// Combien de ses chunks sont générés (sur les couches du monde, `World::vertical`).
    generated: u8,
    light: LightState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum LightState {
    #[default]
    Missing,
    Lighting(u32),
    Done,
}

/// Ce que renvoient les threads de fond.
enum JobResult {
    Generated {
        pos: IVec3,
        chunk: Option<Chunk>,
        /// Relu depuis la sauvegarde (avec ses blocs inconnus) plutôt que généré.
        saved: Option<UnknownBlocks>,
    },
    Lit {
        column: IVec2,
        ticket: u32,
        light: ColumnLight,
    },
    Meshed {
        pos: IVec3,
        ticket: u32,
        mesh: ChunkMesh,
    },
}

/// Charge, éclaire, maille et décharge les chunks autour d'un point (le joueur).
///
/// À appeler à chaque image : [`ChunkStreamer::update`], puis [`ChunkStreamer::next_event`]
/// jusqu'à épuisement du budget d'envois GPU.
pub struct ChunkStreamer {
    config: StreamingConfig,
    /// Quand le streamer est détruit, le pool l'est aussi : ses threads finissent
    /// leur tâche en cours puis s'arrêtent.
    pool: rayon::ThreadPool,
    generator: Arc<TerrainGenerator>,
    world: World,
    slots: HashMap<IVec3, Slot>,
    columns: HashMap<IVec2, ColumnInfo>,
    /// Chunks à générer, du plus proche au plus lointain.
    load_queue: VecDeque<IVec3>,
    /// Colonnes et chunks peut-être prêts (on vérifie au moment de lancer).
    light_candidates: HashSet<IVec2>,
    mesh_candidates: HashSet<IVec3>,
    /// Le chunk du joueur lors du dernier tri.
    centre: Option<IVec3>,
    /// Les chunks chargés sans aucune lumière du ciel (voir [`StreamingConfig::mesh_depth`]).
    dark: HashSet<IVec3>,
    /// La couche du joueur au dernier rafraîchissement.
    centre_layer: Option<i32>,
    next_ticket: u32,
    in_flight: usize,
    /// Compteurs de [`StreamStats::jobs_done`] et [`StreamStats::jobs_discarded`].
    jobs_done: u64,
    jobs_discarded: u64,
    // Chaque tâche reçoit un clone du `Sender` ; le `Receiver` reste ici.
    sender: Sender<JobResult>,
    receiver: Receiver<JobResult>,
    /// Les maillages d'un bloc modifié : envoyés tout de suite, hors budget (le joueur attend de voir
    /// son coup), en tête de file.
    urgent: VecDeque<StreamEvent>,
    /// Les maillages arrivés des threads de fond, pas encore envoyés : le plus récent de chaque chunk
    /// seulement (un remaillage rend l'ancien inutile), et le plus proche de la caméra part en premier.
    waiting: HashMap<IVec3, ChunkMesh>,
    /// Les libérations de maillages, étalées sur plusieurs images (elles sont sans urgence).
    unloads: VecDeque<IVec3>,
    unloads_pending: HashSet<IVec3>,
    /// Chunks modifiés pendant qu'un maillage tournait : un seul remaillage de plus quand il revient,
    /// au lieu d'une tâche par modification dont chaque résultat serait jeté (un chunk qui change à
    /// chaque tick ne s'actualisait jamais).
    meshing_dirty: HashSet<IVec3>,
    /// La direction horizontale de la caméra (unitaire), et celle du dernier tri de la file.
    view: Option<Vec2>,
    sorted_view: Option<Vec2>,
    /// Chunks chargés modifiés par le joueur depuis leur génération.
    edited: HashSet<IVec3>,
    /// Chunks modifiés puis déchargés avant d'être sauvegardés : on les garde en
    /// mémoire et on les rend au lieu de les régénérer, sinon les modifications
    /// seraient perdues. Une fois écrits sur disque, on peut les oublier.
    saved_edits: HashMap<IVec3, Arc<Chunk>>,
    /// La sauvegarde du monde, si elle est activée.
    store: Option<Arc<SaveStore>>,
    /// Le résumé des chunks modifiés, pour le terrain lointain (voir `edited_surface.rs`).
    edited_surfaces: Arc<crate::edited_surface::EditedSurfaces>,
    /// Chunks modifiés depuis la dernière sauvegarde.
    dirty: HashSet<IVec3>,
    /// Blocs de mods absents, par chunk (voir `save.rs`).
    unknown: HashMap<IVec3, UnknownBlocks>,
    /// Chunks à remailler en tâche de fond à la prochaine image (modifiés par des machines).
    remesh_pending: HashSet<IVec3>,
    /// Positions des blocs actifs des chunks arrivés depuis le dernier appel à
    /// [`ChunkStreamer::take_loaded_block_entities`] : les machines doivent les relancer.
    loaded_block_entities: Vec<IVec3>,
    /// Les chunks générés pour la première fois (ni relus de la sauvegarde, ni vides) depuis le
    /// dernier appel à [`ChunkStreamer::take_fresh_chunks`] : les mods peuvent les décorer.
    fresh_chunks: Vec<IVec3>,
    /// La sauvegarde en cours en tâche de fond : les chunks qu'elle écrit et le canal
    /// par lequel elle rendra son résultat.
    saving: Option<(Vec<IVec3>, Receiver<anyhow::Result<SaveReport>>)>,
    /// Les délais demande → génération → lumière → maillage → envoi, si le suivi est activé
    /// (voir `pipeline_timing.rs`).
    timings: Option<PipelineTimings>,
}

impl ChunkStreamer {
    pub fn new(generator: TerrainGenerator, config: StreamingConfig) -> Self {
        // `unbounded` : `send` ne bloque jamais le thread de fond. La taille reste
        // bornée de fait par `max_jobs_in_flight`.
        let (sender, receiver) = crossbeam_channel::unbounded();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.worker_threads)
            .thread_name(|i| format!("chunks-{i}"))
            .build()
            .expect("impossible de créer les threads de fond");
        Self {
            config,
            pool,
            // Le monde partage le registre du générateur (avant que `generator` soit déplacé).
            world: {
                let mut world = World::with_registry(generator.registry().clone());
                world.set_biomes(generator.biomes().clone());
                world.set_fluids(std::sync::Arc::new(crate::fluids::FluidRegistry::builtin()));
                world.set_vertical(generator.chunk_range());
                world
            },
            generator: Arc::new(generator),
            slots: HashMap::default(),
            columns: HashMap::default(),
            load_queue: VecDeque::new(),
            light_candidates: HashSet::default(),
            mesh_candidates: HashSet::default(),
            centre: None,
            dark: HashSet::default(),
            centre_layer: None,
            next_ticket: 0,
            in_flight: 0,
            jobs_done: 0,
            jobs_discarded: 0,
            sender,
            receiver,
            urgent: VecDeque::new(),
            waiting: HashMap::default(),
            unloads: VecDeque::new(),
            unloads_pending: HashSet::default(),
            meshing_dirty: HashSet::default(),
            view: None,
            sorted_view: None,
            edited: HashSet::default(),
            saved_edits: HashMap::default(),
            store: None,
            edited_surfaces: Arc::new(crate::edited_surface::EditedSurfaces::new()),
            dirty: HashSet::default(),
            unknown: HashMap::default(),
            remesh_pending: HashSet::default(),
            loaded_block_entities: Vec::new(),
            fresh_chunks: Vec::new(),
            saving: None,
            timings: None,
        }
    }

    /// Active ou éteint le suivi des délais du pipeline des chunks ; allumer repart de zéro. Éteint
    /// (par défaut), il ne coûte qu'un test par étape.
    pub fn set_pipeline_timing(&mut self, enabled: bool) {
        self.timings = enabled.then(PipelineTimings::default);
    }

    /// Le bilan des délais depuis l'activation, ou `None` si le suivi est éteint.
    pub fn pipeline_report(&self) -> Option<PipelineReport> {
        self.timings.as_ref().map(PipelineTimings::report)
    }

    /// Le jeu a envoyé le maillage de ce chunk au GPU : dernière étape du suivi des délais.
    pub fn note_uploaded(&mut self, chunk: IVec3) {
        if let Some(timings) = &mut self.timings {
            timings.uploaded(chunk);
        }
    }

    /// Active la sauvegarde : les chunks déjà sauvegardés seront relus au lieu d'être
    /// générés, et [`ChunkStreamer::begin_save`] écrira les chunks modifiés.
    pub fn with_save(mut self, store: Arc<SaveStore>) -> Self {
        self.edited_surfaces.attach_store(store.clone());
        self.store = Some(store);
        self
    }

    /// Vrai s'il y a des modifications pas encore écrites sur disque.
    pub fn has_unsaved_changes(&self) -> bool {
        !self.dirty.is_empty() || self.saving.is_some()
    }

    /// Lance une sauvegarde en tâche de fond. Ne fait rien (et renvoie `false`) sans
    /// sauvegarde activée, sans modification, ou si une sauvegarde tourne déjà.
    ///
    /// Le thread principal ne fait que rassembler des `Arc` des chunks modifiés : aucune
    /// copie de blocs, aucun verrou. Le joueur peut continuer à modifier ces chunks pendant
    /// l'écriture : `Arc::make_mut` lui donnera une copie, la sauvegarde garde la version
    /// du moment où elle a commencé (un « instantané »).
    pub fn begin_save(&mut self) -> bool {
        let Some(store) = self.store.clone() else {
            return false;
        };
        if self.saving.is_some() || self.dirty.is_empty() {
            return false;
        }
        let chunks = self.snapshot_dirty();
        let positions: Vec<IVec3> = chunks.iter().map(|c| c.pos).collect();
        let registry = self.world.registry().clone();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        // Un thread à part, pas le pool : la sauvegarde ne doit pas retarder le chargement.
        std::thread::Builder::new()
            .name("sauvegarde".into())
            .spawn(move || {
                let _span = engine_trace::span("sauvegarde");
                let _ = sender.send(store.save_chunks(&chunks, &registry));
            })
            .expect("impossible de lancer le thread de sauvegarde");
        self.saving = Some((positions, receiver));
        true
    }

    /// Le résultat de la sauvegarde en tâche de fond, quand elle se termine.
    pub fn poll_save(&mut self) -> Option<anyhow::Result<SaveReport>> {
        let (_, receiver) = self.saving.as_ref()?;
        let result = receiver.try_recv().ok()?;
        Some(self.finish_save(result))
    }

    /// Sauvegarde tout, et attend la fin (à la fermeture du jeu).
    pub fn save_blocking(&mut self) -> anyhow::Result<SaveReport> {
        let mut total = SaveReport::default();
        // D'abord, laisser finir une sauvegarde déjà en route.
        if let Some((_, receiver)) = self.saving.as_ref() {
            let result = receiver
                .recv()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("le thread de sauvegarde s'est arrêté")));
            total = self.finish_save(result)?;
        }
        let Some(store) = self.store.clone() else {
            return Ok(total);
        };
        if self.dirty.is_empty() {
            return Ok(total);
        }
        let chunks = self.snapshot_dirty();
        let positions = chunks.iter().map(|c| c.pos).collect();
        let result = store.save_chunks(&chunks, self.world.registry());
        self.saving = Some((positions, crossbeam_channel::never()));
        let report = self.finish_save(result)?;
        Ok(SaveReport {
            chunks: total.chunks + report.chunks,
            regions: total.regions + report.regions,
            bytes: total.bytes + report.bytes,
        })
    }

    /// Prend des `Arc` des chunks modifiés et vide la liste des modifications.
    fn snapshot_dirty(&mut self) -> Vec<ChunkToSave> {
        // `drain` vide l'ensemble en rendant ses éléments.
        self.dirty
            .drain()
            .map(|pos| ChunkToSave {
                pos,
                chunk: self
                    .world
                    .chunk_arc(pos)
                    .or_else(|| self.saved_edits.get(&pos).cloned())
                    .unwrap_or_default(),
                unknown: self.unknown.get(&pos).cloned().unwrap_or_default(),
            })
            .collect()
    }

    fn finish_save(&mut self, result: anyhow::Result<SaveReport>) -> anyhow::Result<SaveReport> {
        let Some((positions, _)) = self.saving.take() else {
            return result;
        };
        match &result {
            Ok(_) => {
                // Sur disque maintenant : les chunks déchargés entre-temps peuvent quitter
                // la mémoire (sauf s'ils ont été remodifiés depuis le début de la sauvegarde).
                for pos in positions {
                    if !self.dirty.contains(&pos) && self.saved_edits.remove(&pos).is_some() {
                        self.unknown.remove(&pos);
                    }
                }
            }
            // Échec : on réessaiera à la prochaine sauvegarde.
            Err(_) => self.dirty.extend(positions),
        }
        result
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// Le monde, modifiable directement (pour préparer un test).
    #[cfg(test)]
    pub(crate) fn world_mut_for_tests(&mut self) -> &mut World {
        &mut self.world
    }

    pub fn generator(&self) -> &TerrainGenerator {
        &self.generator
    }

    /// Le générateur partagé : le terrain lointain l'utilise dans ses propres threads.
    pub fn generator_arc(&self) -> Arc<TerrainGenerator> {
        self.generator.clone()
    }

    /// Le résumé des chunks modifiés : le terrain lointain le lit pour montrer les constructions et
    /// les creusements hors du rayon des chunks détaillés.
    pub fn edited_surfaces(&self) -> Arc<crate::edited_surface::EditedSurfaces> {
        self.edited_surfaces.clone()
    }

    pub fn config(&self) -> &StreamingConfig {
        &self.config
    }

    /// Vrai si les blocs de ce chunk sont connus (généré, ou hors de la hauteur du monde,
    /// où il n'y a que de l'air). Le joueur ne doit pas bouger tant que son chunk n'est
    /// pas prêt, sinon il tomberait à travers un sol pas encore chargé.
    pub fn is_generated(&self, chunk: IVec3) -> bool {
        !self.world.vertical().contains(&chunk.y)
            || matches!(self.slots.get(&chunk), Some(Slot::Generated(_)))
    }

    /// Vrai si la lumière de la colonne qui contient ce chunk est calculée.
    pub fn is_lit(&self, chunk: IVec3) -> bool {
        is_lit(&self.columns, chunk)
    }

    /// Change un bloc, met à jour la lumière et remaille ce qui doit l'être.
    ///
    /// Renvoie `false` (et ne fait rien) si le chunk n'est pas encore généré.
    /// Les maillages des chunks dont les blocs changent sont refaits tout de suite et
    /// arrivent en tête de la file d'événements : le jeu les envoie au GPU dès cette
    /// image. Ceux dont seule la lumière change (une ombre qui descend loin, la lueur
    /// d'une torche) partent en priorité sur les threads de fond.
    pub fn set_block(&mut self, pos: IVec3, block: BlockId) -> bool {
        self.change_block(pos, block, true)
    }

    /// Comme [`ChunkStreamer::set_block`], mais tout le remaillage part en tâche de fond,
    /// groupé par chunk et une seule fois par image. Pour les machines, qui peuvent changer
    /// des centaines de blocs par tick : remailler chaque fois sur ce thread ferait saccader.
    pub fn set_block_quiet(&mut self, pos: IVec3, block: BlockId) -> bool {
        self.change_block(pos, block, false)
    }

    /// Pose un bloc **sans** recalculer la lumière ni remailler : pour remplir vite une
    /// zone (tests de charge, structures générées). Appeler [`ChunkStreamer::remesh_all`]
    /// ensuite pour voir le résultat. `false` si le chunk n'est pas chargé.
    pub fn set_block_raw(&mut self, pos: IVec3, block: BlockId) -> bool {
        let chunk = World::split(pos).0;
        if !matches!(self.slots.get(&chunk), Some(Slot::Generated(_))) {
            return false;
        }
        self.world.set_block(pos, block);
        self.ensure_biomes(chunk);
        self.edited.insert(chunk);
        self.dirty.insert(chunk);
        true
    }

    /// Remaille tous les chunks chargés (en tâche de fond).
    pub fn remesh_all(&mut self) {
        let loaded: Vec<IVec3> = self
            .slots
            .iter()
            .filter(|(_, s)| matches!(s, Slot::Generated(MeshState::Done)))
            .map(|(p, _)| *p)
            .collect();
        self.remesh_pending.extend(loaded);
    }

    /// Une propriété visuelle de l'entité a changé (allumage d'un four, par exemple).
    /// Le chunk et les voisins directs au bord sont remaillés : un raccord peut changer.
    pub fn invalidate_block_visuals(&mut self, pos: IVec3) {
        for d in std::iter::once(IVec3::ZERO).chain(crate::FACINGS) {
            self.remesh_pending.insert(World::split(pos + d).0);
        }
    }

    /// La quantité de fluide d'une cellule, en 4096e de bloc (`None` : rien d'enregistré,
    /// donc un réservoir plein si c'est un bloc de fluide).
    pub fn fluid_amount(&self, pos: IVec3) -> Option<u16> {
        let (chunk, [x, y, z]) = World::split(pos);
        self.world.chunk(chunk)?.fluid_amount(x, y, z)
    }

    /// Pose une cellule de fluide (ou de l'air) avec sa quantité. Le bloc ne change que
    /// s'il le faut (la lumière et les maillages suivent) ; une quantité seule change la
    /// hauteur de la surface : on remaille le chunk (et ses voisins si la cellule est au
    /// bord, car les coins de surface sont partagés). `false` si le chunk n'est pas chargé.
    pub fn set_fluid(&mut self, pos: IVec3, block: BlockId, amount: Option<u16>) -> bool {
        let (chunk, [x, y, z]) = World::split(pos);
        if !matches!(self.slots.get(&chunk), Some(Slot::Generated(_))) {
            return false;
        }
        if self.world.block(pos) != block && !self.change_block(pos, block, false) {
            return false;
        }
        if self.world.chunk(chunk).is_none() {
            return true; // De l'air dans un chunk vide : rien à enregistrer.
        }
        let previous = self
            .world
            .chunk(chunk)
            .and_then(|c| c.fluid_amount(x, y, z));
        if previous == amount {
            return true;
        }
        self.world
            .chunk_mut(chunk)
            .set_fluid_amount(x, y, z, amount);
        self.edited.insert(chunk);
        self.dirty.insert(chunk);
        // Les chunks voisins seulement si la cellule touche leur bord.
        let span = |v: usize| match v {
            0 => -1..=0,
            31 => 0..=1,
            _ => 0..=0,
        };
        for dx in span(x) {
            for dy in span(y) {
                for dz in span(z) {
                    self.remesh_pending.insert(chunk + IVec3::new(dx, dy, dz));
                }
            }
        }
        true
    }

    /// L'état actif du bloc à `pos`.
    pub fn block_entity(&self, pos: IVec3) -> Option<&BlockEntity> {
        let (chunk, [x, y, z]) = World::split(pos);
        self.world.chunk(chunk)?.block_entity(x, y, z)
    }

    /// L'état actif du bloc à `pos`, modifiable. Le chunk est noté « à sauvegarder ».
    pub fn block_entity_mut(&mut self, pos: IVec3) -> Option<&mut BlockEntity> {
        let (chunk, [x, y, z]) = World::split(pos);
        // On vérifie d'abord sans `chunk_mut` (qui créerait le chunk s'il manquait).
        self.world.chunk(chunk)?.block_entity(x, y, z)?;
        self.edited.insert(chunk);
        self.dirty.insert(chunk);
        self.world.chunk_mut(chunk).block_entity_mut(x, y, z)
    }

    /// Ajoute (ou retire, avec `None`) l'état actif d'un bloc. `false` si le chunk n'est pas chargé.
    pub fn set_block_entity(&mut self, pos: IVec3, entity: Option<BlockEntity>) -> bool {
        let (chunk, [x, y, z]) = World::split(pos);
        if !matches!(self.slots.get(&chunk), Some(Slot::Generated(_))) {
            return false;
        }
        if crate::connections::ports(self.block_entity(pos).and_then(|e| e.caps.as_deref()))
            != crate::connections::ports(entity.as_ref().and_then(|e| e.caps.as_deref()))
        {
            self.invalidate_block_visuals(pos);
        }
        self.edited.insert(chunk);
        self.dirty.insert(chunk);
        self.world
            .chunk_mut(chunk)
            .set_block_entity(x, y, z, entity);
        true
    }

    /// Pose les deux moitiés ensemble, même si elles sont dans des chunks différents.
    pub fn place_bed(&mut self, foot: IVec3, look: Vec3) -> Option<[(IVec3, BlockId); 2]> {
        let parts = crate::bed::placement(self.world.registry(), foot, look)?;
        self.place_pair(parts).then_some(parts)
    }

    pub fn place_pair(&mut self, parts: [(IVec3, BlockId); 2]) -> bool {
        let registry = self.world.registry();
        if crate::paired_block::partner(registry, parts[0].0, parts[0].1) != Some(parts[1]) {
            return false;
        }
        if parts.iter().any(|&(pos, block)| {
            let old = self.world.block(pos);
            !self.can_edit_cell(pos)
                || !(old.is_air() || registry.def(old).liquid)
                || !crate::is_supported(self, registry, pos, registry.def(block).support)
        }) {
            return false;
        }
        for (pos, block) in parts {
            self.change_cell(pos, block, true);
        }
        true
    }

    pub fn toggle_door(&mut self, pos: IVec3) -> bool {
        let Some(parts) = crate::door::toggled(&self.world, pos) else {
            return false;
        };
        if parts.iter().any(|&(p, _)| !self.can_edit_cell(p)) {
            return false;
        }
        for (p, block) in parts {
            self.change_cell(p, block, true);
        }
        true
    }

    fn can_edit_cell(&self, pos: IVec3) -> bool {
        matches!(
            self.slots.get(&World::split(pos).0),
            Some(Slot::Generated(_))
        )
    }

    fn change_block(&mut self, pos: IVec3, block: BlockId, immediate: bool) -> bool {
        if !self.can_edit_cell(pos) {
            return false;
        }
        let old = self.world.block(pos);
        if old == block {
            return true;
        }
        if let Some((other, expected)) =
            crate::paired_block::partner(self.world.registry(), pos, old)
        {
            // Attendre le chunk voisin évite de laisser une moitié dans une sauvegarde.
            if !self.can_edit_cell(other) {
                return false;
            }
            if self.world.block(other) == expected {
                self.change_cell(other, BlockId::AIR, immediate);
            }
        }
        self.change_cell(pos, block, immediate);
        true
    }

    fn change_cell(&mut self, pos: IVec3, block: BlockId, immediate: bool) {
        let chunk = World::split(pos).0;
        // Copy-on-write : si une tâche lit encore ce chunk, elle garde l'ancienne
        // version et nous modifions une copie (voir `World::chunk_mut`).
        self.world.set_block(pos, block);
        self.ensure_biomes(chunk);
        self.edited.insert(chunk);
        self.dirty.insert(chunk);
        // Un bloc remplacé perd sa quantité de fluide (le simulateur la remet s'il le faut).
        let local = World::split(pos).1;
        if self
            .world
            .chunk(chunk)
            .is_some_and(|c| c.fluid_amount(local[0], local[1], local[2]).is_some())
        {
            self.world
                .chunk_mut(chunk)
                .set_fluid_amount(local[0], local[1], local[2], None);
        }
        // Un bloc remplacé perd son état actif (une machine cassée oublie sa mémoire).
        let local = World::split(pos).1;
        if self
            .world
            .chunk(chunk)
            .is_some_and(|c| c.block_entity(local[0], local[1], local[2]).is_some())
        {
            self.world
                .chunk_mut(chunk)
                .set_block_entity(local[0], local[1], local[2], None);
        }
        // Le bloc inconnu qui était là (s'il y en avait un) est remplacé : on l'oublie.
        if let Some(unknown) = self.unknown.get_mut(&chunk) {
            let local = World::split(pos).1;
            unknown.remove(&Chunk::index(local[0], local[1], local[2]));
        }

        // La lumière. On sépare les champs empruntés : `columns` en lecture pendant
        // que `world` est modifié. Le compilateur l'accepte car ce sont deux champs distincts.
        let columns = &self.columns;
        let light_changed = update_light(&mut self.world, pos, |c| is_lit(columns, c));

        // Une colonne voisine en cours d'éclairage a lu les anciens blocs : à refaire.
        for dx in -1..=1 {
            for dz in -1..=1 {
                let column = IVec2::new(chunk.x + dx, chunk.z + dz);
                if let Some(info) = self.columns.get_mut(&column)
                    && matches!(info.light, LightState::Lighting(_))
                {
                    info.light = LightState::Missing;
                    self.light_candidates.insert(column);
                }
            }
        }

        // Les chunks qui contiennent le bloc ou un de ses 26 voisins : leurs faces
        // ou leur AO changent. On les remaille ici même, le chunk modifié en premier
        // (`push_front` : on les ajoute donc dans l'ordre inverse).
        let mut geometry: Vec<IVec3> = Vec::new();
        // À 2 blocs sur un axe : le culling des feuilles d'un bloc regarde 2 blocs devant.
        let two_away = crate::FACE_DIRECTIONS.map(|d| d * 2);
        for offset in NEIGHBOURS_27.into_iter().chain(two_away) {
            let c = World::split(pos + offset).0;
            if !geometry.contains(&c) {
                geometry.push(c);
            }
        }
        let optical_depth = crate::mesh::WATER_DEPTH_LIMIT;
        if World::split(pos + IVec3::Y * optical_depth).0.y != chunk.y {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let c = World::split(pos + IVec3::new(dx, optical_depth, dz)).0;
                    if !geometry.contains(&c) {
                        geometry.push(c);
                    }
                }
            }
        }
        if !immediate {
            self.remesh_pending.extend(geometry);
            self.remesh_pending.extend(light_changed);
            return;
        }
        for &c in geometry.iter().rev() {
            self.remesh_now(c);
        }
        // Les autres : seule leur lumière a changé.
        for c in light_changed {
            if !geometry.contains(&c) {
                self.remesh_soon(c);
            }
        }
    }

    /// Remaille un chunk sur ce thread, immédiatement.
    ///
    /// Un maillage prend environ une demi-milliseconde : pour quelques chunks, c'est plus
    /// simple et plus réactif que de passer par les threads de fond, dont la file est
    /// peut-être pleine de chunks à charger.
    fn remesh_now(&mut self, pos: IVec3) {
        if !matches!(self.slots.get(&pos), Some(Slot::Generated(_))) {
            return;
        }
        if !self.surroundings_lit(pos) {
            // Il sera maillé normalement quand ses voisins seront éclairés.
            self.slots.insert(pos, Slot::Generated(MeshState::Missing));
            self.mesh_candidates.insert(pos);
            return;
        }
        let leaf_culling = self.config.leaf_culling;
        let mesh = Neighbourhood::from_world(&self.world, pos)
            .map(|hood| hood.with_leaf_culling(leaf_culling))
            .map_or_else(ChunkMesh::default, |hood| self.config.mesher.mesh(&hood));
        // `Done` : si une tâche de fond maillait l'ancienne version, son résultat
        // (qui attend `Meshing(son ticket)`) sera jeté à l'arrivée.
        self.slots.insert(pos, Slot::Generated(MeshState::Done));
        // Un ancien maillage en attente ne doit pas écraser le nouveau.
        self.waiting.remove(&pos);
        self.meshing_dirty.remove(&pos);
        self.urgent
            .retain(|e| !matches!(e, StreamEvent::Mesh(p, _) if *p == pos));
        self.cancel_unload(pos);
        // Un maillage vide est envoyé aussi : le jeu retire alors l'ancien.
        self.urgent.push_front(StreamEvent::Mesh(pos, mesh));
    }

    /// Relance le maillage d'un chunk sur un thread de fond, sans attendre la file.
    fn remesh_soon(&mut self, pos: IVec3) {
        if !matches!(self.slots.get(&pos), Some(Slot::Generated(_))) {
            return;
        }
        // Un maillage tourne déjà : on ne lance pas une deuxième tâche dont l'arrivée rendrait la
        // première périmée. Un seul remaillage de plus quand celle-ci revient (voir `receive`).
        if matches!(
            self.slots.get(&pos),
            Some(Slot::Generated(MeshState::Meshing(_)))
        ) {
            self.meshing_dirty.insert(pos);
            return;
        }
        if self.surroundings_lit(pos) {
            self.spawn_mesh(pos);
        } else {
            self.slots.insert(pos, Slot::Generated(MeshState::Missing));
            self.mesh_candidates.insert(pos);
        }
    }

    /// Remplace les fluides connus du monde (ceux des mods compris).
    pub fn set_fluids(&mut self, fluids: std::sync::Arc<crate::fluids::FluidRegistry>) {
        self.world.set_fluids(fluids);
    }

    /// Un chunk de ciel (vide à la génération) créé par une modification n'a pas encore
    /// ses biomes : on les calcule, pour la teinte de l'herbe et le climat des plantes.
    fn ensure_biomes(&mut self, chunk: IVec3) {
        if self
            .world
            .chunk(chunk)
            .is_some_and(|c| c.biome(0, 0).is_none())
        {
            let biomes = self.generator.biome_columns(chunk);
            self.world.chunk_mut(chunk).set_biomes(biomes);
        }
    }

    /// Remplace le registre des blocs (rechargement à chaud de `blocks.ron`).
    ///
    /// Le nouveau registre doit avoir les mêmes blocs dans le même ordre : seules leurs
    /// propriétés changent (couleur, texture, lumière...). La lumière et les maillages
    /// dépendent de ces propriétés : on recalcule toute la lumière, ce qui remaillera tout.
    /// Les anciens maillages restent affichés jusque-là.
    pub fn replace_registry(&mut self, registry: Arc<BlockRegistry>) {
        self.world.set_registry(registry);
        for (&column, info) in &mut self.columns {
            if info.light != LightState::Missing {
                // Une tâche d'éclairage en route sera jetée à l'arrivée (état changé).
                info.light = LightState::Missing;
                self.light_candidates.insert(column);
            }
        }
        for (&pos, slot) in &mut self.slots {
            if let Slot::Generated(state @ (MeshState::Meshing(_) | MeshState::Done)) = slot {
                *state = MeshState::Missing;
                self.mesh_candidates.insert(pos);
            }
        }
    }

    /// Change d'algorithme de maillage et remaille tout ce qui est chargé.
    pub fn set_mesher(&mut self, mesher: Mesher) {
        self.config.mesher = mesher;
        for (&pos, slot) in &mut self.slots {
            if let Slot::Generated(state @ (MeshState::Meshing(_) | MeshState::Done)) = slot {
                // Les résultats encore en route seront jetés : l'état ne vaut plus `Meshing`.
                *state = MeshState::Missing;
                self.mesh_candidates.insert(pos);
            }
        }
    }

    /// Une image : suivre le joueur, récupérer les résultats, lancer de nouvelles tâches.
    pub fn update(&mut self, player: Vec3) {
        self.update_view(player, Vec3::ZERO);
    }

    /// Comme [`ChunkStreamer::update`], en disant où la caméra regarde : les colonnes devant elle sont
    /// chargées un peu avant celles de derrière (voir [`StreamingConfig::view_bias`]). Un regard presque
    /// vertical ne change pas la direction retenue ; `Vec3::ZERO` n'en donne aucune.
    pub fn update_view(&mut self, player: Vec3, forward: Vec3) {
        let _span = engine_trace::span("chunks : file et réception");
        let centre = World::chunk_containing(player);
        let flat = Vec2::new(forward.x, forward.z);
        if flat.length_squared() > 0.04 {
            self.view = Some(flat.normalize());
        }
        if self.centre != Some(centre) {
            self.centre = Some(centre);
            self.refresh(centre);
        } else if self.view_changed_enough() {
            self.resort_load_queue(centre);
        }
        self.receive();
        // Les chunks modifiés en silence (machines) : un seul remaillage par chunk et par image.
        for pos in std::mem::take(&mut self.remesh_pending) {
            self.remesh_soon(pos);
        }
        self.schedule(centre);
    }

    /// Le prochain événement à traiter (maillage à envoyer au GPU, chunk à libérer), sans limite :
    /// d'abord les maillages d'un bloc modifié, puis les maillages arrivés du plus proche au plus
    /// lointain, puis les libérations. Le jeu préfère [`ChunkStreamer::next_event_budgeted`].
    pub fn next_event(&mut self) -> Option<StreamEvent> {
        self.next_event_budgeted(&mut TransferBudget::unlimited())
    }

    /// Le prochain événement qui tient dans `budget` (voir `transfer.rs`), ou `None` quand l'image
    /// est pleine. Les maillages d'un bloc modifié passent toujours (hors budget, un par action du
    /// joueur) ; le premier maillage de l'image passe toujours, aussi gros soit-il. Le budget est
    /// mis à jour par le jeu une fois l'envoi fait ([`TransferBudget::record_upload`]).
    pub fn next_event_budgeted(&mut self, budget: &mut TransferBudget) -> Option<StreamEvent> {
        if let Some(event) = self.urgent.pop_front() {
            return Some(event);
        }
        if let Some(pos) = self.nearest_waiting() {
            let bytes = self.waiting[&pos].byte_size();
            if budget.allows_upload(bytes) {
                let mesh = self.waiting.remove(&pos).expect("maillage choisi");
                return Some(StreamEvent::Mesh(pos, mesh));
            }
        }
        if budget.allows_unload() {
            let pos = self.unloads.pop_front()?;
            self.unloads_pending.remove(&pos);
            return Some(StreamEvent::Unload(pos));
        }
        None
    }

    /// Le maillage en attente le plus prioritaire (le plus proche de la caméra, avec le biais de vue).
    fn nearest_waiting(&self) -> Option<IVec3> {
        let centre = self.centre?;
        self.waiting
            .keys()
            .min_by(|a, b| {
                self.priority(**a, centre)
                    .total_cmp(&self.priority(**b, centre))
            })
            .copied()
    }

    /// Une libération en attente pour ce chunk n'a plus lieu d'être (il est revenu avec un maillage).
    fn cancel_unload(&mut self, pos: IVec3) {
        if self.unloads_pending.remove(&pos) {
            self.unloads.retain(|p| *p != pos);
        }
    }

    /// Plus petit = plus urgent. Voir [`StreamingConfig::column_order`] et
    /// [`StreamingConfig::view_bias`] : on complète les colonnes dans l'ordre de leur distance
    /// horizontale (avec le biais de vue), puis chacune du chunk le plus proche en hauteur au plus
    /// loin ; l'ancien ordre était la distance 3D de chaque chunk.
    fn priority(&self, pos: IVec3, centre: IVec3) -> f32 {
        let d = pos - centre;
        if !self.config.column_order {
            return (d.x * d.x + d.y * d.y + d.z * d.z) as f32;
        }
        let mut score = (d.x * d.x + d.z * d.z) as f32;
        if self.config.view_bias > 0.0
            && score > 9.0
            && let Some(view) = self.view
        {
            let cosine = (d.x as f32 * view.x + d.z as f32 * view.y) / score.sqrt();
            score *= 1.0 + self.config.view_bias * (1.0 - cosine) * 0.5;
        }
        score * 8.0 + d.y.abs() as f32
    }

    /// La caméra a tourné d'au moins 25 degrés depuis le dernier tri de la file de chargement.
    fn view_changed_enough(&self) -> bool {
        if self.config.view_bias <= 0.0 || !self.config.column_order || self.load_queue.is_empty() {
            return false;
        }
        match (self.view, self.sorted_view) {
            (Some(now), Some(then)) => now.dot(then) < 0.9,
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// Retrie la file de chargement avec la direction actuelle de la caméra.
    fn resort_load_queue(&mut self, centre: IVec3) {
        let mut queue: Vec<IVec3> = std::mem::take(&mut self.load_queue).into();
        queue.sort_by_cached_key(|&pos| (self.priority(pos, centre) * 16.0) as i64);
        self.load_queue = queue.into();
        self.sorted_view = self.view;
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            loaded: self
                .slots
                .values()
                .filter(|s| matches!(s, Slot::Generated(_)))
                .count(),
            lit_columns: self
                .columns
                .values()
                .filter(|c| c.light == LightState::Done)
                .count(),
            queued: self.load_queue.len(),
            in_flight: self.in_flight,
            meshes_waiting: self.waiting.len() + self.urgent.len(),
            bytes_waiting: self.waiting.values().map(ChunkMesh::byte_size).sum(),
            unloads_waiting: self.unloads.len(),
            candidates: self.light_candidates.len() + self.mesh_candidates.len(),
            jobs_done: self.jobs_done,
            jobs_discarded: self.jobs_discarded,
        }
    }

    /// Plus rien à faire tant que le joueur ne bouge pas.
    pub fn is_idle(&self) -> bool {
        self.load_queue.is_empty()
            && self.in_flight == 0
            && self.light_candidates.is_empty()
            && self.remesh_pending.is_empty()
            && self.mesh_candidates.is_empty()
            && self.urgent.is_empty()
            && self.waiting.is_empty()
            && self.unloads.is_empty()
    }

    /// Le rayon de génération : assez pour éclairer toutes les voisines (même en
    /// diagonale, à √2 près) des colonnes visibles, plus leurs propres voisines.
    fn load_radius(&self) -> i32 {
        self.config.view_radius + 3
    }

    /// Une colonne doit être éclairée si elle touche (diagonales comprises) une colonne visible.
    fn wants_light(&self, column: IVec2, centre: IVec2) -> bool {
        let view_sq = self.config.view_radius.pow(2);
        (-1..=1).any(|dx| {
            (-1..=1).any(|dz| (column + IVec2::new(dx, dz) - centre).length_squared() <= view_sq)
        })
    }

    /// Le joueur a changé de chunk : décharger ce qui est trop loin, retrier la file.
    fn refresh(&mut self, centre: IVec3) {
        let load_radius = self.load_radius();
        let unload_radius = load_radius + self.config.unload_margin;
        let flat = IVec2::new(centre.x, centre.z);

        // 1. Déchargement. `retain` garde les éléments pour lesquels la closure renvoie `true`.
        let mut unloaded = Vec::new();
        // Le joueur a changé de couche : les chunks sombres qui entrent dans la tranche maillée
        // (voir `mesh_depth`) deviennent candidats.
        if self.centre_layer != Some(centre.y) {
            self.centre_layer = Some(centre.y);
            let fresh: Vec<IVec3> = self
                .dark
                .iter()
                .copied()
                .filter(|p| self.slots.get(p) == Some(&Slot::Generated(MeshState::Missing)))
                .filter(|p| self.in_vertical_view(*p, centre))
                .collect();
            self.mesh_candidates.extend(fresh);
        }
        self.slots.retain(|&pos, _| {
            let keep = horizontal_distance_sq(pos, centre) <= unload_radius * unload_radius;
            if !keep {
                unloaded.push(pos);
            }
            keep
        });
        self.columns
            .retain(|&c, _| (c - flat).length_squared() <= unload_radius * unload_radius);
        self.dark
            .retain(|&p| horizontal_distance_sq(p, centre) <= unload_radius * unload_radius);
        self.light_candidates
            .retain(|&c| (c - flat).length_squared() <= unload_radius * unload_radius);
        // Les maillages en attente des chunks déchargés ne servent plus : un seul passage
        // sur chaque file (un passage par chunk coûtait 60 ms pour 7 000 chunks d'un coup, après
        // une téléportation lointaine).
        if !unloaded.is_empty() {
            let gone: HashSet<IVec3> = unloaded.iter().copied().collect();
            self.urgent
                .retain(|e| !matches!(e, StreamEvent::Mesh(p, _) if gone.contains(p)));
            self.waiting.retain(|p, _| !gone.contains(p));
            self.meshing_dirty.retain(|p| !gone.contains(p));
        }
        for pos in unloaded {
            // Le dernier `Arc` disparaît ici (ou à la fin de la tâche qui le lit encore) :
            // `Drop` libère alors la mémoire du chunk. Côté GPU, c'est pareil : le jeu
            // retire le maillage, et les `wgpu::Buffer` sont libérés par leur `Drop`.
            let chunk = self.world.remove(pos);
            if self.edited.contains(&pos)
                && let Some(chunk) = &chunk
            {
                self.edited_surfaces
                    .record(pos, chunk, self.world.registry());
            }
            if self.edited.remove(&pos) {
                if self.store.is_none() || self.dirty.contains(&pos) || self.is_being_saved(pos) {
                    // Pas encore sur disque : on le garde en mémoire.
                    self.saved_edits.insert(pos, chunk.unwrap_or_default());
                } else {
                    // Déjà sauvegardé : il sera relu depuis le disque.
                    self.unknown.remove(&pos);
                }
            }
            self.mesh_candidates.remove(&pos);
            if self.unloads_pending.insert(pos) {
                self.unloads.push_back(pos);
            }
        }

        // 2. Nouvelle file de chargement, triée du plus proche au plus lointain.
        let mut wanted = Vec::new();
        let vertical = self.world.vertical();
        for dx in -load_radius..=load_radius {
            for dz in -load_radius..=load_radius {
                if dx * dx + dz * dz > load_radius * load_radius {
                    continue;
                }
                for y in vertical.clone() {
                    let pos = IVec3::new(centre.x + dx, y, centre.z + dz);
                    if !self.slots.contains_key(&pos) {
                        wanted.push(pos);
                    }
                }
            }
        }
        // Tri par priorité (voir `priority`) : des colonnes entières, de la plus proche à la plus
        // lointaine, avec un petit biais pour ce que la caméra regarde. Une valeur entière par clé
        // (la distance au carré, sans racine) évite de comparer des flottants.
        wanted.sort_by_cached_key(|&pos| (self.priority(pos, centre) * 16.0) as i64);
        self.sorted_view = self.view;
        if let Some(timings) = &mut self.timings {
            // Une demande garde sa date d'origine ; une demande abandonnée (le joueur s'est
            // éloigné avant que le chunk soit chargé) est oubliée.
            let queued: HashSet<IVec3> = wanted.iter().copied().collect();
            for &pos in &wanted {
                timings.requested(pos);
            }
            timings.retain(|pos| queued.contains(&pos) || self.slots.contains_key(&pos));
        }
        self.load_queue = wanted.into();

        // 3. Ce qui entre dans les rayons de lumière et de vue doit être éclairé ou maillé.
        let to_light: Vec<IVec2> = self
            .columns
            .iter()
            .filter(|(c, info)| info.light == LightState::Missing && self.wants_light(**c, flat))
            .map(|(c, _)| *c)
            .collect();
        self.light_candidates.extend(to_light);
        let view_sq = self.config.view_radius.pow(2);
        for (&pos, slot) in &self.slots {
            if *slot == Slot::Generated(MeshState::Missing)
                && horizontal_distance_sq(pos, centre) <= view_sq
            {
                self.mesh_candidates.insert(pos);
            }
        }
    }

    /// Récupère tous les résultats arrivés, sans attendre.
    fn receive(&mut self) {
        // `try_recv` renvoie `Err` dès que le canal est vide : on ne bloque jamais l'image.
        while let Ok(result) = self.receiver.try_recv() {
            self.in_flight -= 1;
            self.jobs_done += 1;
            match result {
                JobResult::Generated { pos, chunk, saved } => {
                    // Le chunk a pu être déchargé pendant sa génération : on jette le résultat.
                    if self.slots.get(&pos) != Some(&Slot::Generating) {
                        self.jobs_discarded += 1;
                        continue;
                    }
                    let fresh = saved.is_none() && chunk.is_some();
                    if let Some(chunk) = chunk {
                        self.world.insert(pos, Arc::new(chunk));
                    }
                    if fresh {
                        self.fresh_chunks.push(pos);
                    }
                    if let Some(unknown) = saved {
                        // Un chunk relu de la sauvegarde diffère de la génération : il doit
                        // être gardé comme un chunk modifié.
                        self.edited.insert(pos);
                        if !unknown.is_empty() {
                            self.unknown.insert(pos, unknown);
                        }
                    }
                    self.mark_generated(pos);
                }
                JobResult::Lit {
                    column,
                    ticket,
                    light,
                } => {
                    let Some(info) = self.columns.get_mut(&column) else {
                        self.jobs_discarded += 1;
                        continue; // Colonne déchargée entre-temps.
                    };
                    if info.light != LightState::Lighting(ticket) {
                        self.jobs_discarded += 1;
                        continue; // Résultat périmé (un bloc a changé à côté).
                    }
                    info.light = LightState::Done;
                    // Les chunks sans lumière du ciel : leur maillage peut attendre le joueur.
                    let bottom = self.world.vertical().start;
                    for (i, data) in light.iter().enumerate() {
                        let pos = IVec3::new(column.x, bottom + i as i32, column.y);
                        if data.as_ref().is_some_and(|d| d.iter().all(|v| v >> 4 == 0)) {
                            self.dark.insert(pos);
                        } else {
                            self.dark.remove(&pos);
                        }
                    }
                    apply_column_light(&mut self.world, column, light);
                    // Ses chunks et ceux des colonnes voisines attendaient peut-être cette lumière.
                    for dx in -1..=1 {
                        for dz in -1..=1 {
                            for y in self.world.vertical() {
                                self.mesh_candidates.insert(IVec3::new(
                                    column.x + dx,
                                    y,
                                    column.y + dz,
                                ));
                            }
                        }
                    }
                }
                JobResult::Meshed { pos, ticket, mesh } => {
                    if self.slots.get(&pos) == Some(&Slot::Generated(MeshState::Meshing(ticket))) {
                        self.slots.insert(pos, Slot::Generated(MeshState::Done));
                        if let Some(timings) = &mut self.timings {
                            timings.meshed(pos);
                        }
                        // Le plus récent remplace celui qui attend encore (jamais d'envoi périmé) ;
                        // et une libération en attente de ce chunk n'a plus lieu d'être.
                        self.waiting.insert(pos, mesh);
                        self.cancel_unload(pos);
                        // Modifié pendant le maillage : un dernier remaillage, une seule tâche.
                        if self.meshing_dirty.remove(&pos) {
                            self.remesh_soon(pos);
                        }
                    } else {
                        self.meshing_dirty.remove(&pos);
                        self.jobs_discarded += 1;
                    }
                }
            }
        }
    }

    /// Les chunks neufs (générés, pas relus de la sauvegarde) depuis le dernier appel.
    pub fn take_fresh_chunks(&mut self) -> Vec<IVec3> {
        std::mem::take(&mut self.fresh_chunks)
    }

    /// Les blocs actifs des chunks chargés récemment (et on oublie la liste).
    pub fn take_loaded_block_entities(&mut self) -> Vec<IVec3> {
        std::mem::take(&mut self.loaded_block_entities)
    }

    /// Un chunk vient d'être généré (ou rendu depuis les modifications gardées).
    fn mark_generated(&mut self, pos: IVec3) {
        if let Some(chunk) = self.world.chunk(pos) {
            let origin = World::chunk_origin(pos);
            for (i, _) in chunk.block_entities() {
                let local = IVec3::new((i % 32) as i32, (i / 1024) as i32, ((i / 32) % 32) as i32);
                self.loaded_block_entities.push(origin + local);
            }
        }
        if let Some(timings) = &mut self.timings {
            timings.generated(pos);
        }
        self.slots.insert(pos, Slot::Generated(MeshState::Missing));
        let column = IVec2::new(pos.x, pos.z);
        let layers = self.world.vertical().len();
        let info = self.columns.entry(column).or_default();
        info.generated += 1;
        if info.generated as usize == layers {
            // Colonne complète : elle et ses voisines peuvent peut-être être éclairées.
            for dx in -1..=1 {
                for dz in -1..=1 {
                    self.light_candidates
                        .insert(IVec2::new(column.x + dx, column.y + dz));
                }
            }
        }
    }

    /// Lance des tâches tant qu'il reste de la place : maillages d'abord (ils rendent
    /// des chunks visibles), puis lumière, puis générations.
    fn schedule(&mut self, centre: IVec3) {
        let max = self.config.max_jobs_in_flight;
        if self.in_flight < max && !self.mesh_candidates.is_empty() {
            let mut candidates: Vec<IVec3> = self.mesh_candidates.iter().copied().collect();
            candidates.sort_by_cached_key(|&pos| (self.priority(pos, centre) * 16.0) as i64);
            for pos in candidates {
                if self.in_flight >= max {
                    break; // Les autres restent candidats pour l'image suivante.
                }
                self.mesh_candidates.remove(&pos);
                if self.ready_to_mesh(pos, centre) {
                    self.spawn_mesh(pos);
                }
            }
        }

        if self.in_flight < max && !self.light_candidates.is_empty() {
            let flat = IVec2::new(centre.x, centre.z);
            let mut candidates: Vec<IVec2> = self.light_candidates.iter().copied().collect();
            candidates.sort_by_cached_key(|&c| {
                (self.priority(IVec3::new(c.x, centre.y, c.y), centre) * 16.0) as i64
            });
            for column in candidates {
                if self.in_flight >= max {
                    break;
                }
                self.light_candidates.remove(&column);
                if self.ready_to_light(column, flat) {
                    self.spawn_light(column);
                }
            }
        }

        while self.in_flight < max {
            let Some(pos) = self.load_queue.pop_front() else {
                break;
            };
            // L'API `entry` : une seule recherche dans la table pour tester et insérer.
            if let Entry::Vacant(entry) = self.slots.entry(pos) {
                // Un chunk modifié par le joueur revient tel qu'il l'a laissé.
                if let Some(chunk) = self.saved_edits.remove(&pos) {
                    entry.insert(Slot::Generating);
                    self.world.insert(pos, chunk);
                    self.edited.insert(pos);
                    self.mark_generated(pos);
                } else {
                    entry.insert(Slot::Generating);
                    self.spawn_generation(pos);
                }
            }
        }
    }

    /// Une colonne s'éclaire quand elle et ses 8 voisines sont entièrement générées.
    fn ready_to_light(&self, column: IVec2, centre: IVec2) -> bool {
        self.wants_light(column, centre)
            && self
                .columns
                .get(&column)
                .is_some_and(|c| c.light == LightState::Missing)
            && (-1..=1).all(|dx| {
                (-1..=1).all(|dz| {
                    self.columns
                        .get(&IVec2::new(column.x + dx, column.y + dz))
                        .is_some_and(|c| c.generated as usize == self.world.vertical().len())
                })
            })
    }

    /// On ne maille un chunk que quand sa colonne et les 8 voisines sont éclairées :
    /// ses faces de bordure, sa lumière douce et son AO sont alors justes du premier coup.
    /// Si on maillait plus tôt, il faudrait le remailler à chaque arrivée d'un voisin.
    /// C'est pour ça qu'on génère et qu'on éclaire plus loin que le rayon de vue.
    fn ready_to_mesh(&self, pos: IVec3, centre: IVec3) -> bool {
        let view = self.config.view_radius;
        self.slots.get(&pos) == Some(&Slot::Generated(MeshState::Missing))
            && horizontal_distance_sq(pos, centre) <= view * view
            && self.in_vertical_view(pos, centre)
            && self.surroundings_lit(pos)
    }

    /// Un chunk sombre loin sous le joueur attend qu'il descende (voir `mesh_depth`).
    fn in_vertical_view(&self, pos: IVec3, centre: IVec3) -> bool {
        match self.config.mesh_depth {
            Some(depth) => pos.y >= centre.y - depth || !self.dark.contains(&pos),
            None => true,
        }
    }

    fn surroundings_lit(&self, pos: IVec3) -> bool {
        (-1..=1).all(|dx| (-1..=1).all(|dz| self.is_lit(IVec3::new(pos.x + dx, 0, pos.z + dz))))
    }

    fn ticket(&mut self) -> u32 {
        self.next_ticket = self.next_ticket.wrapping_add(1);
        self.next_ticket
    }

    fn is_being_saved(&self, pos: IVec3) -> bool {
        self.saving
            .as_ref()
            .is_some_and(|(positions, _)| positions.contains(&pos))
    }

    fn spawn_generation(&mut self, pos: IVec3) {
        // On clone l'`Arc` (un compteur +1) et le `Sender` : la closure `move`
        // en prend possession et les emporte sur le thread de fond.
        let generator = Arc::clone(&self.generator);
        let store = self.store.clone();
        let registry = self.world.registry().clone();
        let sender = self.sender.clone();
        self.in_flight += 1;
        self.pool.spawn(move || {
            let _span = engine_trace::span("génération");
            // D'abord la sauvegarde : un chunk modifié par le joueur y est peut-être.
            let loaded = store.and_then(|store| match store.load_chunk(pos, &registry) {
                Ok(loaded) => loaded,
                Err(err) => {
                    log::error!("{err:#} : on le régénère");
                    None
                }
            });
            let (chunk, saved) = match loaded {
                Some(loaded) => {
                    // Les biomes ne sont pas sauvegardés : on les recalcule.
                    let mut chunk = loaded.chunk;
                    chunk.set_biomes(generator.biome_columns(pos));
                    (Some(chunk), Some(loaded.unknown))
                }
                None => (generator.generate(pos), None),
            };
            // Si le jeu s'est fermé entre-temps, le `Receiver` n'existe plus : on ignore.
            let _ = sender.send(JobResult::Generated { pos, chunk, saved });
        });
    }

    fn spawn_light(&mut self, column: IVec2) {
        let ticket = self.ticket();
        if let Some(info) = self.columns.get_mut(&column) {
            info.light = LightState::Lighting(ticket);
        }
        let region = LightRegion::from_world(&self.world, column);
        let sender = self.sender.clone();
        self.in_flight += 1;
        self.pool.spawn(move || {
            let _span = engine_trace::span("lumière");
            let light = region.compute();
            let _ = sender.send(JobResult::Lit {
                column,
                ticket,
                light,
            });
        });
    }

    fn spawn_mesh(&mut self, pos: IVec3) {
        // Un chunk vide n'a rien à dessiner : pas besoin de tâche.
        let Some(hood) = Neighbourhood::from_world(&self.world, pos)
            .map(|hood| hood.with_leaf_culling(self.config.leaf_culling))
        else {
            self.slots.insert(pos, Slot::Generated(MeshState::Done));
            if let Some(timings) = &mut self.timings {
                timings.forget(pos);
            }
            return;
        };
        if let Some(timings) = &mut self.timings {
            timings.mesh_started(pos);
        }
        let ticket = self.ticket();
        let mesher = self.config.mesher;
        let sender = self.sender.clone();
        self.slots
            .insert(pos, Slot::Generated(MeshState::Meshing(ticket)));
        self.in_flight += 1;
        self.pool.spawn(move || {
            let _span = engine_trace::span("maillage");
            let mesh = mesher.mesh(&hood);
            let _ = sender.send(JobResult::Meshed { pos, ticket, mesh });
        });
    }
}

/// La colonne de ce chunk est éclairée (au-dessus et en dessous du monde : oui).
fn is_lit(columns: &HashMap<IVec2, ColumnInfo>, chunk: IVec3) -> bool {
    columns
        .get(&IVec2::new(chunk.x, chunk.z))
        .is_some_and(|c| c.light == LightState::Done)
}

/// Le bloc et ses 26 voisins.
const NEIGHBOURS_27: [IVec3; 27] = {
    let mut offsets = [IVec3::ZERO; 27];
    let mut i = 0;
    // Une boucle `while` : les boucles `for` ne sont pas encore permises dans un `const`.
    while i < 27 {
        offsets[i] = IVec3::new(i as i32 % 3 - 1, i as i32 / 9 - 1, (i as i32 / 3) % 3 - 1);
        i += 1;
    }
    offsets
};

/// Distance horizontale au carré, en chunks : le rayon de vue est un cercle, pas une sphère.
fn horizontal_distance_sq(a: IVec3, b: IVec3) -> i32 {
    let d = a - b;
    d.x * d.x + d.z * d.z
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::mesh::mesh_binary;
    use crate::registry::test_blocks::{snow, stone};

    const RADIUS: i32 = 2;

    fn streamer() -> ChunkStreamer {
        let config = StreamingConfig {
            view_radius: RADIUS,
            unload_margin: 1,
            ..StreamingConfig::default()
        };
        ChunkStreamer::new(TerrainGenerator::new(7), config)
    }

    #[test]
    fn changing_machine_ports_remeshes_the_neighbour_chunk_but_changing_amounts_does_not() {
        use crate::{EnergyStorage, GridPos, Grids, Sides};
        use std::cell::RefCell;
        let mut s = streamer();
        pump(&mut s, Vec3::new(4.0, 80.0, 4.0));
        let pos = IVec3::new(32, 80, 4);
        let battery = s.world().registry().id("core:battery").unwrap();
        assert!(s.set_block_quiet(pos, battery));
        s.remesh_pending.clear();
        let cell = RefCell::new(s);
        let grids = Grids::new(&cell);
        grids.write_or_create(GridPos::world(pos), |e| {
            e.caps_mut().energy = Some(EnergyStorage::new(100, 10))
        });
        let expected = HashSet::from_iter([IVec3::new(0, 2, 0), IVec3::new(1, 2, 0)]);
        assert_eq!(cell.borrow().remesh_pending, expected);
        cell.borrow_mut().remesh_pending.clear();
        grids.write(GridPos::world(pos), |e| {
            e.caps_mut().energy.as_mut().unwrap().amount = 42
        });
        assert!(
            cell.borrow().remesh_pending.is_empty(),
            "le contenu ne change pas la forme"
        );
        grids.write(GridPos::world(pos), |e| {
            e.caps_mut().energy.as_mut().unwrap().sides = Sides::NONE
        });
        assert_eq!(cell.borrow().remesh_pending, expected);
    }

    /// Charge tout autour de `player` (pour les tests d'autres modules).
    pub(crate) fn pump_until_idle(streamer: &mut ChunkStreamer, player: Vec3) {
        pump(streamer, player);
    }

    /// Fait tourner le streamer jusqu'à ce qu'il n'ait plus rien à faire,
    /// et renvoie tous les événements produits.
    fn pump(streamer: &mut ChunkStreamer, player: Vec3) -> Vec<StreamEvent> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut events = Vec::new();
        loop {
            streamer.update(player);
            events.extend(std::iter::from_fn(|| streamer.next_event()));
            if streamer.is_idle() {
                return events;
            }
            assert!(Instant::now() < deadline, "le chargement ne se termine pas");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn meshed(events: &[StreamEvent]) -> HashMap<IVec3, &ChunkMesh> {
        events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Mesh(pos, mesh) => Some((*pos, mesh)),
                StreamEvent::Unload(_) => None,
            })
            .collect()
    }

    #[test]
    fn loads_every_chunk_in_radius() {
        let mut s = streamer();
        let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));

        // Rayon de génération = rayon de vue + 3.
        let r = RADIUS + 3;
        let columns = (-r..=r)
            .flat_map(|x| (-r..=r).map(move |z| x * x + z * z))
            .filter(|&d| d <= r * r)
            .count();
        assert_eq!(
            s.stats().loaded,
            columns * crate::WORLD_HEIGHT_CHUNKS as usize
        );

        // Tout chunk non vide dans le rayon de vue a reçu un maillage, et seulement eux.
        let meshes = meshed(&events);
        assert!(!meshes.is_empty());
        for pos in meshes.keys() {
            assert!(horizontal_distance_sq(*pos, IVec3::ZERO) <= RADIUS * RADIUS);
        }
        for pos in s.world().chunk_positions() {
            let inside = horizontal_distance_sq(pos, IVec3::ZERO) <= RADIUS * RADIUS;
            let has_faces = !mesh_binary(s.world(), pos).is_empty();
            assert_eq!(
                meshes.contains_key(&pos),
                inside && has_faces,
                "chunk {pos}"
            );
        }
    }

    /// Le maillage fait en tâche de fond doit être identique au maillage direct :
    /// les faces de bordure tiennent bien compte des voisins.
    #[test]
    fn pipeline_timing_is_off_by_default_and_measures_every_uploaded_chunk_when_on() {
        let mut off = streamer();
        assert!(off.pipeline_report().is_none());
        off.note_uploaded(IVec3::ZERO); // sans effet, sans panique

        let mut on = streamer();
        on.set_pipeline_timing(true);
        let events = pump(&mut on, Vec3::new(0.5, 70.0, 0.5));
        let uploaded = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Mesh(..)))
            .count();
        assert!(uploaded > 0);
        for event in &events {
            if let StreamEvent::Mesh(pos, _) = event {
                on.note_uploaded(*pos);
            }
        }
        let report = on.pipeline_report().unwrap();
        let total = report.stage(crate::PipelineStage::Total);
        assert_eq!(total.count, uploaded as u64, "un chunk, une mesure");
        assert!(total.p50_ms <= total.max_ms);
        for stage in [
            crate::PipelineStage::Generation,
            crate::PipelineStage::Light,
            crate::PipelineStage::Meshing,
            crate::PipelineStage::Upload,
        ] {
            assert_eq!(report.stage(stage).count, total.count);
            assert!(report.stage(stage).max_ms <= total.max_ms + 0.01);
        }
        assert!(
            report
                .to_string()
                .contains(&format!("{uploaded} chunks mesurés"))
        );
    }

    #[test]
    fn streamed_meshes_match_direct_meshing() {
        let mut s = streamer();
        let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let meshes = meshed(&events);
        for (pos, mesh) in meshes {
            assert_eq!(
                mesh.vertices,
                mesh_binary(s.world(), pos).vertices,
                "chunk {pos}"
            );
        }
    }

    #[test]
    fn small_moves_unload_nothing() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        // Un chunk plus loin : la marge d'hystérésis garde tout.
        let events = pump(&mut s, Vec3::new(37.0, 40.0, 5.0));
        assert!(!events.iter().any(|e| matches!(e, StreamEvent::Unload(_))));
    }

    #[test]
    fn far_chunks_are_unloaded() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let events = pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
        let unloaded: Vec<IVec3> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Unload(pos) => Some(*pos),
                StreamEvent::Mesh(..) => None,
            })
            .collect();
        assert!(unloaded.contains(&IVec3::new(0, 0, 0)));
        assert!(s.world().chunk(IVec3::new(0, 0, 0)).is_none());
        // Il ne reste que des chunks proches de la nouvelle position (rayon 2 + 3 + marge 1).
        assert!(
            s.slots
                .keys()
                .all(|p| horizontal_distance_sq(*p, IVec3::new(20, 0, 0)) <= 6 * 6)
        );
    }

    /// Le bloc opposé : de l'air devient de la pierre, un bloc plein devient de l'air.
    fn toggled(s: &ChunkStreamer, pos: IVec3) -> BlockId {
        if s.world().block(pos).is_air() {
            stone()
        } else {
            BlockId::AIR
        }
    }

    #[test]
    fn editing_a_border_block_remeshes_both_chunks_at_once() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        // x = 31 : dernier bloc du chunk x = 0, collé au chunk x = 1.
        let pos = IVec3::new(31, 40, 5);
        let block = toggled(&s, pos);
        assert!(s.set_block(pos, block));
        assert_eq!(s.world().block(pos), block);

        // Les deux maillages sont en tête de file, sans attendre de tâche de fond.
        let first: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).take(2).collect();
        let positions: Vec<IVec3> = first
            .iter()
            .map(|e| match e {
                StreamEvent::Mesh(p, mesh) => {
                    assert_eq!(mesh, &mesh_binary(s.world(), *p), "maillage à jour");
                    *p
                }
                StreamEvent::Unload(p) => panic!("déchargement inattendu de {p}"),
            })
            .collect();
        assert_eq!(positions, [IVec3::new(0, 1, 0), IVec3::new(1, 1, 0)]);
    }

    #[test]
    fn doors_place_toggle_and_break_as_a_pair_across_a_vertical_chunk_border() {
        let mut s = streamer();
        let bottom = IVec3::new(5, 127, 5);
        let top = bottom + IVec3::Y;
        let registry = s.world().registry().clone();
        let rock = registry.id("core:cobblestone").unwrap();
        let door = registry.id("core:wooden_glass_door").unwrap();
        let parts = crate::door::placement(&registry, door, bottom, Vec3::Z).unwrap();
        s.slots
            .insert(World::split(bottom).0, Slot::Generated(MeshState::Missing));
        s.set_block_quiet(bottom - IVec3::Y, rock);
        assert!(!s.place_pair(parts));
        assert!(s.world.block(bottom).is_air());
        s.slots
            .insert(World::split(top).0, Slot::Generated(MeshState::Missing));
        s.set_block_quiet(top, rock);
        assert!(!s.place_pair(parts));
        assert!(s.world.block(bottom).is_air());
        s.set_block_quiet(top, BlockId::AIR);
        assert!(s.place_pair(parts));

        let origin = top.as_vec3() + Vec3::new(0.5, 0.5, -1.0);
        assert_eq!(
            crate::raycast(s.world(), origin, Vec3::Z, 2.0)
                .unwrap()
                .block_pos,
            top
        );
        let mesh = crate::mesh_binary(s.world(), World::split(top).0);
        assert!(!mesh.indices.is_empty() && !mesh.translucent_indices.is_empty());
        assert!(s.toggle_door(top));
        for (p, _) in parts {
            assert!(crate::door::is_open(registry.def(s.world.block(p))));
            assert_eq!(registry.light_opacity(s.world.block(p)), 0);
        }
        assert!(crate::raycast(s.world(), origin, Vec3::Z, 2.0).is_none());
        assert!(s.toggle_door(bottom));
        assert_eq!(parts.map(|(p, _)| s.world.block(p)), parts.map(|(_, b)| b));
        let items = crate::ItemRegistry::builtin(&registry).unwrap();
        assert_eq!(
            items.of_block(s.world.block(top)),
            items.id("core:wooden_glass_door")
        );
        assert!(s.set_block(top, BlockId::AIR));
        assert!(s.world.block(bottom).is_air() && s.world.block(top).is_air());
        assert!(s.place_pair(parts));
        let floor = bottom - IVec3::Y;
        assert!(s.set_block(floor, BlockId::AIR));
        let settled = crate::settle(&mut s, floor);
        assert_eq!(settled.broken, vec![parts[0]]);
        assert!(s.world.block(bottom).is_air() && s.world.block(top).is_air());
    }

    #[test]
    fn both_bed_halves_cross_chunk_borders_and_require_space_and_support() {
        let mut s = streamer();
        let foot = IVec3::new(31, 170, 31);
        let head = foot + IVec3::X;
        let registry = s.world().registry().clone();
        let rock = registry.id("core:cobblestone").unwrap();
        s.slots
            .insert(World::split(foot).0, Slot::Generated(MeshState::Missing));
        s.set_block_quiet(foot - IVec3::Y, rock);
        assert!(s.place_bed(foot, Vec3::X).is_none());
        assert!(s.world().block(foot).is_air());
        s.slots
            .insert(World::split(head).0, Slot::Generated(MeshState::Missing));
        assert!(s.place_bed(foot, Vec3::X).is_none());
        s.set_block_quiet(head - IVec3::Y, rock);
        s.set_block_quiet(head, rock);
        assert!(s.place_bed(foot, Vec3::X).is_none());
        assert!(s.world().block(foot).is_air());
        assert_eq!(s.world().block(head), rock);
        s.set_block_quiet(head, BlockId::AIR);

        let items = crate::ItemRegistry::builtin(&registry).unwrap();
        let bed = items.id(crate::bed::ITEM).unwrap();
        for broken in [head, foot] {
            let parts = s.place_bed(foot, Vec3::X).unwrap();
            assert_eq!(parts.map(|(p, _)| p), [foot, head]);
            for (p, block) in parts {
                assert_eq!(items.of_block(block), Some(bed));
                assert_eq!(crate::bed::head_at(s.world(), p), Some(head));
                let hit = crate::raycast(
                    s.world(),
                    p.as_vec3() + Vec3::new(0.5, 2.0, 0.5),
                    Vec3::NEG_Y,
                    3.0,
                )
                .unwrap();
                assert_eq!(hit.block_pos, p);
            }
            assert!(s.set_block(broken, BlockId::AIR));
            assert!(s.world().block(foot).is_air() && s.world().block(head).is_air());
        }

        s.place_bed(foot, Vec3::X).unwrap();
        let floor = head - IVec3::Y;
        s.set_block(floor, BlockId::AIR);
        let settled = crate::settle(&mut s, floor);
        assert_eq!(settled.broken.len(), 1);
        assert_eq!(items.of_block(settled.broken[0].1), Some(bed));
        assert!(s.world().block(foot).is_air() && s.world().block(head).is_air());
    }

    #[test]
    fn breaking_a_leaf_two_blocks_from_the_border_remeshes_the_neighbour() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let leaves = BlockRegistry::builtin().id("core:oak_leaves").unwrap();
        // Un cube de feuilles 3 × 3 × 3 à cheval sur les chunks x = 0 et x = 1, dans le ciel :
        // son centre (32, 150, 5), entouré, est opaque, et la face +X de la feuille en
        // x = 31 (chunk 0) est cachée contre lui.
        for x in 31..34 {
            for y in 149..152 {
                for z in 4..7 {
                    assert!(s.set_block(IVec3::new(x, y, z), leaves));
                }
            }
        }
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        // x = 33, deuxième bloc du chunk x = 1 : le centre n'est plus entouré, la face en
        // x = 31, à deux blocs, redevient visible. Le chunk 0 doit être remaillé.
        assert!(s.set_block(IVec3::new(33, 150, 5), BlockId::AIR));
        let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
        let remeshed = meshed(&events);
        let chunk = IVec3::new(0, 4, 0);
        let mesh = remeshed.get(&chunk).expect("le chunk voisin est remaillé");
        assert_eq!(*mesh, &mesh_binary(s.world(), chunk));
    }

    #[test]
    fn interior_edit_remeshes_only_its_chunk() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let pos = IVec3::new(10, 40, 10);
        assert!(s.set_block(pos, toggled(&s, pos)));
        let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Mesh(p, _) if p == IVec3::new(0, 1, 0)));
    }

    #[test]
    fn edits_outside_loaded_chunks_are_refused() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert!(!s.set_block(IVec3::new(5000, 40, 0), stone()));
    }

    #[test]
    fn edits_survive_unloading() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let dug = IVec3::new(3, 20, 3); // sous terre : de la pierre (ou une grotte)
        let built = IVec3::new(8, 120, 8); // dans le ciel
        let dug_block = toggled(&s, dug);
        assert!(s.set_block(dug, dug_block));
        assert!(s.set_block(built, snow()));

        // On part loin : tout est déchargé...
        pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
        assert!(s.world().chunk(IVec3::ZERO).is_none());
        // ... puis on revient : les modifications sont toujours là.
        let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert_eq!(s.world().block(dug), dug_block);
        assert_eq!(s.world().block(built), snow());
        // Et le chunk du ciel, vide à la génération, a bien un maillage pour ce bloc.
        assert!(meshed(&events).contains_key(&IVec3::new(0, 3, 0)));
    }

    #[test]
    fn unloading_an_edited_chunk_notes_its_surface_for_the_far_terrain() {
        let mut s = streamer();
        let surfaces = s.edited_surfaces();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let built = IVec3::new(8, 120, 8);
        assert!(s.set_block(built, stone()));
        // Tant que le chunk est chargé, rien n'est noté (le lointain ne le couvre pas).
        assert!(surfaces.is_empty());
        pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
        assert!(s.world().chunk(IVec3::ZERO).is_none());
        let ground = s.generator().column(8, 8).height;
        let found = surfaces.surface_at(8, 8, ground, s.world().registry());
        assert_eq!(found, Some((120, stone())));
        // Sa colonne est signalée une fois, pour reconstruire les tuiles lointaines qui la couvrent.
        assert_eq!(surfaces.take_changed(), vec![(0, 0)]);
        assert!(surfaces.take_changed().is_empty());
    }

    #[test]
    fn mod_blocks_can_be_placed_and_meshed() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let registry = BlockRegistry::load(&repo.join("assets"), Some(&repo.join("mods"))).unwrap();
        let ruby = registry.id("example:ruby").unwrap();
        let ruby_texture = registry.def(ruby).face_textures[0];
        assert_ne!(ruby_texture, 0);
        let generator = TerrainGenerator::with_registry(7, Arc::new(registry)).unwrap();
        let config = StreamingConfig {
            view_radius: RADIUS,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(generator, config);
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));

        // Un rubis seul dans le ciel : 6 faces.
        let pos = IVec3::new(8, 120, 8);
        assert!(s.set_block(pos, ruby));
        let Some(StreamEvent::Mesh(_, mesh)) = s.next_event() else {
            panic!("pas de maillage");
        };
        assert_eq!(mesh.quad_count(), 6);
        // Le rubis a une texture : ses faces portent sa couche.
        assert!(mesh.vertices.iter().all(|v| v.texture == ruby_texture));
    }

    #[test]
    fn torch_light_reaches_and_remeshes_the_next_chunk() {
        let mut s = streamer();
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let torch = BlockRegistry::builtin().id("core:torch").unwrap();
        // Dans le ciel, à 12 blocs du chunk x = 1 : sa lumière y déborde.
        let pos = IVec3::new(20, 120, 5);
        assert!(s.set_block(pos, torch));
        let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert_eq!(s.world().light(pos).1, 14);
        assert_eq!(s.world().light(IVec3::new(33, 120, 5)).1, 1);
        // Le chunk voisin n'a pas changé de blocs, seulement de lumière : il est remaillé
        // en tâche de fond.
        assert!(meshed(&events).contains_key(&IVec3::new(1, 3, 0)));
        // Et tout reste cohérent avec un maillage direct.
        for (pos, mesh) in meshed(&events) {
            assert_eq!(mesh, &mesh_binary(s.world(), pos), "chunk {pos}");
        }
    }

    fn saving_streamer(dir: &std::path::Path) -> ChunkStreamer {
        let store = Arc::new(crate::save::SaveStore::open(dir).unwrap());
        streamer().with_save(store)
    }

    /// Le test d'intégration de la sauvegarde : on modifie, on sauvegarde, on « relance
    /// le jeu » avec un streamer neuf sur le même dossier, et le monde doit être identique.
    #[test]
    fn reloaded_world_has_the_same_blocks() {
        let dir = crate::save::tests::temp_dir("streamer");
        let player = Vec3::new(5.0, 40.0, 5.0);
        let mut first = saving_streamer(&dir);
        pump(&mut first, player);
        // Du verre : il ne peut pas déjà se trouver là (le terrain n'en génère pas).
        let glass = BlockRegistry::builtin().id("core:glass").unwrap();
        let edits = [
            (IVec3::new(3, 20, 3), glass),
            (IVec3::new(8, 120, 8), snow()), // dans un chunk de ciel, vide à la génération
            (IVec3::new(31, 40, 5), glass),  // en bordure de chunk
            (IVec3::new(-20, 39, -7), glass),
        ];
        for (pos, block) in edits {
            assert!(first.set_block(pos, block));
        }
        assert!(first.has_unsaved_changes());
        let report = first.save_blocking().unwrap();
        assert_eq!(report.chunks, 4);
        assert!(!first.has_unsaved_changes());

        let mut second = saving_streamer(&dir);
        pump(&mut second, player);
        for (pos, block) in edits {
            assert_eq!(second.world().block(pos), block, "en {pos}");
        }
        // Tous les chunks chargés sont identiques, bloc pour bloc.
        for pos in first.world().chunk_positions() {
            let a = first.world().chunk(pos).map(|c| c.blocks().to_vec());
            let b = second.world().chunk(pos).map(|c| c.blocks().to_vec());
            assert_eq!(a, b, "chunk {pos}");
        }
    }

    #[test]
    fn background_save_finishes_and_frees_unloaded_chunks() {
        let dir = crate::save::tests::temp_dir("background");
        let mut s = saving_streamer(&dir);
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert!(!s.begin_save(), "rien à sauvegarder");
        assert!(s.set_block(IVec3::new(3, 20, 3), BlockId::AIR));
        // On part loin : le chunk modifié est gardé en mémoire, pas encore sauvegardé.
        pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
        assert!(s.saved_edits.contains_key(&IVec3::new(0, 0, 0)));

        assert!(s.begin_save());
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let report = loop {
            if let Some(result) = s.poll_save() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(report.chunks, 1);
        // Écrit sur disque : plus besoin de le garder en mémoire...
        assert!(s.saved_edits.is_empty());
        // ... et en revenant, il est relu depuis le disque.
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert!(s.world().block(IVec3::new(3, 20, 3)).is_air());
    }

    #[test]
    fn replacing_the_registry_relights_and_remeshes() {
        let mut s = streamer();
        let first = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        // Même blocs, propriétés changées : la pierre devient une source de lumière.
        let text = include_str!("../../../assets/core/blocks.ron").replacen(
            r#"id: "core:stone","#,
            r#"id: "core:stone", light_emission: 5,"#,
            1,
        );
        let mut registry = BlockRegistry::empty();
        registry.add_source("core", &text, "test").unwrap();
        assert_eq!(registry.light_emission(stone()), 5);
        s.replace_registry(Arc::new(registry));
        let second = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert_eq!(
            meshed(&first).len(),
            meshed(&second).len(),
            "tout est remaillé"
        );
        // Une pierre à l'air libre éclaire maintenant ses voisins.
        let lit = s.world().chunk_positions().any(|c| {
            s.world()
                .chunk(c)
                .is_some_and(|ch| ch.light_data().iter().any(|l| l & 0x0F > 0))
        });
        assert!(lit);
    }

    #[test]
    fn changing_mesher_remeshes_everything() {
        let mut s = streamer();
        let first = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        s.set_mesher(Mesher::Culling);
        let second = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        assert_eq!(meshed(&first).len(), meshed(&second).len());
        // En culling, il y a forcément au moins autant de triangles.
        let triangles = |events: &[StreamEvent]| -> usize {
            meshed(events).values().map(|m| m.triangle_count()).sum()
        };
        assert!(triangles(&second) > triangles(&first));
    }

    // ---------- Priorités, file des maillages et budget (lot 6) ----------

    /// Les positions de la file de chargement, dans l'ordre où elles partiront.
    fn load_order(s: &ChunkStreamer) -> Vec<IVec3> {
        s.load_queue.iter().copied().collect()
    }

    fn streamer_with(column_order: bool, view_bias: f32) -> ChunkStreamer {
        let config = StreamingConfig {
            view_radius: 6,
            unload_margin: 1,
            column_order,
            view_bias,
            ..StreamingConfig::default()
        };
        ChunkStreamer::new(TerrainGenerator::new(7), config)
    }

    /// La distance horizontale au carré d'un chunk au centre (0, y, 0).
    fn flat_sq(p: IVec3) -> i32 {
        p.x * p.x + p.z * p.z
    }

    #[test]
    fn columns_are_completed_from_the_nearest_to_the_farthest() {
        let mut s = streamer_with(true, 0.0);
        s.update(Vec3::new(5.0, 40.0, 5.0));
        let order = load_order(&s);
        assert!(order.len() > 500);
        // La distance horizontale ne diminue jamais : une colonne est finie avant la suivante.
        assert!(order.windows(2).all(|w| flat_sq(w[0]) <= flat_sq(w[1])));
        // La colonne du joueur est partie la première : ses chunks sont déjà en cours de génération.
        assert!(!order.iter().take(3).any(|p| p.x == 0 && p.z == 0));
        assert!(s.stats().in_flight >= 6);
    }

    #[test]
    fn the_legacy_order_interleaves_the_columns() {
        // L'ancien tri par distance 3D mélange les colonnes : le haut d'une colonne proche passe
        // après le bas d'une colonne plus loin (ce que `column_order` corrige).
        let mut s = streamer_with(false, 0.0);
        s.update(Vec3::new(5.0, 40.0, 5.0));
        let order = load_order(&s);
        assert!(order.windows(2).any(|w| flat_sq(w[0]) > flat_sq(w[1])));
    }

    #[test]
    fn what_the_camera_looks_at_loads_first_without_starving_the_back() {
        let mut s = streamer_with(true, 0.6);
        s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::X);
        let order = load_order(&s);
        let index = |x: i32, z: i32| {
            order
                .iter()
                .position(|p| p.x == x && p.z == z && p.y == 2)
                .unwrap_or_else(|| panic!("({x}, {z}) manque"))
        };
        // À même distance, devant avant derrière.
        assert!(index(5, 0) < index(-5, 0));
        // Mais pas de famine : une colonne proche de derrière passe avant une lointaine de devant.
        assert!(index(-3, 0) < index(8, 0));
        assert!(index(-5, 0) < index(8, 0));
        // Aucun biais sous 3 chunks : le sol sous les pieds ne dépend pas du regard.
        assert!(index(-2, 0) < index(3, 0));
    }

    #[test]
    fn turning_the_camera_resorts_the_queue_and_a_small_turn_does_not() {
        let mut s = streamer_with(true, 0.6);
        s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::X);
        let ahead = load_order(&s);
        // Un petit pas de côté (moins de 25 degrés) ne retrie rien.
        s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::new(1.0, 0.0, 0.2));
        assert_eq!(load_order(&s)[..50], ahead[..50]);
        // Un demi-tour renverse les préférences.
        s.update_view(Vec3::new(5.0, 40.0, 5.0), -Vec3::X);
        let behind = load_order(&s);
        let index =
            |order: &[IVec3], x: i32| order.iter().position(|p| p.x == x && p.z == 0 && p.y == 2);
        assert!(index(&ahead, 5) < index(&ahead, -5));
        assert!(index(&behind, -5) < index(&behind, 5));
    }

    /// Fait tourner le streamer sans rien envoyer : tout reste en attente dans `waiting`.
    fn settle_without_draining(s: &mut ChunkStreamer, player: Vec3) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            s.update(player);
            let working = s.load_queue.len()
                + s.in_flight
                + s.light_candidates.len()
                + s.mesh_candidates.len();
            if working == 0 {
                return;
            }
            assert!(Instant::now() < deadline, "le chargement ne se termine pas");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn waiting_meshes_leave_nearest_first_and_the_budget_limits_each_frame() {
        let mut s = streamer();
        let player = Vec3::new(5.0, 40.0, 5.0);
        settle_without_draining(&mut s, player);
        let stats = s.stats();
        assert!(
            stats.meshes_waiting > 10 && stats.bytes_waiting > 0,
            "{stats:?}"
        );
        // Un budget d'un seul octet : un maillage par image (le premier passe toujours).
        let centre = World::chunk_containing(player);
        let mut distances = Vec::new();
        let mut frames = 0;
        while s.stats().meshes_waiting > 0 && frames < 10_000 {
            frames += 1;
            let mut budget = TransferBudget::new(1, Duration::MAX, 0);
            let mut sent = 0;
            while let Some(event) = s.next_event_budgeted(&mut budget) {
                if let StreamEvent::Mesh(pos, mesh) = event {
                    budget.record_upload(mesh.byte_size(), Duration::ZERO);
                    distances.push(flat_sq(pos - centre));
                    sent += 1;
                }
            }
            assert!(
                sent <= 1,
                "plus d'un maillage dans une image à budget minimal"
            );
        }
        assert!(
            distances.windows(2).all(|w| w[0] <= w[1]),
            "du plus proche au plus lointain"
        );
        assert!(distances.len() > 10);
    }

    #[test]
    fn a_big_budget_sends_everything_in_one_frame_and_unloads_are_spread() {
        let mut s = streamer();
        let player = Vec3::new(5.0, 40.0, 5.0);
        settle_without_draining(&mut s, player);
        let waiting = s.stats().meshes_waiting;
        let mut budget = TransferBudget::unlimited();
        let sent = std::iter::from_fn(|| s.next_event_budgeted(&mut budget))
            .filter(|e| matches!(e, StreamEvent::Mesh(..)))
            .count();
        assert_eq!(sent, waiting);
        // Une téléportation : beaucoup de libérations, étalées par le budget.
        s.update(Vec3::new(30.0 * 32.0, 40.0, 5.0));
        let queued = s.stats().unloads_waiting;
        assert!(queued > 30, "{queued}");
        let mut frame = TransferBudget::new(usize::MAX, Duration::MAX, 10);
        let unloaded = std::iter::from_fn(|| {
            let event = s.next_event_budgeted(&mut frame)?;
            if let StreamEvent::Unload(_) = event {
                frame.record_unload(Duration::ZERO);
            }
            Some(event)
        })
        .filter(|e| matches!(e, StreamEvent::Unload(_)))
        .count();
        assert_eq!(unloaded, 10);
        assert_eq!(s.stats().unloads_waiting, queued - 10);
    }

    #[test]
    fn a_mesh_that_comes_back_cancels_its_pending_unload() {
        let mut s = streamer();
        let home = Vec3::new(5.0, 40.0, 5.0);
        settle_without_draining(&mut s, home);
        // On s'éloigne (toutes les libérations en attente), puis on revient avant de les envoyer.
        s.update(Vec3::new(40.0 * 32.0, 40.0, 5.0));
        assert!(s.stats().unloads_waiting > 20);
        settle_without_draining(&mut s, home);
        let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
        let meshed: HashSet<IVec3> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Mesh(p, _) => Some(*p),
                _ => None,
            })
            .collect();
        assert!(!meshed.is_empty());
        // Un Unload après un Mesh du même chunk l'effacerait côté GPU : il n'y en a jamais.
        let mut seen = HashSet::default();
        for event in &events {
            match event {
                StreamEvent::Mesh(p, _) => {
                    seen.insert(*p);
                }
                StreamEvent::Unload(p) => {
                    assert!(!seen.contains(p), "{p} libéré après son maillage")
                }
            }
        }
    }

    #[test]
    fn a_chunk_edited_while_meshing_is_remeshed_once_more_not_once_per_edit() {
        let mut s = streamer();
        let player = Vec3::new(5.0, 40.0, 5.0);
        pump(&mut s, player);
        let target = IVec3::new(0, 1, 0);
        let (done, discarded) = (s.jobs_done, s.jobs_discarded);
        // Dix modifications visuelles d'affilée, bien plus vite qu'un maillage ne tourne.
        for _ in 0..10 {
            s.invalidate_block_visuals(IVec3::new(5, 40, 5));
            s.update(player);
        }
        assert!(s.in_flight <= 8, "{}", s.in_flight);
        let events = pump(&mut s, player);
        // Au plus deux maillages par chunk touché (le premier, puis un seul de rattrapage), et aucun
        // résultat jeté : avant, chacune des dix modifications lançait sa tâche et jetait la précédente.
        let touched = [target, IVec3::new(0, 2, 0), IVec3::new(0, 0, 0)];
        let jobs = s.jobs_done - done;
        assert!(
            jobs <= 2 * touched.len() as u64,
            "{jobs} tâches pour dix modifications"
        );
        assert_eq!(s.jobs_discarded - discarded, 0);
        let latest = meshed(&events);
        assert!(
            latest.contains_key(&target),
            "le chunk modifié a été remaillé"
        );
    }
}
