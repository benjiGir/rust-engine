//! L'établi de taille et les icônes génériques des vingt-trois pièces (couche, dalle, pentes,
//! microbloc, grain, escalier, marche, poutre, tranches, panneaux, planchette, muret, barrière,
//! portillon, vitres). Dans le jeu, une pile de pièces est dessinée avec les textures de son
//! matériau (voir `render/item_art.rs`) : ces icônes ne servent que là où aucun matériau n'est
//! connu (les recettes, les guides).

use std::path::{Path, PathBuf};

use super::workstations::rect;
use super::{COPPER, INK, IRON, Image, STONE, planks, scale};

/// La pierre claire des pièces sans matériau.
const PIECE: [f32; 3] = [0.66, 0.68, 0.70];

/// Un établi de taille : du bois cerclé de fer ; le dessus porte un ciseau et une équerre posés sur
/// des copeaux ; la face, un râtelier d'outils.
fn table(top: bool) -> Image {
    let mut image = planks([0.50, 0.35, 0.22], 971);
    rect(&mut image, [0, 0, 16, 1], scale(IRON, 0.8));
    rect(&mut image, [0, 15, 16, 16], scale(IRON, 0.6));
    rect(&mut image, [0, 0, 1, 16], scale(IRON, 0.75));
    rect(&mut image, [15, 0, 16, 16], scale(IRON, 0.6));
    if top {
        rect(&mut image, [2, 2, 14, 14], scale(STONE, 0.6));
        // Un bloc en cours de taille : une marche taillée dans un cube.
        rect(&mut image, [3, 3, 9, 9], scale(PIECE, 0.9));
        rect(&mut image, [3, 3, 9, 5], scale(PIECE, 1.15));
        rect(&mut image, [7, 3, 9, 7], scale(PIECE, 0.7));
        // Le ciseau, de biais, et son manche de cuivre.
        for i in 0..5 {
            image.set(10 + i / 2, 8 + i, scale(IRON, 1.2));
        }
        rect(&mut image, [12, 12, 14, 14], COPPER);
        // Des copeaux.
        for (x, y) in [(4, 11), (6, 12), (3, 13), (9, 12)] {
            image.set(x, y, scale(PIECE, 1.2));
        }
    } else {
        rect(&mut image, [2, 4, 14, 12], INK);
        // Trois outils pendus : ciseau, équerre, maillet.
        rect(&mut image, [4, 5, 5, 10], IRON);
        rect(&mut image, [4, 10, 5, 11], COPPER);
        rect(&mut image, [7, 5, 8, 11], scale(IRON, 0.8));
        rect(&mut image, [7, 10, 11, 11], scale(IRON, 0.8));
        rect(&mut image, [11, 5, 13, 8], COPPER);
        rect(&mut image, [12, 8, 13, 11], scale(COPPER, 0.7));
    }
    image
}

