//! Le graphe compilé : des nœuds rangés dans un tableau (les fonctions nommées sont partagées), leurs
//! bornes, et deux façons de l'évaluer :
//!
//! - [`DensityGraph::sample`] : une position, exactement (les `Interpolated` évaluent leur enfant) ;
//! - [`DensityGraph::fill_chunk`] : un chunk de 32³ blocs. Chaque `Interpolated` n'est calculé
//!   qu'aux coins des cellules de 4 × 8 × 4 blocs (9 × 5 × 9 coins) puis interpolé, et les caches
//!   (`FlatCache`, `Cache2d`, `CacheOnce`) évitent de refaire ce qui ne change pas.
//!
//! Les deux donnent la même valeur aux coins des cellules ; entre eux, `fill_chunk` interpole. Deux
//! chunks voisins partagent leurs coins de bord : l'interpolation est continue d'un chunk à l'autre.

use std::collections::{BTreeMap, HashMap};

use glam::IVec3;

use super::def::{DensityDef, DensityFile, SplineDef, SplineValue};
use super::noise::NormalNoise;
use crate::chunk::CHUNK_SIZE;

/// Un nœud du graphe, par son indice.
pub type NodeId = u32;

/// La largeur d'une cellule d'interpolation (x et z), en blocs.
pub const CELL_WIDTH: i32 = 4;
/// Sa hauteur, en blocs.
pub const CELL_HEIGHT: i32 = 8;
const SIZE: i32 = CHUNK_SIZE as i32;
/// Coins par axe dans un chunk : 9 en x et z, 5 en y.
const CORNERS_XZ: usize = (SIZE / CELL_WIDTH) as usize + 1;
const CORNERS_Y: usize = (SIZE / CELL_HEIGHT) as usize + 1;

/// Ce qui empêche de compiler un fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DensityError {
    Parse(String),
    UnknownNoise(String),
    UnknownFunction(String),
    Cycle(String),
    BadSpline(String),
}

impl std::fmt::Display for DensityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "fichier de densité illisible : {e}"),
            Self::UnknownNoise(n) => write!(f, "bruit inconnu : {n}"),
            Self::UnknownFunction(n) => write!(f, "fonction inconnue : {n}"),
            Self::Cycle(n) => write!(f, "la fonction {n} dépend d'elle-même"),
            Self::BadSpline(why) => write!(f, "spline invalide : {why}"),
        }
    }
}

impl std::error::Error for DensityError {}

#[derive(Debug, Clone, Copy)]
enum Binary {
    Add,
    Mul,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy)]
enum Unary {
    Abs,
    Square,
    Cube,
    HalfNegative,
    QuarterNegative,
    Squeeze,
}

#[derive(Debug, Clone)]
enum Node {
    Constant(f64),
    Noise {
        noise: usize,
        xz: f64,
        y: f64,
    },
    Shifted {
        noise: usize,
        xz: f64,
        y: f64,
        shift: [NodeId; 3],
    },
    Binary(Binary, NodeId, NodeId),
    Unary(Unary, NodeId),
    Clamp {
        input: NodeId,
        min: f64,
        max: f64,
    },
    YGradient {
        from_y: f64,
        to_y: f64,
        from_value: f64,
        to_value: f64,
    },
    Spline(usize),
    /// L'indice dans la liste des nœuds interpolés.
    Interpolated(NodeId, usize),
    FlatCache(NodeId, usize),
    Cache2d(NodeId, usize),
    CacheOnce(NodeId, usize),
    RangeChoice {
        input: NodeId,
        min: f64,
        max: f64,
        inside: NodeId,
        outside: NodeId,
    },
}

#[derive(Debug, Clone)]
enum CompiledValue {
    Constant(f64),
    Spline(usize),
}

#[derive(Debug, Clone)]
struct CompiledSpline {
    coordinate: NodeId,
    /// (abscisse, valeur, pente), abscisses croissantes.
    points: Vec<(f64, CompiledValue, f64)>,
}

