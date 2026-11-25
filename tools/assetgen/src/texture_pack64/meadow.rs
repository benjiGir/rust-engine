//! Dessins botaniques natifs. Une toile de deux blocs garde la tige continue à la couture.
use super::{Pixel, SIDE, color, shade, terrain::mix};
use crate::Rng;
use std::f32::consts::{PI, TAU};

mod cultivated;

pub(crate) fn draw_cultivated(name: &str, side: usize) -> Option<Vec<Pixel>> {
    cultivated::draw(name, side)
}

type Point = (f32, f32);
type Rgb = [u8; 3];
const STEM: Rgb = [74, 107, 48];
const LEAF: Rgb = [65, 108, 43];
const SHOOT: Rgb = [116, 143, 58];
const BARK: Rgb = [100, 74, 53];

struct Plant {
    resolution: usize,
    height: usize,
    pixels: Vec<Pixel>,
}

fn between(a: Point, b: Point, t: f32) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

impl Plant {
    fn new(height: usize) -> Self {
        Self::with_resolution(height, SIDE)
    }

    fn with_resolution(height: usize, resolution: usize) -> Self {
        Self {
            resolution,
            height,
            pixels: vec![[0; 4]; resolution * (height * resolution / SIDE)],
        }
    }

    fn paint(
        &mut self,
        min: Point,
        max: Point,
        mut sample: impl FnMut(f32, f32, f32) -> Option<Rgb>,
    ) {
        let side = self.resolution;
        let scale = side as f32 / SIDE as f32;
        let height = self.pixels.len() / side;
        for y in (min.1 * scale).floor().max(0.0) as usize
            ..=((max.1 * scale).ceil() as usize).min(height - 1)
        {
            for x in (min.0 * scale).floor().max(0.0) as usize
                ..=((max.0 * scale).ceil() as usize).min(side - 1)
            {
                let noise = (crate::textures::texel_hash(x, y, 61441) & 31) as f32 / 31.0;
                if let Some(rgb) = sample((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale, noise)
                {
                    self.pixels[y * side + x] = color(rgb);
                }
            }
        }
    }

    fn ellipse(&mut self, center: Point, radii: Point, angle: f32, tint: Rgb) {
        let (sin, cos) = angle.sin_cos();
        let r = radii.0.max(radii.1);
        self.paint(
            (center.0 - r, center.1 - r),
            (center.0 + r, center.1 + r),
            |x, y, grain| {
                let (x, y) = (x - center.0, y - center.1);
                let u = (x * cos + y * sin) / radii.0;
                let v = (-x * sin + y * cos) / radii.1;
                let roundness = 1.0 - u * u - v * v;
                (roundness >= 0.0).then(|| {
                    shade(
                        tint,
                        0.75 + 0.26 * roundness - 0.14 * v + 0.14 * u + 0.10 * grain,
                    )
                })
            },
        );
    }

    // Feuille courbe, au bord parfois dentelé. Les nervures secondaires rejoignent la médiane.
    fn blade(&mut self, root: Point, tip: Point, width: f32, bend: f32, tint: Rgb, serration: f32) {
        let delta = (tip.0 - root.0, tip.1 - root.1);
        let length = delta.0.hypot(delta.1);
        let direction = (delta.0 / length, delta.1 / length);
        let margin = width + bend.abs() + 1.0;
        self.paint(
            (root.0.min(tip.0) - margin, root.1.min(tip.1) - margin),
            (root.0.max(tip.0) + margin, root.1.max(tip.1) + margin),
            |x, y, grain| {
                let (x, y) = (x - root.0, y - root.1);
                let t = (x * direction.0 + y * direction.1) / length;
                if !(0.0..1.0).contains(&t) {
                    return None;
                }
                let arc = (PI * t).sin();
                let across = -x * direction.1 + y * direction.0 - bend * arc;
                let teeth = 1.0 - serration * (t * 7.0 + across.signum() * 0.19).fract().abs();
                let radius = width * (0.22 * (1.0 - t) + 0.87 * arc) * teeth;
                if across.abs() > radius {
                    return None;
                }
                let midrib = (1.0 - across.abs() / 0.85).max(0.0);
                let veins = if width >= 3.0 && (t * 8.0 - across.abs() * 0.18).fract().abs() < 0.19
                {
                    0.13
                } else {
                    0.0
                };
                let light = 0.78 + 0.23 * t + 0.14 * grain + 0.10 * across.signum() + veins;
                Some(mix(shade(tint, light), [158, 171, 94], midrib * 0.27))
            },
        );
    }

    // Toutes les feuilles et fleurs d'une tige utilisent ces mêmes points d'attache.
    fn stem(
        &mut self,
        root: Point,
        tip: Point,
        radius: f32,
        bend: f32,
        tint: Rgb,
    ) -> impl Fn(f32) -> Point + use<> {
        let at = move |t: f32| {
            let (x, y) = between(root, tip, t);
            (x + bend * (PI * t).sin(), y)
        };
        let steps = ((tip.0 - root.0).hypot(tip.1 - root.1) * 1.4).ceil() as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let r = radius * (1.0 - 0.46 * t);
            self.ellipse(at(t), (r.max(0.68), r.max(0.68)), 0.0, tint);
        }
        at
    }

