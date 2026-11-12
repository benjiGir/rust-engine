//! Objets originaux : silhouettes, outils, armures et provisions.

use std::path::{Path, PathBuf};

use super::{COPPER, GRASS, INK, IRON, Image, SIZE, flask, scale};

/// Une icône dessinée à partir d'un masque de 16 lignes de 16 caractères. Chaque
/// caractère choisit une couleur dans `palette` ; « . » est transparent. Un contour
/// sombre est ajouté autour, pour que l'icône se détache du fond.
fn icon(mask: &[&str; 16], palette: &[(char, [f32; 3])]) -> Image {
    let mut image = Image::transparent();
    for (y, row) in mask.iter().enumerate() {
        for (x, c) in row.chars().enumerate().take(SIZE) {
            if let Some((_, color)) = palette.iter().find(|(k, _)| *k == c) {
                image.set(x, y, *color);
            }
        }
    }
    outlined(image)
}

pub(super) fn outlined(mut image: Image) -> Image {
    let opaque = |img: &Image, x: i32, y: i32| {
        (0..16).contains(&x)
            && (0..16).contains(&y)
            && img.pixels[x as usize + y as usize * SIZE][3] > 0
    };
    let copy = Image {
        pixels: image.pixels,
    };
    for y in 0..16i32 {
        for x in 0..16i32 {
            if !opaque(&copy, x, y)
                && [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dx, dy)| opaque(&copy, x + dx, y + dy))
            {
                image.set(x as usize, y as usize, [0.08, 0.06, 0.05]);
            }
        }
    }
    image
}

const HANDLE: [f32; 3] = [0.43, 0.30, 0.21];

/// Les cinq outils, dessinés en diagonale : la tête (H, h plus sombre) et le manche (S).
fn tool(kind: &str, head: [f32; 3]) -> Image {
    let mask: [&str; 16] = match kind {
        "pickaxe" => [
            "................",
            "....L...........",
            "...LHHH.........",
            "...hHHHH........",
            ".....LHHHH......",
            "......hHHGG.....",
            "........SSHH....",
            ".......SS.HHH...",
            "......SS...HHh..",
            ".....SS.....HH..",
            "....SS......Hh..",
            "...SS.......h...",
            "..SS............",
            ".SS.............",
            "................",
            "................",
        ],
        "axe" => [
            "................",
            "...LLLL.........",
            "..LHHHHh........",
            "..LHHHHHH.......",
            "..LHHHHHHGG.....",
            "..LHHHHhGGh.....",
            "..LHHHh.SS......",
            "..LHHh.SS.......",
            "...Lh.SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "................",
            "................",
        ],
        "shovel" => [
            "................",
            "............L...",
            "..........LLHH..",
            ".........LHHHHh.",
            "........LHHHHHh.",
            "........LHHHHh..",
            ".........hHHh...",
            ".........GG.....",
            "........SS......",
            ".......SS.......",
            "......SS........",
            ".....SS.........",
            "..GGSS..........",
            ".G..G...........",
            ".G..G...........",
            "..GG............",
        ],
        "hoe" => [
            "................",
            "......LLLLLL....",
            "......HHHHHHH...",
            "..........GHH...",
            ".........SS.h...",
            "........SS......",
            ".......SS.......",
            "......SS........",
            ".....SS.........",
            "....SS..........",
            "...SS...........",
            "..SS............",
            ".SS.............",
            "SS..............",
            "................",
            "................",
        ],
        _ => [
            "................",
            "..............L.",
            ".............LH.",
            "............LHh.",
            "...........LHh..",
            "..........LHh...",
            ".........LHh....",
            "........LHh.....",
            ".......LHh......",
            "....G.LHh.......",
            ".....GHh........",
            ".....SGG........",
            "....SS..G.......",
            "...SS...........",
            "..G.............",
            "................",
        ],
    };
    let mut image = icon(
        &mask,
        &[
            ('H', head),
            ('L', head.map(|c| c * 0.65 + 0.32)),
            ('h', scale(head, 0.68)),
            ('S', HANDLE),
            ('G', COPPER),
        ],
    );
    // Un fil clair sur le tranchant, des ligatures sur le manche, sans changer la prise.
    for (y, row) in mask.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == 'H' && (y == 0 || mask[y - 1].as_bytes()[x] == b'.') {
                image.set(x, y, head.map(|c| c * 0.72 + 0.25));
            }
            if c == 'S' && (x + y) % 4 == 0 {
                image.set(x, y, [0.71, 0.56, 0.36]);
            }
        }
    }
    image
}

