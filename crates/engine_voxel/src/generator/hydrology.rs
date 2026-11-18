//! L'hydrologie macro du générateur v2 (prototype, lot 3) : un graphe de drainage hiérarchique,
//! défini en coordonnées monde, acyclique et de profondeur bornée **par construction**.
//!
//! # Le modèle (trois niveaux)
//!
//! 1. **Cellules fines** de [`FINE_SIZE`] = 128 blocs : un nœud jitteré et une altitude macro. Les
//!    cours d'eau sont les arêtes cellule → cellule (8 voisins), lissées à l'échelle des blocs
//!    (voir `rivers`).
//! 2. **Bassins** (« régions ») de forme libre, d'environ [`BASIN_FINE`] × [`BASIN_FINE`] cellules :
//!    une graine par super-cellule (un point aléatoire, ramené au creux voisin), et chaque cellule
//!    appartient au bassin où sa **descente** l'emmène (voir `regions`). Les frontières ne suivent
//!    plus aucune grille : elles sont irrégulières et sensibles au relief.
//! 3. **Bassins versants** : l'ensemble des bassins qui se vident au même exutoire (embouchure ou
//!    lac terminal). Son identité sert aux cartes et aux contrats de bord.
//!
//! ## Niveau des bassins
//!
//! Chaque bassin a une **clé** totale `(altitude de la graine + légère pente vers le rivage,
//! hachage, cellule)`. Il s'écoule vers celui de ses voisins (en contact par une arête de cellules)
//! de clé strictement plus basse, par la **passe la plus basse** de leur frontière. Une clé qui
//! décroît strictement le long de chaque arête rend le graphe **acyclique**. Sans voisin plus bas,
//! le bassin est un **puits déclaré** : océan (sa graine est sous la mer) ou **lac terminal** (bassin
//! fermé intentionnel). La profondeur est **bornée par définition** : un bassin dont la marche
//! dépasse [`MAX_BASIN_HOPS`] arêtes devient lui-même un puits (« lac plafonné »).
//!
//! ## Niveau des cellules
//!
//! Dans un bassin, les cellules forment un **arbre de moindre coût** enraciné à l'exutoire (la
//! cellule de sortie, au bord) ou, pour un puits, à la graine. Le coût d'un pas pénalise l'altitude :
//! les cours suivent les fonds de vallée, les affluents descendent les versants et se fondent dans
//! le cours principal. La cellule de sortie passe à la cellule d'entrée du bassin aval (contact
//! voisin), qui est une cellule ordinaire de son arbre : le fleuve traverse chaque bassin par le
//! chemin le moins coûteux, et les affluents s'y raccordent. Les cellules de mer sont des racines
//! supplémentaires (l'eau qui atteint la mer s'y arrête). L'arbre d'un bassin couvre exactement ses
//! cellules, toutes **connexes** par construction : aucun orphelin, aucun cycle.
//!
//! ## Contrat de bord
//!
//! La passe entre deux bassins A → B ([`Crossing`]) et ses attributs ([`EdgeContract`] : identité du
//! bassin versant, accumulation, largeur, niveaux d'eau de part et d'autre) sont des fonctions pures de
//! l'arête ordonnée (A, B) : les deux voisins appellent la même fonction.
//!
//! ## Profil d'altitude et accumulation
//!
//! Le **niveau d'eau** d'une cellule est le maximum, le long de son chemin vers l'exutoire, de
//! `altitude - RIVER_DEPTH`, plancher à la mer ou au niveau du lac terminal : il **ne remonte jamais
//! vers l'aval**. Si le terrain monte vers l'aval, c'est le terrain que le lot 4 rehausse (remblai).
//! L'**accumulation** d'une cellule est le nombre de cellules en amont (elle comprise, les bassins
//! amont inclus par leur total à la cellule d'entrée), saturé à [`ACCUMULATION_CAP`] : elle croît
//! vers l'aval et couvre la somme des affluents. Largeur et profondeur s'en déduisent
//! ([`river_width`]).
//!
//! # Lacs
//!
//! - **Lacs terminaux** : un puits qui n'est pas l'océan est un lac au niveau de sa graine.
//! - **Lacs de passage** : certaines cellules de rivière (tirage, isolées dans un voisinage de 7 × 7)
//!   portent un lac au niveau de la rivière en ce point ; le déversoir est l'arête aval.
//!
//! # Coût et caches
//!
//! Tout est mémoïsé dans des tables bornées (vidées quand elles débordent) ; un cache accélère une
//! valeur sans jamais la décider. [`Hydrology::stats`] compte les évaluations qui ratent le cache :
//! sur un contexte neuf, c'est le coût à froid d'une requête.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;

