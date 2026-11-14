//! Le stockage : coffres à paliers, blocs de métal, contrôleur, kits, améliorations et sacs à dos.
//!
//! Les coffres à paliers reprennent le dessin du coffre de base (sangles, serrure) dans la matière
//! de leur palier. Les icônes des objets sont des masques de 16 lignes ; chaque lettre choisit
//! une couleur de la palette (voir `items::icon`).
use std::path::{Path, PathBuf};

use super::items::icon;
use super::{COPPER, INK, IRON, Image, scale, workstations::rect};

const GOLD: [f32; 3] = [0.96, 0.78, 0.25];
const SAPPHIRE: [f32; 3] = [0.27, 0.48, 0.85];
const COAL: [f32; 3] = [0.17, 0.17, 0.19];
const LEATHER: [f32; 3] = [0.58, 0.36, 0.2];
const SLATE: [f32; 3] = [0.28, 0.35, 0.38];

/// Les matières des paliers, du cuivre au saphir : (nom, couleur).
pub(super) const TIERS: [(&str, [f32; 3]); 4] = [
    ("copper", COPPER),
    ("iron", IRON),
    ("gold", GOLD),
    ("sapphire", SAPPHIRE),
];

/// Un coffre à paliers : un corps de métal en lattes, des sangles plus sombres, une serrure claire.
fn chest(color: [f32; 3], side: bool, seed: u64) -> Image {
    let mut image = Image::filled(color).noise(color, 0.07, seed);
    for y in [5, 11] {
        rect(&mut image, [0, y, 16, y + 1], scale(color, 0.78));
    }
    rect(&mut image, [0, 0, 16, 1], scale(color, 1.12));
    for x in [2, 12] {
        rect(&mut image, [x, 1, x + 2, 15], scale(color, 0.62));
        rect(&mut image, [x, 2, x + 1, 14], scale(color, 0.85));
        for y in [3, 12] {
            image.set(x, y, scale(color, 1.3));
        }
    }
    if side {
        rect(&mut image, [1, 5, 15, 6], INK);
        rect(&mut image, [6, 4, 10, 8], [0.92, 0.9, 0.8]);
        rect(&mut image, [7, 5, 9, 7], scale(INK, 1.4));
        image.set(7, 7, INK);
    } else {
        rect(&mut image, [1, 1, 15, 2], scale(color, 0.7));
        rect(&mut image, [1, 14, 15, 15], scale(color, 0.7));
    }
    image
}

/// Un bloc plein de métal (ou de charbon) : un biseau clair en haut à gauche, sombre en bas à
/// droite, et des reflets en petits groupes.
fn metal_block(color: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(color).noise(color, 0.09, seed);
    rect(&mut image, [0, 0, 16, 1], scale(color, 1.2));
    rect(&mut image, [0, 0, 1, 16], scale(color, 1.2));
    rect(&mut image, [0, 15, 16, 16], scale(color, 0.7));
    rect(&mut image, [15, 0, 16, 16], scale(color, 0.7));
    rect(&mut image, [7, 0, 8, 16], scale(color, 0.88));
    rect(&mut image, [0, 7, 16, 8], scale(color, 0.88));
    for (x, y) in [(3, 3), (4, 3), (11, 4), (12, 11), (4, 12), (5, 12)] {
        image.set(x, y, scale(color, 1.28));
    }
    image
}

