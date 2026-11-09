//! Les tests de la simulation, dans un petit monde en plein ciel (y ≈ 110 à 150) : on y
//! construit des bassins de pierre et on y verse de l'eau.

use glam::Vec3;

use super::*;
use crate::fluids::FluidRegistry;
use crate::generator::TerrainGenerator;
use crate::streaming::{StreamingConfig, tests::pump_until_idle};

fn world() -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: 2,
        ..StreamingConfig::default()
    };
    let mut s = ChunkStreamer::new(TerrainGenerator::new(7), config);
    pump_until_idle(&mut s, Vec3::new(5.0, 120.0, 5.0));
    s
}

fn sim(s: &ChunkStreamer) -> FluidSim {
    FluidSim::new(s.world().registry(), &FluidRegistry::builtin())
}

fn id(s: &ChunkStreamer, key: &str) -> BlockId {
    s.world().registry().id(key).unwrap()
}

/// Un bassin de pierre : un sol de `w × d` et des murs de `h` blocs, coin en `origin`.
fn basin(s: &mut ChunkStreamer, origin: IVec3, w: i32, d: i32, h: i32) {
    let stone = id(s, "core:stone");
    for x in -1..=w {
        for z in -1..=d {
            s.set_block(origin + IVec3::new(x, -1, z), stone);
            for y in 0..h {
                if x == -1 || z == -1 || x == w || z == d {
                    s.set_block(origin + IVec3::new(x, y, z), stone);
                }
            }
        }
    }
}

/// Verse `amount` d'eau finie en `pos`.
fn pour(s: &mut ChunkStreamer, sim: &mut FluidSim, pos: IVec3, amount: i32) {
    let water = id(s, "core:water");
    assert!(s.set_fluid(pos, water, Some(amount as u16)));
    sim.wake(pos);
}

/// Fait tourner jusqu'à ce que tout dorme ; renvoie le nombre d'étapes.
fn settle(s: &mut ChunkStreamer, sim: &mut FluidSim, max: usize) -> usize {
    for step in 0..max {
        sim.step(s, Duration::from_secs(1));
        if sim.active() == 0 {
            return step + 1;
        }
    }
    panic!(
        "toujours {} cellules actives après {max} étapes",
        sim.active()
    );
}

fn water_kind(sim: &FluidSim) -> usize {
    sim.kinds()
        .iter()
        .position(|k| k.key == "core:water")
        .unwrap()
}

#[test]
fn volume_is_conserved_and_spreads_flat() {
    let mut s = world();
    let mut sim = sim(&s);
    let origin = IVec3::new(2, 120, 2);
    basin(&mut s, origin, 6, 6, 4);
    // 9 blocs d'eau versés dans un coin, en hauteur.
    for k in 0..9 {
        pour(
            &mut s,
            &mut sim,
            origin + IVec3::new(0, 3, 0) + IVec3::new(k % 3, 0, k / 3),
            FULL,
        );
    }
    let kind = water_kind(&sim);
    let (min, max) = (origin - IVec3::ONE, origin + IVec3::new(7, 5, 7));
    let before = sim.total(&s, kind, min, max);
    settle(&mut s, &mut sim, 3000);
    assert_eq!(
        sim.total(&s, kind, min, max),
        before,
        "du volume s'est perdu ou créé"
    );
    // 9 blocs sur 36 cases : un quart de bloc partout, à peu de chose près (surface plate).
    for x in 0..6 {
        for z in 0..6 {
            let amount = s.fluid_amount(origin + IVec3::new(x, 0, z)).unwrap_or(0) as i32;
            assert!(
                (amount - FULL / 4).abs() < FULL / 10,
                "({x}, {z}) : {amount}"
            );
        }
    }
}

