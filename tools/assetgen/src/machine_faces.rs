//! Panneaux, charnières et garnitures des machines aux deux résolutions natives.

type Rgb = [u8; 3];
const STEEL: Rgb = [100, 127, 128];
const INK: Rgb = [31, 40, 42];
const COPPER: Rgb = [171, 110, 68];

pub(crate) const NAMES: &[&str] = &[
    "furnace_front",
    "furnace_top",
    "furnace_side_panel",
    "furnace_back_panel",
    "machine_side_panel",
    "machine_back_panel",
    "bench_side_panel",
    "bench_back_panel",
    "chest_inside",
    "chest_top",
    "chest_side",
    "chest_side_panel",
    "chest_back_panel",
    "copper_chest_side_panel",
    "copper_chest_back_panel",
    "copper_chest_top",
    "copper_chest_side",
    "iron_chest_side_panel",
    "iron_chest_back_panel",
    "iron_chest_top",
    "iron_chest_side",
    "gold_chest_side_panel",
    "gold_chest_back_panel",
    "gold_chest_top",
    "gold_chest_side",
    "sapphire_chest_side_panel",
    "sapphire_chest_back_panel",
    "sapphire_chest_top",
    "sapphire_chest_side",
    "vault_side_panel",
    "vault_back_panel",
];

pub(crate) const FORGE_FACES: &[&str] = &[
    "alloy_furnace_top",
    "alloy_furnace_side",
    "alloy_furnace_side_panel",
    "alloy_furnace_back_panel",
];

fn shade(color: Rgb, factor: f32) -> Rgb {
    color.map(|value| (f32::from(value) * factor).clamp(0.0, 255.0).round() as u8)
}

struct Panel {
    size: usize,
    width: usize,
    pixels: Vec<[u8; 4]>,
}

impl Panel {
    fn new(size: usize, base: Rgb, seed: u64) -> Self {
        Self::with_width(size, size, base, seed)
    }

    fn with_width(size: usize, width: usize, base: Rgb, seed: u64) -> Self {
        let pixels = (0..size * width)
            .map(|i| {
                let (x, y) = (i % width, i / width);
                let scale = size / 16;
                let fine = crate::textures::texel_hash(x, y, seed) % 17;
                let grain = crate::textures::texel_hash(x / (3 * scale), y / scale, seed + 1) % 11;
                let [r, g, b] = shade(base, 0.88 + fine as f32 * 0.007 + grain as f32 * 0.012);
                [r, g, b, 255]
            })
            .collect();
        Self {
            size,
            width,
            pixels,
        }
    }

    fn rect(&mut self, [x, y, w, h]: [usize; 4], [r, g, b]: Rgb) {
        let s = self.size / 16;
        for row in y * s..(y + h) * s {
            let start = row * self.width + x * s;
            self.pixels[start..start + w * s].fill([r, g, b, 255]);
        }
    }

    fn plate(&mut self, [x, y, w, h]: [usize; 4], color: Rgb) {
        self.rect([x, y, w, h], shade(color, 0.56));
        self.rect([x + 1, y + 1, w - 2, h - 2], color);
        self.rect([x + 1, y + 1, w - 2, 1], shade(color, 1.27));
    }

