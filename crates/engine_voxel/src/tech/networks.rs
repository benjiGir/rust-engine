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

use crate::tech::network_visual::{Movement, TransferVisuals, TransitContent, pair_flows};
use std::collections::VecDeque;

use glam::IVec3;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::block::BlockId;
use crate::grid::{BlockAccess, GridId, GridPos, Grids};
use crate::tech::capabilities::{Capabilities, Face, Io, ItemStack, Sides};
use crate::tech::circuits::FACINGS;
use crate::tech::machines::{BlockBehavior, MachineContext};
use std::sync::Arc;

use crate::items::ItemRegistry;
use crate::registry::BlockRegistry;
use crate::tech::crafting::MachineRecipes;

/// Un réseau plus grand que cela n'est pas parcouru jusqu'au bout.
const MAX_NETWORK: usize = 200_000;
/// Débit maximal d'un réseau d'énergie, par tick.
pub const ENERGY_RATE: u64 = 4_000;
/// Les réseaux d'objets passent un tick sur ce nombre (5 fois par seconde), et déplacent au plus
/// [`ITEM_RATE`] objets par passage.
pub const ITEM_PERIOD: u64 = 4;
pub const ITEM_RATE: u32 = 8;
/// Débit maximal d'un réseau de fluide, en mB par tick.
pub const FLUID_RATE: u64 = 500;
/// Au-dessous de cet écart de remplissage, deux tampons sont considérés à l'équilibre.
const BALANCE_EPSILON: f64 = 0.02;

/// Ce qu'un réseau transporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
pub enum NetworkKind {
    Energy,
    Fluid,
    Kinetic,
    /// Des objets, d'un inventaire à l'autre (des courroies, des tuyaux à objets).
    Items,
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
            Self::Items => "objets",
        }
    }

    /// Les faces par lesquelles cette capacité échange, si le bloc en a une de cette sorte.
    pub(crate) fn sides(self, caps: &Capabilities) -> Option<Sides> {
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
            Self::Items => caps.inventory.as_ref().map(|i| i.sides),
        }
    }
}

/// Une machine branchée sur un réseau : où elle est, et la face qui touche le conduit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub pos: IVec3,
    pub face: Face,
}

/// La « classe » d'un conduit : sa sorte et son canal. Deux conduits ne se relient que si leur
/// classe est la même.
type Class = (NetworkKind, u16);

/// Une composante connexe de conduits de même sorte.
#[derive(Debug, Clone)]
pub struct Network {
    pub kind: NetworkKind,
    /// Le canal commun de ses conduits (0 : aucun).
    pub channel: u16,
    /// Le débit demandé par le plus lent de ses conduits (`throughput`), s'il y en a un.
    pub rate: Option<u32>,
    pub grid: GridId,
    pub members: Vec<IVec3>,
    pub endpoints: Vec<Endpoint>,
    /// Pour un réseau qui contient des conduits à sens unique : pour chaque machine branchée,
    /// celles où son contenu peut aller en suivant les sens. `None` : tout va partout.
    routes: Option<HashMap<IVec3, HashSet<IVec3>>>,
}

impl Network {
    /// Ce que la machine `from` donne (objets, énergie, fluide) peut-il arriver à la machine `to` ?
    pub fn allows(&self, from: IVec3, to: IVec3) -> bool {
        self.routes
            .as_ref()
            .is_none_or(|r| r.get(&from).is_some_and(|set| set.contains(&to)))
    }
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
    /// La classe (sorte et canal) de chaque conduit, et son débit, par numéro de bloc.
    kinds: Vec<Option<Class>>,
    throughputs: Vec<Option<u32>>,
    /// Le sens des conduits à sens unique, par numéro de bloc.
    flows: Vec<Option<IVec3>>,
    /// Les cases d'inventaire que les réseaux d'objets remplissent et vident, par numéro de bloc.
    ports: Vec<Option<crate::registry::InventoryPorts>>,
    /// La disposition des cases des coffres à paliers (les tuyaux n'en voient que les cases
    /// principales), et les contrôleurs de stockage, par numéro de bloc.
    layouts: Vec<Option<crate::tech::storage::Layout>>,
    controllers: Vec<bool>,
    /// Ce qu'il faut savoir des objets pour les réseaux d'objets (voir `set_item_rules`).
    rules: Option<(Arc<ItemRegistry>, Arc<MachineRecipes>)>,
    /// Les ticks écoulés : les réseaux d'objets passent un tick sur [`ITEM_PERIOD`].
    ticks: u64,
    nets: HashMap<u32, Network>,
    index: HashMap<GridPos, u32>,
    next_id: u32,
    last_rebuilt: usize,
    visuals: TransferVisuals,
}

impl Networks {
    pub fn new(registry: &BlockRegistry) -> Self {
        let mut kinds: Vec<Option<Class>> = Vec::new();
        let mut throughputs: Vec<Option<u32>> = Vec::new();
        let mut flows: Vec<Option<IVec3>> = Vec::new();
        let mut ports: Vec<Option<crate::registry::InventoryPorts>> = Vec::new();
        let mut layouts: Vec<Option<crate::tech::storage::Layout>> = Vec::new();
        let mut controllers: Vec<bool> = Vec::new();
        // Les canaux (`mon_mod:vapeur`) reçoivent un numéro à la première rencontre ; 0 : aucun.
        let mut channels: Vec<&str> = Vec::new();
        for (id, def) in registry.iter() {
            // Un bloc de mod se déclare conduit dans `blocks.ron` (`conduit: Kinetic`) ; ceux du jeu
            // de base le sont par leur comportement natif.
            let kind = def
                .conduit
                .or_else(|| def.behavior.as_deref().and_then(NetworkKind::of_behavior));
            let channel = def.channel.as_deref().map_or(0, |c| {
                let index = channels.iter().position(|k| *k == c).unwrap_or_else(|| {
                    channels.push(c);
                    channels.len() - 1
                });
                index as u16 + 1
            });
            let i = id.0 as usize;
            if kinds.len() <= i {
                kinds.resize(i + 1, None);
                throughputs.resize(i + 1, None);
                flows.resize(i + 1, None);
                ports.resize(i + 1, None);
                layouts.resize(i + 1, None);
                controllers.resize(i + 1, false);
            }
            kinds[i] = kind.map(|k| (k, channel));
            throughputs[i] = def.throughput;
            flows[i] = def.flow;
            ports[i] = def.ports.clone();
            layouts[i] = crate::tech::storage::layout_of(registry, id);
            controllers[i] = def.behavior.as_deref() == Some(crate::tech::storage::CONTROLLER_KEY);
        }
        Self {
            kinds,
            throughputs,
            flows,
            ports,
            layouts,
            controllers,
            ..Self::default()
        }
    }

