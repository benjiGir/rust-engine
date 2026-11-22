//! La simulation des fluides : de l'eau et de la lave dont le **volume est conservé**.
//!
//! # L'idée
//!
//! Chaque cellule de fluide contient une **quantité** : 4096 = un bloc plein (on compte
//! en 4096e de bloc, des entiers, pour que les transferts soient exacts : ce qui quitte
//! une cellule arrive dans une autre, rien ne se perd par arrondi). Dix fois par seconde,
//! chaque cellule **active** fait trois choses, dans cet ordre :
//!
//! 1. **tomber** : elle verse dans la cellule du dessous tout ce qui y tient ;
//! 2. **s'étaler** : elle donne à chaque voisine horizontale moins remplie un quart de
//!    la différence (on tend vers un niveau plat) ;
//! 3. **remonter** : si elle est trop pleine, l'excédent monte d'une cellule.
//!
//! L'étape 3 fait la magie des **vases communicants**. Une cellule peut contenir un peu
//! plus qu'un bloc (`compressibility` : 2 % de plus par bloc de fluide au-dessus d'elle).
//! Au fond d'une colonne de 10 blocs, l'eau est « comprimée » ; si cette colonne est
//! reliée par le bas à une autre moins haute, la pression pousse l'excédent dans l'autre,
//! qui le fait remonter... jusqu'à ce que les deux surfaces soient au même niveau.
//! C'est l'algorithme « compressible » classique des jeux en 2D, étendu à la 3D.
//!
//! # Coût
//!
//! On ne calcule que les cellules **actives** : celles qui viennent de changer et leurs
//! voisines. Une cellule dont rien ne bouge s'**endort** (elle sort de la liste) ; un
//! lac au repos ne coûte rien. Chaque étape a un **budget** en millisecondes : s'il est
//! dépassé, le reste attend l'étape suivante (l'eau coule un peu moins vite, mais le
//! jeu ne saccade pas).
//!
//! # Les réservoirs
//!
//! L'eau des océans, des lacs et des rivières générés n'a pas de quantité enregistrée :
//! c'est un **réservoir**, plein et stable. Il ne se vide jamais (sinon creuser au bord de
//! la mer viderait l'océan cellule par cellule) : il remplit les trous voisins jusqu'à
//! « plein », puis se rendort. Le fluide posé par le joueur, lui, est fini.
//!
//! Un réservoir ne doit pourtant pas inonder tout ce qui est plat : l'eau qu'il alimente a une
//! **portée** ([`MAX_REACH`] cellules). La cellule voisine d'une source ne contient au plus que 7/8
//! de bloc, la suivante 6/8, et ainsi de suite jusqu'à 1/8 : un ruisseau qui s'amincit puis
//! s'arrête, comme dans Minecraft. En tombant, l'eau retrouve toute sa portée en bas (une cascade
//! repart). L'eau versée par le joueur n'a pas de portée : son volume est fini et se conserve.
//!
//! # Lecture puis écriture
//!
//! Pendant une étape, on **lit** les quantités du monde sans les modifier et on note
//! les transferts dans une table (`deltas`) ; on n'applique le tout qu'à la fin. Ainsi
//! l'ordre dans lequel on parcourt les cellules ne change rien au résultat.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use glam::IVec3;

use crate::block::BlockId;
use crate::fluids::FluidRegistry;
use crate::registry::{BlockRegistry, Shape};
use crate::streaming::ChunkStreamer;
use crate::world::World;

/// Un bloc plein, en unités de quantité.
pub const FULL: i32 = 4096;
/// En dessous, un flux n'est pas adouci (voir `smooth`).
const MIN_FLOW: i32 = 8;
/// Une cellule plus mince que ça (1/16 de bloc) ne s'étale plus : c'est la « tension
/// de surface » qui empêche une flaque de s'étendre à l'infini.
const SURFACE_TENSION: i32 = FULL / 16;
/// Au plus un bloc déplacé par flux et par étape.
const MAX_SPEED: i32 = FULL;
/// Deux voisines dont les niveaux diffèrent de moins que ça (1 % de bloc) sont à
/// l'équilibre : on ne les fait plus échanger. Sans cette tolérance, l'eau passerait des
/// minutes à corriger des écarts invisibles avant de s'endormir.
const LEVEL_TOLERANCE: i32 = FULL / 100;
/// Un flux vertical plus petit que ça, entre deux cellules pleines, est ignoré (même raison).
const PRESSURE_TOLERANCE: i32 = 4;
/// Un changement plus petit que ça ne réveille pas les voisins.
const WAKE_THRESHOLD: i32 = 2;