    fn flower(&mut self, at: Point, count: usize, span: f32, broad: f32, tint: Rgb, disk: Rgb) {
        for i in 0..count {
            let a = TAU * i as f32 / count as f32 + 0.23;
            let (sin, cos) = a.sin_cos();
            let r = span * (0.94 + 0.12 * (i as f32 * 2.4).sin());
            let t = shade(tint, 0.92 + (1.0 - sin) * 0.10);
            self.ellipse(
                (at.0 + cos * r * 0.53, at.1 + sin * r * 0.43),
                (r * 0.61, broad),
                a,
                t,
            );
        }
        let heart = span * if count >= 12 { 0.39 } else { 0.30 };
        self.ellipse(at, (heart, heart * 0.78), 0.0, disk);
        for i in 0..(heart * 4.0) as usize {
            let a = i as f32 * 2.39996;
            let r = (i as f32 / (heart * 4.0)).sqrt() * heart;
            self.ellipse(
                (at.0 + r * a.cos(), at.1 + 0.73 * r * a.sin()),
                (0.68, 0.61),
                a,
                shade(disk, 1.27 + 0.17 * (i % 3) as f32),
            );
        }
    }
}

fn grass(p: &mut Plant, tall: bool) {
    let mut rng = Rng::new(1507);
    let base = p.height as f32 + 1.0;
    for i in 0..if tall { 18 } else { 13 } {
        let root = (22.0 + rng.next_f32() * 22.0, base);
        let tip = (
            4.0 + rng.next_f32() * 56.0,
            base * (0.07 + rng.next_f32() * 0.65),
        );
        let tint = mix([44, 86, 39], [135, 154, 66], rng.next_f32());
        p.blade(
            root,
            tip,
            1.2 + rng.next_f32() * 1.23,
            rng.signed() * 8.0,
            tint,
            0.08,
        );
        if i % 5 == 0 {
            let at = p.stem(root, tip, 1.05, 1.0, SHOOT);
            for j in 0..6 {
                let pos = at(0.84 + j as f32 * 0.026);
                p.ellipse(pos, (1.2, 2.1), 0.34, [155, 147, 87]);
            }
        }
    }
}

