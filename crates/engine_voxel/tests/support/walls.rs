//! La **forme** des parois d'une zone, lue dans les hauteurs de sol générées : combien de parois
//! distinctes, si elles bouclent, si elles se doublent, si elles serpentent. Partagé par
//! `relief_v2.rs` (mesures et bornes testées).
//!
//! Définitions (une colonne de paroi domine d'au moins 8 blocs une colonne à un ou deux blocs d'elle ;
//! les colonnes proches d'une zone réservée ou noyée ne comptent pas : ce sont des marches
//! d'hydrologie) :
//!
//! - **composante** : un ensemble connexe (8 voisins) de colonnes de paroi dilatées de 2 blocs, d'au
//!   moins 60 colonnes ; une paroi et son pied ne font qu'une composante ;
//! - **anneau** : une région de sol d'au moins 150 colonnes entièrement entourée de parois, sans
//!   toucher le bord de la zone (un massif cerné de murs, une boucle de courbe de niveau) ;
//! - **doublage** : deux parois séparées de 3 à 23 blocs sur une même ligne ou colonne de blocs (un
//!   mur mince, une paroi doublée, une courbe qui repasse) ;
//! - **sinuosité** : le périmètre des composantes divisé par le double de leur étendue (la plus
//!   grande dimension de leur boîte englobante). Une paroi droite et épaisse de quelques blocs vaut 1 ;
//!   une boucle ou un ver vaut davantage.

use engine_voxel::TerrainGenerator;

/// Seuil de dénivelé d'une colonne de paroi, en blocs.
const WALL: i32 = 8;
/// Taille minimale (colonnes dilatées) d'une composante de paroi comptée.
const MIN_PART: usize = 60;
/// Surface minimale (colonnes) d'un anneau compté.
const MIN_RING: usize = 150;
/// Les écarts (blocs) entre deux parois qui font un doublage.
const DOUBLED_GAP: std::ops::Range<usize> = 3..24;

/// Les mesures de forme d'une ou plusieurs zones.
#[derive(Debug, Default, Clone, Copy)]
pub struct Shape {
    /// Colonnes de paroi.
    pub walls: usize,
    /// Composantes de paroi (dilatées) d'au moins 60 colonnes.
    pub parts: usize,
    /// Régions de sol entourées de parois.
    pub rings: usize,
    /// Paires de parois voisines à moins de 24 blocs, sur les lignes et les colonnes de blocs, hors
    /// celles dont l'intervalle est entaillé (une crête fine à arche : voulue).
    pub doubled: usize,
    /// Les paires voisines dont l'intervalle est entaillé (arches voulues).
    pub passages: usize,
    /// Somme des périmètres et des doubles étendues des composantes.
    pub perimeter: usize,
    pub extent2: usize,
}

impl Shape {
    pub fn add(&mut self, other: &Shape) {
        self.walls += other.walls;
        self.parts += other.parts;
        self.rings += other.rings;
        self.doubled += other.doubled;
        self.passages += other.passages;
        self.perimeter += other.perimeter;
        self.extent2 += other.extent2;
    }

    /// Le périmètre rapporté au double de l'étendue : 1 pour une paroi droite, davantage pour des
    /// boucles et des vers.
    pub fn sinuosity(&self) -> f64 {
        self.perimeter as f64 / self.extent2.max(1) as f64
    }

    /// Les doublages pour 1 000 colonnes de paroi.
    pub fn doubled_per_1000(&self) -> f64 {
        1000.0 * self.doubled as f64 / self.walls.max(1) as f64
    }

    pub fn line(&self, label: &str) -> String {
        format!(
            "{label:>11} | colonnes de paroi {:>6} | composantes {:>3} | anneaux {:>2} | doublages {:>4} ({:.0} pour 1000, plus {} de crêtes à arche) | sinuosité {:.2}",
            self.walls,
            self.parts,
            self.rings,
            self.doubled,
            self.doubled_per_1000(),
            self.passages,
            self.sinuosity(),
        )
    }
}

/// Les hauteurs du sol et les colonnes noyées ou réservées d'un carré de `size` blocs de côté dont le
/// coin est `(x0, z0)`.
fn read(
    generator: &TerrainGenerator,
    x0: i32,
    z0: i32,
    size: usize,
) -> (Vec<i32>, Vec<bool>, Vec<f32>, Vec<bool>) {
    let mut ground = Vec::with_capacity(size * size);
    let mut wet = Vec::with_capacity(size * size);
    let mut slope = Vec::with_capacity(size * size);
    let mut notch = Vec::with_capacity(size * size);
    for z in 0..size as i32 {
        for x in 0..size as i32 {
            ground.push(generator.column(x0 + x, z0 + z).height);
            let info = generator.surface_info(x0 + x, z0 + z).expect("v2");
            wet.push(info.reserved || info.water.is_some());
            slope.push(info.slope);
            notch.push(info.overhang > 0.0);
        }
    }
    (ground, wet, slope, notch)
}

/// La forme des parois (dénivelé de 8 blocs ou plus) et des bandes de falaise (pente de 1,0 ou plus :
/// la roche nue des cartes de matériau) du carré de `size` blocs de côté dont le coin est `(x0, z0)`.
pub fn shape(generator: &TerrainGenerator, x0: i32, z0: i32, size: usize) -> (Shape, Shape) {
    let (ground, wet, slope, notch) = read(generator, x0, z0, size);
    let cliffs: Vec<bool> = slope.iter().map(|&s| s >= CLIFF).collect();
    (
        shape_of(&ground, &wet, &notch, size, None),
        shape_of(&ground, &wet, &notch, size, Some(&cliffs)),
    )
}

