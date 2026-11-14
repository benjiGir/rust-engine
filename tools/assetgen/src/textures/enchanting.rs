//! L'enchantement avancé : étagères (cinq matières), bibliothèque, enclume simple, livres (dont le
//! Guide du voyageur), tomes et charmes de potion. Les couleurs des matières suivent les stats des étagères de `blocks.ron`.

use std::path::{Path, PathBuf};

use super::items::outlined;
use super::workstations::rect;
use super::{COPPER, INK, IRON, Image, planks, scale};

/// Une matière d'étagère : (nom, couleur du cadre, couleurs des livres, couleur de l'accent).
struct Shelf {
    name: &'static str,
    frame: [f32; 3],
    books: [[f32; 3]; 4],
    accent: [f32; 3],
    seed: u64,
}

const SHELVES: [Shelf; 5] = [
    Shelf {
        name: "oak",
        frame: [0.52, 0.37, 0.22],
        books: [
            [0.66, 0.22, 0.20],
            [0.30, 0.45, 0.62],
            [0.78, 0.62, 0.28],
            [0.34, 0.50, 0.30],
        ],
        accent: [0.78, 0.62, 0.28],
        seed: 3010,
    },
    Shelf {
        name: "sapphire",
        frame: [0.20, 0.30, 0.55],
        books: [
            [0.27, 0.48, 0.85],
            [0.55, 0.75, 0.96],
            [0.16, 0.28, 0.60],
            [0.78, 0.88, 1.0],
        ],
        accent: [0.55, 0.75, 0.96],
        seed: 3020,
    },
    Shelf {
        name: "amethyst",
        frame: [0.38, 0.22, 0.50],
        books: [
            [0.67, 0.39, 0.90],
            [0.85, 0.62, 0.98],
            [0.45, 0.24, 0.66],
            [0.95, 0.80, 1.0],
        ],
        accent: [0.85, 0.62, 0.98],
        seed: 3030,
    },
    Shelf {
        name: "jade",
        frame: [0.17, 0.42, 0.28],
        books: [
            [0.24, 0.70, 0.42],
            [0.55, 0.88, 0.60],
            [0.14, 0.45, 0.30],
            [0.80, 0.95, 0.78],
        ],
        accent: [0.55, 0.88, 0.60],
        seed: 3040,
    },
    Shelf {
        name: "volcanic",
        frame: [0.14, 0.12, 0.15],
        books: [
            [0.30, 0.10, 0.08],
            [0.88, 0.36, 0.10],
            [0.18, 0.16, 0.20],
            [0.98, 0.64, 0.18],
        ],
        accent: [0.98, 0.50, 0.12],
        seed: 3050,
    },
];

/// Les dos de livres d'une rangée : de petits rectangles de hauteurs différentes.
fn book_row(image: &mut Image, y0: usize, shelf: &Shelf) {
    let widths = [2, 1, 2, 2, 1, 2, 2, 2];
    let mut x = 1;
    for (i, width) in widths.iter().enumerate() {
        if x + width > 15 {
            break;
        }
        let color = shelf.books[(i * 3 + usize::from(shelf.seed as u8)) % 4];
        let top = y0 + (i * 5 + 2) % 3;
        rect(image, [x, top, x + width, y0 + 5], color);
        rect(image, [x, top, x + 1, y0 + 5], scale(color, 1.25));
        image.set(x, top, scale(shelf.accent, 1.2));
        x += width;
    }
}

/// Le flanc d'une étagère : un cadre, deux rangées de livres, une planche entre les deux.
fn shelf_side(shelf: &Shelf) -> Image {
    let mut image = planks(shelf.frame, shelf.seed);
    rect(&mut image, [1, 1, 15, 15], scale(INK, 0.9));
    book_row(&mut image, 2, shelf);
    book_row(&mut image, 9, shelf);
    // Les planches.
    for y in [1, 7, 14] {
        rect(&mut image, [1, y, 15, y + 1], scale(shelf.frame, 0.9));
    }
    // Un accent de matière sur les montants.
    for y in [4, 11] {
        image.set(0, y, shelf.accent);
        image.set(15, y, shelf.accent);
    }
    image
}

/// Le dessus : le bois du cadre, un liseré et un motif de la matière au centre.
fn shelf_top(shelf: &Shelf) -> Image {
    let mut image = planks(shelf.frame, shelf.seed + 1);
    rect(&mut image, [0, 0, 16, 1], scale(shelf.frame, 0.7));
    rect(&mut image, [0, 15, 16, 16], scale(shelf.frame, 0.7));
    rect(&mut image, [0, 0, 1, 16], scale(shelf.frame, 0.75));
    rect(&mut image, [15, 0, 16, 16], scale(shelf.frame, 0.75));
    for (x, y) in [
        (7, 5),
        (8, 5),
        (6, 6),
        (9, 6),
        (5, 7),
        (10, 7),
        (5, 8),
        (10, 8),
        (6, 9),
        (9, 9),
        (7, 10),
        (8, 10),
    ] {
        image.set(x, y, shelf.accent);
    }
    image
}

