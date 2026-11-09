//! Mesures des fluides : `cargo bench -p engine_voxel --bench fluids`.
//!
//! Deux scénarios, dans un monde en plein ciel :
//! - **un grand lac qui se vide** : 24 × 24 × 3 blocs d'eau (finie) dans un bassin,
//!   dont le fond s'ouvre sur une grande salle plus bas ;
//! - **une grotte inondée** : une salle vide de 30 × 6 × 30, dont un mur s'ouvre sur un
//!   réservoir (une rangée d'eau qui ne s'épuise pas, comme la mer).
//!
//! Le second est aussi joué avec un modèle « à niveaux », comme les fluides du genre
//! (8 niveaux fixes, des sources infinies, l'eau coule à 7 blocs au plus de sa source),
//! pour comparer les coûts.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, ChunkStreamer, FLUID_FULL, FluidRegistry, FluidSim, StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

fn world() -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: 3,
        ..StreamingConfig::default()
    };
    let mut s = ChunkStreamer::new(TerrainGenerator::new(3), config);
    loop {
        s.update(Vec3::new(16.0, 120.0, 16.0));
        while s.next_event().is_some() {}
        if s.is_idle() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    s
}

/// Une boîte creuse de pierre : murs, sol et plafond autour de l'intérieur `min..=max`.
fn hollow_box(s: &mut ChunkStreamer, min: IVec3, max: IVec3) {
    let stone = s.world().registry().id("core:stone").unwrap();
    for x in min.x - 1..=max.x + 1 {
        for y in min.y - 1..=max.y + 1 {
            for z in min.z - 1..=max.z + 1 {
                let inside = (min.x..=max.x).contains(&x)
                    && (min.y..=max.y).contains(&y)
                    && (min.z..=max.z).contains(&z);
                s.set_block_raw(
                    IVec3::new(x, y, z),
                    if inside { BlockId::AIR } else { stone },
                );
            }
        }
    }
}

struct Run {
    /// L'étape où l'essentiel est fait (le lac vidé à 95 %, la grotte remplie à 95 %).
    done_at: Option<usize>,
    steps: usize,
    total: Duration,
    max_step: Duration,
    cells: usize,
}

/// Fait tourner la simulation jusqu'à ce qu'elle dorme (ou `limit` étapes).
fn run(
    s: &mut ChunkStreamer,
    sim: &mut FluidSim,
    limit: usize,
    done: impl Fn(&ChunkStreamer, &FluidSim) -> bool,
) -> Run {
    let mut run = Run {
        done_at: None,
        steps: 0,
        total: Duration::ZERO,
        max_step: Duration::ZERO,
        cells: 0,
    };
    while run.steps < limit && sim.active() > 0 {
        if run.done_at.is_none() && done(s, sim) {
            run.done_at = Some(run.steps);
        }
        let report = sim.step(s, Duration::from_secs(1));
        run.steps += 1;
        run.total += report.duration;
        run.max_step = run.max_step.max(report.duration);
        run.cells += report.processed;
    }
    run
}

fn print(name: &str, run: &Run) {
    let per_cell = run.total.as_secs_f64() * 1e9 / run.cells.max(1) as f64;
    let done = run.done_at.map_or("jamais".to_string(), |s| {
        format!("{:.1} s", s as f64 / 10.0)
    });
    println!(
        "{name} : essentiel fait en {done} de jeu ; endormi après {} étapes ({:.1} s de jeu à 10 Hz), {:.0} ms de calcul au total, pire étape {:.2} ms, \
         {} cellules calculées ({:.0} par étape, {per_cell:.0} ns par cellule)",
        run.steps,
        run.steps as f64 / 10.0,
        run.total.as_secs_f64() * 1000.0,
        run.max_step.as_secs_f64() * 1000.0,
        run.cells,
        run.cells as f64 / run.steps.max(1) as f64,
    );
}

fn lake(s: &mut ChunkStreamer) -> Run {
    let water = s.world().registry().id("core:water").unwrap();
    let mut sim = FluidSim::new(s.world().registry(), &FluidRegistry::builtin());
    // La grande salle du bas, puis le bassin du lac juste au-dessus.
    hollow_box(s, IVec3::new(-10, 100, -10), IVec3::new(40, 106, 40));
    hollow_box(s, IVec3::new(0, 108, 0), IVec3::new(23, 112, 23));
    for x in 0..24 {
        for z in 0..24 {
            for y in 108..111 {
                s.set_fluid(IVec3::new(x, y, z), water, Some(FLUID_FULL as u16));
            }
        }
    }
    // On ouvre le fond (2 × 2) : le plafond de la salle et le sol du bassin.
    for (x, z) in [(11, 11), (12, 11), (11, 12), (12, 12)] {
        s.set_block_raw(IVec3::new(x, 107, z), BlockId::AIR);
        sim.wake(IVec3::new(x, 107, z));
    }
    // Fait : il reste moins de 5 % de l'eau dans le bassin du haut.
    let start = (24 * 24 * 3 * FLUID_FULL) as i64;
    run(s, &mut sim, 6000, move |s, sim| {
        sim.total(s, 0, IVec3::new(0, 108, 0), IVec3::new(23, 112, 23)) * 20 < start
    })
}

