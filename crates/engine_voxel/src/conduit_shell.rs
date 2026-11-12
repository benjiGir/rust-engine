//! Enveloppes de dessin ouvertes sur le contenu ; les boîtes physiques restent inchangées.
use crate::{BlockBox, BlockDef, NetworkKind};
use glam::Vec3;

pub struct ConduitShell {
    pub rails: Vec<BlockBox>,
    pub windows: Vec<BlockBox>,
}

pub fn conduit_shell(def: &BlockDef, boxes: &[BlockBox]) -> Option<ConduitShell> {
    let kind = def
        .conduit
        .or_else(|| def.behavior.as_deref().and_then(NetworkKind::of_behavior));
    let connected = def.connections.is_some();
    let closed_items = kind == Some(NetworkKind::Items)
        && (connected || boxes.first().is_some_and(|b| b.max.y >= 0.5));
    if kind != Some(NetworkKind::Fluid) && !closed_items {
        return None;
    }
    let mut shell = ConduitShell {
        rails: Vec::new(),
        windows: Vec::new(),
    };
    for (i, b) in boxes.iter().enumerate() {
        if !connected && i > 0 {
            shell.rails.push(*b);
            continue;
        }
        shell.windows.push(*b);
        let edge = ((b.max - b.min).min_element() * 0.15).min(1.0 / 32.0);
        for axis in 0..3 {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            for high_u in [false, true] {
                for high_v in [false, true] {
                    let mut min = b.min;
                    let mut max = b.max;
                    for (a, high) in [(u, high_u), (v, high_v)] {
                        if high {
                            min[a] = max[a] - edge;
                        } else {
                            max[a] = min[a] + edge;
                        }
                    }
                    if (max - min).cmpgt(Vec3::ZERO).all() {
                        shell.rails.push(BlockBox { min, max });
                    }
                }
            }
        }
    }
    Some(shell)
}