fn fern(p: &mut Plant) {
    for (i, (x, y, bend)) in [
        (6., 26., -8.),
        (15., 11., -7.),
        (29., 5., -4.),
        (43., 13., 5.),
        (57., 22., 9.),
    ]
    .into_iter()
    .enumerate()
    {
        let path = p.stem((32.0, 64.5), (x, y), 1.12, bend, [114, 130, 67]);
        for j in 0..10 {
            let t = 0.20 + j as f32 * 0.079;
            let a = path(t);
            let b = path(t + 0.02);
            let d = (b.0 - a.0, b.1 - a.1);
            let scale = (1.0 - t) * 10.0 / d.0.hypot(d.1);
            for s in [-1.0, 1.0] {
                let tip = (
                    a.0 + s * d.1 * scale + d.0 * 1.6,
                    a.1 - s * d.0 * scale + d.1 * 1.6,
                );
                let green = shade([76, 133, 63], 0.83 + 0.06 * i as f32 + 0.09 * t);
                p.blade(a, tip, 1.15 + 1.5 * (1.0 - t), s * 1.1, green, 0.30);
            }
        }
    }
}

fn wildflower(p: &mut Plant, name: &str) {
    let (n, tint, span, broad, disk) = match name {
        "poppy" => (5, [212, 49, 39], 10.1, 5.3, [53, 36, 40]),
        "cornflower" => (11, [83, 119, 202], 9.8, 1.9, [72, 63, 130]),
        "daisy" => (12, [242, 234, 208], 10.7, 1.7, [217, 154, 43]),
        "dandelion" => (22, [243, 200, 54], 9.0, 1.6, [220, 152, 29]),
        "marigold" => (14, [231, 144, 43], 10.4, 2.7, [188, 91, 22]),
        "tulip" => (0, [213, 64, 102], 8.0, 4.0, [119, 40, 69]),
        "allium" => (0, [153, 104, 186], 10.0, 2.0, [211, 163, 226]),
        _ => unreachable!(),
    };
    let stems = if matches!(name, "poppy" | "tulip" | "allium") {
        &[(20., 28., 0.68), (36., 14., 1.0)][..]
    } else {
        &[(15., 34., 0.68), (46., 25., 0.76), (29., 14., 1.0)][..]
    };
    for &(x, y, scale) in stems {
        let at = p.stem(
            (30.0 + (x - 30.0) * 0.13, 64.6),
            (x, y),
            1.45 * scale,
            (x - 30.0) * -0.17,
            STEM,
        );
        for (t, side) in [(0.20, -1.0), (0.34, 1.0), (0.52, -1.0)] {
            let a = at(t);
            p.blade(
                a,
                (a.0 + side * 12.0 * scale, a.1 - 9.0 * scale),
                3.25 * scale,
                side * 2.1,
                LEAF,
                if name == "tulip" { 0.0 } else { 0.22 },
            );
        }
        if name == "tulip" {
            for (dx, dy, a, light) in [
                (-3.4, -1.3, -1.94, 0.86),
                (3.1, -0.9, -1.18, 0.98),
                (0.0, -2.8, -1.57, 1.18),
            ] {
                p.ellipse(
                    (x + dx * scale, y + dy * scale),
                    (span * scale, broad * scale),
                    a,
                    shade(tint, light),
                );
            }
        } else if name == "allium" {
            for i in 0..23 {
                let a = i as f32 * 2.39996;
                let r = (i as f32 / 23.0).sqrt() * span * scale;
                let center = (x + a.cos() * r, y + a.sin() * r);
                p.flower(
                    center,
                    5,
                    2.0 * scale,
                    0.69,
                    shade(tint, 0.85 + 0.09 * (i % 5) as f32),
                    disk,
                );
            }
        } else {
            p.flower((x, y), n, span * scale, broad * scale, tint, disk);
        }
    }
}