fn cave(s: &mut ChunkStreamer) -> Run {
    let water = s.world().registry().id("core:water").unwrap();
    let mut sim = FluidSim::new(s.world().registry(), &FluidRegistry::builtin());
    // La boîte inclut la rangée x = -41 : le réservoir est enfermé du côté extérieur.
    hollow_box(s, IVec3::new(-41, 120, -40), IVec3::new(-11, 125, -11));
    // Un mur de réservoir (la mer) le long de x = -41, ouvert sur la salle.
    for z in -40..=-11 {
        for y in 120..=125 {
            s.set_block_raw(IVec3::new(-41, y, z), water);
            sim.wake(IVec3::new(-40, y, z));
        }
    }
    // Fait : la salle (30 × 6 × 30) pleine à 95 %.
    let volume = (30 * 6 * 30 * FLUID_FULL) as i64;
    run(s, &mut sim, 6000, move |s, sim| {
        sim.total(s, 0, IVec3::new(-40, 120, -40), IVec3::new(-11, 125, -11)) * 20 > volume * 19
    })
}

/// Le même inondation avec un modèle à niveaux du genre : une source (niveau 8) étend
/// de l'eau courante de niveau 7, 6, ... jusqu'à 1 ; l'eau qui tombe repart à 8.
/// Pas de volume : la source ne s'épuise jamais, l'eau courante s'arrête à 7 blocs.
fn cave_with_levels(s: &mut ChunkStreamer) -> Run {
    let water = s.world().registry().id("core:water").unwrap();
    hollow_box(s, IVec3::new(-41, 130, 10), IVec3::new(-11, 135, 39));
    let mut level = std::collections::HashMap::<IVec3, i32>::new();
    let mut active: VecDeque<IVec3> = VecDeque::new();
    let mut queued: HashSet<IVec3> = HashSet::new();
    for z in 10..=39 {
        for y in 130..=135 {
            let p = IVec3::new(-41, y, z);
            s.set_block_raw(p, water);
            level.insert(p, 8);
            active.push_back(IVec3::new(-40, y, z));
        }
    }
    let empty = |s: &ChunkStreamer, p: IVec3| s.world().block(p).is_air();
    let mut run = Run {
        done_at: None,
        steps: 0,
        total: Duration::ZERO,
        max_step: Duration::ZERO,
        cells: 0,
    };
    while !active.is_empty() && run.steps < 6000 {
        let start = Instant::now();
        let batch: Vec<IVec3> = active.drain(..).collect();
        queued.clear();
        let mut changes = Vec::new();
        for p in batch {
            run.cells += 1;
            if level.get(&p) == Some(&8) && s.world().block(p + IVec3::NEG_X) == water && p.x == -41
            {
                continue; // une source
            }
            // Le niveau voulu : 8 sous de l'eau, sinon le plus haut voisin moins 1.
            let above = level.get(&(p + IVec3::Y)).copied().unwrap_or(0);
            let side = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
                .iter()
                .filter_map(|o| level.get(&(p + *o)).copied())
                .max()
                .unwrap_or(0);
            let wanted = if above > 0 { 8 } else { (side - 1).max(0) };
            let current = level.get(&p).copied().unwrap_or(0);
            if wanted != current && (wanted == 0 || current > 0 || empty(s, p)) {
                changes.push((p, wanted));
            }
        }
        for (p, l) in changes {
            if l == 0 {
                level.remove(&p);
                s.set_fluid(p, BlockId::AIR, None);
            } else {
                level.insert(p, l);
                s.set_fluid(p, water, Some((l * FLUID_FULL / 8) as u16));
            }
            for o in [
                IVec3::X,
                IVec3::NEG_X,
                IVec3::Y,
                IVec3::NEG_Y,
                IVec3::Z,
                IVec3::NEG_Z,
            ] {
                let n = p + o;
                let fits = empty(s, n) || level.contains_key(&n);
                if fits && queued.insert(n) {
                    active.push_back(n);
                }
            }
        }
        let elapsed = start.elapsed();
        run.steps += 1;
        run.total += elapsed;
        run.max_step = run.max_step.max(elapsed);
    }
    run
}

fn main() {
    let mut s = world();
    print("lac qui se vide (volume conservé)", &lake(&mut s));
    print("grotte inondée (volume conservé)", &cave(&mut s));
    print(
        "grotte inondée (niveaux, modèle du genre)",
        &cave_with_levels(&mut s),
    );
}
