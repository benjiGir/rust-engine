//! Les tests du prototype macro v2 (lots 2 et 3) : géographie et hydrologie, avant tout bloc.
//!
//! Le passage du lot 3 est **bloquant** : déterminisme (ordre, threads, caches), raccords entre
//! régions (bords, coins de quatre régions, coordonnées négatives), acyclicité et terminaison
//! bornée, profils d'eau sans remontée, confluences et lacs cohérents, coût borné.
//! Voir `docs/plan-generation-rendu.md`.

use std::collections::HashSet;

use engine_voxel::{
    ACCUMULATION_CAP, BASIN_FINE, BASIN_SIZE, BasinFlow, FINE_SIZE, FineCell, MAX_BASIN_HOPS,
    MAX_FINE_PATH, MacroSample, MacroWorld, SEA_LEVEL, SinkKind, WORLD_HEIGHT_CHUNKS, basin_of,
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
            tiny.hydrology().cached_entries() <= 9 * 64,
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
            assert_eq!(basin_of(crossing.exit), from);
            assert_eq!(basin_of(crossing.entry), to);
            assert_eq!(
                (crossing.exit.0 - crossing.entry.0).abs()
                    + (crossing.exit.1 - crossing.entry.1).abs(),
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
            let mut hops_before = h.chain(basin_of(start)).hops;
            max_hops = max_hops.max(hops_before);
            assert!(hops_before <= MAX_BASIN_HOPS || h.chain(basin_of(start)).capped);
            loop {
                assert!(seen.insert(cur), "graine {seed} : cycle en {cur:?}");
                assert!(steps <= MAX_FINE_PATH, "graine {seed} : chemin trop long");
                let Some(next) = h.next(cur) else { break };
                // Le sens du graphe de bassins : les arêtes inter-bassins diminuent la profondeur.
                let (b, nb) = (basin_of(cur), basin_of(next));
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
                    // À l'intérieur d'un bassin : on se rapproche strictement de la cible.
                    let target = match h.basin_flow(b) {
                        BasinFlow::Sink(_) => h.basin_target(b),
                        BasinFlow::Flow { crossing, .. } => crossing.exit,
                    };
                    let d2 = |c: FineCell| {
                        (c.0 - target.0).pow(2) as i64 + (c.1 - target.1).pow(2) as i64
                    };
                    assert!(d2(next) < d2(cur), "graine {seed} : pas qui s'éloigne");
                }
                cur = next;
                steps += 1;
            }
            longest = longest.max(steps);
            // Terminaison déclarée : la mer, ou le nœud d'un puits déclaré (lac terminal).
            if h.is_sea(cur) {
                oceans += 1;
            } else {
                let b = basin_of(cur);
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

/// Borne analytique, large, des altitudes évaluées par une requête de point à froid : 25 cellules
/// voisines, chacune avec au plus un parcours aval de `MAX_FINE_PATH` cellules et une
/// accumulation de `ACCUMULATION_CAP` cellules (neuf altitudes par cellule au plus), plus les
/// nœuds (36 altitudes) et les passes (16) de 5 bassins par bassin du chemin.
const fn query_elevation_bound() -> u64 {
    let cells = 25 * (MAX_FINE_PATH as u64 + ACCUMULATION_CAP as u64 * 9) * 9;
    let basins = 5 * (MAX_BASIN_HOPS as u64 + 1) * (36 + 4 * 16);
    cells + basins
}

/// Borne mesurée (avec marge) sur le même coût : le coût réel est environ cent fois plus bas.
const MEASURED_QUERY_ELEVATIONS: u64 = 12_000;

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
            // Au plus (MAX + 1) bassins, chacun avec ses quatre voisins : 36 altitudes de nœud
            // et 16 de passe par arête examinée.
            let bound = 5 * (MAX_BASIN_HOPS as u64 + 1) * (36 + 4 * 16);
            assert!(stats.elevations <= bound, "{} > {bound}", stats.elevations);
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
    // Toutes les cellules d'un même chemin terminent dans le même bassin versant.
    for cell in land_cells(&world, 300, 1) {
        let id = h.basin_id(basin_of(cell));
        let mut cur = cell;
        while let Some(n) = h.next(cur) {
            cur = n;
        }
        // Le puits atteint est dans un bassin de la chaîne : même bassin versant.
        assert_eq!(id, h.basin_id(basin_of(cur)), "cellule {cell:?}");
    }
    assert_eq!(BASIN_FINE * FINE_SIZE, BASIN_SIZE);
}
