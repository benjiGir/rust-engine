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
pub use renderer::{CubeInstance, Fog, MeshId, RenderStats, Renderer};