/// La bibliothèque : un meuble haut, doré, plein de reliures.
fn library_side() -> Image {
    let shelf = &SHELVES[0];
    let mut image = planks([0.34, 0.20, 0.30], 3060);
    rect(&mut image, [1, 1, 15, 15], scale(INK, 0.8));
    book_row(&mut image, 2, shelf);
    book_row(&mut image, 9, shelf);
    for y in [1, 7, 14] {
        rect(&mut image, [1, y, 15, y + 1], [0.80, 0.62, 0.25]);
    }
    for y in 0..16 {
        image.set(0, y, [0.80, 0.62, 0.25]);
        image.set(15, y, [0.80, 0.62, 0.25]);
    }
    image
}

fn library_top() -> Image {
    let mut image = planks([0.34, 0.20, 0.30], 3070);
    rect(&mut image, [1, 1, 15, 15], [0.80, 0.62, 0.25]);
    rect(&mut image, [2, 2, 14, 14], [0.24, 0.12, 0.24]);
    // Un livre ouvert : deux pages et leur reliure.
    rect(&mut image, [3, 5, 8, 12], [0.95, 0.92, 0.80]);
    rect(&mut image, [8, 5, 13, 12], [0.90, 0.86, 0.72]);
    rect(&mut image, [7, 5, 9, 12], [0.55, 0.35, 0.15]);
    for y in [7, 9, 11] {
        rect(&mut image, [4, y, 7, y + 1], [0.55, 0.50, 0.40]);
        rect(&mut image, [9, y, 12, y + 1], [0.55, 0.50, 0.40]);
    }
    image
}

/// L'enclume : du fer en aplat, un biseau clair en haut.
fn anvil(top: bool) -> Image {
    let color = [0.30, 0.31, 0.35];
    let mut image = Image::filled(color).noise(color, 0.10, 3080);
    rect(&mut image, [0, 0, 16, 1], scale(IRON, 1.0));
    rect(&mut image, [0, 15, 16, 16], scale(color, 0.6));
    rect(&mut image, [0, 0, 1, 16], scale(IRON, 0.85));
    rect(&mut image, [15, 0, 16, 16], scale(color, 0.65));
    if top {
        rect(&mut image, [3, 3, 13, 13], scale(IRON, 0.8));
        rect(&mut image, [4, 4, 12, 12], scale(IRON, 0.95));
        for (x, y) in [(6, 6), (7, 6), (10, 9)] {
            image.set(x, y, scale(IRON, 1.3));
        }
    } else {
        rect(&mut image, [2, 6, 14, 7], scale(color, 0.7));
        rect(&mut image, [3, 10, 13, 11], scale(COPPER, 0.8));
    }
    image
}

const EMBLEM_SWORD: [&str; 6] = ["..#..", "..#..", "..#..", ".###.", "..#..", "..#.."];
const EMBLEM_PICKAXE: [&str; 6] = [".###.", "#.#.#", "..#..", "..#..", "..#..", "..#.."];
const EMBLEM_SHIELD: [&str; 6] = ["#####", "#####", "#####", ".###.", ".###.", "..#.."];
const EMBLEM_ARROW: [&str; 6] = ["..#..", "..#..", "..#..", "#####", ".###.", "..#.."];
/// La rose des vents du Guide du voyageur.
const EMBLEM_COMPASS: [&str; 6] = ["..#..", "..#..", "#####", ".#.#.", "..#..", "..#.."];
const EMBLEM_STAR: [&str; 6] = [".....", "..#..", ".###.", "..#..", ".....", "....."];