/// Un pictogramme : des rectangles de pierre sur fond transparent, clairs dessus et sombres
/// dessous.
fn piece(kind: &str) -> Image {
    let mut image = Image::transparent();
    let light = scale(PIECE, 1.15);
    let dark = scale(PIECE, 0.7);
    let line = |image: &mut Image, area: [usize; 4]| {
        rect(image, area, PIECE);
        rect(image, [area[0], area[1], area[2], area[1] + 1], light);
        rect(image, [area[0], area[3] - 1, area[2], area[3]], dark);
    };
    match kind {
        "layer" => line(&mut image, [2, 10, 14, 12]),
        "slab" => line(&mut image, [2, 7, 14, 12]),
        "slope_layer" => {
            for x in 2..14 {
                let height = 1 + (13 - x) / 8;
                rect(&mut image, [x, 12 - height, x + 1, 12], PIECE);
                image.set(x, 12 - height, light);
            }
        }
        "slope" => {
            for x in 2..14 {
                let height = (14 - x) * 8 / 12 + 1;
                rect(&mut image, [x, 12 - height, x + 1, 12], PIECE);
                image.set(x, 12 - height, light);
            }
            rect(&mut image, [2, 11, 14, 12], dark);
        }
        "micro" => {
            line(&mut image, [5, 6, 11, 12]);
            rect(&mut image, [10, 7, 11, 12], dark);
        }
        "nano" => line(&mut image, [7, 9, 9, 11]),
        "stairs" => {
            for (i, x) in [2usize, 6, 10].into_iter().enumerate() {
                line(&mut image, [x, 10 - i * 3, x + 4, 12]);
            }
        }
        "vertical_stairs" => {
            line(&mut image, [3, 4, 8, 12]);
            line(&mut image, [8, 8, 13, 12]);
            rect(&mut image, [12, 9, 13, 12], dark);
        }
        "step" => {
            line(&mut image, [3, 5, 9, 12]);
            rect(&mut image, [8, 6, 9, 12], dark);
        }
        "beam" => {
            line(&mut image, [5, 2, 11, 13]);
            rect(&mut image, [10, 3, 11, 13], dark);
        }
        "slice" => {
            line(&mut image, [3, 2, 5, 13]);
            line(&mut image, [8, 2, 10, 13]);
        }
        "corner_slice" => {
            line(&mut image, [3, 8, 7, 12]);
            line(&mut image, [8, 5, 11, 8]);
        }
        "half_layer" => {
            line(&mut image, [2, 9, 14, 11]);
            rect(&mut image, [3, 8, 13, 9], light);
        }
        "half_panel" => {
            line(&mut image, [2, 5, 14, 8]);
            line(&mut image, [2, 9, 8, 12]);
        }
        "quarter_panel" => line(&mut image, [4, 6, 8, 10]),
        "board" => {
            line(&mut image, [2, 3, 4, 13]);
            line(&mut image, [4, 11, 14, 13]);
        }
        "corner_slope_layer" => {
            for x in 2..14 {
                let height = 1 + (x - 2).min(13 - x);
                rect(&mut image, [x, 12 - height, x + 1, 12], PIECE);
                image.set(x, 12 - height, light);
            }
            rect(&mut image, [2, 11, 14, 12], dark);
        }
        "inner_slope_layer" => {
            for x in 2..14 {
                let height = 7 - (x - 2).min(13 - x).min(5);
                rect(&mut image, [x, 12 - height, x + 1, 12], PIECE);
                image.set(x, 12 - height, light);
            }
            rect(&mut image, [2, 11, 14, 12], dark);
        }
        "wall" => {
            line(&mut image, [2, 7, 14, 12]);
            line(&mut image, [6, 3, 10, 12]);
        }
        "fence" => {
            line(&mut image, [3, 3, 5, 13]);
            line(&mut image, [11, 3, 13, 13]);
            line(&mut image, [5, 5, 11, 6]);
            line(&mut image, [5, 9, 11, 10]);
        }
        "gate" => {
            line(&mut image, [2, 3, 4, 13]);
            line(&mut image, [12, 3, 14, 13]);
            line(&mut image, [4, 5, 12, 6]);
            line(&mut image, [4, 10, 12, 11]);
            rect(&mut image, [7, 6, 9, 10], dark);
        }
        "pane" => {
            let glass = [0.62, 0.82, 0.90];
            rect(&mut image, [3, 3, 13, 13], scale(glass, 0.8));
            rect(&mut image, [4, 4, 12, 12], glass);
            rect(&mut image, [4, 4, 6, 8], scale(glass, 1.15));
        }
        _ => {
            let glass = [0.62, 0.82, 0.90];
            rect(&mut image, [2, 8, 14, 11], scale(glass, 0.8));
            rect(&mut image, [3, 8, 13, 10], glass);
        }
    }
    image
}

pub(super) fn generate(textures: &Path, items: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = vec![
        (textures.join("shaping_table_top.png"), table(true)),
        (textures.join("shaping_table_side.png"), table(false)),
    ];
    for kind in [
        "layer",
        "slab",
        "slope_layer",
        "slope",
        "micro",
        "nano",
        "stairs",
        "vertical_stairs",
        "step",
        "beam",
        "slice",
        "corner_slice",
        "half_layer",
        "half_panel",
        "quarter_panel",
        "board",
        "corner_slope_layer",
        "inner_slope_layer",
        "wall",
        "fence",
        "gate",
        "pane",
        "flat_pane",
    ] {
        out.push((items.join(format!("shape_{kind}.png")), piece(kind)));
    }
    out
}
