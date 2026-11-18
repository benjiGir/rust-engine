//! Menuiserie, maçonnerie et cuivre : dessins adaptés aux coordonnées des boîtes.
use std::path::{Path, PathBuf};

use super::{COPPER, INK, IRON, Image, STONE, planks, scale};

pub(super) fn rect(image: &mut Image, area: [usize; 4], color: [f32; 3]) {
    let [x0, y0, x1, y1] = area;
    for y in y0..y1 {
        for x in x0..x1 {
            image.set(x, y, color);
        }
    }
}

pub(super) fn draw(kind: &str) -> Image {
    let wood = [0.52, 0.37, 0.25];
    let slate = [0.28, 0.35, 0.38];
    let linen = [0.85, 0.82, 0.68];
    let sage = [0.32, 0.49, 0.43];
    let mut image = match kind {
        "chest_top" | "chest_side" | "workbench_top" | "workbench_side" | "bed_side" => {
            planks(wood, 116)
        }
        _ => Image::filled(slate).noise(slate, 0.06, 412),
    };
    match kind {
        "chest_side" | "chest_top" | "vault_side" | "vault_top" => {
            let vault = kind.starts_with("vault");
            if vault {
                image = Image::filled(slate).noise(slate, 0.05, 416);
            }
            for x in [2, 12] {
                rect(&mut image, [x, 1, x + 2, 15], scale(COPPER, 0.72));
                rect(&mut image, [x, 2, x + 1, 14], COPPER);
                for y in [3, 12] {
                    image.set(x, y, IRON);
                }
            }
            if kind.ends_with("side") {
                rect(&mut image, [1, 5, 15, 6], INK);
                rect(&mut image, [6, 4, 10, 8], COPPER);
                image.set(7, 6, INK);
                if vault {
                    rect(&mut image, [5, 5, 11, 11], scale(IRON, 0.65));
                    rect(&mut image, [7, 4, 9, 12], COPPER);
                    rect(&mut image, [4, 7, 12, 9], COPPER);
                    rect(&mut image, [7, 7, 9, 9], INK);
                }
            }
        }
        "workbench_top" => {
            rect(&mut image, [2, 2, 14, 14], [0.69, 0.54, 0.36]);
            rect(&mut image, [3, 3, 13, 4], INK);
            for x in (3..13).step_by(2) {
                image.set(x, 4, INK);
            }
            for (x, y) in [(5, 9), (6, 8), (7, 7), (9, 11)] {
                image.set(x, y, scale(wood, 0.65));
            }
            for (x, y) in [(1, 1), (14, 1), (1, 14), (14, 14)] {
                image.set(x, y, COPPER);
            }
        }
        "workbench_side" => {
            rect(&mut image, [0, 0, 16, 1], scale(wood, 1.3));
            rect(&mut image, [0, 3, 16, 4], scale(wood, 0.6));
            rect(&mut image, [4, 4, 12, 5], INK);
            rect(&mut image, [7, 5, 9, 6], COPPER);
            for x in [2, 13] {
                for y in [2, 11] {
                    image.set(x, y, COPPER);
                }
            }
        }
        "furnace_front" | "alloy_furnace_side" => {
            image = super::cobblestone();
            rect(&mut image, [2, 0, 14, 2], slate);
            rect(&mut image, [4, 3, 12, 6], INK);
            for x in [5, 8, 11] {
                rect(&mut image, [x, 3, x + 1, 6], scale(IRON, 0.65));
            }
            rect(&mut image, [3, 7, 13, 8], COPPER);
            rect(&mut image, [3, 14, 13, 15], scale(STONE, 0.6));
            // L'animation GPU reste exactement dans x=4..11, y=8..13.
            rect(&mut image, [4, 8, 12, 14], INK);
            if kind.starts_with("alloy") {
                rect(&mut image, [5, 10, 11, 13], [0.64, 0.25, 0.09]);
                rect(&mut image, [6, 11, 10, 13], [0.91, 0.57, 0.18]);
            }
        }
        "furnace_top" | "alloy_furnace_top" => {
            for i in 1..15 {
                image.set(i, 1, scale(IRON, 0.7));
                image.set(1, i, scale(IRON, 0.7));
            }
            if kind.starts_with("alloy") {
                rect(&mut image, [6, 6, 10, 10], INK);
            }
        }
        "bed_top" => {
            image = Image::filled(wood);
            rect(&mut image, [2, 1, 14, 15], sage);
            for y in [6, 12] {
                rect(&mut image, [2, y, 14, y + 1], scale(sage, 1.25));
            }
            rect(&mut image, [3, 2, 13, 5], linen);
            rect(&mut image, [3, 4, 13, 5], scale(linen, 0.86));
            for y in 6..15 {
                image.set(3, y, scale(linen, 0.8));
                image.set(12, y, scale(linen, 0.8));
            }
        }
        "bed_side" => {
            // Matelas entre y=5 et 8 du modèle, donc lignes 8 à 10 du PNG.
            rect(&mut image, [2, 8, 14, 11], sage);
            rect(&mut image, [2, 8, 14, 9], scale(sage, 1.25));
            rect(&mut image, [3, 7, 13, 8], linen);
            rect(&mut image, [1, 11, 15, 12], scale(wood, 1.25));
        }
        "bed_head_top" | "bed_foot_top" => {
            let head = kind == "bed_head_top";
            image = planks(wood, 116);
            rect(
                &mut image,
                [2, if head { 2 } else { 0 }, 14, if head { 16 } else { 15 }],
                sage,
            );
            for x in [2, 13] {
                rect(
                    &mut image,
                    [
                        x,
                        if head { 2 } else { 0 },
                        x + 1,
                        if head { 16 } else { 15 },
                    ],
                    scale(sage, 0.70),
                );
            }
            if head {
                rect(&mut image, [3, 3, 13, 9], linen);
                rect(&mut image, [3, 8, 13, 9], scale(linen, 0.75));
                rect(&mut image, [3, 10, 13, 11], scale(sage, 1.35));
            } else {
                rect(&mut image, [3, 10, 13, 12], scale(sage, 1.25));
                for x in (4..13).step_by(2) {
                    image.set(x, 13, scale(linen, 0.72));
                }
            }
        }
        "bed_long_side" => {
            image = planks(wood, 116);
            rect(&mut image, [0, 5, 16, 7], scale(linen, 0.86));
            rect(&mut image, [0, 7, 16, 11], sage);
            rect(&mut image, [0, 7, 16, 8], scale(sage, 1.22));
            rect(&mut image, [0, 10, 16, 11], scale(sage, 0.70));
            rect(&mut image, [0, 11, 16, 12], scale(wood, 1.20));
        }
        "enchanting_top" | "rune_table" => {
            rect(&mut image, [1, 1, 15, 15], scale(slate, 1.1));
            for (x, y) in [(2, 2), (12, 2), (2, 12), (12, 12)] {
                rect(&mut image, [x, y, x + 2, y + 1], COPPER);
                image.set(x, y + 1, COPPER);
            }
            if kind == "enchanting_top" {
                rect(&mut image, [3, 4, 13, 12], linen);
                rect(&mut image, [7, 4, 9, 12], [0.39, 0.27, 0.39]);
                for y in [6, 8, 10] {
                    rect(&mut image, [4, y, 6, y + 1], scale(linen, 0.65));
                    rect(&mut image, [10, y, 12, y + 1], scale(linen, 0.65));
                }
            } else {
                for (x, y) in [
                    (7, 4),
                    (8, 5),
                    (9, 6),
                    (10, 7),
                    (9, 8),
                    (8, 9),
                    (7, 10),
                    (6, 9),
                    (5, 8),
                    (4, 7),
                    (5, 6),
                    (6, 5),
                ] {
                    image.set(x, y, [0.48, 0.71, 0.68]);
                }
                rect(&mut image, [7, 6, 9, 9], [0.72, 0.58, 0.83]);
            }
        }
        "enchanting_side" => {
            rect(&mut image, [0, 5, 16, 6], COPPER);
            rect(&mut image, [0, 7, 16, 8], scale(slate, 0.6));
            rect(&mut image, [5, 10, 11, 14], scale(slate, 0.75));
            for x in [6, 9] {
                rect(&mut image, [x, 10, x + 1, 14], COPPER);
            }
            rect(&mut image, [3, 2, 13, 5], linen);
        }
        "brewing_top" | "brewing_side" => {
            image = Image::filled(COPPER).noise(COPPER, 0.05, 417);
            if kind.ends_with("side") {
                rect(&mut image, [0, 14, 16, 16], slate);
                rect(&mut image, [3, 9, 9, 10], scale(COPPER, 1.2));
                rect(&mut image, [11, 8, 15, 14], [0.33, 0.55, 0.50]);
                rect(&mut image, [12, 9, 13, 13], [0.60, 0.75, 0.64]);
            } else {
                rect(&mut image, [1, 1, 15, 15], slate);
                rect(&mut image, [3, 4, 9, 12], COPPER);
                rect(&mut image, [5, 6, 7, 10], scale(COPPER, 0.75));
                rect(&mut image, [11, 5, 15, 11], [0.33, 0.55, 0.50]);
                rect(&mut image, [6, 7, 14, 9], COPPER);
            }
        }
        "anvil_top" | "anvil_side" => {
            image = Image::filled(scale(IRON, 0.65)).noise(scale(IRON, 0.65), 0.05, 419);
            if kind.ends_with("side") {
                rect(&mut image, [0, 1, 16, 2], scale(IRON, 1.05));
                rect(&mut image, [0, 5, 16, 6], scale(IRON, 0.45));
                rect(&mut image, [0, 12, 16, 13], scale(IRON, 0.8));
            } else {
                for (x, y) in [(5, 5), (6, 6), (9, 9), (10, 8)] {
                    image.set(x, y, scale(IRON, 0.85));
                }
            }
        }
        _ => unreachable!("matière de poste inconnue : {kind}"),
    }
    image
}

pub(super) fn mod_textures(root: &Path) -> Vec<(PathBuf, Image)> {
    [
        ("entrepot", "vault_top"),
        ("entrepot", "vault_side"),
        ("magie", "rune_table"),
        ("forge", "anvil_top"),
        ("forge", "anvil_side"),
        ("forge", "alloy_furnace_top"),
        ("forge", "alloy_furnace_side"),
    ]
    .into_iter()
    .map(|(namespace, kind)| {
        (
            root.join(format!("mods/{namespace}/textures/{kind}.png")),
            draw(kind),
        )
    })
    .collect()
}
