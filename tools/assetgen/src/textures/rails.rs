//! Les rails (droits et en virage), le chargeur et le déchargeur, et les icônes des wagons.
//!
//! Un rail est dessiné pour la direction nord-sud (traverses en travers, deux fers le long de
//! l'image) et pour le virage sud-est (un quart de cercle centré sur le coin bas-droit) ; le
//! mailleur les fait tourner de quarts de tour (voir `FLAG_TURN_SHIFT`). Le fond est
//! transparent : le sol reste visible autour des traverses.
use std::path::{Path, PathBuf};

use super::{COPPER, INK, IRON, Image, SIZE, items::icon, scale, workstations::rect};

const WOOD: [f32; 3] = [0.43, 0.30, 0.19];

/// Les couleurs d'un rail : le fer, les traverses, et un accent selon la sorte.
fn palette(kind: &str) -> ([f32; 3], [f32; 3]) {
    match kind {
        "booster_rail" => ([0.93, 0.74, 0.30], scale(WOOD, 1.0)),
        "activator_rail" => ([0.55, 0.20, 0.17], scale(WOOD, 0.85)),
        _ => (scale(IRON, 0.95), WOOD),
    }
}

/// Une traverse ou un fer à ce pixel : le dessin du rail droit nord-sud.
fn straight_pixel(kind: &str, x: usize, y: usize) -> Option<[f32; 3]> {
    let (iron, wood) = palette(kind);
    if x == 4 || x == 11 {
        return Some(if y % 4 == 1 { scale(iron, 1.15) } else { iron });
    }
    if x == 3 || x == 12 {
        return Some(scale(iron, 0.55));
    }
    let tie = y % 4 == 1 || y % 4 == 2;
    if tie && (2..=13).contains(&x) {
        return Some(if y % 4 == 1 {
            scale(wood, 1.15)
        } else {
            scale(wood, 0.8)
        });
    }
    None
}

/// Le dessin d'un rail droit de cette sorte (nord-sud).
pub(super) fn straight(kind: &str) -> Image {
    let mut image = Image::transparent();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if let Some(color) = straight_pixel(kind, x, y) {
                image.set(x, y, color);
            }
        }
    }
    match kind {
        // Le rail accélérateur : des plaques de cuivre entre les fers.
        "booster_rail" => {
            for y in [2usize, 6, 10, 14] {
                rect(&mut image, [5, y, 11, y + 1], scale(COPPER, 0.95));
            }
        }
        // Le détecteur : une plaque de pierre entre les fers.
        "detector_rail" => {
            rect(&mut image, [5, 5, 11, 11], [0.55, 0.57, 0.57]);
            rect(&mut image, [5, 5, 11, 6], [0.72, 0.74, 0.73]);
            rect(&mut image, [5, 10, 11, 11], [0.38, 0.4, 0.4]);
        }
        // L'activateur : trois pastilles rouges.
        "activator_rail" => {
            for y in [3usize, 7, 11] {
                rect(&mut image, [6, y, 10, y + 2], [0.92, 0.25, 0.2]);
            }
        }
        // Le butoir : une poutre en travers, tenue par deux montants.
        "buffer_stop" => {
            rect(&mut image, [1, 6, 15, 10], scale(INK, 1.7));
            rect(&mut image, [1, 6, 15, 7], [0.82, 0.62, 0.2]);
            rect(&mut image, [1, 9, 15, 10], scale(INK, 1.1));
            for x in [2usize, 12] {
                rect(&mut image, [x, 4, x + 2, 12], scale(IRON, 0.75));
            }
        }
        _ => {}
    }
    image
}

/// Le virage sud-est : le même rail plié autour du coin bas-droit.
pub(super) fn curve() -> Image {
    let mut image = Image::transparent();
    let (iron, wood) = palette("rail");
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0);
            let radius = (dx * dx + dy * dy).sqrt();
            // L'angle depuis le sud (0) vers l'est (quart de tour) : de 180° à 270°.
            let angle = dy.atan2(dx).rem_euclid(std::f32::consts::TAU);
            let along = (angle - std::f32::consts::PI) * 8.0;
            if (angle < std::f32::consts::PI - 0.08) || (angle > 1.5 * std::f32::consts::PI + 0.08)
            {
                continue;
            }
            let on = |r: f32| (radius - r).abs() < 0.62;
            if on(4.5) || on(11.5) {
                image.set(x, y, iron);
            } else if (radius - 4.5).abs() < 1.7 || (radius - 11.5).abs() < 1.7 {
                image.set(x, y, scale(iron, 0.55));
            } else if (3.2..12.8).contains(&radius) {
                let phase = along.rem_euclid(4.0);
                if phase < 1.0 {
                    image.set(x, y, scale(wood, 1.15));
                } else if phase < 2.0 {
                    image.set(x, y, scale(wood, 0.8));
                }
            }
        }
    }
    image
}

