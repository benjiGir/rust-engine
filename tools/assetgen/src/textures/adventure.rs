//! L'aventure : fragments de rareté, poussière, gemmes (six formes, cinq qualités), sceaux et
//! tables de récupération et de réforge. Les couleurs sont celles de `rarities.ron` et `gems.ron`.

use std::path::{Path, PathBuf};

use super::items::{icon, outlined};
use super::workstations::rect;
use super::{COPPER, INK, IRON, Image, STONE, planks, scale};

/// (nom, couleur) : les raretés, de la commune à la mythique.
const RARITIES: [(&str, [f32; 3]); 5] = [
    ("common", [0.78, 0.80, 0.82]),
    ("uncommon", [0.38, 0.84, 0.44]),
    ("rare", [0.34, 0.61, 1.0]),
    ("epic", [0.75, 0.42, 1.0]),
    ("mythic", [1.0, 0.67, 0.19]),
];

/// (nom, couleur, forme) : les six gemmes.
const GEMS: [(&str, [f32; 3], usize); 6] = [
    ("ruby", [0.88, 0.20, 0.26], 0),
    ("topaz", [0.94, 0.75, 0.20], 1),
    ("emerald", [0.24, 0.78, 0.43], 2),
    ("onyx", [0.32, 0.33, 0.43], 3),
    ("amethyst", [0.67, 0.39, 0.90], 4),
    ("opal", [0.51, 0.86, 0.93], 5),
];

/// Le fragment : un éclat anguleux de la couleur de la rareté, clair sur l'arête.
fn shard(color: [f32; 3]) -> Image {
    icon(
        &[
            "................",
            "................",
            ".......LL.......",
            "......LPPK......",
            ".....LPPPPK.....",
            ".....LPPPDK.....",
            "....LPPPPPDK....",
            "....LPPPPDDK....",
            "...LPPPPPDDDK...",
            "...LPPPPDDDKK...",
            "....PPPDDDKK....",
            ".....PDDDKK.....",
            "......DDKK......",
            ".......KK.......",
            "................",
            "................",
        ],
        &[
            ('L', scale(color, 1.3)),
            ('P', color),
            ('D', scale(color, 0.72)),
            ('K', scale(color, 0.5)),
        ],
    )
}

/// La poussière : un petit tas de grains nacrés.
fn dust() -> Image {
    let base = [0.82, 0.74, 0.92];
    icon(
        &[
            "................",
            "................",
            "................",
            "................",
            "................",
            ".......L........",
            "......LPP.......",
            ".....LPPPP......",
            "....LPPPPPD.....",
            "...LPPLPPPPD....",
            "..LPPPPPPLPPD...",
            ".LPPPPPPPPPPDD..",
            ".DDPPPPPPPPDDD..",
            "..DDDDDDDDDD....",
            "................",
            "................",
        ],
        &[
            ('L', [0.98, 0.95, 1.0]),
            ('P', base),
            ('D', scale(base, 0.68)),
        ],
    )
}

/// Une gemme : la forme dit la sorte, la taille et l'éclat disent la qualité (1 à 5).
fn gem(color: [f32; 3], shape: usize, quality: usize) -> Image {
    let mut image = Image::transparent();
    let light = scale(color, 1.35);
    let dark = scale(color, 0.62);
    // Chaque forme est un masque de 8 lignes ; la qualité l'agrandit d'un pixel par palier
    // (1 : très petit, 5 : plein cadre) et ajoute des facettes claires.
    let outline: [&[(i32, i32)]; 6] = [
        // Rubis : losange.
        &[
            (0, 3),
            (1, 2),
            (2, 1),
            (3, 0),
            (4, 1),
            (5, 2),
            (6, 3),
            (5, 4),
            (4, 5),
            (3, 6),
            (2, 5),
            (1, 4),
        ],
        // Topaze : rectangle taillé.
        &[
            (1, 0),
            (5, 0),
            (6, 1),
            (6, 5),
            (5, 6),
            (1, 6),
            (0, 5),
            (0, 1),
        ],
        // Émeraude : octogone allongé.
        &[
            (2, 0),
            (4, 0),
            (6, 2),
            (6, 4),
            (4, 6),
            (2, 6),
            (0, 4),
            (0, 2),
        ],
        // Onyx : hexagone.
        &[(1, 0), (5, 0), (6, 3), (5, 6), (1, 6), (0, 3)],
        // Améthyste : cristal pointu.
        &[(3, 0), (5, 2), (5, 5), (3, 6), (1, 5), (1, 2)],
        // Opale : goutte.
        &[
            (3, 0),
            (5, 2),
            (6, 4),
            (5, 6),
            (3, 6),
            (1, 6),
            (0, 4),
            (1, 2),
        ],
    ];
    let size = 8 + quality as i32; // 9 à 13 pixels de côté
    let pts = outline[shape];
    let (ox, oy) = ((16 - size) / 2, (16 - size) / 2);
    // Remplissage : un point est dedans s'il est du bon côté de chaque arête (polygone convexe).
    let scale_f = f64::from(size - 1) / 6.0;
    let poly: Vec<(f64, f64)> = pts
        .iter()
        .map(|&(x, y)| (f64::from(x) * scale_f, f64::from(y) * scale_f))
        .collect();
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (f64::from(px) + 0.5, f64::from(py) + 0.5);
            let mut inside = true;
            for i in 0..poly.len() {
                let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
                let cross = (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
                if cross < -0.5 {
                    inside = false;
                    break;
                }
            }
            if inside {
                let t = (f64::from(px) + f64::from(py)) / f64::from(size * 2);
                let shade = if t < 0.38 {
                    light
                } else if t > 0.62 {
                    dark
                } else {
                    color
                };
                image.plot(ox + px, oy + py, shade);
            }
        }
    }
    // Les facettes : de plus en plus de points clairs avec la qualité.
    let glints = [
        (ox + size / 3, oy + size / 3),
        (ox + size / 2, oy + size / 4),
        (ox + size * 2 / 3, oy + size / 2),
        (ox + size / 3, oy + size * 2 / 3),
    ];
    for (x, y) in glints.into_iter().take(quality.div_ceil(2)) {
        image.plot(x, y, [1.0, 1.0, 1.0]);
    }
    outlined(image)
}

