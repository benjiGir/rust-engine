//! Les tests du prototype macro v2 (lots 2 et 3) : géographie et hydrologie, avant tout bloc.
//!
//! Le passage du lot 3 est **bloquant** : déterminisme (ordre, threads, caches), raccords entre
//! régions (bords, coins de quatre régions, coordonnées négatives), acyclicité et terminaison
//! bornée, profils d'eau sans remontée, confluences et lacs cohérents, coût borné.
//! Voir `docs/plan-generation-rendu.md`.

use std::collections::HashSet;

use engine_voxel::{
    ACCUMULATION_CAP, BASIN_FINE, BASIN_SIZE, BasinFlow, CACHE_TABLES, FINE_SIZE, FineCell,
    MAX_BASIN_CELLS, MAX_BASIN_HOPS, MAX_FINE_PATH, MacroSample, MacroWorld, SEA_LEVEL,
    STREAM_MIN_ACCUMULATION, SinkKind, WORLD_HEIGHT_CHUNKS,
};

const SEEDS: [u32; 5] = [7, 42, 1337, 20260, 987_654_321];

/// Un petit générateur de nombres pour les échantillons : déterministe, sans dépendance.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 24
    }

    /// Un entier dans `[-range, range)`.
    fn coord(&mut self, range: i32) -> i32 {
        (self.next() % (2 * range as u64)) as i32 - range
    }
}

/// Des cellules terrestres tirées sur une large zone, négatives comprises.
fn land_cells(world: &MacroWorld, n: usize, seed: u64) -> Vec<FineCell> {
    let mut rng = Lcg(seed);
    let mut cells = Vec::new();
    while cells.len() < n {
        let cell = (rng.coord(900), rng.coord(900));
        if !world.hydrology().is_sea(cell) {
            cells.push(cell);
        }
    }
    cells
}

fn land_points(world: &MacroWorld, n: usize, seed: u64) -> Vec<(f64, f64)> {
    let mut rng = Lcg(seed);
    let mut out = Vec::new();
    while out.len() < n {
        let (x, z) = (rng.coord(30_000) as f64, rng.coord(30_000) as f64);
        if world.geography().sample(x, z).macro_elevation >= SEA_LEVEL as f32 {
            out.push((x, z));
        }
    }
    out
}

// ---- déterminisme ----

