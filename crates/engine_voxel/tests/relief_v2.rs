//! Le relief vertical de la génération v2 (lot 4) : parois, surplombs réels et arches, mesurés dans
//! les **blocs** générés, avec leurs garanties (rien ne flotte, rien dans les familles ou zones
//! interdites).
//!
//! Définitions (en blocs, lues dans les blocs et jamais dans le contexte de génération) :
//!
//! - **colonne à surplomb réel** : une colonne qui a, à moins de 30 blocs sous son sol, un bloc d'air
//!   sous un bloc plein (hors feuillage et bois), ouvert sur le côté (une colonne voisine est plus
//!   basse que ce bloc d'air et son bloc voisin est de l'air). Les cavernes fermées ne comptent pas ;
//!   l'**avancée** d'une colonne est le plus long alignement de ces blocs d'air sous un toit,
//!   mesuré vers l'intérieur depuis l'ouverture ;
//! - **colonne de paroi** : une colonne dont le sol domine d'au moins 8 blocs celui d'une colonne à un
//!   ou deux blocs d'elle ;
//! - **famille autorisée** : massif et piémont, ou côte haute (sol de la géographie macro à plus de
//!   25 blocs au-dessus de la mer), hors zones réservées et hors eau.
//!
//! `cargo test -p engine_voxel --test relief_v2 print_relief_metrics --release -- --ignored --nocapture`
//! imprime le tableau chiffré utilisé par le plan.

#[allow(dead_code)]
mod support {
    pub mod area;
    pub mod fingerprint;
    pub mod walls;
}

use engine_voxel::{GenerationVersion, ReliefFamily, SEA_LEVEL, TerrainGenerator};
use glam::IVec3;
use support::area::{Area, around, around_by, mountain_spots};
use support::fingerprint::{Keys, chunk_hash};
use support::walls::{self, Shape};

const SEEDS: [u32; 4] = [7, 42, 1337, 20260];
const NEIGHBOURS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

fn v2(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V2)
        .expect("la génération v2 s'initialise")
}

/// Le relief seul (ni cavernes, ni arbres, ni structures) : ce que les mesures de parois et de
/// surplombs doivent lire.
fn relief(seed: u32) -> TerrainGenerator {
    v2(seed).relief_only()
}

/// Des points répartis sur la carte (tous types de relief), pour chercher des structures là où il
/// ne devrait pas y en avoir.
fn sweep_spots(step: i32, reach: i32) -> Vec<(i32, i32)> {
    (-reach..=reach)
        .flat_map(|j| (-reach..=reach).map(move |i| (i * step, j * step)))
        .collect()
}

/// Ce que les blocs d'une zone disent de son relief vertical.
#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    columns: usize,
    /// Colonnes de paroi : de 8 blocs ou plus, de 16 ou plus, de 24 ou plus.
    walls: usize,
    walls16: usize,
    walls24: usize,
    overhang_columns: usize,
    overhang_cells: usize,
    advance_sum: usize,
    advance_max: usize,
    advance3: usize,
    advance6: usize,
    /// Dans une famille autorisée / ailleurs.
    walls_allowed: usize,
    walls_forbidden: usize,
    overhangs_allowed: usize,
    overhangs_forbidden: usize,
    /// Parois à moins de 200 blocs du rivage.
    coastal_walls: usize,
    /// Colonnes qui ont une arche : un passage d'air sous un toit, ouvert aux deux bouts.
    arches: usize,
}

impl Stats {
    fn add(&mut self, other: &Stats) {
        self.columns += other.columns;
        self.walls += other.walls;
        self.walls16 += other.walls16;
        self.walls24 += other.walls24;
        self.overhang_columns += other.overhang_columns;
        self.overhang_cells += other.overhang_cells;
        self.advance_sum += other.advance_sum;
        self.advance_max = self.advance_max.max(other.advance_max);
        self.advance3 += other.advance3;
        self.advance6 += other.advance6;
        self.walls_allowed += other.walls_allowed;
        self.walls_forbidden += other.walls_forbidden;
        self.overhangs_allowed += other.overhangs_allowed;
        self.overhangs_forbidden += other.overhangs_forbidden;
        self.coastal_walls += other.coastal_walls;
        self.arches += other.arches;
    }

