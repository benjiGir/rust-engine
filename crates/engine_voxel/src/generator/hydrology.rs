//! L'hydrologie macro du générateur v2 (prototype, lot 3) : un graphe de drainage hiérarchique,
//! défini en coordonnées monde, acyclique et de profondeur bornée **par construction**.
//!
//! # Le modèle à deux échelles
//!
//! - **Cellules fines** de [`FINE_SIZE`] = 64 blocs. Chacune a un nœud (position jitterée) et une
//!   altitude macro (celle de la géographie). Les cours d'eau sont les arêtes fine → fine.
//! - **Bassins** de [`BASIN_FINE`] × [`BASIN_FINE`] cellules fines, soit 512 blocs. Le graphe entre
//!   bassins est le niveau supérieur : c'est lui qui fixe où l'eau passe d'un bassin à l'autre.
//!
//! ## Niveau supérieur (bassins)
//!
//! Le **nœud** d'un bassin est sa cellule fine la plus basse parmi les 6 × 6 cellules intérieures.
//! Un bassin s'écoule vers celui de ses quatre voisins dont l'ordre `(altitude du nœud, hachage)`
//! est strictement plus bas, par la **passe la plus basse** de leur frontière. Cet ordre total
//! strict rend le graphe **acyclique** : l'altitude du nœud (avec départage par hachage) décroît
//! à chaque pas. Un bassin sans voisin plus bas est un **puits déclaré** : océan (son nœud est
//! sous la mer) ou **lac terminal** (bassin fermé intentionnel).
//!
//! **Profondeur bornée par définition.** Le nombre de pas jusqu'au puits naturel est mesuré par une
//! marche d'au plus [`MAX_BASIN_HOPS`] + 1 bassins. Un bassin dont la marche dépasse cette borne
//! est lui-même déclaré puits (« lac plafonné », [`SinkKind::Capped`]). Ses voisins amont, plus
//! éloignés encore, le sont aussi ; ses voisins aval gardent leur arête. Aucun chemin n'a donc plus
//! de [`MAX_BASIN_HOPS`] arêtes inter-bassins, quelle que soit la graine, et la résolution d'un
//! bassin ne regarde jamais plus de [`MAX_BASIN_HOPS`] + 1 bassins. (La borne n'est pas atteinte en
//! pratique : voir le test de terminaison.)
//!
//! ## Niveau fin (cellules)
//!
//! Dans un bassin, chaque cellule fine s'écoule vers sa **cible** : la cellule de sortie (la
//! passe la plus basse vers le bassin aval, le **contrat de bord**) ou, pour un puits, le nœud du
//! bassin. Parmi ses voisins du même bassin **strictement plus proches de la cible** (distance
//! euclidienne en cellules), elle prend le plus bas. La distance entière décroît à chaque pas :
//! pas de cycle, et au plus 98 pas (en pratique moins de 15) pour atteindre la cible. La cellule de
//! sortie passe à la cellule d'entrée du bassin aval, juste de l'autre côté de la frontière. Les
//! rivières suivent donc les creux quand la géométrie le permet, mais **ne peuvent jamais**
//! s'enfermer dans un minimum local : l'ordre sur les bassins puis sur les distances les mène au
//! puits. Aucun remplissage de dépressions, aucune couture : la direction d'une cellule est une
//! fonction pure de `(graine, cellule)` qui lit au plus son bassin, ses quatre bassins voisins et
//! leurs cellules de bord.
//!
//! ## Contrat de bord
//!
//! La passe entre deux bassins A → B ([`Crossing`]) et ses attributs ([`EdgeContract`] : identité du
//! bassin versant, accumulation, largeur, niveaux d'eau de part et d'autre) sont des fonctions pures de
//! l'arête ordonnée (A, B). Les deux voisins appellent la même fonction : ils lisent la même valeur,
//! quel que soit l'ordre ou la région dans laquelle on calcule.
//!
//! ## Profil d'altitude et accumulation
//!
//! Le **niveau d'eau** d'une cellule est le maximum, le long de son chemin vers l'exutoire, de
//! `altitude - RIVER_DEPTH`, plancher à la mer ou au niveau du lac terminal : il **ne remonte jamais
//! vers l'aval** (le maximum d'un chemin qui s'allonge ne peut que croître vers l'amont). Si le
//! terrain monte vers l'aval, c'est le terrain que le lot 4 rehausse (remblai), pas l'eau qui remonte.
//! Arrondir par `floor` conserve l'ordre. L'**accumulation** d'une cellule est le nombre de cellules
//! en amont (elle comprise), saturé à [`ACCUMULATION_CAP`] : exact tant qu'il est inférieur, donc
//! croissante vers l'aval (confluences cohérentes), et le parcours est borné. Largeur et profondeur
//! s'en déduisent ([`river_width`]).
//!
//! # Lacs
//!
//! - **Lacs terminaux** : un puits qui n'est pas l'océan est un lac au niveau de son nœud.
//! - **Lacs de passage** : certaines cellules de rivière (tirage, isolées dans un voisinage de 7 × 7)
//!   portent un lac au niveau de la rivière en ce point ; le déversoir est l'arête aval (dont le niveau
//!   est inférieur ou égal : palier ou cascade).
//!
//! # Coût et caches
//!
//! Tout est mémoïsé dans des tables bornées (vidées quand elles débordent) ; un cache accélère une
//! valeur sans jamais la décider. [`Hydrology::stats`] compte les évaluations qui ratent le cache :
//! sur un contexte neuf, c'est le coût à froid d'une requête.

