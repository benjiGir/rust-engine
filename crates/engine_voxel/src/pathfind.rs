//! La recherche de chemin (phase O5) : un A* sur les blocs, pour les créatures.
//!
//! Une case du chemin est la position des pieds. Une créature peut s'y tenir si son corps
//! (sa hauteur en blocs) y passe sans bloc plein ni danger, et si un bloc plein la porte,
//! ou si c'est de l'eau (elle nage, plus lentement). D'une case à l'autre :
//! - à plat, dans les 8 directions (en diagonale seulement si les deux côtés sont libres :
//!   pas de coin coupé) ;
//! - une marche d'un bloc vers le haut (un saut), s'il y a la place au-dessus de la tête ;
//! - une descente d'au plus `max_drop` blocs (au-delà, elle se ferait mal).
//!
//! Le nombre de cases explorées est borné : sans chemin, ou trop loin, on renvoie le
//! chemin vers la case la plus proche du but (la créature s'en approche quand même).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::IVec3;
use rustc_hash::FxHashMap as HashMap;

use crate::block::BlockId;
use crate::world::World;

/// Les règles d'une créature.
pub struct PathRules<F: Fn(BlockId) -> bool> {
    /// Sa hauteur, en blocs (arrondie au-dessus).
    pub height: i32,
    /// La plus grande chute acceptée.
    pub max_drop: i32,
    /// Au plus tant de cases explorées.
    pub max_nodes: usize,
    /// Les blocs à ne jamais traverser (la lave).
    pub danger: F,
}

/// Un chemin : les cases à suivre, la première après le départ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub cells: Vec<IVec3>,
    /// Faux : le but n'a pas été atteint, le chemin mène au plus près.
    pub complete: bool,
}

/// Coûts, en dixièmes de bloc (des entiers pour le tas).
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
const CLIMB: u32 = 10;
const DROP_PER_BLOCK: u32 = 5;
const SWIM: u32 = 30;

const HORIZONTAL: [IVec3; 8] = [
    IVec3::new(1, 0, 0),
    IVec3::new(-1, 0, 0),
    IVec3::new(0, 0, 1),
    IVec3::new(0, 0, -1),
    IVec3::new(1, 0, 1),
    IVec3::new(1, 0, -1),
    IVec3::new(-1, 0, 1),
    IVec3::new(-1, 0, -1),
];

struct Walker<'a, F: Fn(BlockId) -> bool> {
    world: &'a World,
    rules: &'a PathRules<F>,
}

impl<F: Fn(BlockId) -> bool> Walker<'_, F> {
    /// Le corps passe-t-il en `p` (sans bloc plein ni danger) ?
    fn body_free(&self, p: IVec3) -> bool {
        (0..self.rules.height).all(|h| {
            let block = self.world.block(p + IVec3::Y * h);
            let def = self.world.registry().def(block);
            (!def.solid || crate::door::is_open(def)) && !(self.rules.danger)(block)
        })
    }

    fn is_water(&self, p: IVec3) -> bool {
        self.world.registry().def(self.world.block(p)).liquid
    }

    /// Le coût en plus pour se tenir en `p`, ou `None` si on ne peut pas s'y tenir.
    fn stand(&self, p: IVec3) -> Option<u32> {
        if !self.body_free(p) {
            return None;
        }
        if self.world.is_solid(p - IVec3::Y) {
            Some(0)
        } else if self.is_water(p) {
            Some(SWIM)
        } else {
            None
        }
    }

    /// Les cases voisines et le coût pour y aller.
    fn neighbours(&self, from: IVec3, out: &mut Vec<(IVec3, u32)>) {
        out.clear();
        let headroom = self.body_free(from + IVec3::Y);
        for (i, d) in HORIZONTAL.iter().enumerate() {
            let diagonal = i >= 4;
            if diagonal {
                // Pas de coin coupé : les deux côtés doivent laisser passer.
                let side_a = from + IVec3::new(d.x, 0, 0);
                let side_b = from + IVec3::new(0, 0, d.z);
                if !self.body_free(side_a) || !self.body_free(side_b) {
                    continue;
                }
            }
            let step = if diagonal { DIAGONAL } else { STRAIGHT };
            let next = from + *d;
            if let Some(extra) = self.stand(next) {
                out.push((next, step + extra));
                continue;
            }
            // Une marche : sauter d'un bloc (pas en diagonale).
            if !diagonal
                && headroom
                && let Some(extra) = self.stand(next + IVec3::Y)
            {
                out.push((next + IVec3::Y, step + CLIMB + extra));
                continue;
            }
            // Le vide : descendre, si ce n'est pas trop haut.
            if self.body_free(next) {
                for drop in 1..=self.rules.max_drop {
                    let below = next - IVec3::Y * drop;
                    if let Some(extra) = self.stand(below) {
                        out.push((below, step + DROP_PER_BLOCK * drop as u32 + extra));
                        break;
                    }
                    if !self.body_free(below) {
                        break;
                    }
                }
            }
        }
    }
}