#[test]
fn samples_are_identical_in_sequence_reverse_parallel_and_any_cache_state() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let pts = land_points(&world, 150, 9 + seed as u64);
        let reference: Vec<MacroSample> = pts.iter().map(|&(x, z)| world.sample(x, z)).collect();

        // Ordre inverse, caches pleins (le premier passage les a remplis).
        let reverse: Vec<_> = pts.iter().rev().map(|&(x, z)| world.sample(x, z)).collect();
        assert!(
            reference.iter().eq(reverse.iter().rev()),
            "graine {seed} : ordre inverse, caches pleins"
        );

        // Caches vides, puis vidés au milieu du parcours.
        let fresh = MacroWorld::new(seed);
        let mut cleared = Vec::new();
        for (i, &(x, z)) in pts.iter().enumerate() {
            if i % 37 == 0 {
                fresh.clear_caches();
            }
            cleared.push(fresh.sample(x, z));
        }
        assert_eq!(reference, cleared, "graine {seed} : caches vidés en route");

        // Caches évincés en permanence (64 entrées par table).
        let tiny = MacroWorld::with_cache_limit(seed, 64);
        let evicted: Vec<_> = pts.iter().map(|&(x, z)| tiny.sample(x, z)).collect();
        assert_eq!(reference, evicted, "graine {seed} : caches évincés");
        assert!(
            tiny.hydrology().cached_entries() <= CACHE_TABLES * 64,
            "mémoire bornée"
        );

        // Parallèle : quatre threads se disputent le même contexte dans des ordres différents.
        let shared = MacroWorld::new(seed);
        let results: Vec<Vec<(usize, MacroSample)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|t| {
                    let (shared, pts) = (&shared, &pts);
                    scope.spawn(move || {
                        let mut order: Vec<usize> = (0..pts.len()).collect();
                        match t {
                            0 => {}
                            1 => order.reverse(),
                            2 => order.sort_by_key(|i| (i * 7919) % pts.len()),
                            _ => order.sort_by_key(|i| pts.len() - (i * 31) % pts.len()),
                        }
                        order
                            .into_iter()
                            .map(|i| (i, shared.sample(pts[i].0, pts[i].1)))
                            .collect()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (t, res) in results.into_iter().enumerate() {
            for (i, s) in res {
                assert_eq!(s, reference[i], "graine {seed}, thread {t}, point {i}");
            }
        }
    }
}

#[test]
fn hydrology_cells_do_not_depend_on_evaluation_order() {
    let world = MacroWorld::new(42);
    let cells = land_cells(&world, 400, 5);
    let h = world.hydrology();
    let snapshot = |cells: &[FineCell]| -> Vec<(Option<FineCell>, i32, u32)> {
        cells
            .iter()
            .map(|&c| (h.next(c), h.level_block(c), h.accumulation(c)))
            .collect()
    };
    let forward = snapshot(&cells);
    h.clear_caches();
    let mut reversed = cells.clone();
    reversed.reverse();
    let mut backward = snapshot(&reversed);
    backward.reverse();
    assert_eq!(forward, backward);
}

// ---- raccords entre régions ----

/// Les échantillons d'un carré de côté `size` dont le coin est (x0, z0), au pas `step`, calculés
/// dans un contexte neuf : la « région » ne connaît rien de ses voisines.
fn region(seed: u32, x0: i32, z0: i32, size: i32, step: i32) -> Vec<((i32, i32), MacroSample)> {
    let world = MacroWorld::new(seed);
    let mut out = Vec::new();
    for dz in (0..size).step_by(step as usize) {
        for dx in (0..size).step_by(step as usize) {
            out.push((
                (x0 + dx, z0 + dz),
                world.sample((x0 + dx) as f64, (z0 + dz) as f64),
            ));
        }
    }
    out
}

#[test]
fn separate_regions_agree_with_a_united_region_at_edges_and_four_region_corners() {
    // Des coins de quatre régions de 512 blocs, positifs et négatifs, et un bord de 2 048.
    let corners = [
        (0, 0),
        (BASIN_SIZE, BASIN_SIZE),
        (-BASIN_SIZE, -BASIN_SIZE),
        (-BASIN_SIZE, 3 * BASIN_SIZE),
        (-4 * BASIN_SIZE, 0),
    ];
    for seed in SEEDS {
        for (cx, cz) in corners {
            let half = 96;
            // La région réunie : 4 × la zone autour du coin.
            let united = region(seed, cx - half, cz - half, 2 * half, 8);
            // Quatre régions séparées, chacune dans son propre contexte.
            let quadrants = [
                (cx - half, cz - half),
                (cx, cz - half),
                (cx - half, cz),
                (cx, cz),
            ];
            let mut separate = Vec::new();
            for (qx, qz) in quadrants {
                separate.extend(region(seed, qx, qz, half, 8));
            }
            separate.sort_by_key(|&(p, _)| p);
            let mut united_sorted = united.clone();
            united_sorted.sort_by_key(|&(p, _)| p);
            assert_eq!(
                united_sorted, separate,
                "graine {seed}, coin ({cx}, {cz}) : régions séparées et réunies"
            );
        }
    }
}

#[test]
fn edge_contracts_are_read_identically_by_both_neighbours() {
    for seed in SEEDS {
        // Chaque voisin calcule dans son propre contexte neuf, depuis son côté de l'arête.
        let (upstream_side, downstream_side) = (MacroWorld::new(seed), MacroWorld::new(seed));
        let mut rng = Lcg(77 + seed as u64);
        let mut checked = 0;
        for _ in 0..4000 {
            let from = (rng.coord(80), rng.coord(80));
            let h = upstream_side.hydrology();
            let BasinFlow::Flow { to, crossing } = h.basin_flow(from) else {
                continue;
            };
            // Le voisin aval relit l'arête : même passe, mêmes niveaux, même identité.
            let a = h.contract(from, to).expect("l'arête existe");
            let b = downstream_side
                .hydrology()
                .contract(from, to)
                .expect("l'arête existe aussi de l'autre côté");
            assert_eq!(a, b);
            // L'entrée est dans le bassin aval, collée à la sortie ; le niveau ne remonte pas.
            assert_eq!(h.basin_of(crossing.exit), from);
            assert_eq!(h.basin_of(crossing.entry), to);
            // Deux cellules voisines (8 voisins) de part et d'autre de la frontière.
            assert_eq!(
                (crossing.exit.0 - crossing.entry.0)
                    .abs()
                    .max((crossing.exit.1 - crossing.entry.1).abs()),
                1
            );
            assert!(
                a.level_out >= a.level_in,
                "niveau de part et d'autre du bord"
            );
            assert!(a.accumulation >= 1 && a.accumulation <= ACCUMULATION_CAP);
            // L'arête inverse n'existe pas : le sens d'écoulement est unique.
            assert!(downstream_side.hydrology().contract(to, from).is_none());
            checked += 1;
        }
        assert!(checked > 200, "graine {seed} : {checked} arêtes vérifiées");
    }
}

// ---- acyclicité et terminaison bornée ----

#[test]
fn every_path_is_acyclic_and_ends_at_a_declared_sink_within_the_bound() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        let mut longest = 0;
        let mut max_hops = 0;
        let mut capped = 0;
        let mut oceans = 0;
        let mut lakes = 0;
        for start in land_cells(&world, 1500, 31 + seed as u64) {
            let mut seen = HashSet::new();
            let mut cur = start;
            let mut steps = 0;
            let mut hops_before = h.chain(h.basin_of(start)).hops;
            max_hops = max_hops.max(hops_before);
            assert!(hops_before <= MAX_BASIN_HOPS || h.chain(h.basin_of(start)).capped);
            loop {
                assert!(seen.insert(cur), "graine {seed} : cycle en {cur:?}");
                assert!(steps <= MAX_FINE_PATH, "graine {seed} : chemin trop long");
                let Some(next) = h.next(cur) else { break };
                // Le sens du graphe de bassins : les arêtes inter-bassins diminuent la profondeur.
                let (b, nb) = (h.basin_of(cur), h.basin_of(next));
                if b != nb {
                    let BasinFlow::Flow { to, crossing } = h.basin_flow(b) else {
                        panic!("graine {seed} : sortie d'un puits {b:?}");
                    };
                    assert_eq!((to, crossing.exit, crossing.entry), (nb, cur, next));
                    let hops = h.chain(b).hops;
                    assert_eq!(h.chain(nb).hops + 1, hops, "profondeur +1 par arête");
                    assert!(hops <= hops_before || steps == 0);
                    hops_before = hops;
                } else {
                    // À l'intérieur d'un bassin : on se rapproche strictement de la racine de l'arbre.
                    assert_eq!(
                        h.depth_in_basin(next) + 1,
                        h.depth_in_basin(cur),
                        "graine {seed} : pas qui ne descend pas l'arbre"
                    );
                }
                cur = next;
                steps += 1;
            }
            longest = longest.max(steps);
            // Terminaison déclarée : la mer, ou le nœud d'un puits déclaré (lac terminal).
            if h.is_sea(cur) {
                oceans += 1;
            } else {
                let b = h.basin_of(cur);
                match h.basin_flow(b) {
                    BasinFlow::Sink(kind) => {
                        assert_eq!(cur, h.basin_target(b), "le lac terminal est au nœud");
                        if kind == SinkKind::Capped {
                            capped += 1;
                        }
                        lakes += 1;
                    }
                    BasinFlow::Flow { .. } => panic!("graine {seed} : cours interrompu en {cur:?}"),
                }
            }
        }
        eprintln!(
            "graine {seed} : chemin le plus long {longest} pas, profondeur max {max_hops}, \
             {oceans} vers la mer, {lakes} vers un lac terminal ({capped} plafonnés)"
        );
        assert!(max_hops <= MAX_BASIN_HOPS);
        assert_eq!(
            capped, 0,
            "la borne de profondeur n'est pas atteinte en pratique"
        );
        assert!(longest > 5, "des chemins non triviaux");
        assert!(
            oceans > 0,
            "graine {seed} : aucune embouchure sur l'échantillon"
        );
    }
}

