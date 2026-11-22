//! Les donjons : des salles reliées en **graphe**, enterrées, avec un escalier jusqu'à la
//! surface, des couloirs, une salle du trésor et une salle du boss.
//!
//! # La disposition
//!
//! Elle ne dépend que de la graine du site (comme toutes les structures : chaque chunk la
//! recalcule et n'écrit que sa part). On pose l'entrée sur une grille, puis on fait pousser un
//! **arbre** : chaque nouvelle salle s'accroche à une salle existante, sur une case voisine libre.
//! Puis les rôles sont distribués d'après la distance à l'entrée :
//! - la salle la plus profonde est celle du **boss** ;
//! - jusqu'à deux autres bouts de branche sont des salles du **trésor** ;
//! - les autres sont des salles ordinaires, tirées parmi celles que le fichier décrit (`weight`).
//!
//! # Le butin à paliers
//!
//! La profondeur d'une salle (sa distance à l'entrée dans le graphe) donne son **palier** : plus on
//! s'enfonce, plus la table de butin de ses coffres est riche. Dans un plan, un coffre nomme sa
//! table de trois façons : `"@tier"` (le palier de la salle), `"@treasure"` (un palier au-dessus) ou
//! `"@boss"` (le dernier palier), ou une table précise.
//!
//! # La construction
//!
//! Dans l'ordre : les coques des couloirs, les salles (qui écrasent les coques), les intérieurs des
//! couloirs (qui percent les portes dans les murs des salles), la lumière, l'escalier.

use anyhow::{Context as _, Result, bail};
use glam::IVec3;
use serde::Deserialize;

use super::{Builder, StructureSet, Template, qualify_block, scoped};
use crate::block::BlockId;
use crate::registry::BlockRegistry;
use std::collections::HashMap;