/// Le contrôleur : une ardoise cerclée de cuivre ; le dessus porte une rune, les côtés un écran.
fn controller(top: bool) -> Image {
    let mut image = Image::filled(SLATE).noise(SLATE, 0.06, 931);
    rect(&mut image, [0, 0, 16, 1], COPPER);
    rect(&mut image, [0, 15, 16, 16], scale(COPPER, 0.7));
    rect(&mut image, [0, 0, 1, 16], scale(COPPER, 0.9));
    rect(&mut image, [15, 0, 16, 16], scale(COPPER, 0.7));
    let glow = [0.45, 0.82, 0.78];
    if top {
        rect(&mut image, [4, 4, 12, 12], scale(SLATE, 0.6));
        rect(&mut image, [6, 6, 10, 10], glow);
        rect(&mut image, [7, 7, 9, 9], [0.9, 1.0, 0.95]);
        for (x, y) in [(2, 2), (13, 2), (2, 13), (13, 13)] {
            image.set(x, y, COPPER);
        }
    } else {
        rect(&mut image, [3, 3, 13, 9], INK);
        rect(&mut image, [4, 4, 12, 8], scale(glow, 0.55));
        for x in (4..12).step_by(2) {
            rect(&mut image, [x, 6, x + 1, 8], glow);
        }
        rect(&mut image, [3, 11, 13, 12], scale(COPPER, 0.8));
        for x in [4, 7, 10] {
            image.set(x, 13, glow);
        }
    }
    image
}

/// Le kit : une plaque de la couleur du palier, rivetée. Le kit direct y ajoute des chevrons.
fn kit(color: [f32; 3], direct: bool) -> Image {
    let mut mask = [
        "................",
        "................",
        "................",
        "..KKKKKKKKKKKK..",
        "..KRPPPPPPPPRK..",
        "..KPPPPPPPPPPK..",
        "..KPPPLLLLPPPK..",
        "..KPPPLLLLPPPK..",
        "..KPPPPPPPPPPK..",
        "..KRPPPPPPPPRK..",
        "..KKKKKKKKKKKK..",
        "................",
        "................",
        "................",
        "................",
        "................",
    ]
    .map(String::from);
    if direct {
        for (y, row) in [(5, "..KPAPAPAPAPPK.."), (8, "..KPAPAPAPAPPK..")] {
            mask[y] = row.to_string();
        }
    }
    let rows: Vec<&str> = mask.iter().map(String::as_str).collect();
    let mask: [&str; 16] = rows.try_into().expect("16 lignes");
    icon(
        &mask,
        &[
            ('K', scale(color, 0.45)),
            ('P', color),
            ('R', scale(color, 1.35)),
            ('L', [0.95, 0.93, 0.85]),
            ('A', scale(color, 0.62)),
        ],
    )
}