// ---- profils d'eau, confluences, lacs ----

#[test]
fn water_never_climbs_downstream_even_after_rounding_to_blocks() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        let mut confluences = 0;
        for cell in land_cells(&world, 1200, 3 + seed as u64) {
            let Some(next) = h.next(cell) else { continue };
            assert!(
                h.level(cell) >= h.level(next),
                "graine {seed} : remontée flottante"
            );
            assert!(
                h.level_block(cell) >= h.level_block(next),
                "graine {seed} : remontée en blocs"
            );
            // L'accumulation croît vers l'aval ; à une confluence, elle couvre tous les affluents.
            assert!(h.accumulation(next) >= (h.accumulation(cell) + 1).min(ACCUMULATION_CAP));
            let mut inflow = 0;
            let mut tributaries = 0;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let u = (next.0 + dx, next.1 + dz);
                    if u != next && h.next(u) == Some(next) {
                        tributaries += 1;
                        inflow += h.accumulation(u);
                        assert!(h.level(u) >= h.level(next), "affluent plus bas que l'aval");
                    }
                }
            }
            if tributaries >= 2 {
                confluences += 1;
            }
            assert!(h.accumulation(next) >= (1 + inflow).min(ACCUMULATION_CAP));
            // La polyligne : le niveau lu en un nœud est celui de la cellule.
            let [nx, nz] = h.node(cell);
            if let Some(r) = h.river_near(nx, nz).filter(|r| r.distance < 0.01) {
                assert!((r.level - h.level(cell)).abs() < 1e-3, "niveau au nœud");
            }
        }
        assert!(
            confluences > 0,
            "graine {seed} : aucune confluence échantillonnée"
        );
    }
}