/// Un graphe de densité compilé pour une graine.
#[derive(Debug, Clone)]
pub struct DensityGraph {
    nodes: Vec<Node>,
    bounds: Vec<(f64, f64)>,
    noises: Vec<NormalNoise>,
    splines: Vec<CompiledSpline>,
    spline_bounds: Vec<(f64, f64)>,
    names: BTreeMap<String, NodeId>,
    /// Combien de nœuds de chaque sorte de cache, et d'interpolés.
    interpolated: usize,
    flat: usize,
    cache2d: usize,
    once: usize,
}

struct Compiler<'a> {
    file: &'a DensityFile,
    seed: u64,
    graph: DensityGraph,
    noise_index: HashMap<String, usize>,
    in_progress: Vec<String>,
}

impl DensityGraph {
    /// Lit un fichier RON (voir `def.rs`) et le compile pour la graine `seed`.
    pub fn parse(ron_text: &str, seed: u64) -> Result<Self, DensityError> {
        let file: DensityFile =
            ron::from_str(ron_text).map_err(|e| DensityError::Parse(e.to_string()))?;
        Self::compile(&file, seed)
    }

    /// Compile toutes les fonctions du fichier pour la graine `seed`.
    pub fn compile(file: &DensityFile, seed: u64) -> Result<Self, DensityError> {
        let mut compiler = Compiler {
            file,
            seed,
            graph: DensityGraph {
                nodes: Vec::new(),
                bounds: Vec::new(),
                noises: Vec::new(),
                splines: Vec::new(),
                spline_bounds: Vec::new(),
                names: BTreeMap::new(),
                interpolated: 0,
                flat: 0,
                cache2d: 0,
                once: 0,
            },
            noise_index: HashMap::new(),
            in_progress: Vec::new(),
        };
        for name in file.functions.keys() {
            compiler.function(name)?;
        }
        Ok(compiler.graph)
    }

    /// La fonction nommée `name`.
    pub fn function(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// Les noms des fonctions.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    /// Le nombre de nœuds (après partage des fonctions nommées).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Des bornes de la fonction : toute valeur est dans `[min, max]` (sans être forcément atteinte).
    pub fn bounds(&self, id: NodeId) -> (f64, f64) {
        self.bounds[id as usize]
    }

    /// La valeur exacte en (x, y, z).
    pub fn sample(&self, id: NodeId, x: i32, y: i32, z: i32) -> f64 {
        let mut eval = Eval::new(self, None);
        eval.eval(id, x, y, z)
    }

    /// Les valeurs exactes en plusieurs positions, dans l'ordre : les caches (`FlatCache`, `Cache2d`...)
    /// servent d'une position à l'autre, contrairement à des appels séparés à [`Self::sample`].
    pub fn sample_many(&self, id: NodeId, positions: impl IntoIterator<Item = IVec3>) -> Vec<f64> {
        let mut eval = Eval::new(self, None);
        positions
            .into_iter()
            .map(|p| eval.eval(id, p.x, p.y, p.z))
            .collect()
    }

    /// Remplit `out` (32³ valeurs, dans l'ordre de `Chunk::index` : x + 32 z + 1024 y) avec la
    /// fonction `id` sur le chunk dont le coin minimal est `origin` (en blocs).
    pub fn fill_chunk(&self, id: NodeId, origin: IVec3, out: &mut [f64]) {
        assert_eq!(out.len(), (SIZE * SIZE * SIZE) as usize);
        // 1. Les coins de chaque nœud interpolé (calculés exactement).
        let mut grids = vec![Vec::new(); self.interpolated];
        if self.interpolated > 0 {
            let mut eval = Eval::new(self, None);
            for node in &self.nodes {
                let Node::Interpolated(child, slot) = node else {
                    continue;
                };
                let mut grid = Vec::with_capacity(CORNERS_XZ * CORNERS_XZ * CORNERS_Y);
                for cy in 0..CORNERS_Y as i32 {
                    for cz in 0..CORNERS_XZ as i32 {
                        for cx in 0..CORNERS_XZ as i32 {
                            grid.push(eval.eval(
                                *child,
                                origin.x + cx * CELL_WIDTH,
                                origin.y + cy * CELL_HEIGHT,
                                origin.z + cz * CELL_WIDTH,
                            ));
                        }
                    }
                }
                grids[*slot] = grid;
            }
        }
        // 2. Chaque bloc, les nœuds interpolés lus dans leur grille.
        let mut eval = Eval::new(
            self,
            Some(Grids {
                origin,
                values: &grids,
            }),
        );
        let mut i = 0;
        for ly in 0..SIZE {
            for lz in 0..SIZE {
                for lx in 0..SIZE {
                    out[i] = eval.eval(id, origin.x + lx, origin.y + ly, origin.z + lz);
                    i += 1;
                }
            }
        }
    }
}

impl Compiler<'_> {
    fn push(&mut self, node: Node, bounds: (f64, f64)) -> NodeId {
        self.graph.nodes.push(node);
        self.graph.bounds.push(bounds);
        (self.graph.nodes.len() - 1) as NodeId
    }