use std::collections::VecDeque;
use std::hash::Hash;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_hash::{FxHashMap, FxHashSet};

use super::SEA_LEVEL;
use super::geography::Geography;
use super::macro_noise::{gradient_noise, lattice, unit};

/// Côté d'une cellule fine, en blocs.
pub const FINE_SIZE: i32 = 64;
/// Cellules fines par côté de bassin.
pub const BASIN_FINE: i32 = 8;
/// Côté d'un bassin, en blocs.
pub const BASIN_SIZE: i32 = FINE_SIZE * BASIN_FINE;
/// Nombre maximal d'arêtes inter-bassins d'un chemin (voir la documentation du module).
pub const MAX_BASIN_HOPS: u32 = 48;
/// Nombre maximal de pas fins dans un bassin : la distance entière à la cible décroît strictement.
pub const MAX_STEPS_IN_BASIN: usize = 98;
/// Nombre maximal de cellules fines d'un chemin complet.
pub const MAX_FINE_PATH: usize = (MAX_BASIN_HOPS as usize + 1) * (MAX_STEPS_IN_BASIN + 1);
/// Saturation de l'accumulation (nombre de cellules amont).
pub const ACCUMULATION_CAP: u32 = 384;
/// Accumulation à partir de laquelle une arête est dessinée comme un cours d'eau (sinon : ruissellement).
pub const STREAM_MIN_ACCUMULATION: u32 = 10;
/// Le lit est creusé de cette profondeur sous l'altitude macro du nœud, pour le profil d'eau.
pub const RIVER_DEPTH: f32 = 1.5;
/// Les tables de cache sont vidées au-delà de ce nombre d'entrées.
pub const DEFAULT_CACHE_LIMIT: usize = 1 << 18;
/// Amplitude (blocs) du tirage qui fait serpenter les cours d'eau.
const MEANDER: f32 = 3.0;
/// Probabilité qu'une cellule de rivière porte un lac de passage (avant isolement).
const FLOW_LAKE_PROBABILITY: f64 = 0.05;

const SALT_NODE_X: u64 = 0x41;
const SALT_NODE_Z: u64 = 0x42;
const SALT_TIE: u64 = 0x43;
const SALT_CROSSING: u64 = 0x44;
const SALT_STEP: u64 = 0x45;
const SALT_LAKE: u64 = 0x46;
const SALT_LAKE_RADIUS: u64 = 0x47;
const SALT_LAKE_SHAPE: u64 = 0x48;
const SALT_BASIN_ID: u64 = 0x49;

/// Une cellule fine, par ses indices (x et z divisés par [`FINE_SIZE`], arrondis vers le bas).
pub type FineCell = (i32, i32);
/// Un bassin, par ses indices (x et z divisés par [`BASIN_SIZE`], arrondis vers le bas).
pub type BasinCell = (i32, i32);

/// Pourquoi un bassin est un puits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkKind {
    /// L'océan : le nœud du bassin est sous le niveau de la mer.
    Ocean,
    /// Un lac terminal : bassin fermé, sans exutoire.
    Lake,
    /// Un lac terminal imposé par la borne de profondeur ([`MAX_BASIN_HOPS`]).
    Capped,
}

/// La passe entre deux bassins : où l'eau change de bassin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossing {
    /// La cellule de sortie, dans le bassin amont, au bord.
    pub exit: FineCell,
    /// La cellule d'entrée, dans le bassin aval, juste de l'autre côté du bord.
    pub entry: FineCell,
    /// L'altitude de la passe : le plus haut des deux côtés.
    pub pass: f32,
}