#[test]
fn lakes_have_a_level_and_an_outlet_or_are_declared_closed() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        let (mut through, mut closed) = (0, 0);
        let mut rng = Lcg(99 + seed as u64);
        for _ in 0..30_000 {
            let cell = (rng.coord(400), rng.coord(400));
            for lake in h.lakes_near(cell) {
                if lake.terminal {
                    closed += 1;
                    assert!(
                        lake.level >= SEA_LEVEL as f32,
                        "un lac terminal est sur la terre"
                    );
                } else {
                    through += 1;
                    // Un lac de passage a un déversoir au niveau inférieur ou égal.
                    let g = engine_voxel::fine_cell(lake.center[0], lake.center[1]);
                    let outlet = h.next(g).expect("le lac de passage n'est pas un puits");
                    assert_eq!(lake.level, h.level(g));
                    assert!(h.level(outlet) <= lake.level);
                    assert!(lake.radius < FINE_SIZE as f64);
                }
            }
        }
        assert!(
            through > 0 && closed > 0,
            "graine {seed} : {through} lacs de passage, {closed} fermés"
        );
    }
}

// ---- coût borné ----

/// Borne analytique (très large) des altitudes évaluées par une requête de point à froid. Une requête
/// touche au plus 50 cellules (la fenêtre de 5 × 5 et leurs aval) ; chacune parcourt au plus
/// `MAX_BASIN_HOPS + 1` bassins vers l'aval et, pour l'accumulation, au plus
/// `10 × (ACCUMULATION_CAP + 10)` bassins amont (le parcours s'arrête à la saturation) ; chaque
/// bassin coûte au plus `9 × MAX_BASIN_CELLS` altitudes (ses cellules et leur couronne) plus les
/// graines (36 altitudes pour chacune des 25 graines proches).
const fn query_elevation_bound() -> u64 {
    let regions = 50 * (MAX_BASIN_HOPS as u64 + 1) + 50 * 10 * (ACCUMULATION_CAP as u64 + 10);
    regions * (9 * MAX_BASIN_CELLS as u64 + 25 * 36)
}

/// Bornes mesurées (avec marge, environ 2,2 fois le pire observé sur 600 points et sur 1 500
/// bassins) sur les mêmes coûts : une requête de point à froid évalue au plus environ 4 500
/// altitudes (1 700 en moyenne), la résolution d'un bassin au plus environ 1 400. Ce sont des
/// constantes : elles ne grandissent pas avec ce qu'on observe.
const MEASURED_QUERY_ELEVATIONS: u64 = 10_000;
const MEASURED_BASIN_ELEVATIONS: u64 = 3_000;