    fn function(&mut self, name: &str) -> Result<NodeId, DensityError> {
        if let Some(&id) = self.graph.names.get(name) {
            return Ok(id);
        }
        if self.in_progress.iter().any(|n| n == name) {
            return Err(DensityError::Cycle(name.to_string()));
        }
        let def = self
            .file
            .functions
            .get(name)
            .ok_or_else(|| DensityError::UnknownFunction(name.to_string()))?;
        self.in_progress.push(name.to_string());
        let id = self.node(def)?;
        self.in_progress.pop();
        self.graph.names.insert(name.to_string(), id);
        Ok(id)
    }

    fn noise(&mut self, name: &str) -> Result<usize, DensityError> {
        if let Some(&i) = self.noise_index.get(name) {
            return Ok(i);
        }
        let def = self
            .file
            .noises
            .get(name)
            .ok_or_else(|| DensityError::UnknownNoise(name.to_string()))?;
        self.graph
            .noises
            .push(NormalNoise::new(self.seed, name, def));
        let i = self.graph.noises.len() - 1;
        self.noise_index.insert(name.to_string(), i);
        Ok(i)
    }

    fn bounds(&self, id: NodeId) -> (f64, f64) {
        self.graph.bounds[id as usize]
    }

    fn node(&mut self, def: &DensityDef) -> Result<NodeId, DensityError> {
        Ok(match def {
            DensityDef::Constant(c) => self.push(Node::Constant(*c), (*c, *c)),
            DensityDef::Ref(name) => return self.function(name),
            DensityDef::Noise {
                noise,
                xz_scale,
                y_scale,
            } => {
                let noise = self.noise(noise)?;
                let m = self.graph.noises[noise].max_value();
                self.push(
                    Node::Noise {
                        noise,
                        xz: *xz_scale,
                        y: *y_scale,
                    },
                    (-m, m),
                )
            }
            DensityDef::ShiftedNoise {
                noise,
                xz_scale,
                y_scale,
                shift_x,
                shift_y,
                shift_z,
            } => {
                let noise = self.noise(noise)?;
                let shift = [
                    self.node(shift_x)?,
                    self.node(shift_y)?,
                    self.node(shift_z)?,
                ];
                let m = self.graph.noises[noise].max_value();
                self.push(
                    Node::Shifted {
                        noise,
                        xz: *xz_scale,
                        y: *y_scale,
                        shift,
                    },
                    (-m, m),
                )
            }
            DensityDef::Add(a, b)
            | DensityDef::Mul(a, b)
            | DensityDef::Min(a, b)
            | DensityDef::Max(a, b) => {
                let (a, b) = (self.node(a)?, self.node(b)?);
                let ((alo, ahi), (blo, bhi)) = (self.bounds(a), self.bounds(b));
                let (op, bounds) = match def {
                    DensityDef::Add(..) => (Binary::Add, (alo + blo, ahi + bhi)),
                    DensityDef::Mul(..) => {
                        let products = [alo * blo, alo * bhi, ahi * blo, ahi * bhi];
                        (
                            Binary::Mul,
                            (
                                products.iter().copied().fold(f64::INFINITY, f64::min),
                                products.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                            ),
                        )
                    }
                    DensityDef::Min(..) => (Binary::Min, (alo.min(blo), ahi.min(bhi))),
                    _ => (Binary::Max, (alo.max(blo), ahi.max(bhi))),
                };
                self.push(Node::Binary(op, a, b), bounds)
            }
            DensityDef::Clamp { input, min, max } => {
                let input = self.node(input)?;
                let (lo, hi) = self.bounds(input);
                self.push(
                    Node::Clamp {
                        input,
                        min: *min,
                        max: *max,
                    },
                    (lo.clamp(*min, *max), hi.clamp(*min, *max)),
                )
            }
            DensityDef::Abs(a)
            | DensityDef::Square(a)
            | DensityDef::Cube(a)
            | DensityDef::HalfNegative(a)
            | DensityDef::QuarterNegative(a)
            | DensityDef::Squeeze(a) => {
                let a = self.node(a)?;
                let (lo, hi) = self.bounds(a);
                let op = match def {
                    DensityDef::Abs(_) => Unary::Abs,
                    DensityDef::Square(_) => Unary::Square,
                    DensityDef::Cube(_) => Unary::Cube,
                    DensityDef::HalfNegative(_) => Unary::HalfNegative,
                    DensityDef::QuarterNegative(_) => Unary::QuarterNegative,
                    _ => Unary::Squeeze,
                };
                let bounds = match op {
                    Unary::Abs | Unary::Square => {
                        let (a, b) = (unary(op, lo), unary(op, hi));
                        let low = if lo <= 0.0 && hi >= 0.0 {
                            0.0
                        } else {
                            a.min(b)
                        };
                        (low, a.max(b))
                    }
                    // Les autres sont croissantes.
                    _ => (unary(op, lo), unary(op, hi)),
                };
                self.push(Node::Unary(op, a), bounds)
            }
            DensityDef::YClampedGradient {
                from_y,
                to_y,
                from_value,
                to_value,
            } => self.push(
                Node::YGradient {
                    from_y: f64::from(*from_y),
                    to_y: f64::from(*to_y),
                    from_value: *from_value,
                    to_value: *to_value,
                },
                (from_value.min(*to_value), from_value.max(*to_value)),
            ),
            DensityDef::Spline(spline) => {
                let index = self.spline(spline)?;
                let bounds = self.graph.spline_bounds[index];
                self.push(Node::Spline(index), bounds)
            }
            DensityDef::Interpolated(a)
            | DensityDef::FlatCache(a)
            | DensityDef::Cache2d(a)
            | DensityDef::CacheOnce(a) => {
                let child = self.node(a)?;
                let bounds = self.bounds(child);
                let graph = &mut self.graph;
                let node = match def {
                    DensityDef::Interpolated(_) => {
                        graph.interpolated += 1;
                        Node::Interpolated(child, graph.interpolated - 1)
                    }
                    DensityDef::FlatCache(_) => {
                        graph.flat += 1;
                        Node::FlatCache(child, graph.flat - 1)
                    }
                    DensityDef::Cache2d(_) => {
                        graph.cache2d += 1;
                        Node::Cache2d(child, graph.cache2d - 1)
                    }
                    _ => {
                        graph.once += 1;
                        Node::CacheOnce(child, graph.once - 1)
                    }
                };
                self.push(node, bounds)
            }
            DensityDef::RangeChoice {
                input,
                min_inclusive,
                max_exclusive,
                when_in_range,
                when_out_of_range,
            } => {
                let input = self.node(input)?;
                let inside = self.node(when_in_range)?;
                let outside = self.node(when_out_of_range)?;
                let (ilo, ihi) = self.bounds(input);
                let (a, b) = (self.bounds(inside), self.bounds(outside));
                // Si l'entrée ne peut jamais (ou toujours) tomber dans la plage, une seule branche compte.
                let bounds = if ihi < *min_inclusive || ilo >= *max_exclusive {
                    b
                } else if ilo >= *min_inclusive && ihi < *max_exclusive {
                    a
                } else {
                    (a.0.min(b.0), a.1.max(b.1))
                };
                self.push(
                    Node::RangeChoice {
                        input,
                        min: *min_inclusive,
                        max: *max_exclusive,
                        inside,
                        outside,
                    },
                    bounds,
                )
            }
        })
    }