    fn line(&self, label: &str) -> String {
        let share = |a: usize, b: usize| {
            if a + b == 0 {
                "  - ".to_string()
            } else {
                format!("{:3.0} %", 100.0 * a as f64 / (a + b) as f64)
            }
        };
        format!(
            "{label:>9} | colonnes {:>7} | parois {:>5} (>=16 {:>4}, >=24 {:>4}, côtières {:>3}) | surplombs {:>5} col. {:>6} blocs, avancée moy {:.1} max {:>2}, >=3 {:>4}, >=6 {:>4}, arches {:>3} | autorisé: parois {}, surplombs {} | interdit: {} parois, {} surplombs",
            self.columns,
            self.walls,
            self.walls16,
            self.walls24,
            self.coastal_walls,
            self.overhang_columns,
            self.overhang_cells,
            self.advance_sum as f64 / self.overhang_columns.max(1) as f64,
            self.advance_max,
            self.advance3,
            self.advance6,
            self.arches,
            share(self.walls_allowed, self.walls_forbidden),
            share(self.overhangs_allowed, self.overhangs_forbidden),
            self.walls_forbidden,
            self.overhangs_forbidden,
        )
    }
}

/// Un bloc plein de roche ou de sol (ni feuillage, ni bois, ni cactus) : un toit possible.
fn roof(area: &Area, x: i32, y: i32, z: i32) -> bool {
    let b = area.block(x, y, z);
    if b.is_air() || !area.is_solid(b) {
        return false;
    }
    let k = area.key(b);
    !(k.ends_with("_leaves") || k.ends_with("_log") || k == "core:cactus")
}

/// Un bloc d'air sous un toit : un bloc plein de roche ou de sol au-dessus dans la colonne, à
/// moins de 40 blocs.
fn sheltered(area: &Area, x: i32, y: i32, z: i32) -> bool {
    area.contains(x, z)
        && area.block(x, y, z).is_air()
        && (y + 1..y + 41).any(|h| roof(area, x, h, z))
}

/// La famille de relief autorisée à porter parois et surplombs, d'après la géographie macro.
fn allowed(generator: &TerrainGenerator, x: i32, z: i32) -> bool {
    let info = generator.surface_info(x, z).expect("v2");
    if info.reserved || info.water.is_some() || info.river_bed || info.lake_bed {
        return false;
    }
    let geo = generator
        .macro_world()
        .expect("v2")
        .geography()
        .sample(x as f64, z as f64);
    match geo.family {
        ReliefFamily::Massif | ReliefFamily::Foothills => true,
        ReliefFamily::Coast => geo.macro_elevation > SEA_LEVEL as f32 + 25.0,
        _ => false,
    }
}

/// Les mesures d'une zone, hors d'une bordure de 3 blocs (les voisines doivent exister).
fn analyse(area: &Area) -> Stats {
    analyse_with(area, &mut |_| {})
}

/// Une colonne remarquable d'une zone.
#[derive(Debug, Clone, Copy)]
struct Found {
    x: i32,
    z: i32,
    /// Une paroi (de ce dénivelé) ou un surplomb (de cette avancée, en blocs).
    kind: Kind,
    allowed: bool,
    arch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Wall(i32),
    Overhang(usize),
}