/// La portée d'une source qui coule : l'eau d'un réservoir s'étale au plus sur ce nombre de
/// cellules, en s'amincissant (comme un ruisseau), au lieu d'inonder tout ce qui est plat.
/// Une cellule à `k` de portée restante ne contient au plus que `k / (MAX_REACH + 1)` de bloc.
const MAX_REACH: u8 = 7;

const SIDES: [IVec3; 4] = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];
const NEIGHBOURS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Y,
    IVec3::NEG_Y,
    IVec3::Z,
    IVec3::NEG_Z,
];

/// Ce qu'une étape accumule : les transferts, les cellules à figer, les portées à noter.
type FlowOut<'a> = (
    &'a mut HashMap<IVec3, (usize, i32)>,
    &'a mut Vec<IVec3>,
    &'a mut Vec<(IVec3, u8)>,
);

/// Un fluide, tel que la simulation le voit.
#[derive(Debug, Clone)]
pub struct FluidKind {
    pub key: String,
    pub block: BlockId,
    /// Surpression admise par bloc de profondeur, en unités de quantité.
    pub compress: i32,
    /// Une mise à jour toutes les `every` étapes : la lave (visqueuse) bouge 6 fois moins
    /// souvent que l'eau.
    pub every: u64,
    pub infinite: bool,
    pub temperature: u32,
    pub density: i32,
    pub damage: f32,
}

/// Ce qu'il y a dans une cellule, pour la simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    /// Un bloc plein (ou hors du monde chargé) : le fluide ne passe pas.
    Solid,
    /// De l'air (ou une plante que le fluide emportera).
    Empty,
    Fluid {
        kind: usize,
        mass: i32,
        reservoir: bool,
    },
}

/// Le bilan d'une étape.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FluidReport {
    /// Cellules calculées pendant cette étape.
    pub processed: usize,
    /// Cellules encore actives après l'étape (en attente pour la suivante).
    pub active: usize,
    /// Cellules modifiées.
    pub changed: usize,
    pub duration: Duration,
}

pub struct FluidSim {
    kinds: Vec<FluidKind>,
    by_block: HashMap<BlockId, usize>,
    /// Ce que devient le fluide le plus chaud au contact d'un autre (lave + eau).
    rock: Option<BlockId>,
    /// Les cellules actives, dans l'ordre où les traiter, et le même ensemble pour savoir
    /// vite si une cellule y est déjà.
    queue: VecDeque<IVec3>,
    queued: HashSet<IVec3>,
    step: u64,
    /// La portée restante des cellules alimentées par un réservoir (voir [`MAX_REACH`]). Les
    /// autres (de l'eau versée par le joueur) n'en ont pas : leur volume est fini, il se
    /// conserve et s'étale jusqu'à l'équilibre.
    reach: HashMap<IVec3, u8>,
}

/// La quantité qui devrait se trouver dans la cellule du **bas** quand deux cellules
/// superposées se partagent `total` : tout, si ça tient dans un bloc ; sinon un bloc, plus
/// une part de surpression, d'autant plus grande que le total est grand.
fn stable_below(total: i32, compress: i32) -> i32 {
    if total <= FULL {
        FULL
    } else if total < 2 * FULL + compress {
        (FULL * FULL + total * compress) / (FULL + compress)
    } else {
        (total + compress) / 2
    }
}

/// Les gros flux sont divisés par deux : l'eau se déplace en douceur au lieu d'osciller.
fn smooth(flow: i32) -> i32 {
    if flow > MIN_FLOW { flow / 2 } else { flow }
}

impl FluidSim {
    pub fn new(blocks: &BlockRegistry, fluids: &FluidRegistry) -> Self {
        let mut kinds = Vec::new();
        let mut by_block = HashMap::new();
        for def in fluids.iter() {
            let Some(block) = def.block.as_deref().and_then(|key| blocks.id(key)) else {
                continue;
            };
            by_block.insert(block, kinds.len());
            kinds.push(FluidKind {
                key: def.id.clone(),
                block,
                compress: (def.compressibility * FULL as f32).round().max(1.0) as i32,
                every: (def.viscosity as u64 / 5).max(1),
                infinite: def.infinite,
                temperature: def.temperature,
                density: def.density,
                damage: def.damage,
            });
        }
        Self {
            kinds,
            by_block,
            rock: blocks.id("core:volcanic_rock"),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            step: 0,
            reach: HashMap::new(),
        }
    }

    pub fn kinds(&self) -> &[FluidKind] {
        &self.kinds
    }

