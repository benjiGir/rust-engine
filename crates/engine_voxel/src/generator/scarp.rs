//! Les **escarpements** de la génération v2 : où le relief saute de 10 à 44 blocs sur une paroi, et
//! comment le pied de cette paroi est évidé.
//!
//! # Des parois qui suivent la structure du relief
//!
//! Une isoligne d'un bruit isotrope boucle partout où ce bruit oscille : des vers, des anneaux, des
//! murs doublés. Les parois sont donc posées sur la **structure macro** de la géographie, jamais sur
//! un bruit :
//!
//! - **flancs de chaîne** : des lignes parallèles à l'axe de la chaîne, à `FLANK_FIRST` blocs de
//!   l'axe puis tous les `FLANK_SPACING` blocs (un décalage lent de ±28 blocs les écarte de la
//!   règle). La distance à l'axe est déjà une distance (`GeoSample::axis_distance`) : le côté haut est
//!   celui de l'axe. Chaque ligne n'existe que par **plages** : un masque lent propre à chaque ligne
//!   la fait apparaître, durer quelques centaines de blocs et s'effacer (la hauteur décroît avec le
//!   masque : la paroi s'éteint) ;
//! - **crêtes secondaires** : deux parois face à face, de part et d'autre de l'axe d'une crête
//!   secondaire (`GeoSample::ridge_distance`) : une crête relevée aux flancs raides, large de 50 à
//!   110 blocs. Là où un zonage lent de 140 blocs le dit, la crête s'affine jusqu'à 14 blocs : c'est
//!   elle qui porte les arches (voir plus bas) ;
//! - **failles** : de rares lignes droites d'orientation quelconque, de 280 à 540 blocs de long, dont
//!   les bouts s'effacent, tirées d'une grille de 1 000 blocs (au plus une par case, centrée à ±150
//!   blocs du milieu : deux failles sont toujours à plus de 160 blocs l'une de l'autre).
//!
//! Un zonage de 700 blocs réserve chaque zone aux flancs de chaîne **ou** aux crêtes : les deux poids
//! se croisent à zéro sur la frontière, aucune paroi ne s'y raccorde mal et deux familles ne se
//! croisent pas en grille. Une faille, elle, traverse les zones, mais près de la ligne d'une autre
//! paroi (à moins d'une trentaine de blocs) elles s'effacent toutes deux avant de se toucher : deux
//! parois ne sont jamais à moins d'une trentaine de blocs l'une de l'autre, sauf les deux faces d'une
//! crête fine (une arche voulue).
//!
//! # Le profil d'une paroi
//!
//! Avec `d` la distance signée à la ligne (positive du côté haut), `H` la hauteur de la paroi et `R`
//! (`RAMP`) la demi-largeur de la rampe, le relief macro est déplacé de `H (marche(d) - rampe(d))` :
//! une marche franche de `H` blocs en `d = 0`, et une rampe qui la compense peu à peu de part et
//! d'autre pour que l'effet s'éteigne à `R` blocs de la ligne (une paroi de falaise suivie d'un
//! glacis, comme une cuesta). Pour une crête secondaire, la rampe ne couvre que le côté bas : la
//! crête reste relevée entre ses deux parois. Les déplacements des (au plus deux) parois d'un point se
//! **somment** : chacune est continue et nulle hors de sa plage, aucune distance ne saute quand l'une
//! cède la place à l'autre.
//!
//! Les valeurs (`d`, `H`, entaille) sont calculées aux points de la grille de 4 blocs et s'interpolent
//! entre eux : `d` est quasi linéaire près de la ligne, donc la marche reste franche (un à deux
//! blocs). Loin de la ligne (au-delà de `RAMP + 6`, soit 36 blocs, moins que la moitié de l'espacement
//! des lignes de flanc) la hauteur s'annule : le changement de ligne, à mi-chemin entre deux lignes,
//! ne se voit jamais. Les 3 premiers blocs de hauteur sont retranchés : une paroi s'éteint sans jamais
//! laisser de marche de moins de 3 blocs.
//!
//! # L'entaille et les arches
//!
//! Voir `density.rs` pour l'entaille du pied (profondeur `reach`, forme `shape`, hauteur `lid`, qui
//! varient lentement le long de la paroi : voûtes basses, porches hauts, alcôves). Une **arche** naît
//! sur une crête fine : les deux entailles, qui se font face à moins de `reach` blocs, se rejoignent
//! sous la crête, dont la roche entière reste en toit. `limit_reach` plafonne la profondeur par
//! l'épaisseur de la masse haute (pas de dalle évidée de part en part), sauf sous une crête fine où le
//! plafond monte jusqu'à une marge négative : le toit y est tenu par les deux bouts du passage.