    /// Les objets et les recettes de machines que connaissent les réseaux d'objets : sans eux,
    /// un réseau d'objets déplace tout, sans filtre de recette.
    pub fn set_item_rules(&mut self, items: Arc<ItemRegistry>, recipes: Arc<MachineRecipes>) {
        self.rules = Some((items, recipes));
    }

    fn class_of(&self, block: BlockId) -> Option<Class> {
        self.kinds.get(block.0 as usize).copied().flatten()
    }

    fn flow_of(&self, block: BlockId) -> Option<IVec3> {
        self.flows.get(block.0 as usize).copied().flatten()
    }

    fn throughput_of(&self, block: BlockId) -> Option<u32> {
        self.throughputs.get(block.0 as usize).copied().flatten()
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
        self.visuals.invalidate();
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
                .and_then(|g| self.class_of(g.borrow().block(seed.pos)))
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
    fn flood(&mut self, grids: Grids, seed: GridPos, class: Class) {
        let (kind, channel) = class;
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
                if seen.contains(&n) || self.class_of(grid.block(n)) != Some(class) {
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
        // Le plus lent des conduits qui annoncent un débit fixe le débit du réseau.
        let rate = members
            .iter()
            .filter_map(|&m| self.throughput_of(grid.block(m)))
            .min();
        let routes = self.routes(&*grid, &members, &endpoints);
        self.nets.insert(
            id,
            Network {
                kind,
                channel,
                rate,
                grid: seed.grid,
                members,
                endpoints,
                routes,
            },
        );
    }

    /// Les trajets des objets dans un réseau qui a des conduits à sens unique : de chaque machine,
    /// ce qu'elle peut atteindre. Un objet entre dans le réseau par un conduit voisin de la
    /// machine (sauf par l'avant d'un conduit à sens unique), avance de conduit en conduit sans
    /// remonter un sens unique, et sort par un conduit voisin de la machine d'arrivée (par
    /// l'avant seulement, pour un conduit à sens unique). `None` si aucun conduit n'a de sens.
    fn routes(
        &self,
        grid: &(impl BlockAccess + ?Sized),
        members: &[IVec3],
        endpoints: &[Endpoint],
    ) -> Option<HashMap<IVec3, HashSet<IVec3>>> {
        let flow = |p: IVec3| self.flow_of(grid.block(p));
        if !members.iter().any(|&m| flow(m).is_some()) {
            return None;
        }
        let member_set: HashSet<IVec3> = members.iter().copied().collect();
        // Un objet peut passer du conduit `a` au conduit voisin `b`.
        let can_move = |a: IVec3, b: IVec3| {
            flow(a).is_none_or(|d| a + d == b) && flow(b).is_none_or(|d| b + d != a)
        };
        let conduit_of = |e: &Endpoint| e.pos + FACINGS[e.face as usize];
        let mut routes: HashMap<IVec3, HashSet<IVec3>> = HashMap::default();
        for from in endpoints {
            let start = conduit_of(from);
            // Entrer par le côté d'un conduit à sens unique est permis, pas par son avant.
            let entry = routes.entry(from.pos).or_default();
            if flow(start).is_some_and(|d| start + d == from.pos) {
                continue;
            }
            let mut reached: HashSet<IVec3> = HashSet::default();
            reached.insert(start);
            let mut queue = VecDeque::from([start]);
            while let Some(c) = queue.pop_front() {
                for d in FACINGS {
                    let next = c + d;
                    if member_set.contains(&next) && !reached.contains(&next) && can_move(c, next) {
                        reached.insert(next);
                        queue.push_back(next);
                    }
                }
            }
            for to in endpoints {
                let end = conduit_of(to);
                let exits = flow(end).is_none_or(|d| end + d == to.pos);
                if to.pos != from.pos && exits && reached.contains(&end) {
                    entry.insert(to.pos);
                }
            }
        }
        Some(routes)
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
                // La moitié droite d'un grand coffre échange par l'inventaire de la gauche.
                let n = crate::blocks::double_chest::storage_pos(grid, n);
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
    pub fn take_visual_transfers(&mut self) -> Vec<crate::VisualTransfer> {
        self.visuals.take()
    }

    pub fn tick(&mut self, grids: Grids) -> Vec<GridPos> {
        let mut woken = Vec::new();
        self.ticks += 1;
        let items_pass = self.ticks.is_multiple_of(ITEM_PERIOD);
        for net in self.nets.values() {
            let Some(cell) = grids.get(net.grid) else {
                continue;
            };
            let mut grid = cell.borrow_mut();
            let mut movements = Vec::new();
            let changed = match net.kind {
                NetworkKind::Energy => tick_energy(&mut *grid, net),
                NetworkKind::Fluid => tick_fluid(&mut *grid, net, &mut movements),
                NetworkKind::Kinetic => tick_kinetic(&mut *grid, net),
                NetworkKind::Items if items_pass => tick_items(
                    &mut *grid,
                    net,
                    ItemTables {
                        ports: &self.ports,
                        layouts: &self.layouts,
                        controllers: &self.controllers,
                    },
                    self.rules.as_ref(),
                    self.ticks / ITEM_PERIOD,
                    &mut movements,
                ),
                NetworkKind::Items => Vec::new(),
            };
            for movement in movements {
                self.visuals.record(net, &*grid, movement);
            }
            woken.extend(changed.into_iter().map(|p| GridPos::new(net.grid, p)));
        }
        woken
    }
}

// ----- Les objets -----

/// Une extrémité d'un réseau d'objets : les cases dont elle donne et celles qu'elle reçoit.
struct ItemPort {
    pos: IVec3,
    give: Vec<usize>,
    take: Vec<usize>,
    /// Le type de recette dont elle n'accepte que les ingrédients.
    accepts: Option<String>,
    /// Le seul objet qu'elle reçoit (d'après sa case-modèle), s'il y en a un.
    only: Option<String>,
    producer: bool,
    consumer: bool,
    buffer: bool,
    /// Un coffre à paliers : seules ses cases principales sont à la portée des tuyaux, et ses
    /// améliorations (piles plus grandes, néant) s'appliquent.
    layout: Option<crate::tech::storage::Layout>,
    /// Le contrôleur de stockage : il dépose dans les coffres qui lui sont reliés.
    controller: bool,
}

/// Ce que les réseaux d'objets savent de chaque bloc, par numéro de bloc.
#[derive(Clone, Copy)]
struct ItemTables<'a> {
    ports: &'a [Option<crate::registry::InventoryPorts>],
    layouts: &'a [Option<crate::tech::storage::Layout>],
    controllers: &'a [bool],
}

/// La place que cette extrémité offre à `count` objets de cette sorte.
fn room_at(
    grid: &(impl BlockAccess + ?Sized),
    target: &ItemPort,
    rules: Option<&(Arc<ItemRegistry>, Arc<MachineRecipes>)>,
    stack: &ItemStack,
    count: u32,
) -> u32 {
    use crate::tech::capabilities::MAX_STACK;
    let items = rules.map(|(items, _)| items.as_ref());
    if target.controller {
        let members = crate::tech::storage::controller_members(grid, target.pos);
        return crate::tech::storage::controller_room(
            grid,
            &members,
            items,
            &stack.item,
            stack.damage,
        )
        .min(count);
    }
    let Some(inventory) = grid
        .block_entity(target.pos)
        .and_then(|b| b.caps.as_deref())
        .and_then(|c| c.inventory.as_ref())
    else {
        return 0;
    };
    match target.layout {
        Some(layout) => {
            let policy = crate::tech::storage::Policy::of(&inventory.slots, layout, 0);
            crate::tech::storage::accept(
                &inventory.slots,
                layout,
                &policy,
                items,
                &stack.item,
                stack.damage,
                count,
            )
        }
        None => crate::tech::storage::room_in(
            &inventory.slots,
            &target.take,
            &stack.item,
            stack.damage,
            MAX_STACK,
        )
        .min(count),
    }
}

/// Range `count` objets (dont on a vérifié la place) chez cette extrémité : les piles du même
/// objet d'abord, puis les cases vides.
fn put_at(
    grid: &mut (impl BlockAccess + ?Sized),
    target: &ItemPort,
    rules: Option<&(Arc<ItemRegistry>, Arc<MachineRecipes>)>,
    stack: &ItemStack,
    count: u32,
) {
    use crate::tech::capabilities::MAX_STACK;
    let items = rules.map(|(items, _)| items.as_ref());
    if target.controller {
        let members = crate::tech::storage::controller_members(&*grid, target.pos);
        crate::tech::storage::controller_insert(
            grid,
            &members,
            items,
            &stack.item,
            stack.damage,
            count,
        );
        return;
    }
    let Some(inventory) = grid
        .block_entity_mut(target.pos)
        .and_then(|b| b.caps.as_mut())
        .and_then(|c| c.inventory.as_mut())
    else {
        return;
    };
    match target.layout {
        Some(layout) => {
            let policy = crate::tech::storage::Policy::of(&inventory.slots, layout, 0);
            crate::tech::storage::insert(
                &mut inventory.slots,
                layout,
                &policy,
                items,
                &stack.item,
                stack.damage,
                count,
                crate::tech::storage::Fill::Any,
            );
        }
        None => {
            crate::tech::storage::put_in(
                &mut inventory.slots,
                &target.take,
                &stack.item,
                stack.damage,
                count,
                MAX_STACK,
                crate::tech::storage::Fill::Any,
            );
        }
    }
}

/// Un passage d'un réseau d'objets : des producteurs vers les consommateurs, puis le surplus des
/// producteurs vers les tampons (les coffres), puis les tampons vers les consommateurs. Au plus
/// `rate` objets par passage ; `turn` fait tourner qui passe en premier (chacun son tour).
fn tick_items(
    grid: &mut (impl BlockAccess + ?Sized),
    net: &Network,
    tables: ItemTables,
    rules: Option<&(Arc<ItemRegistry>, Arc<MachineRecipes>)>,
    turn: u64,
    movements: &mut Vec<Movement>,
) -> Vec<IVec3> {
    if net.endpoints.len() < 2 {
        return Vec::new();
    }
    // Les extrémités (une machine peut toucher le réseau par plusieurs faces : une seule fois).
    let mut seen: HashSet<IVec3> = HashSet::default();
    let mut ends: Vec<ItemPort> = Vec::new();
    for e in &net.endpoints {
        if !seen.insert(e.pos) {
            continue;
        }
        let block = grid.block(e.pos);
        let Some(inventory) = grid
            .block_entity(e.pos)
            .and_then(|b| b.caps.as_deref())
            .and_then(|c| c.inventory.as_ref())
        else {
            continue;
        };
        let io = inventory.sides.io();
        let declared = tables.ports.get(block.0 as usize).and_then(Option::as_ref);
        let layout = tables.layouts.get(block.0 as usize).copied().flatten();
        let controller = tables
            .controllers
            .get(block.0 as usize)
            .copied()
            .unwrap_or(false);
        // Un coffre à paliers n'offre que ses cases principales : les emplacements d'amélioration
        // et les filtres ne regardent pas les tuyaux.
        let all: Vec<usize> = match layout {
            Some(l) => l.main_range().collect(),
            None => (0..inventory.slots.len()).collect(),
        };
        let (give, take) = match declared {
            Some(p) => (p.outputs.clone(), p.inputs.clone()),
            None => (all.clone(), all),
        };
        let give = if io == Io::In { Vec::new() } else { give };
        let take = if io == Io::Out { Vec::new() } else { take };
        ends.push(ItemPort {
            pos: e.pos,
            producer: !give.is_empty() && (declared.is_some() || io == Io::Out),
            consumer: !take.is_empty() && (declared.is_some() || io == Io::In),
            buffer: declared.is_none() && io == Io::Both,
            layout,
            controller,
            accepts: declared.and_then(|p| p.accepts.clone()),
            only: declared
                .and_then(|p| p.filter)
                .and_then(|f| inventory.slots.get(f))
                .and_then(|s| s.as_ref())
                .map(|s| s.item.clone()),
            give,
            take,
        });
    }
    let mut budget = net.rate.unwrap_or(ITEM_RATE);
    let mut changed: Vec<IVec3> = Vec::new();
    type Role = fn(&ItemPort) -> bool;
    let producers: Role = |p| p.producer;
    let consumers: Role = |p| p.consumer;
    let buffers: Role = |p| p.buffer;
    for (from, to) in [
        (producers, consumers),
        (producers, buffers),
        (buffers, consumers),
    ] {
        let givers: Vec<usize> = (0..ends.len()).filter(|&i| from(&ends[i])).collect();
        let targets: Vec<usize> = (0..ends.len()).filter(|&i| to(&ends[i])).collect();
        if givers.is_empty() || targets.is_empty() {
            continue;
        }
        for g in 0..givers.len() {
            let giver = &ends[givers[(g + turn as usize) % givers.len()]];
            for &slot in &giver.give {
                if budget == 0 {
                    break;
                }
                let Some(stack) = grid
                    .block_entity(giver.pos)
                    .and_then(|b| b.caps.as_deref())
                    .and_then(|c| c.inventory.as_ref())
                    .and_then(|i| i.slots.get(slot).cloned().flatten())
                else {
                    continue;
                };
                for t in 0..targets.len() {
                    let target = &ends[targets[(t + turn as usize) % targets.len()]];
                    if target.pos == giver.pos
                        || budget == 0
                        || stack.count == 0
                        || !net.allows(giver.pos, target.pos)
                    {
                        continue;
                    }
                    if target.only.as_ref().is_some_and(|only| *only != stack.item) {
                        continue;
                    }
                    // Une machine n'accepte que ce que ses recettes consomment.
                    if let (Some(kind), Some((items, recipes))) = (&target.accepts, rules)
                        && !items
                            .id(&stack.item)
                            .is_some_and(|id| recipes.accepts(kind, id))
                    {
                        continue;
                    }
                    let room = room_at(&*grid, target, rules, &stack, budget.min(stack.count));
                    if room == 0 {
                        continue;
                    }
                    // Retirer chez l'un, ranger chez l'autre.
                    if let Some(inv) = grid
                        .block_entity_mut(giver.pos)
                        .and_then(|b| b.caps.as_mut())
                        .and_then(|c| c.inventory.as_mut())
                        && let Some(Some(s)) = inv.slots.get_mut(slot)
                    {
                        s.count -= room.min(s.count);
                        if s.count == 0 {
                            inv.slots[slot] = None;
                        }
                    }
                    put_at(grid, target, rules, &stack, room);
                    movements.push(Movement {
                        from: giver.pos,
                        to: target.pos,
                        from_face: None,
                        to_face: None,
                        content: TransitContent::Item(stack.item.clone()),
                        amount: u64::from(room),
                    });
                    budget -= room;
                    changed.push(giver.pos);
                    changed.push(target.pos);
                    break;
                }
            }
        }
    }
    changed.sort_by_key(|p| p.to_array());
    changed.dedup();
    changed
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
    plan_traced(slots, rate, None)
}
fn plan_traced(
    slots: &[Slot],
    rate: u64,
    mut transfers: Option<&mut Vec<(usize, usize, u64)>>,
) -> Vec<i64> {
    let mut delta = vec![0i64; slots.len()];
    let pick = |io: Io| -> Vec<usize> { (0..slots.len()).filter(|&i| slots[i].io == io).collect() };
    let (sources, sinks, buffers) = (pick(Io::Out), pick(Io::In), pick(Io::Both));
    let mut src: Vec<u64> = sources.iter().map(|&i| slots[i].avail).collect();
    let mut snk: Vec<u64> = sinks.iter().map(|&i| slots[i].room).collect();
    let mut buf_room: Vec<u64> = buffers.iter().map(|&i| slots[i].room).collect();
    let mut buf_avail: Vec<u64> = buffers.iter().map(|&i| slots[i].avail).collect();
    let mut budget = rate;

    // Déplace `amount` des `from` vers les `to`, au prorata.
    let mut flow = |amount: u64,
                    from: (&[usize], &mut Vec<u64>),
                    to: (&[usize], &mut Vec<u64>),
                    delta: &mut Vec<i64>| {
        let taken = split(amount, from.1);
        let given = split(amount, to.1);
        if let Some(transfers) = transfers.as_deref_mut() {
            pair_flows(from.0, &taken, to.0, &given, transfers);
        }
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
                if let Some(transfers) = transfers {
                    transfers.push((buffers[h], buffers[l], amount));
                }
                delta[buffers[h]] -= amount as i64;
                delta[buffers[l]] += amount as i64;
            }
        }
    }
    delta
}