fn heather(p: &mut Plant) {
    for (i, (x, y)) in [(12., 34.), (22., 20.), (32., 13.), (41., 23.), (51., 29.)]
        .into_iter()
        .enumerate()
    {
        let a = p.stem(
            (26.0 + i as f32 * 2.0, 65.0),
            (x, y),
            1.25,
            (32.0 - x) * 0.14,
            BARK,
        );
        for j in 0..6 {
            let t = 0.15 + j as f32 * 0.087;
            let (x, y) = a(t);
            let s = if j % 2 == 0 { -1.0 } else { 1.0 };
            let size = 5.0;
            p.blade((x, y), (x + s * size, y - size * 0.7), 1.8, s, LEAF, 0.0);
        }
        for j in 0..13 {
            let (x, y) = a(0.52 + j as f32 * 0.039);
            let s = if j % 2 == 0 { -1.0 } else { 1.0 };
            let tint = [183, 98, 143];
            p.ellipse(
                (x + s * 2.1, y),
                (1.9, 2.5),
                s * 0.45,
                shade(tint, 0.85 + 0.12 * (j % 3) as f32),
            );
            p.ellipse((x + s * 2.5, y - 1.0), (0.95, 0.82), 0.0, shade(tint, 1.36));
        }
    }
}

fn woody(p: &mut Plant, name: &str) {
    let needles = matches!(name, "spruce" | "fir" | "redwood");
    let tint = match name {
        "spruce" => [65, 111, 81],
        "fir" => [50, 99, 76],
        "redwood" => [74, 120, 62],
        "acacia" => [99, 129, 54],
        "olive" => [112, 132, 86],
        "maple" => [159, 100, 47],
        "birch" => [106, 143, 57],
        "jungle" => [48, 125, 66],
        _ => [77, 125, 53],
    };
    let bark = match name {
        "birch" => [164, 149, 123],
        "redwood" => [117, 67, 43],
        "acacia" => [122, 108, 80],
        _ => BARK,
    };
    let trunk = p.stem((31.0, 65.0), (32.0, 6.0), 2.05, -1.7, bark);
    for j in 0..6 {
        let height = j as f32 / 5.0;
        let t = match name {
            "acacia" => 0.47 + height * 0.43,
            "redwood" => 0.37 + height * 0.55,
            _ => 0.27 + height * 0.575,
        };
        let at = trunk(t);
        for sign in [-1.0, 1.0] {
            let (reach, rise) = match name {
                "spruce" => (24.0 - 16.0 * height, -3.5 * (1.0 - height)),
                "fir" => (23.0 - 14.0 * height, 4.0),
                "redwood" => (15.0 - 9.0 * height, 6.2),
                "acacia" => (12.0 + 10.0 * (PI * height).sin(), 7.0 - 5.0 * height),
                "olive" => (11.0 + 7.0 * (PI * height).sin(), 10.0),
                _ => (13.0 + (j as f32 * 1.8).sin() * 8.0, 8.5),
            };
            let tip = (at.0 + sign * reach, at.1 - rise);
            let branch = p.stem(at, tip, 1.23, 1.1 * sign, bark);
            if name == "dead_bush" {
                for offset in [0.48, 0.78] {
                    let pos = branch(offset);
                    let _ = p.stem(
                        pos,
                        (pos.0 + sign * 4.0, pos.1 - 8.0),
                        0.9,
                        -1.1,
                        shade(bark, 1.16),
                    );
                }
                continue;
            }
            for k in 0..if needles { 12 } else { 5 } {
                let pos = branch(0.24 + k as f32 * if needles { 0.067 } else { 0.17 });
                let dy = if k % 2 == 0 { -1.0 } else { 0.46 };
                if name == "cherry" {
                    p.flower(
                        (pos.0 + sign * 2.5, pos.1 + dy * 3.0),
                        5,
                        3.7,
                        1.5,
                        [227, 155, 176],
                        [174, 93, 130],
                    );
                } else {
                    let (size, width) = match name {
                        "spruce" => (5.5, 1.02),
                        "fir" | "redwood" => (4.2, 0.97),
                        "jungle" => (12.0, 2.8),
                        "olive" | "acacia" => (7.3, 1.45),
                        "maple" => (7.2, 3.15),
                        _ => (7.0, 2.55),
                    };
                    let leaf = shade(tint, 0.81 + 0.11 * (k % 4) as f32);
                    p.blade(
                        pos,
                        (pos.0 + sign * size, pos.1 + dy * size),
                        width,
                        sign,
                        leaf,
                        0.23,
                    );
                }
            }
        }
    }
}

