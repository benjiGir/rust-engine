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
/// Côté des textures de blocs, en pixels.
pub const TEXTURE_SIZE: u32 = 16;
/// 16, 8, 4, 2, 1 : cinq niveaux de mipmap.
const TEXTURE_MIPS: u32 = 5;
/// Le shader des chunks, intégré au binaire. Son chemin sur le disque permet au jeu
/// de le surveiller et de le recharger à chaud pendant le développement.
pub const CHUNK_SHADER_SOURCE: &str = include_str!("chunk_shader.wgsl");
pub const CHUNK_SHADER_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/chunk_shader.wgsl");

/// Une boîte à dessiner (les entités sont faites de boîtes colorées) : son centre,
/// sa taille, sa couleur, sa rotation autour de la verticale et la lumière qu'elle reçoit.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CubeInstance {
    pub center: [f32; 3],
    pub size: [f32; 3],
    pub color: [f32; 3],
    /// Rotation autour de l'axe vertical, en radians (même convention que la caméra).
    pub yaw: f32,
    /// Lumière du ciel et des blocs à cet endroit, de 0 à 1.
    pub light: [f32; 2],
}

impl CubeInstance {
    /// Une boîte de taille `size` centrée en `center`, en plein jour.
    pub fn new(center: Vec3, size: Vec3, color: Vec3) -> Self {
        Self {
            center: center.to_array(),
            size: size.to_array(),
            color: color.to_array(),
            yaw: 0.0,
            light: [1.0, 0.0],
        }
    }

    pub fn with_yaw(mut self, yaw: f32) -> Self {
        self.yaw = yaw;
        self
    }

    pub fn with_light(mut self, sky: f32, block: f32) -> Self {
        self.light = [sky, block];
        self
    }
}

/// Une boîte du bras ou de l'objet tenu, en coordonnées de vue (voir `hand.wgsl`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct HandInstance {
    /// La matrice de la boîte (position, rotation, taille), colonne par colonne. Elle
    /// s'applique à un cube unité centré sur l'origine.
    pub model: [[f32; 4]; 4],
    pub color: [f32; 4],
    /// 0 : couleur unie ; sinon la couche de texture (un bloc tenu en main).
    pub texture: u32,
}

impl HandInstance {
    pub fn new(model: glam::Mat4, color: [f32; 3], texture: u32) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color: [color[0], color[1], color[2], 1.0],
            texture,
        }
    }
}

/// Le tampon uniforme du bras.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct HandUniform {
    proj: [[f32; 4]; 4],
    light: [f32; 4],
    sun_color: [f32; 4],
}

/// L'interface egui d'une image, prête à dessiner (voir `egui::Context::tessellate`).
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    /// Les textures qu'egui ajoute, modifie ou libère (police, images...).
    pub textures: egui::TexturesDelta,
    /// Pixels physiques par point egui (2 sur un écran Retina).
    pub pixels_per_point: f32,
}

/// Une image dessinée hors écran par [`Renderer::capture`].
pub struct Capture {
    pub width: u32,
    pub height: u32,
    /// 4 octets par pixel (rouge, vert, bleu, alpha), ligne par ligne depuis le haut.
    pub rgba: Vec<u8>,
    pub stats: RenderStats,
}

/// Ce qui a été dessiné pendant la dernière image.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Maillages (chunks) dessinés, après frustum culling.
    pub meshes_drawn: usize,
    /// Maillages chargés au total.
    pub meshes_total: usize,
    /// Maillages dans le champ mais écartés, invisibles depuis la caméra (culling des
    /// grottes).
    pub meshes_hidden: usize,
    /// Triangles envoyés au GPU (maillages dessinés et cubes instanciés).
    pub triangles: usize,
    /// Temps du GPU sur la passe du décor, mesuré par le GPU (la dernière mesure
    /// arrivée, de quelques images plus tôt). `None` si le GPU ne sait pas mesurer.
    pub gpu_scene: Option<std::time::Duration>,
    /// Temps passé à attendre l'image suivante de l'écran (la VSync) : pas du travail.
    pub wait: std::time::Duration,
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
    /// L'origine du chunk, son lacet et la base des textures (32 octets), lus comme une
    /// instance par le shader.
    origin_buffer: wgpu::Buffer,
    /// Les indices opaques, puis les translucides, dans un seul tampon.
    index_buffer: wgpu::Buffer,
    opaque_count: u32,
    translucent_count: u32,
    bounds: Aabb,
}