/// L'état d'un plan routé : ce que chaque extrémité peut encore donner et recevoir.
struct Routed {
    avail: Vec<u64>,
    room: Vec<u64>,
    delta: Vec<i64>,
    budget: u64,
    transfers: Vec<(usize, usize, u64)>,
    observe: bool,
}

impl Routed {
    /// Les `from` donnent vers les `to` : chaque source répartit son don au prorata de la place
    /// des destinations qui lui sont permises.
    fn flow(
        &mut self,
        slots: &[Slot],
        allows: &impl Fn(IVec3, IVec3) -> bool,
        from: &[usize],
        to: &[usize],
    ) {
        for &s in from {
            let targets: Vec<usize> = to
                .iter()
                .copied()
                .filter(|&t| allows(slots[s].pos, slots[t].pos))
                .collect();
            let rooms: Vec<u64> = targets.iter().map(|&t| self.room[t]).collect();
            let amount = self.avail[s].min(self.budget).min(rooms.iter().sum());
            for (&t, given) in targets.iter().zip(split(amount, &rooms)) {
                if self.observe
                    && given > 0
                    && self.transfers.len() < crate::tech::network_visual::MAX_VISUAL_TRANSFERS
                {
                    self.transfers.push((s, t, given));
                }
                self.delta[t] += given as i64;
                self.room[t] -= given;
                self.delta[s] -= given as i64;
                self.avail[s] -= given;
                self.budget -= given;
            }
        }
    }
}