    fn spline(&mut self, def: &SplineDef) -> Result<usize, DensityError> {
        if def.points.is_empty() {
            return Err(DensityError::BadSpline("aucun point".into()));
        }
        if def
            .points
            .windows(2)
            .any(|w| w[0].location >= w[1].location)
        {
            return Err(DensityError::BadSpline(
                "les abscisses doivent croître strictement".into(),
            ));
        }
        let coordinate = self.node(&def.coordinate)?;
        let mut points = Vec::with_capacity(def.points.len());
        let mut value_bounds = Vec::with_capacity(def.points.len());
        for p in &def.points {
            let (value, bounds) = match &p.value {
                SplineValue::Constant(c) => (CompiledValue::Constant(*c), (*c, *c)),
                SplineValue::Spline(inner) => {
                    let i = self.spline(inner)?;
                    (CompiledValue::Spline(i), self.graph.spline_bounds[i])
                }
            };
            points.push((p.location, value, p.derivative));
            value_bounds.push(bounds);
        }
        // Bornes : toutes les valeurs, plus le dépassement possible de l'hermite (au plus un quart de
        // l'écart dû aux pentes et aux valeurs sur chaque segment), plus les prolongements linéaires.
        let mut lo = value_bounds
            .iter()
            .map(|b| b.0)
            .fold(f64::INFINITY, f64::min);
        let mut hi = value_bounds
            .iter()
            .map(|b| b.1)
            .fold(f64::NEG_INFINITY, f64::max);
        let spread = hi - lo;
        let mut slack: f64 = 0.0;
        for w in points.windows(2) {
            let width = w[1].0 - w[0].0;
            let pull = w[0].2.abs().max(w[1].2.abs()) * width + spread;
            slack = slack.max(0.25 * pull);
        }
        lo -= slack;
        hi += slack;
        let (clo, chi) = self.graph.bounds[coordinate as usize];
        let (first, last) = (&points[0], &points[points.len() - 1]);
        let (first_bounds, last_bounds) = (value_bounds[0], value_bounds[value_bounds.len() - 1]);
        if clo < first.0 {
            let reach = first.2 * (clo - first.0);
            lo = lo.min(first_bounds.0 + reach.min(0.0));
            hi = hi.max(first_bounds.1 + reach.max(0.0));
        }
        if chi > last.0 {
            let reach = last.2 * (chi - last.0);
            lo = lo.min(last_bounds.0 + reach.min(0.0));
            hi = hi.max(last_bounds.1 + reach.max(0.0));
        }
        self.graph
            .splines
            .push(CompiledSpline { coordinate, points });
        self.graph.spline_bounds.push((lo, hi));
        Ok(self.graph.splines.len() - 1)
    }
}

