//! Le contour du bloc visé et le réticule, dessinés en lignes (`LineList`).
//!
//! Deux pipelines partagent un shader et un petit tampon de sommets réécrit à
//! chaque image : 24 sommets pour les 12 arêtes du contour, puis 4 pour la croix.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::frustum::Aabb;

/// Un sommet de ligne : position et couleur.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

const OUTLINE_VERTICES: usize = 24;
const CROSSHAIR_VERTICES: usize = 4;
const OUTLINE_COLOR: [f32; 3] = [0.05, 0.05, 0.05];
const CROSSHAIR_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
/// Demi-longueur d'une branche du réticule, en fraction de la hauteur de l'écran.
const CROSSHAIR_SIZE: f32 = 0.025;
/// On grossit un peu le contour : collé aux faces, il se battrait avec elles dans
/// le tampon de profondeur (« z-fighting ») et clignoterait.
const OUTLINE_GROW: f32 = 0.002;

/// Les 12 arêtes d'une boîte, deux sommets par arête.
pub fn outline_vertices(aabb: &Aabb) -> [LineVertex; OUTLINE_VERTICES] {
    let min = aabb.min - Vec3::splat(OUTLINE_GROW);
    let max = aabb.max + Vec3::splat(OUTLINE_GROW);
    // Les 8 coins : le bit 0 de `i` choisit x, le bit 1 choisit y, le bit 2 choisit z.
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    // Deux coins forment une arête s'ils ne diffèrent que d'un bit.
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7), // le long de x
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7), // le long de y
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7), // le long de z
    ];
    let mut vertices = [LineVertex::zeroed(); OUTLINE_VERTICES];
    for (k, (a, b)) in EDGES.into_iter().enumerate() {
        vertices[2 * k] = vertex(corner(a), OUTLINE_COLOR);
        vertices[2 * k + 1] = vertex(corner(b), OUTLINE_COLOR);
    }
    vertices
}

/// Une croix au centre de l'écran. `aspect` (largeur / hauteur) corrige la branche
/// horizontale, sinon elle serait plus longue sur un écran large.
pub fn crosshair_vertices(aspect: f32) -> [LineVertex; CROSSHAIR_VERTICES] {
    let h = CROSSHAIR_SIZE / aspect;
    let v = CROSSHAIR_SIZE;
    [
        vertex(Vec3::new(-h, 0.0, 0.0), CROSSHAIR_COLOR),
        vertex(Vec3::new(h, 0.0, 0.0), CROSSHAIR_COLOR),
        vertex(Vec3::new(0.0, -v, 0.0), CROSSHAIR_COLOR),
        vertex(Vec3::new(0.0, v, 0.0), CROSSHAIR_COLOR),
    ]
}

fn vertex(position: Vec3, color: [f32; 3]) -> LineVertex {
    LineVertex {
        position: position.to_array(),
        color,
    }
}

/// Les ressources GPU des lignes.
pub(crate) struct Lines {
    world_pipeline: wgpu::RenderPipeline,
    screen_pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
}

impl Lines {
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("lines.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines"),
            bind_group_layouts: &[Some(camera_layout)],
            ..Default::default()
        });
        // Une closure qui crée un pipeline : seuls le point d'entrée et la profondeur changent.
        let pipeline = |entry: &str, depth_compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<LineVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                    })],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    // Chaque paire de sommets forme un segment indépendant.
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                // La passe a un tampon de profondeur : chaque pipeline doit le déclarer,
                // même s'il ne s'en sert pas. Les lignes ne l'écrivent jamais.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            // Le contour est caché par les blocs placés devant lui.
            world_pipeline: pipeline("vs_world", wgpu::CompareFunction::LessEqual),
            // Le réticule est toujours visible.
            screen_pipeline: pipeline("vs_screen", wgpu::CompareFunction::Always),
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lines"),
                size: ((OUTLINE_VERTICES + CROSSHAIR_VERTICES) * size_of::<LineVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }

    /// Prépare les sommets de cette image (avant la passe de rendu).
    pub(crate) fn upload(&self, queue: &wgpu::Queue, outline: Option<&Aabb>, aspect: f32) {
        if let Some(aabb) = outline {
            queue.write_buffer(
                &self.buffer,
                0,
                bytemuck::cast_slice(&outline_vertices(aabb)),
            );
        }
        let offset = (OUTLINE_VERTICES * size_of::<LineVertex>()) as u64;
        queue.write_buffer(
            &self.buffer,
            offset,
            bytemuck::cast_slice(&crosshair_vertices(aspect)),
        );
    }

    /// Dessine dans la passe en cours. Le groupe de la caméra doit déjà être lié.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, outline: bool, crosshair: bool) {
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        if outline {
            pass.set_pipeline(&self.world_pipeline);
            pass.draw(0..OUTLINE_VERTICES as u32, 0..1);
        }
        if crosshair {
            pass.set_pipeline(&self.screen_pipeline);
            let start = OUTLINE_VERTICES as u32;
            pass.draw(start..start + CROSSHAIR_VERTICES as u32, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_has_12_axis_aligned_edges_of_block_length() {
        let aabb = Aabb::new(Vec3::new(2.0, 3.0, 4.0), Vec3::new(3.0, 4.0, 5.0));
        let vertices = outline_vertices(&aabb);
        for edge in vertices.chunks(2) {
            let d = Vec3::from(edge[1].position) - Vec3::from(edge[0].position);
            // Une seule composante non nulle, de longueur ~1 (grossie de 2 × 0,002).
            let nonzero = d.to_array().iter().filter(|c| c.abs() > 1e-6).count();
            assert_eq!(nonzero, 1, "arête {edge:?}");
            assert!((d.length() - (1.0 + 2.0 * OUTLINE_GROW)).abs() < 1e-5);
        }
    }

    #[test]
    fn crosshair_branches_look_equal_on_a_wide_screen() {
        let aspect = 16.0 / 9.0;
        let v = crosshair_vertices(aspect);
        // En pixels, la branche horizontale vaut largeur × x / 2, la verticale hauteur × y / 2.
        let horizontal_px = (v[1].position[0] - v[0].position[0]) * aspect;
        let vertical_px = v[3].position[1] - v[2].position[1];
        assert!((horizontal_px - vertical_px).abs() < 1e-6);
    }
}