/// Comme [`analyse`], et signale chaque colonne de paroi ou de surplomb.
fn analyse_with(area: &Area, found: &mut dyn FnMut(Found)) -> Stats {
    let (x0, z0, x1, z1) = (area.min.0, area.min.1, area.max.0, area.max.1);
    let w = (x1 - x0 + 1) as usize;
    let idx = |x: i32, z: i32| (z - z0) as usize * w + (x - x0) as usize;
    let ground: Vec<i32> = (z0..=z1)
        .flat_map(|z| (x0..=x1).map(move |x| (x, z)))
        .map(|(x, z)| area.ground(x, z).map_or(-1, |(y, _)| y))
        .collect();
    // Les colonnes réservées ou noyées : les marches des lits et des lacs (hydrologie) ne sont pas des
    // parois de relief.
    let wet: Vec<bool> = (z0..=z1)
        .flat_map(|z| (x0..=x1).map(move |x| (x, z)))
        .map(|(x, z)| {
            let info = area.generator.surface_info(x, z).expect("v2");
            info.reserved || info.water.is_some()
        })
        .collect();
    let mut s = Stats::default();
    for z in z0 + 3..=z1 - 3 {
        for x in x0 + 3..=x1 - 3 {
            s.columns += 1;
            let g = ground[idx(x, z)];
            if g < 0 {
                continue;
            }
            let ok = allowed(area.generator, x, z);
            // Les parois.
            let drop = NEIGHBOURS
                .iter()
                .flat_map(|&(dx, dz)| (1..=2).map(move |k| (dx * k, dz * k)))
                .map(|(dx, dz)| g - ground[idx(x + dx, z + dz)])
                .max()
                .unwrap_or(0);
            let near_water = (-3..=3).any(|dz| (-3..=3).any(|dx| wet[idx(x + dx, z + dz)]));
            if drop >= 8 && !near_water {
                s.walls += 1;
                s.walls16 += usize::from(drop >= 16);
                s.walls24 += usize::from(drop >= 24);
                if ok {
                    s.walls_allowed += 1;
                } else {
                    s.walls_forbidden += 1;
                }
                found(Found {
                    x,
                    z,
                    kind: Kind::Wall(drop),
                    allowed: ok,
                    arch: false,
                });
                let geo = area
                    .generator
                    .macro_world()
                    .expect("v2")
                    .geography()
                    .sample(x as f64, z as f64);
                s.coastal_walls += usize::from(geo.coast_distance < 200.0);
            }
            // Les surplombs réels.
            let mut cells = 0;
            let mut advance = 0;
            let mut arch = false;
            for y in (g - 30).max(SEA_LEVEL + 1)..g {
                if !sheltered(area, x, y, z) {
                    continue;
                }
                let under_roof = roof(area, x, y + 1, z);
                let mut best = 0;
                for &(dx, dz) in &NEIGHBOURS {
                    let (nx, nz) = (x + dx, z + dz);
                    if ground[idx(nx, nz)] >= y || !area.block(nx, y, nz).is_air() {
                        continue;
                    }
                    // Vers l'intérieur : le sens opposé à l'ouverture, tant que l'air reste abrité.
                    let mut run = 0;
                    while run < 40 && sheltered(area, x - dx * run, y, z - dz * run) {
                        run += 1;
                    }
                    best = best.max(run);
                    // Une arche : le passage est ouvert à l'autre bout aussi.
                    let (ex, ez) = (x - dx * run, z - dz * run);
                    if run >= 2
                        && under_roof
                        && area.contains(ex, ez)
                        && area.block(ex, y, ez).is_air()
                        && ground[idx(ex, ez)] < y
                    {
                        arch = true;
                    }
                }
                // Un bloc d'air est un surplomb s'il touche un toit ; l'avancée se lit à toute hauteur.
                if best > 0 {
                    cells += usize::from(under_roof);
                    advance = advance.max(best as usize);
                }
            }
            s.arches += usize::from(arch);
            if cells > 0 {
                s.overhang_columns += 1;
                s.overhang_cells += cells;
                s.advance_sum += advance;
                s.advance_max = s.advance_max.max(advance);
                s.advance3 += usize::from(advance >= 3);
                s.advance6 += usize::from(advance >= 6);
                if ok {
                    s.overhangs_allowed += 1;
                } else {
                    s.overhangs_forbidden += 1;
                }
                found(Found {
                    x,
                    z,
                    kind: Kind::Overhang(advance),
                    allowed: ok,
                    arch,
                });
            }
        }
    }
    s
}