#[test]
fn a_cold_query_evaluates_a_bounded_number_of_cells() {
    let mut worst = (0, 0);
    let mut total = 0;
    let mut count = 0;
    for seed in SEEDS {
        let probe = MacroWorld::new(seed);
        for (x, z) in land_points(&probe, 120, 1 + seed as u64) {
            // Un contexte neuf par requête : tout est à froid.
            let world = MacroWorld::new(seed);
            world.sample(x, z);
            let stats = world.hydrology().stats();
            assert!(stats.elevations <= query_elevation_bound());
            assert!(
                stats.elevations <= MEASURED_QUERY_ELEVATIONS,
                "graine {seed}, ({x}, {z}) : {} altitudes",
                stats.elevations
            );
            if stats.elevations > worst.0 {
                worst = (stats.elevations, stats.work);
            }
            total += stats.elevations;
            count += 1;
        }
    }
    eprintln!(
        "requête de point à froid : {} altitudes en moyenne, {} au pire ({} calculs au total) ; \
         borne analytique {}",
        total / count,
        worst.0,
        worst.1,
        query_elevation_bound()
    );
}

#[test]
fn resolving_a_basin_looks_at_a_bounded_number_of_basins() {
    for seed in SEEDS {
        let mut worst = 0;
        for i in 0..300 {
            let world = MacroWorld::new(seed);
            let mut rng = Lcg(i + 1000 * seed as u64);
            let b = (rng.coord(100), rng.coord(100));
            world.hydrology().chain(b);
            let stats = world.hydrology().stats();
            // Au plus (MAX + 1) bassins parcourus, chacun coûtant au plus ses cellules et leur
            // couronne (9 × MAX_BASIN_CELLS) plus les graines proches.
            let bound = (MAX_BASIN_HOPS as u64 + 1) * (9 * MAX_BASIN_CELLS as u64 + 25 * 36);
            assert!(stats.elevations <= bound, "{} > {bound}", stats.elevations);
            assert!(
                stats.elevations <= MEASURED_BASIN_ELEVATIONS,
                "graine {seed}, bassin {b:?} : {} altitudes",
                stats.elevations
            );
            worst = worst.max(stats.elevations);
        }
        eprintln!("graine {seed} : résoudre un bassin évalue au plus {worst} altitudes à froid");
    }
}

// ---- géographie ----

#[test]
fn geography_is_continuous_across_region_borders_and_in_negative_coordinates() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let mut max_macro = (0.0f32, 0.0f64, 0.0f64);
        let mut max_carved = (0.0f32, 0.0f64, 0.0f64);
        // Des lignes qui traversent des frontières de 512 blocs, des deux côtés de zéro.
        for &(z0, x0) in &[
            (-1024.0, -2048.0),
            (333.0, -513.0),
            (-4096.0, 1000.0),
            (2048.0, 31_000.0),
        ] {
            let mut last: Option<(f32, f32)> = None;
            for i in 0..1200 {
                let x = x0 + i as f64 * 4.0;
                let s = world.sample(x, z0);
                let pair = (s.geo.macro_elevation, s.elevation);
                if let Some(l) = last {
                    let (dm, dc) = ((pair.0 - l.0).abs(), (pair.1 - l.1).abs());
                    if dm > max_macro.0 {
                        max_macro = (dm, x, z0);
                    }
                    if dc > max_carved.0 {
                        max_carved = (dc, x, z0);
                    }
                }
                last = Some(pair);
            }
        }
        // Pas de marche artificielle : 4 blocs d'écart ne changent jamais l'altitude de plus que
        // la pente la plus raide du relief macro et que la profondeur d'un lit.
        assert!(
            max_macro.0 < 9.0,
            "graine {seed} : marche macro {max_macro:?}"
        );
        assert!(
            max_carved.0 < 13.0,
            "graine {seed} : marche modelée {max_carved:?}"
        );
    }
}