fn crop(p: &mut Plant, name: &str, stage: usize) {
    let max_height = [15.0, 27.0, 41.0, 54.0][stage];
    let mut rng = Rng::new(233);
    for i in 0..[3, 5, 7, 8][stage] {
        let root = (24.0 + rng.next_f32() * 16.0, 65.0);
        let sign = if i % 2 == 0 { -1.0 } else { 1.0 };
        let height = max_height * (0.68 + rng.next_f32() * 0.30);
        let tip = (root.0 + sign * (3.0 + rng.next_f32() * 16.0), 64.0 - height);
        let tint = shade(LEAF, 0.83 + rng.next_f32() * 0.45);
        let reach = 6.0 + stage as f32 * 3.0;
        let tip = (tip.0 + sign * reach * 0.51, 64.0 - height * 0.70);
        match name {
            "carrot" => {
                let at = p.stem(root, tip, 1.05, sign * 2.0, STEM);
                for j in 0..6 {
                    let t = 0.35 + 0.11 * j as f32;
                    let a = at(t);
                    for dir in [-1.0, 1.0] {
                        p.blade(
                            a,
                            (a.0 + dir * reach * (1.0 - t), a.1 - 4.5),
                            1.6,
                            dir,
                            tint,
                            0.32,
                        );
                    }
                }
            }
            "potato" => {
                let at = p.stem(root, tip, 1.4, sign, STEM);
                for j in 0..4 {
                    let a = at(0.33 + j as f32 * 0.19);
                    let s = if j % 2 == 0 { 1.0 } else { -1.0 };
                    p.blade(
                        a,
                        (a.0 + reach * s, a.1 - 6.0),
                        2.1 + stage as f32 * 0.8,
                        s,
                        tint,
                        0.1,
                    );
                }
            }
            "cabbage" => p.blade(
                root,
                (tip.0, 63.0 - height * 0.48),
                3.5 + stage as f32 * 1.8,
                sign * 5.0,
                mix(tint, [89, 145, 122], 0.62),
                0.13,
            ),
            _ => {
                let at = p.stem(root, (tip.0, 64.0 - height * 0.37), 1.15, sign * -3.0, STEM);
                for j in 0..4 {
                    let a = at(0.37 + j as f32 * 0.19);
                    p.blade(
                        a,
                        (a.0 + sign * 8.1, a.1 - 8.5),
                        2.8 + stage as f32,
                        sign,
                        tint,
                        0.32,
                    );
                }
            }
        }
    }
    match (name, stage) {
        ("cabbage", 3) => {
            for (x, radius, tint) in [
                (29.0, 10.2, [85, 132, 106]),
                (35.0, 9.0, [113, 153, 117]),
                (31.5, 6.4, [141, 173, 128]),
            ] {
                p.ellipse((x, 53.0), (radius, radius * 0.90), (x - 31.5) * 0.21, tint);
            }
        }
        ("melon", 3) => {
            // Le fruit est posé sous les tiges, avec des rayures qui suivent sa courbure.
            p.paint((23.0, 46.0), (51.0, 63.0), |x, y, grain| {
                let u = (x - 37.0) / 13.0;
                let v = (y - 55.0) / 8.0;
                let roundness = 1.0 - u * u - v * v;
                let bands = ((u * (1.0 + v * 0.19) + 0.11 * v * v) * 17.0).sin();
                let tint = mix([40, 89, 48], [91, 133, 56], (bands * 0.5 + 0.5).powf(0.65));
                (roundness >= 0.0)
                    .then(|| shade(tint, 0.68 + 0.30 * roundness - 0.16 * v + 0.10 * grain))
            });
        }
        _ => {}
    }
}