/// Les mesures des douze zones de massif (trois par graine, 160 × 160 blocs).
fn massif_stats(seed: u32) -> Stats {
    let generator = relief(seed);
    let mut total = Stats::default();
    for spot in mountain_spots(seed, 3) {
        total.add(&analyse(&around(&generator, spot)));
    }
    total
}

/// Les mesures d'un balayage de la carte (reliefs de tout type, 96 × 96 blocs chacun).
fn sweep_stats(seed: u32) -> Stats {
    let generator = relief(seed);
    let mut total = Stats::default();
    for spot in sweep_spots(900, 4) {
        total.add(&analyse(&around_by(&generator, spot, 1)));
    }
    total
}

#[test]
#[ignore = "affiche le tableau chiffré des parois et des surplombs"]
fn print_relief_metrics() {
    for (name, f) in [
        ("massifs", massif_stats as fn(u32) -> Stats),
        ("balayage", sweep_stats as fn(u32) -> Stats),
    ] {
        println!("--- {name} ---");
        let mut total = Stats::default();
        for seed in SEEDS {
            let s = f(seed);
            println!("{}", s.line(&format!("graine {seed}")));
            total.add(&s);
        }
        println!("{}", total.line("total"));
    }
    println!("--- forme des parois (massifs, carrés de {SHAPE_SIZE} blocs) ---");
    let (mut walls_total, mut cliffs_total) = (Shape::default(), Shape::default());
    for seed in SEEDS {
        println!("zones de la graine {seed} : {:?}", mountain_spots(seed, 3));
        let (w, c) = shape_stats(seed);
        println!("{}", w.line(&format!("parois {seed}")));
        println!("{}", c.line(&format!("falaises {seed}")));
        walls_total.add(&w);
        cliffs_total.add(&c);
    }
    println!("{}", walls_total.line("parois"));
    println!("{}", cliffs_total.line("falaises"));
}

#[test]
#[ignore = "liste les surplombs les plus avancés et les arches, et toute structure hors famille autorisée"]
fn list_remarkable_structures() {
    for seed in SEEDS {
        let generator = relief(seed);
        let mut best: Vec<Found> = Vec::new();
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            analyse_with(&area, &mut |f| {
                if !f.allowed {
                    println!("graine {seed} HORS FAMILLE {f:?}");
                }
                if matches!(f.kind, Kind::Overhang(a) if a >= 5) || f.arch {
                    best.push(f);
                }
            });
        }
        best.sort_by_key(|f| {
            std::cmp::Reverse(match f.kind {
                Kind::Overhang(a) => a + if f.arch { 100 } else { 0 },
                Kind::Wall(_) => 0,
            })
        });
        for f in best.iter().take(6) {
            println!("graine {seed} {f:?}");
        }
    }
}