    /// Le fluide d'un bloc, s'il en est un.
    pub fn kind_of(&self, block: BlockId) -> Option<&FluidKind> {
        self.by_block.get(&block).map(|&i| &self.kinds[i])
    }

    /// Le nombre de cellules actives.
    pub fn active(&self) -> usize {
        self.queue.len()
    }

    fn push(&mut self, pos: IVec3) {
        if self.queued.insert(pos) {
            self.queue.push_back(pos);
        }
    }

    /// Quelque chose a changé en `pos` (un bloc cassé, du fluide versé) : la cellule et
    /// ses voisines se réveillent.
    pub fn wake(&mut self, pos: IVec3) {
        self.push(pos);
        for o in NEIGHBOURS {
            self.push(pos + o);
        }
    }

    fn cell(&self, s: &ChunkStreamer, pos: IVec3) -> Cell {
        if !s.world().block_y_range().contains(&pos.y) || !s.is_generated(World::split(pos).0) {
            return Cell::Solid;
        }
        let block = s.world().block(pos);
        if let Some(&kind) = self.by_block.get(&block) {
            return match s.fluid_amount(pos) {
                Some(mass) => Cell::Fluid {
                    kind,
                    mass: mass as i32,
                    reservoir: false,
                },
                None => Cell::Fluid {
                    kind,
                    mass: FULL,
                    reservoir: true,
                },
            };
        }
        let def = s.world().registry().def(block);
        if block.is_air() || (!def.solid && def.shape != Shape::Cube && !def.liquid) {
            Cell::Empty
        } else {
            Cell::Solid
        }
    }

    /// Une étape (à appeler 10 fois par seconde).
    pub fn step(&mut self, s: &mut ChunkStreamer, budget: Duration) -> FluidReport {
        let _span = engine_trace::span("fluides");
        let start = Instant::now();
        self.step += 1;
        // Les transferts de l'étape : position → (fluide, variation de quantité).
        let mut deltas: HashMap<IVec3, (usize, i32)> = HashMap::new();
        let mut later: Vec<IVec3> = Vec::new();
        let mut solidify: Vec<IVec3> = Vec::new();
        // Les cellules qui ont donné quelque chose : elles restent éveillées, même si
        // elles ont reçu autant (une cellule de passage dans un tube ne change pas de
        // quantité, mais l'eau la traverse : elle ne doit pas s'endormir).
        let mut moving: Vec<IVec3> = Vec::new();
        let mut reach_new: Vec<(IVec3, u8)> = Vec::new();
        let mut processed = 0;
        while let Some(pos) = self.queue.pop_front() {
            // Le budget, vérifié toutes les 64 cellules (lire l'horloge a un coût).
            if processed % 64 == 0 && start.elapsed() > budget {
                self.queue.push_front(pos);
                break;
            }
            self.queued.remove(&pos);
            let Cell::Fluid {
                kind,
                mass,
                reservoir,
            } = self.cell(s, pos)
            else {
                self.reach.remove(&pos);
                continue;
            };
            if !self.step.is_multiple_of(self.kinds[kind].every) {
                later.push(pos); // Un fluide visqueux : son tour viendra.
                continue;
            }
            processed += 1;
            let reach = if reservoir {
                Some(MAX_REACH + 1)
            } else {
                self.reach.get(&pos).copied()
            };
            if self.flow(
                s,
                pos,
                (kind, mass, reservoir, reach),
                (&mut deltas, &mut solidify, &mut reach_new),
            ) {
                moving.push(pos);
            }
        }
        for pos in later {
            self.push(pos);
        }
        for (pos, reach) in reach_new {
            let entry = self.reach.entry(pos).or_insert(0);
            *entry = (*entry).max(reach);
        }

        // Appliquer les transferts.
        let mut changed = 0;
        let mut positions: Vec<(IVec3, (usize, i32))> = deltas.into_iter().collect();
        // Un ordre fixe : le résultat ne dépend pas de l'ordre d'une `HashMap`.
        positions.sort_by_key(|(p, _)| p.to_array());
        for (pos, (kind, delta)) in positions {
            if delta == 0 {
                continue;
            }
            let block = self.kinds[kind].block;
            let current = match self.cell(s, pos) {
                Cell::Fluid {
                    reservoir: true, ..
                } => continue, // Un réservoir ne change pas.
                Cell::Fluid { mass, kind: k, .. } if k == kind => mass,
                Cell::Empty => 0,
                // Un autre fluide est arrivé là entre-temps : on renonce.
                _ => continue,
            };
            let mass = current + delta;
            let done = if mass <= 0 {
                s.set_fluid(pos, BlockId::AIR, None)
            } else {
                s.set_fluid(pos, block, Some(mass.min(u16::MAX as i32) as u16))
            };
            if done {
                changed += 1;
                if delta.abs() >= WAKE_THRESHOLD {
                    self.wake(pos);
                }
            }
        }
        for pos in moving {
            self.wake(pos);
        }
        if self.kinds.iter().any(|k| k.infinite) {
            self.make_sources(s);
        }
        // Deux fluides qui se touchent : le plus chaud fige (la lave devient roche).
        for pos in solidify {
            if let Cell::Fluid { mass, .. } = self.cell(s, pos) {
                let block = if mass >= FULL / 2 {
                    self.rock.unwrap_or(BlockId::AIR)
                } else {
                    BlockId::AIR
                };
                if s.set_fluid(pos, block, None) {
                    self.wake(pos);
                }
            }
        }
        FluidReport {
            processed,
            active: self.queue.len(),
            changed,
            duration: start.elapsed(),
        }
    }

