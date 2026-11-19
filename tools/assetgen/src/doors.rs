//! Planches huilées, ferrures en cuivre et panneaux d'acier brossé.

pub const TEXTURES: [&str; 6] = [
    "wooden_door_lower",
    "wooden_door_upper",
    "wooden_door_window",
    "iron_door_lower",
    "iron_door_upper",
    "iron_door_window",
];

pub fn draw(name: &str, side: usize) -> Option<Vec<[u8; 4]>> {
    if !TEXTURES.contains(&name) {
        return None;
    }
    let wood = name.starts_with("wooden");
    let lower = name.ends_with("lower");
    let window = name.ends_with("window");
    let body = if wood {
        [170, 119, 74]
    } else {
        [137, 151, 154]
    };
    let trim = if wood { [135, 87, 52] } else { [97, 113, 123] };
    let metal = if wood {
        [188, 127, 77]
    } else {
        [178, 188, 191]
    };
    let mut pixels = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let (u, v) = (x * 16 / side, y * 16 / side);
            let edge = u == 0 || u == 15;
            let crossbar = lower && v >= 13 || !lower && v < 3;
            let mut color = if edge || crossbar { trim } else { body };
            let grain = if wood {
                [0.84, 1.05, 1.0, 0.92, 1.09][(u + 3) % 5]
            } else {
                [1.0, 0.95, 1.02, 0.98][y % 4]
            };
            let fine = if side == 32 {
                (x * 19 + y * 31 + (x * y) % 17) % 13
            } else {
                (u * 19 + v * 13) % 11
            };
            let mut shade = grain * (0.94 + fine as f32 * 0.01);
            if window
                && ((u == 2 || u == 13) && (2..=12).contains(&v)
                    || (2..=13).contains(&u) && (v == 2 || v == 12))
            {
                color = metal;
                shade = if v == 2 || u == 2 { 1.11 } else { 0.73 };
            }
            if lower && (11..=14).contains(&u) && (2..=5).contains(&v) {
                color = metal;
                shade = if v == 2 || u == 11 { 1.13 } else { 0.82 };
                if (12..=13).contains(&u) && v == 4 {
                    shade = 0.40;
                }
            }
            if u <= 2
                && (if lower {
                    (10..=11).contains(&v)
                } else {
                    (5..=6).contains(&v)
                })
            {
                color = metal;
                shade = if u == 1 { 1.10 } else { 0.75 };
            }
            if !wood && (u == 1 || u == 14) && matches!(v, 1 | 14) {
                color = metal;
                shade = 1.12;
            }
            let [r, g, b] = color.map(|c| (c as f32 * shade).round().min(255.0) as u8);
            pixels.push([r, g, b, 255]);
        }
    }
    Some(pixels)
}
