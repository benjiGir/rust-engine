//! Le monde infini : charger et mailler les chunks autour du joueur, en tâche de fond.
//!
//! # Qui fait quoi
//!
//! - Le **thread principal** possède le [`World`] et l'état de chaque chunk. Il décide
//!   quoi charger, lance des tâches, et reçoit leurs résultats par un canal.
//! - Les **threads de fond** (un pool `rayon` à nous) génèrent les chunks et
//!   calculent les maillages. Ils ne touchent jamais au `World`.
//!
//! Le pool laisse deux cœurs libres : un pour le thread principal (rendu, entrées),
//! un pour le système et le pilote graphique. Avec un thread de fond par cœur,
//! le thread principal devait parfois attendre son tour : des images à 40 ms.
//!
//! # Pourquoi pas de `Mutex` sur le monde ?
//!
//! Un `Mutex<World>` obligerait chaque thread à attendre son tour pour lire un bloc :
//! les threads passeraient leur temps à se bloquer les uns les autres. Ici, chacun
//! travaille sur ses propres données :
//! - une tâche de génération ne reçoit qu'une position et un `Arc<TerrainGenerator>`
//!   (lecture seule), et renvoie un `Chunk` tout neuf ;
//! - une tâche de maillage reçoit un [`Neighbourhood`], des `Arc<Chunk>` en lecture
//!   seule. Le thread principal peut continuer à lire ces mêmes chunks en même temps.
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

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use glam::{IVec3, Vec3};

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::generator::{TerrainGenerator, WORLD_HEIGHT_CHUNKS};
use crate::mesh::{ChunkMesh, Mesher, Neighbourhood};
use crate::world::World;

/// Les réglages du chargement.
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Rayon (horizontal, en chunks) dans lequel les chunks sont maillés et dessinés.
    pub view_radius: i32,
    /// Hystérésis : un chunk n'est déchargé qu'à `view_radius + 1 + unload_margin`.
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
            mesher: Mesher::Greedy,
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
    /// Chunks qui attendent d'être générés.
    pub queued: usize,
    /// Tâches en cours sur les threads de fond.
    pub in_flight: usize,
    /// Maillages prêts, pas encore envoyés au GPU.
    pub meshes_waiting: usize,
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
    /// Une tâche de maillage tourne, lancée pendant cette « époque ».
    Meshing(u32),
    Done,
}

/// Ce que renvoient les threads de fond.
enum JobResult {
    Generated {
        pos: IVec3,
        chunk: Option<Chunk>,
    },
    Meshed {
        pos: IVec3,
        epoch: u32,
        mesh: ChunkMesh,
    },
}