/// Les composantes solides d'une zone qui ne touchent ni le fond du monde ni le bord de la zone (par
/// où elles pourraient se prolonger) : des blocs qui flottent. Renvoie, pour chacune, sa taille et un
/// de ses blocs.
fn floating(area: &Area) -> Vec<(usize, (i32, i32, i32))> {
    let (x0, z0) = area.min;
    let (w, h) = (
        (area.max.0 - x0 + 1) as usize,
        (area.max.1 - z0 + 1) as usize,
    );
    let height = (engine_voxel::WORLD_HEIGHT_CHUNKS * 32) as usize;
    let index = |x: usize, y: usize, z: usize| (y * h + z) * w + x;
    let solid: Vec<bool> = (0..height)
        .flat_map(|y| (0..h).flat_map(move |z| (0..w).map(move |x| (x, y, z))))
        .map(|(x, y, z)| {
            let b = area.block(x0 + x as i32, y as i32, z0 + z as i32);
            !b.is_air() && area.is_solid(b)
        })
        .collect();
    let mut seen = vec![false; solid.len()];
    let mut queue = std::collections::VecDeque::new();
    let flood = |queue: &mut std::collections::VecDeque<(usize, usize, usize)>,
                 seen: &mut Vec<bool>|
     -> usize {
        let mut size = 0;
        while let Some((x, y, z)) = queue.pop_front() {
            size += 1;
            let around = [
                (x.wrapping_sub(1), y, z),
                (x + 1, y, z),
                (x, y.wrapping_sub(1), z),
                (x, y + 1, z),
                (x, y, z.wrapping_sub(1)),
                (x, y, z + 1),
            ];
            for (nx, ny, nz) in around {
                if nx < w && ny < height && nz < h {
                    let i = index(nx, ny, nz);
                    if solid[i] && !seen[i] {
                        seen[i] = true;
                        queue.push_back((nx, ny, nz));
                    }
                }
            }
        }
        size
    };
    // Les sources : le fond du monde et les bords de la zone.
    for y in 0..height {
        for z in 0..h {
            for x in 0..w {
                let edge = x == 0 || z == 0 || x == w - 1 || z == h - 1;
                let i = index(x, y, z);
                if solid[i] && !seen[i] && (y <= 1 || edge) {
                    seen[i] = true;
                    queue.push_back((x, y, z));
                    flood(&mut queue, &mut seen);
                }
            }
        }
    }
    let mut out = Vec::new();
    for y in 0..height {
        for z in 0..h {
            for x in 0..w {
                let i = index(x, y, z);
                if solid[i] && !seen[i] {
                    seen[i] = true;
                    queue.push_back((x, y, z));
                    let size = flood(&mut queue, &mut seen);
                    out.push((size, (x0 + x as i32, y as i32, z0 + z as i32)));
                }
            }
        }
    }
    out
}

// ----- Les garanties -----

/// Le total des mesures de tous les massifs échantillonnés.
/// Côté (blocs) des carrés de massif où l'on mesure la forme des parois.
const SHAPE_SIZE: usize = 400;

/// La forme des parois sur les trois zones de massif de la graine (carrés de 400 blocs).
fn shape_stats(seed: u32) -> (Shape, Shape) {
    let generator = relief(seed);
    let (mut walls_total, mut cliffs_total) = (Shape::default(), Shape::default());
    for (x, z) in mountain_spots(seed, 3) {
        let half = SHAPE_SIZE as i32 / 2;
        let (w, c) = walls::shape(&generator, x - half, z - half, SHAPE_SIZE);
        walls_total.add(&w);
        cliffs_total.add(&c);
    }
    (walls_total, cliffs_total)
}

fn massif_total() -> Stats {
    let mut total = Stats::default();
    for seed in SEEDS {
        total.add(&massif_stats(seed));
    }
    total
}

#[test]
fn walls_and_overhangs_are_numerous_in_massifs_but_stay_localized() {
    let total = massif_total();
    // Avant le réglage des escarpements : 17 colonnes de paroi et 90 blocs de surplomb sur ces
    // mêmes douze zones (relief seul).
    assert!(total.walls >= 2_000, "parois : {}", total.walls);
    assert!(
        total.walls16 >= 800,
        "parois de 16 blocs : {}",
        total.walls16
    );
    assert!(
        total.walls24 >= 200,
        "parois de 24 blocs : {}",
        total.walls24
    );
    assert!(
        total.overhang_cells >= 800,
        "blocs de surplomb : {}",
        total.overhang_cells
    );
    assert!(
        total.advance3 >= 400,
        "colonnes à plus de 3 blocs d'avancée : {}",
        total.advance3
    );
    assert!(
        total.advance_sum >= total.overhang_columns * 4,
        "avancée moyenne trop courte : {} pour {} colonnes",
        total.advance_sum,
        total.overhang_columns
    );
    assert!(total.arches >= 3, "arches : {}", total.arches);
    // Localisé : les parois couvrent moins d'une colonne sur dix, les surplombs moins d'une sur cent.
    assert!(
        total.walls * 10 < total.columns,
        "{} parois pour {} colonnes",
        total.walls,
        total.columns
    );
    assert!(
        total.overhang_columns * 100 < total.columns,
        "{} colonnes à surplomb pour {} colonnes",
        total.overhang_columns,
        total.columns
    );
}