use super::macro_noise::{fbm, smooth as smooth64};
use super::{MAX_HEIGHT, SEA_LEVEL, density, hash3};

/// Demi-largeur (blocs) de la rampe d'une paroi : son effet s'éteint à cette distance de la ligne.
pub(super) const RAMP: f32 = 30.0;
/// Au-delà de cette distance (blocs) à la ligne, la hauteur est nulle.
const REACH_OUT: f32 = RAMP + 6.0;
/// Les valeurs de `dist` sont bornées à cette distance (blocs).
const DIST_MAX: f32 = 64.0;
/// Distance de la première ligne de flanc à l'axe de la chaîne, et espacement des lignes (blocs).
const FLANK_FIRST: f64 = 35.0;
const FLANK_SPACING: f64 = 84.0;
/// Côté (blocs) de la grille des failles.
const FAULT_CELL: i32 = 1000;

/// Une paroi : la marche de `height` blocs que le relief fait sur une ligne.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Wall {
    /// Distance estimée (blocs, bornée) à la ligne de rupture, positive du côté haut.
    pub dist: f32,
    /// Hauteur de la marche (blocs) : 0 hors d'un escarpement.
    pub height: f32,
    /// Part de la rampe qui ne couvre que le côté bas (1 : crête secondaire relevée entre ses
    /// deux parois ; 0 : glacis des deux côtés).
    pub hold: f32,
}

impl Wall {
    /// La part de `height` dont le pied de la paroi est abaissé à la distance `dist` : le relief du pied
    /// vaut le relief macro moins `height × ramp()`, celui du sommet ce pied plus `height`.
    pub fn ramp(&self) -> f32 {
        let d = self.dist;
        (1.0 - self.hold) * smooth(-RAMP, RAMP, d) + self.hold * 0.5 * smooth(-RAMP, 0.0, d)
    }

    /// De combien (blocs) cette paroi relève le relief à sa distance.
    pub fn lift(&self) -> f32 {
        self.height * smooth(-0.5, 0.5, self.dist)
    }

    fn lerp(a: Wall, b: Wall, t: f32) -> Wall {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Wall {
            dist: l(a.dist, b.dist),
            height: l(a.height, b.height),
            hold: l(a.hold, b.hold),
        }
    }
}

/// Les escarpements en un point : jusqu'à deux parois (la première : flanc de chaîne ou crête, selon
/// la zone ; la seconde : une faille) qui se **somment** sur le relief, chacune continue et nulle hors
/// de sa plage : aucune distance ne saute quand l'une cède la place à l'autre.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Scarp {
    pub walls: [Wall; 2],
    /// Profondeur d'entaille du pied de la paroi (blocs), avant les gardes de zone.
    pub reach: f32,
    /// Forme de l'entaille : exposant de son profil (0,7 : voûte basse et longue ; 2,2 : porche haut).
    pub shape: f32,
    /// Fraction de la hauteur de la paroi occupée par l'entaille (le reste est le toit).
    pub lid: f32,
    /// Part de crête fine (arche possible), de 0 à 1.
    pub arch: f32,
}

