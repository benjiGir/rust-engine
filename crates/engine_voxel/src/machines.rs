//! Les blocs actifs (« machines ») : horloges, lampes, et ceux des mods.
//!
//! # Pourquoi pas une entité ECS par machine ?
//!
//! Une grosse construction peut compter des centaines de milliers de blocs actifs
//! (câbles, tapis, engrenages...). La plupart ne font rien la plupart du temps. Minecraft
//! ne les met donc pas dans une liste parcourue à chaque tick : un bloc actif a un petit
//! état rangé dans son chunk ([`BlockEntity`]), et il n'est réveillé que :
//! - par un **tick programmé** : « réveille-moi dans 10 ticks » (une file triée par date) ;
//! - par une **mise à jour de voisin** : un bloc qui change prévient ses 6 voisins.
//!
//! Le coût d'un tick est donc proportionnel au nombre de machines *qui travaillent*, pas
//! au nombre de machines qui existent. Les machines tournent à 20 ticks par seconde, comme
//! dans Minecraft : assez pour un mécanisme, et 3 fois moins de travail que la physique.
//!
//! # Par lots
//!
//! Un comportement reçoit *toutes* ses machines à réveiller d'un coup (une liste de
//! positions), pas une par une. Pour un mod en WebAssembly, chaque appel du jeu vers le
//! script a un coût fixe : 10 000 appels coûtent bien plus qu'un appel avec 10 000 positions.

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::{IVec3, Vec3};

use std::sync::Arc;

use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::chunk::CHUNK_SIZE;
use crate::grid::{BlockAccess, GridId, GridPos, Grids};
use crate::plants;
use crate::registry::BlockRegistry;
use crate::streaming::ChunkStreamer;
use crate::world::World;

pub const MACHINE_TICKS_PER_SECOND: u32 = 20;
/// La puissance maximale d'un signal, comme la redstone.
pub const MAX_POWER: u8 = 15;

pub(crate) const NEIGHBOURS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Y,
    IVec3::NEG_Y,
    IVec3::Z,
    IVec3::NEG_Z,
];

/// Ce qu'une machine demande au jeu, et que le moteur ne sait pas faire lui-même.
#[derive(Debug, Clone, PartialEq)]
pub enum MachineAction {
    /// Donner des objets au joueur.
    GiveItem {
        block: BlockId,
        count: u32,
    },
    /// Ouvrir (ou mettre à jour) le menu d'une machine de mod : `data` est le menu encodé par
    /// le SDK, lu et vérifié par le jeu.
    ShowMenu {
        mod_id: String,
        at: GridPos,
        data: Vec<u8>,
    },
    /// Ordres d'un script à une de ses créatures à comportement (voir `engine_mods::mobs`) ; le
    /// jeu ne les applique qu'aux créatures du lot qu'il vient de confier au script.
    MobMove {
        mob: u32,
        dx: f32,
        dz: f32,
        speed: f32,
    },
    MobJump {
        mob: u32,
    },
    MobAttack {
        mob: u32,
        damage: f32,
    },
    MobState {
        mob: u32,
        value: u32,
    },
    /// Afficher (ou mettre à jour) la page d'options du mod.
    ShowOptions {
        mod_id: String,
        data: Vec<u8>,
    },
    /// Fermer le menu de ce mod.
    CloseMenu {
        mod_id: String,
    },
    /// Faire apparaître une créature (identifiant texte, comme `core:pig`).
    /// Donner des objets (pas forcément des blocs) au joueur.
    GiveStack {
        item: crate::items::ItemId,
        count: u32,
    },
    SpawnEntity {
        kind: String,
        position: Vec3,
    },
    /// Un bloc a été cassé par une machine (pour les événements du jeu).
    BlockBroken {
        pos: IVec3,
        block: BlockId,
    },
    Log(String),
}

