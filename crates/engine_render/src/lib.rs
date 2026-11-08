//! `engine_render` : le rendu 3D avec wgpu.
//!
//! Trois façons de dessiner, éclairées par un soleil fixe et vues par une caméra
//! perspective, avec un tampon de profondeur :
//! - des cubes instanciés ([`CubeInstance`]) : un seul maillage répété ;
//! - des maillages quelconques ([`Renderer::add_mesh`]), un par chunk, triés
//!   par frustum culling ([`Frustum`]) ;
//! - des lignes : le contour d'une boîte ([`Renderer::outline`]) et un réticule.

mod camera;
mod frustum;
mod lines;
mod mesh;
mod renderer;

pub use camera::Camera;
pub use frustum::{Aabb, Frustum};
pub use mesh::ColorVertex;
pub use renderer::{Capture, CubeInstance, Fog, MeshId, RenderStats, Renderer, Sun};

/// Une couleur pour [`Renderer::clear_color`], sans que le jeu dépende de wgpu.
pub fn wgpu_color(r: f64, g: f64, b: f64) -> wgpu::Color {
    wgpu::Color { r, g, b, a: 1.0 }
}