impl Scarp {
    /// Les valeurs de deux points, interpolées.
    pub fn lerp(a: Scarp, b: Scarp, t: f32) -> Scarp {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Scarp {
            walls: [
                Wall::lerp(a.walls[0], b.walls[0], t),
                Wall::lerp(a.walls[1], b.walls[1], t),
            ],
            reach: l(a.reach, b.reach),
            shape: l(a.shape, b.shape),
            lid: l(a.lid, b.lid),
            arch: l(a.arch, b.arch),
        }
    }

    /// La paroi qui commande l'entaille : la plus haute près de sa ligne.
    pub fn main(&self) -> Wall {
        let score = |w: &Wall| w.height * (1.0 - smooth(4.0, 30.0, w.dist.abs()));
        if score(&self.walls[1]) > score(&self.walls[0]) {
            self.walls[1]
        } else {
            self.walls[0]
        }
    }

    /// De combien (blocs, avant la garde de zone `gate`) le pied du relief est abaissé par les rampes.
    pub fn foot_drop(&self, gate: f32) -> f32 {
        self.walls.iter().map(|w| w.height * gate * w.ramp()).sum()
    }

    /// De combien le relief est relevé par les marches.
    pub fn lift(&self, gate: f32) -> f32 {
        self.walls.iter().map(|w| gate * w.lift()).sum()
    }
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    smooth64(edge0 as f64, edge1 as f64, x as f64) as f32
}

/// Ce que la géographie macro dit d'un point.
pub(super) struct Terrain {
    pub chain: f32,
    pub elevation: f32,
    /// Distances estimées (blocs) à l'axe de la chaîne et à celui de la crête secondaire.
    pub axis: f32,
    pub ridge: f32,
}

/// Une paroi candidate : sa distance signée, sa force (0 à 1 : plage, extrémité) et sa famille.
#[derive(Clone, Copy)]
struct Candidate {
    dist: f32,
    weight: f32,
    hold: f32,
    arch: f32,
}

/// La ligne de flanc de chaîne la plus proche.
fn flank(seed: u32, x: f64, z: f64, t: &Terrain) -> Candidate {
    let jitter = 28.0 * fbm(seed, 0x64, x / 420.0, z / 420.0, 2, 0.5);
    let u = (t.axis as f64 - FLANK_FIRST - jitter) / FLANK_SPACING;
    let k = u.round().max(0.0);
    let dist = ((k - u) * FLANK_SPACING) as f32;
    // Un masque par ligne : des plages de quelques centaines de blocs.
    let mask = fbm(seed, 0x70 + k as u64, x / 230.0, z / 230.0, 2, 0.5) as f32;
    Candidate {
        dist: dist.clamp(-DIST_MAX, DIST_MAX),
        weight: smooth(-0.5, -0.05, mask),
        hold: 0.0,
        arch: 0.0,
    }
}

/// Les deux parois d'une crête secondaire, à `half` blocs de son axe (la distance est celle à la
/// paroi la plus proche, positive côté axe).
fn crest(seed: u32, x: f64, z: f64, t: &Terrain, potential: f32) -> Candidate {
    let near = smooth(300.0, 220.0, t.ridge);
    let arch = smooth(
        -0.3,
        0.0,
        fbm(seed, 0x78, x / 140.0, z / 140.0, 2, 0.5) as f32,
    );
    let wide = 26.0
        + 30.0
            * smooth(
                -0.3,
                0.4,
                fbm(seed, 0x7a, x / 330.0, z / 330.0, 2, 0.5) as f32,
            );
    // La crête ne s'affine que là où la paroi est haute (20 blocs au moins) : l'entaille y est
    // partout profonde (voir `scarp`), un passage s'y ouvre, pas un mur mince. Le zonage de 140
    // blocs rétrécit la crête lentement (de 1 à 2 blocs de demi-largeur par bloc au plus).
    let arch = arch * smooth(18.0, 26.0, potential);
    let half = wide + (7.0 - wide) * arch;
    let mask = fbm(seed, 0x79, x / 260.0, z / 260.0, 2, 0.5) as f32;
    Candidate {
        dist: (half - t.ridge).clamp(-DIST_MAX, DIST_MAX),
        weight: near * smooth(-0.5, -0.05, mask),
        hold: 1.0,
        arch,
    }
}