use super::SEA_LEVEL;
use super::geography::Geography;
use super::macro_noise::{gradient_noise, lattice, unit};

mod regions;
mod rivers;

use regions::{NO_PARENT, Region, RegionTree, SeedInfo};

/// Côté d'une cellule fine, en blocs.
pub const FINE_SIZE: i32 = 128;
/// Cellules fines par côté de super-cellule : un bassin couvre en moyenne autant de cellules.
pub const BASIN_FINE: i32 = 8;
/// Côté d'une super-cellule (une graine de bassin chacune), en blocs.
pub const BASIN_SIZE: i32 = FINE_SIZE * BASIN_FINE;
/// Nombre maximal d'arêtes inter-bassins d'un chemin (voir la documentation du module).
pub const MAX_BASIN_HOPS: u32 = 48;
/// Nombre maximal de pas de l'appartenance d'une cellule à son bassin : à chaque pas, le carré de la
/// distance pondérée à la graine diminue d'au moins 0,49 (poids minimal au carré) depuis 192 au plus
/// (7 cellules en diagonale × 1,4 de poids, au carré), soit 392 pas au plus.
pub const MAX_OWNER_STEPS: usize = 392;
/// Nombre maximal de cellules d'un bassin (carré de côté `2 × MAX_OWNER_STEPS + 1` autour de la
/// graine) : borne théorique très large, la taille réelle est d'une centaine de cellules.
pub const MAX_BASIN_CELLS: usize = (2 * MAX_OWNER_STEPS + 1) * (2 * MAX_OWNER_STEPS + 1);
/// Nombre maximal de pas fins dans un bassin : l'arbre est connexe, sa profondeur est au plus le
/// nombre de cellules.
pub const MAX_STEPS_IN_BASIN: usize = MAX_BASIN_CELLS;
/// Nombre maximal de cellules fines d'un chemin complet.
pub const MAX_FINE_PATH: usize = (MAX_BASIN_HOPS as usize + 1) * (MAX_STEPS_IN_BASIN + 1);
/// Saturation de l'accumulation (nombre de cellules amont).
pub const ACCUMULATION_CAP: u32 = 1024;
/// Accumulation à partir de laquelle une arête est dessinée comme un cours d'eau (sinon : ruissellement).
pub const STREAM_MIN_ACCUMULATION: u32 = 6;
/// Le lit est creusé de cette profondeur sous l'altitude macro du nœud, pour le profil d'eau.
pub const RIVER_DEPTH: f32 = 1.5;
/// Segments par courbe de cours d'eau (voir `rivers`).
pub const RIVER_SEGMENTS: usize = 6;
/// Largeur maximale d'une rivière, en blocs (avant l'évasement des embouchures).
pub const MAX_RIVER_WIDTH: f32 = 64.0;
/// Les tables de cache sont vidées au-delà de ce nombre d'entrées.
pub const DEFAULT_CACHE_LIMIT: usize = 1 << 18;
/// Nombre de tables de cache (borne de [`Hydrology::cached_entries`] : `CACHE_TABLES × limite`).
pub const CACHE_TABLES: usize = 12;
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
const SALT_SEED_WEIGHT: u64 = 0x4a;
const SALT_ROUGH: u64 = 0x4b;
const SALT_MEANDER: u64 = 0x4c;
const SALT_BORDER: u64 = 0x4d;
/// Déplacements (blocs) du point, grande et petite octave, dans [`Hydrology::basin_at`].
const BORDER_WARP: (f64, f64) = (50.0, 18.0);

/// Une cellule fine, par ses indices (x et z divisés par [`FINE_SIZE`], arrondis vers le bas).
pub type FineCell = (i32, i32);
/// Un bassin, par les indices de la super-cellule de sa graine.
pub type BasinCell = (i32, i32);

/// Pourquoi un bassin est un puits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkKind {
    /// L'océan : la graine du bassin est sous le niveau de la mer.
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
    /// La cellule d'entrée, dans le bassin aval, voisine de la sortie (8 voisins).
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
    /// Le bassin qui porte l'identité du bassin versant : le dernier bassin avant l'océan (l'embouchure)
    /// ou le puits lui-même (lac terminal, ou bassin océanique).
    pub outlet: BasinCell,
    pub capped: bool,
}

/// Le contrat de bord d'une arête A → B : lu à l'identique par les deux voisins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeContract {
    pub from: BasinCell,
    pub to: BasinCell,
    pub exit: FineCell,
    pub entry: FineCell,
    /// L'identité du bassin versant (l'embouchure ou le lac où l'eau finit).
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
    /// 0 à la source d'un ruisseau, 1 dès qu'il est établi : l'intensité de sa vallée.
    pub taper: f32,
}