    /// Les flux d'une cellule : tomber, s'étaler, remonter. Renvoie `true` si elle a
    /// donné quelque chose.
    fn flow(
        &self,
        s: &ChunkStreamer,
        pos: IVec3,
        (kind, mass, reservoir, reach): (usize, i32, bool, Option<u8>),
        out: FlowOut,
    ) -> bool {
        let (deltas, solidify, reach_new) = out;
        let compress = self.kinds[kind].compress;
        let mut remaining = mass;
        // `Cell` : la fermeture `transfer` la modifie pendant qu'on la lit ailleurs.
        let gave = std::cell::Cell::new(false);
        // Un transfert de `amount` de cette cellule vers `to`. Un réservoir donne sans
        // perdre ; il ne reçoit jamais.
        let mut transfer = |to: IVec3, amount: i32, remaining: &mut i32| {
            if amount <= 0 {
                return;
            }
            gave.set(true);
            deltas.entry(to).or_insert((kind, 0)).1 += amount;
            if !reservoir {
                deltas.entry(pos).or_insert((kind, 0)).1 -= amount;
                *remaining -= amount;
            }
        };
        // Ce que voit la cellule autour d'elle.
        let mut neighbour = |offset: IVec3| -> Option<(i32, bool)> {
            match self.cell(s, pos + offset) {
                Cell::Solid => None,
                Cell::Empty => Some((0, false)),
                Cell::Fluid {
                    kind: k,
                    mass,
                    reservoir,
                } if k == kind => Some((mass, reservoir)),
                Cell::Fluid { kind: other, .. } => {
                    // Un autre fluide : le plus chaud des deux fige.
                    let hot = if self.kinds[kind].temperature >= self.kinds[other].temperature {
                        pos
                    } else {
                        pos + offset
                    };
                    solidify.push(hot);
                    None
                }
            }
        };

        // 1. Tomber.
        if let Some((below, below_reservoir)) = neighbour(IVec3::NEG_Y)
            && !below_reservoir
        {
            let wanted = if reservoir {
                FULL - below // Un réservoir ne fait que remplir jusqu'à « plein ».
            } else {
                stable_below(remaining + below, compress) - below
            };
            let flow = smooth(wanted).clamp(0, MAX_SPEED.min(remaining));
            // Dans du vide, tout tombe ; entre deux cellules d'eau, un écart de pression
            // minuscule ne vaut pas un échange.
            if below == 0 || flow >= PRESSURE_TOLERANCE {
                transfer(pos - IVec3::Y, flow, &mut remaining);
                // L'eau qui tombe retrouve toute sa portée en bas : une cascade repart.
                if flow > 0 && reach.is_some() {
                    reach_new.push((pos - IVec3::Y, MAX_REACH));
                }
            }
        }
        if remaining <= 0 {
            return gave.get();
        }

        // 2. S'étaler, vers chaque voisine horizontale moins remplie.
        // Une eau alimentée par un réservoir s'amincit et s'arrête après `MAX_REACH` cellules.
        let spreads = reach.is_none_or(|k| k > 1);
        if (mass > SURFACE_TENSION || reservoir) && spreads {
            for side in SIDES {
                let Some((other, other_reservoir)) = neighbour(side) else {
                    continue;
                };
                if other_reservoir {
                    continue;
                }
                // Un réservoir remplit jusqu'à plein ; une cellule finie, vers l'équilibre.
                // Entre deux cellules pleines, c'est la pression qui pousse, par petits
                // écarts : on garde la tolérance fine. Ailleurs (une surface libre), 1 %.
                let tolerance = if mass > FULL || other >= FULL {
                    PRESSURE_TOLERANCE
                } else {
                    LEVEL_TOLERANCE
                };
                if mass - other <= tolerance {
                    continue;
                }
                // Un quart de l'écart vers chaque voisine : même si les quatre sont plus
                // basses, la cellule ne descend pas sous leur moyenne (pas d'oscillation),
                // inutile d'adoucir davantage.
                let target = if reservoir {
                    (FULL - other) / 2
                } else {
                    (mass - other) / 4
                };
                let mut flow = target.clamp(0, remaining);
                if let Some(k) = reach {
                    // La cellule voisine ne dépasse pas la hauteur que sa portée permet.
                    let next = (k - 1).max(self.reach.get(&(pos + side)).copied().unwrap_or(0));
                    let cap = FULL * i32::from(next) / i32::from(MAX_REACH + 1);
                    flow = flow.min((cap - other).max(0));
                    if flow > 0 {
                        reach_new.push((pos + side, k - 1));
                    }
                }
                transfer(pos + side, flow, &mut remaining);
                if remaining <= 0 {
                    return gave.get();
                }
            }
        }

        // 3. Remonter : l'excédent sous pression monte (jamais depuis un réservoir).
        if !reservoir
            && let Some((above, above_reservoir)) = neighbour(IVec3::Y)
            && !above_reservoir
        {
            let flow = smooth(remaining - stable_below(remaining + above, compress))
                .clamp(0, MAX_SPEED.min(remaining));
            if flow >= PRESSURE_TOLERANCE {
                transfer(pos + IVec3::Y, flow, &mut remaining);
            }
        }
        gave.get()
    }