/// Où l'entaille est profonde : des plages de 70 blocs, de 0 à 1.
fn notch_patch(seed: u32, x: f64, z: f64) -> f32 {
    smooth(
        -0.35,
        0.05,
        fbm(seed, 0x63, x / 70.0, z / 70.0, 2, 0.5) as f32,
    )
}

/// La faille la plus proche (parmi celles des neuf cases voisines de la grille de failles).
fn fault(seed: u32, x: f64, z: f64) -> Candidate {
    let cell = FAULT_CELL as f64;
    let (ci, cj) = ((x / cell).floor() as i32, (z / cell).floor() as i32);
    let mut best = Candidate {
        dist: DIST_MAX,
        weight: 0.0,
        hold: 0.0,
        arch: 0.0,
    };
    for dj in -1..=1 {
        for di in -1..=1 {
            let (i, j) = (ci + di, cj + dj);
            let h = hash3(seed, i, 0, j, 0x66);
            let unit = |shift: u32| ((h >> shift) & 0xffff) as f64 / 65536.0;
            // Sept cases sur dix porte une faille.
            if unit(0) > 0.7 {
                continue;
            }
            let (cx, cz) = (
                (i as f64 + 0.35 + 0.3 * unit(16)) * cell,
                (j as f64 + 0.35 + 0.3 * unit(32)) * cell,
            );
            let angle = std::f64::consts::PI * unit(48);
            let half_len = 140.0 + 130.0 * unit(8);
            // Centre au milieu de la case (±150) et demi-longueur d'au plus 270 : deux failles voisines
            // sont toujours à plus de 160 blocs l'une de l'autre, hors de portée de leurs rampes.
            let (sin, cos) = angle.sin_cos();
            let (rx, rz) = (x - cx, z - cz);
            let along = (rx * cos + rz * sin).abs();
            let across = -rx * sin + rz * cos;
            let side = if h >> 63 == 0 { 1.0 } else { -1.0 };
            let end = 1.0 - smooth64(0.55 * half_len, half_len, along) as f32;
            if end > best.weight {
                best = Candidate {
                    dist: (side * across).clamp(-DIST_MAX as f64, DIST_MAX as f64) as f32,
                    weight: end,
                    hold: 0.0,
                    arch: 0.0,
                };
            }
        }
    }
    best
}

