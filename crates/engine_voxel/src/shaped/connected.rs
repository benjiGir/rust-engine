//! Les formes qui se raccordent : muret, barrière, portillon, vitre et vitre à plat.
//!
//! Une forme raccordée **occupe seule sa case** : c'est l'usage normal et cela simplifie les raccords.
//! Sa géométrie n'est pas rangée dans la case : elle se calcule à la demande depuis les **voisins**
//! ([`Neighbours`]), comme les conduits (voir `connections.rs`), donc rien ne se sauvegarde de plus
//! que `{ forme, ouvert }` et une case se raccorde toute seule quand un voisin arrive ou part.
//!
//! # Qui se raccorde à qui
//!
//! - un muret : à un muret, à un portillon et aux cubes pleins opaques ;
//! - une barrière : à une barrière, à un portillon et aux cubes pleins opaques ;
//! - un portillon : à rien en particulier, mais il s'oriente selon ses voisins (une barrière, un
//!   muret ou un cube de chaque côté) : il ferme l'axe où il a des voisins, l'axe est-ouest sinon ;
//! - une vitre, une vitre à plat : à leur pareille et aux cubes pleins opaques.
//!
//! Une barrière ou un muret ne se raccorde à un portillon que par l'axe qu'il ferme : un portillon
//! n'a pas de côté ouvert sur ses flancs.
//!
//! # Les dimensions (seizièmes)
//!
//! | Forme | Poteau | Bras |
//! |---|---|---|
//! | muret | 8 × 16 × 8 (14 de haut s'il est droit et sans muret au-dessus) | 6 de large, 14 de haut |
//! | barrière | 4 × 16 × 4 | deux traverses de 2 de large, à 6 et à 12 de haut |
//! | vitre | 2 × 16 × 2 | 2 de large, 16 de haut |
//! | vitre à plat | 8 × 2 × 8 au sol | 8 de large, 2 de haut |
//!
//! La collision d'une barrière et d'un portillon fermé monte à **24** seizièmes : on ne saute pas
//! par-dessus. Un portillon ouvert n'a plus de collision et son battant est dessiné de côté.

use glam::IVec3;

use super::grid::IntBox;
use super::slope::HORIZONTAL;
use super::{Shaped, grid};
use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::registry::{BlockRegistry, Shape};

/// Une forme raccordée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Link {
    Wall,
    Fence,
    Gate,
    Pane,
    FlatPane,
}

impl Link {
    pub const ALL: [Link; 5] = [
        Link::Wall,
        Link::Fence,
        Link::Gate,
        Link::Pane,
        Link::FlatPane,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<Link> {
        Self::ALL.get(usize::from(code)).copied()
    }

    /// Une forme de cette sorte se raccorde-t-elle à une forme `other` voisine ?
    fn joins(self, other: Link) -> bool {
        matches!(
            (self, other),
            (Link::Wall, Link::Wall | Link::Gate)
                | (Link::Fence, Link::Fence | Link::Gate)
                | (Link::Gate, Link::Wall | Link::Fence)
                | (Link::Pane, Link::Pane)
                | (Link::FlatPane, Link::FlatPane)
        )
    }
}

/// Ce que la géométrie d'une forme raccordée sait de ses voisins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Neighbours {
    /// Un bras vers le nord (-Z), l'est (+X), le sud (+Z), l'ouest (-X).
    pub sides: [bool; 4],
    /// Un muret au-dessus (le poteau monte jusqu'à lui).
    pub above: bool,
    /// L'axe d'un portillon : 0 il ferme l'axe est-ouest (X), 1 nord-sud (Z).
    pub axis: u8,
}

impl Neighbours {
    /// Aucun voisin.
    pub const NONE: Neighbours = Neighbours {
        sides: [false; 4],
        above: false,
        axis: 0,
    };

    /// Deux voisins en ligne (est et ouest) : la forme des icônes.
    pub fn sample() -> Neighbours {
        Neighbours {
            sides: [false, true, false, true],
            ..Self::NONE
        }
    }

    fn count(&self) -> usize {
        self.sides.iter().filter(|&&s| s).count()
    }

    /// Exactement deux bras opposés : une ligne droite.
    fn straight(&self) -> bool {
        self.count() == 2 && ((self.sides[0] && self.sides[2]) || (self.sides[1] && self.sides[3]))
    }
}

/// Le bras vers le côté `side` (0 nord, 1 est, 2 sud, 3 ouest) d'un poteau qui occupe
/// `[lo, hi]` en x et z : étroit de `lateral`, sur la hauteur `y`.
fn arm(side: usize, lo: u8, hi: u8, lateral: (u8, u8), y: (u8, u8)) -> IntBox {
    match side {
        0 => [lateral.0, y.0, 0, lateral.1, y.1, lo],
        1 => [hi, y.0, lateral.0, 16, y.1, lateral.1],
        2 => [lateral.0, y.0, hi, lateral.1, y.1, 16],
        _ => [0, y.0, lateral.0, lo, y.1, lateral.1],
    }
}