#[inline]
fn unary(op: Unary, v: f64) -> f64 {
    match op {
        Unary::Abs => v.abs(),
        Unary::Square => v * v,
        Unary::Cube => v * v * v,
        Unary::HalfNegative => {
            if v > 0.0 {
                v
            } else {
                v * 0.5
            }
        }
        Unary::QuarterNegative => {
            if v > 0.0 {
                v
            } else {
                v * 0.25
            }
        }
        Unary::Squeeze => {
            let c = v.clamp(-1.0, 1.0);
            c / 2.0 - c * c * c / 24.0
        }
    }
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// Les coins des nœuds interpolés d'un chunk.
struct Grids<'a> {
    origin: IVec3,
    values: &'a [Vec<f64>],
}

impl Grids<'_> {
    #[inline]
    fn lookup(&self, slot: usize, x: i32, y: i32, z: i32) -> f64 {
        let (lx, ly, lz) = (x - self.origin.x, y - self.origin.y, z - self.origin.z);
        let (cx, cy, cz) = (
            (lx / CELL_WIDTH) as usize,
            (ly / CELL_HEIGHT) as usize,
            (lz / CELL_WIDTH) as usize,
        );
        let (tx, ty, tz) = (
            f64::from(lx % CELL_WIDTH) / f64::from(CELL_WIDTH),
            f64::from(ly % CELL_HEIGHT) / f64::from(CELL_HEIGHT),
            f64::from(lz % CELL_WIDTH) / f64::from(CELL_WIDTH),
        );
        let grid = &self.values[slot];
        let at = |x: usize, y: usize, z: usize| grid[x + CORNERS_XZ * (z + CORNERS_XZ * y)];
        let x00 = lerp(tx, at(cx, cy, cz), at(cx + 1, cy, cz));
        let x10 = lerp(tx, at(cx, cy + 1, cz), at(cx + 1, cy + 1, cz));
        let x01 = lerp(tx, at(cx, cy, cz + 1), at(cx + 1, cy, cz + 1));
        let x11 = lerp(tx, at(cx, cy + 1, cz + 1), at(cx + 1, cy + 1, cz + 1));
        lerp(tz, lerp(ty, x00, x10), lerp(ty, x01, x11))
    }

    fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        (0..SIZE).contains(&(x - self.origin.x))
            && (0..SIZE).contains(&(y - self.origin.y))
            && (0..SIZE).contains(&(z - self.origin.z))
    }
}