#[test]
fn a_puddle_does_not_spread_forever() {
    let mut s = world();
    let mut sim = sim(&s);
    let stone = id(&s, "core:stone");
    let center = IVec3::new(10, 130, 10);
    // Un grand sol plat, sans murs.
    for x in -12..=12 {
        for z in -12..=12 {
            s.set_block(center + IVec3::new(x, -1, z), stone);
        }
    }
    pour(&mut s, &mut sim, center, FULL);
    settle(&mut s, &mut sim, 3000);
    let wet = (-12..=12)
        .flat_map(|x| (-12..=12).map(move |z| (x, z)))
        .filter(|&(x, z)| s.fluid_amount(center + IVec3::new(x, 0, z)).is_some())
        .count();
    // La tension de surface arrête l'étalement : au plus 1 / (1/16) = 16 cases... et
    // quelques-unes de plus, plus minces, au bord.
    assert!(wet <= 25, "la flaque couvre {wet} cases");
    let kind = water_kind(&sim);
    assert_eq!(
        sim.total(
            &s,
            kind,
            center - IVec3::new(12, 1, 12),
            center + IVec3::new(12, 1, 12)
        ),
        FULL as i64
    );
}

/// Un tube en U : deux puits reliés par le fond. L'eau versée d'un côté remonte de
/// l'autre jusqu'au même niveau (les vases communicants).
#[test]
fn communicating_vessels() {
    let mut s = world();
    let mut sim = sim(&s);
    let stone = id(&s, "core:stone");
    let base = IVec3::new(4, 110, 4);
    // Un bloc de pierre plein, creusé : deux puits de 1 × 1 sur 8 de haut, et un tunnel
    // de 4 blocs qui les relie en bas.
    for x in -1..=6 {
        for z in -1..=1 {
            for y in -1..=9 {
                s.set_block(base + IVec3::new(x, y, z), stone);
            }
        }
    }
    for y in 0..9 {
        s.set_block(base + IVec3::new(0, y, 0), BlockId::AIR);
        s.set_block(base + IVec3::new(5, y, 0), BlockId::AIR);
    }
    for x in 0..=5 {
        s.set_block(base + IVec3::new(x, 0, 0), BlockId::AIR);
    }
    // 10 blocs d'eau dans le puits de gauche.
    for y in 0..9 {
        pour(&mut s, &mut sim, base + IVec3::new(0, y, 0), FULL);
    }
    pour(&mut s, &mut sim, base + IVec3::new(1, 0, 0), FULL);
    settle(&mut s, &mut sim, 20_000);
    // La hauteur d'eau de chaque puits : sa quantité totale, en blocs.
    let column = |x: i32| -> f32 {
        (1..9)
            .map(|y| s.fluid_amount(base + IVec3::new(x, y, 0)).unwrap_or(0) as f32)
            .sum::<f32>()
            / FULL as f32
    };
    let (left, right) = (column(0), column(5));
    assert!(
        right > 1.0,
        "l'eau n'est pas remontée à droite ({left:.2} / {right:.2})"
    );
    assert!(
        (left - right).abs() < 0.6,
        "niveaux différents : {left:.2} / {right:.2}"
    );
}

#[test]
fn sealed_water_does_not_leak_and_falls_asleep() {
    let mut s = world();
    let mut sim = sim(&s);
    let origin = IVec3::new(20, 125, 20);
    basin(&mut s, origin, 2, 2, 3);
    let stone = id(&s, "core:stone");
    for x in 0..2 {
        for z in 0..2 {
            s.set_block(origin + IVec3::new(x, 2, z), stone); // un couvercle
        }
    }
    pour(&mut s, &mut sim, origin, FULL * 3);
    let steps = settle(&mut s, &mut sim, 5000);
    assert!(steps < 5000);
    let kind = water_kind(&sim);
    assert_eq!(
        sim.total(&s, kind, origin - IVec3::splat(3), origin + IVec3::splat(5)),
        (FULL * 3) as i64
    );
    // Rien en dehors du bassin.
    for y in -2..4 {
        assert!(s.fluid_amount(origin + IVec3::new(-2, y, 0)).is_none());
    }
}