/// Le soleil : direction (vers le soleil) et intensité de la lumière du jour,
/// de 0 (nuit noire) à 1 (plein midi). La lumière du ciel stockée dans les blocs
/// est multipliée par cette intensité : la nuit tombe sans rien recalculer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    pub direction: Vec3,
    pub daylight: f32,
    /// Teinte de la lumière du ciel : blanche à midi, orangée au coucher, bleutée la nuit.
    pub color: Vec3,
}

impl Default for Sun {
    fn default() -> Self {
        Self {
            direction: Vec3::new(0.4, 1.0, 0.3).normalize(),
            daylight: 1.0,
            color: Vec3::ONE,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    /// Position de la caméra (xyz) et début du brouillard (w).
    camera_fog_start: [f32; 4],
    /// Couleur du brouillard (rgb) et fin du brouillard (w).
    fog_color_end: [f32; 4],
    /// Direction du soleil (xyz) et intensité du jour (w).
    sun: [f32; 4],
    /// Teinte de la lumière du ciel (xyz).
    sun_color: [f32; 4],
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
    translucent_pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    camera_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    texture_bind_group: wgpu::BindGroup,
    /// Le moteur de rendu d'egui (l'interface : inventaire, inspecteur).
    egui: egui_wgpu::Renderer,
    egui_free: Vec<egui::TextureId>,
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
    /// Les seuls maillages à dessiner (`None` : tous), voir [`Renderer::set_visible_meshes`].
    visible_meshes: Option<std::collections::HashSet<MeshId>>,
    /// Le chronomètre du GPU (`None` sans `TIMESTAMP_QUERY`).
    gpu_timer: Option<crate::gpu_timer::GpuTimer>,
    /// `None` : pas de brouillard.
    pub fog: Option<Fog>,
    pub sun: Sun,
    /// Le temps en secondes, pour animer le vent dans les feuilles et les herbes.
    pub time: f32,
    hand_pipeline: wgpu::RenderPipeline,
    hand_buffer: wgpu::Buffer,
    hand_uniform: wgpu::Buffer,
    hand_bind_group: wgpu::BindGroup,
    hand_capacity: usize,
    hand_count: u32,
    /// La lumière (ciel, blocs) à la position du joueur, pour éclairer le bras.
    hand_light: [f32; 2],
    /// Le champ de vision du bras, en degrés : fixe, pour que le bras garde sa forme
    /// quel que soit le champ de vision choisi pour le monde.
    pub hand_fov: f32,
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
                // Les timestamps du GPU, s'il les a (voir `gpu_timer.rs`).
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                ..Default::default()
            })
            .await?;
        let gpu_timer = crate::gpu_timer::GpuTimer::new(&device, &queue);

