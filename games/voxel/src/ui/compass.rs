//! La boussole : une règle graduée en haut de l'écran qui montre vers où l'on regarde, quand une
//! boussole (`core:compass`) est dans la barre d'objets.
//!
//! Le nord est le sens des z négatifs (`docs/plan-biomes.md`). La caméra regarde dans la direction
//! `(cos yaw, sin yaw)` du plan (x, z) : `yaw = -90°` regarde donc plein nord.

use engine_render::egui::{self, Align2, Color32, FontId, Pos2, Stroke};

use super::skin;

/// Le cap en degrés (0 = nord, 90 = est, 180 = sud, 270 = ouest) pour un `yaw` de caméra en radians.
pub fn bearing_degrees(yaw: f32) -> f32 {
    // Direction regardée (dx, dz) = (cos yaw, sin yaw) ; le nord est -z, l'est +x.
    let (dz, dx) = yaw.sin_cos();
    dx.atan2(-dz).to_degrees().rem_euclid(360.0)
}

/// Les seize repères de la règle : (cap en degrés, nom). Les points cardinaux sont en gras.
const MARKS: [(f32, &str); 8] = [
    (0.0, "N"),
    (45.0, "NE"),
    (90.0, "E"),
    (135.0, "SE"),
    (180.0, "S"),
    (225.0, "SO"),
    (270.0, "O"),
    (315.0, "NO"),
];

/// L'écart signé, de -180 à 180, entre deux caps.
pub fn delta(from: f32, to: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// Dessine la règle en haut au centre de `ctx`, pour un cap `heading` (degrés).
pub fn draw(ctx: &egui::Context, heading: f32) {
    egui::Area::new(egui::Id::unique("compass"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 26.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(25, 26, 25, 170));
            painter.rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, skin::BORDER),
                egui::StrokeKind::Inside,
            );
            let half_window = 80.0; // degrés visibles de chaque côté
            let center = rect.center();
            let scale = (rect.width() / 2.0 - 6.0) / half_window;
            // Un petit trait tous les 15 degrés, les repères nommés par-dessus.
            for step in 0..24 {
                let cap = step as f32 * 15.0;
                let d = delta(heading, cap);
                if d.abs() <= half_window {
                    let x = center.x + d * scale;
                    painter.line_segment(
                        [
                            Pos2::new(x, rect.bottom() - 7.0),
                            Pos2::new(x, rect.bottom() - 3.0),
                        ],
                        Stroke::new(1.0, skin::MUTED),
                    );
                }
            }
            for (cap, name) in MARKS {
                let d = delta(heading, cap);
                if d.abs() <= half_window {
                    let color = if cap == 0.0 { skin::RED } else { skin::TEXT };
                    let size = if name.len() == 1 { 15.0 } else { 11.0 };
                    painter.text(
                        Pos2::new(center.x + d * scale, rect.top() + 11.0),
                        Align2::CENTER_CENTER,
                        name,
                        FontId::proportional(size),
                        color,
                    );
                }
            }
            // Le repère central.
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(center.x - 4.0, rect.bottom() - 1.0),
                    Pos2::new(center.x + 4.0, rect.bottom() - 1.0),
                    Pos2::new(center.x, rect.bottom() - 7.0),
                ],
                skin::COPPER,
                Stroke::NONE,
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        delta(a, b).abs() < 0.01
    }

    #[test]
    fn the_camera_looking_toward_negative_z_faces_north() {
        assert!(close(bearing_degrees(-std::f32::consts::FRAC_PI_2), 0.0));
        // +x : l'est ; +z : le sud ; -x : l'ouest.
        assert!(close(bearing_degrees(0.0), 90.0));
        assert!(close(bearing_degrees(std::f32::consts::FRAC_PI_2), 180.0));
        assert!(close(bearing_degrees(std::f32::consts::PI), 270.0));
    }

    #[test]
    fn deltas_wrap_around_north() {
        assert!(close(delta(350.0, 10.0), 20.0));
        assert!(close(delta(10.0, 350.0), -20.0));
        assert!((delta(0.0, 180.0).abs() - 180.0).abs() < 0.01);
    }
}