/// Un réservoir (l'eau générée) remplit un trou voisin sans se vider, puis se rendort.
#[test]
fn reservoirs_fill_holes_and_stay_full() {
    let mut s = world();
    let mut sim = sim(&s);
    let water = id(&s, "core:water");
    let origin = IVec3::new(-10, 118, -10);
    basin(&mut s, origin, 4, 1, 2);
    // Trois cellules de réservoir (sans quantité enregistrée) et un trou à côté.
    for x in 0..3 {
        s.set_block(origin + IVec3::new(x, 0, 0), water);
    }
    sim.wake(origin + IVec3::new(3, 0, 0));
    settle(&mut s, &mut sim, 2000);
    for x in 0..3 {
        assert_eq!(
            s.fluid_amount(origin + IVec3::new(x, 0, 0)),
            None,
            "le réservoir doit rester plein"
        );
    }
    let filled = s.fluid_amount(origin + IVec3::new(3, 0, 0)).unwrap_or(0) as i32;
    assert!(filled > FULL * 3 / 4, "le trou n'est rempli qu'à {filled}");
}

/// Du fluide versé au bord d'un chunk passe dans le voisin, sans perte.
#[test]
fn fluid_crosses_chunk_borders() {
    let mut s = world();
    let mut sim = sim(&s);
    // Le bassin chevauche x = 32 (la frontière entre les chunks 0 et 1).
    let origin = IVec3::new(28, 122, 3);
    basin(&mut s, origin, 8, 1, 2);
    pour(&mut s, &mut sim, origin, FULL * 4);
    settle(&mut s, &mut sim, 4000);
    let kind = water_kind(&sim);
    assert_eq!(
        sim.total(&s, kind, origin - IVec3::ONE, origin + IVec3::new(9, 2, 2)),
        (FULL * 4) as i64
    );
    assert!(
        s.fluid_amount(origin + IVec3::new(7, 0, 0)).is_some(),
        "l'eau n'a pas atteint l'autre chunk"
    );
}

/// La lave au contact de l'eau fige en roche volcanique.
#[test]
fn lava_meeting_water_turns_to_rock() {
    let mut s = world();
    let mut sim = sim(&s);
    let (water, lava, rock) = (
        id(&s, "core:water"),
        id(&s, "core:lava"),
        id(&s, "core:volcanic_rock"),
    );
    let origin = IVec3::new(-20, 120, 8);
    basin(&mut s, origin, 2, 1, 1);
    s.set_fluid(origin, water, Some(FULL as u16));
    s.set_fluid(origin + IVec3::X, lava, Some(FULL as u16));
    sim.wake(origin);
    for _ in 0..20 {
        sim.step(&mut s, Duration::from_secs(1));
    }
    assert_eq!(s.world().block(origin + IVec3::X), rock);
}

/// Le seau : il prend la quantité d'une cellule et la reverse ailleurs, sans perte.
#[test]
fn a_bucket_moves_a_cell_of_fluid() {
    let mut s = world();
    let mut sim = sim(&s);
    let origin = IVec3::new(-30, 125, -30);
    basin(&mut s, origin, 3, 1, 2);
    pour(&mut s, &mut sim, origin, FULL * 3 / 4);
    let kind = water_kind(&sim);
    let (taken_kind, taken) = sim.take(&mut s, origin).unwrap();
    assert_eq!((taken_kind, taken), (kind, FULL * 3 / 4));
    assert!(s.world().block(origin).is_air());
    // Reversé deux cases plus loin : la même quantité, rien de perdu.
    assert_eq!(sim.pour(&mut s, origin + IVec3::X * 2, kind, taken), taken);
    settle(&mut s, &mut sim, 2000);
    assert_eq!(
        sim.total(&s, kind, origin - IVec3::ONE, origin + IVec3::new(4, 2, 2)),
        taken as i64
    );
    // Un réservoir donne un bloc plein et reste un réservoir ailleurs.
    let water = id(&s, "core:water");
    s.set_fluid(origin, BlockId::AIR, None);
    s.set_block(origin, water);
    assert_eq!(sim.take(&mut s, origin).unwrap().1, FULL);
}