/// Une carte d'amélioration : un cadre sombre, un fond de la couleur de la fonction, un symbole.
fn upgrade(kind: &str) -> Image {
    let (color, symbol): ([f32; 3], [&str; 10]) = match kind {
        "stack_2" => (
            [0.35, 0.55, 0.85],
            [
                "..........",
                ".SSSSSS...",
                ".S....S...",
                ".S.SSSSSS.",
                ".SSS....S.",
                "...S.SS.S.",
                "...S.S..S.",
                "...S.SS.S.",
                "...SSSSSS.",
                "..........",
            ],
        ),
        "stack_4" => (
            [0.55, 0.4, 0.85],
            [
                "..........",
                ".SSSSSS...",
                ".S....S...",
                ".S.SSSSSS.",
                ".SSS....S.",
                "...S.S.SS.",
                "...S.SSSS.",
                "...S....S.",
                "...SSSSSS.",
                "..........",
            ],
        ),
        "compact" => (
            [0.35, 0.7, 0.45],
            [
                "..........",
                ".S.S.S....",
                "..........",
                ".S.S.S.SS.",
                "..........",
                ".S.S.S.SS.",
                "..........",
                "......SSSS",
                "......SSSS",
                "..........",
            ],
        ),
        "void" => (
            [0.2, 0.15, 0.3],
            [
                "..SSSSSS..",
                ".S......S.",
                "S..SSSS..S",
                "S.S....S.S",
                "S.S.SS.S.S",
                "S.S.SS.S.S",
                "S.S....S.S",
                "S..SSSS..S",
                ".S......S.",
                "..SSSSSS..",
            ],
        ),
        "funnel" => (
            [0.75, 0.55, 0.3],
            [
                "SSSSSSSSSS",
                ".SSSSSSSS.",
                "..SSSSSS..",
                "...SSSS...",
                "....SS....",
                "....SS....",
                "....SS....",
                "....SS....",
                "..........",
                "..........",
            ],
        ),
        "pickup" => (
            [0.8, 0.65, 0.3],
            [
                "....SS....",
                "....SS....",
                "....SS....",
                "..SSSSSS..",
                "...SSSS...",
                "....SS....",
                "..........",
                "S........S",
                "SSSSSSSSSS",
                "SSSSSSSSSS",
            ],
        ),
        "magnet" => (
            [0.75, 0.25, 0.25],
            [
                "SSS....SSS",
                "SSS....SSS",
                "SSS....SSS",
                "SSS....SSS",
                "SSS....SSS",
                "SSSS..SSSS",
                ".SSSSSSSS.",
                "..SSSSSS..",
                "..........",
                "..........",
            ],
        ),
        "restock" => (
            [0.3, 0.65, 0.7],
            [
                "..........",
                "..SSSSSS..",
                ".S......S.",
                "S....SS..S",
                "S...SSSS.S",
                "S..SSSSSS.",
                "S........S",
                ".S......S.",
                "..SSSSSS..",
                "..........",
            ],
        ),
        "feed" => (
            [0.8, 0.35, 0.3],
            [
                "......S...",
                ".....SS...",
                "..SSSSSSS.",
                ".SSSSSSSSS",
                "SSSSSSSSSS",
                "SSSSSSSSSS",
                "SSSSSSSSSS",
                ".SSSSSSSS.",
                "..SSSSSS..",
                "..........",
            ],
        ),
        _ => unreachable!("amélioration inconnue : {kind}"),
    };
    let rows: Vec<String> = (0..16)
        .map(|y| match y {
            1 | 14 => ".KKKKKKKKKKKKKK.".to_string(),
            3..=12 => format!(".KP{}PK.", symbol[y - 3].replace('.', "P")),
            2 | 13 => ".KPPPPPPPPPPPPK.".to_string(),
            _ => "................".to_string(),
        })
        .collect();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mask: [&str; 16] = refs.try_into().expect("16 lignes");
    icon(
        &mask,
        &[
            ('K', scale(color, 0.4)),
            ('P', scale(color, 0.85)),
            ('S', [0.97, 0.95, 0.88]),
        ],
    )
}

/// Un sac à dos : un rabat, une boucle, une poche, des bretelles.
fn backpack(color: [f32; 3], buckle: [f32; 3]) -> Image {
    let mask = [
        "................",
        ".....SSSSSS.....",
        "....S......S....",
        "...HHHHHHHHHH...",
        "..HHHHHHHHHHHH..",
        "..HLLLLLLLLLLH..",
        "..HHHHHBBHHHHH..",
        "..HHHHHBBHHHHH..",
        "..HhhhhhhhhhhH..",
        "..HhhLLLLLLhhH..",
        "..HhhhhhhhhhhH..",
        "..HhhhhhhhhhhH..",
        "..HHHHHHHHHHHH..",
        "...HHHHHHHHHH...",
        "................",
        "................",
    ];
    icon(
        &mask,
        &[
            ('H', color),
            ('h', scale(color, 0.78)),
            ('L', scale(color, 1.25)),
            ('S', scale(color, 0.55)),
            ('B', buckle),
        ],
    )
}

/// Le flanc d'un sac posé (toutes ses faces de côté, poche comprise) : le corps dans la matière
/// du palier, le rabat plus clair en haut avec sa boucle, des coutures, la poche en bas.
fn placed_backpack_side(color: [f32; 3], buckle: [f32; 3], seed: u64) -> Image {
    let mut image = Image::filled(color).noise(color, 0.10, seed);
    let stitch = scale(color, 1.4);
    // Le rabat (les rangées 3 à 5 tombent sur le rabat du modèle), bordé d'une ombre.
    rect(&mut image, [0, 2, 16, 6], scale(color, 1.15));
    rect(&mut image, [0, 6, 16, 7], scale(color, 0.7));
    rect(&mut image, [7, 4, 9, 8], buckle);
    image.set(7, 4, scale(buckle, 1.2));
    // Les coutures des côtés et de la poche.
    for y in (8..15).step_by(2) {
        image.set(3, y, stitch);
        image.set(12, y, stitch);
    }
    for x in (4..12).step_by(2) {
        image.set(x, 9, stitch);
    }
    // Le bas, plus sombre, usé par le sol.
    rect(&mut image, [0, 15, 16, 16], scale(color, 0.6));
    image
}