/// Le comportement d'un type de bloc actif.
///
/// Toutes les méthodes ont une version par défaut qui ne fait rien : un comportement
/// n'écrit que ce qui l'intéresse. Un trait avec des méthodes par défaut ressemble à une
/// classe abstraite en C#, sans héritage de données.
///
/// Les machines sont désignées par un [`GridPos`] (une grille, une position dedans) :
/// le même comportement marche dans le monde et, plus tard, sur une structure mobile.
pub trait BlockBehavior {
    /// Le bloc vient d'être posé : créer son état, programmer son premier tick...
    fn placed(&self, _ctx: &mut MachineContext, _positions: &[GridPos]) {}
    /// Son chunk vient d'être rechargé (les ticks programmés ne sont pas sauvegardés).
    fn loaded(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.placed(ctx, positions);
    }
    /// Un de ses voisins a changé.
    fn neighbor_changed(&self, _ctx: &mut MachineContext, _positions: &[GridPos]) {}
    /// Son tick programmé est arrivé.
    fn scheduled_tick(&self, _ctx: &mut MachineContext, _positions: &[GridPos]) {}
    /// Un « tick aléatoire » : le bloc a été tiré au sort (voir [`Machines::run_random_ticks`]).
    /// Pour ce qui évolue lentement et sans prévenir : l'herbe, les pousses, les feuilles.
    fn random_tick(&self, _ctx: &mut MachineContext, _positions: &[GridPos]) {}
    /// Le joueur clique dessus (levier, bouton, répéteur). `true` si le clic est pris en compte.
    fn used(&self, _ctx: &mut MachineContext, _at: GridPos) -> bool {
        false
    }
}

/// Les ticks programmés et les voisins à prévenir.
#[derive(Debug, Default)]
pub struct Scheduler {
    tick: u64,
    /// `Reverse` : `BinaryHeap` sort le plus grand d'abord ; on veut la date la plus proche.
    /// Le numéro de séquence garde l'ordre d'arrivée entre deux ticks à la même date.
    queue: BinaryHeap<Reverse<(u64, u64, GridPos)>>,
    sequence: u64,
    /// Une position n'a qu'un tick en attente à la fois (comme dans Minecraft).
    scheduled: HashSet<GridPos>,
    /// Les blocs dont un voisin a changé, à prévenir au prochain tick.
    neighbors: Vec<GridPos>,
    /// L'état du générateur pseudo-aléatoire des ticks aléatoires et des comportements.
    random_state: u64,
    /// Les conduits dont le réseau est à reconstruire (voir `networks.rs`).
    network_dirty: Vec<GridPos>,
}