fn shears() -> Image {
    let mask = [
        "................",
        "...h........h...",
        "...LH......LH...",
        "....LH....LH....",
        ".....LH..LH.....",
        "......LHLH......",
        ".......GG.......",
        "......HHHH......",
        ".....R.HH.R.....",
        "....R.R..R.R....",
        "....R.R..R.R....",
        ".....R....R.....",
        "................",
        "................",
        "................",
        "................",
    ];
    icon(
        &mask,
        &[
            ('H', IRON),
            ('L', [0.88, 0.92, 0.90]),
            ('h', scale(IRON, 0.65)),
            ('G', COPPER),
            ('R', [0.48, 0.25, 0.20]),
        ],
    )
}

/// Un seau, vide ou plein d'un fluide (la couleur du contenu).
fn bucket(content: Option<[f32; 3]>) -> Image {
    let mask = [
        "................",
        "......hhhh......",
        "....hh....hh....",
        "...h........h...",
        "...h........h...",
        "..LLLLLLLLLLLL..",
        "..hFFFFFFFFFFh..",
        "..hHFFFFFFFFHh..",
        "...LHHHHHHHHh...",
        "...LHHHHHHHHh...",
        "...LHHHHHHHHh...",
        "....LHHHHHHh....",
        "....LHHHHHHh....",
        ".....hhhhhh.....",
        "................",
        "................",
    ];
    icon(
        &mask,
        &[
            ('H', IRON),
            ('L', [0.83, 0.89, 0.87]),
            ('h', scale(IRON, 0.58)),
            ('F', content.unwrap_or(INK)),
        ],
    )
}

/// Une côtelette : la viande, un bord de gras et l'os. `meat` et `fat` : ses couleurs
/// (rose cru, brun grillé ; plus rouge pour le bœuf).
fn chop(meat: [f32; 3], fat: [f32; 3]) -> Image {
    let mask = [
        "................",
        "................",
        "................",
        ".....MMMMM......",
        "...MMMMMMMMM....",
        "..MMMmMMMMMMF...",
        "..MMMMMMmMMMF...",
        ".MMMMMMMMMMMF...",
        ".MMmMMMMMMMF....",
        ".MMMMMMMMMFB....",
        "..MMMMMMMF..BB..",
        "...FFFFFF....BBb",
        ".............bb.",
        "................",
        "................",
        "................",
    ];
    let bone = [0.94, 0.92, 0.85];
    icon(
        &mask,
        &[
            ('M', meat),
            ('m', scale(meat, 0.78)),
            ('F', fat),
            ('B', bone),
            ('b', scale(bone, 0.8)),
        ],
    )
}

/// Une peau tannée : un cuir aux bords irréguliers, avec des coutures.
fn leather() -> Image {
    let mask = [
        "................",
        "................",
        "...HH......HH...",
        "...HHHHHHHHHH...",
        "....HHhHHhHHH...",
        "....HHHHHHHH....",
        "...HHHhHHHhHH...",
        "...HHHHHHHHHH...",
        "....HhHHHHhH....",
        "....HHHHHHHH....",
        "...HHHHhHHHHH...",
        "...HH......HH...",
        "................",
        "................",
        "................",
        "................",
    ];
    let hide = [0.58, 0.36, 0.2];
    icon(&mask, &[('H', hide), ('h', scale(hide, 0.65))])
}

/// Une pomme rouge, sa queue et une feuille.
fn apple() -> Image {
    let mask = [
        "................",
        "........s.......",
        "........sLL.....",
        ".......s.LL.....",
        "....RRRsRRR.....",
        "...RRRRRRRRR....",
        "..RRRRRRRRRRR...",
        "..RrRRRRRRRRR...",
        "..RrRRRRRRRRR...",
        "..RRRRRRRRRRR...",
        "..RRRRRRRRRRd...",
        "...RRRRRRRRd....",
        "....RRRdRRd.....",
        ".....dd.dd......",
        "................",
        "................",
    ];
    let red = [0.82, 0.12, 0.1];
    icon(
        &mask,
        &[
            ('R', red),
            ('r', [0.98, 0.55, 0.5]),
            ('d', scale(red, 0.65)),
            ('s', [0.4, 0.28, 0.15]),
            ('L', [0.3, 0.62, 0.2]),
        ],
    )
}

