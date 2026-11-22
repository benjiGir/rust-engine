//! `engine_render` : le rendu 3D avec wgpu.
//!
//! Trois façons de dessiner, éclairées par le soleil et vues par une caméra
//! perspective, avec un tampon de profondeur :
//! - des cubes instanciés ([`CubeInstance`]) : un seul maillage répété ;
//! - des maillages quelconques ([`Renderer::add_mesh`]), un par chunk, triés
//!   par frustum culling ([`Frustum`]) ;
//! - des lignes : le contour d'une boîte ([`Renderer::outline`]) et un réticule.

mod arena;
mod camera;
mod frustum;
mod gpu_timer;
mod lines;
mod mesh;
mod renderer;
mod shader_pack;
mod sky;
mod sun_rays;
mod sun_shadows;

pub use arena::ArenaStats;
pub use camera::Camera;
pub use frustum::{Aabb, Frustum};
pub use mesh::ColorVertex;
pub use renderer::{
    CHUNK_SHADER_PATH, CHUNK_SHADER_SOURCE, Capture, CubeInstance, Fog, HandInstance, MeshId,
    RenderStats, Renderer, Sun, TEXTURE_SIZE, UiFrame, texture_side,
};
pub use shader_pack::{ShaderEffects, ShaderSources};
pub use sky::{SKY_SHADER_PATH, SKY_SHADER_SOURCE, Sky};
// Le jeu construit son interface avec la même version d'egui que le renderer.
pub use egui;

/// Une couleur pour [`Renderer::clear_color`], sans que le jeu dépende de wgpu.
pub fn wgpu_color(r: f64, g: f64, b: f64) -> wgpu::Color {
    wgpu::Color { r, g, b, a: 1.0 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shaders_are_valid_wgsl() {
        for source in [
            &crate::renderer::terrain_with_flame(super::CHUNK_SHADER_SOURCE),
            crate::renderer::CUBE_SHADER_SOURCE,
            crate::renderer::HAND_SHADER_SOURCE,
            include_str!("sky.wgsl"),
            crate::sun_rays::SOURCE,
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        }
    }
}