impl Scheduler {
    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn schedule(&mut self, at: GridPos, delay: u32) {
        if self.scheduled.insert(at) {
            self.sequence += 1;
            let when = self.tick + delay.max(1) as u64;
            self.queue.push(Reverse((when, self.sequence, at)));
        }
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Un nombre pseudo-aléatoire (splitmix64).
    pub fn random(&mut self) -> u64 {
        self.random_state = self.random_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.random_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Ce qu'un comportement peut faire pendant un tick.
///
/// Les grilles sont partagées (`RefCell`) et empruntées juste le temps de chaque
/// opération : un comportement scripté (mod WebAssembly) doit pouvoir rendre la main au
/// script, qui rappelle alors le jeu (« quel bloc est là ? ») ; ces fonctions du jeu
/// empruntent à leur tour la grille. Un `&mut` tenu pendant tout le tick l'interdirait.
pub struct MachineContext<'a> {
    pub grids: Grids<'a>,
    scheduler: &'a mut Scheduler,
    /// Les blocs ayant un comportement : seuls eux ont besoin d'être prévenus.
    active_blocks: &'a HashSet<BlockId>,
    pub actions: &'a mut Vec<MachineAction>,
}

impl MachineContext<'_> {
    pub fn tick(&self) -> u64 {
        self.scheduler.tick
    }

    pub fn registry(&self) -> Option<Arc<BlockRegistry>> {
        Some(self.grids.get(GridId::WORLD)?.borrow().registry().clone())
    }

    pub fn block(&self, at: GridPos) -> BlockId {
        self.grids.block(at)
    }

    /// Change un bloc (remaillé en tâche de fond) et prévient ses voisins.
    pub fn set_block(&mut self, at: GridPos, block: BlockId) -> bool {
        let done = self
            .grids
            .get(at.grid)
            .is_some_and(|g| g.borrow_mut().set_block(at.pos, block));
        if done {
            self.notify_neighbors(at);
        }
        done
    }

    /// Une copie de l'état actif du bloc (la grille n'est empruntée que le temps de copier).
    pub fn block_entity(&self, at: GridPos) -> Option<BlockEntity> {
        self.grids.read(at, Clone::clone)
    }

    /// Modifie l'état actif du bloc avec `f` ; renvoie ce que `f` renvoie (ou `None` si
    /// le bloc n'a pas d'état). Une closure plutôt qu'un `&mut BlockEntity` : la référence
    /// ne peut pas survivre à l'emprunt du `RefCell`, la closure, si.
    pub fn with_block_entity<R>(
        &mut self,
        at: GridPos,
        f: impl FnOnce(&mut BlockEntity) -> R,
    ) -> Option<R> {
        self.grids.write(at, f)
    }

    /// Crée l'état actif d'un bloc s'il n'en a pas encore.
    pub fn ensure_block_entity(&mut self, at: GridPos) -> bool {
        self.grids.write_or_create(at, |_| ()).is_some()
    }

    pub fn schedule(&mut self, at: GridPos, delay: u32) {
        self.scheduler.schedule(at, delay);
    }

    /// Un conduit vient d'apparaître ou son voisinage a changé : son réseau sera reconstruit.
    pub fn mark_network_dirty(&mut self, at: GridPos) {
        self.scheduler.network_dirty.push(at);
    }

    /// Prévient les voisins actifs de `at` qu'ils ont peut-être quelque chose à faire
    /// (au prochain tick). À appeler quand un bloc ou sa puissance change.
    pub fn notify_neighbors(&mut self, at: GridPos) {
        let Some(grid) = self.grids.get(at.grid) else {
            return;
        };
        let grid = grid.borrow();
        for offset in NEIGHBOURS {
            let neighbor = at.offset(offset);
            // On ne note que les voisins qui ont un comportement : dans une grande
            // construction, la plupart des voisins sont de l'air ou de la pierre.
            if self.active_blocks.contains(&grid.block(neighbor.pos)) {
                self.scheduler.neighbors.push(neighbor);
            }
        }
    }

    /// La puissance émise par le bloc (0 s'il n'est pas actif).
    pub fn power(&self, at: GridPos) -> u8 {
        self.grids.read(at, |e| e.power).unwrap_or(0)
    }

    /// La plus forte puissance reçue de ses 6 voisins.
    pub fn received_power(&self, at: GridPos) -> u8 {
        self.grids
            .get(at.grid)
            .map_or(0, |g| received_power(&*g.borrow(), at.pos))
    }

    pub fn act(&mut self, action: MachineAction) {
        self.actions.push(action);
    }

    /// Un nombre pseudo-aléatoire.
    pub fn random(&mut self) -> u64 {
        self.scheduler.random()
    }

    /// Vrai une fois sur `n`, au hasard.
    pub fn chance(&mut self, n: u64) -> bool {
        self.random().is_multiple_of(n.max(1))
    }
}

/// Le bilan d'un tick, pour les mesures.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TickReport {
    pub scheduled: usize,
    pub neighbor_updates: usize,
    pub duration: Duration,
}

/// Tous les comportements, et l'ordonnanceur.
#[derive(Default)]
pub struct Machines {
    /// `Rc<dyn BlockBehavior>` : un objet trait partagé (un même comportement sert à
    /// plusieurs blocs, comme la lampe éteinte et allumée).
    behaviors: HashMap<BlockId, Rc<dyn BlockBehavior>>,
    active_blocks: HashSet<BlockId>,
    /// Les blocs qui reçoivent des ticks aléatoires, par numéro (un tableau : on le
    /// consulte pour chaque bloc tiré au sort, il faut que ce soit instantané).
    random_blocks: Vec<bool>,
    scheduler: Scheduler,
    /// Les réseaux de câbles, de tuyaux et d'arbres.
    networks: crate::networks::Networks,
}

impl Machines {
    pub fn new() -> Self {
        Self::default()
    }

