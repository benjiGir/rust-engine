//! Frustum culling : ne pas envoyer au GPU ce que la caméra ne peut pas voir.
//!
//! Le « frustum » est la pyramide tronquée que voit la caméra, bornée par 6 plans
//! (gauche, droite, bas, haut, proche, lointain). On les extrait directement de la
//! matrice `view_proj` (méthode de Gribb et Hartmann), puis on teste chaque boîte.

use glam::{Mat4, Vec3, Vec4};

/// Une boîte alignée sur les axes (« Axis-Aligned Bounding Box »).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }
}

/// Les 6 plans du frustum. Chaque plan est un `Vec4` (a, b, c, d) : un point `p`
/// est du côté visible quand `a·x + b·y + c·z + d >= 0`.
#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Un point `p` est visible si son image `clip = M·p` vérifie
    /// `-w <= x <= w`, `-w <= y <= w` et `0 <= z <= w` (profondeur wgpu entre 0 et 1).
    /// Chaque inégalité s'écrit avec les lignes de `M` : par exemple `x >= -w`
    /// devient `(ligne3 + ligne0)·p >= 0`. Ce sont nos 6 plans.
    pub fn from_view_proj(m: Mat4) -> Self {
        let [r0, r1, r2, r3] = [m.row(0), m.row(1), m.row(2), m.row(3)];
        let planes = [
            r3 + r0, // gauche
            r3 - r0, // droite
            r3 + r1, // bas
            r3 - r1, // haut
            r2,      // proche (z >= 0)
            r3 - r2, // lointain (z <= w)
        ]
        // On normalise pour que `d` soit une vraie distance (utile pour déboguer).
        .map(|p| p / p.truncate().length());
        Self { planes }
    }

    /// Vrai si la boîte est au moins en partie dans le frustum.
    ///
    /// Pour chaque plan, on prend le coin de la boîte le plus « en avant » selon
    /// la normale du plan (le « p-vertex »). S'il est derrière, toute la boîte l'est.
    /// Le test est prudent : une boîte près d'un coin du frustum peut être gardée
    /// à tort, mais une boîte visible n'est jamais rejetée.
    pub fn intersects(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|plane| {
            let normal = plane.truncate();
            // `select` choisit composante par composante : max si la normale est positive.
            let p = Vec3::select(normal.cmpge(Vec3::ZERO), aabb.max, aabb.min);
            normal.dot(p) + plane.w >= 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    /// Caméra à l'origine, regard vers +X, de 0,1 à 1000 unités.
    fn frustum() -> Frustum {
        Frustum::from_view_proj(Camera::default().view_projection(16.0 / 9.0))
    }

    fn unit_box_at(center: Vec3) -> Aabb {
        Aabb::new(center - 0.5, center + 0.5)
    }

    #[test]
    fn box_in_front_is_visible() {
        assert!(frustum().intersects(&unit_box_at(Vec3::new(10.0, 0.0, 0.0))));
    }

    #[test]
    fn box_behind_is_culled() {
        assert!(!frustum().intersects(&unit_box_at(Vec3::new(-10.0, 0.0, 0.0))));
    }

    #[test]
    fn box_far_to_the_side_is_culled() {
        // 10 devant, 100 sur le côté : bien hors du champ de vision de 70°.
        assert!(!frustum().intersects(&unit_box_at(Vec3::new(10.0, 0.0, 100.0))));
        assert!(!frustum().intersects(&unit_box_at(Vec3::new(10.0, 100.0, 0.0))));
    }

    #[test]
    fn box_beyond_far_plane_is_culled() {
        assert!(!frustum().intersects(&unit_box_at(Vec3::new(2000.0, 0.0, 0.0))));
    }

    #[test]
    fn box_around_the_camera_is_visible() {
        // La caméra est dans la boîte : elle en voit forcément une partie.
        let aabb = Aabb::new(Vec3::splat(-50.0), Vec3::splat(50.0));
        assert!(frustum().intersects(&aabb));
    }

    #[test]
    fn box_crossing_the_edge_is_visible() {
        // Le centre est hors champ, mais la boîte touche le bord du frustum.
        let aabb = Aabb::new(Vec3::new(10.0, -1.0, 5.0), Vec3::new(12.0, 1.0, 30.0));
        assert!(frustum().intersects(&aabb));
    }
}