/// Une boîte dans le repère du portillon (t le long de son axe, s en travers) remise dans la case.
fn gate_box(axis: u8, t: (u8, u8), s: (u8, u8), y: (u8, u8)) -> IntBox {
    if axis == 0 {
        [t.0, y.0, s.0, t.1, y.1, s.1]
    } else {
        [s.0, y.0, t.0, s.1, y.1, t.1]
    }
}

impl Link {
    /// Les boîtes dessinées (et visées), en seizièmes.
    pub(crate) fn visual_boxes(self, open: bool, n: &Neighbours) -> Vec<IntBox> {
        let mut out = Vec::new();
        match self {
            Link::Wall => {
                let tall = n.above || !n.straight();
                out.push([4, 0, 4, 12, if tall { 16 } else { 14 }, 12]);
                for (side, &on) in n.sides.iter().enumerate() {
                    if on {
                        out.push(arm(side, 4, 12, (5, 11), (0, 14)));
                    }
                }
            }
            Link::Fence => {
                out.push([6, 0, 6, 10, 16, 10]);
                for (side, &on) in n.sides.iter().enumerate() {
                    if on {
                        out.push(arm(side, 6, 10, (7, 9), (6, 9)));
                        out.push(arm(side, 6, 10, (7, 9), (12, 15)));
                    }
                }
            }
            Link::Gate => {
                let a = n.axis;
                out.push(gate_box(a, (0, 2), (6, 10), (0, 16)));
                out.push(gate_box(a, (14, 16), (6, 10), (0, 16)));
                if open {
                    // Le battant tourné de 90° autour de la charnière (côté t = 0).
                    out.push(gate_box(a, (0, 2), (10, 16), (6, 9)));
                    out.push(gate_box(a, (0, 2), (10, 16), (12, 15)));
                    out.push(gate_box(a, (0, 2), (13, 15), (9, 12)));
                } else {
                    out.push(gate_box(a, (2, 14), (7, 9), (6, 9)));
                    out.push(gate_box(a, (2, 14), (7, 9), (12, 15)));
                    out.push(gate_box(a, (7, 9), (7, 9), (9, 12)));
                }
            }
            Link::Pane => {
                out.push([7, 0, 7, 9, 16, 9]);
                for (side, &on) in n.sides.iter().enumerate() {
                    if on {
                        out.push(arm(side, 7, 9, (7, 9), (0, 16)));
                    }
                }
            }
            Link::FlatPane => {
                out.push([4, 0, 4, 12, 2, 12]);
                for (side, &on) in n.sides.iter().enumerate() {
                    if on {
                        out.push(arm(side, 4, 12, (4, 12), (0, 2)));
                    }
                }
                // Deux bras voisins (nord-est, est-sud...) comblent leur coin : la nappe est continue.
                for side in 0..4 {
                    let next = (side + 1) % 4;
                    if n.sides[side] && n.sides[next] {
                        let (x, z) = match (side, next) {
                            (0, 1) => ((12, 16), (0, 4)),
                            (1, 2) => ((12, 16), (12, 16)),
                            (2, 3) => ((0, 4), (12, 16)),
                            _ => ((0, 4), (0, 4)),
                        };
                        out.push([x.0, 0, z.0, x.1, 2, z.1]);
                    }
                }
            }
        }
        out
    }

    /// Les boîtes de collision, en seizièmes (la hauteur monte au-delà de 16 : 24). Vide : on traverse.
    pub(crate) fn collision_boxes(self, open: bool, n: &Neighbours) -> Vec<IntBox> {
        match self {
            Link::Fence => {
                let mut out = vec![[6, 0, 6, 10, 24, 10]];
                for (side, &on) in n.sides.iter().enumerate() {
                    if on {
                        out.push(arm(side, 6, 10, (6, 10), (0, 24)));
                    }
                }
                out
            }
            Link::Gate if open => Vec::new(),
            Link::Gate => vec![gate_box(n.axis, (0, 16), (6, 10), (0, 24))],
            _ => self.visual_boxes(open, n),
        }
    }
}

/// Un cube plein opaque : une forme raccordée s'y accroche.
fn is_opaque_cube(registry: &BlockRegistry, block: BlockId) -> bool {
    let def = registry.def(block);
    def.shape == Shape::Cube
        && def.solid
        && !def.transparent
        && !def.liquid
        && def.alpha >= 1.0
        && !def.cutout
}

/// La forme raccordée qui occupe la case dont l'entité est `entity`, si c'en est une.
fn link_in(entity: Option<&BlockEntity>) -> Option<Link> {
    Shaped::link_of(&entity?.data)
}

/// L'axe d'un portillon en `pos` d'après ses voisins non-portillons (une barrière, un muret, un cube).
fn gate_axis<'a>(
    registry: &BlockRegistry,
    pos: IVec3,
    block: &impl Fn(IVec3) -> BlockId,
    entity: &impl Fn(IVec3) -> Option<&'a BlockEntity>,
) -> u8 {
    let sides = sides_of(registry, Link::Gate, pos, block, entity);
    u8::from((sides[0] || sides[2]) && !(sides[1] || sides[3]))
}