/// Le dessus d'un sac posé : le rabat cousu tout autour, la poignée au milieu.
fn placed_backpack_top(color: [f32; 3], seed: u64) -> Image {
    let flap = scale(color, 1.15);
    let mut image = Image::filled(flap).noise(flap, 0.08, seed);
    let stitch = scale(color, 1.45);
    for i in (4..12).step_by(2) {
        image.set(i, 5, stitch);
        image.set(i, 11, stitch);
        image.set(4, i, stitch);
        image.set(11, i, stitch);
    }
    rect(&mut image, [6, 8, 10, 9], scale(color, 0.55));
    image
}

/// Toutes les textures de blocs et icônes d'objets du stockage.
pub(super) fn generate(textures: &Path, items: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    for (n, (name, color)) in TIERS.iter().enumerate() {
        let seed = 940 + n as u64;
        out.push((
            textures.join(format!("{name}_chest_top.png")),
            chest(*color, false, seed),
        ));
        out.push((
            textures.join(format!("{name}_chest_side.png")),
            chest(*color, true, seed),
        ));
        out.push((items.join(format!("{name}_kit.png")), kit(*color, false)));
        if *name != "copper" {
            out.push((
                items.join(format!("{name}_direct_kit.png")),
                kit(*color, true),
            ));
        }
    }
    for (name, color, seed) in [
        ("iron_block", IRON, 960),
        ("copper_block", COPPER, 961),
        ("gold_block", GOLD, 962),
        ("sapphire_block", SAPPHIRE, 963),
        ("coal_block", COAL, 964),
    ] {
        out.push((
            textures.join(format!("{name}.png")),
            metal_block(color, seed),
        ));
    }
    out.push((
        textures.join("storage_controller_top.png"),
        controller(true),
    ));
    out.push((
        textures.join("storage_controller_side.png"),
        controller(false),
    ));
    for kind in [
        "stack_2", "stack_4", "compact", "void", "funnel", "pickup", "magnet", "restock", "feed",
    ] {
        out.push((items.join(format!("upgrade_{kind}.png")), upgrade(kind)));
    }
    // Les sacs posés : flanc et dessus par palier, un dessous commun en cuir sombre.
    for (n, (name, color, buckle)) in [
        ("leather", LEATHER, GOLD),
        ("copper", COPPER, [0.9, 0.9, 0.85]),
        ("iron", IRON, GOLD),
        ("sapphire", SAPPHIRE, GOLD),
    ]
    .into_iter()
    .enumerate()
    {
        let seed = 980 + n as u64;
        out.push((
            textures.join(format!("backpack_{name}_side.png")),
            placed_backpack_side(color, buckle, seed),
        ));
        out.push((
            textures.join(format!("backpack_{name}_top.png")),
            placed_backpack_top(color, seed),
        ));
    }
    out.push((
        textures.join("backpack_bottom.png"),
        Image::filled(scale(LEATHER, 0.55)).noise(scale(LEATHER, 0.55), 0.08, 990),
    ));
    out.push((items.join("backpack_leather.png"), backpack(LEATHER, GOLD)));
    out.push((
        items.join("backpack_copper.png"),
        backpack(COPPER, [0.9, 0.9, 0.85]),
    ));
    out.push((items.join("backpack_iron.png"), backpack(IRON, GOLD)));
    out.push((
        items.join("backpack_sapphire.png"),
        backpack(SAPPHIRE, GOLD),
    ));
    out
}
