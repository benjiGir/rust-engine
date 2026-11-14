//! Menuiserie 32 : planches décalées, fibres sinueuses, nœuds, écorces et cernes.
use super::draw::{Canvas, Color, noise, seed, shade};

pub(super) fn palette(name: &str) -> Color {
    if name.starts_with("birch") {
        [188, 170, 121]
    } else if name.starts_with("spruce") {
        [111, 82, 52]
    } else if name.starts_with("acacia") {
        [168, 104, 65]
    } else if name.starts_with("jungle") {
        [149, 113, 75]
    } else {
        [157, 121, 72]
    }
}

pub(super) fn planks(base: Color, seed: u64) -> Canvas {
    let mut c = Canvas::new(base);
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32, (y % 31) as f32);
            let row = (v / 7.75) as i32;
            let across = v % 7.75;
            let joint = (u + row as f32 * 12.0).rem_euclid(31.0);
            let knot_x = if row % 2 == 0 { 13.0 } else { 24.0 };
            let dx = (u - knot_x + 15.5).rem_euclid(31.0) - 15.5;
            let dy = across - 4.5;
            let radius = (dx * dx * 0.14 + dy * dy * 1.9).sqrt();
            let grain = (across * 2.0 + (u * 0.20).sin() * 0.5 + 2.0 / (dx.abs() + 1.0)).sin();
            let factor = if across < 0.65 || joint < 0.6 {
                0.53
            } else if across < 1.25 {
                1.24
            } else if joint < 1.4 {
                0.82
            } else if radius < 1.0 {
                0.57
            } else if radius < 2.4 && radius.rem_euclid(0.9) < 0.35 {
                0.8
            } else {
                0.98 + grain * 0.065
                    + noise(u / 31.0, v / 31.0, 7, seed) * 0.11
                    + row as f32 * 0.014
            };
            c.put(x, y, shade(base, factor));
        }
    }
    c.periodic(true);
    c
}

pub(super) fn draw(name: &str) -> Option<Canvas> {
    let base = palette(name);
    if name.ends_with("_planks") {
        return Some(planks(base, seed(name)));
    }
    if name.ends_with("_log_top") {
        let mut c = Canvas::new(shade(base, 0.50));
        for y in 1..31 {
            for x in 1..31 {
                let (dx, dy) = (x as f32 - 14.0, y as f32 - 16.0);
                let r = (dx * dx * 0.88 + dy * dy).sqrt();
                let angle = dy.atan2(dx);
                let ring = (r + (angle * 3.0).sin() * 0.45).rem_euclid(2.75);
                let factor = if r > 14.0 {
                    0.53
                } else if r > 13.0 {
                    0.7
                } else if ring < 0.6 {
                    0.72
                } else if ring < 1.15 {
                    1.16
                } else {
                    1.01
                };
                c.put(x, y, shade(base, factor));
            }
        }
        c.line((24, 5), (21, 10), shade(base, 0.52));
        c.line((21, 10), (20, 13), shade(base, 0.58));
        c.line((5, 24), (10, 22), shade(base, 0.56));
        return Some(c);
    }
    if !name.ends_with("_log") {
        return None;
    }
    let birch = name.starts_with("birch");
    let base = if birch {
        [196, 195, 175]
    } else {
        shade(base, 0.63)
    };
    let mut c = Canvas::new(base);
    for y in 0..32 {
        for x in 0..32 {
            let (u, v) = ((x % 31) as f32 / 31.0, (y % 31) as f32 / 31.0);
            let bend = (v * std::f32::consts::TAU).sin() * 0.17;
            let lane = (u * 6.0 + bend + noise(u, v, 4, seed(name)) * 0.3).fract();
            let f = if birch {
                0.97 + noise(u, v, 8, 661) * 0.10
            } else if lane < 0.15 {
                0.56
            } else if lane < 0.27 {
                1.30
            } else {
                0.90 + noise(u, v, 11, 664) * 0.23
            };
            c.put(x, y, shade(base, f));
        }
    }
    if birch {
        for (x, y, w) in [(3, 4, 7), (20, 12, 9), (7, 23, 5), (24, 28, 5)] {
            c.line((x, y), (x + w, y), [69, 73, 67]);
            c.line((x + 2, y + 1), (x + w - 1, y + 1), [98, 99, 88]);
            c.line((x, y - 1), (x + w - 3, y - 1), shade(base, 1.1));
        }
    } else {
        for y in 11..22 {
            for x in 9..18 {
                let r = (((x - 13) as f32 * 0.8).powi(2) + ((y - 16) as f32 * 0.43).powi(2)).sqrt();
                if r < 3.3 {
                    c.put(
                        x,
                        y,
                        shade(
                            base,
                            if r < 1.2 {
                                0.43
                            } else if r < 2.0 {
                                1.15
                            } else {
                                0.67
                            },
                        ),
                    );
                }
            }
        }
    }
    c.periodic(true);
    Some(c)
}