#[test]
fn land_sea_chains_and_summits_look_like_a_world() {
    let top = (WORLD_HEIGHT_CHUNKS * 32) as f32;
    let (mut best_peak, mut massif_seeds) = (0.0f32, 0);
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let g = world.geography();
        let (mut land, mut total, mut massif, mut clipped) = (0, 0, 0, 0);
        let mut rng = Lcg(seed as u64);
        for _ in 0..6000 {
            let (x, z) = (rng.coord(30_000) as f64, rng.coord(30_000) as f64);
            let s = g.sample(x, z);
            total += 1;
            if s.macro_elevation >= SEA_LEVEL as f32 {
                land += 1;
                best_peak = best_peak.max(s.elevation);
                if s.chain_height >= 38.0 {
                    massif += 1;
                }
                if s.elevation >= top - 4.0 {
                    clipped += 1;
                }
            }
            assert!(s.elevation >= 3.0 && s.elevation < top - 1.0);
        }
        let fraction = land as f64 / total as f64;
        assert!(
            (0.2..0.6).contains(&fraction),
            "graine {seed} : {:.0} % de terres",
            fraction * 100.0
        );
        // Pas d'écrêtage systématique : presque aucun sommet collé au plafond.
        assert!(
            clipped * 200 <= land.max(1),
            "graine {seed} : {clipped} sommets écrêtés"
        );
        if massif > 0 {
            massif_seeds += 1;
        }
    }
    assert!(
        massif_seeds >= 4,
        "des massifs sur presque toutes les graines"
    );
    assert!(
        best_peak > 140.0,
        "des sommets élevés existent : {best_peak}"
    );
}

#[test]
fn basin_ids_label_drainage_basins_consistently() {
    let world = MacroWorld::new(42);
    let h = world.hydrology();
    // Le long d'un cours, l'identité du bassin versant ne change pas d'un bassin à l'autre, sauf à
    // l'entrée du bassin océanique (le puits) : il porte alors sa propre identité.
    let mut crossings = 0;
    for cell in land_cells(&world, 300, 1) {
        let mut cur = cell;
        while let Some(n) = h.next(cur) {
            let (b, nb) = (h.basin_of(cur), h.basin_of(n));
            if b != nb && !h.is_sea(n) {
                if !h.is_open_sea(nb) {
                    assert_eq!(h.basin_id(b), h.basin_id(nb), "cellule {cell:?}");
                }
                crossings += 1;
            }
            cur = n;
        }
        // Un cours qui finit dans un lac terminal : même identité que sa cellule de départ.
        if !h.is_sea(cur) {
            assert_eq!(h.basin_id(h.basin_of(cell)), h.basin_id(h.basin_of(cur)));
        }
    }
    assert!(
        crossings > 50,
        "des cours qui traversent des bassins : {crossings}"
    );
    assert_eq!(BASIN_FINE * FINE_SIZE, BASIN_SIZE);
}

// ---- forme et lisibilité du réseau ----

/// Les cellules d'un bassin, retrouvées par balayage d'une fenêtre autour de sa super-cellule.
fn basin_cells(h: &engine_voxel::Hydrology, b: (i32, i32)) -> Vec<FineCell> {
    let mut cells = Vec::new();
    for z in (b.1 * BASIN_FINE - 2 * BASIN_FINE)..(b.1 * BASIN_FINE + 3 * BASIN_FINE) {
        for x in (b.0 * BASIN_FINE - 2 * BASIN_FINE)..(b.0 * BASIN_FINE + 3 * BASIN_FINE) {
            if h.basin_of((x, z)) == b {
                cells.push((x, z));
            }
        }
    }
    cells
}