/// Les pièces d'armure.
fn armor(piece: &str, color: [f32; 3]) -> Image {
    let mask: [&str; 16] = match piece {
        "helmet" => [
            "................",
            "......LLLL......",
            "....LLHHHHLL....",
            "...LHHHHHHHHh...",
            "...LHHHHHHHHh...",
            "..LHHHHHHHHHHh..",
            "..LHhhhhhhhhHh..",
            "..LHvvvvvvvvHh..",
            "..LHvvvHHvvvHh..",
            "..LHvvvHHvvvHh..",
            "...HH..HH..HH...",
            "...Hh......hH...",
            "................",
            "................",
            "................",
            "................",
        ],
        "chestplate" => [
            "................",
            "...LL......LL...",
            "..LHHH....HHHh..",
            ".LHHHHG..GHHHHh.",
            ".HHHHHHGGHHHHHH.",
            ".hhhhHHHHHHhhhh.",
            "....LHHHHHHh....",
            "....LHHLHHHh....",
            "....LHHLHHHh....",
            "....LHHLHHHh....",
            "....LHHHHHHh....",
            "....hhhhhhhh....",
            "....GGGGGGGG....",
            ".....HHHHHH.....",
            "................",
            "................",
        ],
        "leggings" => [
            "................",
            "....LLLLLLLL....",
            "....GGGGGGGG....",
            "....HHHHHHHh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....hhh..hhh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....LHH..HHh....",
            "....hhh..hhh....",
            "................",
            "................",
        ],
        _ => [
            "................",
            "................",
            "................",
            "...LLL....LLL...",
            "...LHH....LHH...",
            "...LHH....LHH...",
            "...LHH....LHH...",
            "...GHH....GHH...",
            "...LHH....LHH...",
            "..LLHH...LLHH...",
            ".LHHHH..LHHHH...",
            ".HHHHH..HHHHH...",
            ".hhhhh..hhhhh...",
            "................",
            "................",
            "................",
        ],
    };
    icon(
        &mask,
        &[
            ('H', color),
            ('L', color.map(|c| c * 0.65 + 0.30)),
            ('h', scale(color, 0.62)),
            ('v', INK),
            ('G', [0.49, 0.34, 0.23]),
        ],
    )
}

/// Un objet « matière » : lingot, minerai brut, gemme, poudre, bâton...
fn material(kind: &str, color: [f32; 3]) -> Image {
    let mask: [&str; 16] = match kind {
        "ingot" => [
            "................",
            "................",
            "................",
            "................",
            "................",
            "......LLLLLLL...",
            ".....LLLLLLLLL..",
            "....LHHHHHHHL...",
            "...LHHHHHHHH....",
            "..hHHHHHHHHh....",
            "..hhhhhhhhh.....",
            "................",
            "................",
            "................",
            "................",
            "................",
        ],
        "raw" => [
            "................",
            "................",
            "................",
            "......HHH.......",
            ".....HLHHH......",
            "....HHHHhHH.....",
            "...HHLHHHHHH....",
            "...HHHHHhHHHH...",
            "...HHHhHHHLHH...",
            "....HHHHHHHh....",
            ".....hHHHhH.....",
            ".......hh.......",
            "................",
            "................",
            "................",
            "................",
        ],
        "gem" => [
            "................",
            "................",
            "................",
            "......LLLL......",
            ".....LLHHHH.....",
            "....LHHHHHHH....",
            "...HHHHHHHHHh...",
            "...hHHHHHHHhh...",
            "....hHHHHHhh....",
            ".....hHHHhh.....",
            "......hHhh......",
            ".......hh.......",
            "................",
            "................",
            "................",
            "................",
        ],
        "dust" => [
            "................",
            "................",
            "................",
            "................",
            "................",
            "................",
            ".......H........",
            "......HHL.......",
            ".....HHHHH......",
            "....HHLHHHH.....",
            "...HHHHHHHHH....",
            "..hhhhhhhhhhh...",
            "................",
            "................",
            "................",
            "................",
        ],
        "seeds" => [
            "................",
            "................",
            "................",
            "................",
            ".....H....H.....",
            "....HH...HH.....",
            "..........h.....",
            "...H...H........",
            "..HH..HH...H....",
            "......h...HH....",
            "....H.......H...",
            "...HH......HH...",
            "................",
            "................",
            "................",
            "................",
        ],
        _ => [
            "................",
            "..............hS",
            ".............hS.",
            "............hS..",
            "...........hS...",
            "..........hS....",
            ".........hS.....",
            "........hS......",
            ".......hS.......",
            "......hS........",
            ".....hS.........",
            "....hS..........",
            "...hS...........",
            "..hS............",
            "................",
            "................",
        ],
    };
    icon(
        &mask,
        &[
            ('H', color),
            ('h', scale(color, 0.7)),
            ('L', scale(color, 1.3)),
            ('S', color),
        ],
    )
}

