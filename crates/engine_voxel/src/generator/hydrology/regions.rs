//! Les bassins de forme libre du générateur v2 : graines, appartenance des cellules, graphe des
//! bassins, arbres de drainage et accumulation.
//!
//! # L'appartenance : une descente, pas une grille
//!
//! Chaque super-cellule de [`BASIN_FINE`] × [`BASIN_FINE`] cellules a une **graine** : un point
//! aléatoire de la super-cellule, ramené à la cellule la plus basse de son voisinage de 3 × 3 (un
//! centre pris au hasard évite qu'un terrain en pente range toutes les graines au même coin, ce qui
//! dessinerait un réseau carré). Chaque graine a un
//! poids de distance entre [`WEIGHT_MIN`] et [`WEIGHT_MIN`] + [`WEIGHT_SPAN`] (bassins de tailles
//! variées, frontières courbes). La graine **la plus proche** d'une cellule (distance pondérée, parmi
//! les 7 × 7 super-cellules voisines, par couronnes élaguées : une preuve par la borne de distance
//! l'établit, et un test la recoupe) est son attracteur.
//!
//! Une cellule qui n'est pas une graine **descend** : elle prend, parmi ses voisins strictement plus
//! proches de son attracteur, le plus bas (relief et petit tirage). Le bassin d'une cellule est celui
//! où cette descente aboutit. Conséquences, toutes par construction :
//!
//! - les bassins sont **connexes** (chaque cellule est reliée à sa graine par des cellules du même
//!   bassin) : jamais d'orphelin ;
//! - la descente se termine : la distance pondérée à la graine la plus proche décroît à chaque pas
//!   d'au moins 0,49 en carré (poids minimal au carré), depuis 192 au plus : [`MAX_OWNER_STEPS`] = 392
//!   pas au plus ;
//! - les frontières suivent le relief : près d'une bissectrice, la pente décide du bassin.
//!
//! Une cellule ne dépend que d'altitudes de cellules voisines et de graines proches : c'est une
//! fonction pure de `(graine du monde, cellule)`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::{
    ACCUMULATION_CAP, BASIN_FINE, BasinCell, BasinChain, BasinFlow, Crossing, FineCell, Hydrology,
    MAX_BASIN_HOPS, MAX_OWNER_STEPS, SALT_CROSSING, SALT_ROUGH, SALT_SEED_WEIGHT, SALT_STEP,
    SALT_TIE, SinkKind,
};
use crate::generator::SEA_LEVEL;
use crate::generator::macro_noise::{lattice, unit};

/// Salts du centre aléatoire d'une graine, dans sa super-cellule.
const SALT_SEED_X: u64 = 0x4e;
const SALT_SEED_Z: u64 = 0x4f;
/// Poids de distance d'une graine : `WEIGHT_MIN + WEIGHT_SPAN × tirage`.
const WEIGHT_MIN: f64 = 0.7;
const WEIGHT_SPAN: f64 = 0.7;
/// Rayon maximal (en super-cellules) de la fenêtre des graines candidates : la graine d'une
/// super-cellule est n'importe où dans celle-ci, donc une graine de la couronne `r` est à au moins
/// `r × BASIN_FINE - (BASIN_FINE - 1)` cellules, et à partir de la couronne 4 elle ne peut plus
/// l'emporter sur la graine de la propre super-cellule de la cellule (distance pondérée au plus
/// 7 × √2 × 1,4 = 13,9 contre au moins 25 × 0,7 = 17,5).
const WINDOW: i32 = 3;
/// Pente du rivage dans la clé d'ordre des bassins : des blocs par bloc de distance au rivage
/// (au plus [`TILT_MAX_DISTANCE`]) : elle fait préférer l'écoulement vers la mer aux lacs fermés.
const TILT: f64 = 0.012;
const TILT_MAX_DISTANCE: f64 = 2600.0;
/// Amplitude (blocs) du tirage qui fait serpenter la descente vers la graine.
const MEANDER: f32 = 3.0;
/// Coût d'un pas de l'arbre : `longueur × (1 + HILL_COST × altitude relative) × (1 + ROUGHNESS × tirage)`.
const HILL_COST: f64 = 5.0;
const ROUGHNESS: f64 = 0.45;

/// Pas de l'arbre d'une cellule sans parent (une racine).
pub(super) const NO_PARENT: u32 = u32::MAX;