/// Compteurs de travail : un échec de cache = un calcul.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HydroStats {
    /// Altitudes macro évaluées (le poste dominant).
    pub elevations: u64,
    /// Tous les calculs mémoïsés (altitudes, graines, bassins, passes, arbres, niveaux, accumulations).
    pub work: u64,
}

/// La largeur d'une rivière (blocs) selon son accumulation : un ruisseau de quelques blocs, des
/// fleuves de plusieurs dizaines de blocs, croissante et bornée par [`MAX_RIVER_WIDTH`].
pub fn river_width(accumulation: u32) -> f32 {
    (2.5 + 1.3 * (accumulation as f32).powf(0.55)).min(MAX_RIVER_WIDTH)
}

/// La cellule fine contenant le point (x, z).
pub fn fine_cell(x: f64, z: f64) -> FineCell {
    (
        (x / FINE_SIZE as f64).floor() as i32,
        (z / FINE_SIZE as f64).floor() as i32,
    )
}

/// Une table mémoïque bornée : vidée au débordement, jamais décisive.
struct Memo<K, V> {
    map: Mutex<FxHashMap<K, V>>,
    limit: usize,
}

impl<K: Eq + std::hash::Hash + Copy, V: Clone> Memo<K, V> {
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
            .cloned()
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

/// L'hydrologie d'une graine.
pub struct Hydrology {
    seed: u32,
    geography: Geography,
    elevations: Memo<FineCell, f32>,
    seeds: Memo<BasinCell, SeedInfo>,
    owners: Memo<FineCell, BasinCell>,
    regions: Memo<BasinCell, Arc<Region>>,
    natural: Memo<BasinCell, BasinFlow>,
    chains: Memo<BasinCell, BasinChain>,
    trees: Memo<BasinCell, Arc<RegionTree>>,
    accumulations: Memo<BasinCell, Arc<Vec<u32>>>,
    totals: Memo<BasinCell, u32>,
    levels: Memo<FineCell, f32>,
    flow_lakes: Memo<FineCell, Option<Lake>>,
    edges: Memo<FineCell, Option<rivers::Edge>>,
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
            seeds: Memo::new(limit),
            owners: Memo::new(limit),
            regions: Memo::new(limit),
            natural: Memo::new(limit),
            chains: Memo::new(limit),
            trees: Memo::new(limit),
            accumulations: Memo::new(limit),
            totals: Memo::new(limit),
            levels: Memo::new(limit),
            flow_lakes: Memo::new(limit),
            edges: Memo::new(limit),
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
        self.seeds.clear();
        self.owners.clear();
        self.regions.clear();
        self.natural.clear();
        self.chains.clear();
        self.trees.clear();
        self.accumulations.clear();
        self.totals.clear();
        self.levels.clear();
        self.flow_lakes.clear();
        self.edges.clear();
    }