        // On part de la configuration proposée par wgpu pour cette surface.
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface non supportée par cette carte graphique")?;
        // On préfère un format sRGB : le GPU convertit alors nos couleurs linéaires
        // pour l'écran, ce qui donne des dégradés corrects.
        let caps = surface.get_capabilities(&adapter);
        log::debug!("modes de présentation possibles : {:?}", caps.present_modes);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        // `AutoVsync` : synchronisé sur l'écran. `AutoNoVsync` pour mesurer les FPS max.
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // egui produit des couleurs déjà « encodées » pour l'écran. Dessinées dans une vue
        // sRGB, elles seraient encodées une seconde fois (couleurs délavées). On autorise
        // donc une seconde vue, sans conversion sRGB, de la même image.
        config.view_formats = vec![config.format.remove_srgb_suffix()];
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
                attributes: &wgpu::vertex_attr_array![
                    2 => Float32x3, 3 => Float32x3, 4 => Float32x3, 5 => Float32, 6 => Float32x2
                ],
            }),
        ];
        let pipeline = create_pipeline(
            &device,
            &[Some(&camera_layout)],
            format,
            &device.create_shader_module(wgpu::include_wgsl!("shader.wgsl")),
            &cube_layouts,
            "cubes",
            false,
        );

        // Les textures des blocs : un « tableau de textures » (une couche par texture)
        // et un échantillonneur, liés au groupe 1 des shaders de chunks.
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("block textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture_bind_group = create_texture_bind_group(&device, &queue, &texture_layout, &[]);
        let (chunk_pipeline, translucent_pipeline) = create_chunk_pipelines(
            &device,
            &camera_layout,
            &texture_layout,
            format,
            CHUNK_SHADER_SOURCE,
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
        let egui = egui_wgpu::Renderer::new(
            &device,
            format.remove_srgb_suffix(),
            egui_wgpu::RendererOptions::default(),
        );

        let instance_capacity = 1024;
        let instance_buffer = create_instance_buffer(&device, instance_capacity);

        // Le bras : son propre uniforme (une projection à lui), les textures des blocs
        // (pour un bloc tenu), et le même cube unité que les entités.
        let hand_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hand"),
            size: size_of::<HandUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hand_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hand"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: hand_uniform.as_entire_binding(),
            }],
        });
        let hand_layouts = [
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<HandInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Uint32
                ],
            }),
        ];
        let hand_pipeline = create_pipeline(
            &device,
            &[Some(&camera_layout), Some(&texture_layout)],
            format,
            &device.create_shader_module(wgpu::include_wgsl!("hand.wgsl")),
            &hand_layouts,
            "hand",
            false,
        );
        let hand_capacity = 512;
        let hand_buffer = create_hand_buffer(&device, hand_capacity);

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
            translucent_pipeline,
            format,
            camera_layout,
            texture_layout,
            texture_bind_group,
            egui,
            egui_free: Vec::new(),
            lines,
            outline: None,
            crosshair: false,
            meshes: HashMap::new(),
            next_mesh_id: 0,
            frustum_culling: true,
            visible_meshes: None,
            gpu_timer,
            fog: None,
            sun: Sun::default(),
            time: 0.0,
            hand_pipeline,
            hand_buffer,
            hand_uniform,
            hand_bind_group,
            hand_capacity,
            hand_count: 0,
            hand_light: [1.0, 0.0],
            hand_fov: 70.0,
            clear_color: wgpu::Color {
                r: 0.45,
                g: 0.65,
                b: 0.9,
                a: 1.0,
            },
        })
    }

    /// Remplace les textures des blocs. `layers[i]` devient la couche `i + 1`
    /// (la couche 0 est blanche : les blocs sans texture gardent leur couleur).
    /// Chaque couche : 16 × 16 pixels RGBA, 1 024 octets.
    pub fn set_block_textures(&mut self, layers: &[Vec<u8>]) {
        self.texture_bind_group =
            create_texture_bind_group(&self.device, &self.queue, &self.texture_layout, layers);
    }

    /// Recompile le shader des chunks à partir de `source`. En cas d'erreur (WGSL
    /// invalide), on garde l'ancien pipeline et on renvoie l'erreur : le jeu ne plante pas.
    ///
    /// Par défaut, wgpu panique sur une erreur de validation. Une « portée d'erreur »
    /// (`push_error_scope`) les capture à la place : on crée le shader et les pipelines,
    /// puis on regarde si une erreur est survenue.
    pub fn reload_chunk_shader(&mut self, source: &str) -> Result<()> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipelines = create_chunk_pipelines(
            &self.device,
            &self.camera_layout,
            &self.texture_layout,
            self.format,
            source,
        );
        if let Some(error) = pollster::block_on(scope.pop()) {
            anyhow::bail!("shader invalide, l'ancien est gardé :\n{error}");
        }
        (self.chunk_pipeline, self.translucent_pipeline) = pipelines;
        Ok(())
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

    /// Active ou coupe la synchronisation verticale (une image par rafraîchissement de
    /// l'écran, sans déchirure ; coupée, le jeu dessine aussi vite qu'il peut).
    pub fn set_vsync(&mut self, on: bool) {
        self.config.present_mode = if on {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        self.surface.configure(&self.device, &self.config);
        log::debug!("présentation : {:?}", self.config.present_mode);
    }

    /// Les boîtes du bras et de l'objet tenu, avec la lumière à la position du joueur.
    /// Une liste vide : pas de bras (menu, capture sans bras).
    pub fn set_hand(&mut self, parts: &[HandInstance], light: [f32; 2]) {
        if parts.len() > self.hand_capacity {
            self.hand_capacity = parts.len().next_power_of_two();
            self.hand_buffer = create_hand_buffer(&self.device, self.hand_capacity);
        }
        self.queue
            .write_buffer(&self.hand_buffer, 0, bytemuck::cast_slice(parts));
        self.hand_count = parts.len() as u32;
        self.hand_light = light;
    }

    /// Charge un maillage (un chunk) sur le GPU. Il sera dessiné à chaque image
    /// jusqu'à [`Renderer::remove_mesh`] ou [`Renderer::clear_meshes`].
    ///
    /// `origin` : la position dans le monde de l'origine du chunk, à laquelle les
    /// sommets sont relatifs. `bounds` sert au frustum culling : la boîte (dans le
    /// monde) doit contenir tous les sommets.
    /// Renvoie `None` pour un maillage vide (wgpu refuse les tampons vides).
    pub fn add_mesh(
        &mut self,
        vertices: &[ColorVertex],
        indices: &[u32],
        translucent_indices: &[u32],
        origin: Vec3,
        bounds: Aabb,
    ) -> Option<MeshId> {
        self.add_mesh_in_frame(
            vertices,
            indices,
            translucent_indices,
            origin,
            origin,
            bounds,
        )
    }

    /// Comme [`Renderer::add_mesh`] pour un maillage d'une **structure mobile** : `origin` est
    /// la position actuelle de l'origine du chunk dans le monde, `uv_base` son origine dans le
    /// repère de la structure (les textures y sont accrochées). On le déplace ensuite avec
    /// [`Renderer::set_mesh_transform`].
    pub fn add_mesh_in_frame(
        &mut self,
        vertices: &[ColorVertex],
        indices: &[u32],
        translucent_indices: &[u32],
        origin: Vec3,
        uv_base: Vec3,
        bounds: Aabb,
    ) -> Option<MeshId> {
        self.add_mesh_scaled(
            vertices,
            indices,
            translucent_indices,
            origin,
            uv_base,
            1.0,
            bounds,
        )
    }

    /// Un maillage dont les positions locales sont multipliées par `scale` : un terrain
    /// lointain dessine une cellule de 4, 8, 16... blocs avec les mêmes sommets compacts qu'un
    /// chunk (voir `engine_voxel::far`).
    #[allow(clippy::too_many_arguments)]
    pub fn add_mesh_scaled(
        &mut self,
        vertices: &[ColorVertex],
        indices: &[u32],
        translucent_indices: &[u32],
        origin: Vec3,
        uv_base: Vec3,
        scale: f32,
        bounds: Aabb,
    ) -> Option<MeshId> {
        if indices.is_empty() && translucent_indices.is_empty() {
            return None;
        }
        // Les deux listes à la suite : on dessinera chaque partie avec une plage d'indices.
        let all_indices: Vec<u32> = indices.iter().chain(translucent_indices).copied().collect();
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
                contents: bytemuck::cast_slice(&all_indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let origin_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("chunk origin"),
                contents: bytemuck::bytes_of(&[
                    origin.extend(0.0).to_array(),
                    uv_base.extend(scale).to_array(),
                ]),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let id = MeshId(self.next_mesh_id);
        self.next_mesh_id += 1;
        self.meshes.insert(
            id,
            GpuMesh {
                vertex_buffer,
                origin_buffer,
                index_buffer,
                opaque_count: indices.len() as u32,
                translucent_count: translucent_indices.len() as u32,
                bounds,
            },
        );
        Some(id)
    }

    /// Déplace un maillage de structure mobile : où est l'origine de son chunk dans le monde,
    /// son lacet (rotation autour de la verticale), et sa boîte englobante dans le monde (pour
    /// le frustum culling).
    pub fn set_mesh_transform(&mut self, id: MeshId, origin: Vec3, yaw: f32, bounds: Aabb) {
        let Some(mesh) = self.meshes.get_mut(&id) else {
            return;
        };
        mesh.bounds = bounds;
        let value = [origin.x, origin.y, origin.z, yaw];
        self.queue
            .write_buffer(&mesh.origin_buffer, 0, bytemuck::bytes_of(&value));
    }

    /// Retire un maillage ; ses tampons GPU sont libérés.
    pub fn remove_mesh(&mut self, id: MeshId) {
        self.meshes.remove(&id);
    }

    /// Ne dessiner que ces maillages (le jeu sait lesquels sont visibles, par exemple
    /// grâce au culling des grottes), ou tous avec `None`. Valable jusqu'au prochain appel.
    pub fn set_visible_meshes(&mut self, ids: Option<&[MeshId]>) {
        match ids {
            Some(ids) => {
                let set = self.visible_meshes.get_or_insert_with(Default::default);
                set.clear();
                set.extend(ids.iter().copied());
            }
            None => self.visible_meshes = None,
        }
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
    pub fn render(
        &mut self,
        camera: &Camera,
        mut ui: Option<UiFrame>,
    ) -> Result<Option<RenderStats>> {
        {
            let _span = engine_trace::span("interface : préparation");
            self.prepare_ui(&mut ui);
        }
        // Avec la VSync, attendre l'image suivante se fait ici : ce temps-là n'est pas
        // du travail, mais il apparaît dans la trace.
        let acquire = engine_trace::span("attente de l'écran");
        let acquire_start = std::time::Instant::now();
        // Un `enum` riche : chaque cas porte (ou non) une texture. `match` nous oblige
        // à traiter tous les cas, sinon le code ne compile pas.
        use wgpu::CurrentSurfaceTexture as Current;
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            Current::Success(frame) => (frame, false),
            Current::Suboptimal(frame) => (frame, true),
            // Fenêtre cachée ou GPU en retard : on saute simplement cette image.
            Current::Timeout => {
                log::debug!("image sautée : délai dépassé");
                return Ok(None);
            }
            Current::Occluded => {
                log::debug!("image sautée : fenêtre cachée");
                // Rien à dessiner : on laisse le processeur se reposer un peu au lieu de
                // reboucler des centaines de milliers de fois par seconde.
                std::thread::sleep(std::time::Duration::from_millis(16));
                return Ok(None);
            }
            // Surface périmée (fenêtre redimensionnée, écran changé) : on la reconfigure.
            Current::Outdated | Current::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(None);
            }
            Current::Validation => anyhow::bail!("erreur de validation de la surface"),
        };
        drop(acquire);
        let wait = acquire_start.elapsed();
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        });

        let size = [self.config.width, self.config.height];
        let mut stats = {
            let _span = engine_trace::span("dessin");
            self.draw_scene(&view, &ui_view, None, camera, size, ui)
        };
        stats.wait = wait;
        let _span = engine_trace::span("présentation");
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(Some(stats))
    }

    /// Dessine la scène dans `view` (l'écran ou une texture) et envoie les commandes.
    /// `&self` suffit : wgpu ne demande qu'une référence partagée pour envoyer des données
    /// (`write_buffer`) ou des commandes (`submit`) au GPU.
    fn draw_scene(
        &mut self,
        view: &wgpu::TextureView,
        ui_view: &wgpu::TextureView,
        depth_override: Option<&wgpu::TextureView>,
        camera: &Camera,
        size: [u32; 2],
        ui: Option<UiFrame>,
    ) -> RenderStats {
        let aspect = size[0] as f32 / size[1].max(1) as f32;
        // `depth_override` : la profondeur d'une capture ; sinon celle de l'écran.
        // On lit le champ directement (et pas via une méthode) : Rust voit alors que
        // `self.depth_view` (lu) et `self.egui` (modifié plus bas) sont deux champs
        // distincts, et accepte les deux emprunts en même temps.
        let depth_view = depth_override.unwrap_or(&self.depth_view);
        let view_proj = camera.view_projection(aspect);
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
            sun: self.sun.direction.extend(self.sun.daylight).to_array(),
            sun_color: self.sun.color.extend(self.time).to_array(),
        };
        let frustum = Frustum::from_view_proj(view_proj);
        if let Some(timer) = &mut self.gpu_timer {
            timer.collect(&self.device);
            timer.begin_frame();
        }
        let mut stats = RenderStats {
            meshes_total: self.meshes.len(),
            gpu_scene: self.gpu_timer.as_ref().and_then(|t| t.last()),
            ..RenderStats::default()
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        if self.hand_count > 0 {
            // Une projection à part : proche (0,05) pour ne jamais couper le bras.
            let proj = glam::camera::rh::proj::directx::perspective(
                self.hand_fov.to_radians(),
                aspect,
                0.05,
                10.0,
            );
            let hand = HandUniform {
                proj: proj.to_cols_array_2d(),
                light: [
                    self.hand_light[0],
                    self.hand_light[1],
                    self.sun.daylight,
                    0.0,
                ],
                sun_color: self.sun.color.extend(1.0).to_array(),
            };
            self.queue
                .write_buffer(&self.hand_uniform, 0, bytemuck::bytes_of(&hand));
        }
        // Les écritures de tampons partent avant les commandes de cette image.
        self.lines
            .upload(&self.queue, self.outline.as_ref(), aspect);

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
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.gpu_timer.as_ref().and_then(|t| t.pass_writes()),
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

            // `filter` saute les chunks hors champ : ils ne coûtent alors rien au GPU.
            let culling = self.frustum_culling;
            let allowed = self.visible_meshes.as_ref();
            let visible: Vec<&GpuMesh> = self
                .meshes
                .iter()
                .filter(|(_, m)| !culling || frustum.intersects(&m.bounds))
                // Dans le champ mais caché (derrière la roche) : compté à part.
                .filter(|(id, _)| {
                    let shown = allowed.is_none_or(|ids| ids.contains(id));
                    stats.meshes_hidden += usize::from(!shown);
                    shown
                })
                .map(|(_, m)| m)
                .collect();

            // Passe 1 : l'opaque, dans n'importe quel ordre (le tampon de profondeur trie).
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_pipeline(&self.chunk_pipeline);
            for mesh in &visible {
                stats.meshes_drawn += 1;
                stats.triangles += ((mesh.opaque_count + mesh.translucent_count) / 3) as usize;
                if mesh.opaque_count == 0 {
                    continue;
                }
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, mesh.origin_buffer.slice(..));
                // Indices en `u32` : un chunk peut dépasser les 65 536 sommets d'un `u16`.
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.opaque_count, 0, 0..1);
            }

            // Passe 2 : le translucide. Le mélange alpha dépend de l'ordre : un objet
            // translucide doit être dessiné après ce qu'il y a derrière lui. On trie donc
            // les chunks du plus loin au plus proche (un tri grossier, par chunk et pas
            // par face : suffisant pour de l'eau et du verre).
            let mut translucent: Vec<(f32, &GpuMesh)> = visible
                .iter()
                .filter(|m| m.translucent_count > 0)
                .map(|m| {
                    let centre = (m.bounds.min + m.bounds.max) / 2.0;
                    (centre.distance_squared(camera.position), *m)
                })
                .collect();
            // `total_cmp` : une comparaison complète des flottants (gère NaN), à l'envers.
            translucent.sort_by(|a, b| b.0.total_cmp(&a.0));
            pass.set_pipeline(&self.translucent_pipeline);
            for (_, mesh) in translucent {
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, mesh.origin_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                let start = mesh.opaque_count;
                pass.draw_indexed(start..start + mesh.translucent_count, 0, 0..1);
            }

            // En dernier, par-dessus le décor.
            self.lines
                .draw(&mut pass, self.outline.is_some(), self.crosshair);
        }

        if let Some(timer) = &self.gpu_timer {
            timer.resolve(&mut encoder);
        }

        // Le bras, dans sa propre passe : la profondeur est effacée avant, il passe donc
        // devant tout le décor et ne traverse jamais un mur trop proche.
        if self.hand_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.hand_pipeline);
            pass.set_bind_group(0, &self.hand_bind_group, &[]);
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, self.hand_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..self.hand_count);
        }

        // L'interface egui, dans sa propre passe, par-dessus tout (sans profondeur).
        if let Some(ui) = &ui {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: size,
                pixels_per_point: ui.pixels_per_point,
            };
            let extra = self.egui.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &ui.primitives,
                &screen,
            );
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ui_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // `Load` : on garde l'image de la scène et on dessine par-dessus.
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            // egui demande une passe `'static` : `forget_lifetime` la détache de
            // l'encodeur (wgpu vérifie alors à l'exécution qu'on la termine avant lui).
            let mut pass = pass.forget_lifetime();
            self.egui.render(&mut pass, &ui.primitives, &screen);
            drop(pass);
            self.queue
                .submit(extra.into_iter().chain([encoder.finish()]));
        } else {
            self.queue.submit([encoder.finish()]);
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.after_submit();
        }
        // Les textures qu'egui n'utilise plus : libérées après une image dessinée.
        for id in std::mem::take(&mut self.egui_free) {
            self.egui.free_texture(&id);
        }
        stats
    }

    /// Envoie au GPU les textures nouvelles ou modifiées d'egui (la police, les icônes).
    /// Fait dès le début de l'image, même si elle est ensuite sautée : sinon une texture
    /// créée pendant une image sautée serait perdue (et egui panique si on oublie ses
    /// modifications).
    fn prepare_ui(&mut self, ui: &mut Option<UiFrame>) {
        let Some(ui) = ui else { return };
        let textures = std::mem::take(&mut ui.textures);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.egui
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        self.egui_free.extend(textures.free.iter().copied());
    }

    /// Dessine une image hors de l'écran et renvoie ses pixels.
    ///
    /// Utile pour des captures d'écran, même quand la fenêtre est cachée : on dessine
    /// dans une texture à nous, puis on la recopie dans un tampon que le processeur
    /// peut lire. `width` est arrondie à un multiple de 64 : wgpu exige que chaque
    /// ligne copiée fasse un multiple de 256 octets (64 pixels × 4 octets).
    pub fn capture(
        &mut self,
        camera: &Camera,
        width: u32,
        height: u32,
        mut ui: Option<UiFrame>,
    ) -> Result<Capture> {
        self.prepare_ui(&mut ui);
        let width = width.div_ceil(64).max(1) * 64;
        let height = height.max(1);
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Le même format que l'écran : nos pipelines sont créés pour lui.
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[self.config.format.remove_srgb_suffix()],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ui_view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        });
        let depth = create_depth_texture_view(&self.device, width, height);
        let stats = self.draw_scene(&view, &ui_view, Some(&depth), camera, [width, height], ui);

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: (width * height * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("capture copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);

        // Lire un tampon GPU est asynchrone : on demande la lecture, puis on attend
        // que le GPU ait fini (`poll`). Le résultat arrive par un petit canal.
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv()??;
        let mut rgba = slice.get_mapped_range()?.to_vec();
        buffer.unmap();
        // L'écran du Mac stocke souvent les pixels en BGRA : on remet dans l'ordre RGBA.
        if matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        Ok(Capture {
            width,
            height,
            rgba,
            stats,
        })
    }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    create_depth_texture_view(device, config.width, config.height)
}