fn berries(p: &mut Plant, blueberry: bool, stage: usize) {
    let mut rng = Rng::new(if blueberry { 4017 } else { 7439 });
    let (shade_leaf, light_leaf, fruit, calyx) = if blueberry {
        ([39, 75, 60], [115, 155, 106], [78, 105, 165], [48, 63, 111])
    } else {
        ([43, 80, 38], [126, 161, 75], [198, 48, 40], [112, 37, 42])
    };
    let mut twigs = Vec::new();
    let mut clusters = Vec::new();
    let canes: usize = if stage == 0 { 5 } else { 10 };
    for i in 0..canes {
        let spread = i as f32 / (canes - 1) as f32 * 2.0 - 1.0;
        let root = (31.5 + spread * 9.0 + rng.signed() * 2.0, 65.0);
        let tip = (
            32.0 + spread * 25.0,
            8.0 + spread.abs().powf(1.5) * 16.0 + rng.next_f32() * 4.0,
        );
        let cane = p.stem(
            root,
            tip,
            1.15 + rng.next_f32() * 0.75,
            spread * 5.5,
            shade(BARK, 0.86 + rng.next_f32() * 0.35),
        );
        for j in 0..if stage == 0 { 5 } else { 7 } {
            let at = cane(0.36 + j as f32 * 0.10);
            let direction = if (i + j).is_multiple_of(2) { -1.0 } else { 1.0 };
            let twig = (
                (at.0 + direction * (5.0 + rng.next_f32() * 4.0)).clamp(4.5, 59.5),
                at.1 - 2.0 - rng.next_f32() * 4.0,
            );
            twigs.push((at, twig, rng.next_f32()));
            if j >= 1 && (i + 2 * j).is_multiple_of(5) {
                clusters.push((twig.0, twig.1 + 5.0));
            }
        }
    }

    // Des feuilles sombres forment le cœur du buisson. Les pousses de face
    // reprennent les mêmes rameaux avec une lumière et une direction variées.
    for &(at, tip, vigor) in &twigs {
        p.blade(
            at,
            tip,
            3.4 + vigor * 3.0,
            1.2,
            mix(shade_leaf, light_leaf, vigor * 0.42),
            0.18,
        );
    }
    let serration = if blueberry { 0.07 } else { 0.25 };
    for &(_, (x, y), vigor) in &twigs {
        let tint = mix(shade_leaf, light_leaf, 0.28 + vigor * 0.60);
        for side in [-1.0, 1.0] {
            let tip = (
                (x + side * (5.4 + 3.1 * rng.next_f32())).clamp(1.0, 63.0),
                (y - 1.4 - 6.0 * rng.next_f32()).max(2.0),
            );
            let base = (x - side * 2.0, y + 1.5);
            p.blade(
                base,
                tip,
                2.0 + 1.9 * rng.next_f32(),
                side * 1.45,
                tint,
                serration,
            );
        }
    }

    if stage != 2 {
        return;
    }
    for (x, y) in clusters {
        for (dx, dy, size) in [(-2.2, 0.4, 2.15), (2.35, -0.85, 2.3), (0.35, 2.8, 2.55)] {
            let center = ((x + dx).clamp(2.5, 61.5), y + dy);
            p.ellipse(
                center,
                (size, size * 1.06),
                0.0,
                shade(fruit, 0.88 + 0.24 * rng.next_f32()),
            );
            p.ellipse(
                (center.0 - 0.76, center.1 - 0.73),
                (0.79, 0.70),
                0.0,
                shade(fruit, 1.36),
            );
            p.ellipse((center.0 + 0.49, center.1 + 0.82), (0.77, 0.59), 0.0, calyx);
        }
    }
}

