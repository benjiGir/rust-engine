//! Carte de profondeur locale du soleil. La projection suit la caméra avec un
//! pas d'un texel pour limiter le scintillement quand le joueur se déplace.

use glam::{Mat4, Vec3, Vec4};

use crate::{Camera, Sun};

pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const RESOLUTION: u32 = 2048;
const RADIUS: f32 = 76.0;
pub(crate) const DEPTH_SPAN: f32 = 384.0;

struct Target {
    view: wgpu::TextureView,
    binding: wgpu::BindGroup,
}

pub(crate) struct SunShadows {
    pub layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target: Target,
    enabled: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ShadowFrame {
    pub projection: Mat4,
    pub strength: f32,
    pub moon: bool,
}

impl SunShadows {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lecture des ombres solaires"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filtrage des ombres"),
            compare: Some(wgpu::CompareFunction::LessEqual),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let target = Target::new(device, &layout, &sampler, 1);
        Self {
            layout,
            sampler,
            target,
            enabled: false,
        }
    }

    pub fn set_enabled(&mut self, device: &wgpu::Device, enabled: bool) {
        if self.enabled != enabled {
            let size = if enabled { RESOLUTION } else { 1 };
            self.target = Target::new(device, &self.layout, &self.sampler, size);
            self.enabled = enabled;
        }
    }

    pub fn frame(
        &self,
        camera: &Camera,
        sun: Sun,
        moonlight: bool,
        overcast: f32,
    ) -> Option<ShadowFrame> {
        self.enabled
            .then(|| shadow_frame(camera, sun, moonlight, overcast))
            .flatten()
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.target.view
    }

    pub fn binding(&self) -> &wgpu::BindGroup {
        &self.target.binding
    }
}

impl Target {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("carte des ombres du soleil"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
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
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("carte des ombres du soleil"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Self { view, binding }
    }
}

fn shadow_frame(camera: &Camera, sun: Sun, moonlight: bool, overcast: f32) -> Option<ShadowFrame> {
    let solar = sun.direction.normalize_or_zero();
    let moon = moonlight && solar.y < -0.035;
    let (direction, strength) = if moon {
        (
            -solar,
            ((-solar.y - 0.035) / 0.24).clamp(0.0, 1.0) * (1.0 - overcast * 0.74),
        )
    } else {
        (
            solar,
            ((solar.y - 0.035) / 0.14).clamp(0.0, 1.0) * (sun.daylight * 3.0).min(1.0),
        )
    };
    if strength <= 0.0 {
        return None;
    }
    let right = Vec3::Z.cross(direction).normalize();
    let up = direction.cross(right);
    let center = camera.position + camera.forward_flat() * 16.0;
    let texel = 2.0 * RADIUS / RESOLUTION as f32;
    let align = |axis: Vec3| (center.dot(axis) / texel).round() * texel;
    // Les points vers le soleil ont la profondeur la plus petite. Cette
    // matrice produit directement les coordonnées clip [−1,1] × [−1,1] × [0,1].
    let projection = Mat4::from_cols(
        (right / RADIUS).extend(-align(right) / RADIUS),
        (up / RADIUS).extend(-align(up) / RADIUS),
        (-direction / DEPTH_SPAN).extend(0.5 + center.dot(direction) / DEPTH_SPAN),
        Vec4::W,
    )
    .transpose();
    Some(ShadowFrame {
        projection,
        strength,
        moon,
    })
}

pub(crate) fn depth_pipeline(
    device: &wgpu::Device,
    groups: &[Option<&wgpu::BindGroupLayout>],
    shader: &wgpu::ShaderModule,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> wgpu::RenderPipeline {
    let label = Some("profondeur du soleil");
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label,
        bind_group_layouts: groups,
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_sun_shadow"),
            buffers,
            compilation_options: Default::default(),
        },
        // Un fragment sert uniquement à découper les trous des plantes.
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_sun_shadow"),
            targets: &[],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: wgpu::DepthBiasState {
                constant: 1,
                slope_scale: 1.0,
                clamp: 0.0,
            },
        }),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_depth_points_toward_the_sun_and_stays_valid_at_noon() {
        let camera = Camera {
            position: Vec3::new(-32.0, 78.0, 17.0),
            ..Camera::default()
        };
        for direction in [Vec3::Y, Vec3::new(0.5, 0.2, 0.6).normalize()] {
            let frame = shadow_frame(
                &camera,
                Sun {
                    direction,
                    ..Sun::default()
                },
                false,
                0.0,
            )
            .unwrap();
            let center = camera.position + camera.forward_flat() * 16.0;
            let midpoint = frame.projection.project_point3(center);
            assert!(midpoint.truncate().length() < 0.002);
            assert!((midpoint.z - 0.5).abs() < 1e-4);
            let closer = frame.projection.project_point3(center + 15.0 * direction);
            assert!(closer.z < midpoint.z);
            assert!(closer.truncate().distance(midpoint.truncate()) < 1e-4);
        }
        let night = Sun {
            direction: Vec3::NEG_Y,
            ..Sun::default()
        };
        assert!(shadow_frame(&camera, night, false, 0.0).is_none());
        let lunar = shadow_frame(&camera, night, true, 0.0).unwrap();
        assert!(lunar.moon);
        let eye = lunar.projection.project_point3(camera.position).z;
        let above = lunar
            .projection
            .project_point3(camera.position + Vec3::Y * 10.0)
            .z;
        assert!(above < eye);
        let overcast = shadow_frame(&camera, night, true, 1.0).unwrap();
        assert!(overcast.strength < lunar.strength);
    }
}
