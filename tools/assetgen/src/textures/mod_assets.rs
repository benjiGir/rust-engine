//! Les quatre mods de contenu suivent le même générateur déterministe que le jeu.
use super::{COPPER, INK, IRON, Image, items, machines, scale, workstations::rect};
use std::path::{Path, PathBuf};

pub(super) fn generate(root: &Path) -> Vec<(PathBuf, Image)> {
    let mut out = Vec::new();
    for (kind, color) in [("gate", COPPER), ("return_gate", [0.33, 0.50, 0.67])] {
        let mut image = Image::filled(scale(color, 0.65));
        for y in 0..16 {
            for x in 0..16 {
                let wave = ((x + y * 2) % 7) as f32 / 7.0;
                image.set(x, y, scale(color, 0.72 + wave * 0.3));
            }
        }
        for (x, y) in [(2, 2), (12, 2), (2, 12), (12, 12)] {
            rect(&mut image, [x, y, x + 2, y + 1], IRON);
            image.set(x, y + 1, IRON);
        }
        out.push((
            root.join(format!("mods/mecanique/textures/{kind}.png")),
            image,
        ));
    }
    for kind in ["crypt_brick", "spike_trap", "spawner", "boss_spawner"] {
        let mut image = super::cobblestone();
        if kind == "crypt_brick" {
            for pixel in &mut image.pixels {
                pixel[0] = (pixel[0] as f32 * 0.72) as u8;
                pixel[1] = (pixel[1] as f32 * 0.78) as u8;
                pixel[2] = (pixel[2] as f32 * 0.85) as u8;
            }
        } else {
            image = machines::draw("machine_frame", false);
            if kind == "spike_trap" {
                for x in [3, 10] {
                    for y in [3, 10] {
                        rect(&mut image, [x, y, x + 3, y + 3], IRON);
                        image.set(x + 1, y + 1, scale(IRON, 1.25));
                    }
                }
                rect(&mut image, [1, 14, 15, 16], scale(IRON, 0.6));
            } else {
                let crystal = if kind == "boss_spawner" {
                    [0.68, 0.36, 0.30]
                } else {
                    [0.35, 0.64, 0.53]
                };
                rect(&mut image, [5, 5, 11, 11], crystal);
                rect(&mut image, [6, 6, 8, 9], scale(crystal, 1.3));
                for k in [1, 7, 14] {
                    rect(&mut image, [k, 0, k + 1, 16], scale(COPPER, 0.75));
                }
            }
        }
        out.push((
            root.join(format!("mods/donjons/textures/{kind}.png")),
            image,
        ));
    }
    for kind in ["sorter", "harvester", "drill"] {
        for face in ["top", "side"] {
            let mut image = machines::draw("machine_frame", false);
            if kind == "sorter" {
                rect(&mut image, [4, 4, 12, 12], INK);
                rect(&mut image, [5, 5, 11, 6], COPPER);
                for y in [7, 9] {
                    rect(&mut image, [5, y, 11, y + 1], [0.43, 0.62, 0.61]);
                }
            } else if kind == "harvester" {
                rect(&mut image, [2, 4, 14, 12], [0.37, 0.47, 0.32]);
                rect(&mut image, [2, 11, 14, 13], COPPER);
                for x in [3, 6, 9, 12] {
                    rect(&mut image, [x, 12, x + 1, 16], IRON);
                }
                if face == "top" {
                    for x in [5, 8, 11] {
                        rect(&mut image, [x, 5, x + 1, 10], [0.65, 0.64, 0.38]);
                    }
                }
            } else {
                rect(&mut image, [5, 2, 11, 7], COPPER);
                rect(&mut image, [6, 7, 10, 13], IRON);
                for y in [7, 10, 13] {
                    rect(&mut image, [6, y, 10, y + 1], INK);
                }
            }
            out.push((
                root.join(format!("mods/entrepot/textures/{kind}_{face}.png")),
                image,
            ));
        }
    }
    for name in [
        "wand",
        "mana_crystal",
        "rune_flame",
        "rune_heal",
        "rune_blink",
        "rune_power",
        "rune_reach",
    ] {
        let image = if name == "mana_crystal" {
            items::material("gem", [0.42, 0.64, 0.72])
        } else if name == "wand" {
            let mut image = Image::transparent();
            for k in 0..10 {
                image.plot(2 + k, 13 - k, [0.53, 0.36, 0.25]);
                image.plot(3 + k, 13 - k, COPPER);
            }
            for y in 1i32..6 {
                for x in 9i32..14 {
                    if (x - 11).abs() + (y - 3).abs() < 3 {
                        image.plot(x, y, [0.50, 0.74, 0.76]);
                    }
                }
            }
            items::outlined(image)
        } else {
            rune(name)
        };
        out.push((root.join(format!("mods/magie/items/{name}.png")), image));
    }
    let mut sigil = rune("warden_sigil");
    for (x, y) in [(4, 4), (11, 4), (4, 11), (11, 11)] {
        sigil.set(x, y, COPPER);
    }
    out.push((root.join("mods/donjons/items/warden_sigil.png"), sigil));
    for name in [
        "bronze_ingot",
        "steel_ingot",
        "steel_plate",
        "bronze_hammer",
        "pickaxe_head",
        "axe_head",
        "sword_blade",
        "steel_pickaxe",
        "steel_axe",
        "steel_sword",
        "steel_helmet",
        "steel_chestplate",
        "steel_leggings",
        "steel_boots",
    ] {
        let image = match name {
            "bronze_ingot" => items::material("ingot", COPPER),
            "steel_ingot" => items::material("ingot", IRON),
            "steel_plate" => super::metal_plate(IRON),
            "bronze_hammer" => {
                let mut image = Image::transparent();
                for k in 0..10 {
                    image.plot(2 + k, 13 - k, [0.50, 0.35, 0.23]);
                    image.plot(3 + k, 13 - k, [0.63, 0.45, 0.29]);
                }
                for k in 0..7 {
                    for t in 0..4 {
                        image.plot(
                            4 + k,
                            1 + k + t,
                            scale(COPPER, if t == 0 { 1.25 } else { 0.9 }),
                        );
                    }
                }
                items::outlined(image)
            }
            "pickaxe_head" | "axe_head" | "sword_blade" => forged_part(name),
            _ => {
                let kind = name.strip_prefix("steel_").unwrap();
                if ["helmet", "chestplate", "leggings", "boots"].contains(&kind) {
                    items::armor(kind, IRON)
                } else {
                    items::tool(kind, IRON)
                }
            }
        };
        out.push((root.join(format!("mods/forge/items/{name}.png")), image));
    }
    out
}

