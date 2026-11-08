//! Le maillage d'un cube unité, de (0,0,0) à (1,1,1).
//!
//! Chaque face a ses 4 propres sommets (24 au total) pour porter sa normale :
//! un sommet partagé entre trois faces ne pourrait avoir qu'une seule normale.

use bytemuck::{Pod, Zeroable};

/// `#[repr(C)]` garantit la disposition mémoire attendue par le GPU,
/// et `Pod` (« plain old data ») permet de le copier en octets sans `unsafe`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Un sommet de maillage coloré et éclairé, pour les chunks : la couleur est par
/// sommet au lieu d'être par instance.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ColorVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Couleur et opacité.
    pub color: [f32; 4],
    /// Lumière du ciel, lumière des blocs et occlusion ambiante, de 0 à 1.
    pub light: [f32; 3],
    /// Couche de texture (0 : pas de texture, blanc).
    pub texture: u32,
}

/// Pour chaque face : sa normale et ses 4 coins, dans l'ordre anti-horaire vu de l'extérieur.
const FACES: [([f32; 3], [[f32; 3]; 4]); 6] = [
    // +X
    (
        [1.0, 0.0, 0.0],
        [
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [1.0, 0.0, 1.0],
        ],
    ),
    // -X
    (
        [-1.0, 0.0, 0.0],
        [
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
        ],
    ),
    // +Y (dessus)
    (
        [0.0, 1.0, 0.0],
        [
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    ),
    // -Y (dessous)
    (
        [0.0, -1.0, 0.0],
        [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    ),
    // +Z
    (
        [0.0, 0.0, 1.0],
        [
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    ),
    // -Z
    (
        [0.0, 0.0, -1.0],
        [
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ],
    ),
];

pub fn cube() -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in FACES {
        let base = vertices.len() as u16;
        vertices.extend(corners.map(|position| Vertex { position, normal }));
        // Deux triangles par face : 0-1-2 et 0-2-3.
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn cube_has_24_vertices_and_12_triangles() {
        let (v, i) = cube();
        assert_eq!(v.len(), 24);
        assert_eq!(i.len(), 36);
    }

    /// Les triangles doivent tourner dans le sens anti-horaire vu de l'extérieur,
    /// sinon le « backface culling » les cache.
    #[test]
    fn triangles_face_outwards() {
        let (v, i) = cube();
        for tri in i.chunks(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]].map(|k| Vec3::from(v[k as usize].position));
            let geometric = (b - a).cross(c - a).normalize();
            let declared = Vec3::from(v[tri[0] as usize].normal);
            assert!(
                geometric.dot(declared) > 0.99,
                "triangle {tri:?} mal orienté"
            );
        }
    }
}