/// Charge, maille et décharge les chunks autour d'un point (le joueur).
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
    /// Chunks à générer, du plus proche au plus lointain.
    load_queue: VecDeque<IVec3>,
    /// Chunks peut-être prêts à être maillés (on vérifie au moment de lancer).
    mesh_candidates: HashSet<IVec3>,
    /// Le chunk du joueur lors du dernier tri.
    centre: Option<IVec3>,
    /// Change à chaque changement de mailleur : les résultats d'une ancienne époque sont jetés.
    epoch: u32,
    in_flight: usize,
    // Chaque tâche reçoit un clone du `Sender` ; le `Receiver` reste ici.
    sender: Sender<JobResult>,
    receiver: Receiver<JobResult>,
    events: VecDeque<StreamEvent>,
    /// Chunks chargés modifiés par le joueur depuis leur génération.
    edited: HashSet<IVec3>,
    /// Chunks modifiés puis déchargés : on les garde en mémoire et on les rend
    /// au lieu de les régénérer, sinon les modifications seraient perdues.
    /// (La sauvegarde sur disque viendra plus tard.)
    saved_edits: HashMap<IVec3, Arc<Chunk>>,
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
            world: World::with_registry(generator.registry().clone()),
            generator: Arc::new(generator),
            slots: HashMap::new(),
            load_queue: VecDeque::new(),
            mesh_candidates: HashSet::new(),
            centre: None,
            epoch: 0,
            in_flight: 0,
            sender,
            receiver,
            events: VecDeque::new(),
            edited: HashSet::new(),
            saved_edits: HashMap::new(),
        }
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn generator(&self) -> &TerrainGenerator {
        &self.generator
    }

    pub fn config(&self) -> &StreamingConfig {
        &self.config
    }

    /// Vrai si les blocs de ce chunk sont connus (généré, ou hors de la hauteur du monde,
    /// où il n'y a que de l'air). Le joueur ne doit pas bouger tant que son chunk n'est
    /// pas prêt, sinon il tomberait à travers un sol pas encore chargé.
    pub fn is_generated(&self, chunk: IVec3) -> bool {
        !(0..WORLD_HEIGHT_CHUNKS).contains(&chunk.y)
            || matches!(self.slots.get(&chunk), Some(Slot::Generated(_)))
    }

    /// Change un bloc et remaille tout de suite ce qui doit l'être.
    ///
    /// Renvoie `false` (et ne fait rien) si le chunk n'est pas encore généré.
    /// Les maillages arrivent en tête de la file d'événements : le jeu les envoie
    /// au GPU dès cette image, sans attendre derrière le chargement normal.
    pub fn set_block(&mut self, pos: IVec3, block: BlockId) -> bool {
        let (chunk, local) = World::split(pos);
        if !matches!(self.slots.get(&chunk), Some(Slot::Generated(_))) {
            return false;
        }
        if self.world.block(pos) == block {
            return true;
        }
        // Copy-on-write : si une tâche de maillage lit encore ce chunk, elle garde
        // l'ancienne version et nous modifions une copie (voir `World::chunk_mut`).
        self.world.set_block(pos, block);
        self.edited.insert(chunk);

        // Un bloc au bord du chunk change aussi les faces du voisin qui le touche.
        let mut to_remesh = vec![chunk];
        for axis in 0..3 {
            let mut offset = IVec3::ZERO;
            if local[axis] == 0 {
                offset[axis] = -1;
            } else if local[axis] == CHUNK_SIZE - 1 {
                offset[axis] = 1;
            } else {
                continue;
            }
            to_remesh.push(chunk + offset);
        }
        // Dans l'ordre inverse : `push_front` met donc le chunk modifié en premier.
        for pos in to_remesh.into_iter().rev() {
            self.remesh_now(pos);
        }
        true
    }

    /// Remaille un chunk sur ce thread, immédiatement.
    ///
    /// Un maillage prend moins d'une demi-milliseconde : pour 1 à 4 chunks, c'est plus
    /// simple et plus réactif que de passer par les threads de fond, dont la file est
    /// peut-être pleine de chunks à charger.
    fn remesh_now(&mut self, pos: IVec3) {
        if !matches!(self.slots.get(&pos), Some(Slot::Generated(_))) {
            return;
        }
        if !self.neighbours_generated(pos) {
            // Il sera maillé normalement quand ses voisins arriveront.
            self.slots.insert(pos, Slot::Generated(MeshState::Missing));
            self.mesh_candidates.insert(pos);
            return;
        }
        let mesh = Neighbourhood::from_world(&self.world, pos)
            .map_or_else(ChunkMesh::default, |hood| self.config.mesher.mesh(&hood));
        // `Done` : si une tâche de fond maillait l'ancienne version, son résultat
        // (qui attend `Meshing`) sera jeté à l'arrivée.
        self.slots.insert(pos, Slot::Generated(MeshState::Done));
        // Un ancien maillage en attente ne doit pas écraser le nouveau.
        self.events
            .retain(|e| !matches!(e, StreamEvent::Mesh(p, _) if *p == pos));
        // Un maillage vide est envoyé aussi : le jeu retire alors l'ancien.
        self.events.push_front(StreamEvent::Mesh(pos, mesh));
    }

    /// Change d'algorithme de maillage et remaille tout ce qui est chargé.
    pub fn set_mesher(&mut self, mesher: Mesher) {
        self.config.mesher = mesher;
        self.epoch += 1;
        for (&pos, slot) in &mut self.slots {
            if let Slot::Generated(state @ (MeshState::Meshing(_) | MeshState::Done)) = slot {
                *state = MeshState::Missing;
                self.mesh_candidates.insert(pos);
            }
        }
    }

    /// Une image : suivre le joueur, récupérer les résultats, lancer de nouvelles tâches.
    pub fn update(&mut self, player: Vec3) {
        let centre = World::chunk_containing(player);
        if self.centre != Some(centre) {
            self.centre = Some(centre);
            self.refresh(centre);
        }
        self.receive();
        self.schedule(centre);
    }

    /// Le prochain événement à traiter (maillage à envoyer au GPU, chunk à libérer).
    pub fn next_event(&mut self) -> Option<StreamEvent> {
        self.events.pop_front()
    }

    pub fn stats(&self) -> StreamStats {
        StreamStats {
            loaded: self
                .slots
                .values()
                .filter(|s| matches!(s, Slot::Generated(_)))
                .count(),
            queued: self.load_queue.len(),
            in_flight: self.in_flight,
            meshes_waiting: self
                .events
                .iter()
                .filter(|e| matches!(e, StreamEvent::Mesh(..)))
                .count(),
        }
    }

    /// Plus rien à faire tant que le joueur ne bouge pas.
    pub fn is_idle(&self) -> bool {
        self.load_queue.is_empty()
            && self.in_flight == 0
            && self.mesh_candidates.is_empty()
            && self.events.is_empty()
    }

    /// Le joueur a changé de chunk : décharger ce qui est trop loin, retrier la file.
    fn refresh(&mut self, centre: IVec3) {
        let load_radius = self.config.view_radius + 1;
        let unload_radius = load_radius + self.config.unload_margin;

        // 1. Déchargement. `retain` garde les éléments pour lesquels la closure renvoie `true`.
        let mut unloaded = Vec::new();
        self.slots.retain(|&pos, _| {
            let keep = horizontal_distance_sq(pos, centre) <= unload_radius * unload_radius;
            if !keep {
                unloaded.push(pos);
            }
            keep
        });
        for pos in unloaded {
            // Le dernier `Arc` disparaît ici (ou à la fin de la tâche qui le lit encore) :
            // `Drop` libère alors les 64 Ko du chunk. Côté GPU, c'est pareil : le jeu
            // retire le maillage, et les `wgpu::Buffer` sont libérés par leur `Drop`.
            let chunk = self.world.remove(pos);
            if self.edited.remove(&pos) {
                // Un chunk modifié devenu tout vide n'est plus dans le monde : on garde un chunk d'air.
                self.saved_edits.insert(pos, chunk.unwrap_or_default());
            }
            self.mesh_candidates.remove(&pos);
            self.events
                .retain(|e| !matches!(e, StreamEvent::Mesh(p, _) if *p == pos));
            self.events.push_back(StreamEvent::Unload(pos));
        }

        // 2. Nouvelle file de chargement, triée du plus proche au plus lointain.
        let mut wanted = Vec::new();
        for dx in -load_radius..=load_radius {
            for dz in -load_radius..=load_radius {
                if dx * dx + dz * dz > load_radius * load_radius {
                    continue;
                }
                for y in 0..WORLD_HEIGHT_CHUNKS {
                    let pos = IVec3::new(centre.x + dx, y, centre.z + dz);
                    if !self.slots.contains_key(&pos) {
                        wanted.push(pos);
                    }
                }
            }
        }
        // `sort_by_key` trie selon la valeur renvoyée : ici la distance au carré
        // (inutile de calculer la racine pour comparer des distances).
        wanted.sort_by_key(|&pos| (pos - centre).length_squared());
        self.load_queue = wanted.into();

        // 3. Les chunks déjà générés qui entrent dans le rayon de vue doivent être maillés.
        let view_sq = self.config.view_radius * self.config.view_radius;
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
            match result {
                JobResult::Generated { pos, chunk } => {
                    // Le chunk a pu être déchargé pendant sa génération : on jette le résultat.
                    if self.slots.get(&pos) != Some(&Slot::Generating) {
                        continue;
                    }
                    if let Some(chunk) = chunk {
                        self.world.insert(pos, Arc::new(chunk));
                    }
                    self.slots.insert(pos, Slot::Generated(MeshState::Missing));
                    // Ce chunk, et ses voisins qui n'attendaient peut-être que lui.
                    self.mesh_candidates.insert(pos);
                    for offset in NEIGHBOUR_OFFSETS {
                        self.mesh_candidates.insert(pos + offset);
                    }
                }
                JobResult::Meshed { pos, epoch, mesh } => {
                    if self.slots.get(&pos) == Some(&Slot::Generated(MeshState::Meshing(epoch))) {
                        self.slots.insert(pos, Slot::Generated(MeshState::Done));
                        self.events.push_back(StreamEvent::Mesh(pos, mesh));
                    }
                }
            }
        }
    }

    /// Lance des tâches tant qu'il reste de la place : maillages d'abord
    /// (ils rendent des chunks visibles), puis générations.
    fn schedule(&mut self, centre: IVec3) {
        if self.in_flight < self.config.max_jobs_in_flight && !self.mesh_candidates.is_empty() {
            let mut candidates: Vec<IVec3> = self.mesh_candidates.iter().copied().collect();
            candidates.sort_by_key(|&pos| (pos - centre).length_squared());
            for pos in candidates {
                if self.in_flight >= self.config.max_jobs_in_flight {
                    break; // Les autres restent candidats pour l'image suivante.
                }
                self.mesh_candidates.remove(&pos);
                if self.ready_to_mesh(pos, centre) {
                    self.spawn_mesh(pos);
                }
            }
        }

        while self.in_flight < self.config.max_jobs_in_flight {
            let Some(pos) = self.load_queue.pop_front() else {
                break;
            };
            // L'API `entry` : une seule recherche dans la table pour tester et insérer.
            if let Entry::Vacant(entry) = self.slots.entry(pos) {
                // Un chunk modifié par le joueur revient tel qu'il l'a laissé.
                if let Some(chunk) = self.saved_edits.remove(&pos) {
                    entry.insert(Slot::Generated(MeshState::Missing));
                    self.world.insert(pos, chunk);
                    self.edited.insert(pos);
                    self.mesh_candidates.insert(pos);
                    for offset in NEIGHBOUR_OFFSETS {
                        self.mesh_candidates.insert(pos + offset);
                    }
                } else {
                    entry.insert(Slot::Generating);
                    self.spawn_generation(pos);
                }
            }
        }
    }

    /// On ne maille un chunk que quand ses 6 voisins sont générés : ses faces de
    /// bordure sont alors justes du premier coup. Si on maillait plus tôt, il faudrait
    /// le remailler à l'arrivée de chaque voisin (jusqu'à 6 fois le même travail).
    /// C'est pour ça qu'on génère un chunk de plus que le rayon de vue.
    fn ready_to_mesh(&self, pos: IVec3, centre: IVec3) -> bool {
        let view = self.config.view_radius;
        self.slots.get(&pos) == Some(&Slot::Generated(MeshState::Missing))
            && horizontal_distance_sq(pos, centre) <= view * view
            && self.neighbours_generated(pos)
    }

    /// Les 6 voisins sont générés (au-dessus et en dessous du monde, rien à attendre).
    fn neighbours_generated(&self, pos: IVec3) -> bool {
        NEIGHBOUR_OFFSETS
            .iter()
            .all(|&offset| self.is_generated(pos + offset))
    }

    fn spawn_generation(&mut self, pos: IVec3) {
        // On clone l'`Arc` (un compteur +1) et le `Sender` : la closure `move`
        // en prend possession et les emporte sur le thread de fond.
        let generator = Arc::clone(&self.generator);
        let sender = self.sender.clone();
        self.in_flight += 1;
        self.pool.spawn(move || {
            let chunk = generator.generate(pos);
            // Si le jeu s'est fermé entre-temps, le `Receiver` n'existe plus : on ignore.
            let _ = sender.send(JobResult::Generated { pos, chunk });
        });
    }

    fn spawn_mesh(&mut self, pos: IVec3) {
        // Un chunk vide n'a rien à dessiner : pas besoin de tâche.
        let Some(hood) = Neighbourhood::from_world(&self.world, pos) else {
            self.slots.insert(pos, Slot::Generated(MeshState::Done));
            return;
        };
        let epoch = self.epoch;
        let mesher = self.config.mesher;
        let sender = self.sender.clone();
        self.slots
            .insert(pos, Slot::Generated(MeshState::Meshing(epoch)));
        self.in_flight += 1;
        self.pool.spawn(move || {
            let mesh = mesher.mesh(&hood);
            let _ = sender.send(JobResult::Meshed { pos, epoch, mesh });
        });
    }
}