    /// La règle de source infinie (si un fluide l'active) : une cellule finie posée sur
    /// un sol, entre au moins deux réservoirs du même fluide, devient un réservoir.
    fn make_sources(&mut self, s: &mut ChunkStreamer) {
        let candidates: Vec<IVec3> = self.queue.iter().copied().collect();
        for pos in candidates {
            let Cell::Fluid {
                kind,
                reservoir: false,
                ..
            } = self.cell(s, pos)
            else {
                continue;
            };
            if !self.kinds[kind].infinite {
                continue;
            }
            let sources = SIDES
                .iter()
                .filter(|&&o| matches!(self.cell(s, pos + o), Cell::Fluid { kind: k, reservoir: true, .. } if k == kind))
                .count();
            let floor = !matches!(self.cell(s, pos - IVec3::Y), Cell::Empty);
            if sources >= 2 && floor {
                s.set_fluid(pos, self.kinds[kind].block, None);
            }
        }
    }

    /// Prend le fluide d'une cellule (un seau qu'on remplit) : renvoie le fluide et la
    /// quantité prise (au plus un bloc). La cellule devient de l'air ; un réservoir voisin
    /// (la mer) la remplira de nouveau.
    pub fn take(&mut self, s: &mut ChunkStreamer, pos: IVec3) -> Option<(usize, i32)> {
        let Cell::Fluid { kind, mass, .. } = self.cell(s, pos) else {
            return None;
        };
        if !s.set_fluid(pos, BlockId::AIR, None) {
            return None;
        }
        self.wake(pos);
        Some((kind, mass.min(FULL)))
    }

    /// Verse `amount` du fluide `kind` dans une cellule (vide, ou du même fluide fini),
    /// sans dépasser deux blocs : renvoie ce qui a été versé.
    pub fn pour(&mut self, s: &mut ChunkStreamer, pos: IVec3, kind: usize, amount: i32) -> i32 {
        let existing = match self.cell(s, pos) {
            Cell::Empty => 0,
            Cell::Fluid {
                kind: k,
                mass,
                reservoir: false,
            } if k == kind => mass,
            _ => return 0,
        };
        let poured = amount.min(2 * FULL - existing).max(0);
        if poured > 0
            && s.set_fluid(
                pos,
                self.kinds[kind].block,
                Some((existing + poured) as u16),
            )
        {
            self.wake(pos);
            poured
        } else {
            0
        }
    }

    /// La quantité totale de fluide finie d'un fluide dans une boîte (pour les tests et
    /// les mesures), en unités.
    pub fn total(&self, s: &ChunkStreamer, kind: usize, min: IVec3, max: IVec3) -> i64 {
        let mut total = 0i64;
        for x in min.x..=max.x {
            for y in min.y..=max.y {
                for z in min.z..=max.z {
                    if let Cell::Fluid { kind: k, mass, .. } = self.cell(s, IVec3::new(x, y, z))
                        && k == kind
                    {
                        total += mass as i64;
                    }
                }
            }
        }
        total
    }
}

#[cfg(test)]
mod tests;