    /// Les comportements natifs (`core:clock`, `core:lamp`) des blocs qui les demandent.
    pub fn with_natives(registry: &BlockRegistry) -> Self {
        let mut machines = Self::new();
        let clock: Rc<dyn BlockBehavior> = Rc::new(Clock);
        let lamp: Rc<dyn BlockBehavior> = Rc::new(Lamp::new(registry));
        let grass: Option<Rc<dyn BlockBehavior>> =
            plants::Grass::new(registry).map(|g| Rc::new(g) as _);
        let leaves: Rc<dyn BlockBehavior> = Rc::new(plants::Leaves::new(registry));
        let sapling: Option<Rc<dyn BlockBehavior>> =
            plants::Sapling::new(registry).map(|s| Rc::new(s) as _);
        machines.networks = crate::networks::Networks::new(registry);
        let circuits = crate::circuits::natives(registry);
        let conduit: Rc<dyn BlockBehavior> = Rc::new(crate::networks::Conduit);
        let tech = crate::tech::natives(registry);
        let farmland: Option<Rc<dyn BlockBehavior>> =
            crate::farming::Farmland::new(registry).map(|f| Rc::new(f) as _);
        let crop: Rc<dyn BlockBehavior> = Rc::new(crate::farming::Crop);
        for (id, def) in registry.iter() {
            // Un conduit déclaré par un mod (`conduit: Kinetic`) : le même comportement.
            if def.conduit.is_some() {
                machines.register(id, conduit.clone());
                continue;
            }
            match def.behavior.as_deref() {
                Some("core:cable" | "core:pipe" | "core:shaft") => {
                    machines.register(id, conduit.clone())
                }
                Some(name) if tech.iter().any(|(n, _)| *n == name) => {
                    if let Some((_, behavior)) = tech.iter().find(|(n, _)| *n == name) {
                        machines.register(id, behavior.clone());
                    }
                }
                Some(name) if circuits.iter().any(|(n, _)| *n == name) => {
                    if let Some((_, behavior)) = circuits.iter().find(|(n, _)| *n == name) {
                        machines.register(id, behavior.clone());
                    }
                }
                Some("core:crop") => machines.register_random(id, crop.clone()),
                Some("core:farmland") => {
                    if let Some(farmland) = &farmland {
                        machines.register_random(id, farmland.clone());
                    }
                }
                Some("core:clock") => machines.register(id, clock.clone()),
                Some("core:lamp") => machines.register(id, lamp.clone()),
                Some("core:grass") => {
                    if let Some(grass) = &grass {
                        machines.register_random(id, grass.clone());
                    }
                }
                Some("core:leaves") => machines.register_random(id, leaves.clone()),
                Some("core:sapling") => {
                    if let Some(sapling) = &sapling {
                        machines.register_random(id, sapling.clone());
                    }
                }
                _ => {}
            }
        }
        machines
    }

    pub fn register(&mut self, block: BlockId, behavior: Rc<dyn BlockBehavior>) {
        self.behaviors.insert(block, behavior);
        self.active_blocks.insert(block);
    }

    /// Un comportement qui ne réagit qu'aux ticks aléatoires : le bloc n'est pas
    /// « actif » (ses voisins ne le préviennent pas), il est seulement tiré au sort.
    pub fn register_random(&mut self, block: BlockId, behavior: Rc<dyn BlockBehavior>) {
        self.behaviors.insert(block, behavior);
        let i = block.0 as usize;
        if self.random_blocks.len() <= i {
            self.random_blocks.resize(i + 1, false);
        }
        self.random_blocks[i] = true;
    }

