//! Assemblages, rivets et surfaces usinées ; les motifs respectent les UV des boîtes.
use super::draw::{COPPER, Canvas, INK, IRON, SLATE, WOOD, mix, noise, seed, shade};
use super::{terrain, wood};

fn metal(base: [u8; 3], name: &str) -> Canvas {
    let mut c = Canvas::new(base);
    for y in 0..32 {
        for x in 0..32 {
            let n = noise(x as f32 / 31.0, y as f32 / 31.0, 8, seed(name));
            c.put(
                x,
                y,
                shade(base, 0.95 + n * 0.1 + if y % 7 == 0 { 0.025 } else { 0.0 }),
            );
        }
    }
    c
}

fn chest(name: &str) -> Canvas {
    let vault = name.starts_with("vault");
    let base = if vault { SLATE } else { WOOD };
    let band = if name.starts_with("iron") {
        IRON
    } else if name.starts_with("gold") {
        [191, 153, 64]
    } else if name.starts_with("sapphire") {
        [84, 146, 179]
    } else {
        COPPER
    };
    let mut c = if vault {
        metal(base, name)
    } else {
        wood::planks(base, 721)
    };
    for x in [4, 24] {
        c.panel([x, 2, 4, 28], shade(band, 0.85));
        for y in [5, 25] {
            c.bolt(x + 1, y, band);
        }
    }
    if name.ends_with("side") {
        c.rect([2, 10, 28, 2], INK);
        c.line((2, 12), (29, 12), shade(base, 1.3));
        c.panel([12, 8, 8, 8], band);
        c.rect([15, 11, 2, 3], INK);
        c.put(15, 10, shade(band, 1.55));
        if vault {
            c.ellipse((16.0, 19.0), (6.0, 6.0), INK);
            c.ellipse((16.0, 19.0), (4.7, 4.7), band);
            for (a, b) in [((12, 19), (20, 19)), ((16, 15), (16, 23))] {
                c.line(a, b, INK);
            }
            c.bolt(16, 19, IRON);
        }
    } else {
        for y in [3, 28] {
            c.line((8, y), (23, y), shade(band, 0.7));
        }
    }
    c
}

fn furnace(name: &str) -> Canvas {
    let mut c = terrain::rock("cobblestone");
    if name.ends_with("front") || name.ends_with("side") {
        c.panel([5, 1, 22, 12], shade(SLATE, 0.65));
        for x in [9, 14, 19, 24] {
            c.rect([x, 4, 2, 7], INK);
            c.line((x + 1, 4), (x + 1, 9), shade(IRON, 0.65));
        }
        c.panel([6, 14, 20, 16], shade(IRON, 0.65));
        c.rect([7, 15, 18, 14], [77, 58, 45]);
        c.line((7, 15), (24, 15), COPPER);
        // Même fenêtre UV que le classique : 4..11 / 8..13 en seizièmes.
        c.rect([8, 16, 16, 12], [27, 31, 35]);
        c.line((8, 27), (23, 27), [51, 46, 40]);
        for (x, y) in [(4, 14), (27, 14), (4, 29), (27, 29)] {
            c.bolt(x, y, IRON);
        }
    } else {
        c.panel([3, 3, 26, 26], SLATE);
        for y in [9, 15, 21] {
            c.line((7, y), (24, y), INK);
            c.line((7, y + 1), (24, y + 1), shade(IRON, 0.9));
        }
        for (x, y) in [(4, 4), (26, 4), (4, 26), (26, 26)] {
            c.bolt(x, y, COPPER);
        }
    }
    c
}

fn bed(name: &str) -> Canvas {
    let mut c = wood::planks(WOOD, 871);
    let sage = [78, 125, 110];
    let linen = [218, 210, 177];
    if name == "bed_long_side" {
        c.rect([0, 10, 32, 4], shade(linen, 0.86));
        c.rect([0, 14, 32, 8], sage);
        c.line((0, 14), (31, 14), shade(sage, 1.26));
        c.line((0, 21), (31, 21), shade(sage, 0.73));
        c.line((0, 22), (31, 22), shade(WOOD, 1.30));
        for x in (2..30).step_by(3) {
            c.put(x, 19, shade(sage, 1.23));
        }
        return c;
    }
    if matches!(name, "bed_head_top" | "bed_foot_top") {
        let head = name == "bed_head_top";
        let y0 = if head { 4 } else { 0 };
        let y1 = if head { 32 } else { 30 };
        c.rect([4, y0, 24, y1 - y0], sage);
        for y in y0..y1 {
            for x in 4..28 {
                let shade_factor = if !(6..26).contains(&x) {
                    0.73
                } else if x % 3 == 0 && y % 4 == 2 {
                    1.12
                } else {
                    1.0
                };
                c.put(x, y, shade(sage, shade_factor));
            }
        }
        for x in [5, 26] {
            for y in (y0 + 1..y1).step_by(4) {
                c.put(x, y, shade(linen, 0.71));
            }
        }
        if head {
            c.panel([6, 6, 20, 12], linen);
            c.line((8, 8), (23, 8), shade(linen, 1.13));
            c.line((6, 20), (25, 20), shade(sage, 1.39));
            c.line((6, 21), (25, 21), shade(sage, 0.74));
        } else {
            c.rect([6, 19, 20, 4], shade(sage, 1.24));
            for x in (8..25).step_by(3) {
                c.put(x, 26, shade(linen, 0.80));
            }
        }
        return c;
    }
    if name.ends_with("top") {
        c.rect([4, 2, 24, 28], sage);
        for y in 4..30 {
            for x in 5..27 {
                let fold = if !(7..=24).contains(&x) {
                    0.75
                } else if (x + y / 2) % 12 == 0 {
                    1.1
                } else {
                    1.0
                };
                c.put(x, y, shade(sage, fold));
            }
        }
        for x in [6, 25] {
            for y in (12..29).step_by(3) {
                c.rect([x, y, 1, 2], linen);
            }
        }
        c.panel([6, 4, 20, 7], linen);
        c.line((8, 5), (23, 5), shade(linen, 1.1));
        for y in [13, 25] {
            c.line((7, y), (24, y), shade(sage, 0.76));
            c.line((7, y + 1), (24, y + 1), shade(sage, 1.25));
        }
    } else {
        c.rect([4, 16, 24, 6], sage);
        c.line((4, 16), (27, 16), shade(sage, 1.3));
        for x in (6..27).step_by(3) {
            c.put(x, 20, linen);
        }
        c.rect([6, 14, 20, 2], linen);
        c.line((2, 22), (29, 22), shade(WOOD, 1.4));
        for x in [4, 26] {
            c.bolt(x, 24, COPPER);
        }
    }
    c
}