/// Ce que fait un bassin de son eau.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasinFlow {
    Sink(SinkKind),
    Flow { to: BasinCell, crossing: Crossing },
}

/// La profondeur d'un bassin dans le graphe : arêtes jusqu'au puits et puits atteint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasinChain {
    pub hops: u32,
    pub terminal: BasinCell,
    pub capped: bool,
}

/// Le contrat de bord d'une arête A → B : lu à l'identique par les deux voisins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeContract {
    pub from: BasinCell,
    pub to: BasinCell,
    pub exit: FineCell,
    pub entry: FineCell,
    /// L'identité du bassin versant (le puits où l'eau finit).
    pub basin: u64,
    pub accumulation: u32,
    pub width: f32,
    /// Niveau d'eau en blocs côté amont (cellule de sortie) et côté aval (cellule d'entrée) :
    /// `level_out >= level_in`.
    pub level_out: i32,
    pub level_in: i32,
    /// Faux si la cellule de sortie est déjà en mer (une baie au bord) : l'eau s'arrête à la
    /// passe, `level_in` vaut alors `level_out`.
    pub continues: bool,
}

/// Un lac : disque (déformé) de centre, rayon et niveau d'eau donnés.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lake {
    pub center: [f64; 2],
    pub radius: f64,
    pub level: f32,
    /// Vrai pour un bassin fermé (sans déversoir) ; faux pour un lac de passage.
    pub terminal: bool,
}

/// Un point dans un lac.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakePoint {
    pub level: f32,
    pub terminal: bool,
    /// 0 au rivage, 1 au centre.
    pub depth_fraction: f32,
}

/// La rivière la plus proche d'un point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiverPoint {
    /// Distance à l'axe, en blocs.
    pub distance: f32,
    pub width: f32,
    /// Niveau de l'eau en ce point (interpolé le long de l'arête, jamais croissant vers l'aval).
    pub level: f32,
    pub accumulation: u32,
    pub basin: u64,
}

/// Compteurs de travail : un échec de cache = un calcul.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HydroStats {
    /// Altitudes macro évaluées (le poste dominant).
    pub elevations: u64,
    /// Tous les calculs mémoïsés (altitudes, bassins, passes, directions, niveaux, accumulations).
    pub work: u64,
}

/// La largeur d'une rivière (blocs) selon son accumulation.
pub fn river_width(accumulation: u32) -> f32 {
    (1.4 + 0.9 * (accumulation as f32).powf(0.62)).min(40.0)
}

/// Les cellules fine et bassin contenant le point (x, z).
pub fn fine_cell(x: f64, z: f64) -> FineCell {
    (
        (x / FINE_SIZE as f64).floor() as i32,
        (z / FINE_SIZE as f64).floor() as i32,
    )
}

/// Le bassin d'une cellule fine.
pub fn basin_of(f: FineCell) -> BasinCell {
    (f.0.div_euclid(BASIN_FINE), f.1.div_euclid(BASIN_FINE))
}

/// Une table mémoïque bornée : vidée au débordement, jamais décisive.
struct Memo<K, V> {
    map: Mutex<FxHashMap<K, V>>,
    limit: usize,
}

impl<K: Eq + Hash + Copy, V: Copy> Memo<K, V> {
    fn new(limit: usize) -> Self {
        Self {
            map: Mutex::new(FxHashMap::default()),
            limit,
        }
    }

    fn get(&self, key: &K) -> Option<V> {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .copied()
    }