/// La graine d'un bassin.
#[derive(Debug, Clone, Copy)]
pub(super) struct SeedInfo {
    pub cell: FineCell,
    pub z: f32,
    pub weight: f64,
    /// La clé d'ordre du bassin : altitude de la graine plus la pente du rivage (1/256 de bloc).
    pub key: i64,
    pub tie: u64,
}

/// La frontière entre un bassin et un voisin : la passe la plus basse.
#[derive(Debug, Clone, Copy)]
pub(super) struct Border {
    pub to: BasinCell,
    pub exit: FineCell,
    pub entry: FineCell,
    pub pass: f32,
    tie: u64,
}

/// Les cellules d'un bassin (par ordre de découverte, la graine d'abord) et ses frontières.
#[derive(Debug)]
pub(super) struct Region {
    pub cells: Vec<FineCell>,
    pub index: FxHashMap<FineCell, u32>,
    pub borders: Vec<Border>,
}

/// L'arbre de drainage d'un bassin : parent de chaque cellule (indice dans `cells`), profondeur,
/// ordre de découverte par coût croissant (un parent avant ses enfants), indice de la cellule de
/// sortie.
#[derive(Debug)]
pub(super) struct RegionTree {
    pub parent: Vec<u32>,
    pub depth: Vec<u32>,
    pub order: Vec<u32>,
    pub exit: Option<u32>,
}

fn dist2(a: FineCell, b: FineCell) -> i64 {
    let (dx, dz) = ((a.0 - b.0) as i64, (a.1 - b.1) as i64);
    dx * dx + dz * dz
}

const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

impl Hydrology {
    // ---- graines ----