/// Les côtés où une forme `link` en `pos` se raccorde.
fn sides_of<'a>(
    registry: &BlockRegistry,
    link: Link,
    pos: IVec3,
    block: &impl Fn(IVec3) -> BlockId,
    entity: &impl Fn(IVec3) -> Option<&'a BlockEntity>,
) -> [bool; 4] {
    std::array::from_fn(|i| {
        let q = pos + super::slope::direction_of(HORIZONTAL[i]);
        let b = block(q);
        if b.is_air() {
            return false;
        }
        if super::is_shaped_block(registry.def(b)) {
            let Some(other) = link_in(entity(q)) else {
                return false;
            };
            if !link.joins(other) {
                return false;
            }
            // Vers un portillon : seulement par l'axe qu'il ferme (est-ouest : côtés 1 et 3).
            return other != Link::Gate
                || gate_axis(registry, q, block, entity) == u8::from(i % 2 == 0);
        }
        is_opaque_cube(registry, b)
    })
}

/// Les voisins d'une forme raccordée `link` en `pos`.
pub fn neighbours_at<'a>(
    registry: &BlockRegistry,
    link: Link,
    pos: IVec3,
    block: impl Fn(IVec3) -> BlockId,
    entity: impl Fn(IVec3) -> Option<&'a BlockEntity>,
) -> Neighbours {
    let sides = sides_of(registry, link, pos, &block, &entity);
    let above = link == Link::Wall
        && !block(pos + IVec3::Y).is_air()
        && super::is_shaped_block(registry.def(block(pos + IVec3::Y)))
        && link_in(entity(pos + IVec3::Y)) == Some(Link::Wall);
    let axis = if link == Link::Gate {
        u8::from((sides[0] || sides[2]) && !(sides[1] || sides[3]))
    } else {
        0
    };
    Neighbours { sides, above, axis }
}

/// L'occupation d'une forme raccordée selon ses voisins.
pub(crate) fn occupancy(link: Link, open: bool, n: &Neighbours) -> grid::Occupancy {
    grid::boxes_occupancy(&link.visual_boxes(open, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sides(n: bool, e: bool, s: bool, w: bool) -> Neighbours {
        Neighbours {
            sides: [n, e, s, w],
            ..Neighbours::NONE
        }
    }

    #[test]
    fn a_wall_grows_arms_toward_its_neighbours() {
        let boxes = |n: &Neighbours| Link::Wall.visual_boxes(false, n);
        // Seul : le poteau, haut (un bout).
        assert_eq!(boxes(&Neighbours::NONE), [[4, 0, 4, 12, 16, 12]]);
        // Un voisin : le poteau et un bras.
        assert_eq!(boxes(&sides(false, true, false, false)).len(), 2);
        // Deux en ligne : le poteau est au niveau des bras (14), pas plus haut.
        let line = boxes(&sides(false, true, false, true));
        assert_eq!(line.len(), 3);
        assert_eq!(line[0][4], 14);
        // Deux en coin : le poteau monte à 16.
        let corner = boxes(&sides(true, true, false, false));
        assert_eq!(corner.len(), 3);
        assert_eq!(corner[0][4], 16);
        // Quatre voisins : quatre bras.
        assert_eq!(boxes(&sides(true, true, true, true)).len(), 5);
        // Un muret au-dessus relève le poteau d'une ligne droite.
        let mut up = sides(false, true, false, true);
        up.above = true;
        assert_eq!(boxes(&up)[0][4], 16);
    }

    #[test]
    fn a_fence_is_taller_to_walk_into_than_to_look_at() {
        let n = sides(false, true, false, false);
        let visual = Link::Fence.visual_boxes(false, &n);
        // Le poteau et deux traverses.
        assert_eq!(visual.len(), 3);
        assert!(visual.iter().all(|b| b[4] <= 16));
        let collision = Link::Fence.collision_boxes(false, &n);
        assert_eq!(collision.len(), 2);
        assert!(collision.iter().all(|b| b[4] == 24));
    }

    #[test]
    fn an_open_gate_does_not_collide() {
        let n = Neighbours::NONE;
        assert!(!Link::Gate.collision_boxes(false, &n).is_empty());
        assert!(Link::Gate.collision_boxes(true, &n).is_empty());
        // Ouvert, le battant est dessiné de côté : plus de boîte au milieu de l'axe.
        let closed = Link::Gate.visual_boxes(false, &n);
        let open = Link::Gate.visual_boxes(true, &n);
        assert_ne!(closed, open);
        assert!(closed.iter().any(|b| b[0] == 2 && b[3] == 14));
        assert!(open.iter().all(|b| !(b[0] == 2 && b[3] == 14)));
    }

    #[test]
    fn a_flat_pane_fills_the_corner_between_two_arms() {
        let isolated = Link::FlatPane.visual_boxes(false, &Neighbours::NONE);
        assert_eq!(isolated, [[4, 0, 4, 12, 2, 12]]);
        let corner = Link::FlatPane.visual_boxes(false, &sides(false, true, true, false));
        // Le centre, deux bras et le coin sud-est comblé.
        assert_eq!(corner.len(), 4);
        assert!(corner.contains(&[12, 0, 12, 16, 2, 16]));
    }
}