/// Le rôle d'une salle dans le donjon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Role {
    /// Là où l'on arrive : l'escalier y descend. Une seule.
    Entry,
    /// Une salle ordinaire.
    Room,
    /// Un cul-de-sac avec du butin.
    Treasure,
    /// La salle la plus profonde.
    Boss,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomEntry {
    template: String,
    role: Role,
    #[serde(default = "one")]
    weight: u32,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DungeonEntry {
    rooms: Vec<RoomEntry>,
    /// Le nombre de salles (min, max), entrée et boss compris.
    count: (u32, u32),
    /// La distance entre les centres de deux salles voisines, en blocs.
    cell: i32,
    /// De combien le plancher est sous la surface, en blocs.
    depth: i32,
    /// Les murs des couloirs, leur sol et leur toit.
    wall: String,
    /// Un bloc lumineux encastré dans le toit des couloirs (facultatif).
    #[serde(default)]
    light: Option<String>,
    /// Un escalier en demi-marches (facultatif) : un bloc à `Facing4` (la partie haute au nord).
    /// Sans lui, l'escalier est fait de marches pleines qu'on franchit en sautant.
    #[serde(default)]
    stairs: Option<String>,
    /// Les tables de butin par palier, du plus pauvre au plus riche.
    tiers: Vec<String>,
}

/// Un donjon résolu.
#[derive(Debug, Clone)]
pub struct GraphDef {
    rooms: Vec<(Role, Template, u32)>,
    count: (u32, u32),
    cell: i32,
    depth: i32,
    wall: BlockId,
    light: Option<BlockId>,
    /// Les quatre orientations de l'escalier : la partie haute au nord, à l'est, au sud, à l'ouest.
    stairs: Option<[BlockId; 4]>,
    tiers: Vec<String>,
}

impl GraphDef {
    /// Jusqu'où le donjon s'étend depuis son point (en blocs) : le plus long chemin, plus
    /// l'escalier.
    pub fn radius(&self) -> i32 {
        MAX_GRID * self.cell * 2 + self.depth + 16
    }

    /// Le plancher doit être au moins à cette hauteur : la surface doit être assez haute.
    pub fn depth(&self) -> i32 {
        self.depth
    }

    /// Les noms des tables de butin qu'il utilise (pour vérifier qu'elles existent).
    pub fn tiers(&self) -> &[String] {
        &self.tiers
    }
}

/// Les salles s'étendent au plus à cette distance (en cases de la grille) de l'entrée.
const MAX_GRID: i32 = 3;
/// Les couloirs : 3 de large, 3 de haut à l'intérieur, une coque de 1 autour.
const HALF_WIDTH: i32 = 1;

/// Lit les donjons d'un fichier. `templates` : les plans du fichier, par nom qualifié.
pub(super) fn resolve(
    namespace: &str,
    entries: &HashMap<String, DungeonEntry>,
    templates: &HashMap<String, Template>,
    blocks: &BlockRegistry,
) -> Result<HashMap<String, GraphDef>> {
    let mut out = HashMap::new();
    for (name, e) in entries {
        let def = resolve_one(namespace, e, templates, blocks)
            .with_context(|| format!("donjon « {name} »"))?;
        out.insert(scoped(name, namespace), def);
    }
    Ok(out)
}

fn resolve_one(
    namespace: &str,
    e: &DungeonEntry,
    templates: &HashMap<String, Template>,
    blocks: &BlockRegistry,
) -> Result<GraphDef> {
    let count_of = |role: Role| e.rooms.iter().filter(|r| r.role == role).count();
    if count_of(Role::Entry) != 1 || count_of(Role::Boss) != 1 {
        bail!("il faut une salle d'entrée et une salle de boss, une seule de chaque");
    }
    if count_of(Role::Room) == 0 {
        bail!("il faut au moins une salle ordinaire");
    }
    if e.rooms.iter().any(|r| r.weight == 0) {
        bail!("le poids d'une salle va d'au moins 1");
    }
    if e.count.0 < 3 || e.count.0 > e.count.1 || e.count.1 > 9 {
        bail!("le nombre de salles va de 3 à 9 (min <= max)");
    }
    if e.depth < 8 || e.depth > 120 {
        bail!("la profondeur va de 8 à 120 blocs");
    }
    if e.tiers.is_empty() {
        bail!("il faut au moins un palier de butin");
    }
    let mut rooms = Vec::new();
    for r in &e.rooms {
        let key = scoped(&r.template, namespace);
        let template = templates
            .get(&key)
            .cloned()
            .with_context(|| format!("plan « {key} » inconnu (dans ce fichier)"))?;
        rooms.push((r.role, template, r.weight));
    }
    let widest = rooms.iter().map(|(_, t, _)| t.size()).max().unwrap_or(0);
    if e.cell < widest + 6 {
        bail!(
            "`cell` doit être d'au moins {} (la plus grande salle + 6)",
            widest + 6
        );
    }
    let block = |key: &str| {
        let key = qualify_block(key, namespace);
        blocks
            .id(&key)
            .with_context(|| format!("bloc inconnu « {key} »"))
    };
    Ok(GraphDef {
        rooms,
        count: e.count,
        cell: e.cell,
        depth: e.depth,
        wall: block(&e.wall)?,
        light: e.light.as_deref().map(block).transpose()?,
        stairs: e
            .stairs
            .as_deref()
            .map(|key| -> Result<[BlockId; 4]> {
                let key = qualify_block(key, namespace);
                let variant = |facing: &str| {
                    let full = format!("{key}[facing={facing}]");
                    blocks.id(&full).with_context(|| {
                        format!("`stairs` : « {full} » n'existe pas (il faut un état Facing4)")
                    })
                };
                Ok([
                    variant("north")?,
                    variant("east")?,
                    variant("south")?,
                    variant("west")?,
                ])
            })
            .transpose()?,
        tiers: e.tiers.iter().map(|t| scoped(t, namespace)).collect(),
    })
}

/// Une salle posée : sa case de la grille, son rôle, son plan, son parent.
#[derive(Debug, Clone, Copy)]
struct Node {
    grid: (i32, i32),
    parent: Option<usize>,
    role: Role,
    room: usize,
    depth: i32,
}

/// La disposition d'un donjon : ses salles, dans l'ordre (l'entrée en premier).
#[derive(Debug, Clone)]
pub struct Layout {
    nodes: Vec<Node>,
}

impl Layout {
    /// Le nombre de salles.
    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// (case, rôle, profondeur) de chaque salle.
    #[cfg(test)]
    pub fn rooms(&self) -> Vec<((i32, i32), Role, i32)> {
        self.nodes
            .iter()
            .map(|n| (n.grid, n.role, n.depth))
            .collect()
    }

    /// Les liaisons (parent, enfant) du graphe.
    pub fn edges(&self) -> Vec<(usize, usize)> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| Some((n.parent?, i)))
            .collect()
    }
}

