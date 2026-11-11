//! Les réseaux : câbles (énergie), tuyaux (fluides, gaz) et arbres de transmission (rotation).
//!
//! # Pourquoi des réseaux
//!
//! Simuler chaque conduit séparément (chacun passe son contenu au voisin) est lent et
//! instable : l'énergie mettrait N ticks à traverser N câbles et oscillerait. On regroupe donc
//! les conduits en **réseaux** :
//!
//! 1. **Le graphe** : un réseau est une composante connexe de conduits de même sorte
//!    ([`NetworkKind`]), avec ses **extrémités** : les machines branchées, et la face par
//!    laquelle elles touchent le conduit (si leur capacité accepte cette face).
//! 2. **Recalcul incrémental** : poser, casser ou changer un voisin d'un conduit marque son
//!    réseau « sale ». Au tick suivant, seuls les réseaux sales sont défaits et reconstruits par
//!    un parcours en largeur : un conduit posé fusionne ses voisins, un conduit cassé coupe le
//!    réseau en autant de morceaux qu'il faut. Rien n'est recalculé ailleurs. Les réseaux ne sont
//!    pas sauvegardés : au chargement d'un chunk, ses conduits se déclarent et le graphe se refait.
//! 3. **Un bilan par réseau et par tick** : [`Networks::tick`] répartit d'un coup entre les
//!    extrémités. Le contenu ne « voyage » pas : une machine à 100 câbles d'une centrale est
//!    servie dans le même tick que sa voisine.
//!
//! # Répartition (énergie et fluides)
//!
//! Chaque extrémité a un rôle ([`Io`]) : producteur (`Out`), consommateur (`In`) ou tampon
//! (`Both` : batterie, réservoir). Dans l'ordre, dans la limite du débit du réseau :
//! 1. les producteurs alimentent les consommateurs, au prorata de ce que chacun peut donner et
//!    de ce que chacun peut recevoir ;
//! 2. le surplus des producteurs remplit les tampons ;
//! 3. les tampons servent les consommateurs encore en manque ;
//! 4. les tampons s'équilibrent entre eux (du plus plein au plus vide).
//!
//! Pour les fluides, un réseau ne transporte qu'un fluide à la fois (celui du plus gros
//! producteur) ; les gaz sont des fluides comme les autres (densité négative dans `fluids.ron`).
//!
//! # Rotation
//!
//! Les sources (une contrainte fournie) imposent leur vitesse au réseau ; les machines
//! reçoivent cette vitesse si la contrainte demandée ne dépasse pas la contrainte fournie, sinon
//! le réseau est **bloqué** et tout s'arrête. Deux sources de sens opposés se bloquent aussi.
//!
//! Les réseaux sont indépendants : leur calcul pourra se répartir sur plusieurs cœurs. Aujourd'hui
//! il tourne sur le thread des machines, et reste sous la milliseconde pour des milliers de câbles
//! (voir les tests).

use std::collections::VecDeque;

use glam::IVec3;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::block::BlockId;
use crate::capabilities::{Capabilities, Face, Io, Sides};
use crate::circuits::FACINGS;
use crate::grid::{BlockAccess, GridId, GridPos, Grids};
use crate::machines::{BlockBehavior, MachineContext};
use crate::registry::BlockRegistry;

/// Un réseau plus grand que cela n'est pas parcouru jusqu'au bout.
const MAX_NETWORK: usize = 200_000;
/// Débit maximal d'un réseau d'énergie, par tick.
pub const ENERGY_RATE: u64 = 4_000;
/// Débit maximal d'un réseau de fluide, en mB par tick.
pub const FLUID_RATE: u64 = 500;
/// Au-dessous de cet écart de remplissage, deux tampons sont considérés à l'équilibre.
const BALANCE_EPSILON: f64 = 0.02;

/// Ce qu'un réseau transporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetworkKind {
    Energy,
    Fluid,
    Kinetic,
}

impl NetworkKind {
    /// La sorte de conduit d'un comportement de bloc (`core:cable`...).
    pub fn of_behavior(behavior: &str) -> Option<Self> {
        match behavior {
            "core:cable" => Some(Self::Energy),
            "core:pipe" => Some(Self::Fluid),
            "core:shaft" => Some(Self::Kinetic),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Energy => "énergie",
            Self::Fluid => "fluide",
            Self::Kinetic => "rotation",
        }
    }

    /// Les faces par lesquelles cette capacité échange, si le bloc en a une de cette sorte.
    fn sides(self, caps: &Capabilities) -> Option<Sides> {
        match self {
            Self::Energy => caps.energy.as_ref().map(|e| e.sides),
            Self::Fluid => {
                let mut tanks = caps.tanks.iter();
                let first = tanks.next()?;
                // Les faces de tous les réservoirs, rôle du premier.
                let bits = tanks.fold(first.sides.0 & 0b11_1111, |acc, t| {
                    acc | t.sides.0 & 0b11_1111
                });
                Some(Sides(bits).with_io(first.sides.io()))
            }
            Self::Kinetic => caps.kinetic.as_ref().map(|k| k.sides),
        }
    }
}