    pub(super) fn seed_info(&self, b: BasinCell) -> SeedInfo {
        if let Some(s) = self.seeds.get(&b) {
            return s;
        }
        // Un centre aléatoire dans la super-cellule, puis la cellule la plus basse de son voisinage de
        // 3 × 3 : des graines irrégulières, avec un léger penchant pour les creux.
        let centre = |salt: u64| (unit(self.seed, salt, b.0, b.1) * BASIN_FINE as f64) as i32;
        let (cx, cz) = (centre(SALT_SEED_X), centre(SALT_SEED_Z));
        let mut best: Option<(f32, u64, FineCell)> = None;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (ox, oz) = (cx + dx, cz + dz);
                if !(0..BASIN_FINE).contains(&ox) || !(0..BASIN_FINE).contains(&oz) {
                    continue;
                }
                let f = (b.0 * BASIN_FINE + ox, b.1 * BASIN_FINE + oz);
                let e = self.elevation(f);
                let tie = lattice(self.seed, SALT_TIE, f.0, f.1);
                if best.is_none_or(|(be, bt, _)| (e, tie) < (be, bt)) {
                    best = Some((e, tie, f));
                }
            }
        }
        let (z, tie, cell) = best.expect("un voisinage a des cellules");
        let [x, zz] = self.node(cell);
        let shore = self
            .geography
            .coast_distance_at(x, zz)
            .clamp(0.0, TILT_MAX_DISTANCE);
        let info = SeedInfo {
            cell,
            z,
            weight: WEIGHT_MIN + WEIGHT_SPAN * unit(self.seed, SALT_SEED_WEIGHT, b.0, b.1),
            key: ((z as f64 + TILT * shore) * 256.0).round() as i64,
            tie,
        };
        self.count();
        self.seeds.put(b, info);
        info
    }

    /// Vrai si `a` est strictement plus bas que `b` dans l'ordre total des bassins.
    pub(super) fn lower(&self, a: BasinCell, b: BasinCell) -> bool {
        let (sa, sb) = (self.seed_info(a), self.seed_info(b));
        (sa.key, sa.tie, sa.cell, a) < (sb.key, sb.tie, sb.cell, b)
    }

    /// La graine la plus proche d'une cellule (distance pondérée, égalité : la plus petite
    /// super-cellule).
    fn nearest(&self, c: FineCell) -> (BasinCell, SeedInfo) {
        let sb = (c.0.div_euclid(BASIN_FINE), c.1.div_euclid(BASIN_FINE));
        let mut best: Option<(f64, BasinCell, SeedInfo)> = None;
        let consider = |b: BasinCell, best: &mut Option<(f64, BasinCell, SeedInfo)>| {
            let s = self.seed_info(b);
            let v = s.weight * s.weight * dist2(c, s.cell) as f64;
            if best.is_none_or(|(bv, bb, _)| v < bv || (v == bv && b < bb)) {
                *best = Some((v, b, s));
            }
        };
        for dz in -1..=1 {
            for dx in -1..=1 {
                consider((sb.0 + dx, sb.1 + dz), &mut best);
            }
        }
        // Les couronnes suivantes ne sont examinées que si une de leurs graines peut l'emporter.
        for ring in 2..=WINDOW {
            let (best_v, ..) = best.expect("des candidats");
            let floor = (ring * BASIN_FINE - (BASIN_FINE - 1)) as f64 * WEIGHT_MIN;
            if best_v <= floor * floor {
                break;
            }
            for dz in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs().max(dz.abs()) == ring {
                        consider((sb.0 + dx, sb.1 + dz), &mut best);
                    }
                }
            }
        }
        let (_, b, s) = best.expect("des candidats");
        (b, s)
    }

    /// Un pas de la descente d'une cellule vers sa graine : le voisin strictement plus proche, le
    /// plus bas.
    fn descent_step(&self, c: FineCell, seed: FineCell) -> FineCell {
        let here = dist2(c, seed);
        let mut best: Option<(f32, u64, FineCell)> = None;
        for (dx, dz) in NEIGHBOURS {
            let n = (c.0 + dx, c.1 + dz);
            if dist2(n, seed) >= here {
                continue;
            }
            let tie = lattice(self.seed, SALT_STEP, n.0, n.1);
            let cost = self.elevation(n) + MEANDER * (tie >> 40) as f32 / (1u64 << 24) as f32;
            if best.is_none_or(|(bc, bt, _)| (cost, tie) < (bc, bt)) {
                best = Some((cost, tie, n));
            }
        }
        // Un voisin plus proche existe toujours (un pas vers la graine) ; le repli par le signe est
        // exactement ce pas.
        best.map(|(_, _, n)| n)
            .unwrap_or((c.0 + (seed.0 - c.0).signum(), c.1 + (seed.1 - c.1).signum()))
    }

    /// Le bassin d'une cellule : celui où sa descente aboutit (voir la documentation du module).
    pub fn basin_of(&self, f: FineCell) -> BasinCell {
        if let Some(o) = self.owners.get(&f) {
            return o;
        }
        let mut path: Vec<FineCell> = Vec::new();
        let mut cur = f;
        let owner = loop {
            if let Some(o) = self.owners.get(&cur) {
                break o;
            }
            let (b, seed) = self.nearest(cur);
            if cur == seed.cell {
                break b;
            }
            path.push(cur);
            assert!(
                path.len() <= MAX_OWNER_STEPS,
                "descente plus longue que sa borne"
            );
            cur = self.descent_step(cur, seed.cell);
        };
        self.count();
        self.owners.put(cur, owner);
        for p in path {
            self.owners.put(p, owner);
        }
        owner
    }

    // ---- les cellules d'un bassin ----

    /// Les cellules d'un bassin et ses frontières, par un parcours depuis la graine : un bassin est
    /// connexe, le parcours le trouve tout entier.
    pub(super) fn region(&self, b: BasinCell) -> Arc<Region> {
        if let Some(r) = self.regions.get(&b) {
            return r;
        }
        let seed = self.seed_info(b).cell;
        let mut cells = vec![seed];
        let mut index: FxHashMap<FineCell, u32> = FxHashMap::default();
        index.insert(seed, 0);
        let mut borders: Vec<Border> = Vec::new();
        let mut head = 0;
        while head < cells.len() {
            let x = cells[head];
            head += 1;
            let zx = self.elevation(x);
            for (dx, dz) in NEIGHBOURS {
                let y = (x.0 + dx, x.1 + dz);
                let o = self.basin_of(y);
                if o == b {
                    if let std::collections::hash_map::Entry::Vacant(e) = index.entry(y) {
                        e.insert(cells.len() as u32);
                        cells.push(y);
                    }
                    continue;
                }
                let pass = zx.max(self.elevation(y));
                let tie = lattice(
                    self.seed,
                    SALT_CROSSING,
                    x.0.wrapping_mul(31).wrapping_add(y.0),
                    x.1.wrapping_mul(31).wrapping_add(y.1),
                );
                let border = Border {
                    to: o,
                    exit: x,
                    entry: y,
                    pass,
                    tie,
                };
                match borders.iter_mut().find(|e| e.to == o) {
                    Some(e) => {
                        if (pass, tie) < (e.pass, e.tie) {
                            *e = border;
                        }
                    }
                    None => borders.push(border),
                }
            }
        }
        borders.sort_by_key(|e| e.to);
        let region = Arc::new(Region {
            cells,
            index,
            borders,
        });
        self.count();
        self.regions.put(b, region.clone());
        region
    }

    /// Les bassins voisins (en contact par une arête de cellules), triés.
    pub fn neighbours(&self, b: BasinCell) -> Vec<BasinCell> {
        self.region(b).borders.iter().map(|e| e.to).collect()
    }

    /// Le nombre de cellules d'un bassin.
    pub fn basin_cell_count(&self, b: BasinCell) -> usize {
        self.region(b).cells.len()
    }

    /// La passe de `from` vers son voisin `to` (la paire de cellules de contact la plus basse), s'il
    /// est bien en contact.
    pub fn crossing(&self, from: BasinCell, to: BasinCell) -> Option<Crossing> {
        self.region(from)
            .borders
            .iter()
            .find(|e| e.to == to)
            .map(|e| Crossing {
                exit: e.exit,
                entry: e.entry,
                pass: e.pass,
            })
    }

    // ---- le graphe des bassins ----

    /// L'arête « naturelle » d'un bassin, avant la borne de profondeur.
    pub(super) fn natural_flow(&self, b: BasinCell) -> BasinFlow {
        if let Some(f) = self.natural.get(&b) {
            return f;
        }
        let coastal = self.seed_info(b).z < SEA_LEVEL as f32 || {
            let region = self.region(b);
            region.cells.iter().any(|&c| self.is_sea(c))
        };
        let flow = if coastal {
            // Un bassin qui touche la mer s'y vide : ses cellules de mer sont les racines de son arbre.
            BasinFlow::Sink(SinkKind::Ocean)
        } else {
            let region = self.region(b);
            let mut best: Option<&Border> = None;
            for border in &region.borders {
                if !self.lower(border.to, b) {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some(o) => {
                        border.pass < o.pass
                            || (border.pass == o.pass && self.lower(border.to, o.to))
                    }
                };
                if better {
                    best = Some(border);
                }
            }
            match best {
                Some(e) => BasinFlow::Flow {
                    to: e.to,
                    crossing: Crossing {
                        exit: e.exit,
                        entry: e.entry,
                        pass: e.pass,
                    },
                },
                // Sans voisin plus bas : un lac terminal (bassin fermé, sans mer).
                None => BasinFlow::Sink(SinkKind::Lake),
            }
        };
        self.count();
        self.natural.put(b, flow);
        flow
    }

    /// Vrai si le bassin est de la pleine mer (sa graine est sous le niveau de la mer) : l'embouchure
    /// d'un bassin versant est alors le bassin précédent, alors qu'un bassin côtier (qui touche la mer)
    /// est lui-même l'embouchure.
    pub fn is_open_sea(&self, b: BasinCell) -> bool {
        self.seed_info(b).z < SEA_LEVEL as f32
    }

    /// La profondeur d'un bassin : au plus [`MAX_BASIN_HOPS`] arêtes, sinon c'est un puits plafonné.
    pub fn chain(&self, b: BasinCell) -> BasinChain {
        if let Some(c) = self.chains.get(&b) {
            return c;
        }
        let capped = BasinChain {
            hops: MAX_BASIN_HOPS + 1,
            terminal: b,
            outlet: b,
            capped: true,
        };
        let mut path = vec![b];
        let mut cur = b;
        // `tail` : (arêtes après le dernier bassin du chemin, puits atteint, exutoire du dernier
        // bassin du chemin).
        let tail = loop {
            match self.natural_flow(cur) {
                BasinFlow::Sink(_) => break Some((0, cur, cur)),
                BasinFlow::Flow { to, .. } => {
                    if let Some(known) = self.chains.get(&to) {
                        break if known.capped {
                            None
                        } else {
                            let outlet = if known.hops == 0 && self.is_open_sea(to) {
                                cur
                            } else {
                                known.outlet
                            };
                            Some((known.hops + 1, known.terminal, outlet))
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
        let Some((after_last, terminal, last_outlet)) = tail else {
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
        let sink_is_ocean = after_last == 0 && self.is_open_sea(terminal);
        let mut outlet = last_outlet;
        for (i, &cell) in path.iter().enumerate().rev() {
            // Le bassin juste avant un puits océanique porte l'identité de l'embouchure.
            if i + 2 == path.len() && sink_is_ocean {
                outlet = cell;
            }
            self.chains.put(
                cell,
                BasinChain {
                    hops: (len - 1 - i as u32) + after_last,
                    terminal,
                    outlet,
                    capped: false,
                },
            );
        }
        self.count();
        BasinChain {
            hops: len - 1 + after_last,
            terminal,
            outlet,
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

    /// La cellule terminale d'un bassin sans exutoire : sa cellule la plus basse (et non sa graine, qui
    /// n'est que le creux voisin d'un point tiré au hasard : l'eau ne doit jamais remonter vers elle).
    pub(super) fn sink_cell(&self, b: BasinCell) -> FineCell {
        let region = self.region(b);
        let mut best: Option<(f32, u64, FineCell)> = None;
        for &c in &region.cells {
            let (e, tie) = (self.elevation(c), lattice(self.seed, SALT_TIE, c.0, c.1));
            if best.is_none_or(|(be, bt, _)| (e, tie) < (be, bt)) {
                best = Some((e, tie, c));
            }
        }
        best.expect("un bassin a des cellules").2
    }

    // ---- l'arbre de drainage ----

    /// Le coût du pas d'une cellule `from` vers `to` (millièmes) : l'altitude relative de la cible
    /// pèse beaucoup, pour que les cours suivent les fonds de vallée.
    fn step_cost(&self, from: FineCell, to: FineCell, rel_to: f64) -> i64 {
        let len = if from.0 != to.0 && from.1 != to.1 {
            std::f64::consts::SQRT_2
        } else {
            1.0
        };
        let rough = unit(self.seed, SALT_ROUGH, from.0, from.1);
        ((len * (1.0 + HILL_COST * rel_to) * (1.0 + ROUGHNESS * rough)) * 1024.0).round() as i64
    }

    /// L'arbre de moindre coût d'un bassin, enraciné à son exutoire (ou à sa graine pour un puits)
    /// et aux cellules de mer.
    pub(super) fn tree(&self, b: BasinCell) -> Arc<RegionTree> {
        if let Some(t) = self.trees.get(&b) {
            return t;
        }
        let region = self.region(b);
        let n = region.cells.len();
        let z: Vec<f32> = region.cells.iter().map(|&c| self.elevation(c)).collect();
        let (zmin, zmax) = z
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let span = (zmax - zmin) as f64 + 12.0;
        let rel: Vec<f64> = z.iter().map(|&v| (v - zmin) as f64 / span).collect();
        let exit = match self.basin_flow(b) {
            BasinFlow::Flow { crossing, .. } => region.index.get(&crossing.exit).copied(),
            BasinFlow::Sink(_) => region.index.get(&self.sink_cell(b)).copied(),
        };
        let mut dist = vec![i64::MAX; n];
        let mut parent = vec![NO_PARENT; n];
        let mut heap: BinaryHeap<Reverse<(i64, u32)>> = BinaryHeap::new();
        for (i, &zi) in z.iter().enumerate() {
            if zi < SEA_LEVEL as f32 || exit == Some(i as u32) {
                dist[i] = 0;
                heap.push(Reverse((0, i as u32)));
            }
        }
        let mut order = Vec::with_capacity(n);
        while let Some(Reverse((d, i))) = heap.pop() {
            if d > dist[i as usize] {
                continue;
            }
            order.push(i);
            let c = region.cells[i as usize];
            for (dx, dz) in NEIGHBOURS {
                let Some(&j) = region.index.get(&(c.0 + dx, c.1 + dz)) else {
                    continue;
                };
                // Les racines (mer, exutoire) ne sont jamais rattachées à un autre parent.
                if dist[j as usize] == 0 {
                    continue;
                }
                let cand = d + self.step_cost(region.cells[j as usize], c, rel[i as usize]);
                if cand < dist[j as usize] {
                    dist[j as usize] = cand;
                    parent[j as usize] = i;
                    heap.push(Reverse((cand, j)));
                }
            }
        }
        debug_assert_eq!(order.len(), n, "un bassin est connexe");
        let mut depth = vec![0u32; n];
        for &i in &order {
            let p = parent[i as usize];
            if p != NO_PARENT {
                depth[i as usize] = depth[p as usize] + 1;
            }
        }
        let tree = Arc::new(RegionTree {
            parent,
            depth,
            order,
            exit: match self.basin_flow(b) {
                BasinFlow::Flow { .. } => exit,
                BasinFlow::Sink(_) => None,
            },
        });
        self.count();
        self.trees.put(b, tree.clone());
        tree
    }

    // ---- accumulation ----

    /// Le nombre de cellules qui se vident par l'exutoire d'un bassin (lui compris), saturé à
    /// [`ACCUMULATION_CAP`] : ses cellules, plus le total de chaque bassin amont qui s'y déverse. Les bassins
    /// amont sont parcourus dans l'ordre des voisins et le parcours s'arrête dès que la saturation est
    /// atteinte : le résultat (un minimum) ne change pas, et le travail est borné par la saturation.
    pub(super) fn region_total(&self, b: BasinCell) -> u32 {
        if let Some(t) = self.totals.get(&b) {
            return t;
        }
        let region = self.region(b);
        let mut sum = region.cells.len() as u64;
        for border in &region.borders {
            if sum >= ACCUMULATION_CAP as u64 {
                break;
            }
            if let BasinFlow::Flow { to, .. } = self.basin_flow(border.to)
                && to == b
            {
                sum += self.region_total(border.to) as u64;
            }
        }
        let total = sum.min(ACCUMULATION_CAP as u64) as u32;
        self.count();
        self.totals.put(b, total);
        total
    }

    /// L'accumulation de chaque cellule d'un bassin (saturée) : une cellule, plus tout ce qui s'y
    /// déverse, y compris le total des bassins amont qui entrent par elle.
    pub(super) fn region_accumulations(&self, b: BasinCell) -> Arc<Vec<u32>> {
        if let Some(a) = self.accumulations.get(&b) {
            return a;
        }
        let region = self.region(b);
        let tree = self.tree(b);
        let mut sum = vec![1u64; region.cells.len()];
        for border in &region.borders {
            let BasinFlow::Flow { to, crossing } = self.basin_flow(border.to) else {
                continue;
            };
            if to != b {
                continue;
            }
            let entry = *region
                .index
                .get(&crossing.entry)
                .expect("l'entrée est dans le bassin aval");
            sum[entry as usize] += self.region_total(border.to) as u64;
        }
        let mut acc = vec![0u32; region.cells.len()];
        for &i in tree.order.iter().rev() {
            let v = sum[i as usize].min(ACCUMULATION_CAP as u64) as u32;
            acc[i as usize] = v;
            let p = tree.parent[i as usize];
            if p != NO_PARENT {
                sum[p as usize] += v as u64;
            }
        }
        let acc = Arc::new(acc);
        self.count();
        self.accumulations.put(b, acc.clone());
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_seed_is_found_in_the_nearby_super_cells() {
        // Une recherche large (11 × 11 super-cellules) donne la même graine que la recherche réelle.
        let h = Hydrology::new(42);
        for i in 0..4000 {
            let c = (i * 37 % 500 - 250, i * 91 % 500 - 250);
            let (b, _) = h.nearest(c);
            let sb = (c.0.div_euclid(BASIN_FINE), c.1.div_euclid(BASIN_FINE));
            let mut best: Option<(f64, BasinCell)> = None;
            for dz in -5..=5 {
                for dx in -5..=5 {
                    let cand = (sb.0 + dx, sb.1 + dz);
                    let s = h.seed_info(cand);
                    let v = s.weight * s.weight * dist2(c, s.cell) as f64;
                    if best.is_none_or(|(bv, bb)| v < bv || (v == bv && cand < bb)) {
                        best = Some((v, cand));
                    }
                }
            }
            assert_eq!(Some(b), best.map(|(_, b)| b), "cellule {c:?}");
        }
    }

    #[test]
    fn a_basin_is_connected_and_every_cell_owns_its_basin() {
        let h = Hydrology::new(7);
        let (mut largest, mut smallest) = (0, usize::MAX);
        for i in 0..60 {
            let b = (i % 9 - 4, i / 9 - 3);
            let region = h.region(b);
            largest = largest.max(region.cells.len());
            smallest = smallest.min(region.cells.len());
            for &c in &region.cells {
                assert_eq!(h.basin_of(c), b);
            }
            assert_eq!(h.seed_info(b).cell, region.cells[0]);
            let tree = h.tree(b);
            assert_eq!(tree.order.len(), region.cells.len(), "arbre couvrant");
        }
        assert!(
            largest <= 400 && smallest >= 3,
            "tailles de bassins : de {smallest} à {largest} cellules"
        );
    }

    #[test]
    fn region_borders_are_symmetric() {
        let h = Hydrology::new(11);
        for i in 0..40 {
            let b = (i % 7 - 3, i / 7 - 3);
            for n in h.neighbours(b) {
                assert!(
                    h.neighbours(n).contains(&b),
                    "{b:?} voit {n:?}, pas l'inverse"
                );
            }
        }
    }
}