/// L'escarpement d'un point de la grille : voir la documentation du module.
pub(super) fn scarp(seed: u32, x: f64, z: f64, t: &Terrain) -> Scarp {
    let sea = SEA_LEVEL as f32;
    let family = smooth(24.0, 46.0, t.chain) * smooth(sea + 8.0, sea + 22.0, t.elevation);
    if family <= 0.0 {
        return Scarp::default();
    }
    // Une zone de 700 blocs est aux flancs de chaîne ou aux crêtes, jamais aux deux : les poids se
    // croisent à zéro sur la frontière des zones, donc aucune paroi ne s'y raccorde mal.
    let sel = fbm(seed, 0x6b, x / 700.0, z / 700.0, 2, 0.5) as f32;
    // La hauteur que la paroi aurait sans les effacements : de 10 blocs à 44 sur les plus hautes
    // chaînes, modulée lentement. Le pied n'est jamais ramené à moins de 12 blocs au-dessus de la mer,
    // ni le sommet au-dessus du plafond du monde (la marche déplace le relief de ± la moitié de sa
    // hauteur).
    let tall = smooth(26.0, 62.0, t.chain);
    let q = 0.45
        + 0.55
            * smooth(
                -0.3,
                0.5,
                fbm(seed, 0x62, x / 260.0, z / 260.0, 2, 0.5) as f32,
            );
    let potential = ((10.0 + 34.0 * tall * q) * family.sqrt())
        .min(2.0 * (t.elevation - (sea + 12.0)).max(0.0))
        .min(2.0 * (MAX_HEIGHT as f32 - 6.0 - t.elevation).max(0.0));
    let (mut first, arch) = if sel < 0.0 {
        let mut c = flank(seed, x, z, t);
        c.weight *= smooth(0.0, -0.06, sel);
        (c, 0.0)
    } else {
        let mut c = crest(seed, x, z, t, potential);
        c.weight *= smooth(0.0, 0.06, sel);
        let a = c.arch;
        (c, a)
    };
    let mut second = fault(seed, x, z);
    // Une faille et une autre paroi ne se croisent pas : près de la ligne de l'une (à moins de 8
    // blocs, jusqu'à 45), l'autre s'efface avant de la toucher, et elle aussi : deux parois ne sont
    // jamais à moins d'une trentaine de blocs l'une de l'autre.
    let presence = |c: &Candidate| c.weight * (1.0 - smooth(8.0, 45.0, c.dist.abs()));
    let (p_first, p_second) = (presence(&first), presence(&second));
    first.weight *= 1.0 - smooth(0.0, 0.2, p_second);
    second.weight *= 1.0 - smooth(0.0, 0.2, p_first);
    // La hauteur d'une paroi : sa part de `potential`, nulle au-delà de sa rampe. Les 3 premiers blocs
    // sont retranchés : une paroi s'éteint sans jamais laisser de marche de moins de 3 blocs.
    let wall = |c: &Candidate| {
        let near = 1.0 - smooth(RAMP + 1.0, REACH_OUT, c.dist.abs());
        Wall {
            dist: c.dist,
            height: (potential * c.weight * near - 3.0).max(0.0),
            hold: c.hold,
        }
    };
    // L'entaille : profondeur par plages (70 blocs), rugosité fine, forme et hauteur lentes.
    let patch = notch_patch(seed, x, z).max(0.85 * arch);
    let rough = 0.75 + 0.5 * (0.5 + 0.5 * fbm(seed, 0x65, x / 23.0, z / 23.0, 1, 0.5) as f32);
    let reach = density::REACH_MAX * (1.0 + arch) * patch * rough.min(1.2);
    let shape = 0.7
        + 1.5
            * smooth(
                -0.35,
                0.35,
                fbm(seed, 0x67, x / 60.0, z / 60.0, 2, 0.5) as f32,
            );
    let lid = 0.45
        + 0.3
            * smooth(
                -0.35,
                0.35,
                fbm(seed, 0x68, x / 80.0, z / 80.0, 2, 0.5) as f32,
            );
    Scarp {
        walls: [wall(&first), wall(&second)],
        reach,
        shape,
        lid,
        arch,
    }
}

/// Plafonne la profondeur d'entaille de chaque point de la grille (`n` × `n`) par l'épaisseur locale
/// de la masse haute : le plus grand `dist` des points à deux cases (8 blocs) moins une marge de 2,5
/// blocs (de -6 sous une crête fine : une arche). Une dalle étroite n'est donc pas évidée de part
/// en part (le toit garde toujours un appui), et seule une crête fine **reliée** à des masses plus
/// épaisses, ou volontairement fine, peut former une arche.
///
/// Les points du bord de la zone voient moins de voisins, mais la marge de 8 blocs garantit que les
/// colonnes du cœur n'interpolent que des points qui en voient 5 × 5 : pas de dépendance au contexte.
pub(super) fn limit_reach(grid: &mut [Scarp], n: usize) {
    let dist: Vec<f32> = grid.iter().map(|g| g.main().dist).collect();
    for j in 0..n {
        for i in 0..n {
            let g = &mut grid[j * n + i];
            if g.reach <= 0.0 {
                continue;
            }
            let mut deepest = f32::MIN;
            for nj in j.saturating_sub(2)..=(j + 2).min(n - 1) {
                for ni in i.saturating_sub(2)..=(i + 2).min(n - 1) {
                    deepest = deepest.max(dist[nj * n + ni]);
                }
            }
            let margin = 2.5 - 8.5 * g.arch;
            g.reach = g.reach.min((deepest - margin).max(0.0));
        }
    }
}