#[test]
fn wall_shapes_are_long_straight_and_do_not_loop() {
    // La forme des parois, lue dans les hauteurs : peu de composantes longues et droites, pas d'anneau,
    // pas de paroi doublée à moins de 24 blocs (hors crêtes à arche). Avant les parois posées sur la
    // structure macro (isolignes d'un bruit de 140 blocs) : sur ces mêmes zones, 0 anneau de paroi,
    // 26 doublages pour 1 000 colonnes et une sinuosité de 1,29 ; et pour les bandes de falaise
    // (pente de 1,0 ou plus, la roche nue des cartes de matériau) 54 anneaux, 129 doublages pour
    // 1 000 colonnes et une sinuosité de 2,50.
    let (mut wall, mut cliff) = (Shape::default(), Shape::default());
    for seed in SEEDS {
        let (w, c) = shape_stats(seed);
        wall.add(&w);
        cliff.add(&c);
    }
    assert!(wall.walls > 8_000, "{}", wall.line("parois"));
    assert!(wall.rings <= 2, "{}", wall.line("parois"));
    assert!(wall.doubled_per_1000() <= 20.0, "{}", wall.line("parois"));
    assert!(wall.sinuosity() <= 1.3, "{}", wall.line("parois"));
    // Des parois longues : moins de composantes que de colonnes de paroi par 100.
    assert!(wall.parts * 100 < wall.walls, "{}", wall.line("parois"));
    assert!(cliff.rings <= 12, "{}", cliff.line("falaises"));
    assert!(
        cliff.doubled_per_1000() <= 100.0,
        "{}",
        cliff.line("falaises")
    );
    assert!(cliff.sinuosity() <= 1.9, "{}", cliff.line("falaises"));
}

#[test]
fn walls_and_overhangs_exist_only_in_allowed_families() {
    // Les massifs : tout est dans une famille autorisée. Le balayage de la carte (plaines, plateaux,
    // côtes basses, mers, vallées) : rien hors de ces familles, ni dans une zone réservée ou un lit.
    for stats in [massif_total(), {
        let mut total = Stats::default();
        for seed in SEEDS {
            total.add(&sweep_stats(seed));
        }
        total
    }] {
        assert_eq!(stats.walls_forbidden, 0, "{}", stats.line("parois"));
        assert_eq!(stats.overhangs_forbidden, 0, "{}", stats.line("surplombs"));
        assert!(stats.walls_allowed > 0 && stats.overhangs_allowed > 0);
    }
}

#[test]
fn relief_overhangs_never_float() {
    // Chaque composante solide touche le fond du monde ou le bord de la zone : le toit d'un
    // surplomb, une arche et le haut d'une paroi sont tous reliés au sol.
    for seed in SEEDS {
        let generator = relief(seed);
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            let pieces = floating(&area);
            assert!(
                pieces.is_empty(),
                "graine {seed}, zone {spot:?} : blocs flottants {pieces:?}"
            );
        }
    }
}

