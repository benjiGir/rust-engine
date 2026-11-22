//! Le moteur de densité (lot 1 du plan v3) : chaque nœud, les splines, la compilation, les bornes, le
//! remplissage par chunk (interpolation par cellules et caches), le déterminisme et le coût.

use engine_voxel::worldgen::density_graph::{CELL_HEIGHT, CELL_WIDTH, DensityError, DensityGraph};
use glam::IVec3;

fn graph(functions: &str) -> DensityGraph {
    let text = format!(
        "(noises: {{ \"a\": (first_octave: -5, amplitudes: [1.0, 1.0]), \"b\": (first_octave: -7, amplitudes: [1.0, 0.5, 0.25]) }}, functions: {{ {functions} }})"
    );
    DensityGraph::parse(&text, 42).unwrap_or_else(|e| panic!("{e}"))
}

fn value(functions: &str, name: &str, x: i32, y: i32, z: i32) -> f64 {
    let g = graph(functions);
    g.sample(g.function(name).unwrap(), x, y, z)
}

#[test]
fn arithmetic_nodes_compute_what_they_say() {
    let one = |f: &str| value(&format!("\"f\": {f}"), "f", 0, 0, 0);
    assert_eq!(one("Add(Constant(2.0), Constant(3.0))"), 5.0);
    assert_eq!(one("Mul(Constant(2.0), Constant(-3.0))"), -6.0);
    assert_eq!(one("Min(Constant(2.0), Constant(-3.0))"), -3.0);
    assert_eq!(one("Max(Constant(2.0), Constant(-3.0))"), 2.0);
    assert_eq!(one("Clamp(input: Constant(7.0), min: -1.0, max: 1.0)"), 1.0);
    assert_eq!(one("Abs(Constant(-2.5))"), 2.5);
    assert_eq!(one("Square(Constant(-3.0))"), 9.0);
    assert_eq!(one("Cube(Constant(-2.0))"), -8.0);
    assert_eq!(one("HalfNegative(Constant(-2.0))"), -1.0);
    assert_eq!(one("HalfNegative(Constant(2.0))"), 2.0);
    assert_eq!(one("QuarterNegative(Constant(-2.0))"), -0.5);
    // c / 2 - c³ / 24, bornée à [-1, 1].
    assert!((one("Squeeze(Constant(0.5))") - (0.25 - 0.125 / 24.0)).abs() < 1e-12);
    assert!((one("Squeeze(Constant(9.0))") - (0.5 - 1.0 / 24.0)).abs() < 1e-12);
    let choose = |v: f64| {
        one(&format!(
            "RangeChoice(input: Constant({v}), min_inclusive: 0.0, max_exclusive: 1.0, when_in_range: Constant(10.0), when_out_of_range: Constant(20.0))"
        ))
    };
    assert_eq!(choose(0.0), 10.0);
    assert_eq!(choose(0.99), 10.0);
    assert_eq!(choose(1.0), 20.0);
    assert_eq!(choose(-0.1), 20.0);
}

#[test]
fn the_y_gradient_is_clamped_outside_its_range() {
    let f = "\"g\": YClampedGradient(from_y: -64, to_y: 64, from_value: 1.0, to_value: -1.0)";
    assert_eq!(value(f, "g", 0, -200, 0), 1.0);
    assert_eq!(value(f, "g", 0, -64, 0), 1.0);
    assert_eq!(value(f, "g", 0, 0, 0), 0.0);
    assert_eq!(value(f, "g", 0, 32, 0), -0.5);
    assert_eq!(value(f, "g", 0, 500, 0), -1.0);
}