fn mushroom(p: &mut Plant, name: &str) {
    let (cap, stem, spots) = match name {
        "red_mushroom" => ([180, 57, 41], [200, 180, 147], [230, 215, 180]),
        "glowcap" => ([69, 157, 175], [112, 157, 156], [144, 223, 224]),
        _ => ([141, 96, 60], [181, 152, 117], [183, 133, 88]),
    };
    for (x, y, size) in [(18.0, 41.0, 0.69), (38.0, 26.0, 1.0)] {
        let _ = p.stem((x + 0.8, 66.0), (x, y + 3.0), 3.5 * size, 2.4 * size, stem);
        p.ellipse(
            (x, y + 3.4 * size),
            (17.0 * size, 4.7 * size),
            0.0,
            shade(stem, 0.67),
        );
        for i in -6..=6 {
            let offset = i as f32 * 2.16 * size;
            p.ellipse(
                (x + offset, y + 4.6 * size),
                (0.62, 2.4 * size),
                -offset * 0.06,
                shade(stem, 0.85),
            );
        }
        p.ellipse((x, y - 2.4 * size), (17.4 * size, 9.8 * size), 0.0, cap);
        for (dx, dy, radius) in [
            (-11., 0., 1.45),
            (-5., -5., 1.6),
            (2., -8., 1.25),
            (6., -2., 2.),
            (0., 1., 0.95),
            (12., -1., 1.15),
        ] {
            p.ellipse(
                (x + dx * size, y + dy * size),
                (radius * size, radius * size * 0.79),
                0.0,
                spots,
            );
        }
    }
}

fn water_plant(p: &mut Plant, name: &str) {
    if name == "lily_pad" {
        p.paint((4., 4.), (60., 60.), |x, y, noise| {
            let (x, y) = (x - 32.0, y - 32.0);
            let angle = y.atan2(x);
            let distance = x.hypot(y);
            let edge = 26.0 + 1.55 * (angle * 5.0).sin() + 0.58 * (angle * 11.0).cos();
            if distance > edge || (angle.abs() < 0.21 && distance > 3.0) {
                return None;
            }
            let vein =
                (1.0 - ((angle + distance * 0.014) * 7.0).sin().abs() * distance * 0.26).max(0.0);
            let base = mix(
                [47, 87, 48],
                [103, 137, 67],
                (0.61 - y / 96.0 + noise * 0.19).clamp(0.0, 1.0),
            );
            Some(mix(
                base,
                [151, 161, 85],
                0.29 * vein + 0.22 * f32::from(edge - distance < 1.4),
            ))
        });
    } else if name == "seaweed" {
        for i in 0..8 {
            let x = 8.0 + (i * 29 % 47) as f32;
            let y = 5.0 + (i * 11 % 30) as f32;
            let tint = shade([62, 116, 77], 0.81 + 0.09 * i as f32);
            p.blade(
                (25.0 + i as f32 * 2.0, 65.0),
                (x, y),
                3.35,
                (x - 32.0) * 0.41,
                tint,
                0.12,
            );
        }
    } else {
        for (x, bend) in [(17., 4.), (31., -7.), (47., 5.)] {
            let at = p.stem((x, -1.0), (x, 65.0), 1.43, bend, STEM);
            for j in 0..9 {
                let a = at(j as f32 / 9.0);
                let side = if j % 2 == 0 { 1.0 } else { -1.0 };
                p.blade(
                    a,
                    (a.0 + side * 9.6, a.1 + 8.0),
                    3.4,
                    side * 1.7,
                    shade(LEAF, 0.88 + 0.13 * (j % 3) as f32),
                    0.05,
                );
            }
        }
        let (first, remaining) = p.pixels.split_at_mut(SIDE);
        remaining[(SIDE - 2) * SIDE..].copy_from_slice(first);
    }
}

