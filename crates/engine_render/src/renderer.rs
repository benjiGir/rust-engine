//! Le renderer wgpu : tout ce que Unity fait entre ton mesh et l'écran.
//!
//! Vocabulaire wgpu :
//! - `Instance` : point d'entrée, choisit le backend (Metal, DirectX 12, Vulkan).
//! - `Surface` : la zone de la fenêtre où l'on dessine.
//! - `Adapter` : la carte graphique choisie. `Device` : notre connexion à elle.
//! - `Queue` : la file où l'on envoie les commandes et les données.
//! - `RenderPipeline` : shaders + format des sommets + état de rendu, figés ensemble.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::camera::Camera;
use crate::frustum::{Aabb, Frustum};
use crate::lines::Lines;
use crate::mesh::{self, ColorVertex, Vertex};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Un cube à dessiner : sa position (coin minimum) et sa couleur.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CubeInstance {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl CubeInstance {
    pub fn new(position: Vec3, color: Vec3) -> Self {
        Self {
            position: position.to_array(),
            color: color.to_array(),
        }
    }
}

/// Ce qui a été dessiné pendant la dernière image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Maillages (chunks) dessinés, après frustum culling.
    pub meshes_drawn: usize,
    /// Maillages chargés au total.
    pub meshes_total: usize,
    /// Triangles envoyés au GPU (maillages dessinés et cubes instanciés).
    pub triangles: usize,
}

/// Identifiant d'un maillage chargé, renvoyé par [`Renderer::add_mesh`].
///
/// Un « handle » opaque : le jeu le garde et le rend pour retirer le maillage,
/// sans jamais toucher aux tampons GPU eux-mêmes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshId(u64);

/// Un brouillard linéaire : invisible avant `start`, couleur du ciel après `end`.
/// Il cache la limite du monde chargé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    pub start: f32,
    pub end: f32,
}

/// Un maillage chargé sur le GPU : ses tampons et sa boîte englobante.
///
/// Pas besoin d'écrire de code de libération : quand un `GpuMesh` est détruit
/// (retiré de la `HashMap`), Rust appelle `Drop` sur ses deux `wgpu::Buffer`,
/// qui rendent leur mémoire au GPU. C'est le RAII, comme `using` en C# mais automatique.
struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    bounds: Aabb,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    /// Position de la caméra (xyz) et début du brouillard (w).
    camera_fog_start: [f32; 4],
    /// Couleur du brouillard (rgb) et fin du brouillard (w).
    fog_color_end: [f32; 4],
}

pub struct Renderer {
    // `'static` : la surface garde un `Arc<Window>`, elle ne peut donc pas lui survivre.
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
    chunk_pipeline: wgpu::RenderPipeline,
    meshes: HashMap<MeshId, GpuMesh>,
    next_mesh_id: u64,
    lines: Lines,
    /// Une boîte dont on dessine les arêtes, par exemple le bloc visé.
    pub outline: Option<Aabb>,
    /// Une croix au centre de l'écran.
    pub crosshair: bool,
    /// Couleur du ciel.
    pub clear_color: wgpu::Color,
    /// Ne dessine que les maillages dont la boîte touche le champ de vision.
    pub frustum_culling: bool,
    /// `None` : pas de brouillard.
    pub fog: Option<Fog>,
}