const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Fait pousser le graphe d'un donjon à partir de sa graine.
pub fn layout(def: &GraphDef, rng: &mut crate::worldgen::trees::Rng64) -> Layout {
    let n = (def.count.0 + rng.below(def.count.1 - def.count.0 + 1)) as usize;
    let entry_room = def
        .rooms
        .iter()
        .position(|(r, _, _)| *r == Role::Entry)
        .unwrap_or(0);
    let mut nodes = vec![Node {
        grid: (0, 0),
        parent: None,
        role: Role::Entry,
        room: entry_room,
        depth: 0,
    }];
    let mut attempts = 0;
    while nodes.len() < n && attempts < 400 {
        attempts += 1;
        let parent = rng.below(nodes.len() as u32) as usize;
        let (dx, dz) = DIRECTIONS[rng.below(4) as usize];
        let grid = (nodes[parent].grid.0 + dx, nodes[parent].grid.1 + dz);
        if grid.0.abs() > MAX_GRID
            || grid.1.abs() > MAX_GRID
            || nodes.iter().any(|m| m.grid == grid)
        {
            continue;
        }
        nodes.push(Node {
            grid,
            parent: Some(parent),
            role: Role::Room,
            room: 0,
            depth: nodes[parent].depth + 1,
        });
    }
    // Le boss : la salle la plus profonde (la première en cas d'égalité).
    let boss = (1..nodes.len())
        .max_by_key(|&i| (nodes[i].depth, std::cmp::Reverse(i)))
        .unwrap_or(0);
    let boss_room = def
        .rooms
        .iter()
        .position(|(r, _, _)| *r == Role::Boss)
        .unwrap_or(0);
    if boss != 0 {
        nodes[boss].role = Role::Boss;
        nodes[boss].room = boss_room;
    }
    // Les trésors : jusqu'à deux bouts de branche (sans enfant), ni l'entrée ni le boss.
    let has_child = |i: usize, nodes: &[Node]| nodes.iter().any(|m| m.parent == Some(i));
    let mut leaves: Vec<usize> = (1..nodes.len())
        .filter(|&i| i != boss && !has_child(i, &nodes))
        .collect();
    let treasure_rooms: Vec<usize> = def
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, (r, _, _))| *r == Role::Treasure)
        .map(|(i, _)| i)
        .collect();
    for _ in 0..2 {
        if leaves.is_empty() || treasure_rooms.is_empty() {
            break;
        }
        let leaf = leaves.remove(rng.below(leaves.len() as u32) as usize);
        nodes[leaf].role = Role::Treasure;
        nodes[leaf].room = treasure_rooms[rng.below(treasure_rooms.len() as u32) as usize];
    }
    // Les autres : des salles ordinaires, au hasard pondéré.
    let ordinary: Vec<(usize, u32)> = def
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, (r, _, _))| *r == Role::Room)
        .map(|(i, (_, _, w))| (i, *w))
        .collect();
    let total: u32 = ordinary.iter().map(|(_, w)| w).sum();
    for node in nodes.iter_mut().filter(|n| n.role == Role::Room) {
        let mut pick = rng.below(total.max(1));
        node.room = ordinary
            .iter()
            .find(|(_, w)| {
                if pick < *w {
                    true
                } else {
                    pick -= w;
                    false
                }
            })
            .map_or(ordinary[0].0, |(i, _)| *i);
    }
    Layout { nodes }
}

impl Layout {
    /// Le palier d'une salle (indice dans `tiers`) : de plus en plus riche avec la profondeur.
    fn tier(&self, i: usize, def: &GraphDef) -> usize {
        let last = def.tiers.len() - 1;
        let deepest = self.nodes.iter().map(|n| n.depth).max().unwrap_or(1).max(1);
        match self.nodes[i].role {
            Role::Boss => last,
            _ => ((self.nodes[i].depth * (last as i32 + 1) / (deepest + 1)) as usize).min(last),
        }
    }
}