fn create_depth_texture_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width,
            height,
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

fn create_hand_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hand instances"),
        size: (capacity * size_of::<HandInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
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
/// Les deux pipelines des chunks (opaque et translucide), à partir du source WGSL.
fn create_chunk_pipelines(
    device: &wgpu::Device,
    camera_layout: &wgpu::BindGroupLayout,
    texture_layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    source: &str,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    // Un sommet compact (voir `ColorVertex`) : position (u16), normale (i8 → -1..1),
    // couleur et lumière (u8 → 0..1), couche de texture. Le GPU convertit lui-même les
    // formats `Snorm` et `Unorm`.
    let layouts = [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<ColorVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Uint16x4, 1 => Snorm8x4, 2 => Unorm8x4, 3 => Unorm8x4, 4 => Uint32
            ],
        }),
        // L'origine du chunk, son lacet et la base des textures : une seule valeur pour tout
        // le maillage (une « instance »).
        Some(wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4],
        }),
    ];
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("chunk shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let groups = [Some(camera_layout), Some(texture_layout)];
    let opaque = create_pipeline(device, &groups, format, &shader, &layouts, "chunks", false);
    // Même shader, mais avec mélange alpha et sans écriture de profondeur.
    let translucent = create_pipeline(
        device,
        &groups,
        format,
        &shader,
        &layouts,
        "translucent chunks",
        true,
    );
    (opaque, translucent)
}