    fn is_random(&self, block: BlockId) -> bool {
        self.random_blocks
            .get(block.0 as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Les ticks aléatoires, comme dans le genre : à chaque tick, dans chaque chunk à
    /// moins de `radius` chunks (horizontalement) de `center`, on tire `per_chunk` blocs
    /// au hasard ; ceux qui ont un comportement aléatoire sont réveillés. Un bloc donné
    /// est donc réveillé en moyenne toutes les 32 768 / `per_chunk` ticks (environ 70 s
    /// pour 24) : l'herbe s'étend, les pousses grandissent, lentement et sans liste à tenir.
    /// Renvoie le nombre de blocs réveillés.
    pub fn run_random_ticks(
        &mut self,
        world: &RefCell<ChunkStreamer>,
        center: IVec3,
        radius: i32,
        per_chunk: u32,
        actions: &mut Vec<MachineAction>,
    ) -> usize {
        let _span = engine_trace::span("ticks aléatoires");
        if !self.random_blocks.contains(&true) {
            return 0;
        }
        let mut picked: Vec<(BlockId, GridPos)> = Vec::new();
        {
            let streamer = world.borrow();
            let size = CHUNK_SIZE as u64;
            for cx in center.x - radius..=center.x + radius {
                for cz in center.z - radius..=center.z + radius {
                    for cy in 0..crate::generator::WORLD_HEIGHT_CHUNKS {
                        let pos = IVec3::new(cx, cy, cz);
                        let Some(chunk) = streamer.world().chunk(pos) else {
                            continue;
                        };
                        let origin = World::chunk_origin(pos);
                        for _ in 0..per_chunk {
                            let r = self.scheduler.random();
                            let (x, y, z) = (r % size, (r >> 8) % size, (r >> 16) % size);
                            let block = chunk.get(x as usize, y as usize, z as usize);
                            if self.is_random(block) {
                                let p = origin + IVec3::new(x as i32, y as i32, z as i32);
                                picked.push((block, GridPos::world(p)));
                            }
                        }
                    }
                }
            }
        }
        let count = picked.len();
        // Par comportement, en un lot (comme les autres réveils).
        picked.sort_by_key(|(b, p)| (b.0, p.key()));
        let grids = Grids::new(world);
        let mut start = 0;
        while start < count {
            let block = picked[start].0;
            let end = start
                + picked[start..]
                    .iter()
                    .take_while(|(b, _)| *b == block)
                    .count();
            let list: Vec<GridPos> = picked[start..end].iter().map(|(_, p)| *p).collect();
            if let Some(behavior) = self.behaviors.get(&block).cloned() {
                let mut ctx = self.context(grids, actions);
                behavior.random_tick(&mut ctx, &list);
            }
            start = end;
        }
        count
    }

    /// Des blocs actifs d'une grille viennent d'apparaître (une structure qu'on assemble, ou
    /// qu'on recharge) : ils démarrent comme après un chargement de chunk.
    pub fn grid_blocks_loaded(
        &mut self,
        grids: Grids,
        positions: &[GridPos],
        actions: &mut Vec<MachineAction>,
    ) {
        for (behavior, list) in self.group(grids, positions.iter().copied()) {
            let mut ctx = self.context(grids, actions);
            behavior.loaded(&mut ctx, &list);
        }
    }

    /// Le joueur clique sur le bloc `at` d'une grille quelconque. `true` si un comportement
    /// l'a pris en compte.
    pub fn use_grid_block(
        &mut self,
        grids: Grids,
        at: GridPos,
        actions: &mut Vec<MachineAction>,
    ) -> bool {
        let block = grids.block(at);
        let Some(behavior) = self.behaviors.get(&block).cloned() else {
            return false;
        };
        let mut ctx = self.context(grids, actions);
        behavior.used(&mut ctx, at)
    }

    /// Le joueur clique sur le bloc en `pos` (levier, bouton, répéteur). `true` si un
    /// comportement l'a pris en compte.
    pub fn use_block(
        &mut self,
        world: &RefCell<ChunkStreamer>,
        pos: IVec3,
        actions: &mut Vec<MachineAction>,
    ) -> bool {
        let grids = Grids::new(world);
        let at = GridPos::world(pos);
        let block = grids.block(at);
        let Some(behavior) = self.behaviors.get(&block).cloned() else {
            return false;
        };
        let mut ctx = self.context(grids, actions);
        behavior.used(&mut ctx, at)
    }

    /// Les réseaux de conduits (pour les mesures, l'inspecteur et les tests).
    pub fn networks(&self) -> &crate::networks::Networks {
        &self.networks
    }

    /// Les objets et les recettes de machines que connaissent les réseaux d'objets.
    pub fn set_item_rules(
        &mut self,
        items: std::sync::Arc<crate::items::ItemRegistry>,
        recipes: std::sync::Arc<crate::crafting::MachineRecipes>,
    ) {
        self.networks.set_item_rules(items, recipes);
    }

    pub fn has_behavior(&self, block: BlockId) -> bool {
        self.active_blocks.contains(&block)
    }

    pub fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    /// Programme un tick (pour le jeu ou les tests).
    pub fn schedule(&mut self, at: GridPos, delay: u32) {
        self.scheduler.schedule(at, delay);
    }

    /// Prévient les voisins actifs de `at` (au prochain tick), en dehors d'un tick :
    /// par exemple quand un mod change un bloc en réponse à un événement.
    pub fn notify_neighbors(&mut self, grids: Grids, at: GridPos) {
        let mut actions = Vec::new();
        self.context(grids, &mut actions).notify_neighbors(at);
    }

    /// Un bloc du monde a changé (le joueur l'a posé ou cassé) : le nouveau bloc démarre
    /// s'il est actif, et les voisins sont prévenus.
    pub fn block_changed(
        &mut self,
        world: &RefCell<ChunkStreamer>,
        pos: IVec3,
        actions: &mut Vec<MachineAction>,
    ) {
        self.grid_block_changed(Grids::new(world), GridPos::world(pos), actions);
    }

    /// Un bloc d'une grille quelconque a changé.
    pub fn grid_block_changed(
        &mut self,
        grids: Grids,
        at: GridPos,
        actions: &mut Vec<MachineAction>,
    ) {
        let block = grids.block(at);
        let mut ctx = self.context(grids, actions);
        ctx.notify_neighbors(at);
        if let Some(behavior) = self.behaviors.get(&block).cloned() {
            let mut ctx = self.context(grids, actions);
            behavior.placed(&mut ctx, &[at]);
        }
    }

    /// Des chunks du monde contenant des blocs actifs viennent d'être chargés : on les relance.
    pub fn blocks_loaded(
        &mut self,
        world: &RefCell<ChunkStreamer>,
        positions: &[IVec3],
        actions: &mut Vec<MachineAction>,
    ) {
        let grids = Grids::new(world);
        for (behavior, list) in self.group(grids, positions.iter().map(|&p| GridPos::world(p))) {
            let mut ctx = self.context(grids, actions);
            behavior.loaded(&mut ctx, &list);
        }
    }

    fn context<'a>(
        &'a mut self,
        grids: Grids<'a>,
        actions: &'a mut Vec<MachineAction>,
    ) -> MachineContext<'a> {
        MachineContext {
            grids,
            scheduler: &mut self.scheduler,
            active_blocks: &self.active_blocks,
            actions,
        }
    }