fn industrial(name: &str) -> Canvas {
    let mut c = metal(SLATE, name);
    c.panel([2, 2, 28, 28], SLATE);
    for (x, y) in [(4, 4), (26, 4), (4, 26), (26, 26)] {
        c.bolt(x, y, COPPER);
    }
    let h = seed(name);
    if name.contains("solar") {
        c.rect([5, 5, 22, 22], [45, 67, 94]);
        for i in [5, 12, 19, 26] {
            c.line((i, 5), (i, 26), [110, 144, 156]);
            c.line((5, i), (26, i), [110, 144, 156]);
        }
        c.line((6, 6), (10, 10), [146, 169, 180]);
    } else if name.contains("motor")
        || name.contains("pump")
        || name.contains("fan")
        || name.contains("spawner")
    {
        c.ellipse((16.0, 16.0), (10.0, 10.0), INK);
        c.ellipse((16.0, 16.0), (8.5, 8.5), shade(IRON, 0.7));
        c.ellipse((16.0, 16.0), (6.8, 6.8), INK);
        for i in 0..4 {
            let a = i as f32 * std::f32::consts::FRAC_PI_2;
            c.line(
                (16, 16),
                (16 + (a.cos() * 7.0) as i32, 16 + (a.sin() * 7.0) as i32),
                COPPER,
            );
        }
        c.bolt(16, 16, IRON);
    } else {
        c.panel([7, 6, 18, 10], shade(COPPER, 0.68));
        c.rect([9, 8, 14, 5], [34, 65, 61]);
        for i in 0..4 {
            let x = 10 + i * 3;
            c.rect([x, 12 - ((h >> (i * 2)) % 4) as i32, 2, 2], [117, 188, 150]);
        }
        for y in [20, 23] {
            c.line((8, y), (23, y), INK);
            c.line((8, y + 1), (23, y + 1), shade(IRON, 0.75));
        }
    }
    c
}

pub(super) fn draw(name: &str) -> Option<Canvas> {
    Some(if name.contains("chest_") || name.starts_with("vault_") {
        chest(name)
    } else if name.starts_with("furnace_") || name.starts_with("alloy_furnace_") {
        furnace(name)
    } else if name.starts_with("bed_") {
        bed(name)
    } else if name.starts_with("workbench_") {
        let mut c = wood::planks(WOOD, 746);
        if name.ends_with("top") {
            c.panel([4, 4, 24, 24], [165, 127, 79]);
            for x in (7..26).step_by(2) {
                c.rect([x, 5, 1, if x % 4 == 3 { 3 } else { 2 }], INK);
            }
            for (a, b) in [
                ((8, 21), (19, 10)),
                ((12, 24), (24, 12)),
                ((15, 18), (23, 20)),
            ] {
                c.line(a, b, shade(WOOD, 0.65));
            }
            for (x, y) in [(3, 3), (28, 3), (3, 28), (28, 28)] {
                c.bolt(x, y, COPPER);
            }
        } else {
            c.panel([7, 6, 18, 6], shade(WOOD, 0.8));
            c.rect([13, 8, 6, 2], INK);
            for x in [4, 26] {
                c.bolt(x, 20, COPPER);
            }
        }
        c
    } else if name.ends_with("_block") && !name.contains("coal") {
        let base = if name.contains("copper") {
            COPPER
        } else if name.contains("gold") {
            [201, 167, 79]
        } else if name.contains("sapphire") {
            [63, 126, 174]
        } else {
            IRON
        };
        let mut c = metal(base, name);
        c.panel([0, 0, 32, 32], base);
        for (x, y) in [(3, 3), (28, 3), (3, 28), (28, 28)] {
            c.bolt(x, y, shade(base, 0.85));
        }
        c.line((8, 8), (17, 7), shade(base, 1.15));
        c
    } else if [
        "battery",
        "solar_panel",
        "pump",
        "motor",
        "fan",
        "fan_on",
        "assembler",
        "machine_frame",
        "spawner",
        "boss_spawner",
        "breaker",
        "press",
        "drill_top",
        "drill_side",
        "sorter_top",
        "sorter_side",
        "harvester_top",
        "harvester_side",
        "storage_controller_top",
        "storage_controller_side",
    ]
    .contains(&name)
    {
        industrial(name)
    } else if name.contains("anvil_") {
        let mut c = metal(shade(IRON, 0.65), name);
        for y in [2, 10, 24] {
            c.line((0, y), (31, y), shade(IRON, 0.95));
            c.line((0, y + 1), (31, y + 1), shade(IRON, 0.4));
        }
        for i in 0..7 {
            let x = 5 + i * 3;
            c.line((x, 7 + i % 3), (x + 1, 8 + i % 3), mix(IRON, INK, 0.35));
        }
        c
    } else {
        return None;
    })
}
