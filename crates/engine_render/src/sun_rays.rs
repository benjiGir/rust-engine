//! Diffusion dans l'air entre la caméra et le premier obstacle. La résolution
//! intermédiaire est indépendante de la taille de la carte d'ombre.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const SOURCE: &str = include_str!("sun_rays.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    inverse_view_projection: [[f32; 4]; 4],
    shadow_projections: [[[f32; 4]; 4]; 2],
    eye: [f32; 4],
    light: [f32; 4],
    shadow_blend: [f32; 4],
}

pub(crate) struct RayFrame<'a> {
    pub size: [u32; 2],
    pub depth: &'a wgpu::TextureView,
    pub surface: &'a wgpu::TextureView,
    pub shadows: &'a wgpu::BindGroup,
    pub view_projection: Mat4,
    pub shadow_projections: [Mat4; 2],
    pub shadow_blend: f32,
    pub eye: Vec3,
    pub direction: Vec3,
    pub intensity: f32,
    pub moon: bool,
}

struct Target {
    size: [u32; 2],
    depth: wgpu::TextureView,
    view: wgpu::TextureView,
    scene: wgpu::BindGroup,
    image: wgpu::BindGroup,
}

pub(crate) struct SunRays {
    volumes: wgpu::RenderPipeline,
    compose: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    scene_layout: wgpu::BindGroupLayout,
    image_layout: wgpu::BindGroupLayout,
    target: Option<Target>,
}

impl SunRays {
    pub fn new(
        device: &wgpu::Device,
        shadow_layout: &wgpu::BindGroupLayout,
        surface: wgpu::TextureFormat,
    ) -> Self {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scène éclairée dans la brume"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<Uniform>() as u64),
                    },
                    count: None,
                },
                texture(1, wgpu::TextureSampleType::Depth),
            ],
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("diffusion à demi-résolution"),
            entries: &[texture(
                0,
                wgpu::TextureSampleType::Float { filterable: false },
            )],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("projection et lumière de la brume"),
            size: size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffusion lumineuse"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let pipeline = |entry, groups: &[Option<&wgpu::BindGroupLayout>], format, blend| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: groups,
                ..Default::default()
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_screen"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let volumes = pipeline(
            "fs_volume",
            &[Some(&scene_layout), Some(shadow_layout)],
            FORMAT,
            None,
        );
        let compose = pipeline(
            "fs_compose",
            &[Some(&scene_layout), None, Some(&image_layout)],
            surface,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        );
        Self {
            volumes,
            compose,
            uniform,
            scene_layout,
            image_layout,
            target: None,
        }
    }

    pub fn release(&mut self) {
        self.target = None;
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: RayFrame<'_>,
        timing: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let size = frame.size.map(|side| side.div_ceil(2).max(1));
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.size != size || target.depth != *frame.depth)
        {
            self.target = Some(self.target(device, size, frame.depth));
        }
        let image = self.target.as_ref().expect("cible de diffusion");
        let uniform = Uniform {
            inverse_view_projection: frame.view_projection.inverse().to_cols_array_2d(),
            shadow_projections: frame
                .shadow_projections
                .map(|matrix| matrix.to_cols_array_2d()),
            shadow_blend: [frame.shadow_blend, 0.0, 0.0, 0.0],
            eye: frame.eye.extend(frame.intensity).to_array(),
            light: frame.direction.extend(f32::from(frame.moon)).to_array(),
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("diffusion volumétrique"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &image.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.volumes);
            pass.set_bind_group(0, &image.scene, &[]);
            pass.set_bind_group(1, frame.shadows, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composition des rayons"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.surface,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timing,
                ..Default::default()
            });
            pass.set_pipeline(&self.compose);
            pass.set_bind_group(0, &image.scene, &[]);
            pass.set_bind_group(2, &image.image, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn target(&self, device: &wgpu::Device, size: [u32; 2], depth: &wgpu::TextureView) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rayons lumineux à demi-résolution"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("profondeur de la scène et lumière des astres"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        let image = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image intermédiaire des rayons lumineux"),
            layout: &self.image_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Target {
            size,
            depth: depth.clone(),
            view,
            scene,
            image,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_ray_passes_accept_the_scene_and_shadow_depths() {
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shadow = crate::sun_shadows::SunShadows::new(&device);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let size = [128, 77];
        let texture = |format, extra| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | extra,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let surface = texture(format, wgpu::TextureUsages::empty());
        let depth = texture(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let mut rays = SunRays::new(&device, &shadow.layout, format);
        let mut encoder = device.create_command_encoder(&Default::default());
        rays.encode(
            &device,
            &queue,
            &mut encoder,
            RayFrame {
                size,
                depth: &depth,
                surface: &surface,
                shadows: shadow.binding(),
                view_projection: Mat4::IDENTITY,
                shadow_projections: [Mat4::IDENTITY; 2],
                shadow_blend: 0.5,
                eye: Vec3::ZERO,
                direction: Vec3::Y,
                intensity: 1.0,
                moon: false,
            },
            None,
        );
        queue.submit([encoder.finish()]);
        assert_eq!(rays.target.as_ref().unwrap().size, [64, 39]);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }
}