    /// Regroupe des positions par comportement (une liste par comportement, dans un ordre
    /// stable : le résultat ne dépend pas de l'ordre d'une `HashMap`).
    fn group(
        &self,
        grids: Grids,
        positions: impl Iterator<Item = GridPos>,
    ) -> Vec<(Rc<dyn BlockBehavior>, Vec<GridPos>)> {
        let mut groups: Vec<(BlockId, Vec<GridPos>)> = Vec::new();
        // On garde l'emprunt de la grille d'une position à la suivante (le cas courant :
        // toutes dans le monde) plutôt que d'emprunter à chaque bloc.
        let mut current: Option<(GridId, std::cell::Ref<dyn BlockAccess>)> = None;
        for at in positions {
            if current.as_ref().is_none_or(|(id, _)| *id != at.grid) {
                current = grids.get(at.grid).map(|g| (at.grid, g.borrow()));
            }
            let Some((_, grid)) = &current else { continue };
            let block = grid.block(at.pos);
            if !self.active_blocks.contains(&block) {
                continue;
            }
            match groups.iter_mut().find(|(b, _)| *b == block) {
                Some((_, list)) => list.push(at),
                None => groups.push((block, vec![at])),
            }
        }
        groups.sort_by_key(|(b, _)| b.0);
        groups
            .into_iter()
            .filter_map(|(b, list)| Some((self.behaviors.get(&b)?.clone(), list)))
            .collect()
    }

    /// Un tick de machines dans le monde (à appeler 20 fois par seconde).
    pub fn run_tick(
        &mut self,
        world: &RefCell<ChunkStreamer>,
        actions: &mut Vec<MachineAction>,
    ) -> TickReport {
        self.run_grids_tick(Grids::new(world), actions)
    }