#[test]
fn splines_pass_through_their_points_and_extend_in_straight_lines() {
    // La coordonnée est y / 10 (un gradient de -10 à 10 sur y de -100 à 100).
    let f = "\"c\": YClampedGradient(from_y: -100, to_y: 100, from_value: -10.0, to_value: 10.0),
        \"s\": Spline((coordinate: Ref(\"c\"), points: [
            (location: -1.0, value: 2.0, derivative: 0.5),
            (location: 0.0, value: 0.0),
            (location: 1.0, value: 4.0, derivative: 1.0),
        ]))";
    let g = graph(f);
    let s = g.function("s").unwrap();
    let at = |c: f64| g.sample(s, 0, (c * 10.0) as i32, 0);
    assert!((at(-1.0) - 2.0).abs() < 1e-12);
    assert!((at(0.0) - 0.0).abs() < 1e-12);
    assert!((at(1.0) - 4.0).abs() < 1e-12);
    // Hors des points : une droite de pente donnée.
    assert!((at(-3.0) - (2.0 + 0.5 * -2.0)).abs() < 1e-12);
    assert!((at(5.0) - (4.0 + 1.0 * 4.0)).abs() < 1e-12);
    // Entre deux points de même valeur et de pente nulle, la spline est plate.
    let flat = graph(
        "\"c\": YClampedGradient(from_y: 0, to_y: 100, from_value: 0.0, to_value: 1.0),
        \"s\": Spline((coordinate: Ref(\"c\"), points: [(location: 0.0, value: 3.0), (location: 1.0, value: 3.0)]))",
    );
    let fs = flat.function("s").unwrap();
    for y in 0..=100 {
        assert!((flat.sample(fs, 0, y, 0) - 3.0).abs() < 1e-12);
    }
    // Une valeur peut être une spline imbriquée (sur une autre coordonnée).
    let nested = graph(
        "\"c\": YClampedGradient(from_y: 0, to_y: 100, from_value: 0.0, to_value: 1.0),
        \"s\": Spline((coordinate: Ref(\"c\"), points: [
            (location: 0.0, value: 1.0),
            (location: 1.0, value: (coordinate: Ref(\"c\"), points: [(location: 0.0, value: 5.0), (location: 1.0, value: 7.0)])),
        ]))",
    );
    let ns = nested.function("s").unwrap();
    assert!((nested.sample(ns, 0, 100, 0) - 7.0).abs() < 1e-12);
    assert!((nested.sample(ns, 0, 0, 0) - 1.0).abs() < 1e-12);
}

#[test]
fn named_functions_are_shared_and_errors_are_reported() {
    let shared = graph(
        "\"n\": Noise(noise: \"a\"),
        \"x\": Add(Ref(\"n\"), Ref(\"n\")),
        \"y\": Mul(Ref(\"n\"), Ref(\"x\"))",
    );
    // Noise, Add, Mul : trois nœuds, pas cinq.
    assert_eq!(shared.len(), 3);
    let (n, x) = (shared.function("n").unwrap(), shared.function("x").unwrap());
    assert_eq!(shared.sample(x, 5, 6, 7), 2.0 * shared.sample(n, 5, 6, 7));

    let err = |functions: &str| {
        DensityGraph::parse(&format!("(functions: {{ {functions} }})"), 1).unwrap_err()
    };
    assert_eq!(
        err("\"f\": Ref(\"absent\")"),
        DensityError::UnknownFunction("absent".into())
    );
    assert_eq!(
        err("\"f\": Noise(noise: \"absent\")"),
        DensityError::UnknownNoise("absent".into())
    );
    assert!(matches!(
        err("\"f\": Ref(\"g\"), \"g\": Add(Ref(\"f\"), Constant(1.0))"),
        DensityError::Cycle(_)
    ));
    assert!(matches!(
        err(
            "\"f\": Spline((coordinate: Constant(0.0), points: [(location: 1.0, value: 0.0), (location: 0.0, value: 1.0)]))"
        ),
        DensityError::BadSpline(_)
    ));
    assert!(matches!(
        DensityGraph::parse("(functions: { \"f\": Bidule })", 1).unwrap_err(),
        DensityError::Parse(_)
    ));
}

/// Un graphe proche d'un relief : paramètres en bruit, spline de décalage, bruit 3D déformé, caches.
const RELIEF: &str = "
    \"continents\": FlatCache(Noise(noise: \"b\", xz_scale: 0.25, y_scale: 0.0)),
    \"offset\": Spline((coordinate: Ref(\"continents\"), points: [
        (location: -1.0, value: -0.6, derivative: 0.0),
        (location: -0.2, value: -0.1, derivative: 0.6),
        (location: 0.3, value: 0.2, derivative: 0.4),
        (location: 1.0, value: 0.8, derivative: 0.0),
    ])),
    \"shift\": Cache2d(Mul(Noise(noise: \"a\", xz_scale: 0.5, y_scale: 0.0), Constant(4.0))),
    \"bumps\": ShiftedNoise(noise: \"a\", xz_scale: 1.0, y_scale: 0.5, shift_x: Ref(\"shift\"), shift_y: Constant(0.0), shift_z: Ref(\"shift\")),
    \"depth\": Add(YClampedGradient(from_y: -128, to_y: 256, from_value: 1.5, to_value: -1.5), Ref(\"offset\")),
    \"raw\": Add(Mul(Ref(\"depth\"), Constant(4.0)), Mul(Squeeze(Ref(\"bumps\")), Constant(0.6))),
    \"final\": Interpolated(CacheOnce(Ref(\"raw\"))),
    \"exact\": CacheOnce(Ref(\"raw\")),