    fn put(&self, key: K, value: V) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.limit {
            map.clear();
        }
        map.insert(key, value);
    }

    fn clear(&self) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn len(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Le nœud d'un bassin : sa cellule la plus basse et cette altitude.
#[derive(Debug, Clone, Copy)]
struct BasinInfo {
    target: FineCell,
    z: f32,
    tie: u64,
}

/// L'hydrologie d'une graine.
pub struct Hydrology {
    seed: u32,
    geography: Geography,
    elevations: Memo<FineCell, f32>,
    infos: Memo<BasinCell, BasinInfo>,
    crossings: Memo<(BasinCell, BasinCell), Crossing>,
    natural: Memo<BasinCell, BasinFlow>,
    chains: Memo<BasinCell, BasinChain>,
    nexts: Memo<FineCell, Option<FineCell>>,
    levels: Memo<FineCell, f32>,
    accumulations: Memo<FineCell, u32>,
    flow_lakes: Memo<FineCell, Option<Lake>>,
    elevation_count: AtomicU64,
    work_count: AtomicU64,
}

impl Hydrology {
    pub fn new(seed: u32) -> Self {
        Self::with_cache_limit(seed, DEFAULT_CACHE_LIMIT)
    }

    /// Avec une borne de cache donnée (les tests de cache évincé la réduisent beaucoup).
    pub fn with_cache_limit(seed: u32, limit: usize) -> Self {
        Self {
            seed,
            geography: Geography::new(seed),
            elevations: Memo::new(limit),
            infos: Memo::new(limit),
            crossings: Memo::new(limit),
            natural: Memo::new(limit),
            chains: Memo::new(limit),
            nexts: Memo::new(limit),
            levels: Memo::new(limit),
            accumulations: Memo::new(limit),
            flow_lakes: Memo::new(limit),
            elevation_count: AtomicU64::new(0),
            work_count: AtomicU64::new(0),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    pub fn geography(&self) -> &Geography {
        &self.geography
    }

    /// Vide tous les caches (les compteurs de travail continuent).
    pub fn clear_caches(&self) {
        self.elevations.clear();
        self.infos.clear();
        self.crossings.clear();
        self.natural.clear();
        self.chains.clear();
        self.nexts.clear();
        self.levels.clear();
        self.accumulations.clear();
        self.flow_lakes.clear();
    }

    /// Le nombre d'entrées en cache, toutes tables confondues (pour vérifier la borne mémoire).
    pub fn cached_entries(&self) -> usize {
        self.elevations.len()
            + self.infos.len()
            + self.crossings.len()
            + self.natural.len()
            + self.chains.len()
            + self.nexts.len()
            + self.levels.len()
            + self.accumulations.len()
            + self.flow_lakes.len()
    }

    pub fn stats(&self) -> HydroStats {
        HydroStats {
            elevations: self.elevation_count.load(Ordering::Relaxed),
            work: self.work_count.load(Ordering::Relaxed),
        }
    }

    fn count(&self) {
        self.work_count.fetch_add(1, Ordering::Relaxed);
    }

    // ---- cellules fines ----

    /// Le nœud (position monde) d'une cellule fine : un point jitteré dans sa partie centrale.
    pub fn node(&self, f: FineCell) -> [f64; 2] {
        let jx = 0.30 + 0.40 * unit(self.seed, SALT_NODE_X, f.0, f.1);
        let jz = 0.30 + 0.40 * unit(self.seed, SALT_NODE_Z, f.0, f.1);
        [
            (f.0 as f64 + jx) * FINE_SIZE as f64,
            (f.1 as f64 + jz) * FINE_SIZE as f64,
        ]
    }

    /// L'altitude macro au nœud d'une cellule fine.
    pub fn elevation(&self, f: FineCell) -> f32 {
        if let Some(e) = self.elevations.get(&f) {
            return e;
        }
        let [x, z] = self.node(f);
        let e = self.geography.macro_elevation(x, z);
        self.elevation_count.fetch_add(1, Ordering::Relaxed);
        self.count();
        self.elevations.put(f, e);
        e
    }

    /// Vrai si la cellule est en mer (son altitude macro est sous le niveau de la mer).
    pub fn is_sea(&self, f: FineCell) -> bool {
        self.elevation(f) < SEA_LEVEL as f32
    }

    fn tie(&self, b: BasinCell) -> u64 {
        lattice(self.seed, SALT_TIE, b.0, b.1)
    }

    // ---- niveau supérieur : les bassins ----

    fn basin_info(&self, b: BasinCell) -> BasinInfo {
        if let Some(i) = self.infos.get(&b) {
            return i;
        }
        let mut best: Option<(f32, u64, FineCell)> = None;
        for dz in 1..BASIN_FINE - 1 {
            for dx in 1..BASIN_FINE - 1 {
                let f = (b.0 * BASIN_FINE + dx, b.1 * BASIN_FINE + dz);
                let e = self.elevation(f);
                let tie = lattice(self.seed, SALT_TIE, f.0, f.1);
                if best.is_none_or(|(be, bt, _)| (e, tie) < (be, bt)) {
                    best = Some((e, tie, f));
                }
            }
        }
        let (z, _, target) = best.expect("un bassin a des cellules intérieures");
        let info = BasinInfo {
            target,
            z,
            tie: self.tie(b),
        };
        self.count();
        self.infos.put(b, info);
        info
    }

    /// Vrai si `a` est strictement plus bas que `b` dans l'ordre total du graphe.
    fn lower(&self, a: BasinCell, b: BasinCell) -> bool {
        let (ia, ib) = (self.basin_info(a), self.basin_info(b));
        (ia.z, ia.tie, a) < (ib.z, ib.tie, b)
    }

    /// La passe de `from` vers son voisin direct `to` : la paire de cellules de bord la plus basse.
    pub fn crossing(&self, from: BasinCell, to: BasinCell) -> Crossing {
        if let Some(c) = self.crossings.get(&(from, to)) {
            return c;
        }
        let (dx, dz) = (to.0 - from.0, to.1 - from.1);
        debug_assert_eq!(dx.abs() + dz.abs(), 1, "bassins voisins par un côté");
        let mut best: Option<(f32, u64, Crossing)> = None;
        for u in 0..BASIN_FINE {
            let exit = if dx == 1 {
                (
                    from.0 * BASIN_FINE + BASIN_FINE - 1,
                    from.1 * BASIN_FINE + u,
                )
            } else if dx == -1 {
                (from.0 * BASIN_FINE, from.1 * BASIN_FINE + u)
            } else if dz == 1 {
                (
                    from.0 * BASIN_FINE + u,
                    from.1 * BASIN_FINE + BASIN_FINE - 1,
                )
            } else {
                (from.0 * BASIN_FINE + u, from.1 * BASIN_FINE)
            };
            let entry = (exit.0 + dx, exit.1 + dz);
            let pass = self.elevation(exit).max(self.elevation(entry));
            let tie = lattice(self.seed, SALT_CROSSING, exit.0, exit.1);
            if best.is_none_or(|(bp, bt, _)| (pass, tie) < (bp, bt)) {
                best = Some((pass, tie, Crossing { exit, entry, pass }));
            }
        }
        let crossing = best.expect("un bord a des cellules").2;
        self.count();
        self.crossings.put((from, to), crossing);
        crossing
    }

    /// L'arête « naturelle » d'un bassin, avant la borne de profondeur.
    fn natural_flow(&self, b: BasinCell) -> BasinFlow {
        if let Some(f) = self.natural.get(&b) {
            return f;
        }
        let mut best: Option<(f32, BasinCell, Crossing)> = None;
        for n in [
            (b.0 + 1, b.1),
            (b.0 - 1, b.1),
            (b.0, b.1 + 1),
            (b.0, b.1 - 1),
        ] {
            if !self.lower(n, b) {
                continue;
            }
            let crossing = self.crossing(b, n);
            let better = match best {
                None => true,
                Some((pass, other, _)) => {
                    crossing.pass < pass || (crossing.pass == pass && self.lower(n, other))
                }
            };
            if better {
                best = Some((crossing.pass, n, crossing));
            }
        }
        let flow = match best {
            Some((_, to, crossing)) => BasinFlow::Flow { to, crossing },
            None => {
                let target = self.basin_info(b).target;
                BasinFlow::Sink(if self.is_sea(target) {
                    SinkKind::Ocean
                } else {
                    SinkKind::Lake
                })
            }
        };
        self.count();
        self.natural.put(b, flow);
        flow
    }

    /// La profondeur d'un bassin : au plus [`MAX_BASIN_HOPS`] arêtes, sinon c'est un puits plafonné.
    pub fn chain(&self, b: BasinCell) -> BasinChain {
        if let Some(c) = self.chains.get(&b) {
            return c;
        }
        let capped = BasinChain {
            hops: MAX_BASIN_HOPS + 1,
            terminal: b,
            capped: true,
        };
        let mut path = vec![b];
        let mut cur = b;
        // `tail` : (arêtes restantes après le dernier bassin du chemin, puits atteint).
        let tail = loop {
            match self.natural_flow(cur) {
                BasinFlow::Sink(_) => break Some((0, cur)),
                BasinFlow::Flow { to, .. } => {
                    if let Some(known) = self.chains.get(&to) {
                        break if known.capped {
                            None
                        } else {
                            Some((known.hops + 1, known.terminal))
                        };
                    }
                    path.push(to);
                    cur = to;
                    if path.len() > MAX_BASIN_HOPS as usize + 1 {
                        break None;
                    }
                }
            }
        };
        let Some((after_last, terminal)) = tail else {
            self.count();
            self.chains.put(b, capped);
            return capped;
        };
        let len = path.len() as u32;
        if (len - 1) + after_last > MAX_BASIN_HOPS {
            self.count();
            self.chains.put(b, capped);
            return capped;
        }
        for (i, &cell) in path.iter().enumerate() {
            let hops = (len - 1 - i as u32) + after_last;
            self.chains.put(
                cell,
                BasinChain {
                    hops,
                    terminal,
                    capped: false,
                },
            );
        }
        self.count();
        BasinChain {
            hops: len - 1 + after_last,
            terminal,
            capped: false,
        }
    }

    /// Ce que fait un bassin de son eau (borne de profondeur comprise).
    pub fn basin_flow(&self, b: BasinCell) -> BasinFlow {
        if self.chain(b).capped {
            BasinFlow::Sink(SinkKind::Capped)
        } else {
            self.natural_flow(b)
        }
    }

    /// L'identité du bassin versant d'un bassin : un hachage du puits où l'eau finit.
    pub fn basin_id(&self, b: BasinCell) -> u64 {
        let t = self.chain(b).terminal;
        lattice(self.seed, SALT_BASIN_ID, t.0, t.1) | 1
    }

    /// Le nœud (cellule la plus basse) d'un bassin.
    pub fn basin_target(&self, b: BasinCell) -> FineCell {
        self.basin_info(b).target
    }

    // ---- niveau fin : les cellules ----

    /// La cellule suivante vers l'aval, ou `None` pour une cellule terminale : mer, ou nœud d'un
    /// puits (lac terminal).
    pub fn next(&self, f: FineCell) -> Option<FineCell> {
        if let Some(n) = self.nexts.get(&f) {
            return n;
        }
        let next = if self.is_sea(f) {
            None
        } else {
            let b = basin_of(f);
            match self.basin_flow(b) {
                BasinFlow::Sink(_) => {
                    let target = self.basin_info(b).target;
                    (f != target).then(|| self.step_toward(f, target, b))
                }
                BasinFlow::Flow { crossing, .. } => Some(if f == crossing.exit {
                    crossing.entry
                } else {
                    self.step_toward(f, crossing.exit, b)
                }),
            }
        };
        self.count();
        self.nexts.put(f, next);
        next
    }

    /// Le voisin de `f` dans le bassin `b`, strictement plus proche de `target`, le plus bas.
    fn step_toward(&self, f: FineCell, target: FineCell, b: BasinCell) -> FineCell {
        let d2 = |c: FineCell| {
            let (dx, dz) = ((c.0 - target.0) as i64, (c.1 - target.1) as i64);
            dx * dx + dz * dz
        };
        let here = d2(f);
        let mut best: Option<(f32, u64, FineCell)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let n = (f.0 + dx, f.1 + dz);
                if (dx, dz) == (0, 0) || basin_of(n) != b || d2(n) >= here {
                    continue;
                }
                // Le coût : l'altitude, plus un petit tirage par cellule qui fait serpenter les
                // cours d'eau des plaines (sans lui, ils longeraient les axes de la grille).
                let tie = lattice(self.seed, SALT_STEP, n.0, n.1);
                let cost = self.elevation(n) + MEANDER * (tie >> 40) as f32 / (1u64 << 24) as f32;
                if best.is_none_or(|(c, t, _)| (cost, tie) < (c, t)) {
                    best = Some((cost, tie, n));
                }
            }
        }
        // Il existe toujours un voisin plus proche dans le bassin (un pas vers la cible) ;
        // le repli par le signe est exactement ce pas.
        best.map(|(_, _, n)| n).unwrap_or((
            f.0 + (target.0 - f.0).signum(),
            f.1 + (target.1 - f.1).signum(),
        ))
    }

    /// Le niveau d'eau de la cellule, en blocs (flottant) : jamais croissant vers l'aval.
    pub fn level(&self, f: FineCell) -> f32 {
        if let Some(v) = self.levels.get(&f) {
            return v;
        }
        let mut path: Vec<FineCell> = Vec::new();
        let mut cur = f;
        let base = loop {
            if let Some(v) = self.levels.get(&cur) {
                break v;
            }
            match self.next(cur) {
                None => {
                    // Une cellule terminale : la mer, ou le niveau du lac terminal.
                    let v = if self.is_sea(cur) {
                        SEA_LEVEL as f32
                    } else {
                        self.elevation(cur)
                    };
                    self.count();
                    self.levels.put(cur, v);
                    break v;
                }
                Some(n) => {
                    path.push(cur);
                    cur = n;
                    assert!(path.len() <= MAX_FINE_PATH, "chemin plus long que sa borne");
                }
            }
        };
        let mut level = base;
        for &c in path.iter().rev() {
            level = level.max(self.elevation(c) - RIVER_DEPTH);
            self.count();
            self.levels.put(c, level);
        }
        level
    }

    /// Le niveau d'eau arrondi en blocs : l'arrondi (`floor`) conserve la décroissance.
    pub fn level_block(&self, f: FineCell) -> i32 {
        self.level(f).floor() as i32
    }

    /// Le nombre de cellules en amont de `f`, elle comprise, saturé à [`ACCUMULATION_CAP`].
    pub fn accumulation(&self, f: FineCell) -> u32 {
        if let Some(a) = self.accumulations.get(&f) {
            return a;
        }
        let mut seen: FxHashSet<FineCell> = FxHashSet::default();
        let mut queue = VecDeque::new();
        seen.insert(f);
        queue.push_back(f);
        let mut count = 0u32;
        while let Some(cur) = queue.pop_front() {
            count += 1;
            if count >= ACCUMULATION_CAP {
                break;
            }
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let n = (cur.0 + dx, cur.1 + dz);
                    if (dx, dz) != (0, 0) && !seen.contains(&n) && self.next(n) == Some(cur) {
                        seen.insert(n);
                        queue.push_back(n);
                    }
                }
            }
        }
        self.count();
        self.accumulations.put(f, count);
        count
    }

    /// Le contrat de bord de l'arête `from → to`, s'il y a bien une arête (sinon `None`).
    pub fn contract(&self, from: BasinCell, to: BasinCell) -> Option<EdgeContract> {
        let BasinFlow::Flow {
            to: actual,
            crossing,
        } = self.basin_flow(from)
        else {
            return None;
        };
        if actual != to {
            return None;
        }
        let accumulation = self.accumulation(crossing.exit);
        let continues = !self.is_sea(crossing.exit);
        let level_out = self.level_block(crossing.exit);
        Some(EdgeContract {
            from,
            to,
            exit: crossing.exit,
            entry: crossing.entry,
            basin: self.basin_id(from),
            accumulation,
            width: river_width(accumulation),
            level_out,
            level_in: if continues {
                self.level_block(crossing.entry)
            } else {
                level_out
            },
            continues,
        })
    }

    // ---- lacs ----

    /// Le lac de passage d'une cellule, s'il y en a un.
    fn flow_lake(&self, g: FineCell) -> Option<Lake> {
        if let Some(l) = self.flow_lakes.get(&g) {
            return l;
        }
        let roll = unit(self.seed, SALT_LAKE, g.0, g.1);
        let lake = if roll >= FLOW_LAKE_PROBABILITY {
            None
        } else {
            // Isolé : aucune cellule des 7 × 7 voisines n'a un tirage plus bas. Les disques
            // (rayon au plus 0,95 cellule) de deux lacs ne se touchent donc jamais.
            let isolated = (-3..=3).all(|dz| {
                (-3..=3).all(|dx| {
                    (dx, dz) == (0, 0) || unit(self.seed, SALT_LAKE, g.0 + dx, g.1 + dz) >= roll
                })
            });
            if !isolated
                || self.is_sea(g)
                || self.next(g).is_none()
                || self.accumulation(g) < STREAM_MIN_ACCUMULATION
                || self.level(g) <= SEA_LEVEL as f32 + 2.0
            {
                None
            } else {
                Some(Lake {
                    center: self.node(g),
                    radius: FINE_SIZE as f64
                        * (0.40 + 0.55 * unit(self.seed, SALT_LAKE_RADIUS, g.0, g.1)),
                    level: self.level(g),
                    terminal: false,
                })
            }
        };
        self.count();
        self.flow_lakes.put(g, lake);
        lake
    }

    /// Le lac terminal d'un bassin, si c'est un puits qui n'est pas l'océan.
    fn terminal_lake(&self, b: BasinCell) -> Option<Lake> {
        match self.basin_flow(b) {
            BasinFlow::Sink(SinkKind::Lake | SinkKind::Capped) => {
                let target = self.basin_info(b).target;
                Some(Lake {
                    center: self.node(target),
                    radius: 100.0 + 80.0 * unit(self.seed, SALT_LAKE_RADIUS, b.0, b.1),
                    level: self.elevation(target),
                    terminal: true,
                })
            }
            _ => None,
        }
    }

    /// Les lacs proches d'une cellule (ceux dont le disque peut atteindre un point de cette cellule).
    pub fn lakes_near(&self, f: FineCell) -> Vec<Lake> {
        let mut lakes = Vec::new();
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(l) = self.flow_lake((f.0 + dx, f.1 + dz)) {
                    lakes.push(l);
                }
            }
        }
        let b = basin_of(f);
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(l) = self.terminal_lake((b.0 + dx, b.1 + dz)) {
                    lakes.push(l);
                }
            }
        }
        lakes
    }

    /// Le lac contenant le point (x, z), s'il y en a un (rivage déformé par un bruit lent).
    pub fn lake_at(&self, x: f64, z: f64) -> Option<LakePoint> {
        let mut best: Option<LakePoint> = None;
        for lake in self.lakes_near(fine_cell(x, z)) {
            let (dx, dz) = (x - lake.center[0], z - lake.center[1]);
            let shape = 1.0 + 0.22 * gradient_noise(self.seed, SALT_LAKE_SHAPE, x / 55.0, z / 55.0);
            let radius = lake.radius * shape;
            let fraction = 1.0 - dx.hypot(dz) / radius;
            if fraction > 0.0 && best.is_none_or(|b| fraction as f32 > b.depth_fraction) {
                best = Some(LakePoint {
                    level: lake.level,
                    terminal: lake.terminal,
                    depth_fraction: fraction as f32,
                });
            }
        }
        best
    }

    // ---- requêtes de point ----

    /// L'identité du bassin versant du point (x, z).
    pub fn basin_id_at(&self, x: f64, z: f64) -> u64 {
        self.basin_id(basin_of(fine_cell(x, z)))
    }

    /// La rivière la plus proche du point (x, z) parmi les arêtes des 5 × 5 cellules voisines
    /// (distance à l'axe, à comparer à la largeur), pour les arêtes de cours d'eau seulement.
    pub fn river_near(&self, x: f64, z: f64) -> Option<RiverPoint> {
        let c = fine_cell(x, z);
        let mut best: Option<(f64, RiverPoint)> = None;
        for dz in -2..=2 {
            for dx in -2..=2 {
                let g = (c.0 + dx, c.1 + dz);
                if self.is_sea(g) {
                    continue;
                }
                let Some(n) = self.next(g) else { continue };
                let acc = self.accumulation(g);
                if acc < STREAM_MIN_ACCUMULATION {
                    continue;
                }
                let (a, b) = (self.node(g), self.node(n));
                let (sx, sz) = (b[0] - a[0], b[1] - a[1]);
                let len2 = (sx * sx + sz * sz).max(1e-9);
                let t = (((x - a[0]) * sx + (z - a[1]) * sz) / len2).clamp(0.0, 1.0);
                let distance = (x - (a[0] + sx * t)).hypot(z - (a[1] + sz * t));
                let width = river_width(acc) as f64
                    + (river_width(self.accumulation(n)) - river_width(acc)) as f64 * t;
                let score = distance - width * 0.5;
                if best.is_none_or(|(s, _)| score < s) {
                    let level = self.level(g) as f64 + (self.level(n) - self.level(g)) as f64 * t;
                    best = Some((
                        score,
                        RiverPoint {
                            distance: distance as f32,
                            width: width as f32,
                            level: level as f32,
                            accumulation: acc,
                            basin: self.basin_id(basin_of(g)),
                        },
                    ));
                }
            }
        }
        best.map(|(_, r)| r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basin_of_uses_floor_division_for_negative_cells() {
        assert_eq!(basin_of((0, 0)), (0, 0));
        assert_eq!(basin_of((7, 7)), (0, 0));
        assert_eq!(basin_of((8, -1)), (1, -1));
        assert_eq!(basin_of((-1, -8)), (-1, -1));
        assert_eq!(basin_of((-9, -9)), (-2, -2));
        assert_eq!(fine_cell(-0.5, 63.9), (-1, 0));
    }

    #[test]
    fn width_grows_with_accumulation_and_is_bounded() {
        let mut last = 0.0;
        for acc in 1..=ACCUMULATION_CAP {
            let w = river_width(acc);
            assert!(w >= last && w <= 40.0);
            last = w;
        }
    }

    #[test]
    fn crossing_is_read_identically_from_a_fresh_context() {
        let (a, b) = (Hydrology::new(5), Hydrology::new(5));
        for from in [(0, 0), (-1, -1), (3, -4)] {
            for to in [
                (from.0 + 1, from.1),
                (from.0 - 1, from.1),
                (from.0, from.1 + 1),
                (from.0, from.1 - 1),
            ] {
                assert_eq!(a.crossing(from, to), b.crossing(from, to));
            }
        }
    }
}