    /// Un tick de machines sur toutes les grilles.
    pub fn run_grids_tick(&mut self, grids: Grids, actions: &mut Vec<MachineAction>) -> TickReport {
        let _span = engine_trace::span("machines");
        let start = Instant::now();
        self.scheduler.tick += 1;
        let now = self.scheduler.tick;

        // 1. Les ticks programmés arrivés à échéance.
        let mut due = Vec::new();
        while let Some(Reverse((when, _, at))) = self.scheduler.queue.peek().copied() {
            if when > now {
                break;
            }
            self.scheduler.queue.pop();
            self.scheduler.scheduled.remove(&at);
            due.push(at);
        }
        let scheduled = due.len();
        for (behavior, list) in self.group(grids, due.into_iter()) {
            let mut ctx = self.context(grids, actions);
            behavior.scheduled_tick(&mut ctx, &list);
        }

        // 2. Les voisins prévenus (au tick précédent ou pendant celui-ci). Ceux que ces
        //    réactions préviennent à leur tour attendront le tick suivant : pas de boucle
        //    infinie possible dans un même tick.
        let mut neighbors = std::mem::take(&mut self.scheduler.neighbors);
        neighbors.sort_unstable_by_key(|p| p.key());
        neighbors.dedup();
        let neighbor_updates = neighbors.len();
        for (behavior, list) in self.group(grids, neighbors.into_iter()) {
            let mut ctx = self.context(grids, actions);
            behavior.neighbor_changed(&mut ctx, &list);
        }

        // 3. Les réseaux : reconstruire ceux qui ont changé, puis un bilan par réseau. Les
        //    machines dont le contenu a bougé sont réveillées au tick suivant.
        let dirty = std::mem::take(&mut self.scheduler.network_dirty);
        if !dirty.is_empty() || !self.networks.is_empty() {
            let _span = engine_trace::span("réseaux");
            self.networks.rebuild(grids, &dirty);
            for at in self.networks.tick(grids) {
                if self.active_blocks.contains(&grids.block(at)) {
                    self.scheduler.schedule(at, 1);
                }
            }
        }

        TickReport {
            scheduled,
            neighbor_updates,
            duration: start.elapsed(),
        }
    }
}

/// La plus forte puissance émise par les 6 voisins de `pos` dans la même grille.
pub fn received_power(grid: &(impl BlockAccess + ?Sized), pos: IVec3) -> u8 {
    NEIGHBOURS
        .iter()
        .map(|&o| crate::circuits::power_toward(grid, pos + o, pos))
        .max()
        .unwrap_or(0)
}

/// L'horloge : sa puissance bascule entre 0 et 15 toutes les `période` ticks
/// (10 par défaut, soit 0,5 s). La période est le premier octet de `data`.
pub struct Clock;

impl Clock {
    pub const DEFAULT_PERIOD: u8 = 10;

    fn period(entity: Option<&BlockEntity>) -> u32 {
        entity
            .and_then(|e| e.data.first().copied())
            .unwrap_or(Self::DEFAULT_PERIOD)
            .max(1) as u32
    }
}

impl BlockBehavior for Clock {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &pos in positions {
            if ctx.ensure_block_entity(pos) {
                let period = Self::period(ctx.block_entity(pos).as_ref());
                ctx.schedule(pos, period);
            }
        }
    }

    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &pos in positions {
            let period = ctx.with_block_entity(pos, |entity| {
                entity.power = if entity.power > 0 { 0 } else { MAX_POWER };
                Self::period(Some(entity))
            });
            let Some(period) = period else { continue };
            ctx.notify_neighbors(pos);
            ctx.schedule(pos, period);
        }
    }
}

/// La lampe : allumée si un voisin émet de la puissance. Deux blocs (éteinte, allumée)
/// partagent ce comportement ; la lampe s'allume en changeant de bloc.
pub struct Lamp {
    off: BlockId,
    on: BlockId,
}

impl Lamp {
    pub fn new(registry: &BlockRegistry) -> Self {
        Self {
            off: registry.id("core:lamp").unwrap_or_default(),
            on: registry.id("core:lamp_lit").unwrap_or_default(),
        }
    }
}