/// Le plancher d'un donjon : `depth` blocs sous le sol **le plus bas** de toute son emprise
/// (salles et couloirs), pas seulement sous l'entrée. Sinon, un donjon posé sur une colline
/// dont les salles s'étendent au-dessus d'une vallée ou d'une rivière en sort à l'air libre.
pub(super) fn floor_y(
    def: &GraphDef,
    layout: &Layout,
    sx: i32,
    sz: i32,
    mut height: impl FnMut(i32, i32) -> i32,
) -> i32 {
    let center = |n: &Node| (sx + n.grid.0 * def.cell, sz + n.grid.1 * def.cell);
    let mut lowest = height(sx, sz);
    for node in &layout.nodes {
        let (cx, cz) = center(node);
        let t = &def.rooms[node.room].1;
        // Le centre et les quatre coins de la salle (avec un bloc de marge).
        let (hw, hd) = (t.w / 2 + 1, t.d / 2 + 1);
        for (dx, dz) in [(0, 0), (-hw, -hd), (hw, -hd), (-hw, hd), (hw, hd)] {
            lowest = lowest.min(height(cx + dx, cz + dz));
        }
    }
    for (parent, child) in layout.edges() {
        let (a, b) = (center(&layout.nodes[parent]), center(&layout.nodes[child]));
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        let (dx, dz) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
        for s in (0..=steps).step_by(4) {
            lowest = lowest.min(height(a.0 + dx * s, a.1 + dz * s));
        }
    }
    (lowest - def.depth).max(12)
}