impl Renderer {
    /// L'initialisation de wgpu est asynchrone ; `pollster` attend simplement le résultat.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        // Le « display handle » permet à wgpu de choisir le bon backend pour cet écran.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(window.clone()),
        ));
        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("aucune carte graphique compatible")?;
        log::info!("GPU : {:?}", adapter.get_info());

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("device"),
                ..Default::default()
            })
            .await?;

        // On part de la configuration proposée par wgpu pour cette surface.
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface non supportée par cette carte graphique")?;
        // On préfère un format sRGB : le GPU convertit alors nos couleurs linéaires
        // pour l'écran, ce qui donne des dégradés corrects.
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        // `AutoVsync` : synchronisé sur l'écran. `AutoNoVsync` pour mesurer les FPS max.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let format = config.format;
        let depth_view = create_depth_view(&device, &config);

        // Caméra : un petit tampon « uniform » lu par le vertex shader.
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // Le fragment shader des chunks lit aussi la caméra (pour le brouillard).
                // Oublier une étape ici est une erreur de validation à la création du pipeline.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Pipeline des cubes instanciés. Tampon 0 : un sommet du cube.
        // Tampon 1 : une instance, avancée une fois par cube.
        let cube_layouts = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<CubeInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3],
            }),
        ];
        let pipeline = create_pipeline(
            &device,
            &camera_layout,
            format,
            &device.create_shader_module(wgpu::include_wgsl!("shader.wgsl")),
            &cube_layouts,
            "cubes",
        );

        // Pipeline des chunks : un seul tampon, un sommet = position, normale, couleur.
        let chunk_layouts = [Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<ColorVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
        })];
        let chunk_pipeline = create_pipeline(
            &device,
            &camera_layout,
            format,
            &device.create_shader_module(wgpu::include_wgsl!("chunk_shader.wgsl")),
            &chunk_layouts,
            "chunks",
        );

        let (vertices, indices) = mesh::cube();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let lines = Lines::new(&device, &camera_layout, format, DEPTH_FORMAT);

        let instance_capacity = 1024;
        let instance_buffer = create_instance_buffer(&device, instance_capacity);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            depth_view,
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            instance_buffer,
            instance_capacity,
            instance_count: 0,
            chunk_pipeline,
            lines,
            outline: None,
            crosshair: false,
            meshes: HashMap::new(),
            next_mesh_id: 0,
            frustum_culling: true,
            fog: None,
            clear_color: wgpu::Color {
                r: 0.45,
                g: 0.65,
                b: 0.9,
                a: 1.0,
            },
        })
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }

    /// À appeler quand la fenêtre change de taille.
    pub fn resize(&mut self, width: u32, height: u32) {
        // Une fenêtre minimisée fait 0×0 : wgpu refuse une surface vide.
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
    }

    /// Remplace la liste des cubes à dessiner. Le tampon GPU grandit si besoin.
    pub fn set_instances(&mut self, instances: &[CubeInstance]) {
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buffer = create_instance_buffer(&self.device, self.instance_capacity);
        }
        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
        self.instance_count = instances.len() as u32;
    }

    /// Charge un maillage (un chunk) sur le GPU. Il sera dessiné à chaque image
    /// jusqu'à [`Renderer::remove_mesh`] ou [`Renderer::clear_meshes`].
    ///
    /// `bounds` sert au frustum culling : la boîte doit contenir tous les sommets.
    /// Renvoie `None` pour un maillage vide (wgpu refuse les tampons vides).
    pub fn add_mesh(
        &mut self,
        vertices: &[ColorVertex],
        indices: &[u32],
        bounds: Aabb,
    ) -> Option<MeshId> {
        if indices.is_empty() {
            return None;
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk vertices"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk indices"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let id = MeshId(self.next_mesh_id);
        self.next_mesh_id += 1;
        self.meshes.insert(
            id,
            GpuMesh {
                vertex_buffer,
                index_buffer,
                index_count: indices.len() as u32,
                bounds,
            },
        );
        Some(id)
    }

    /// Retire un maillage ; ses tampons GPU sont libérés.
    pub fn remove_mesh(&mut self, id: MeshId) {
        self.meshes.remove(&id);
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Retire tous les maillages. Leurs tampons GPU sont libérés quand
    /// les `wgpu::Buffer` sont détruits (le trait `Drop` s'en charge).
    pub fn clear_meshes(&mut self) {
        self.meshes.clear();
    }

    /// Dessine une image vue par `camera` et renvoie ce qui a été dessiné,
    /// ou `None` si l'image a été sautée (fenêtre cachée, surface à reconfigurer).
    pub fn render(&mut self, camera: &Camera) -> Result<Option<RenderStats>> {
        // Un `enum` riche : chaque cas porte (ou non) une texture. `match` nous oblige
        // à traiter tous les cas, sinon le code ne compile pas.
        use wgpu::CurrentSurfaceTexture as Current;
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            Current::Success(frame) => (frame, false),
            Current::Suboptimal(frame) => (frame, true),
            // Fenêtre cachée ou GPU en retard : on saute simplement cette image.
            Current::Timeout | Current::Occluded => return Ok(None),
            // Surface périmée (fenêtre redimensionnée, écran changé) : on la reconfigure.
            Current::Outdated | Current::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(None);
            }
            Current::Validation => anyhow::bail!("erreur de validation de la surface"),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let view_proj = camera.view_projection(self.aspect());
        // Sans brouillard, on le repousse très loin. Pas `f32::MAX` : `smoothstep`
        // diviserait par `end - start`, ici 0, et le shader renverrait n'importe quoi.
        let fog = self.fog.unwrap_or(Fog {
            start: 1e30,
            end: 2e30,
        });
        let sky = self.clear_color;
        let uniform = CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_fog_start: camera.position.extend(fog.start).to_array(),
            fog_color_end: [sky.r as f32, sky.g as f32, sky.b as f32, fog.end],
        };
        let frustum = Frustum::from_view_proj(view_proj);
        let mut stats = RenderStats {
            meshes_total: self.meshes.len(),
            ..RenderStats::default()
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        // Les écritures de tampons partent avant les commandes de cette image.
        self.lines
            .upload(&self.queue, self.outline.as_ref(), self.aspect());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            // La passe de rendu emprunte `encoder` : ce bloc la termine avant `finish()`.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            // La caméra est partagée par les deux pipelines (même bind group).
            pass.set_bind_group(0, &self.camera_bind_group, &[]);

            if self.instance_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                // Un seul appel pour tous les cubes : c'est l'instanciation.
                pass.draw_indexed(0..self.index_count, 0, 0..self.instance_count);
                stats.triangles += (self.index_count / 3 * self.instance_count) as usize;
            }

            pass.set_pipeline(&self.chunk_pipeline);
            // `filter` saute les chunks hors champ : ils ne coûtent alors rien au GPU.
            let culling = self.frustum_culling;
            for mesh in self
                .meshes
                .values()
                .filter(|m| !culling || frustum.intersects(&m.bounds))
            {
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                // Indices en `u32` : un chunk peut dépasser les 65 536 sommets d'un `u16`.
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                stats.meshes_drawn += 1;
                stats.triangles += (mesh.index_count / 3) as usize;
            }

            // En dernier, par-dessus le décor.
            self.lines
                .draw(&mut pass, self.outline.is_some(), self.crosshair);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(Some(stats))
    }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cube instances"),
        size: (capacity * size_of::<CubeInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Crée un pipeline. Les deux pipelines du renderer ne diffèrent que par
/// leur shader et le format de leurs sommets ; le reste de l'état est commun.
fn create_pipeline(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    vertex_layouts: &[Option<wgpu::VertexBufferLayout<'_>>],
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(camera_layout)],
        ..Default::default()
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: vertex_layouts,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            // On ne dessine pas les faces arrière : moitié moins de travail.
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