/// Des provisions aux silhouettes distinctes.
fn produce(kind: &str) -> Image {
    let mut image = Image::transparent();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - 8;
            let dy = y as i32 - 9;
            let color = match kind {
                "bread" if dx * dx * 2 + dy * dy * 4 < 65 => Some(if y < 7 {
                    [0.86, 0.65, 0.34]
                } else if y > 11 {
                    [0.51, 0.31, 0.18]
                } else {
                    [0.72, 0.46, 0.24]
                }),
                "carrot" if (5..15).contains(&y) && dx.abs() <= (15 - y) as i32 / 3 => {
                    Some(if x < 8 {
                        [0.93, 0.61, 0.22]
                    } else {
                        [0.77, 0.36, 0.14]
                    })
                }
                "pear" if dx * dx + dy * dy < 22 || (dx * dx + (dy + 4).pow(2) < 8) => {
                    Some(if x < 7 {
                        [0.76, 0.77, 0.33]
                    } else {
                        [0.56, 0.64, 0.28]
                    })
                }
                "cabbage" if dx * dx + dy * dy < 31 => Some(if (x + y / 2) % 4 == 0 {
                    [0.30, 0.47, 0.27]
                } else if x + y < 16 {
                    [0.65, 0.76, 0.44]
                } else {
                    [0.45, 0.62, 0.34]
                }),
                "melon_slice" if (4..13).contains(&y) && dx.abs() <= (y - 3) as i32 / 2 => {
                    Some(if y == 12 {
                        [0.27, 0.45, 0.26]
                    } else if y == 11 {
                        [0.73, 0.80, 0.46]
                    } else {
                        [0.86, 0.40, 0.31]
                    })
                }
                _ => None,
            };
            if let Some(color) = color {
                image.set(x, y, color);
            }
        }
    }
    match kind {
        "bread" => {
            for start in [5, 8, 11] {
                for k in 0..3 {
                    image.set(start - k, 6 + k, [0.96, 0.80, 0.50]);
                }
            }
        }
        "carrot" => {
            for (x, y) in [(8, 2), (8, 3), (8, 4), (7, 3), (6, 2), (10, 2), (9, 3)] {
                image.set(x, y, [0.43, 0.64, 0.33]);
            }
            image.set(7, 8, [0.67, 0.30, 0.12]);
            image.set(8, 11, [0.67, 0.30, 0.12]);
        }
        "pear" => {
            image.set(8, 2, HANDLE);
            image.set(8, 3, HANDLE);
            image.set(9, 2, GRASS);
            image.set(10, 2, GRASS);
        }
        "melon_slice" => {
            for (x, y) in [(7, 7), (9, 9), (6, 10)] {
                image.set(x, y, INK);
            }
        }
        _ => {}
    }
    outlined(image)
}

fn provisions(kind: &str) -> Image {
    let mut image = Image::transparent();
    if kind == "wheat" {
        for (cx, top) in [(5i32, 4i32), (9, 2), (12, 5)] {
            for y in top..15 {
                image.plot(cx - (y - top) / 7, y, [0.60, 0.48, 0.24]);
            }
            for y in top..top + 6 {
                let dx = if (y - top) % 2 == 0 { -1 } else { 1 };
                image.plot(cx + dx, y, [0.91, 0.74, 0.36]);
                image.plot(cx, y, [0.77, 0.61, 0.29]);
            }
        }
    } else {
        for y in 4..14 {
            for x in 2..14 {
                let dx = x as i32 - 8;
                let dy = y as i32 - 9;
                let shape = if kind == "nut" {
                    dx * dx + dy * dy < 21
                } else {
                    dx * dx + dy * dy * 2 < 35
                };
                if !shape {
                    continue;
                }
                let base = match kind {
                    "nut" => [0.51, 0.34, 0.21],
                    "baked_potato" => [0.66, 0.43, 0.24],
                    _ => [0.72, 0.58, 0.36],
                };
                image.set(x, y, scale(base, if x + y < 16 { 1.16 } else { 0.90 }));
                if kind == "nut" && (x == 7 || x == 10) {
                    image.set(x, y, scale(base, 0.67));
                }
            }
        }
        if kind == "baked_potato" {
            for x in 5..12 {
                image.set(x, 8, [0.93, 0.83, 0.57]);
            }
            image.set(8, 7, [0.99, 0.91, 0.66]);
        } else if kind == "potato" {
            for (x, y) in [(5, 7), (10, 10), (8, 6)] {
                image.set(x, y, [0.47, 0.36, 0.23]);
            }
        }
    }
    outlined(image)
}