/// L'état d'une évaluation : les caches, et les grilles d'interpolation d'un chunk.
struct Eval<'a> {
    graph: &'a DensityGraph,
    grids: Option<Grids<'a>>,
    flat: Vec<HashMap<(i32, i32), f64>>,
    last2d: Vec<Option<(i32, i32, f64)>>,
    once: Vec<Option<(i32, i32, i32, f64)>>,
}

impl<'a> Eval<'a> {
    fn new(graph: &'a DensityGraph, grids: Option<Grids<'a>>) -> Self {
        Self {
            graph,
            grids,
            flat: vec![HashMap::new(); graph.flat],
            last2d: vec![None; graph.cache2d],
            once: vec![None; graph.once],
        }
    }

    fn eval(&mut self, id: NodeId, x: i32, y: i32, z: i32) -> f64 {
        let graph = self.graph;
        match &graph.nodes[id as usize] {
            Node::Constant(c) => *c,
            Node::Noise { noise, xz, y: ys } => {
                graph.noises[*noise].sample(f64::from(x) * xz, f64::from(y) * ys, f64::from(z) * xz)
            }
            Node::Shifted {
                noise,
                xz,
                y: ys,
                shift,
            } => {
                let sx = self.eval(shift[0], x, y, z);
                let sy = self.eval(shift[1], x, y, z);
                let sz = self.eval(shift[2], x, y, z);
                graph.noises[*noise].sample(
                    f64::from(x) * xz + sx,
                    f64::from(y) * ys + sy,
                    f64::from(z) * xz + sz,
                )
            }
            Node::Binary(op, a, b) => {
                let a = self.eval(*a, x, y, z);
                match op {
                    // Un produit par zéro n'a pas besoin de l'autre côté.
                    Binary::Mul if a == 0.0 => 0.0,
                    Binary::Mul => a * self.eval(*b, x, y, z),
                    Binary::Add => a + self.eval(*b, x, y, z),
                    Binary::Min => a.min(self.eval(*b, x, y, z)),
                    Binary::Max => a.max(self.eval(*b, x, y, z)),
                }
            }
            Node::Unary(op, a) => unary(*op, self.eval(*a, x, y, z)),
            Node::Clamp { input, min, max } => self.eval(*input, x, y, z).clamp(*min, *max),
            Node::YGradient {
                from_y,
                to_y,
                from_value,
                to_value,
            } => {
                let t = ((f64::from(y) - from_y) / (to_y - from_y)).clamp(0.0, 1.0);
                lerp(t, *from_value, *to_value)
            }
            Node::Spline(s) => self.spline(*s, x, y, z),
            Node::Interpolated(child, slot) => match &self.grids {
                Some(grids) if grids.contains(x, y, z) => grids.lookup(*slot, x, y, z),
                _ => self.eval(*child, x, y, z),
            },
            Node::FlatCache(child, slot) => {
                let key = (x.div_euclid(4) * 4, z.div_euclid(4) * 4);
                if let Some(&v) = self.flat[*slot].get(&key) {
                    return v;
                }
                let v = self.eval(*child, key.0, 0, key.1);
                self.flat[*slot].insert(key, v);
                v
            }
            Node::Cache2d(child, slot) => {
                if let Some((cx, cz, v)) = self.last2d[*slot]
                    && cx == x
                    && cz == z
                {
                    return v;
                }
                let v = self.eval(*child, x, y, z);
                self.last2d[*slot] = Some((x, z, v));
                v
            }
            Node::CacheOnce(child, slot) => {
                if let Some((cx, cy, cz, v)) = self.once[*slot]
                    && (cx, cy, cz) == (x, y, z)
                {
                    return v;
                }
                let v = self.eval(*child, x, y, z);
                self.once[*slot] = Some((x, y, z, v));
                v
            }
            Node::RangeChoice {
                input,
                min,
                max,
                inside,
                outside,
            } => {
                let v = self.eval(*input, x, y, z);
                if v >= *min && v < *max {
                    self.eval(*inside, x, y, z)
                } else {
                    self.eval(*outside, x, y, z)
                }
            }
        }
    }