/// Crée le tableau de textures (couche 0 blanche, puis `layers`) avec ses mipmaps.
fn create_texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    layers: &[Vec<u8>],
) -> wgpu::BindGroup {
    let pixels = (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize;
    let white = vec![255u8; pixels];
    let all: Vec<&[u8]> = std::iter::once(white.as_slice())
        .chain(layers.iter().map(|l| l.as_slice()))
        .collect();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block textures"),
        size: wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: all.len() as u32,
        },
        mip_level_count: TEXTURE_MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // sRGB : les PNG sont en sRGB, le GPU les reconvertit en linéaire à la lecture.
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, data) in all.iter().enumerate() {
        // Une image de la mauvaise taille est remplacée par du blanc plutôt que de planter.
        let mut level: Vec<u8> = if data.len() == pixels {
            data.to_vec()
        } else {
            white.clone()
        };
        let mut size = TEXTURE_SIZE;
        for mip in 0..TEXTURE_MIPS {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &level,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
            level = downsample(&level, size);
            size /= 2;
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("block sampler"),
        // `Repeat` : la texture se répète sur les grands quads du greedy.
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        // `Nearest` : des pixels nets, le style « pixel art ». Les mipmaps évitent le
        // scintillement au loin ; on les mélange en douceur (`Linear`).
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("block textures"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    })
}