    fn stud(&mut self, x: usize, y: usize, color: Rgb) {
        self.rect([x, y, 1, 1], color);
        if self.size == 32 {
            let [r, g, b] = shade(color, 1.42);
            self.pixels[x * 2 + y * 2 * self.width] = [r, g, b, 255];
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Trim {
    WoodTop,
    WoodSide,
    MetalTop,
    MetalSide,
}

impl Trim {
    pub(crate) fn contains(self, index: usize, size: usize) -> bool {
        let scale = size / 16;
        let (x, y) = (index % size / scale, index / size / scale);
        match self {
            Self::WoodTop | Self::MetalTop => [0, 15].contains(&x) || [0, 15].contains(&y),
            Self::WoodSide => y == 3,
            Self::MetalSide => {
                [2, 13].contains(&y) || ([1, 14].contains(&x) && (2..14).contains(&y))
            }
        }
    }
}

/// Les panneaux spécialisés se placent dans un même cadre fini, aux hauteurs du modèle.
pub(crate) fn join_trim(name: &str, pixels: &mut [[u8; 4]], size: usize) -> Option<Trim> {
    use Trim::*;
    let trim = match name {
        "workbench_top" => WoodTop,
        "workbench_side" | "bench_side_panel" | "bench_back_panel" => WoodSide,
        "machine_frame" => MetalTop,
        "battery"
        | "pump"
        | "filler"
        | "motor"
        | "tank"
        | "fan"
        | "fan_on"
        | "assembler"
        | "machine_side_panel"
        | "machine_back_panel"
        | "storage_controller_side"
        | "cart_loader_side"
        | "cart_unloader_side"
        | "sorter_side"
        | "harvester_side"
        | "drill_side" => MetalSide,
        _ => return None,
    };
    let rgb = match trim {
        WoodTop | WoodSide => [112, 75, 47, 255],
        MetalTop | MetalSide => [69, 87, 88, 255],
    };
    let color = crate::textures::finish_pixel(rgb);
    for (index, pixel) in pixels.iter_mut().enumerate() {
        if trim.contains(index, size) {
            *pixel = color;
        }
    }
    Some(trim)
}

fn chest_palette(name: &str) -> (Rgb, Rgb) {
    if name.starts_with("copper") {
        ([157, 93, 61], [202, 134, 85])
    } else if name.starts_with("iron") {
        ([112, 140, 142], [167, 188, 183])
    } else if name.starts_with("gold") {
        ([175, 131, 57], [214, 170, 83])
    } else if name.starts_with("sapphire") {
        ([54, 91, 153], [112, 158, 193])
    } else if name.starts_with("vault") {
        ([79, 94, 99], [148, 145, 124])
    } else {
        ([135, 91, 56], COPPER)
    }
}

fn chest(name: &str, size: usize) -> Panel {
    let (base, strap) = chest_palette(name);
    let vault = name.starts_with("vault");
    let metal = vault || !name.starts_with("chest_");
    let border = shade(base, 0.72);
    let joint = shade(base, 0.47);
    let seam = if vault { 2 } else { 6 };
    let mut p = Panel::new(size, base, 804);
    if name.ends_with("top") {
        let inset = if vault { 0 } else { 2 };
        for edge in [inset, 15 - inset] {
            p.rect([edge, inset, 1, 16 - 2 * inset], border);
            p.rect([inset, edge, 16 - 2 * inset, 1], border);
        }
        for x in [4, 11] {
            p.rect([x, 0, 1, 16], shade(strap, 0.86));
            for y in [3, 12] {
                p.stud(x, y, shade(strap, 1.27));
            }
        }
        return p;
    }
    // Les plinthes et la ligne d'ouverture sont communes aux quatre faces verticales.
    for x in [0, 1, 14, 15] {
        p.rect([x, 0, 1, 16], border);
    }
    for y in [9, 12] {
        p.rect([0, y, 16, 1], shade(base, if metal { 0.86 } else { 0.76 }));
    }
    p.rect([0, if vault { 0 } else { 2 }, 16, 1], border);
    if !name.ends_with("side_panel") {
        for x in [4, 11] {
            p.rect([x, 0, 1, 14], shade(strap, 0.86));
            p.stud(x, 11, shade(strap, 1.27));
        }
    }
    p.rect([0, seam - 1, 16, 1], shade(base, 1.2));
    p.rect([0, seam, 16, 1], joint);
    p.rect([0, 14, 16, 2], border);
    if name.ends_with("back_panel") {
        for x in [4, 11] {
            p.rect([x - 1, seam.saturating_sub(1), 3, 3], strap);
            p.rect([x, seam, 1, 1], joint);
            p.stud(x, seam + 1, shade(strap, 1.25));
        }
    } else if name.ends_with("side") {
        if vault {
            p.plate([5, 5, 6, 6], shade(strap, 0.72));
            p.rect([7, 4, 2, 8], strap);
            p.rect([4, 7, 8, 2], strap);
            p.rect([7, 7, 2, 2], INK);
        } else {
            p.plate([6, 4, 4, 4], strap);
            p.rect([7, 6, 2, 1], INK);
        }
    }
    p
}

const CHEST_FAMILIES: [&str; 5] = [
    "chest",
    "copper_chest",
    "iron_chest",
    "gold_chest",
    "sapphire_chest",
];
const CHEST_HALVES: [&str; 2] = ["right", "left"];
const CHEST_FACES: [&str; 3] = ["top", "front", "back"];

pub(crate) fn paired_chest_names() -> impl Iterator<Item = String> {
    CHEST_FAMILIES
        .into_iter()
        .flat_map(|family| {
            CHEST_HALVES.into_iter().flat_map(move |half| {
                CHEST_FACES
                    .into_iter()
                    .map(move |face| format!("{family}_double_{half}_{face}"))
            })
        })
        .chain(
            CHEST_HALVES
                .into_iter()
                .map(|half| format!("chest_double_{half}_inside")),
        )
}

fn paired_chest(name: &str, size: usize) -> Option<Vec<[u8; 4]>> {
    let (family, suffix) = name.split_once("_double_")?;
    let (half, face) = suffix.split_once('_')?;
    if !CHEST_FAMILIES.contains(&family) || !CHEST_HALVES.contains(&half) {
        return None;
    }
    let (base, strap) = chest_palette(family);
    let border = shade(base, 0.72);
    let joint = shade(base, 0.47);
    let band = shade(strap, 0.86);
    let outer_rails = [5, 26];
    let mut p = Panel::with_width(size, size * 2, base, 804);
    match face {
        "top" => {
            for x in [2, 29] {
                p.rect([x, 2, 1, 12], border);
            }
            for y in [2, 13] {
                p.rect([2, y, 28, 1], border);
            }
            p.rect([3, 8, 26, 1], shade(base, 0.83));
            for x in outer_rails {
                p.rect([x, 0, 1, 16], band);
                for y in [3, 12] {
                    p.stud(x, y, shade(strap, 1.27));
                }
            }
        }
        "front" | "back" => {
            for x in [0, 1, 30, 31] {
                p.rect([x, 0, 1, 16], border);
            }
            for y in [9, 12] {
                p.rect(
                    [0, y, 32, 1],
                    shade(base, if family == "chest" { 0.76 } else { 0.86 }),
                );
            }
            p.rect([0, 2, 32, 1], border);
            for x in outer_rails {
                p.rect([x, 0, 1, 14], band);
                p.stud(x, 11, shade(strap, 1.27));
            }
            p.rect([0, 5, 32, 1], shade(base, 1.2));
            p.rect([0, 6, 32, 1], joint);
            p.rect([0, 14, 32, 2], border);
            if face == "front" {
                // La serrure couvre les deux demi-boîtes qui touchent le milieu.
                p.plate([14, 4, 4, 4], strap);
                p.rect([15, 6, 2, 1], INK);
            } else {
                for x in outer_rails {
                    p.rect([x - 1, 5, 3, 3], strap);
                    p.rect([x, 6, 1, 1], joint);
                    p.stud(x, 7, shade(strap, 1.25));
                }
            }
        }
        "inside" if family == "chest" => {
            p = Panel::with_width(size, size * 2, [65, 48, 37], 889);
            for y in [4, 8, 12] {
                p.rect([0, y, 32, 1], [51, 38, 29]);
            }
        }
        _ => return None,
    }
    // En coordonnées du modèle, la moitié « droite » va de -X à la jointure.
    let offset = if half == "right" { 0 } else { size };
    let mut pixels = Vec::with_capacity(size * size);
    for row in p.pixels.chunks_exact_mut(2 * size) {
        row[size] = row[size - 1];
        pixels.extend_from_slice(&row[offset..offset + size]);
    }
    Some(pixels)
}

fn furnace(name: &str, size: usize) -> Panel {
    let alloy = name.starts_with("alloy_");
    let cap = [76, 88, 91];
    let rim = shade(cap, 0.80);
    let top = name.ends_with("top");
    let mut p = Panel::new(size, if top { cap } else { [110, 119, 119] }, 898);
    if top {
        for edge in [0, 1, 14, 15] {
            p.rect([edge, 0, 1, 16], rim);
            p.rect([0, edge, 16, 1], rim);
        }
        p.plate([3, 3, 10, 10], cap);
        for y in [6, 8, 10] {
            p.rect([5, y, 6, 1], shade(cap, 0.55));
        }
        if alloy {
            p.rect([6, 6, 4, 4], shade(cap, 0.62));
        }
    } else {
        let cap_end = if alloy { 3 } else { 2 };
        for (x, y, w) in [(2, 6, 7), (10, 8, 4), (2, 11, 7), (7, 13, 7)] {
            p.rect([x, y, w, 1], [82, 91, 93]);
            p.rect([x + 1, y + 1, w - 1, 1], [124, 133, 133]);
        }
        for x in [0, 1, 14, 15] {
            p.rect([x, cap_end, 1, 16 - cap_end], [91, 101, 104]);
        }
        p.rect([0, 0, 16, cap_end], rim);
        p.rect([0, cap_end, 16, 1], shade(cap, 0.62));
        p.rect([0, 14, 16, 2], shade(rim, 0.88));
        if name.ends_with("front") || name == "alloy_furnace_side" {
            p.plate([3, 3, 10, 4], cap);
            for x in [5, 8, 11] {
                p.rect([x, 5, 1, 2], INK);
            }
            p.rect([3, 7, 10, 1], COPPER);
            p.rect([4, 8, 8, 6], [25, 29, 32]);
            if alloy {
                p.rect([5, 11, 6, 2], [137, 63, 36]);
            }
        } else if name.ends_with("back_panel") {
            p.plate([5, 4, 6, 9], cap);
            for y in [6, 8, 10] {
                p.rect([6, y, 4, 1], INK);
            }
        }
    }
    for (x, y) in [(3, 3), (12, 3), (3, 12), (12, 12)] {
        p.stud(x, y, COPPER);
    }
    p
}

pub(crate) fn draw(name: &str, size: usize) -> Option<Vec<[u8; 4]>> {
    if name.contains("_double_") {
        return paired_chest(name, size);
    }
    if !NAMES.contains(&name)
        && !FORGE_FACES.contains(&name)
        && !matches!(name, "vault_side" | "vault_top")
    {
        return None;
    }
    if name == "chest_inside" {
        let mut p = Panel::new(size, [65, 48, 37], 889);
        for y in [4, 8, 12] {
            p.rect([1, y, 14, 1], [51, 38, 29]);
        }
        return Some(p.pixels);
    }
    if name.contains("chest_") || name.starts_with("vault_") {
        return Some(chest(name, size).pixels);
    }
    if name.contains("furnace_") {
        return Some(furnace(name, size).pixels);
    }
    let back = name.ends_with("back_panel");
    let mut p = if name.starts_with("bench_") {
        let mut p = Panel::new(size, [133, 93, 59], 894);
        if back {
            p.rect([2, 7, 12, 2], [90, 61, 39]);
        } else {
            for x in [3, 12] {
                p.rect([x, 5, 1, 8], [161, 116, 71]);
            }
        }
        p
    } else {
        let mut p = Panel::new(size, STEEL, 897);
        if back {
            p.plate([4, 4, 8, 9], shade(STEEL, 0.69));
            for y in [6, 8, 10] {
                p.rect([5, y, 6, 1], INK);
            }
            p.stud(8, 12, COPPER);
        } else {
            p.plate([3, 5, 10, 7], shade(STEEL, 0.89));
            for x in [5, 7, 9, 11] {
                p.rect([x, 7, 1, 3], shade(STEEL, 0.63));
            }
        }
        p
    };
    for (x, y) in [(3, 4), (12, 4), (3, 12), (12, 12)] {
        p.stud(x, y, COPPER);
    }
    Some(p.pixels)
}