impl Builder<'_> {
    /// Construit un donjon centré (en x, z) sur le point du site.
    pub(super) fn graph(&mut self, sx: i32, sz: i32, def: &GraphDef, set: &StructureSet) {
        let layout = layout(def, &mut self.rng);
        let y0 = floor_y(def, &layout, sx, sz, |x, z| self.height(x, z));
        let center = |n: &Node| (sx + n.grid.0 * def.cell, sz + n.grid.1 * def.cell);
        // En v3, le plancher descend jusqu'à de la roche pleine sur toute l'emprise (salles et
        // couloirs) : les grandes salles des grottes ne doivent pas le traverser.
        let y0 = {
            let mut points = Vec::new();
            for node in &layout.nodes {
                let (cx, cz) = center(node);
                let t = &def.rooms[node.room].1;
                let (hw, hd) = (t.w / 2 + 1, t.d / 2 + 1);
                for (dx, dz) in [(0, 0), (-hw, -hd), (hw, -hd), (-hw, hd), (hw, hd)] {
                    points.push((cx + dx, cz + dz));
                }
            }
            for (parent, child) in layout.edges() {
                let (a, b) = (center(&layout.nodes[parent]), center(&layout.nodes[child]));
                let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs());
                let (dx, dz) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
                for s in (0..=steps).step_by(4) {
                    points.push((a.0 + dx * s, a.1 + dz * s));
                }
            }
            let height = def.rooms.iter().map(|(_, t, _)| t.h).max().unwrap_or(6);
            self.world.buried_floor((sx, sz), &points, y0, height)
        };
        // En v3, une gaine de roche autour des salles et des couloirs : une grotte qui traverse le donjon
        // est comblée autour de lui (il ne flotte jamais dans le vide). Tout le reste l'écrase ensuite.
        if self.world.encases_buried_structures() {
            let stone = self.b.get("core:stone");
            for node in &layout.nodes {
                let (cx, cz) = center(node);
                let t = &def.rooms[node.room].1;
                let (hw, hd) = (t.w / 2 + 3, t.d / 2 + 3);
                let top = (y0 + t.h + 2).min(self.height(cx, cz) - 4);
                self.fill(
                    IVec3::new(cx - hw, y0 - 3, cz - hd),
                    IVec3::new(cx + hw, top, cz + hd),
                    stone,
                );
            }
            for (parent, child) in layout.edges() {
                let (a, b) = (center(&layout.nodes[parent]), center(&layout.nodes[child]));
                let r = HALF_WIDTH + 3;
                let top = (y0 + 7).min(self.height(a.0, a.1).min(self.height(b.0, b.1)) - 4);
                self.fill(
                    IVec3::new(a.0.min(b.0) - r, y0 - 3, a.1.min(b.1) - r),
                    IVec3::new(a.0.max(b.0) + r, top, a.1.max(b.1) + r),
                    stone,
                );
            }
        }
        // La demi-longueur d'une salle sur un axe (pas de rotation : le plan est tel qu'écrit).
        let half = |n: &Node, axis_x: bool| {
            let t = &def.rooms[n.room].1;
            (if axis_x { t.w } else { t.d }) / 2
        };

        // 1. Les coques des couloirs.
        for (parent, child) in layout.edges() {
            let (a, b) = (layout.nodes[parent], layout.nodes[child]);
            self.corridor(center(&a), center(&b), y0, def, &|_, _| true, true);
        }
        // 2. Les salles : elles écrasent les coques, et leurs coffres choisissent leur palier.
        for (i, node) in layout.nodes.iter().enumerate() {
            let (cx, cz) = center(node);
            let t = &def.rooms[node.room].1;
            let tier = layout.tier(i, def);
            let pick = |name: &str| -> Option<String> {
                match name {
                    "@tier" => Some(def.tiers[tier].clone()),
                    "@treasure" => Some(def.tiers[(tier + 1).min(def.tiers.len() - 1)].clone()),
                    "@boss" => def.tiers.last().cloned(),
                    other => Some(other.to_string()),
                }
            };
            self.template_at(cx, cz, y0, 0, t, set, &pick);
        }
        // 3. Les intérieurs des couloirs, de la porte d'une salle à la porte de l'autre.
        for (parent, child) in layout.edges() {
            let (a, b) = (layout.nodes[parent], layout.nodes[child]);
            let along_x = a.grid.1 == b.grid.1;
            let (ha, hb) = (half(&a, along_x), half(&b, along_x));
            let (ca, cb) = (center(&a), center(&b));
            let dir = if along_x {
                (cb.0 - ca.0).signum()
            } else {
                (cb.1 - ca.1).signum()
            };
            // Ni plus loin que le mur de la salle voisine : on perce les murs, pas l'intérieur.
            let reach = |c: (i32, i32), h: i32, sign: i32| {
                if along_x {
                    (c.0 + sign * (h - 1), c.1)
                } else {
                    (c.0, c.1 + sign * (h - 1))
                }
            };
            let from = reach(ca, ha, dir);
            let to = reach(cb, hb, -dir);
            self.corridor(from, to, y0, def, &|_, _| true, false);
        }
        // 4. La lumière : un bloc lumineux dans le toit, tous les 6 blocs.
        if let Some(light) = def.light {
            for (parent, child) in layout.edges() {
                let (a, b) = (center(&layout.nodes[parent]), center(&layout.nodes[child]));
                let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs());
                let (dx, dz) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
                for s in (3..steps - 2).step_by(6) {
                    self.put(IVec3::new(a.0 + dx * s, y0 + 4, a.1 + dz * s), light);
                }
            }
        }
        // 5. L'escalier jusqu'à la surface, depuis l'entrée, du côté où il n'y a pas de salle.
        let entry = layout.nodes[0];
        let (ex, ez) = center(&entry);
        let free = DIRECTIONS
            .iter()
            .find(|d| !layout.nodes.iter().any(|n| n.grid == (d.0, d.1)))
            .copied()
            .unwrap_or((1, 0));
        let reach = if free.0 != 0 {
            half(&entry, true)
        } else {
            half(&entry, false)
        };
        self.stairs((ex, ez), free, reach, y0, def);
    }

    /// Un couloir droit entre deux points (en x, z) : `shell` pose sa coque (5 × 5), sinon on
    /// perce son intérieur (3 × 3) dans ce qui existe déjà.
    #[allow(clippy::too_many_arguments)]
    fn corridor(
        &mut self,
        from: (i32, i32),
        to: (i32, i32),
        y0: i32,
        def: &GraphDef,
        _keep: &dyn Fn(i32, i32) -> bool,
        shell: bool,
    ) {
        let (dx, dz) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
        let steps = (to.0 - from.0).abs().max((to.1 - from.1).abs());
        // Le côté du couloir : perpendiculaire à sa direction.
        let (sx, sz) = (dz.abs(), dx.abs());
        let reach = if shell { HALF_WIDTH + 1 } else { HALF_WIDTH };
        for s in 0..=steps {
            let (x, z) = (from.0 + dx * s, from.1 + dz * s);
            for side in -reach..=reach {
                let (px, pz) = (x + sx * side, z + sz * side);
                if shell {
                    for y in y0..=y0 + 4 {
                        self.put(IVec3::new(px, y, pz), def.wall);
                    }
                } else if side.abs() <= HALF_WIDTH {
                    for y in y0 + 1..=y0 + 3 {
                        self.air(IVec3::new(px, y, pz));
                    }
                }
            }
        }
    }

    /// L'escalier : il monte d'un bloc par bloc depuis l'entrée jusqu'à émerger de la terre, avec
    /// un petit tour de pierre à la sortie. Avec `stairs`, chaque case porte un bloc d'escalier
    /// (deux demi-marches, on le monte en marchant) ; sans, ce sont des marches pleines qu'on
    /// franchit en sautant.
    fn stairs(&mut self, from: (i32, i32), dir: (i32, i32), reach: i32, y0: i32, def: &GraphDef) {
        let (sx, sz) = (dir.1.abs(), dir.0.abs());
        let start = reach; // juste après le mur de la salle
        // La partie haute de l'escalier est du côté où l'on monte.
        let facing = match dir {
            (0, -1) => 0,
            (1, 0) => 1,
            (0, 1) => 2,
            _ => 3,
        };
        let mut step = 0;
        loop {
            let (x, z) = (
                from.0 + dir.0 * (start + step),
                from.1 + dir.1 * (start + step),
            );
            let floor = y0 + step;
            let ground = self.height(x, z);
            // Sorti de terre : la dernière case reste une marche pleine, à fleur de sol.
            // (Le plancher peut être bien plus bas que le sol de l'entrée : il suit le point le
            // plus bas de l'emprise. La limite ne sert qu'à ne jamais boucler sans fin.)
            let last = floor >= ground || step > 200;
            let steps = def.stairs.filter(|_| !last).map(|s| s[facing]);
            // Avec des demi-marches, le sol de la case est un bloc plus haut : le bloc d'escalier.
            let rise = i32::from(steps.is_some());
            for side in -HALF_WIDTH - 1..=HALF_WIDTH + 1 {
                let (px, pz) = (x + sx * side, z + sz * side);
                let inside = side.abs() <= HALF_WIDTH;
                // La coque d'abord (sol et parois), puis l'intérieur dégagé.
                self.put(IVec3::new(px, floor, pz), def.wall);
                if !inside {
                    for y in floor + 1..=floor + 4 + rise {
                        self.put(IVec3::new(px, y, pz), def.wall);
                    }
                }
            }
            for side in -HALF_WIDTH..=HALF_WIDTH {
                let (px, pz) = (x + sx * side, z + sz * side);
                for y in floor + 1..=floor + 4 + rise {
                    self.air(IVec3::new(px, y, pz));
                }
                if let Some(block) = steps {
                    self.put(IVec3::new(px, floor + 1, pz), block);
                }
            }
            if last {
                break;
            }
            step += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::generator::TerrainGenerator;
    use crate::worldgen::generator::structures::{Blueprint, StructureKind};
    use crate::worldgen::trees::Rng64;

    /// Un plan de salle de 7 × 7 (sol, murs, plafond) avec un coffre au centre.
    fn room(chest: &str) -> String {
        let floor = r#"["=======","=======","=======","=======","=======","=======","======="]"#;
        let walls = r#"["=======","=.....=","=.....=","=.....=","=.....=","=.....=","======="]"#;
        format!(
            r#"(palette: {{'=': "core:cobblestone"}}, layers: [{floor}, {walls}, {walls}, {floor}], chests: [((3, 1, 3), "{chest}")], rotate: false)"#
        )
    }

    fn source(extra: &str) -> String {
        format!(
            r#"(
            structures: [(id: "test:crypt", kind: Graph, dungeon: "crypt", spacing: 12, chance: 1.0)],
            templates: {{
                "entry": {entry}, "hall": {hall}, "vault": {vault}, "arena": {arena},
            }},
            dungeons: {{"crypt": (
                rooms: [(template: "entry", role: Entry), (template: "hall", role: Room, weight: 3),
                        (template: "vault", role: Treasure), (template: "arena", role: Boss)],
                count: (5, 8), cell: 16, depth: 14, wall: "core:cobblestone", light: "core:glowstone",
                tiers: ["low", "mid", "high"]{extra},
            )}},
            loot: {{
                "low": (rolls: (1, 1), entries: [(item: "core:coal", count: (1, 1), weight: 1)]),
                "mid": (rolls: (1, 1), entries: [(item: "core:iron_ingot", count: (1, 1), weight: 1)]),
                "high": (rolls: (1, 1), entries: [(item: "core:gold_ingot", count: (1, 1), weight: 1)]),
            }},
        )"#,
            entry = room("@tier"),
            hall = room("@tier"),
            vault = room("@treasure"),
            arena = room("@boss"),
        )
    }

    fn set_with(text: &str) -> Result<(StructureSet, TerrainGenerator)> {
        let generator = TerrainGenerator::new(42);
        let mut set = StructureSet::builtin(generator.biomes())?;
        set.add_source(
            "test",
            text,
            "test",
            generator.biomes(),
            Some(generator.registry()),
        )?;
        Ok((set, generator))
    }

    fn crypt(set: &StructureSet) -> GraphDef {
        set.defs()
            .iter()
            .find(|d| d.key == "test:crypt")
            .and_then(|d| d.graph.clone())
            .expect("le donjon")
    }

    #[test]
    fn layouts_are_deterministic_trees_with_an_entry_a_deepest_boss_and_dead_end_treasures() {
        let (set, _) = set_with(&source("")).unwrap();
        let def = crypt(&set);
        for seed in 0..60u64 {
            let a = layout(&def, &mut Rng64::new(seed));
            let b = layout(&def, &mut Rng64::new(seed));
            assert_eq!(a.rooms(), b.rooms(), "même graine, même donjon");
            assert!((3..=8).contains(&a.len()), "{} salles", a.len());
            let rooms = a.rooms();
            // Un arbre : n - 1 liaisons, des cases distinctes, des voisines sur la grille.
            assert_eq!(a.edges().len(), a.len() - 1);
            for (p, c) in a.edges() {
                let (gp, gc) = (rooms[p].0, rooms[c].0);
                assert_eq!((gp.0 - gc.0).abs() + (gp.1 - gc.1).abs(), 1);
            }
            let mut cells: Vec<_> = rooms.iter().map(|r| r.0).collect();
            cells.sort();
            cells.dedup();
            assert_eq!(cells.len(), a.len());
            // L'entrée, puis exactement un boss, et c'est la salle la plus profonde.
            assert_eq!(rooms[0].1, Role::Entry);
            let bosses: Vec<_> = rooms.iter().filter(|r| r.1 == Role::Boss).collect();
            assert_eq!(bosses.len(), 1);
            assert_eq!(bosses[0].2, rooms.iter().map(|r| r.2).max().unwrap());
            // Au plus deux trésors, au bout d'une branche (aucun enfant).
            let treasures: Vec<usize> = (0..a.len())
                .filter(|&i| rooms[i].1 == Role::Treasure)
                .collect();
            assert!(treasures.len() <= 2);
            assert!(
                treasures
                    .iter()
                    .all(|&i| a.edges().iter().all(|&(p, _)| p != i))
            );
        }
        // Des graines différentes donnent des donjons différents.
        let shapes: std::collections::HashSet<_> = (0..40u64)
            .map(|s| layout(&def, &mut Rng64::new(s)).rooms())
            .collect();
        assert!(shapes.len() > 10, "{} formes", shapes.len());
    }

    /// Le plan complet d'un donjon posé sur une colonne assez haute.
    fn build(
        set: &StructureSet,
        generator: &TerrainGenerator,
        seed: u64,
    ) -> (Blueprint, (i32, i32)) {
        let (x, z) = generator
            .find_column(0, 0, 16, 4000, |c| c.height > 60)
            .expect("une colonne haute");
        let def = set
            .defs()
            .iter()
            .position(|d| d.key == "test:crypt")
            .unwrap();
        let site = crate::worldgen::generator::structures::StructureSite {
            def,
            kind: StructureKind::Graph,
            x,
            z,
            seed,
        };
        (
            generator.blueprint_with(set, &site).expect("un plan"),
            (x, z),
        )
    }

    #[test]
    fn a_dungeon_has_chests_by_tier_a_boss_vault_and_stairs_to_the_surface() {
        let (set, generator) = set_with(&source("")).unwrap();
        let def = crypt(&set);
        let (plan, (x, z)) = build(&set, &generator, 7);
        let lay = layout(&def, &mut Rng64::new(7));
        // Un coffre par salle.
        assert_eq!(plan.chests().len(), lay.len());
        // Le coffre du boss vient du dernier palier (de l'or) ; un trésor, d'un palier au-dessus de
        // sa salle ; une salle d'entrée, du premier (du charbon).
        let items = |chest: &(IVec3, crate::tech::capabilities::Inventory)| -> Vec<String> {
            chest
                .1
                .slots
                .iter()
                .flatten()
                .map(|s| s.item.clone())
                .collect()
        };
        let rooms = lay.rooms();
        let at = |i: usize| {
            let (gx, gz) = rooms[i].0;
            (x + gx * def.cell, z + gz * def.cell)
        };
        let chest_in = |i: usize| {
            let (cx, cz) = at(i);
            plan.chests()
                .iter()
                .find(|(p, _)| (p.x - cx).abs() <= 3 && (p.z - cz).abs() <= 3)
                .unwrap_or_else(|| panic!("pas de coffre dans la salle {i}"))
        };
        let boss = rooms.iter().position(|r| r.1 == Role::Boss).unwrap();
        assert!(items(chest_in(boss)).iter().all(|i| i == "core:gold_ingot"));
        assert!(items(chest_in(0)).iter().all(|i| i == "core:coal"));
        for (i, r) in rooms
            .iter()
            .enumerate()
            .filter(|(_, r)| r.1 == Role::Treasure)
        {
            // Un trésor est un palier au-dessus de ce que sa profondeur donnerait.
            let deepest = rooms.iter().map(|r| r.2).max().unwrap();
            let tier = ((r.2 * 3 / (deepest + 1)) as usize + 1).min(2);
            let expected = ["core:coal", "core:iron_ingot", "core:gold_ingot"][tier];
            assert!(
                items(chest_in(i)).iter().all(|it| it == expected),
                "trésor {i}"
            );
        }
        // Le donjon est enterré : tout son plancher est sous la surface, et l'escalier en sort
        // (de l'air jusqu'au sol, à une colonne où la surface est atteinte).
        let floor_y = floor_y(&def, &lay, x, z, |px, pz| generator.column(px, pz).height);
        let lowest = plan.blocks().iter().map(|(p, _)| p.y).min().unwrap();
        assert!(
            lowest >= floor_y - 1,
            "rien sous le plancher ({lowest} < {floor_y})"
        );
        let air = generator.registry().id("core:air").unwrap();
        let emerged = plan.blocks().iter().any(|&(p, b)| {
            b == air && p.y > floor_y + 2 && p.y >= generator.column(p.x, p.z).height
        });
        assert!(emerged, "l'escalier atteint la surface");
    }

    #[test]
    fn corridors_open_a_door_in_each_room_wall_and_rooms_stay_closed_otherwise() {
        let (set, generator) = set_with(&source("")).unwrap();
        let def = crypt(&set);
        let (plan, (x, z)) = build(&set, &generator, 11);
        let lay = layout(&def, &mut Rng64::new(11));
        let air = generator.registry().id("core:air").unwrap();
        let floor_y = floor_y(&def, &lay, x, z, |px, pz| generator.column(px, pz).height);
        // Le dernier ordre posé à une case l'emporte : on rejoue le plan.
        let mut cells = std::collections::HashMap::new();
        for &(p, b) in plan.blocks() {
            cells.insert(p, b);
        }
        // Pour chaque liaison, la case du mur de chaque salle, à hauteur de passage, est de l'air :
        // on peut aller d'une salle à l'autre.
        for (parent, child) in lay.edges() {
            let (a, b) = (lay.rooms()[parent].0, lay.rooms()[child].0);
            let (ax, az) = (x + a.0 * def.cell, z + a.1 * def.cell);
            let (bx, bz) = (x + b.0 * def.cell, z + b.1 * def.cell);
            let steps = (bx - ax).abs().max((bz - az).abs());
            let (dx, dz) = ((bx - ax).signum(), (bz - az).signum());
            // La salle fait 7 de large : ses murs sont à ±3 ; le couloir est libre de 3 à steps - 3.
            for s in 3..=steps - 3 {
                let p = IVec3::new(ax + dx * s, floor_y + 2, az + dz * s);
                assert_eq!(cells.get(&p), Some(&air), "couloir fermé en {p}");
            }
        }
    }

    #[test]
    fn broken_dungeons_are_refused_with_a_reason() {
        let bad = |text: &str| format!("{:#}", set_with(text).map(|_| ()).unwrap_err());
        let base = source("");
        assert!(bad(&base.replace("(template: \"arena\", role: Boss)", "")).contains("boss"));
        assert!(bad(&base.replace("cell: 16", "cell: 9")).contains("cell"));
        assert!(
            bad(&base.replace(
                "tiers: [\"low\", \"mid\", \"high\"]",
                "tiers: [\"low\", \"nope\"]"
            ))
            .contains("nope")
        );
        assert!(bad(&base.replace("count: (5, 8)", "count: (2, 8)")).contains("3 à 9"));
        assert!(bad(&base.replace("depth: 14", "depth: 2")).contains("profondeur"));
        assert!(bad(&base.replace("spacing: 12", "spacing: 3")).contains("spacing"));
        assert!(bad(&base.replace("dungeon: \"crypt\"", "dungeon: \"ghost\"")).contains("ghost"));
    }
}