/// Le niveau de mipmap suivant : chaque pixel est la moyenne d'un carré de 2 × 2.
fn downsample(pixels: &[u8], size: u32) -> Vec<u8> {
    let half = (size / 2).max(1);
    let mut out = vec![0u8; (half * half * 4) as usize];
    for y in 0..half {
        for x in 0..half {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(size - 1);
                    let sy = (y * 2 + dy).min(size - 1);
                    sum += pixels[((sx + sy * size) * 4 + c) as usize] as u32;
                }
                out[((x + y * half) * 4 + c) as usize] = (sum / 4) as u8;
            }
        }
    }
    out
}

fn create_pipeline(
    device: &wgpu::Device,
    bind_group_layouts: &[Option<&wgpu::BindGroupLayout>],
    format: wgpu::TextureFormat,
    shader: &wgpu::ShaderModule,
    vertex_layouts: &[Option<wgpu::VertexBufferLayout<'_>>],
    label: &str,
    translucent: bool,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts,
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
                // Translucide : couleur finale = alpha × la nôtre + (1 - alpha) × celle derrière.
                blend: translucent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            // On ne dessine pas les faces arrière : moitié moins de travail. Sauf pour le
            // translucide : sous l'eau, on doit voir la surface par en dessous.
            cull_mode: (!translucent).then_some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            // Le translucide lit la profondeur (il est caché derrière un mur) mais ne
            // l'écrit pas : sinon une vitre cacherait l'eau derrière elle.
            depth_write_enabled: Some(!translucent),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