/// Les icônes des objets (`assets/core/items/`).
pub(super) fn item_icons(dir: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    let mut add = |name: &str, image: Image| out.push((dir.join(format!("{name}.png")), image));
    for (name, kind, color) in [
        ("stick", "stick", HANDLE),
        ("coal", "raw", [0.18, 0.18, 0.2]),
        ("charcoal", "raw", [0.25, 0.2, 0.17]),
        ("raw_iron", "raw", [0.75, 0.62, 0.52]),
        ("raw_copper", "raw", [0.82, 0.5, 0.3]),
        ("raw_gold", "raw", [0.95, 0.78, 0.3]),
        ("iron_ingot", "ingot", [0.8, 0.8, 0.84]),
        ("copper_ingot", "ingot", [0.85, 0.5, 0.3]),
        ("gold_ingot", "ingot", [0.98, 0.82, 0.25]),
        ("sapphire", "gem", [0.3, 0.5, 1.0]),
        ("malachite", "gem", [0.2, 0.7, 0.45]),
        ("jade", "gem", [0.4, 0.9, 0.6]),
        ("cinnabar_dust", "dust", [0.85, 0.15, 0.12]),
        ("flint", "gem", [0.3, 0.3, 0.33]),
        ("seeds", "seeds", [0.6, 0.65, 0.3]),
    ] {
        add(name, material(kind, color));
    }
    for (material, color) in [
        ("wooden", [0.62, 0.46, 0.28]),
        ("stone", [0.55, 0.55, 0.57]),
        ("copper", COPPER),
        ("iron", IRON),
        ("golden", [0.98, 0.82, 0.25]),
        ("sapphire", [0.28, 0.59, 0.72]),
    ] {
        for kind in ["pickaxe", "axe", "shovel", "hoe", "sword"] {
            add(&format!("{material}_{kind}"), tool(kind, color));
        }
        if !matches!(material, "wooden" | "stone") {
            for piece in ["helmet", "chestplate", "leggings", "boots"] {
                add(&format!("{material}_{piece}"), armor(piece, color));
            }
        }
    }
    add("shears", shears());
    add("bucket", bucket(None));
    add("water_bucket", bucket(Some([0.25, 0.45, 0.95])));
    add("lava_bucket", bucket(Some([1.0, 0.5, 0.1])));
    add("raw_pork", chop([0.9, 0.5, 0.5], [0.98, 0.88, 0.85]));
    add("cooked_pork", chop([0.55, 0.32, 0.16], [0.82, 0.62, 0.38]));
    add("raw_beef", chop([0.75, 0.22, 0.2], [0.95, 0.85, 0.8]));
    add("cooked_beef", chop([0.42, 0.24, 0.13], [0.7, 0.5, 0.3]));
    add("leather", leather());
    add("shadow_dust", material("dust", [0.3, 0.22, 0.42]));
    add("spores", material("seeds", [0.78, 0.82, 0.35]));
    add("apple", apple());
    add("wheat", provisions("wheat"));
    add("bread", produce("bread"));
    add("carrot", produce("carrot"));
    add("potato", provisions("potato"));
    add("baked_potato", provisions("baked_potato"));
    add("cabbage", produce("cabbage"));
    add("melon_slice", produce("melon_slice"));
    add("melon_seeds", material("seeds", [0.2, 0.22, 0.12]));
    add("pear", produce("pear"));
    add("nut", provisions("nut"));
    add("flask", flask(None));
    add("water_flask", flask(Some([0.25, 0.5, 0.95])));
    for (name, color) in [
        ("potion_healing", [0.95, 0.2, 0.25]),
        ("potion_regeneration", [0.95, 0.5, 0.7]),
        ("potion_speed", [0.45, 0.85, 0.95]),
        ("potion_strength", [0.65, 0.15, 0.15]),
        ("potion_fire_resistance", [1.0, 0.6, 0.15]),
    ] {
        add(name, flask(Some(color)));
    }
    add("fertilizer", material("dust", [0.5, 0.4, 0.25]));
    out
}