#[test]
fn basins_are_free_form_and_connected_not_grid_squares() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        let (mut regions, mut rectangles, mut fill, mut outside, mut total) = (0, 0, 0.0, 0, 0);
        let (mut smallest, mut largest) = (usize::MAX, 0);
        for j in -8..8 {
            for i in -8..8 {
                let b = (i, j);
                if h.is_open_sea(b) {
                    continue;
                }
                let cells = basin_cells(h, b);
                assert_eq!(
                    cells.len(),
                    h.basin_cell_count(b),
                    "graine {seed}, bassin {b:?}"
                );
                let (x0, x1) = cells.iter().fold((i32::MAX, i32::MIN), |(lo, hi), c| {
                    (lo.min(c.0), hi.max(c.0))
                });
                let (z0, z1) = cells.iter().fold((i32::MAX, i32::MIN), |(lo, hi), c| {
                    (lo.min(c.1), hi.max(c.1))
                });
                let area = ((x1 - x0 + 1) * (z1 - z0 + 1)) as usize;
                regions += 1;
                rectangles += usize::from(cells.len() == area);
                fill += cells.len() as f64 / area as f64;
                outside += cells
                    .iter()
                    .filter(|c| (c.0.div_euclid(BASIN_FINE), c.1.div_euclid(BASIN_FINE)) != b)
                    .count();
                total += cells.len();
                smallest = smallest.min(cells.len());
                largest = largest.max(cells.len());
                // Connexe : l'arbre de drainage couvre toutes les cellules (tout chemin finit à sa racine).
                for &c in &cells {
                    let mut cur = c;
                    let mut steps = 0;
                    while h.basin_of(cur) == b && !h.is_sea(cur) {
                        let Some(n) = h.next(cur) else { break };
                        if h.basin_of(n) != b {
                            break;
                        }
                        cur = n;
                        steps += 1;
                        assert!(steps <= cells.len(), "cycle dans le bassin {b:?}");
                    }
                }
            }
        }
        let (fill, outside) = (fill / regions as f64, outside as f64 / total as f64);
        eprintln!(
            "graine {seed} : {regions} bassins, {rectangles} rectangles, remplissage moyen \
             {fill:.2}, {:.0} % des cellules hors de leur super-cellule, tailles {smallest} à {largest}",
            outside * 100.0
        );
        // Un bassin qui suivrait la grille remplirait son rectangle englobant et resterait dans sa
        // super-cellule ; ici les formes sont libres.
        assert!(
            rectangles * 20 <= regions,
            "graine {seed} : bassins rectangulaires"
        );
        assert!(fill < 0.8, "graine {seed} : remplissage moyen {fill}");
        assert!(
            outside > 0.15,
            "graine {seed} : {outside} des cellules hors de leur super-cellule"
        );
        assert!(largest >= 2 * smallest, "graine {seed} : tailles uniformes");
    }
}

/// Les arêtes de cours d'eau (cellule, aval, courbe) d'un carré de cellules.
fn stream_edges(world: &MacroWorld, half: i32) -> Vec<(FineCell, FineCell, [[f64; 2]; 7])> {
    let h = world.hydrology();
    let mut edges = Vec::new();
    for z in -half..half {
        for x in -half..half {
            let g = (x, z);
            if h.is_sea(g) || h.accumulation(g) < STREAM_MIN_ACCUMULATION {
                continue;
            }
            if let Some(n) = h.next(g) {
                edges.push((g, n, h.river_polyline(g, n)));
            }
        }
    }
    edges
}

#[test]
fn rivers_are_not_aligned_on_the_grid_and_do_not_cross() {
    fn intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
        let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
            (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
        };
        side(c, d, a) * side(c, d, b) < 0.0 && side(a, b, c) * side(a, b, d) < 0.0
    }
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let edges = stream_edges(&world, 40);
        assert!(
            edges.len() > 300,
            "graine {seed} : {} arêtes de cours d'eau",
            edges.len()
        );
        // Direction des segments : la part proche d'un axe (à 5 degrés) reste proche de celle d'une
        // répartition uniforme (11 %), loin des peignes de tronçons droits.
        let (mut aligned, mut segments) = (0, 0);
        for (_, _, line) in &edges {
            for s in line.windows(2) {
                let angle = (s[1][1] - s[0][1])
                    .atan2(s[1][0] - s[0][0])
                    .to_degrees()
                    .rem_euclid(90.0);
                segments += 1;
                aligned += usize::from(!(5.0..=85.0).contains(&angle));
            }
        }
        let share = aligned as f64 / segments as f64;
        eprintln!(
            "graine {seed} : {:.1} % de segments alignés sur un axe",
            share * 100.0
        );
        assert!(
            share < 0.22,
            "graine {seed} : {:.0} % de segments alignés",
            share * 100.0
        );
        // Des cours qui ne partagent aucun nœud ne se coupent jamais.
        let mut pairs = 0;
        for (i, (g1, n1, l1)) in edges.iter().enumerate() {
            for (g2, n2, l2) in &edges[i + 1..] {
                if g1 == g2 || n1 == n2 || g1 == n2 || g2 == n1 {
                    continue;
                }
                if (g1.0 - g2.0).abs() > 3 || (g1.1 - g2.1).abs() > 3 {
                    continue;
                }
                pairs += 1;
                for a in l1.windows(2) {
                    for b in l2.windows(2) {
                        assert!(
                            !intersect(a[0], a[1], b[0], b[1]),
                            "graine {seed} : les cours {g1:?} et {g2:?} se coupent"
                        );
                    }
                }
            }
        }
        assert!(
            pairs > 1000,
            "graine {seed} : {pairs} paires de cours voisins vérifiées"
        );
    }
}