    /// Le nombre d'entrées en cache, toutes tables confondues (pour vérifier la borne mémoire).
    pub fn cached_entries(&self) -> usize {
        self.elevations.len()
            + self.seeds.len()
            + self.owners.len()
            + self.regions.len()
            + self.natural.len()
            + self.chains.len()
            + self.trees.len()
            + self.accumulations.len()
            + self.totals.len()
            + self.levels.len()
            + self.flow_lakes.len()
            + self.edges.len()
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

    // ---- identité des bassins ----

    /// L'identité du bassin versant d'un bassin : un hachage de son exutoire (embouchure ou lac).
    pub fn basin_id(&self, b: BasinCell) -> u64 {
        let t = self.chain(b).outlet;
        lattice(self.seed, SALT_BASIN_ID, t.0, t.1) | 1
    }

    /// La cellule où l'eau d'un bassin finit : sa cellule la plus basse (nœud du lac terminal) pour un
    /// puits, sa graine pour un bassin qui s'écoule.
    pub fn basin_target(&self, b: BasinCell) -> FineCell {
        match self.basin_flow(b) {
            BasinFlow::Sink(_) => self.sink_cell(b),
            BasinFlow::Flow { .. } => self.seed_info(b).cell,
        }
    }

    /// Le bassin du point (x, z) : celui de la cellule dont le nœud est le plus proche (parmi la cellule
    /// du point et ses 8 voisines), pour que les frontières ne suivent pas la grille de 128 blocs.
    pub fn basin_at(&self, x: f64, z: f64) -> BasinCell {
        // Un déplacement du point (deux octaves, au plus ~70 blocs) casse les arêtes droites et les
        // marches de 128 blocs des polygones.
        let wobble = |salt: u64| {
            BORDER_WARP.0 * gradient_noise(self.seed, salt, x / 190.0, z / 190.0)
                + BORDER_WARP.1 * gradient_noise(self.seed, salt ^ 0x100, x / 70.0, z / 70.0)
        };
        let (wx, wz) = (x + wobble(SALT_BORDER), z + wobble(SALT_BORDER ^ 1));
        let c = fine_cell(wx, wz);
        let mut best: Option<(f64, FineCell)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let g = (c.0 + dx, c.1 + dz);
                let n = self.node(g);
                let d = (wx - n[0]).powi(2) + (wz - n[1]).powi(2);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, g));
                }
            }
        }
        self.basin_of(best.expect("neuf cellules").1)
    }

    /// L'identité du bassin versant du point (x, z) (voir [`Hydrology::basin_at`]).
    pub fn basin_id_at(&self, x: f64, z: f64) -> u64 {
        self.basin_id(self.basin_at(x, z))
    }

    // ---- niveau fin : les cellules ----

    /// La cellule suivante vers l'aval, ou `None` pour une cellule terminale : mer, ou graine d'un
    /// puits (lac terminal).
    pub fn next(&self, f: FineCell) -> Option<FineCell> {
        if self.is_sea(f) {
            return None;
        }
        let b = self.basin_of(f);
        let region = self.region(b);
        let tree = self.tree(b);
        let i = *region
            .index
            .get(&f)
            .expect("une cellule est dans son bassin");
        let parent = tree.parent[i as usize];
        if parent != NO_PARENT {
            return Some(region.cells[parent as usize]);
        }
        if tree.exit == Some(i)
            && let BasinFlow::Flow { crossing, .. } = self.basin_flow(b)
        {
            return Some(crossing.entry);
        }
        None
    }

    /// La profondeur de la cellule dans l'arbre de son bassin (0 pour une racine).
    pub fn depth_in_basin(&self, f: FineCell) -> u32 {
        let b = self.basin_of(f);
        let region = self.region(b);
        let tree = self.tree(b);
        tree.depth[*region
            .index
            .get(&f)
            .expect("une cellule est dans son bassin") as usize]
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
        let b = self.basin_of(f);
        let region = self.region(b);
        let acc = self.region_accumulations(b);
        acc[*region
            .index
            .get(&f)
            .expect("une cellule est dans son bassin") as usize]
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
                let sink = self.sink_cell(b);
                Some(Lake {
                    center: self.node(sink),
                    radius: TERMINAL_LAKE_RADIUS.0
                        + TERMINAL_LAKE_RADIUS.1 * unit(self.seed, SALT_LAKE_RADIUS, b.0, b.1),
                    level: self.elevation(sink),
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
        // Lacs terminaux : seules les graines assez proches méritent le calcul du drainage.
        let centre = [
            (f.0 as f64 + 0.5) * FINE_SIZE as f64,
            (f.1 as f64 + 0.5) * FINE_SIZE as f64,
        ];
        let reach = (TERMINAL_LAKE_RADIUS.0 + TERMINAL_LAKE_RADIUS.1) * 1.25
            + FINE_SIZE as f64 * std::f64::consts::FRAC_1_SQRT_2;
        let sb = (f.0.div_euclid(BASIN_FINE), f.1.div_euclid(BASIN_FINE));
        for dz in -1..=1 {
            for dx in -1..=1 {
                let b = (sb.0 + dx, sb.1 + dz);
                let node = self.node(self.seed_info(b).cell);
                if (node[0] - centre[0]).hypot(node[1] - centre[1]) > reach {
                    continue;
                }
                if let Some(l) = self.terminal_lake(b) {
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
}

/// Rayon d'un lac terminal : minimum et étendue (blocs).
const TERMINAL_LAKE_RADIUS: (f64, f64) = (130.0, 130.0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_grows_with_accumulation_and_is_bounded() {
        let mut last = 0.0;
        for acc in 1..=ACCUMULATION_CAP {
            let w = river_width(acc);
            assert!(w >= last && w <= MAX_RIVER_WIDTH);
            last = w;
        }
        assert!(
            river_width(ACCUMULATION_CAP) > 40.0,
            "les fleuves sont larges"
        );
        assert!(
            river_width(STREAM_MIN_ACCUMULATION) < 8.0,
            "les ruisseaux sont étroits"
        );
    }

    #[test]
    fn fine_cell_uses_floor_division_for_negative_points() {
        assert_eq!(fine_cell(-0.5, 127.9), (-1, 0));
        assert_eq!(fine_cell(-128.0, -129.0), (-1, -2));
    }
}