";

#[test]
fn values_always_stay_within_the_computed_bounds() {
    let g = graph(RELIEF);
    for name in g.names().map(str::to_string).collect::<Vec<_>>() {
        let id = g.function(&name).unwrap();
        let (lo, hi) = g.bounds(id);
        assert!(lo <= hi, "{name} : {lo} > {hi}");
        for i in 0..3000 {
            let (x, y, z) = (i * 97 - 150_000, (i * 13) % 384 - 128, i * 61 - 90_000);
            let v = g.sample(id, x, y, z);
            assert!(
                v >= lo - 1e-9 && v <= hi + 1e-9,
                "{name} : {v} hors de [{lo}, {hi}]"
            );
        }
    }
}

fn fill(g: &DensityGraph, name: &str, origin: IVec3) -> Vec<f64> {
    let mut out = vec![0.0; 32 * 32 * 32];
    g.fill_chunk(g.function(name).unwrap(), origin, &mut out);
    out
}

#[test]
fn filling_a_chunk_without_interpolation_matches_sampling_every_block() {
    let g = graph(RELIEF);
    let origin = IVec3::new(-64, 32, 96);
    let filled = fill(&g, "exact", origin);
    let id = g.function("exact").unwrap();
    for (i, v) in filled.iter().enumerate() {
        let (lx, ly, lz) = ((i % 32) as i32, (i / 1024) as i32, ((i / 32) % 32) as i32);
        let expected = g.sample(id, origin.x + lx, origin.y + ly, origin.z + lz);
        assert_eq!(*v, expected, "bloc {lx} {ly} {lz}");
    }
}

#[test]
fn interpolated_values_are_exact_at_cell_corners_and_trilinear_between() {
    let g = graph(RELIEF);
    let exact = g.function("exact").unwrap();
    for origin in [
        IVec3::new(0, 0, 0),
        IVec3::new(-32, -128, -96),
        IVec3::new(320, 224, -4096),
    ] {
        let filled = fill(&g, "final", origin);
        let corner = |cx: i32, cy: i32, cz: i32| {
            g.sample(
                exact,
                origin.x + cx * CELL_WIDTH,
                origin.y + cy * CELL_HEIGHT,
                origin.z + cz * CELL_WIDTH,
            )
        };
        for (i, v) in filled.iter().enumerate() {
            let (lx, ly, lz) = ((i % 32) as i32, (i / 1024) as i32, ((i / 32) % 32) as i32);
            let (cx, cy, cz) = (lx / CELL_WIDTH, ly / CELL_HEIGHT, lz / CELL_WIDTH);
            let tx = f64::from(lx % CELL_WIDTH) / f64::from(CELL_WIDTH);
            let ty = f64::from(ly % CELL_HEIGHT) / f64::from(CELL_HEIGHT);
            let tz = f64::from(lz % CELL_WIDTH) / f64::from(CELL_WIDTH);
            let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
            let expected = lerp(
                tz,
                lerp(
                    ty,
                    lerp(tx, corner(cx, cy, cz), corner(cx + 1, cy, cz)),
                    lerp(tx, corner(cx, cy + 1, cz), corner(cx + 1, cy + 1, cz)),
                ),
                lerp(
                    ty,
                    lerp(tx, corner(cx, cy, cz + 1), corner(cx + 1, cy, cz + 1)),
                    lerp(
                        tx,
                        corner(cx, cy + 1, cz + 1),
                        corner(cx + 1, cy + 1, cz + 1),
                    ),
                ),
            );
            assert!(
                (v - expected).abs() < 1e-9,
                "{origin} bloc {lx} {ly} {lz} : {v} contre {expected}"
            );
            if lx % CELL_WIDTH == 0 && ly % CELL_HEIGHT == 0 && lz % CELL_WIDTH == 0 {
                assert_eq!(*v, corner(cx, cy, cz), "coin exact");
            }
        }
    }
}

#[test]
fn neighbouring_chunks_meet_without_a_seam() {
    // Le dernier bloc d'un chunk et le premier du suivant sont tous deux sur la même cellule, de part
    // et d'autre d'un coin commun : l'écart entre eux est celui d'un pas d'interpolation, pas plus.
    let g = graph(RELIEF);
    let a = fill(&g, "final", IVec3::new(0, 64, 0));
    let b = fill(&g, "final", IVec3::new(32, 64, 0));
    let exact = g.function("exact").unwrap();
    for ly in [0, 8, 16] {
        for lz in [0, 4, 12] {
            let i = |lx: usize| lx + 32 * lz as usize + 1024 * ly as usize;
            // Le premier bloc de b est un coin : exact, et partagé avec la fin de la grille de a.
            assert_eq!(b[i(0)], g.sample(exact, 32, 64 + ly, lz));
            let step = (b[i(0)] - a[i(28)]) / 4.0;
            assert!((a[i(31)] - (a[i(28)] + 3.0 * step)).abs() < 1e-9);
        }
    }
}