fn forged_part(kind: &str) -> Image {
    let mut image = Image::transparent();
    match kind {
        "pickaxe_head" => {
            for x in 2..14 {
                let y = 3 + (x - 7i32).pow(2) / 16;
                image.plot(x, y, IRON);
                image.plot(x, y + 1, scale(IRON, 0.8));
            }
            rect(&mut image, [7, 4, 9, 7], scale(IRON, 0.7));
        }
        "axe_head" => {
            for y in 3..11 {
                for x in 3..11 {
                    if x + y > 8 && x < 10 - y / 4 {
                        image.plot(x, y, IRON);
                    }
                }
            }
            rect(&mut image, [9, 4, 11, 8], scale(IRON, 0.6));
        }
        _ => {
            for k in 0..10 {
                image.plot(3 + k, 12 - k, IRON);
                image.plot(4 + k, 12 - k, scale(IRON, 1.2));
            }
        }
    }
    items::outlined(image)
}

fn rune(name: &str) -> Image {
    let mut image = Image::transparent();
    for y in 2..14 {
        for x in 3..13 {
            if (3..13).contains(&y) || (4..12).contains(&x) {
                image.set(x, y, [0.34, 0.39, 0.41]);
            }
        }
    }
    rect(&mut image, [4, 3, 5, 12], [0.52, 0.55, 0.51]);
    let color = match name {
        "rune_flame" => COPPER,
        "rune_heal" => [0.48, 0.69, 0.44],
        "rune_blink" => [0.66, 0.51, 0.73],
        "rune_reach" => [0.43, 0.66, 0.74],
        _ => [0.84, 0.70, 0.40],
    };
    let glyph: &[(usize, usize)] = match name {
        "rune_flame" => &[
            (8, 4),
            (7, 5),
            (7, 6),
            (9, 6),
            (6, 7),
            (9, 7),
            (6, 8),
            (9, 8),
            (7, 9),
            (8, 9),
        ],
        "rune_heal" => &[
            (7, 5),
            (8, 5),
            (7, 6),
            (8, 6),
            (5, 7),
            (6, 7),
            (7, 7),
            (8, 7),
            (9, 7),
            (10, 7),
            (7, 8),
            (8, 8),
            (7, 9),
            (8, 9),
        ],
        "rune_blink" => &[(6, 5), (7, 5), (8, 6), (9, 7), (8, 8), (7, 9), (6, 9)],
        "rune_reach" => &[
            (5, 9),
            (6, 8),
            (7, 7),
            (8, 6),
            (9, 5),
            (7, 5),
            (8, 5),
            (9, 6),
            (9, 7),
        ],
        _ => &[
            (7, 5),
            (8, 5),
            (6, 6),
            (9, 6),
            (5, 7),
            (10, 7),
            (6, 8),
            (9, 8),
            (7, 9),
            (8, 9),
        ],
    };
    for &(x, y) in glyph {
        image.set(x, y, color);
    }
    items::outlined(image)
}