fn cactus(p: &mut Plant, top: bool) {
    p.paint((0., 0.), (63., 63.), |x, y, grain| {
        let (phase, height) = if top {
            let center = (x - 32.0, y - 32.0);
            (
                center.1.atan2(center.0) * 8.0,
                center.0.hypot(center.1) / 40.0,
            )
        } else {
            (x / 64.0 * TAU * 8.0, y / 64.0)
        };
        let rib = (phase.cos() * 0.5 + 0.5).powf(1.8);
        let tint = mix([43, 82, 49], [99, 138, 79], rib);
        let areole = rib > 0.57 && (height * if top { 4.0 } else { 6.0 }).fract() < 0.17;
        let tone = if areole {
            mix(tint, [203, 183, 127], 0.30 + grain * 0.48)
        } else {
            tint
        };
        Some(shade(tone, 0.88 + grain * 0.19))
    });
    if !top {
        let (first, remaining) = p.pixels.split_at_mut(SIDE);
        remaining[(SIDE - 2) * SIDE..].copy_from_slice(first);
    }
}

fn sunflower(p: &mut Plant) {
    let at = p.stem((31.0, 129.0), (32.0, 25.0), 2.4, -4.0, STEM);
    for (t, s, reach) in [
        (0.18, 1.0, 19.0),
        (0.30, -1.0, 20.0),
        (0.47, 1.0, 25.0),
        (0.63, -1.0, 23.0),
        (0.72, 1.0, 18.0),
    ] {
        let a = at(t);
        p.blade(a, (a.0 + s * reach, a.1 - 13.0), 5.8, s * 3.7, LEAF, 0.27);
    }
    p.flower((32.0, 24.5), 21, 23.0, 2.9, [238, 180, 47], [111, 76, 46]);
}

pub(super) fn draw(name: &str) -> Option<Vec<Pixel>> {
    if matches!(
        name,
        "sunflower" | "sunflower_top" | "tall_grass_double" | "tall_grass_double_top"
    ) {
        let mut p = Plant::new(2 * SIDE);
        if name.starts_with("sunflower") {
            sunflower(&mut p);
        } else {
            grass(&mut p, true);
        }
        // Les deux cartes partagent la ligne de pixels à leur jonction.
        let (upper, lower) = p.pixels.split_at_mut(SIDE * SIDE);
        lower[..SIDE].copy_from_slice(&upper[(SIDE - 1) * SIDE..]);
        return Some(if name.ends_with("_top") { upper } else { lower }.to_vec());
    }

    let mut p = Plant::new(SIDE);
    match name {
        "tall_grass" => grass(&mut p, false),
        "fern" => fern(&mut p),
        "heather" => heather(&mut p),
        "poppy" | "cornflower" | "daisy" | "dandelion" | "marigold" | "tulip" | "allium" => {
            wildflower(&mut p, name)
        }
        "red_mushroom" | "brown_mushroom" | "glowcap" => mushroom(&mut p, name),
        "dead_bush" => woody(&mut p, name),
        "lily_pad" | "seaweed" | "vine" => water_plant(&mut p, name),
        "cactus_side" | "cactus_top" => cactus(&mut p, name == "cactus_top"),
        "pink_petals" => {
            let mut rng = Rng::new(729);
            for i in 0..23 {
                let (x, y) = (5.0 + 54.0 * rng.next_f32(), 5.0 + 54.0 * rng.next_f32());
                let a = TAU * rng.next_f32();
                let tint = shade([223, 155, 175], 0.82 + 0.26 * rng.next_f32());
                p.ellipse((x, y), (2.4 + 1.6 * rng.next_f32(), 1.5), a, tint);
                if i % 6 == 0 {
                    p.flower((x + 1.8, y + 2.2), 5, 3.9, 1.6, tint, [176, 99, 132]);
                }
            }
        }
        _ if name.ends_with("_sapling") => woody(&mut p, name.trim_end_matches("_sapling")),
        _ => {
            let (species, stage) = name.rsplit_once('_')?;
            let stage = stage.parse::<usize>().ok()?;
            match species {
                "carrot" | "potato" | "cabbage" | "melon" if stage < 4 => {
                    crop(&mut p, species, stage)
                }
                "sweet_berry_bush" | "blueberry_bush" if stage < 3 => {
                    berries(&mut p, species == "blueberry_bush", stage)
                }
                _ => return None,
            }
        }
    }
    Some(p.pixels)
}