#[test]
fn results_do_not_depend_on_order_threads_or_repetition() {
    let g = graph(RELIEF);
    let origins: Vec<IVec3> = (0..8)
        .map(|i| IVec3::new(i * 32 - 128, 32, i * 64))
        .collect();
    let forward: Vec<Vec<f64>> = origins.iter().map(|&o| fill(&g, "final", o)).collect();
    let backward: Vec<Vec<f64>> = origins
        .iter()
        .rev()
        .map(|&o| fill(&g, "final", o))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(forward, backward);
    let threaded: Vec<Vec<f64>> = std::thread::scope(|scope| {
        let handles: Vec<_> = origins
            .iter()
            .map(|&o| {
                let g = &g;
                scope.spawn(move || fill(g, "final", o))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(forward, threaded);
    // Une autre compilation de la même graine donne les mêmes valeurs ; une autre graine non.
    let again = graph(RELIEF);
    assert_eq!(fill(&again, "final", origins[3]), forward[3]);
    let other = DensityGraph::parse(
        &format!(
            "(noises: {{ \"a\": (first_octave: -5, amplitudes: [1.0, 1.0]), \"b\": (first_octave: -7, amplitudes: [1.0, 0.5, 0.25]) }}, functions: {{ {RELIEF} }})"
        ),
        43,
    )
    .unwrap();
    assert_ne!(fill(&other, "final", origins[3]), forward[3]);
}

/// `cargo test -p engine_voxel --test density_graph print_fill_cost --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_fill_cost() {
    let g = graph(RELIEF);
    let mut out = vec![0.0; 32 * 32 * 32];
    for name in ["final", "exact"] {
        let id = g.function(name).unwrap();
        let started = std::time::Instant::now();
        let n = 64;
        for i in 0..n {
            g.fill_chunk(id, IVec3::new(i * 32, 64, -i * 32), &mut out);
        }
        println!(
            "{name} : {:.3} ms par chunk",
            started.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
        );
    }
}

#[test]
fn cones_rise_around_their_sites_with_a_crater_and_follow_their_mask() {
    let cones = |mask: &str| {
        graph(&format!(
            "\"f\": Cones(cell: 200, chance: 1.0, mask: {mask}, radius: 60.0, height: 50.0, crater_radius: 10.0, crater_depth: 20.0)"
        ))
    };
    let g = cones("Constant(1.0)");
    let f = g.function("f").unwrap();
    assert_eq!(g.bounds(f), (0.0, 50.0));
    let sites = g.cone_sites(f, (0, 0), (999, 999));
    // Chance 1 : un cône par carré, entier dans son carré.
    assert_eq!(sites.len(), 25);
    for &(x, z) in &sites {
        assert!((60..140).contains(&x.rem_euclid(200)) && (60..140).contains(&z.rem_euclid(200)));
        let at = |dx: i32| g.sample(f, x + dx, 0, z);
        let (center, rim, foot, outside) = (at(0), at(12), at(40), at(61));
        assert!(
            rim > center + 15.0,
            "un cratère : bord {rim}, centre {center}"
        );
        assert!(
            rim > foot && foot > 0.0,
            "un versant : bord {rim}, pied {foot}"
        );
        assert_eq!(outside, 0.0);
        // Ne dépend pas de y.
        assert_eq!(g.sample(f, x + 12, 90, z), rim);
    }
    // Un masque négatif efface tout.
    let g = cones("Constant(-1.0)");
    let f = g.function("f").unwrap();
    assert!(g.cone_sites(f, (0, 0), (999, 999)).is_empty());
    assert_eq!(
        g.sample_many(f, (0..200).map(|i| IVec3::new(i * 5, 0, i * 3)))
            .iter()
            .sum::<f64>(),
        0.0
    );
    // Un carré trop petit pour le cône est refusé.
    let text = "(functions: { \"f\": Cones(cell: 100, chance: 1.0, mask: Constant(1.0), radius: 60.0, height: 50.0, crater_radius: 10.0, crater_depth: 20.0) })";
    assert!(matches!(
        DensityGraph::parse(text, 1),
        Err(DensityError::BadCones(_))
    ));
}