    fn value(&mut self, value: &CompiledValue, x: i32, y: i32, z: i32) -> f64 {
        match value {
            CompiledValue::Constant(c) => *c,
            CompiledValue::Spline(s) => self.spline(*s, x, y, z),
        }
    }

    /// La spline cubique d'hermite par morceaux, prolongée en ligne droite hors de ses points.
    fn spline(&mut self, index: usize, x: i32, y: i32, z: i32) -> f64 {
        let graph = self.graph;
        let spline = &graph.splines[index];
        let c = self.eval(spline.coordinate, x, y, z);
        let points = &spline.points;
        let after = points.partition_point(|p| p.0 <= c);
        if after == 0 {
            let (loc, value, slope) = &points[0];
            return self.value(value, x, y, z) + slope * (c - loc);
        }
        if after == points.len() {
            let (loc, value, slope) = &points[points.len() - 1];
            return self.value(value, x, y, z) + slope * (c - loc);
        }
        let (x0, v0, d0) = &points[after - 1];
        let (x1, v1, d1) = &points[after];
        let (y0, y1) = (self.value(v0, x, y, z), self.value(v1, x, y, z));
        let width = x1 - x0;
        let t = (c - x0) / width;
        let f = d0 * width - (y1 - y0);
        let g = -d1 * width + (y1 - y0);
        lerp(t, y0, y1) + t * (1.0 - t) * lerp(t, f, g)
    }
}