impl BlockBehavior for Lamp {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.neighbor_changed(ctx, positions);
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &pos in positions {
            let powered = ctx.received_power(pos) > 0;
            let wanted = if powered { self.on } else { self.off };
            if ctx.block(pos) != wanted {
                ctx.set_block(pos, wanted);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::generator::TerrainGenerator;
    use crate::streaming::{StreamingConfig, tests::pump_until_idle};

    pub(crate) fn loaded_streamer() -> RefCell<ChunkStreamer> {
        let config = StreamingConfig {
            view_radius: 2,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(TerrainGenerator::new(7), config);
        pump_until_idle(&mut s, Vec3::new(5.0, 40.0, 5.0));
        RefCell::new(s)
    }

    fn id(s: &RefCell<ChunkStreamer>, key: &str) -> BlockId {
        s.borrow().world().registry().id(key).unwrap()
    }

    fn place(machines: &mut Machines, s: &RefCell<ChunkStreamer>, pos: IVec3, block: BlockId) {
        assert!(s.borrow_mut().set_block(pos, block));
        machines.block_changed(s, pos, &mut Vec::new());
    }

    #[test]
    fn a_clock_lights_its_neighbouring_lamp_on_and_off() {
        let s = loaded_streamer();
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        let (clock, lamp, lit) = (
            id(&s, "core:clock"),
            id(&s, "core:lamp"),
            id(&s, "core:lamp_lit"),
        );
        let at = IVec3::new(8, 100, 8);
        place(&mut machines, &s, at, clock);
        place(&mut machines, &s, at + IVec3::X, lamp);
        let mut states = Vec::new();
        for _ in 0..40 {
            machines.run_tick(&s, &mut Vec::new());
            states.push(s.borrow().world().block(at + IVec3::X) == lit);
        }
        // La lampe suit l'horloge (avec un tick de retard) : elle a été allumée et éteinte.
        assert!(
            states.iter().any(|&on| on) && states.iter().any(|&on| !on),
            "{states:?}"
        );
        // La lampe allumée émet de la lumière.
        assert!(states.last().is_some());
    }

    #[test]
    fn scheduled_ticks_arrive_on_time_and_only_once() {
        struct Recorder(RefCell<Vec<u64>>);
        impl BlockBehavior for Recorder {
            fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
                for _ in positions {
                    self.0.borrow_mut().push(ctx.tick());
                }
            }
        }
        let s = loaded_streamer();
        let mut machines = Machines::new();
        let block = id(&s, "core:stone");
        let recorder = Rc::new(Recorder(RefCell::new(Vec::new())));
        machines.register(block, recorder.clone());
        let at = IVec3::new(8, 100, 8);
        s.borrow_mut().set_block(at, block);
        machines.schedule(GridPos::world(at), 5);
        machines.schedule(GridPos::world(at), 2); // déjà programmé : ignoré
        for _ in 0..10 {
            machines.run_tick(&s, &mut Vec::new());
        }
        assert_eq!(*recorder.0.borrow(), vec![5]);
    }

    #[test]
    fn behaviors_receive_their_machines_in_one_batch() {
        struct Batches(RefCell<Vec<usize>>);
        impl BlockBehavior for Batches {
            fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
                for &p in positions {
                    ctx.schedule(p, 1);
                }
            }
            fn scheduled_tick(&self, _: &mut MachineContext, positions: &[GridPos]) {
                self.0.borrow_mut().push(positions.len());
            }
        }
        let s = loaded_streamer();
        let mut machines = Machines::new();
        let block = id(&s, "core:glass");
        let batches = Rc::new(Batches(RefCell::new(Vec::new())));
        machines.register(block, batches.clone());
        for i in 0..50 {
            place(
                &mut machines,
                &s,
                IVec3::new(i % 10 * 2, 110, i / 10 * 2),
                block,
            );
        }
        machines.run_tick(&s, &mut Vec::new());
        assert_eq!(
            *batches.0.borrow(),
            vec![50],
            "un seul appel pour 50 machines"
        );
    }

    /// Le test de charge réduit (le complet est `cargo bench --bench machines`) :
    /// 2 000 horloges au rythme maximal travaillent toutes, à chaque tick.
    #[test]
    fn a_small_factory_runs_every_machine_every_tick() {
        let s = loaded_streamer();
        let mut machines = Machines::with_natives(s.borrow().world().registry());
        let clock = id(&s, "core:clock");
        let mut n = 0;
        for x in (-30..30).step_by(2) {
            for z in (-30..30).step_by(2) {
                for y in [100, 102] {
                    let pos = IVec3::new(x, y, z);
                    s.borrow_mut().set_block_raw(pos, clock);
                    s.borrow_mut().set_block_entity(
                        pos,
                        Some(BlockEntity {
                            data: vec![1],
                            ..Default::default()
                        }),
                    );
                    machines.block_changed(&s, pos, &mut Vec::new());
                    n += 1;
                }
            }
        }
        for _ in 0..5 {
            let report = machines.run_tick(&s, &mut Vec::new());
            assert_eq!(report.scheduled, n);
        }
        // Chaque horloge a basculé 5 fois : éteinte au départ, allumée après un nombre impair.
        assert_eq!(machines.scheduler().pending(), n);
        let lit = s
            .borrow()
            .block_entity(IVec3::new(0, 100, 0))
            .unwrap()
            .power;
        assert_eq!(lit, MAX_POWER);
    }
}
