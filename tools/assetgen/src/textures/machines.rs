//! Pièces mécaniques et circuits, en ardoise, acier et cuivre patiné.
use super::{COPPER, INK, IRON, Image, scale, workstations::rect};

pub(super) fn draw(kind: &str, on: bool) -> Image {
    let slate = [0.29, 0.36, 0.38];
    let patina = [0.28, 0.48, 0.43];
    let glow = if on {
        [0.96, 0.68, 0.29]
    } else {
        scale(COPPER, 0.6)
    };
    let mut image = Image::filled(slate).noise(slate, 0.045, 512);
    match kind {
        "cable" | "pipe" | "shaft" => {
            let base = match kind {
                "cable" => INK,
                "pipe" => patina,
                _ => scale(IRON, 0.7),
            };
            image = Image::filled(base).noise(base, 0.04, 513);
            for k in [4, 10] {
                rect(&mut image, [k, 0, k + 2, 16], scale(COPPER, 0.8));
                rect(&mut image, [0, k, 16, k + 1], scale(COPPER, 0.65));
            }
            rect(&mut image, [7, 0, 8, 16], scale(base, 1.35));
            if kind == "cable" {
                // Le conducteur reste lisible sur une section de seulement quatre pixels.
                rect(&mut image, [7, 0, 9, 16], scale(COPPER, 0.92));
                rect(&mut image, [0, 7, 16, 9], scale(COPPER, 0.92));
            }
        }
        "solar_panel" => {
            image = Image::filled([0.17, 0.29, 0.39]);
            for y in [1, 6, 11] {
                for x in [1, 6, 11] {
                    rect(&mut image, [x, y, x + 4, y + 4], [0.23, 0.39, 0.49]);
                    rect(&mut image, [x, y, x + 4, y + 1], [0.39, 0.55, 0.59]);
                }
            }
            for k in 0..16 {
                image.set(k, 0, COPPER);
                image.set(0, k, COPPER);
                image.set(k, 15, scale(COPPER, 0.7));
                image.set(15, k, scale(COPPER, 0.7));
            }
        }
        "battery" => {
            rect(&mut image, [2, 3, 14, 4], COPPER);
            rect(&mut image, [2, 12, 14, 13], COPPER);
            rect(&mut image, [5, 5, 11, 11], INK);
            for y in [6, 8, 10] {
                rect(&mut image, [6, y, 10, y + 1], patina);
            }
            rect(&mut image, [3, 0, 7, 3], COPPER);
            rect(&mut image, [10, 0, 13, 3], IRON);
        }
        "lamp" | "power_lamp" => {
            let glass = if on {
                [0.94, 0.77, 0.40]
            } else {
                [0.52, 0.57, 0.44]
            };
            rect(&mut image, [2, 4, 14, 12], glass);
            for x in [2, 7, 13] {
                rect(&mut image, [x, 3, x + 1, 13], scale(COPPER, 0.8));
            }
            rect(&mut image, [1, 2, 15, 4], COPPER);
            rect(&mut image, [2, 12, 14, 13], COPPER);
            rect(&mut image, [4, 5, 6, 10], scale(glass, 1.12));
        }
        "lever" | "button" | "inverter" | "repeater" => {
            // Le socle occupe les dernières lignes, les contacts la zone centrale.
            rect(&mut image, [1, 14, 15, 15], scale(IRON, 0.85));
            rect(&mut image, [3, 2, 13, 13], scale(slate, 1.15));
            rect(&mut image, [7, 2, 9, 14], COPPER);
            if kind == "lever" {
                rect(&mut image, [7, 0, 9, 10], [0.53, 0.38, 0.25]);
            }
            if kind == "button" {
                rect(&mut image, [5, 6, 11, 11], glow);
            }
            let dots = if kind == "repeater" {
                vec![(5, 4), (10, 11)]
            } else {
                vec![(7, 7)]
            };
            for (x, y) in dots {
                rect(&mut image, [x, y, x + 2, y + 2], glow);
            }
        }
        "piston" | "sticky_piston" | "piston_head" => {
            image = Image::filled(scale(IRON, 0.7)).noise(scale(IRON, 0.7), 0.04, 514);
            for y in [2, 5, 12] {
                rect(&mut image, [1, y, 15, y + 1], COPPER);
            }
            rect(&mut image, [5, 5, 11, 11], INK);
            rect(
                &mut image,
                [6, 6, 10, 10],
                if kind == "sticky_piston" {
                    patina
                } else {
                    IRON
                },
            );
            if kind == "piston_head" {
                image = super::planks([0.57, 0.42, 0.27], 515);
                for k in [2, 12] {
                    rect(&mut image, [k, 0, k + 2, 16], COPPER);
                }
            }
        }
        "tank" => {
            image = Image::filled(patina).noise(patina, 0.04, 516);
            for y in [1, 3, 12, 14] {
                rect(&mut image, [1, y, 15, y + 1], scale(COPPER, 0.8));
            }
            rect(&mut image, [7, 4, 9, 12], INK);
            rect(&mut image, [7, 7, 8, 11], [0.46, 0.67, 0.70]);
            for y in [5, 8, 11] {
                image.set(10, y, IRON);
            }
        }
        "balloon" => {
            image = Image::filled([0.69, 0.46, 0.32]).noise([0.69, 0.46, 0.32], 0.04, 517);
            for x in [3, 7, 11] {
                rect(&mut image, [x, 0, x + 1, 16], [0.81, 0.69, 0.49]);
            }
            for y in [2, 13] {
                rect(&mut image, [0, y, 16, y + 1], [0.43, 0.33, 0.25]);
            }
            for (x, y) in [(5, 5), (6, 6), (7, 7)] {
                image.set(x, y, [0.82, 0.64, 0.41]);
            }
        }
        "helm" => {
            image = super::planks([0.50, 0.35, 0.23], 518);
            for k in [2, 7, 12] {
                rect(&mut image, [k, 6, k + 2, 10], COPPER);
            }
        }
        "gate" => {
            image = Image::filled(scale(IRON, 0.62)).noise(scale(IRON, 0.62), 0.05, 519);
            for y in [3, 10, 15] {
                rect(&mut image, [0, y, 16, y + 1], COPPER);
            }
        }
        "fan" | "propeller" => {
            image = Image::filled(scale(IRON, 0.8)).noise(scale(IRON, 0.8), 0.04, 520);
            rect(&mut image, [6, 6, 10, 10], COPPER);
            rect(&mut image, [7, 7, 9, 9], glow);
            for k in [2, 12] {
                rect(&mut image, [k, 0, k + 1, 16], scale(IRON, 0.6));
            }
        }
        "pump" | "motor" | "filler" | "assembler" | "press" | "crank" | "machine_frame" => {
            for y in [1, 13] {
                rect(&mut image, [1, y, 15, y + 1], scale(IRON, 0.75));
            }
            for (x, y) in [(2, 2), (13, 2), (2, 13), (13, 13)] {
                image.set(x, y, COPPER);
            }
            if kind == "pump" {
                rect(&mut image, [6, 6, 10, 10], patina);
                rect(&mut image, [7, 4, 9, 12], COPPER);
            }
            if kind == "motor" {
                for x in [3, 5, 7, 9, 11] {
                    rect(&mut image, [x, 4, x + 1, 10], INK);
                }
                rect(&mut image, [6, 6, 10, 10], COPPER);
            }
            if kind == "filler" {
                rect(&mut image, [7, 1, 10, 7], COPPER);
                rect(&mut image, [5, 11, 11, 13], patina);
            }
            if kind == "assembler" {
                rect(&mut image, [4, 3, 12, 4], COPPER);
                rect(&mut image, [6, 7, 10, 11], scale(IRON, 1.1));
            }
        }
        _ => unreachable!("machine inconnue {kind}"),
    }
    image
}
