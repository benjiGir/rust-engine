//! Caméra perspective à la première personne, décrite par une position et deux angles.

use glam::camera::rh::{proj, view};
use glam::{Mat4, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub position: Vec3,
    /// Rotation horizontale en radians. 0 = regarde vers +X.
    pub yaw: f32,
    /// Rotation verticale en radians, bornée pour ne pas passer « par-dessus ».
    pub pitch: f32,
    /// Champ de vision vertical en radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl Camera {
    /// Limite du pitch : juste sous 90° pour éviter que `forward` devienne vertical.
    pub const MAX_PITCH: f32 = 89f32.to_radians();

    /// Direction du regard (vecteur unitaire).
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(cos_yaw * cos_pitch, sin_pitch, sin_yaw * cos_pitch)
    }

    /// Direction « à droite », toujours horizontale.
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize()
    }

    /// Direction « devant » projetée sur le sol, pour marcher sans s'envoler.
    pub fn forward_flat(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, sin_yaw)
    }

    pub fn view(&self) -> Mat4 {
        view::look_to_mat4(self.position, self.forward(), Vec3::Y)
    }

    /// Projection perspective. wgpu suit la convention DirectX / Metal :
    /// profondeur entre 0 et 1 et Y vers le haut, contrairement à OpenGL (-1 à 1).
    pub fn projection(&self, aspect: f32) -> Mat4 {
        proj::directx::perspective(self.fov_y, aspect, self.near, self.far)
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_zero_looks_along_x() {
        let cam = Camera::default();
        assert!((cam.forward() - Vec3::X).length() < 1e-6);
        assert!((cam.right() - Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn point_in_front_lands_in_clip_space() {
        let cam = Camera::default();
        let clip = cam.view_projection(16.0 / 9.0) * Vec3::new(10.0, 0.0, 0.0).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5,
            "centre de l'écran"
        );
        assert!((0.0..=1.0).contains(&ndc.z), "profondeur entre 0 et 1");
    }

    #[test]
    fn point_behind_is_clipped() {
        let cam = Camera::default();
        let clip = cam.view_projection(1.0) * Vec3::new(-10.0, 0.0, 0.0).extend(1.0);
        assert!(clip.w < 0.0, "derrière la caméra");
    }
}