#[test]
fn full_generation_never_floats_near_overhangs() {
    // Avec cavernes, minerais, arbres et structures : aucun bloc plein flottant à moins de 12 blocs
    // d'une colonne à surplomb, à hauteur de l'entaille. (Une caverne peut isoler un bloc plus bas :
    // c'est le fait des cavernes de la v1, pas des parois ; la v2 ne les a pas changées.)
    let mut examined = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            for (size, (px, py, pz)) in floating(&area) {
                let near = (-12..=12).any(|dz| {
                    (-12..=12).any(|dx| {
                        let (x, z) = (px + dx, pz + dz);
                        area.contains(x, z)
                            && generator.surface_info(x, z).is_some_and(|info| {
                                // L'entaille tient entre le pied de la paroi et son sommet.
                                info.overhang > 0.0
                                    && py >= info.relief_top - info.escarpment as i32 - 2
                            })
                    })
                });
                examined += 1;
                assert!(
                    !near,
                    "graine {seed} : {size} bloc(s) flottant(s) en ({px}, {py}, {pz}) près d'un surplomb"
                );
            }
        }
    }
    println!("blocs flottants hors surplomb (cavernes) : {examined}");
}

/// Un bloc de roche (pierre, grès, roche volcanique, minerais) ou de gravier : la matière d'une
/// paroi, jamais une herbe, une neige, un sable ou une terre.
fn rocky(key: &str) -> bool {
    matches!(
        key,
        "core:stone"
            | "core:deepstone"
            | "core:sandstone"
            | "core:volcanic_rock"
            | "core:cobblestone"
            | "core:gravel"
    ) || key.ends_with("_ore")
}