/// Un sceau : un disque de cire frappé d'un signe. `remove` : une croix ; sinon un plus.
fn seal(remove: bool) -> Image {
    let wax = if remove {
        [0.78, 0.30, 0.30]
    } else {
        [0.30, 0.52, 0.84]
    };
    let mut image = icon(
        &[
            "................",
            "................",
            "......KKKK......",
            "....KKPPPPKK....",
            "...KPPPPPPPPK...",
            "...KPPPPPPPPK...",
            "..KPPPPPPPPPPK..",
            "..KPPPPPPPPPPK..",
            "..KPPPPPPPPPPK..",
            "..KPPPPPPPPPPK..",
            "...KPPPPPPPPK...",
            "...KPPPPPPPPK...",
            "....KKPPPPKK....",
            "......KKKK......",
            "................",
            "................",
        ],
        &[('K', scale(wax, 0.5)), ('P', wax)],
    );
    let sign = [0.97, 0.92, 0.8];
    if remove {
        for i in 0..5 {
            image.set(5 + i, 5 + i, sign);
            image.set(9 - i, 5 + i, sign);
        }
    } else {
        for i in 0..7 {
            image.set(4 + i, 7, sign);
            image.set(7, 4 + i, sign);
            image.set(8, 4 + i, sign);
        }
    }
    image
}

/// Une table : du bois cerclé de cuivre ; le dessus porte une marque (récupération : des éclats
/// qui s'éparpillent ; réforge : une enclume et des étincelles).
fn table(top: bool, reforge: bool) -> Image {
    let wood = [0.52, 0.37, 0.25];
    let mut image = planks(wood, 970);
    rect(&mut image, [0, 0, 16, 1], scale(COPPER, 0.8));
    rect(&mut image, [0, 15, 16, 16], scale(COPPER, 0.6));
    rect(&mut image, [0, 0, 1, 16], scale(COPPER, 0.75));
    rect(&mut image, [15, 0, 16, 16], scale(COPPER, 0.6));
    if top {
        rect(&mut image, [3, 3, 13, 13], scale(STONE, 0.55));
        rect(&mut image, [4, 4, 12, 12], scale(STONE, 0.8));
        if reforge {
            // L'enclume : un plateau et un pied.
            rect(&mut image, [5, 6, 11, 8], IRON);
            rect(&mut image, [7, 8, 9, 10], scale(IRON, 0.7));
            rect(&mut image, [5, 10, 11, 11], scale(IRON, 0.55));
            for (x, y) in [(4, 5), (11, 5), (12, 8)] {
                image.set(x, y, [1.0, 0.8, 0.3]);
            }
        } else {
            for (x, y, c) in [
                (5, 5, RARITIES[2].1),
                (9, 6, RARITIES[3].1),
                (6, 9, RARITIES[1].1),
                (10, 10, RARITIES[4].1),
                (8, 8, RARITIES[0].1),
            ] {
                rect(&mut image, [x, y, x + 2, y + 2], c);
                image.set(x, y, scale(c, 1.3));
            }
        }
    } else {
        rect(&mut image, [3, 5, 13, 11], INK);
        if reforge {
            rect(&mut image, [4, 6, 12, 8], IRON);
            rect(&mut image, [6, 8, 10, 10], scale(IRON, 0.65));
        } else {
            for (i, (_, c)) in RARITIES.iter().enumerate() {
                rect(&mut image, [4 + 2 * i, 6, 5 + 2 * i, 10], *c);
            }
        }
    }
    image
}

pub(super) fn generate(textures: &Path, items: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    for (name, color) in RARITIES {
        out.push((items.join(format!("shard_{name}.png")), shard(color)));
    }
    out.push((items.join("gem_dust.png"), dust()));
    for (name, color, shape) in GEMS {
        for quality in 1..=5 {
            out.push((
                items.join(format!("gem_{name}_{quality}.png")),
                gem(color, shape, quality),
            ));
        }
    }
    out.push((items.join("seal_unsocket.png"), seal(true)));
    out.push((items.join("seal_socket.png"), seal(false)));
    for (name, reforge) in [("salvage_table", false), ("reforge_table", true)] {
        out.push((
            textures.join(format!("{name}_top.png")),
            table(true, reforge),
        ));
        out.push((
            textures.join(format!("{name}_side.png")),
            table(false, reforge),
        ));
    }
    out
}
