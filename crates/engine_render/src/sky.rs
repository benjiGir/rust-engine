//! Les nuages sont intégrés à demi-résolution. Le ciel les compose à pleine
//! résolution dans les pixels laissés libres par le décor opaque.

use bytemuck::{Pod, Zeroable};

use crate::{Camera, Sun};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub overcast: f32,
    pub flash: f32,
    /// Secondes de météo simulée.
    pub time: f32,
    pub seed: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkyUniform {
    /// Les deux premiers vecteurs incluent les demi angles du champ de vision.
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    eye: [f32; 4],
    sun: [f32; 4],
    horizon: [f32; 4],
    weather: [f32; 3],
    seed: u32,
}

const CLOUD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct CloudTarget {
    size: [u32; 2],
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
}

pub(crate) struct SkyPass {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
    cloud_pipeline: wgpu::RenderPipeline,
    cloud_layout: wgpu::BindGroupLayout,
    cloud_sampler: wgpu::Sampler,
    clouds: Option<CloudTarget>,
}

impl SkyPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("sky.wgsl"));
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforme du ciel"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<SkyUniform>() as u64),
                },
                count: None,
            }],
        });
        let cloud_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image des nuages"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        let create_pipeline = |entry, target: wgpu::TextureFormat, layouts, depth_stencil| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: layouts,
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_sky"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(target.into())],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky_layouts = [Some(&uniform_layout), Some(&cloud_layout)];
        let cloud_layouts = [Some(&uniform_layout)];
        let pipeline = create_pipeline(
            "fs_sky",
            format,
            &sky_layouts[..],
            Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let cloud_pipeline = create_pipeline("fs_clouds", CLOUD_FORMAT, &cloud_layouts[..], None);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("paramètres du ciel"),
            size: size_of::<SkyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paramètres du ciel"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let cloud_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filtrage des nuages"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            uniform,
            binding,
            cloud_pipeline,
            cloud_layout,
            cloud_sampler,
            clouds: None,
        }
    }

    /// `horizon` garde la couleur de fond du monde (rgb) et l'immersion (w).
    pub(crate) fn prepare(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        aspect: f32,
        sun: Sun,
        weather: Sky,
        horizon: [f32; 4],
    ) {
        let forward = camera.forward();
        let right = camera.right();
        let half_height = (camera.fov_y * 0.5).tan();
        let uniform = SkyUniform {
            right: (right * (half_height * aspect)).extend(0.0).to_array(),
            up: (right.cross(forward) * half_height).extend(0.0).to_array(),
            forward: forward.extend(0.0).to_array(),
            eye: camera.position.extend(0.0).to_array(),
            sun: sun.direction.extend(0.0).to_array(),
            horizon,
            weather: [weather.overcast, weather.flash, weather.time],
            seed: weather.seed,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub(crate) fn encode_clouds(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let size = size.map(|side| side.div_ceil(2).max(1));
        if self
            .clouds
            .as_ref()
            .is_none_or(|target| target.size != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("nuages à demi-résolution"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: CLOUD_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("nuages à demi-résolution"),
                layout: &self.cloud_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.cloud_sampler),
                    },
                ],
            });
            self.clouds = Some(CloudTarget {
                size,
                view,
                binding,
            });
        }
        let target = self.clouds.as_ref().expect("cible de la passe des nuages");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("nuages volumétriques"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes,
            ..Default::default()
        });
        pass.set_pipeline(&self.cloud_pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let clouds = self.clouds.as_ref().expect("nuages calculés avant le ciel");
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_bind_group(1, &clouds.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}