#[test]
fn walls_are_rock_not_grass() {
    let (mut faces, mut rock) = (0, 0);
    for seed in SEEDS {
        let generator = relief(seed);
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            for z in area.min.1 + 3..=area.max.1 - 3 {
                for x in area.min.0 + 3..=area.max.0 - 3 {
                    let Some((g, _)) = area.ground(x, z) else {
                        continue;
                    };
                    for (dx, dz) in NEIGHBOURS {
                        let Some((low, _)) = area.ground(x + dx, z + dz) else {
                            continue;
                        };
                        // La face d'une paroi de 8 blocs ou plus : les blocs exposés sur le côté.
                        if g - low < 8 {
                            continue;
                        }
                        for y in low + 1..=g {
                            let b = area.block(x, y, z);
                            if area.is_solid(b) {
                                faces += 1;
                                rock += usize::from(rocky(area.key(b)));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(faces > 5_000, "faces de paroi : {faces}");
    // Les filons de terre ou de gravier des minerais affleurent parfois : au plus 3 %.
    assert!(
        rock as f64 >= faces as f64 * 0.97,
        "faces de paroi sans roche : {rock}/{faces}"
    );
}

#[test]
fn trees_never_grow_on_overhang_columns() {
    let mut columns = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    if info.overhang > 0.0 {
                        columns += 1;
                        let above = area.block(x, info.top + 1, z);
                        assert!(
                            !area.key(above).ends_with("_log"),
                            "graine {seed}, ({x}, {z}) : un arbre sur une colonne à surplomb"
                        );
                    }
                }
            }
        }
    }
    assert!(columns > 500, "colonnes à surplomb examinées : {columns}");
}

#[test]
fn wall_zones_are_deterministic_in_any_order_cache_state_and_thread() {
    // Trois colonnes de chunks autour d'un des surplombs les plus avancés de chaque graine.
    for seed in [42, 20260] {
        let generator = relief(seed);
        let mut spots: Vec<Found> = Vec::new();
        for spot in mountain_spots(seed, 3) {
            analyse_with(&around(&generator, spot), &mut |f| {
                if matches!(f.kind, Kind::Overhang(a) if a >= 6) {
                    spots.push(f);
                }
            });
        }
        let at = spots.first().expect("un surplomb avancé");
        let (cx, cz) = (at.x.div_euclid(32), at.z.div_euclid(32));
        let positions: Vec<IVec3> = (cz - 1..=cz + 1)
            .flat_map(|z| (cx - 1..=cx + 1).map(move |x| (x, z)))
            .flat_map(|(x, z)| (0..6).map(move |y| IVec3::new(x, y, z)))
            .collect();
        let keys = Keys::of(&generator);
        let forward: Vec<u64> = positions
            .iter()
            .map(|&p| chunk_hash(&generator, &keys, p))
            .collect();
        // Ordre inverse, caches vidés avant chaque chunk (le contexte est recalculé à froid).
        let backward: Vec<u64> = positions
            .iter()
            .rev()
            .map(|&p| {
                generator.clear_caches();
                chunk_hash(&generator, &keys, p)
            })
            .collect();
        assert_eq!(
            forward,
            backward.into_iter().rev().collect::<Vec<_>>(),
            "graine {seed} : ordre ou caches"
        );
        // Quatre threads sur un générateur neuf, chacun sur un quart des chunks.
        let fresh = relief(seed);
        let fresh_keys = Keys::of(&fresh);
        let mut parallel = vec![0u64; positions.len()];
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|t| {
                    let (fresh, fresh_keys, positions) = (&fresh, &fresh_keys, &positions);
                    scope.spawn(move || {
                        (t..positions.len())
                            .step_by(4)
                            .map(|i| (i, chunk_hash(fresh, fresh_keys, positions[i])))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for handle in handles {
                for (i, hash) in handle.join().expect("thread de génération") {
                    parallel[i] = hash;
                }
            }
        });
        assert_eq!(forward, parallel, "graine {seed} : threads");
    }
}

#[test]
fn wall_data_is_consistent_across_chunk_seams() {
    // La pente d'une colonne lit ses voisines à ±2 blocs, jusque dans la marge du contexte voisin :
    // elle doit valoir celle qu'on recalcule avec les hauteurs que ces voisines annoncent chez elles.
    // La distance à la ligne de rupture varie d'au plus 2,5 blocs entre deux colonnes voisines à moins
    // de 3 blocs de la ligne, joints de colonnes de chunks compris : aucun saut là où deux contextes se
    // rejoignent.
    let (mut seams, mut walled) = (0, 0);
    for seed in SEEDS {
        let generator = relief(seed);
        for spot in mountain_spots(seed, 3) {
            let (cx, cz) = (spot.0.div_euclid(32), spot.1.div_euclid(32));
            let (x0, z0) = ((cx - 1) * 32 + 4, (cz - 1) * 32 + 4);
            for z in z0..z0 + 88 {
                for x in x0..x0 + 88 {
                    let at = |dx: i32, dz: i32| generator.surface_info(x + dx, z + dz).unwrap();
                    let info = at(0, 0);
                    let gx = (at(2, 0).relief_top - at(-2, 0).relief_top) as f32 / 4.0;
                    let gz = (at(0, 2).relief_top - at(0, -2).relief_top) as f32 / 4.0;
                    // La hauteur annoncée est le sol réel : sans entaille, c'est le relief.
                    if info.overhang <= 0.0 {
                        assert!(
                            (gx.hypot(gz) - info.slope).abs() < 1e-4,
                            "graine {seed}, ({x}, {z}) : pente {} contre {}",
                            info.slope,
                            gx.hypot(gz)
                        );
                    }
                    let seam = x.rem_euclid(32) == 31 || z.rem_euclid(32) == 31;
                    if info.escarpment > 0.0 {
                        walled += 1;
                        for (dx, dz) in [(1, 0), (0, 1)] {
                            let next = at(dx, dz);
                            // Près de la ligne seulement (3 blocs) : plus loin, l'estimation de la distance
                            // (bornée à 16) n'est plus linéaire.
                            if next.escarpment > 0.0
                                && info.wall_distance.abs() < 3.0
                                && next.wall_distance.abs() < 3.0
                            {
                                assert!(
                                    (next.wall_distance - info.wall_distance).abs() <= 2.5,
                                    "graine {seed}, ({x}, {z}) : distance {} puis {}",
                                    info.wall_distance,
                                    next.wall_distance
                                );
                                seams += usize::from(seam);
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        walled > 10_000 && seams > 50,
        "{walled} colonnes, {seams} joints"
    );
}
