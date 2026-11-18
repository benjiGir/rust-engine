use glam::Vec3;

use super::*;

const WIDTH: u32 = 512;
const HEIGHT: u32 = 320;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[test]
#[ignore = "nécessite un adaptateur GPU ; vérifie le vrai pipeline du ciel hors fenêtre"]
fn sky_renders_bodies_weather_and_respects_scene_depth() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("Ciel testé sur {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut sky = SkyPass::new(&device, FORMAT, DEPTH);
    let size = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let target = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let image = target(
        FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = target(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let pixels = image.create_view(&Default::default());
    let depth = depth.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(WIDTH * HEIGHT * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let camera = Camera {
        position: Vec3::new(0.0, 76.0, 0.0),
        pitch: 0.45,
        ..Camera::default()
    };
    let above_clouds = Camera {
        position: Vec3::new(0.0, 305.0, 0.0),
        ..camera
    };
    let clear = Sky {
        overcast: 0.0,
        flash: 0.0,
        time: 0.0,
        seed: 7,
    };
    let mut draw = |camera: Camera, direction, weather, horizon, clear_depth| {
        sky.prepare(
            &queue,
            &camera,
            WIDTH as f32 / HEIGHT as f32,
            Sun {
                direction,
                ..Sun::default()
            },
            weather,
            horizon,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        sky.encode_clouds(&device, &mut encoder, [WIDTH, HEIGHT], None);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pixels,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_depth),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            sky.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            image.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(WIDTH * 4),
                    rows_per_image: Some(HEIGHT),
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (send, receive) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let rgba = slice.get_mapped_range().unwrap().to_vec();
        readback.unmap();
        rgba
    };

    let day_horizon = [0.40, 0.62, 0.78, 0.0];
    let day = draw(camera, camera.forward(), clear, day_horizon, 1.0);
    let covered = draw(
        camera,
        camera.forward(),
        Sky {
            overcast: 1.0,
            ..clear
        },
        day_horizon,
        1.0,
    );
    let drift = draw(
        camera,
        camera.forward(),
        Sky {
            time: 37.0,
            ..clear
        },
        day_horizon,
        1.0,
    );
    let night_horizon = [0.008, 0.012, 0.035, 0.0];
    let sun = draw(above_clouds, camera.forward(), clear, day_horizon, 1.0);
    let night_camera = Camera {
        pitch: 0.70,
        ..above_clouds
    };
    let night = draw(
        night_camera,
        -night_camera.forward(),
        clear,
        night_horizon,
        1.0,
    );
    let hidden = draw(
        night_camera,
        -night_camera.forward(),
        clear,
        night_horizon,
        0.5,
    );
    let center = ((WIDTH * (HEIGHT / 2) + WIDTH / 2) * 4) as usize;
    let green_sum = |image: &[u8]| {
        image
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| u64::from(p[1]))
            .sum::<u64>()
    };
    assert!(
        sun[center] > 240,
        "disque solaire lumineux : {}",
        sun[center]
    );
    assert!(sun[center + 6 * 4] > 235, "disque solaire distinct du halo");
    assert!(
        green_sum(&day) > green_sum(&covered),
        "assombrissement à l'orage"
    );
    assert!(
        night[center].saturating_sub(night[center - 32 * 4]) > 35,
        "disque lunaire lisible"
    );
    assert!(night[center - 8 * 4].saturating_sub(night[center - 32 * 4]) > 35);
    assert_ne!(day, drift, "les nuages dérivent avec le temps");
    assert!(
        hidden
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0, 0, 0, 255])
    );
}