/// Une machine branchée sur un réseau : où elle est, et la face qui touche le conduit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub pos: IVec3,
    pub face: Face,
}

/// Une composante connexe de conduits de même sorte.
#[derive(Debug, Clone)]
pub struct Network {
    pub kind: NetworkKind,
    pub grid: GridId,
    pub members: Vec<IVec3>,
    pub endpoints: Vec<Endpoint>,
}

/// Le bilan des réseaux, pour les mesures et l'inspecteur.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkStats {
    pub networks: usize,
    pub conduits: usize,
    pub endpoints: usize,
    /// Réseaux reconstruits au dernier recalcul.
    pub rebuilt: usize,
}

#[derive(Debug, Default)]
pub struct Networks {
    /// La sorte de conduit de chaque bloc, par numéro de bloc.
    kinds: Vec<Option<NetworkKind>>,
    nets: HashMap<u32, Network>,
    index: HashMap<GridPos, u32>,
    next_id: u32,
    last_rebuilt: usize,
}

impl Networks {
    pub fn new(registry: &BlockRegistry) -> Self {
        let mut kinds: Vec<Option<NetworkKind>> = Vec::new();
        for (id, def) in registry.iter() {
            let kind = def.behavior.as_deref().and_then(NetworkKind::of_behavior);
            let i = id.0 as usize;
            if kinds.len() <= i {
                kinds.resize(i + 1, None);
            }
            kinds[i] = kind;
        }
        Self {
            kinds,
            ..Self::default()
        }
    }

    fn kind_of(&self, block: BlockId) -> Option<NetworkKind> {
        self.kinds.get(block.0 as usize).copied().flatten()
    }

    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    pub fn stats(&self) -> NetworkStats {
        NetworkStats {
            networks: self.nets.len(),
            conduits: self.index.len(),
            endpoints: self.nets.values().map(|n| n.endpoints.len()).sum(),
            rebuilt: self.last_rebuilt,
        }
    }

    /// Le réseau qui contient ce conduit.
    pub fn network_at(&self, at: GridPos) -> Option<&Network> {
        self.nets.get(self.index.get(&at)?)
    }

    pub fn networks(&self) -> impl Iterator<Item = &Network> {
        self.nets.values()
    }

    /// Défait et reconstruit les réseaux touchés par ces positions (des conduits posés, cassés
    /// ou dont un voisin a changé).
    pub fn rebuild(&mut self, grids: Grids, dirty: &[GridPos]) {
        self.last_rebuilt = 0;
        if dirty.is_empty() {
            return;
        }
        let mut seeds: Vec<GridPos> = dirty.to_vec();
        let mut dropped: HashSet<u32> = HashSet::default();
        for at in dirty {
            if let Some(&id) = self.index.get(at) {
                dropped.insert(id);
            }
        }
        for id in dropped {
            self.drop_network(id, &mut seeds);
        }
        seeds.sort_by_key(|p| p.key());
        seeds.dedup();
        for seed in seeds {
            if self.index.contains_key(&seed) {
                continue;
            }
            let Some(kind) = grids
                .get(seed.grid)
                .and_then(|g| self.kind_of(g.borrow().block(seed.pos)))
            else {
                continue;
            };
            self.flood(grids, seed, kind);
            self.last_rebuilt += 1;
        }
    }

    /// Retire un réseau ; ses conduits deviennent des points de départ à reparcourir.
    fn drop_network(&mut self, id: u32, seeds: &mut Vec<GridPos>) {
        if let Some(net) = self.nets.remove(&id) {
            for m in net.members {
                let at = GridPos::new(net.grid, m);
                self.index.remove(&at);
                seeds.push(at);
            }
        }
    }

    /// Le parcours en largeur d'un réseau neuf à partir de `seed`. Il absorbe les réseaux
    /// existants qu'il rencontre (une fusion).
    fn flood(&mut self, grids: Grids, seed: GridPos, kind: NetworkKind) {
        let Some(cell) = grids.get(seed.grid) else {
            return;
        };
        let grid = cell.borrow();
        let id = self.next_id;
        self.next_id += 1;
        let mut members = vec![seed.pos];
        let mut seen: HashSet<IVec3> = HashSet::default();
        seen.insert(seed.pos);
        let mut queue: VecDeque<IVec3> = VecDeque::from([seed.pos]);
        let mut absorbed: HashSet<u32> = HashSet::default();
        while let Some(cell_pos) = queue.pop_front() {
            if members.len() >= MAX_NETWORK {
                break;
            }
            for d in FACINGS {
                let n = cell_pos + d;
                if seen.contains(&n) || self.kind_of(grid.block(n)) != Some(kind) {
                    continue;
                }
                seen.insert(n);
                if let Some(&other) = self.index.get(&GridPos::new(seed.grid, n)) {
                    absorbed.insert(other);
                }
                members.push(n);
                queue.push_back(n);
            }
        }
        // Les réseaux absorbés disparaissent : leurs conduits sont tous dans `members`.
        for other in absorbed {
            if let Some(old) = self.nets.remove(&other) {
                for m in old.members {
                    self.index.remove(&GridPos::new(old.grid, m));
                }
            }
        }
        for &m in &members {
            self.index.insert(GridPos::new(seed.grid, m), id);
        }
        let endpoints = self.find_endpoints(&*grid, kind, &members, &seen);
        self.nets.insert(
            id,
            Network {
                kind,
                grid: seed.grid,
                members,
                endpoints,
            },
        );
    }