/// Pente (blocs par bloc) à partir de laquelle la surface est de la roche nue.
const CLIFF: f32 = 1.0;

/// La forme d'un masque : les parois (dénivelé) si `mask` est `None`, sinon les colonnes de `mask`.
fn shape_of(
    ground: &[i32],
    wet: &[bool],
    notch: &[bool],
    n: usize,
    mask: Option<&[bool]>,
) -> Shape {
    let at = |x: usize, z: usize| z * n + x;
    // Les colonnes de paroi (une bordure de 3 blocs écartée : les voisines doivent exister).
    let mut wall = vec![false; n * n];
    for z in 3..n - 3 {
        for x in 3..n - 3 {
            let g = ground[at(x, z)];
            let drop = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .flat_map(|&(dx, dz)| (1..=2).map(move |k| (dx * k, dz * k)))
                .map(|(dx, dz)| g - ground[at((x as i32 + dx) as usize, (z as i32 + dz) as usize)])
                .max()
                .unwrap_or(0);
            let near_water = (z - 3..=z + 3).any(|nz| (x - 3..=x + 3).any(|nx| wet[at(nx, nz)]));
            wall[at(x, z)] = !near_water
                && match mask {
                    Some(m) => m[at(x, z)],
                    None => drop >= WALL,
                };
        }
    }
    let mut s = Shape {
        walls: wall.iter().filter(|&&w| w).count(),
        ..Shape::default()
    };

    // Les doublages : sur chaque ligne puis chaque colonne, deux suites de colonnes de paroi
    // (réunies si elles sont à 2 blocs ou moins) séparées de 3 à 23 blocs.
    let mut scan = |cell: &dyn Fn(usize, usize) -> bool, notched: &dyn Fn(usize, usize) -> bool| {
        for a in 0..n {
            let mut last_end: Option<usize> = None;
            let mut run_end: Option<usize> = None;
            for b in 0..n {
                if cell(a, b) {
                    match run_end {
                        Some(e) if b - e <= 2 => {}
                        _ => {
                            if let Some(e) = run_end {
                                last_end = Some(e);
                            }
                            if let Some(e) = last_end
                                && DOUBLED_GAP.contains(&(b - e - 1))
                            {
                                if (e + 1..b).any(|k| notched(a, k)) {
                                    s.passages += 1;
                                } else {
                                    s.doubled += 1;
                                }
                            }
                        }
                    }
                    run_end = Some(b);
                }
            }
        }
    };
    scan(&|a, b| wall[at(b, a)], &|a, b| notch[at(b, a)]);
    scan(&|a, b| wall[at(a, b)], &|a, b| notch[at(a, b)]);

    // La dilatation de 2 blocs.
    let mut dil = vec![false; n * n];
    for z in 0..n {
        for x in 0..n {
            if wall[at(x, z)] {
                for nz in z.saturating_sub(2)..=(z + 2).min(n - 1) {
                    for nx in x.saturating_sub(2)..=(x + 2).min(n - 1) {
                        dil[at(nx, nz)] = true;
                    }
                }
            }
        }
    }

    // Les composantes de paroi (8 voisins) : étendue et périmètre.
    let mut seen = vec![false; n * n];
    for start in 0..n * n {
        if !dil[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut count, mut perimeter) = (0usize, 0usize);
        let (mut minx, mut maxx, mut minz, mut maxz) = (n, 0, n, 0);
        while let Some(i) = stack.pop() {
            let (x, z) = (i % n, i / n);
            count += 1;
            minx = minx.min(x);
            maxx = maxx.max(x);
            minz = minz.min(z);
            maxz = maxz.max(z);
            let edge = x == 0 || z == 0 || x == n - 1 || z == n - 1;
            if edge
                || !dil[at(x - 1, z)]
                || !dil[at(x + 1, z)]
                || !dil[at(x, z - 1)]
                || !dil[at(x, z + 1)]
            {
                perimeter += 1;
            }
            for nz in z.saturating_sub(1)..=(z + 1).min(n - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(n - 1) {
                    let j = at(nx, nz);
                    if dil[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if count >= MIN_PART {
            s.parts += 1;
            s.perimeter += perimeter;
            s.extent2 += 2 * (maxx - minx + 1).max(maxz - minz + 1);
        }
    }

    // Les anneaux : les régions de sol (4 voisins) qui ne touchent pas le bord de la zone.
    let mut seen = vec![false; n * n];
    for start in 0..n * n {
        if dil[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut count, mut touches) = (0usize, false);
        while let Some(i) = stack.pop() {
            let (x, z) = (i % n, i / n);
            count += 1;
            touches |= x == 0 || z == 0 || x == n - 1 || z == n - 1;
            for (nx, nz) in [
                (x.wrapping_sub(1), z),
                (x + 1, z),
                (x, z.wrapping_sub(1)),
                (x, z + 1),
            ] {
                if nx < n && nz < n {
                    let j = at(nx, nz);
                    if !dil[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        if !touches && count >= MIN_RING {
            s.rings += 1;
        }
    }
    s
}