/// Comme [`plan`], pour un réseau qui a des conduits à sens unique : chaque producteur ne sert que
/// les extrémités où ses flux peuvent aller (`allows(de, vers)`). Les sources passent dans
/// l'ordre ; les tampons ne s'équilibrent pas entre eux (leurs trajets sont à sens unique).
fn plan_routed(slots: &[Slot], rate: u64, allows: impl Fn(IVec3, IVec3) -> bool) -> Vec<i64> {
    plan_routed_traced(slots, rate, allows, None)
}
fn plan_routed_traced(
    slots: &[Slot],
    rate: u64,
    allows: impl Fn(IVec3, IVec3) -> bool,
    transfers: Option<&mut Vec<(usize, usize, u64)>>,
) -> Vec<i64> {
    let pick = |io: Io| -> Vec<usize> { (0..slots.len()).filter(|&i| slots[i].io == io).collect() };
    let (sources, sinks, buffers) = (pick(Io::Out), pick(Io::In), pick(Io::Both));
    let mut state = Routed {
        avail: slots.iter().map(|s| s.avail).collect(),
        room: slots.iter().map(|s| s.room).collect(),
        delta: vec![0; slots.len()],
        budget: rate,
        transfers: Vec::new(),
        observe: transfers.is_some(),
    };
    // Comme `plan` : producteurs → consommateurs, producteurs → tampons, tampons → consommateurs.
    state.flow(slots, &allows, &sources, &sinks);
    state.flow(slots, &allows, &sources, &buffers);
    state.flow(slots, &allows, &buffers, &sinks);
    // Les tampons s'équilibrent le long des trajets permis : le couple au plus grand écart.
    if state.budget > 0 {
        let best = buffers
            .iter()
            .flat_map(|&h| buffers.iter().map(move |&l| (h, l)))
            .filter(|&(h, l)| {
                h != l
                    && state.avail[h] > 0
                    && state.room[l] > 0
                    && allows(slots[h].pos, slots[l].pos)
                    && slots[h].level - slots[l].level > BALANCE_EPSILON
            })
            .max_by(|a, b| {
                let gap = |&(h, l): &(usize, usize)| slots[h].level - slots[l].level;
                gap(a).total_cmp(&gap(b))
            });
        if let Some((h, l)) = best {
            let gap = slots[h].level - slots[l].level;
            let half = (gap / 2.0 * slots[h].capacity.min(slots[l].capacity) as f64).ceil() as u64;
            let amount = state
                .budget
                .min(state.avail[h])
                .min(state.room[l])
                .min(half.max(1));
            if state.observe
                && state.transfers.len() < crate::tech::network_visual::MAX_VISUAL_TRANSFERS
            {
                state.transfers.push((h, l, amount));
            }
            state.delta[h] -= amount as i64;
            state.delta[l] += amount as i64;
        }
    }
    if let Some(transfers) = transfers {
        transfers.extend(state.transfers);
    }
    state.delta
}

