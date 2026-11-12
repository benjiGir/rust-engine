//! Géométrie de conduits calculée depuis les voisins, sans variantes de blocs sauvegardées.
use std::{borrow::Cow, fmt};

use glam::{IVec3, Vec3};
use serde::Deserialize;

use crate::{BlockBox, BlockDef, BlockId, BlockRegistry, Capabilities, FACINGS, Face, NetworkKind};

/// Dimensions en seizièmes de bloc, indépendantes de la sorte et du canal du réseau.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionSize {
    pub core: f32,
    pub arm: f32,
}

/// Les 64 géométries possibles sont préparées au chargement, sans réserver de BlockId.
#[derive(Clone, PartialEq)]
pub struct ConnectedShape {
    core: f32,
    arm: f32,
    models: [Vec<BlockBox>; 64],
}

impl fmt::Debug for ConnectedShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectedShape")
            .field("core", &self.core)
            .field("arm", &self.arm)
            .finish()
    }
}

impl ConnectedShape {
    pub(crate) fn new(size: &ConnectionSize) -> anyhow::Result<Self> {
        anyhow::ensure!(
            size.core.is_finite()
                && size.arm.is_finite()
                && size.arm > 0.0
                && size.arm <= size.core
                && size.core < 16.0,
            "connections exige 0 < arm <= core < 16 (dimensions finies)"
        );
        let (core, arm) = (size.core / 16.0, size.arm / 16.0);
        let lo = (1.0 - core) * 0.5;
        let hi = 1.0 - lo;
        let models = std::array::from_fn(|mask| {
            let mut boxes = vec![BlockBox {
                min: Vec3::splat(lo),
                max: Vec3::splat(hi),
            }];
            for face in Face::ALL {
                if mask & (1 << face as u8) == 0 {
                    continue;
                }
                let axis = face as usize / 2;
                let mut b = BlockBox {
                    min: Vec3::splat((1.0 - arm) * 0.5),
                    max: Vec3::splat((1.0 + arm) * 0.5),
                };
                if face as u8 & 1 == 0 {
                    b.min[axis] = hi;
                    b.max[axis] = 1.0;
                } else {
                    b.min[axis] = 0.0;
                    b.max[axis] = lo;
                }
                boxes.push(b);
            }
            boxes
        });
        Ok(Self { core, arm, models })
    }

    /// Bits dans l'ordre de Face::ALL : +X, -X, +Y, -Y, +Z, -Z.
    pub fn boxes(&self, mask: u8) -> &[BlockBox] {
        &self.models[(mask & 63) as usize]
    }
}

pub(crate) fn kind(def: &BlockDef) -> Option<NetworkKind> {
    def.conduit
        .or_else(|| def.behavior.as_deref().and_then(NetworkKind::of_behavior))
}

/// Même critère que le graphe des réseaux : sorte/canal, ou capacité ouverte en face.
fn connects(source: &BlockDef, target: &BlockDef, caps: Option<&Capabilities>, face: Face) -> bool {
    let Some(resource) = kind(source) else {
        return false;
    };
    if let Some(other) = kind(target) {
        return resource == other && source.channel == target.channel;
    }
    caps.and_then(|c| resource.sides(c))
        .is_some_and(|s| s.contains(face.opposite()))
}

/// Les seuls bits utiles à la forme : modifier une quantité ne provoque pas de remaillage.
pub(crate) fn ports(caps: Option<&Capabilities>) -> [u8; 4] {
    [
        NetworkKind::Energy,
        NetworkKind::Fluid,
        NetworkKind::Kinetic,
        NetworkKind::Items,
    ]
    .map(|kind| caps.and_then(|c| kind.sides(c)).map_or(0, |s| s.0 & 63))
}

/// Lecture commune au monde, à la physique et aux instantanés de maillage.
pub(crate) fn boxes_at<'a>(
    registry: &'a BlockRegistry,
    pos: IVec3,
    block: impl Fn(IVec3) -> BlockId,
    caps: impl Fn(IVec3) -> Option<&'a Capabilities>,
) -> Cow<'a, [BlockBox]> {
    let def = registry.def(block(pos));
    if let Some(shape) = &def.connections {
        let mut mask = 0;
        for face in Face::ALL {
            let q = pos + FACINGS[face as usize];
            let neighbour = block(q);
            if !neighbour.is_air() && connects(def, registry.def(neighbour), caps(q), face) {
                mask |= 1 << face as u8;
            }
        }
        return Cow::Borrowed(shape.boxes(mask));
    }
    // Un raccord sur une machine en retrait rejoint sa surface réelle, dans sa cellule.
    // Un cube plein touche déjà son bord ; les modèles sans capacité restent inchangés.
    if def.boxes.is_empty() || ports(caps(pos)) == [0; 4] {
        return Cow::Borrowed(&def.boxes);
    }
    let mut boxes = Cow::Borrowed(def.boxes.as_slice());
    for face in Face::ALL {
        let q = pos + FACINGS[face as usize];
        let neighbour = registry.def(block(q));
        if let Some(shape) = &neighbour.connections
            && connects(neighbour, def, caps(pos), face.opposite())
            && let Some(port) = terminal(&def.boxes, face, shape.arm)
        {
            boxes.to_mut().push(port);
        }
    }
    boxes
}

/// Le tronçon entre la frontière de cellule et la première pièce qui coupe son axe.
fn terminal(boxes: &[BlockBox], face: Face, width: f32) -> Option<BlockBox> {
    let axis = face as usize / 2;
    let positive = face as u8 & 1 == 0;
    let mut port = BlockBox {
        min: Vec3::splat((1.0 - width) * 0.5),
        max: Vec3::splat((1.0 + width) * 0.5),
    };
    let candidates = boxes.iter().filter(|b| {
        (0..3)
            .filter(|&a| a != axis)
            .all(|a| b.min[a] < port.max[a] && b.max[a] > port.min[a])
    });
    let edge = if positive {
        candidates.map(|b| b.max[axis]).reduce(f32::max)?
    } else {
        candidates.map(|b| b.min[axis]).reduce(f32::min)?
    };
    if positive {
        port.min[axis] = edge;
        port.max[axis] = 1.0;
    } else {
        port.min[axis] = 0.0;
        port.max[axis] = edge;
    }
    (port.max[axis] - port.min[axis] > 1e-5).then_some(port)
}