const NEIGHBOUR_OFFSETS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Y,
    IVec3::NEG_Y,
    IVec3::Z,
    IVec3::NEG_Z,
];

/// Distance horizontale au carré, en chunks : le rayon de vue est un cercle, pas une sphère.
fn horizontal_distance_sq(a: IVec3, b: IVec3) -> i32 {
    let d = a - b;
    d.x * d.x + d.z * d.z
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::mesh::mesh_greedy;
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

        // Rayon de chargement = rayon de vue + 1.
        let r = RADIUS + 1;
        let columns = (-r..=r)
            .flat_map(|x| (-r..=r).map(move |z| x * x + z * z))
            .filter(|&d| d <= r * r)
            .count();
        assert_eq!(s.stats().loaded, columns * WORLD_HEIGHT_CHUNKS as usize);

        // Tout chunk non vide dans le rayon de vue a reçu un maillage, et seulement eux.
        let meshes = meshed(&events);
        assert!(!meshes.is_empty());
        for pos in meshes.keys() {
            assert!(horizontal_distance_sq(*pos, IVec3::ZERO) <= RADIUS * RADIUS);
        }
        for pos in s.world().chunk_positions() {
            let inside = horizontal_distance_sq(pos, IVec3::ZERO) <= RADIUS * RADIUS;
            let has_faces = !mesh_greedy(s.world(), pos).is_empty();
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
    fn streamed_meshes_match_direct_meshing() {
        let mut s = streamer();
        let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
        let meshes = meshed(&events);
        for (pos, mesh) in meshes {
            assert_eq!(
                mesh.vertices,
                mesh_greedy(s.world(), pos).vertices,
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
        // Il ne reste que des chunks proches de la nouvelle position (rayon 2 + 1 + marge 1).
        assert!(
            s.slots
                .keys()
                .all(|p| horizontal_distance_sq(*p, IVec3::new(20, 0, 0)) <= 4 * 4)
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
                    assert_eq!(mesh, &mesh_greedy(s.world(), *p), "maillage à jour");
                    *p
                }
                StreamEvent::Unload(p) => panic!("déchargement inattendu de {p}"),
            })
            .collect();
        assert_eq!(positions, [IVec3::new(0, 1, 0), IVec3::new(1, 1, 0)]);
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
    fn mod_blocks_can_be_placed_and_meshed() {
        use crate::registry::BlockRegistry;
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let registry = BlockRegistry::load(&repo.join("assets"), Some(&repo.join("mods"))).unwrap();
        let ruby = registry.id("example:ruby").unwrap();
        let ruby_color = registry.def(ruby).color;
        let generator = TerrainGenerator::with_registry(7, Arc::new(registry)).unwrap();
        let config = StreamingConfig {
            view_radius: RADIUS,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(generator, config);
        pump(&mut s, Vec3::new(5.0, 40.0, 5.0));

        // Un rubis seul dans le ciel : 6 faces, toutes de sa couleur.
        let pos = IVec3::new(8, 120, 8);
        assert!(s.set_block(pos, ruby));
        let Some(StreamEvent::Mesh(_, mesh)) = s.next_event() else {
            panic!("pas de maillage");
        };
        assert_eq!(mesh.quad_count(), 6);
        assert!(mesh.vertices.iter().all(|v| v.color == ruby_color));
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
}
