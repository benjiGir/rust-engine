//! Traces de transferts pour le dessin uniquement : ni inventaire, ni délai, ni sauvegarde.
use crate::{BlockAccess, FACINGS, Face, GridId, Network};
use glam::IVec3;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::{collections::VecDeque, sync::Arc};

pub const MAX_VISUAL_TRANSFERS: usize = 256;
const MAX_SEARCH: usize = 8192;
const MAX_PATH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitContent {
    Item(String),
    Fluid(String),
}

#[derive(Debug, Clone)]
pub struct VisualTransfer {
    pub grid: GridId,
    pub content: TransitContent,
    pub amount: u64,
    /// Centres des machines aux deux bouts, conduits entre les deux, en coordonnées locales.
    pub path: Arc<[IVec3]>,
}

pub(crate) struct Movement {
    pub from: IVec3,
    pub to: IVec3,
    pub from_face: Option<Face>,
    pub to_face: Option<Face>,
    pub content: TransitContent,
    pub amount: u64,
}

type PathKey = (GridId, IVec3, IVec3, IVec3, u8, u8);
#[derive(Debug, Default)]
pub(crate) struct TransferVisuals {
    paths: HashMap<PathKey, Option<Arc<[IVec3]>>>,
    pending: Vec<VisualTransfer>,
}
impl TransferVisuals {
    pub fn invalidate(&mut self) {
        self.paths.clear();
        self.pending.clear();
    }
    pub fn take(&mut self) -> Vec<VisualTransfer> {
        std::mem::take(&mut self.pending)
    }
    pub fn record(
        &mut self,
        net: &Network,
        grid: &(impl BlockAccess + ?Sized),
        movement: Movement,
    ) {
        if movement.from == movement.to
            || movement.amount == 0
            || self.pending.len() >= MAX_VISUAL_TRANSFERS
        {
            return;
        }
        let key = (
            net.grid,
            net.members[0],
            movement.from,
            movement.to,
            movement.from_face.map_or(6, |f| f as u8),
            movement.to_face.map_or(6, |f| f as u8),
        );
        if self.paths.len() >= 512 {
            self.paths.clear();
        }
        let path = self.paths.entry(key).or_insert_with(|| {
            path(
                net,
                grid,
                movement.from,
                movement.to,
                movement.from_face,
                movement.to_face,
            )
            .map(Arc::from)
        });
        if let Some(path) = path {
            self.pending.push(VisualTransfer {
                grid: net.grid,
                content: movement.content,
                amount: movement.amount,
                path: path.clone(),
            });
        }
    }
}

/// Parcours multi-source : seules les faces réellement raccordées et les sens permis sont pris.
fn path(
    net: &Network,
    grid: &(impl BlockAccess + ?Sized),
    from: IVec3,
    to: IVec3,
    from_face: Option<Face>,
    to_face: Option<Face>,
) -> Option<Vec<IVec3>> {
    let members: HashSet<_> = net.members.iter().copied().collect();
    let flow = |p| grid.registry().def(grid.block(p)).flow;
    let mut previous = HashMap::default();
    let mut queue = VecDeque::new();
    for e in net
        .endpoints
        .iter()
        .filter(|e| e.pos == from && from_face.is_none_or(|f| f == e.face))
    {
        let p = e.pos + FACINGS[e.face as usize];
        if members.contains(&p) && flow(p).is_none_or(|d| p + d != from) {
            previous.insert(p, from);
            queue.push_back(p);
        }
    }
    while let Some(p) = queue.pop_front() {
        if net.endpoints.iter().any(|e| {
            e.pos == to
                && to_face.is_none_or(|f| f == e.face)
                && e.pos + FACINGS[e.face as usize] == p
        }) && flow(p).is_none_or(|d| p + d == to)
        {
            let mut route = vec![to, p];
            let mut at = p;
            while at != from {
                at = *previous.get(&at)?;
                route.push(at);
                if route.len() > MAX_PATH {
                    return None;
                }
            }
            route.reverse();
            return Some(route);
        }
        for d in FACINGS {
            let q = p + d;
            if members.contains(&q)
                && !previous.contains_key(&q)
                && flow(p).is_none_or(|f| p + f == q)
                && flow(q).is_none_or(|f| q + f != p)
            {
                if previous.len() >= MAX_SEARCH {
                    return None;
                }
                previous.insert(q, p);
                queue.push_back(q);
            }
        }
    }
    None
}

/// Apparie les quantités réellement retirées et ajoutées par une phase de répartition.
pub(crate) fn pair_flows(
    from: &[usize],
    taken: &[u64],
    to: &[usize],
    given: &[u64],
    out: &mut Vec<(usize, usize, u64)>,
) {
    let mut i = 0;
    let mut left = taken.first().copied().unwrap_or(0);
    for (&target, &amount) in to.iter().zip(given) {
        let mut amount = amount;
        while amount > 0 && i < from.len() {
            if left == 0 {
                i += 1;
                left = taken.get(i).copied().unwrap_or(0);
                continue;
            }
            let n = amount.min(left);
            if out.len() < MAX_VISUAL_TRANSFERS {
                out.push((from[i], target, n));
            }
            amount -= n;
            left -= n;
        }
    }
}