    /// Les machines voisines d'un conduit qui échangent par la face qui le touche.
    fn find_endpoints(
        &self,
        grid: &(impl BlockAccess + ?Sized),
        kind: NetworkKind,
        members: &[IVec3],
        member_set: &HashSet<IVec3>,
    ) -> Vec<Endpoint> {
        let mut found: Vec<Endpoint> = Vec::new();
        let mut seen: HashSet<(IVec3, u8)> = HashSet::default();
        for &m in members {
            for (i, d) in FACINGS.iter().enumerate() {
                let n = m + *d;
                if member_set.contains(&n) {
                    continue;
                }
                // La face de la machine tournée vers le conduit : l'opposée de `i`.
                let face = Face::ALL[i ^ 1];
                let Some(caps) = grid.block_entity(n).and_then(|e| e.caps.as_deref()) else {
                    continue;
                };
                if kind.sides(caps).is_some_and(|s| s.contains(face))
                    && seen.insert((n, face as u8))
                {
                    found.push(Endpoint { pos: n, face });
                }
            }
        }
        found.sort_by_key(|e| (e.pos.to_array(), e.face as u8));
        found
    }

    /// Un bilan de tous les réseaux. Renvoie les machines dont le contenu a changé : le jeu
    /// les réveille (une lampe qui reçoit de l'énergie, un ventilateur qui se met à tourner).
    pub fn tick(&mut self, grids: Grids) -> Vec<GridPos> {
        let mut woken = Vec::new();
        for net in self.nets.values() {
            let Some(cell) = grids.get(net.grid) else {
                continue;
            };
            let mut grid = cell.borrow_mut();
            let changed = match net.kind {
                NetworkKind::Energy => tick_energy(&mut *grid, net),
                NetworkKind::Fluid => tick_fluid(&mut *grid, net),
                NetworkKind::Kinetic => tick_kinetic(&mut *grid, net),
            };
            woken.extend(changed.into_iter().map(|p| GridPos::new(net.grid, p)));
        }
        woken
    }
}

// ----- Le comportement des conduits -----

/// Un câble, un tuyau ou un arbre : il se déclare « sale » quand il apparaît, se charge ou
/// qu'un voisin change ; le réseau est reconstruit au tick suivant.
pub struct Conduit;

impl BlockBehavior for Conduit {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.mark_network_dirty(at);
        }
    }

    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        for &at in positions {
            ctx.mark_network_dirty(at);
        }
    }
}

// ----- La répartition -----

/// Une extrémité vue par la répartition : son rôle, son niveau de remplissage (0 à 1), ce
/// qu'elle peut donner et ce qu'elle peut recevoir (au débit près), sa capacité.
#[derive(Debug, Clone, Copy)]
struct Slot {
    pos: IVec3,
    face: Face,
    /// Pour les fluides : le numéro du réservoir concerné.
    tank: usize,
    io: Io,
    level: f64,
    capacity: u64,
    avail: u64,
    room: u64,
}