#[test]
fn main_rivers_collect_tributaries_and_widen_downstream() {
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        // Le cours le plus fort de la zone, puis sa remontée par la branche la plus forte.
        let mut best: Option<(u32, FineCell)> = None;
        for z in -50..50 {
            for x in -50..50 {
                let g = (x, z);
                if !h.is_sea(g) {
                    let acc = h.accumulation(g);
                    if best.is_none_or(|(b, _)| acc > b) {
                        best = Some((acc, g));
                    }
                }
            }
        }
        let (top, mut cur) = best.expect("de la terre");
        assert!(
            top >= 250,
            "graine {seed} : le plus grand fleuve n'a que {top} cellules amont"
        );
        let (mut tributaries, mut length) = (0, 0);
        let mut last_width = engine_voxel::river_width(top);
        loop {
            let mut upstream: Vec<(u32, FineCell)> = Vec::new();
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let u = (cur.0 + dx, cur.1 + dz);
                    if u != cur && h.next(u) == Some(cur) {
                        upstream.push((h.accumulation(u), u));
                    }
                }
            }
            upstream.sort();
            let Some(&(_, main)) = upstream.last() else {
                break;
            };
            tributaries += upstream
                .iter()
                .filter(|&&(a, u)| u != main && a >= STREAM_MIN_ACCUMULATION)
                .count();
            // La largeur ne diminue jamais en descendant (donc ne grandit qu'en remontant... à l'envers).
            let width = engine_voxel::river_width(h.accumulation(main));
            assert!(
                width <= last_width,
                "graine {seed} : la largeur augmente vers l'amont"
            );
            last_width = width;
            cur = main;
            length += 1;
        }
        eprintln!(
            "graine {seed} : fleuve de {top} cellules, {length} cellules de cours principal, {tributaries} affluents"
        );
        assert!(
            length >= 12,
            "graine {seed} : cours principal de {length} cellules"
        );
        assert!(
            tributaries >= 4,
            "graine {seed} : {tributaries} affluents seulement"
        );
    }
}

#[test]
fn estuaries_widen_and_inland_rivers_do_not() {
    let (mut mouths, mut flared, mut inland, mut plain) = (0, 0, 0, 0);
    for seed in SEEDS {
        let world = MacroWorld::new(seed);
        let h = world.hydrology();
        for z in -60..60 {
            for x in -60..60 {
                let g = (x, z);
                if h.is_sea(g) {
                    continue;
                }
                let Some(n) = h.next(g) else { continue };
                let acc = h.accumulation(g);
                if acc < 60 {
                    continue;
                }
                let [nx, nz] = h.node(g);
                let Some(r) = h.river_near(nx, nz) else {
                    continue;
                };
                let ratio = r.width / engine_voxel::river_width(acc);
                if h.is_sea(n) {
                    mouths += 1;
                    flared += usize::from(ratio > 1.4);
                } else if world.geography().sample(nx, nz).coast_distance > 1500.0 {
                    inland += 1;
                    plain += usize::from(ratio < 1.001);
                }
            }
        }
    }
    eprintln!(
        "embouchures évasées : {flared}/{mouths} ; cours intérieurs sans évasement : {plain}/{inland}"
    );
    assert!(mouths >= 10, "{mouths} embouchures");
    assert!(
        flared * 10 >= mouths * 7,
        "{flared}/{mouths} embouchures évasées"
    );
    assert!(
        inland >= 10 && plain == inland,
        "{plain}/{inland} cours intérieurs sans évasement"
    );
}