fn tick_energy(grid: &mut (impl BlockAccess + ?Sized), net: &Network) -> Vec<IVec3> {
    if net.endpoints.len() < 2 {
        return Vec::new();
    }
    let rate = net.rate.map_or(ENERGY_RATE, u64::from);
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
                    store.clone().extract(Some(e.face), rate, true)
                },
                room: if io == Io::Out {
                    0
                } else {
                    store.clone().insert(Some(e.face), rate, true)
                },
            })
        })
        .collect();
    let delta = if net.routes.is_some() {
        plan_routed(&slots, rate, |a, b| net.allows(a, b))
    } else {
        plan(&slots, rate)
    };
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

fn tick_fluid(
    grid: &mut (impl BlockAccess + ?Sized),
    net: &Network,
    movements: &mut Vec<Movement>,
) -> Vec<IVec3> {
    if net.endpoints.len() < 2 {
        return Vec::new();
    }
    let rate = net.rate.map_or(FLUID_RATE, u64::from);
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
                    u64::from(
                        tank.clone()
                            .drain(Some(e.face), Some(&fluid), rate as u32, true),
                    )
                },
                room: if io == Io::Out {
                    0
                } else {
                    u64::from(tank.clone().fill(Some(e.face), &fluid, rate as u32, true))
                },
            })
        })
        .collect();
    let mut transfers = Vec::new();
    let delta = if net.routes.is_some() {
        plan_routed_traced(&slots, rate, |a, b| net.allows(a, b), Some(&mut transfers))
    } else {
        plan_traced(&slots, rate, Some(&mut transfers))
    };
    for (from, to, amount) in transfers {
        movements.push(Movement {
            from: slots[from].pos,
            to: slots[to].pos,
            from_face: Some(slots[from].face),
            to_face: Some(slots[to].face),
            content: TransitContent::Fluid(fluid.clone()),
            amount,
        });
    }
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

    use crate::items::ItemRegistry;
    use crate::streaming::ChunkStreamer;
    use crate::tech::capabilities::{Inventory, ItemStack};
    use crate::tech::crafting::MachineRecipes;
    use crate::tech::machines::Machines;
    use crate::tech::machines::tests::loaded_streamer;
    use std::sync::Arc;

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

    // ----- Les réseaux d'objets et les canaux -----

    const ITEM_BLOCKS: &str = r#"[
        (id: "test:belt", name: "Courroie", color: (0.5, 0.5, 0.5), conduit: Items, channel: "a"),
        (id: "test:belt_b", name: "Courroie B", color: (0.5, 0.5, 0.5), conduit: Items, channel: "b"),
        (id: "test:slow", name: "Courroie lente", color: (0.5, 0.5, 0.5), conduit: Items, channel: "a", throughput: 2),
        (id: "test:src", name: "Source", color: (0.5, 0.5, 0.5), ports: (outputs: [0])),
        (id: "test:sink", name: "Puits", color: (0.5, 0.5, 0.5), ports: (inputs: [0], accepts: "test:press")),
        (id: "test:machine", name: "Machine", color: (0.5, 0.5, 0.5), ports: (inputs: [0], outputs: [1])),
        (id: "test:crate", name: "Caisse", color: (0.5, 0.5, 0.5)),
        (id: "test:diode", name: "Diode", color: (0.5, 0.5, 0.5),
         boxes: [(from: (0, 0, 0), to: (16, 4, 16))], states: [Facing4], conduit: Energy, flow: true),
        (id: "test:valve", name: "Clapet", color: (0.5, 0.5, 0.5),
         boxes: [(from: (0, 0, 0), to: (16, 4, 16))], states: [Facing4], conduit: Fluid, flow: true),
        (id: "test:oneway", name: "Courroie à sens", color: (0.5, 0.5, 0.5),
         boxes: [(from: (0, 0, 0), to: (16, 4, 16))], states: [Facing4], conduit: Items, channel: "a", flow: true),
        (id: "test:sorter", name: "Trieur", color: (0.5, 0.5, 0.5), ports: (inputs: [1], filter: 0)),
    ]"#;

    impl Rig {
        /// Un banc avec les blocs de test des réseaux d'objets, et un type de recette `test:press`
        /// qui consomme du fer.
        fn items() -> Self {
            let mut registry = (*BlockRegistry::builtin()).clone();
            registry.add_source("test", ITEM_BLOCKS, "test").unwrap();
            let registry = Arc::new(registry);
            let items = Arc::new(ItemRegistry::builtin(&registry).unwrap());
            let mut recipes = MachineRecipes::default();
            recipes
                .add_source(
                    "test",
                    r#"(machines: {"press": [(inputs: [("core:iron_ingot", 1)], outputs: [("core:gold_ingot", 1)])]})"#,
                    "test",
                    &items,
                )
                .unwrap();
            let s = loaded_streamer();
            s.borrow_mut().replace_registry(registry.clone());
            let mut machines = Machines::with_natives(&registry);
            machines.set_item_rules(items, Arc::new(recipes));
            let rig = Self { s, machines };
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

        /// Pose un bloc à inventaire (créé avant les conduits, pour être vu comme extrémité).
        fn container(&mut self, pos: IVec3, key: &str, slots: usize) {
            self.place(pos, key);
            Grids::new(&self.s).write_or_create(GridPos::world(pos), |e| {
                e.caps_mut().inventory = Some(Inventory::new(slots));
            });
        }

        fn put(&self, pos: IVec3, slot: usize, item: &str, count: u32) {
            self.with_caps(pos, |c| {
                c.inventory.as_mut().unwrap().slots[slot] = Some(ItemStack {
                    item: item.into(),
                    count,
                    damage: 0,
                });
            });
        }

        fn count(&self, pos: IVec3, slot: usize) -> u32 {
            self.with_caps(pos, |c| {
                c.inventory.as_ref().unwrap().slots[slot]
                    .as_ref()
                    .map_or(0, |s| s.count)
            })
            .unwrap_or(0)
        }

        fn belts(&mut self, from: i32, to: i32, key: &str) {
            for x in from..=to {
                self.place(self.at(x, 0), key);
            }
        }
    }

    #[test]
    fn items_travel_from_a_source_to_a_sink_a_few_at_a_time() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 30);
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.belts(1, 5, "test:belt");
        // Un passage tous les 4 ticks, 8 objets au plus par passage.
        rig.tick(ITEM_PERIOD as usize);
        assert_eq!(rig.count(rig.at(6, 0), 0), 8, "un passage");
        rig.tick(ITEM_PERIOD as usize * 10);
        assert_eq!(rig.count(rig.at(6, 0), 0), 30, "tout est arrivé");
        assert_eq!(rig.count(rig.at(0, 0), 0), 0);
    }

    /// Une ligne de courroies à sens unique, de `from` à `to` (en X), tournées vers `facing`.
    fn oneway(rig: &mut Rig, from: i32, to: i32, facing: &str) {
        rig.belts(from, to, &format!("test:oneway[facing={facing}]"));
    }

    #[test]
    fn a_one_way_belt_carries_items_forward_only() {
        for (facing, arrives) in [("east", true), ("west", false)] {
            let mut rig = Rig::items();
            rig.container(rig.at(0, 0), "test:src", 1);
            rig.put(rig.at(0, 0), 0, "core:iron_ingot", 10);
            rig.container(rig.at(6, 0), "test:sink", 1);
            oneway(&mut rig, 1, 5, facing);
            rig.tick(ITEM_PERIOD as usize * 5);
            let got = rig.count(rig.at(6, 0), 0);
            assert_eq!(got > 0, arrives, "vers {facing} : {got}");
        }
    }

    #[test]
    fn a_one_way_belt_takes_items_from_the_side_but_gives_only_by_its_front() {
        let mut rig = Rig::items();
        // Une source sur le flanc de la courroie, un puits au bout, un autre sur l'autre flanc.
        rig.container(rig.at(3, 1), "test:src", 1);
        rig.put(rig.at(3, 1), 0, "core:iron_ingot", 10);
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.container(rig.at(3, -1), "test:sink", 1);
        oneway(&mut rig, 1, 5, "east");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(6, 0), 0), 10, "au bout");
        assert_eq!(rig.count(rig.at(3, -1), 0), 0, "pas par le flanc");
    }

    #[test]
    fn nothing_enters_a_one_way_belt_by_its_front_or_leaves_by_its_back() {
        let mut rig = Rig::items();
        // La source est devant la courroie, le puits derrière : rien ne passe.
        rig.container(rig.at(6, 0), "test:src", 1);
        rig.put(rig.at(6, 0), 0, "core:iron_ingot", 10);
        rig.container(rig.at(0, 0), "test:sink", 1);
        oneway(&mut rig, 1, 5, "east");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(0, 0), 0), 0);
        assert_eq!(rig.count(rig.at(6, 0), 0), 10);
    }

    #[test]
    fn two_one_way_belts_head_on_block_each_other() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 10);
        rig.container(rig.at(5, 0), "test:sink", 1);
        oneway(&mut rig, 1, 2, "east");
        oneway(&mut rig, 3, 4, "west");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(5, 0), 0), 0);
        // Retournée, la seconde moitié laisse passer.
        oneway(&mut rig, 3, 4, "east");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(5, 0), 0), 10);
    }

    #[test]
    fn a_diode_lets_energy_through_one_way_only() {
        for (facing, passes) in [("east", true), ("west", false)] {
            let mut rig = Rig::items();
            rig.place(rig.at(0, 0), "core:battery");
            rig.charge(rig.at(0, 0), 50_000);
            rig.place(rig.at(6, 0), "core:battery");
            rig.charge(rig.at(6, 0), 0);
            rig.belts(1, 5, &format!("test:diode[facing={facing}]"));
            rig.tick(30);
            let (full, empty) = (rig.energy(rig.at(0, 0)), rig.energy(rig.at(6, 0)));
            assert_eq!(empty > 0, passes, "vers {facing} : {empty}");
            assert_eq!(full < 50_000, passes, "vers {facing} : {full}");
        }
    }

    #[test]
    fn a_diode_keeps_a_lamp_lit_from_its_generator_but_not_the_other_way() {
        let mut rig = Rig::items();
        // Un panneau solaire, une diode vers la lampe : la lampe s'allume.
        rig.place(rig.at(0, 0), "core:solar_panel");
        rig.belts(1, 3, "test:diode[facing=east]");
        rig.place(rig.at(4, 0), "core:power_lamp");
        rig.tick(200);
        assert!(rig.is(rig.at(4, 0), "core:power_lamp_on"));
        // Retournée, la diode coupe la lampe de son panneau.
        let mut rig = Rig::items();
        rig.place(rig.at(0, 0), "core:solar_panel");
        rig.belts(1, 3, "test:diode[facing=west]");
        rig.place(rig.at(4, 0), "core:power_lamp");
        rig.tick(200);
        assert!(rig.is(rig.at(4, 0), "core:power_lamp"));
    }

    #[test]
    fn a_check_valve_lets_a_fluid_through_one_way_only() {
        for (facing, passes) in [("east", true), ("west", false)] {
            let mut rig = Rig::items();
            rig.place(rig.at(0, 0), "core:tank");
            rig.with_caps(rig.at(0, 0), |c| {
                let tank = &mut c.tanks[0];
                tank.fluid = Some("core:water".into());
                tank.amount = tank.capacity;
            });
            rig.place(rig.at(6, 0), "core:tank");
            rig.belts(1, 5, &format!("test:valve[facing={facing}]"));
            rig.tick(30);
            assert_eq!(
                rig.water(rig.at(6, 0)) > 0,
                passes,
                "vers {facing} : {} mB",
                rig.water(rig.at(6, 0))
            );
        }
    }

    #[test]
    fn a_throughput_limits_the_whole_network_to_its_slowest_conduit() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 40);
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.belts(1, 5, "test:belt");
        rig.place(rig.at(3, 0), "test:slow");
        rig.tick(ITEM_PERIOD as usize);
        assert_eq!(rig.count(rig.at(6, 0), 0), 2, "2 objets par passage");
    }

    #[test]
    fn conduits_of_different_channels_do_not_connect() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 10);
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.belts(1, 2, "test:belt");
        rig.belts(3, 5, "test:belt_b");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(6, 0), 0), 0, "deux réseaux séparés");
        assert_eq!(rig.count(rig.at(0, 0), 0), 10);
        // Relier : remplacer une courroie B par une courroie A les réunit.
        rig.belts(3, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(6, 0), 0), 10);
    }

    #[test]
    fn a_machine_only_accepts_what_its_recipes_consume() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 2);
        // La première case a du fer (accepté), la seconde de la pierre (refusée).
        rig.with_caps(rig.at(0, 0), |c| {
            c.inventory.as_mut().unwrap().slots.truncate(1);
        });
        rig.put(rig.at(0, 0), 0, "core:stone", 10);
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.belts(1, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(
            rig.count(rig.at(6, 0), 0),
            0,
            "la pierre n'est pas une entrée de recette"
        );
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 10);
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(6, 0), 0), 10);
    }

    #[test]
    fn a_filter_slot_lets_only_its_item_in() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:stone", 10);
        rig.container(rig.at(6, 0), "test:sorter", 2);
        rig.belts(1, 5, "test:belt");
        // Sans modèle, tout entre : la pierre passe.
        rig.tick(ITEM_PERIOD as usize * 3);
        assert_eq!(rig.count(rig.at(6, 0), 1), 10, "sans modèle, tout entre");
        // Avec un modèle de fer, la pierre reste chez la source.
        rig.put(rig.at(0, 0), 0, "core:stone", 10);
        rig.put(rig.at(6, 0), 0, "core:iron_ingot", 1);
        rig.put(rig.at(6, 0), 1, "core:iron_ingot", 0);
        rig.with_caps(rig.at(6, 0), |c| {
            c.inventory.as_mut().unwrap().slots[1] = None
        });
        rig.tick(ITEM_PERIOD as usize * 3);
        assert_eq!(rig.count(rig.at(6, 0), 1), 0, "la pierre est refusée");
        assert_eq!(rig.count(rig.at(0, 0), 0), 10);
        assert_eq!(rig.count(rig.at(6, 0), 0), 1, "le modèle reste intact");
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 10);
        rig.tick(ITEM_PERIOD as usize * 3);
        assert_eq!(rig.count(rig.at(6, 0), 1), 10);
    }

    #[test]
    fn a_machine_receives_in_its_inputs_and_gives_from_its_outputs() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 5);
        rig.container(rig.at(6, 0), "test:machine", 2);
        // Un produit fini attend dans la sortie, et une caisse attend les produits.
        rig.put(rig.at(6, 0), 1, "core:gold_ingot", 3);
        rig.container(rig.at(6, 1), "test:crate", 3);
        rig.belts(1, 5, "test:belt");
        rig.place(rig.at(5, 1), "test:belt");
        rig.tick(ITEM_PERIOD as usize * 10);
        // Le fer entre dans la case 0, jamais dans la sortie (case 1) ; l'or sort vers la caisse.
        assert_eq!(rig.count(rig.at(6, 0), 0), 5);
        assert_eq!(rig.count(rig.at(6, 0), 1), 0, "la sortie est vidée");
        let crate_gold: u32 = (0..3).map(|s| rig.count(rig.at(6, 1), s)).sum();
        assert_eq!(crate_gold, 3);
    }

    #[test]
    fn pipes_only_touch_the_main_slots_of_a_tiered_chest_and_honour_its_upgrades() {
        use crate::tech::storage::{CHEST_TIERS, Upgrade};
        let layout = CHEST_TIERS[2].layout;
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 200);
        rig.container(rig.at(6, 0), "core:iron_chest", layout.total());
        rig.put(rig.at(6, 0), layout.main, Upgrade::Stack2.key(), 1);
        // Un objet dans un emplacement de filtre : les tuyaux ne doivent pas le prendre.
        rig.put(rig.at(6, 0), layout.filter_range(0).start, "core:coal", 1);
        rig.belts(1, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 30);
        // Pile ×2 : 128 lingots dans la première case, 72 dans la suivante.
        assert_eq!(rig.count(rig.at(6, 0), 0), 128);
        assert_eq!(rig.count(rig.at(6, 0), 1), 72);
        assert_eq!(rig.count(rig.at(0, 0), 0), 0);
        // L'amélioration et le filtre sont intacts : pas écrasés, pas vidés.
        assert_eq!(rig.count(rig.at(6, 0), layout.main), 1);
        assert_eq!(rig.count(rig.at(6, 0), layout.filter_range(0).start), 1);
    }

    #[test]
    fn pipes_never_take_upgrades_or_filters_out_of_a_tiered_chest() {
        use crate::tech::storage::{CHEST_TIERS, Upgrade};
        let layout = CHEST_TIERS[2].layout;
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "core:iron_chest", layout.total());
        rig.put(rig.at(0, 0), 0, "core:stone", 5);
        rig.put(rig.at(0, 0), layout.main, Upgrade::Stack2.key(), 1);
        rig.put(rig.at(0, 0), layout.filter_range(0).start, "core:coal", 1);
        // Une machine (un consommateur) : elle tire ce que le coffre peut donner.
        rig.container(rig.at(6, 0), "test:machine", 2);
        rig.belts(1, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 10);
        let crate_items: Vec<String> = (0..2)
            .filter_map(|i| {
                rig.with_caps(rig.at(6, 0), |c| {
                    c.inventory.as_ref().unwrap().slots[i]
                        .as_ref()
                        .map(|s| s.item.clone())
                })
                .flatten()
            })
            .collect();
        assert_eq!(crate_items, vec!["core:stone".to_string()]);
        assert_eq!(
            rig.count(rig.at(0, 0), layout.main),
            1,
            "l'amélioration reste"
        );
        assert_eq!(rig.count(rig.at(0, 0), layout.filter_range(0).start), 1);
    }

    #[test]
    fn a_void_chest_destroys_filtered_items_that_a_pipe_brings() {
        use crate::tech::storage::{CHEST_TIERS, Upgrade};
        let layout = CHEST_TIERS[1].layout;
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:coal", 20);
        rig.container(rig.at(6, 0), "core:copper_chest", layout.total());
        rig.put(rig.at(6, 0), layout.main, Upgrade::Void.key(), 1);
        rig.put(rig.at(6, 0), layout.filter_range(0).start, "core:coal", 1);
        rig.belts(1, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 6);
        assert_eq!(rig.count(rig.at(0, 0), 0), 0, "la source est vidée");
        let stored: u32 = (0..layout.main).map(|i| rig.count(rig.at(6, 0), i)).sum();
        assert_eq!(stored, 0, "le charbon est détruit à l'entrée");
    }

    #[test]
    fn a_pipe_on_the_controller_deposits_into_the_chests_around_it() {
        use crate::tech::storage::CHEST_TIERS;
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 100);
        rig.container(rig.at(6, 0), "core:storage_controller", 0);
        // Deux coffres contre le contrôleur, un troisième contre le premier.
        rig.container(rig.at(7, 0), "core:chest", CHEST_TIERS[0].layout.total());
        rig.container(rig.at(6, 1), "core:chest", CHEST_TIERS[0].layout.total());
        rig.container(
            rig.at(7, 1),
            "core:copper_chest",
            CHEST_TIERS[1].layout.total(),
        );
        rig.belts(1, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 20);
        assert_eq!(rig.count(rig.at(0, 0), 0), 0);
        let total: u32 = [(7, 0), (6, 1), (7, 1)]
            .iter()
            .map(|&(x, z)| (0..27).map(|i| rig.count(rig.at(x, z), i)).sum::<u32>())
            .sum();
        assert_eq!(total, 100, "tout est dans les coffres reliés");
    }

    #[test]
    fn a_chest_is_a_buffer_between_producers_and_consumers() {
        let mut rig = Rig::items();
        rig.container(rig.at(0, 0), "test:src", 1);
        rig.put(rig.at(0, 0), 0, "core:iron_ingot", 12);
        rig.container(rig.at(4, 0), "test:crate", 3);
        rig.belts(1, 3, "test:belt");
        // Sans consommateur, la production se range dans la caisse.
        rig.tick(ITEM_PERIOD as usize * 5);
        let stored: u32 = (0..3).map(|s| rig.count(rig.at(4, 0), s)).sum();
        assert_eq!(stored, 12);
        // Un consommateur arrive : la caisse le sert.
        rig.container(rig.at(6, 0), "test:sink", 1);
        rig.belts(5, 5, "test:belt");
        rig.tick(ITEM_PERIOD as usize * 5);
        assert_eq!(rig.count(rig.at(6, 0), 0), 12);
    }
}