/// Répartit `total` au prorata de `weights`, sans que personne dépasse son poids : la somme
/// rendue vaut `min(total, Σ poids)`.
fn split(total: u64, weights: &[u64]) -> Vec<u64> {
    let sum: u64 = weights.iter().sum();
    let mut out = vec![0u64; weights.len()];
    if sum == 0 || total == 0 {
        return out;
    }
    let total = total.min(sum);
    for (o, &w) in out.iter_mut().zip(weights) {
        *o = (u128::from(total) * u128::from(w) / u128::from(sum)) as u64;
    }
    let mut rest = total - out.iter().sum::<u64>();
    while rest > 0 {
        let mut progressed = false;
        for (o, &w) in out.iter_mut().zip(weights) {
            if rest > 0 && *o < w {
                *o += 1;
                rest -= 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

/// Le plan d'un tick : pour chaque extrémité, ce qu'elle donne (négatif) ou reçoit (positif).
fn plan(slots: &[Slot], rate: u64) -> Vec<i64> {
    let mut delta = vec![0i64; slots.len()];
    let pick = |io: Io| -> Vec<usize> { (0..slots.len()).filter(|&i| slots[i].io == io).collect() };
    let (sources, sinks, buffers) = (pick(Io::Out), pick(Io::In), pick(Io::Both));
    let mut src: Vec<u64> = sources.iter().map(|&i| slots[i].avail).collect();
    let mut snk: Vec<u64> = sinks.iter().map(|&i| slots[i].room).collect();
    let mut buf_room: Vec<u64> = buffers.iter().map(|&i| slots[i].room).collect();
    let mut buf_avail: Vec<u64> = buffers.iter().map(|&i| slots[i].avail).collect();
    let mut budget = rate;

    // Déplace `amount` des `from` vers les `to`, au prorata.
    let flow = |amount: u64,
                from: (&[usize], &mut Vec<u64>),
                to: (&[usize], &mut Vec<u64>),
                delta: &mut Vec<i64>| {
        let taken = split(amount, from.1);
        let given = split(amount, to.1);
        for ((&slot, left), t) in from.0.iter().zip(from.1.iter_mut()).zip(&taken) {
            delta[slot] -= *t as i64;
            *left -= t;
        }
        for ((&slot, left), g) in to.0.iter().zip(to.1.iter_mut()).zip(&given) {
            delta[slot] += *g as i64;
            *left -= g;
        }
    };

    // 1. Les producteurs alimentent les consommateurs.
    let moved = budget.min(src.iter().sum()).min(snk.iter().sum());
    flow(moved, (&sources, &mut src), (&sinks, &mut snk), &mut delta);
    budget -= moved;
    // 2. Le surplus remplit les tampons.
    let moved = budget.min(src.iter().sum()).min(buf_room.iter().sum());
    flow(
        moved,
        (&sources, &mut src),
        (&buffers, &mut buf_room),
        &mut delta,
    );
    budget -= moved;
    // 3. Les tampons servent les consommateurs en manque.
    let moved = budget.min(buf_avail.iter().sum()).min(snk.iter().sum());
    flow(
        moved,
        (&buffers, &mut buf_avail),
        (&sinks, &mut snk),
        &mut delta,
    );
    budget -= moved;
    // Ce que les tampons ont reçu à l'étape 2 ne se redonne pas dans le même tick, mais leurs
    // réserves d'avant oui : `buf_avail` a été décrémenté par l'étape 3 seulement.

    // 4. Les tampons s'équilibrent : du plus plein vers le plus vide.
    if budget > 0 && buffers.len() > 1 {
        let hi = (0..buffers.len())
            .filter(|&k| buf_avail[k] > 0)
            .max_by(|&a, &b| slots[buffers[a]].level.total_cmp(&slots[buffers[b]].level));
        let lo = (0..buffers.len())
            .filter(|&k| buf_room[k] > 0 && Some(k) != hi)
            .min_by(|&a, &b| slots[buffers[a]].level.total_cmp(&slots[buffers[b]].level));
        if let (Some(h), Some(l)) = (hi, lo) {
            let (sh, sl) = (&slots[buffers[h]], &slots[buffers[l]]);
            let gap = sh.level - sl.level;
            if gap > BALANCE_EPSILON {
                let half = (gap / 2.0 * sh.capacity.min(sl.capacity) as f64).ceil() as u64;
                let amount = budget.min(buf_avail[h]).min(buf_room[l]).min(half.max(1));
                delta[buffers[h]] -= amount as i64;
                delta[buffers[l]] += amount as i64;
            }
        }
    }
    delta
}

fn tick_energy(grid: &mut (impl BlockAccess + ?Sized), net: &Network) -> Vec<IVec3> {
    if net.endpoints.len() < 2 {
        return Vec::new();
    }
    let slots: Vec<Slot> = net
        .endpoints
        .iter()
        .filter_map(|e| {
            let store = grid.block_entity(e.pos)?.caps.as_ref()?.energy.as_ref()?;
            let io = store.sides.io();
            let capacity = store.capacity.max(1);
            Some(Slot {
                pos: e.pos,
                face: e.face,
                tank: 0,
                io,
                level: store.amount as f64 / capacity as f64,
                capacity,
                avail: if io == Io::In {
                    0
                } else {
                    store.clone().extract(Some(e.face), ENERGY_RATE, true)
                },
                room: if io == Io::Out {
                    0
                } else {
                    store.clone().insert(Some(e.face), ENERGY_RATE, true)
                },
            })
        })
        .collect();
    let delta = plan(&slots, ENERGY_RATE);
    let mut changed = Vec::new();
    for (slot, d) in slots.iter().zip(delta) {
        if d == 0 {
            continue;
        }
        let Some(store) = grid
            .block_entity_mut(slot.pos)
            .and_then(|e| e.caps.as_mut())
            .and_then(|c| c.energy.as_mut())
        else {
            continue;
        };
        if d < 0 {
            store.extract(Some(slot.face), d.unsigned_abs(), false);
        } else {
            store.insert(Some(slot.face), d as u64, false);
        }
        changed.push(slot.pos);
    }
    changed
}

fn tick_fluid(grid: &mut (impl BlockAccess + ?Sized), net: &Network) -> Vec<IVec3> {
    if net.endpoints.len() < 2 {
        return Vec::new();
    }
    // Le fluide du réseau : celui du plus gros producteur (ou tampon) non vide.
    let mut best: Option<(String, u32)> = None;
    for e in &net.endpoints {
        let Some(caps) = grid.block_entity(e.pos).and_then(|b| b.caps.as_deref()) else {
            continue;
        };
        for tank in caps.tanks.iter().filter(|t| t.sides.contains(e.face)) {
            if tank.sides.io() != Io::In
                && tank.amount > 0
                && let Some(f) = &tank.fluid
                && best.as_ref().is_none_or(|(_, a)| tank.amount > *a)
            {
                best = Some((f.clone(), tank.amount));
            }
        }
    }
    let Some((fluid, _)) = best else {
        return Vec::new();
    };
    let slots: Vec<Slot> = net
        .endpoints
        .iter()
        .filter_map(|e| {
            let caps = grid.block_entity(e.pos)?.caps.as_deref()?;
            // Le premier réservoir qui touche cette face et peut parler de ce fluide.
            let (index, tank) = caps.tanks.iter().enumerate().find(|(_, t)| {
                t.sides.contains(e.face) && t.fluid.as_deref().is_none_or(|f| f == fluid)
            })?;
            let io = tank.sides.io();
            let capacity = u64::from(tank.capacity.max(1));
            let has = tank.fluid.as_deref() == Some(fluid.as_str());
            Some(Slot {
                pos: e.pos,
                face: e.face,
                tank: index,
                io,
                level: if has {
                    f64::from(tank.amount) / capacity as f64
                } else {
                    0.0
                },
                capacity,
                avail: if io == Io::In || !has {
                    0
                } else {
                    u64::from(tank.clone().drain(
                        Some(e.face),
                        Some(&fluid),
                        FLUID_RATE as u32,
                        true,
                    ))
                },
                room: if io == Io::Out {
                    0
                } else {
                    u64::from(
                        tank.clone()
                            .fill(Some(e.face), &fluid, FLUID_RATE as u32, true),
                    )
                },
            })
        })
        .collect();
    let delta = plan(&slots, FLUID_RATE);
    let mut changed = Vec::new();
    for (slot, d) in slots.iter().zip(delta) {
        if d == 0 {
            continue;
        }
        let Some(tank) = grid
            .block_entity_mut(slot.pos)
            .and_then(|e| e.caps.as_mut())
            .and_then(|c| c.tanks.get_mut(slot.tank))
        else {
            continue;
        };
        if d < 0 {
            tank.drain(
                Some(slot.face),
                Some(&fluid),
                d.unsigned_abs() as u32,
                false,
            );
        } else {
            tank.fill(Some(slot.face), &fluid, d as u32, false);
        }
        changed.push(slot.pos);
    }
    changed
}

fn tick_kinetic(grid: &mut (impl BlockAccess + ?Sized), net: &Network) -> Vec<IVec3> {
    let mut capacity = 0.0f32;
    let mut impact = 0.0f32;
    let mut drive = 0.0f32;
    let mut forward = false;
    let mut backward = false;
    let mut consumers: Vec<IVec3> = Vec::new();
    for e in &net.endpoints {
        let Some(k) = grid
            .block_entity(e.pos)
            .and_then(|b| b.caps.as_ref())
            .and_then(|c| c.kinetic.as_ref())
        else {
            continue;
        };
        if k.stress_capacity > 0.0 && k.speed != 0.0 {
            capacity += k.stress_capacity;
            forward |= k.speed > 0.0;
            backward |= k.speed < 0.0;
            if k.speed.abs() > drive.abs() {
                drive = k.speed;
            }
        }
        if k.stress_impact > 0.0 {
            impact += k.stress_impact;
            consumers.push(e.pos);
        }
    }
    // Des sources de sens contraires, ou trop de contrainte : le réseau est bloqué.
    let blocked = (forward && backward) || impact > capacity;
    let speed = if blocked { 0.0 } else { drive };
    let mut changed = Vec::new();
    for pos in consumers {
        let Some(k) = grid
            .block_entity_mut(pos)
            .and_then(|b| b.caps.as_mut())
            .and_then(|c| c.kinetic.as_mut())
        else {
            continue;
        };
        if (k.speed - speed).abs() > f32::EPSILON {
            k.speed = speed;
            changed.push(pos);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(io: Io, level: f64, avail: u64, room: u64) -> Slot {
        Slot {
            pos: IVec3::ZERO,
            face: Face::PosX,
            tank: 0,
            io,
            level,
            capacity: 1000,
            avail,
            room,
        }
    }

    #[test]
    fn splitting_is_proportional_and_never_exceeds_a_weight() {
        assert_eq!(split(10, &[30, 10]), vec![8, 2]);
        assert_eq!(split(100, &[3, 4]), vec![3, 4], "au plus la somme");
        assert_eq!(split(5, &[1, 1, 1, 1, 1, 1]).iter().sum::<u64>(), 5);
        assert_eq!(split(0, &[5, 5]), vec![0, 0]);
        assert_eq!(split(7, &[0, 0]), vec![0, 0]);
        for total in 0..60 {
            let weights = [7, 13, 1, 22];
            let out = split(total, &weights);
            assert_eq!(out.iter().sum::<u64>(), total.min(43));
            assert!(out.iter().zip(weights).all(|(o, w)| *o <= w));
        }
    }

    #[test]
    fn producers_feed_consumers_then_buffers_then_buffers_feed_consumers() {
        // Un producteur (100) pour deux consommateurs qui veulent 60 chacun : 50 et 50.
        let slots = [
            slot(Io::Out, 1.0, 100, 0),
            slot(Io::In, 0.0, 0, 60),
            slot(Io::In, 0.0, 0, 60),
        ];
        assert_eq!(plan(&slots, 1000), vec![-100, 50, 50]);
        // Sans consommateur, le producteur remplit la batterie.
        let slots = [slot(Io::Out, 1.0, 100, 0), slot(Io::Both, 0.1, 100, 300)];
        assert_eq!(plan(&slots, 1000), vec![-100, 100]);
        // Sans producteur, la batterie sert le consommateur.
        let slots = [slot(Io::Both, 0.5, 80, 300), slot(Io::In, 0.0, 0, 50)];
        assert_eq!(plan(&slots, 1000), vec![-50, 50]);
        // Le débit du réseau limite tout.
        let slots = [slot(Io::Out, 1.0, 5000, 0), slot(Io::In, 0.0, 0, 5000)];
        assert_eq!(plan(&slots, 400), vec![-400, 400]);
    }

    #[test]
    fn buffers_balance_towards_equal_levels() {
        let slots = [slot(Io::Both, 0.9, 500, 100), slot(Io::Both, 0.1, 100, 500)];
        let delta = plan(&slots, 1000);
        assert!(delta[0] < 0 && delta[1] > 0 && delta[0] + delta[1] == 0);
        // Déjà à l'équilibre : rien ne bouge.
        let slots = [
            slot(Io::Both, 0.5, 100, 100),
            slot(Io::Both, 0.51, 100, 100),
        ];
        assert_eq!(plan(&slots, 1000), vec![0, 0]);
    }

    // ----- Les réseaux dans un vrai monde -----

    use std::cell::RefCell;

    use crate::machines::Machines;
    use crate::machines::tests::loaded_streamer;
    use crate::streaming::ChunkStreamer;

    struct Rig {
        s: RefCell<ChunkStreamer>,
        machines: Machines,
    }

    const ORIGIN: IVec3 = IVec3::new(4, 110, 4);

    impl Rig {
        fn new() -> Self {
            let s = loaded_streamer();
            let machines = Machines::with_natives(s.borrow().world().registry());
            let rig = Self { s, machines };
            // Un sol de pierre, et de l'air dégagé au-dessus (le ciel : lumière maximale).
            let stone = rig.id("core:stone");
            for x in -2..60 {
                for z in -2..8 {
                    for y in -2..10 {
                        let block = if y < 0 { stone } else { BlockId::AIR };
                        rig.s
                            .borrow_mut()
                            .set_block(ORIGIN + IVec3::new(x, y, z), block);
                    }
                }
            }
            rig
        }

        fn id(&self, key: &str) -> BlockId {
            self.s.borrow().world().registry().id(key).unwrap()
        }

        fn at(&self, x: i32, z: i32) -> IVec3 {
            ORIGIN + IVec3::new(x, 0, z)
        }

        fn place(&mut self, pos: IVec3, key: &str) {
            let block = self.id(key);
            assert!(self.s.borrow_mut().set_block(pos, block));
            self.machines.block_changed(&self.s, pos, &mut Vec::new());
        }

        fn remove(&mut self, pos: IVec3) {
            assert!(self.s.borrow_mut().set_block(pos, BlockId::AIR));
            self.machines.block_changed(&self.s, pos, &mut Vec::new());
        }

        fn tick(&mut self, n: usize) {
            for _ in 0..n {
                self.machines.run_tick(&self.s, &mut Vec::new());
            }
        }

        fn is(&self, pos: IVec3, key: &str) -> bool {
            self.s.borrow().world().block(pos) == self.id(key)
        }

        fn with_caps<R>(&self, pos: IVec3, f: impl FnOnce(&mut Capabilities) -> R) -> Option<R> {
            Grids::new(&self.s).write(GridPos::world(pos), |e| f(e.caps_mut()))
        }

        fn energy(&self, pos: IVec3) -> u64 {
            self.with_caps(pos, |c| c.energy.as_ref().map_or(0, |e| e.amount))
                .unwrap_or(0)
        }

        fn water(&self, pos: IVec3) -> u32 {
            self.with_caps(pos, |c| c.fluid_amount("core:water"))
                .unwrap_or(0)
        }

        fn speed(&self, pos: IVec3) -> f32 {
            self.with_caps(pos, |c| c.kinetic.as_ref().map_or(0.0, |k| k.speed))
                .unwrap_or(0.0)
        }

        fn charge(&self, pos: IVec3, amount: u64) {
            self.with_caps(pos, |c| c.energy.as_mut().unwrap().amount = amount);
        }
    }

    /// Une ligne de `n` câbles à partir de x = 1, sur la rangée z = 0.
    fn cable_line(rig: &mut Rig, n: i32) {
        for x in 1..=n {
            rig.place(rig.at(x, 0), "core:cable");
        }
    }

    #[test]
    fn a_solar_panel_lights_a_lamp_through_a_cable_in_the_same_time_at_any_distance() {
        let lit_after = |cables: i32| {
            let mut rig = Rig::new();
            rig.place(rig.at(0, 0), "core:solar_panel");
            cable_line(&mut rig, cables);
            rig.place(rig.at(cables + 1, 0), "core:power_lamp");
            // Le panneau produit 80 par seconde : on attend la première seconde.
            for tick in 1..=200 {
                rig.tick(1);
                if rig.is(rig.at(cables + 1, 0), "core:power_lamp_on") {
                    return tick;
                }
            }
            panic!("la lampe ne s'allume pas ({cables} câbles)");
        };
        // Pas de latence : 3 câbles ou 40, la lampe s'allume au même tick.
        assert_eq!(lit_after(3), lit_after(40));
    }

    #[test]
    fn a_battery_keeps_the_lamp_lit_and_the_lamp_goes_out_when_it_runs_dry() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0), "core:battery");
        cable_line(&mut rig, 5);
        rig.place(rig.at(6, 0), "core:power_lamp");
        rig.tick(3);
        assert!(rig.is(rig.at(6, 0), "core:power_lamp"), "batterie vide");
        rig.charge(rig.at(0, 0), 300);
        rig.tick(5);
        assert!(rig.is(rig.at(6, 0), "core:power_lamp_on"));
        // La lampe a un petit tampon (200) : la batterie le remplit puis compense sa conso.
        let before = rig.energy(rig.at(0, 0));
        rig.tick(10);
        assert!(rig.energy(rig.at(0, 0)) < before, "la lampe consomme");
        // Quelques secondes plus tard : tout est consommé, la lampe s'éteint.
        rig.tick(100);
        assert_eq!(rig.energy(rig.at(0, 0)), 0);
        assert!(rig.is(rig.at(6, 0), "core:power_lamp"));
    }

    #[test]
    fn breaking_a_cable_splits_the_network_and_replacing_it_merges_it_again() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0), "core:battery");
        rig.charge(rig.at(0, 0), 50_000);
        cable_line(&mut rig, 9);
        rig.place(rig.at(10, 0), "core:power_lamp");
        rig.tick(5);
        assert!(rig.is(rig.at(10, 0), "core:power_lamp_on"));
        assert_eq!(rig.machines.networks().stats().networks, 1);
        assert_eq!(rig.machines.networks().stats().conduits, 9);

        // Un câble cassé au milieu : deux réseaux, la lampe s'éteint.
        rig.remove(rig.at(5, 0));
        // La lampe vit encore sur son tampon (200 = 50 ticks) puis s'éteint.
        rig.tick(80);
        let stats = rig.machines.networks().stats();
        assert_eq!((stats.networks, stats.conduits), (2, 8));
        assert!(
            rig.is(rig.at(10, 0), "core:power_lamp"),
            "coupée de la batterie"
        );
        // Seuls les réseaux touchés ont été reconstruits (les deux moitiés).
        assert!(stats.rebuilt <= 2);

        // On le remet : un seul réseau, la lampe se rallume.
        rig.place(rig.at(5, 0), "core:cable");
        rig.tick(5);
        assert_eq!(rig.machines.networks().stats().networks, 1);
        assert!(rig.is(rig.at(10, 0), "core:power_lamp_on"));
    }

    #[test]
    fn a_pump_fills_a_tank_through_pipes_and_a_filler_pours_it_out() {
        let mut rig = Rig::new();
        // Une mare sous la pompe : un bloc d'eau dans le sol.
        let water = rig.id("core:water");
        rig.s.borrow_mut().set_block(rig.at(0, 0) - IVec3::Y, water);
        rig.place(rig.at(0, 0), "core:pump");
        for x in 1..=12 {
            rig.place(rig.at(x, 0), "core:pipe");
        }
        rig.place(rig.at(13, 0), "core:tank");
        rig.tick(200);
        assert!(
            rig.water(rig.at(13, 0)) > 3_000,
            "{} mB",
            rig.water(rig.at(13, 0))
        );
        // Un remplisseur au bout d'un second tuyau : il verse un bloc d'eau au-dessus.
        rig.place(rig.at(13, 1), "core:pipe");
        rig.place(rig.at(13, 2), "core:filler");
        let above = rig.at(13, 2) + IVec3::Y;
        // Le tuyau relie le réservoir et le remplisseur ; le fluide passe du plein vers le vide.
        rig.tick(400);
        assert!(rig.is(above, "core:water"), "un bloc d'eau a été versé");
    }

    #[test]
    fn a_motor_turns_a_fan_through_a_shaft_until_the_network_is_overstressed() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0), "core:battery");
        rig.charge(rig.at(0, 0), 90_000);
        rig.place(rig.at(1, 0), "core:cable");
        rig.place(rig.at(2, 0), "core:motor");
        for x in 3..=8 {
            rig.place(rig.at(x, 0), "core:shaft");
        }
        rig.place(rig.at(9, 0), "core:fan");
        rig.tick(10);
        assert_eq!(rig.speed(rig.at(2, 0)), crate::tech::MOTOR_SPEED);
        assert_eq!(rig.speed(rig.at(9, 0)), crate::tech::MOTOR_SPEED);
        assert!(rig.is(rig.at(9, 0), "core:fan_on"), "le ventilateur tourne");

        // Quatre ventilateurs = 512, exactement la contrainte du moteur : ça tient.
        for x in 4..=6 {
            rig.place(rig.at(x, 1), "core:fan");
        }
        rig.tick(10);
        assert!(rig.is(rig.at(9, 0), "core:fan_on"));
        assert!(rig.is(rig.at(5, 1), "core:fan_on"));
        // Un cinquième : 640 > 512, tout s'arrête.
        rig.place(rig.at(7, 1), "core:fan");
        rig.tick(10);
        assert_eq!(rig.speed(rig.at(9, 0)), 0.0, "bloqué");
        assert!(rig.is(rig.at(9, 0), "core:fan"), "il s'arrête");
        // On l'enlève : le réseau repart.
        rig.remove(rig.at(7, 1));
        rig.tick(10);
        assert!(rig.is(rig.at(9, 0), "core:fan_on"));
    }

    #[test]
    fn networks_rebuild_from_the_blocks_when_chunks_load_again() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0), "core:battery");
        rig.charge(rig.at(0, 0), 50_000);
        cable_line(&mut rig, 6);
        rig.place(rig.at(7, 0), "core:power_lamp");
        rig.tick(5);
        assert!(rig.is(rig.at(7, 0), "core:power_lamp_on"));
        // Rien de sauvegardé : on repart de machines neuves, qui relisent les blocs.
        rig.machines = Machines::with_natives(rig.s.borrow().world().registry());
        assert!(rig.machines.networks().is_empty());
        let positions: Vec<IVec3> = (0..=7).map(|x| rig.at(x, 0)).collect();
        rig.machines
            .blocks_loaded(&rig.s, &positions, &mut Vec::new());
        rig.tick(5);
        assert_eq!(rig.machines.networks().stats().networks, 1);
        // Éteinte à la main, la lampe se rallume : le réseau est revenu.
        let lamp = rig.id("core:power_lamp");
        rig.s.borrow_mut().set_block(rig.at(7, 0), lamp);
        rig.machines
            .block_changed(&rig.s, rig.at(7, 0), &mut Vec::new());
        rig.tick(5);
        assert!(rig.is(rig.at(7, 0), "core:power_lamp_on"));
    }

    #[test]
    fn a_big_network_costs_well_under_a_millisecond_per_tick_per_thousand_cables() {
        let mut rig = Rig::new();
        rig.place(rig.at(0, 0), "core:battery");
        rig.charge(rig.at(0, 0), 90_000);
        // Quatre rangées de 50 câbles reliées : 200 câbles, quelques lampes.
        for z in 0..4 {
            for x in 1..=50 {
                rig.place(rig.at(x, z), "core:cable");
            }
        }
        for x in (5..50).step_by(5) {
            rig.place(rig.at(x, 4), "core:power_lamp");
        }
        rig.tick(5);
        let start = std::time::Instant::now();
        rig.tick(100);
        let per_tick = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
        println!("200 câbles + 9 lampes : {per_tick:.3} ms par tick");
        assert!(per_tick < 5.0, "{per_tick:.2} ms par tick");
    }
}