/// L'estimation du reste : la distance (en dixièmes), qui ne surestime jamais.
fn heuristic(a: IVec3, b: IVec3) -> u32 {
    let d = (a - b).abs();
    let (dx, dz) = (d.x.max(d.z) as u32, d.x.min(d.z) as u32);
    (dx - dz) * STRAIGHT + dz * DIAGONAL + d.y as u32 * STRAIGHT
}

/// Cherche un chemin des pieds en `start` vers `goal`.
pub fn find_path<F: Fn(BlockId) -> bool>(
    world: &World,
    start: IVec3,
    goal: IVec3,
    rules: &PathRules<F>,
) -> Path {
    let walker = Walker { world, rules };
    // Pour chaque case vue : le coût depuis le départ, et d'où l'on vient.
    let mut seen: HashMap<IVec3, (u32, IVec3)> = HashMap::default();
    let mut open = BinaryHeap::new();
    seen.insert(start, (0, start));
    open.push(Reverse((heuristic(start, goal), 0u32, start.to_array())));
    let mut best = (heuristic(start, goal), start);
    let mut neighbours = Vec::with_capacity(8);
    let mut explored = 0;
    let mut reached = false;
    while let Some(Reverse((_, cost, cell))) = open.pop() {
        let cell = IVec3::from(cell);
        if seen.get(&cell).is_some_and(|&(c, _)| c < cost) {
            continue; // Une entrée périmée : on a trouvé mieux depuis.
        }
        if cell == goal {
            best = (0, cell);
            reached = true;
            break;
        }
        explored += 1;
        if explored > rules.max_nodes {
            break;
        }
        walker.neighbours(cell, &mut neighbours);
        for &(next, step) in &neighbours {
            let next_cost = cost + step;
            if seen.get(&next).is_some_and(|&(c, _)| c <= next_cost) {
                continue;
            }
            seen.insert(next, (next_cost, cell));
            let h = heuristic(next, goal);
            if h < best.0 {
                best = (h, next);
            }
            open.push(Reverse((next_cost + h, next_cost, next.to_array())));
        }
    }
    // On remonte de la meilleure case au départ.
    let mut cells = Vec::new();
    let mut cell = best.1;
    while cell != start {
        cells.push(cell);
        cell = seen[&cell].1;
    }
    cells.reverse();
    Path {
        cells,
        complete: reached,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockRegistry;

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }

    /// Un sol de pierre de 20 × 20 en y = 0 : on marche en y = 1.
    fn floor() -> World {
        let mut world = World::new();
        let stone = id("core:stone");
        for x in -10..10 {
            for z in -10..10 {
                world.set_block(IVec3::new(x, 0, z), stone);
            }
        }
        world
    }

    fn rules() -> PathRules<impl Fn(BlockId) -> bool> {
        let lava = id("core:lava");
        PathRules {
            height: 2,
            max_drop: 3,
            max_nodes: 2000,
            danger: move |b| b == lava,
        }
    }

    fn wall(world: &mut World, x: i32, zs: std::ops::Range<i32>, height: i32) {
        for z in zs {
            for y in 1..=height {
                world.set_block(IVec3::new(x, y, z), id("core:stone"));
            }
        }
    }

    #[test]
    fn straight_line_on_flat_ground() {
        let world = floor();
        let path = find_path(&world, IVec3::new(0, 1, 0), IVec3::new(5, 1, 0), &rules());
        assert!(path.complete);
        assert_eq!(path.cells.len(), 5);
        assert_eq!(path.cells.last(), Some(&IVec3::new(5, 1, 0)));
    }

    #[test]
    fn goes_around_a_wall() {
        let mut world = floor();
        wall(&mut world, 3, -4..5, 2);
        let path = find_path(&world, IVec3::new(0, 1, 0), IVec3::new(6, 1, 0), &rules());
        assert!(path.complete);
        assert!(
            path.cells
                .iter()
                .all(|c| !(c.x == 3 && (-4..5).contains(&c.z))),
            "jamais dans le mur : {:?}",
            path.cells
        );
        assert!(
            path.cells.iter().any(|c| c.z >= 5 || c.z < -4),
            "fait le tour"
        );
    }

    #[test]
    fn climbs_one_block_but_not_two() {
        let mut world = floor();
        // Un muret d'un bloc sur toute la largeur : on passe par-dessus (monter, redescendre).
        wall(&mut world, 3, -10..10, 1);
        let up = find_path(&world, IVec3::new(0, 1, 0), IVec3::new(6, 1, 0), &rules());
        assert!(up.complete);
        assert!(
            up.cells.iter().any(|c| c.x == 3 && c.y == 2),
            "{:?}",
            up.cells
        );
        // Un mur de deux blocs sur toute la largeur : infranchissable.
        let mut world = floor();
        wall(&mut world, 3, -10..10, 2);
        let blocked = find_path(&world, IVec3::new(0, 1, 0), IVec3::new(6, 1, 0), &rules());
        assert!(!blocked.complete);
        assert_eq!(
            blocked.cells.last().map(|c| c.x),
            Some(2),
            "s'arrête au pied du mur"
        );
    }

    #[test]
    fn drops_three_blocks_but_not_four() {
        let stone = id("core:stone");
        for (depth, possible) in [(3, true), (4, false)] {
            let mut world = World::new();
            // Une terrasse en y = 10, puis un sol plus bas.
            for z in -3..4 {
                for x in -3..1 {
                    world.set_block(IVec3::new(x, 10, z), stone);
                }
                for x in 1..6 {
                    world.set_block(IVec3::new(x, 10 - depth, z), stone);
                }
            }
            let goal = IVec3::new(4, 11 - depth, 0);
            let path = find_path(&world, IVec3::new(-1, 11, 0), goal, &rules());
            assert_eq!(path.complete, possible, "chute de {depth}");
        }
    }

    #[test]
    fn never_walks_through_lava() {
        let mut world = floor();
        let lava = id("core:lava");
        // Un fossé de lave d'un bloc, avec un pont de pierre au bout.
        for z in -10..8 {
            world.set_block(IVec3::new(3, 0, z), lava);
            world.set_block(IVec3::new(3, 1, z), lava);
        }
        let path = find_path(&world, IVec3::new(0, 1, 0), IVec3::new(6, 1, 0), &rules());
        assert!(path.complete);
        assert!(
            path.cells.iter().all(|c| c.x != 3 || c.z >= 8),
            "par le pont : {:?}",
            path.cells
        );
    }

    #[test]
    fn the_node_budget_bounds_the_search() {
        let world = floor();
        let tight = PathRules {
            max_nodes: 5,
            ..rules()
        };
        let path = find_path(&world, IVec3::new(-9, 1, -9), IVec3::new(9, 1, 9), &tight);
        assert!(!path.complete);
        assert!(!path.cells.is_empty(), "s'approche quand même");
    }
}