/// Un livre fermé : couverture de cette couleur, tranche de pages claires, emblème au centre.
fn book(
    cover: [f32; 3],
    emblem: Option<&[&str; 6]>,
    emblem_color: [f32; 3],
    sparkles: bool,
) -> Image {
    let mut image = Image::transparent();
    rect(&mut image, [3, 2, 12, 14], cover);
    rect(&mut image, [3, 2, 5, 14], scale(cover, 0.68));
    rect(&mut image, [5, 2, 12, 3], scale(cover, 1.25));
    rect(&mut image, [3, 13, 12, 14], scale(cover, 0.62));
    rect(&mut image, [12, 3, 13, 13], [0.95, 0.92, 0.80]);
    for y in [5, 8, 11] {
        image.set(12, y, [0.78, 0.74, 0.62]);
    }
    // Le fermoir ou l'emblème.
    match emblem {
        Some(rows) => {
            for (dy, row) in rows.iter().enumerate() {
                for (dx, c) in row.chars().enumerate() {
                    if c == '#' {
                        image.set(6 + dx, 5 + dy, emblem_color);
                    }
                }
            }
        }
        None => {
            rect(&mut image, [8, 7, 10, 9], emblem_color);
            image.set(8, 7, scale(emblem_color, 1.3));
        }
    }
    if sparkles {
        for (x, y) in [(1, 3), (14, 2), (14, 12), (2, 12), (1, 8)] {
            image.set(x, y, [0.70, 0.95, 1.0]);
        }
        for (x, y) in [(2, 3), (1, 4), (14, 3), (15, 2), (13, 2)] {
            image.set(x, y, [0.45, 0.70, 0.95]);
        }
    }
    outlined(image)
}

/// Un charme : une chaîne, un médaillon d'or serti de la couleur de la potion.
fn charm(color: [f32; 3]) -> Image {
    let mut image = Image::transparent();
    let gold = [0.92, 0.72, 0.25];
    // La chaîne : un anneau en haut.
    for (x, y) in [
        (6, 1),
        (7, 0),
        (8, 0),
        (9, 1),
        (10, 2),
        (5, 2),
        (5, 3),
        (10, 3),
        (6, 4),
        (9, 4),
    ] {
        image.set(x, y, scale(gold, 0.85));
    }
    let (cx, cy) = (7.5f32, 10.0f32);
    for y in 0..16 {
        for x in 0..16 {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d <= 5.2 {
                image.set(x, y, if d > 4.0 { gold } else { color });
            }
        }
    }
    // Reflets : l'anneau s'éclaire en haut à gauche, la potion brille.
    image.set(5, 7, scale(gold, 1.25));
    image.set(6, 6, scale(gold, 1.25));
    image.set(6, 8, scale(color, 1.5));
    image.set(7, 8, scale(color, 1.5));
    image.set(9, 12, scale(color, 0.65));
    image.set(10, 11, scale(color, 0.65));
    outlined(image)
}

pub(super) fn generate(textures: &Path, items: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    for shelf in &SHELVES {
        out.push((
            textures.join(format!("shelf_{}_side.png", shelf.name)),
            shelf_side(shelf),
        ));
        out.push((
            textures.join(format!("shelf_{}_top.png", shelf.name)),
            shelf_top(shelf),
        ));
    }
    out.push((textures.join("enchant_library_side.png"), library_side()));
    out.push((textures.join("enchant_library_top.png"), library_top()));
    out.push((textures.join("simple_anvil_side.png"), anvil(false)));
    out.push((textures.join("simple_anvil_top.png"), anvil(true)));
    let cream = [0.95, 0.90, 0.72];
    out.push((
        items.join("book.png"),
        book([0.50, 0.30, 0.18], None, [0.92, 0.72, 0.25], false),
    ));
    // Le Guide du voyageur : vert forêt, rose des vents dorée.
    out.push((
        items.join("guide.png"),
        book(
            [0.16, 0.42, 0.22],
            Some(&EMBLEM_COMPASS),
            [0.95, 0.80, 0.30],
            false,
        ),
    ));
    out.push((
        items.join("enchanted_book.png"),
        book(
            [0.45, 0.25, 0.70],
            Some(&EMBLEM_STAR),
            [0.75, 0.95, 1.0],
            true,
        ),
    ));
    out.push((
        items.join("tome_weapons.png"),
        book([0.65, 0.16, 0.16], Some(&EMBLEM_SWORD), cream, false),
    ));
    out.push((
        items.join("tome_tools.png"),
        book([0.20, 0.50, 0.28], Some(&EMBLEM_PICKAXE), cream, false),
    ));
    out.push((
        items.join("tome_armor.png"),
        book([0.22, 0.32, 0.68], Some(&EMBLEM_SHIELD), cream, false),
    ));
    out.push((
        items.join("tome_extraction.png"),
        book(
            [0.62, 0.52, 0.14],
            Some(&EMBLEM_ARROW),
            [0.20, 0.12, 0.35],
            false,
        ),
    ));
    for (name, color) in [
        ("healing", [0.88, 0.20, 0.26]),
        ("regeneration", [0.95, 0.45, 0.70]),
        ("speed", [0.30, 0.80, 0.92]),
        ("strength", [0.85, 0.48, 0.20]),
        ("fire_resistance", [0.97, 0.66, 0.15]),
    ] {
        out.push((items.join(format!("charm_{name}.png")), charm(color)));
    }
    out
}