/// Un côté du chargeur ou du déchargeur : une caisse d'ardoise, un entonnoir (flèche vers le
/// bas pour charger, vers le haut pour décharger).
fn hopper_side(load: bool) -> Image {
    let slate = [0.31, 0.34, 0.35];
    let mut image = Image::filled(slate).noise(slate, 0.05, if load { 801 } else { 802 });
    for k in 0..SIZE {
        for edge in [0, SIZE - 1] {
            image.set(k, edge, scale(slate, 0.6));
            image.set(edge, k, scale(slate, 0.6));
        }
    }
    let arrow = if load {
        [0.93, 0.74, 0.30]
    } else {
        [0.45, 0.78, 0.55]
    };
    rect(&mut image, [7, 3, 9, 11], arrow);
    // La pointe de la flèche.
    for k in 0..4usize {
        if load {
            rect(&mut image, [4 + k, 8 + k, 12 - k, 9 + k], arrow);
        } else {
            rect(&mut image, [4 + k, 6 - k, 12 - k, 7 - k], arrow);
        }
    }
    image
}

/// Le dessus : un entonnoir sombre dans un cadre de fer.
fn hopper_top(load: bool) -> Image {
    let slate = [0.27, 0.3, 0.31];
    let mut image = Image::filled(slate).noise(slate, 0.04, if load { 803 } else { 804 });
    rect(&mut image, [1, 1, 15, 15], scale(IRON, 0.7));
    rect(&mut image, [3, 3, 13, 13], INK);
    rect(&mut image, [5, 5, 11, 11], scale(INK, 0.6));
    if !load {
        rect(&mut image, [3, 3, 13, 4], [0.45, 0.78, 0.55]);
    } else {
        rect(&mut image, [3, 3, 13, 4], [0.93, 0.74, 0.3]);
    }
    image
}

/// Un wagon vu de côté : la caisse (`C`), le bord (`c`), les roues (`W`) et ce qui la surmonte.
fn cart_icon(kind: &str) -> Image {
    let mut rows: [&str; 16] = [
        "................",
        "................",
        "................",
        "................",
        "................",
        "................",
        "................",
        "..cccccccccccc..",
        "..CCCCCCCCCCCC..",
        "..CCCCCCCCCCCC..",
        "..CCCCCCCCCCCC..",
        "..cccccccccccc..",
        "...WW......WW...",
        "...WW......WW...",
        "................",
        "................",
    ];
    // Ce qui est dessiné dans (ou au-dessus de) la caisse.
    let extra: &[(usize, &str)] = match kind {
        "chest_cart" => &[
            (3, "....KKKKKKKK...."),
            (4, "....KKKKKKKK...."),
            (5, "....KkkkkkkK...."),
            (6, "....KKKLLKKK...."),
        ],
        "hopper_cart" => &[
            (4, "...HHHHHHHHHH..."),
            (5, "....HHHHHHHH...."),
            (6, ".....HHHHHH....."),
        ],
        "locomotive" => &[
            (2, "..........PP...."),
            (3, "..........PP...."),
            (4, ".....FFFFFFFFF.."),
            (5, ".....FFFFFFFFF.."),
            (6, ".....FfffffFFF.."),
        ],
        _ => &[],
    };
    for &(row, text) in extra {
        rows[row] = text;
    }
    let (body, rim) = if kind == "locomotive" {
        ([0.62, 0.2, 0.16], [0.45, 0.13, 0.11])
    } else {
        ([0.52, 0.56, 0.58], [0.78, 0.82, 0.83])
    };
    icon(
        &rows,
        &[
            ('C', body),
            ('c', rim),
            ('W', [0.14, 0.14, 0.16]),
            ('K', [0.5, 0.34, 0.2]),
            ('k', [0.36, 0.24, 0.14]),
            ('L', [0.85, 0.7, 0.25]),
            ('H', [0.2, 0.22, 0.24]),
            ('F', [0.3, 0.32, 0.34]),
            ('f', [0.2, 0.21, 0.23]),
            ('P', [0.15, 0.15, 0.17]),
        ],
    )
}

/// L'attelage : une chaîne de trois maillons.
fn coupler() -> Image {
    let mask = [
        "................",
        "................",
        "................",
        "................",
        "...hhh..........",
        "..hLLLh.........",
        "..hL.Lhhh.......",
        "..hLLLLLLh......",
        "...hhhhL.Lh.....",
        ".......hLLLLh...",
        ".......hL.LLh...",
        "........hLLh....",
        "................",
        "................",
        "................",
        "................",
    ];
    icon(&mask, &[('L', [0.74, 0.78, 0.8]), ('h', [0.36, 0.4, 0.42])])
}

/// Tous les fichiers de cette famille : textures de blocs et icônes d'objets.
pub(super) fn generate(textures: &Path, items: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    for kind in [
        "rail",
        "booster_rail",
        "detector_rail",
        "activator_rail",
        "buffer_stop",
    ] {
        out.push((textures.join(format!("{kind}.png")), straight(kind)));
    }
    out.push((textures.join("rail_curve.png"), curve()));
    for (name, load) in [("cart_loader", true), ("cart_unloader", false)] {
        out.push((textures.join(format!("{name}_top.png")), hopper_top(load)));
        out.push((textures.join(format!("{name}_side.png")), hopper_side(load)));
    }
    for kind in ["minecart", "chest_cart", "hopper_cart", "locomotive"] {
        out.push((items.join(format!("{kind}.png")), cart_icon(kind)));
    }
    out.push((items.join("coupler.png"), coupler()));
    out
}
