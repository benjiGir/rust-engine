use engine_render::egui::{self, TextureId};
use std::collections::HashMap;

/// Petite rasterisation CPU des triangles egui : sert uniquement à vérifier cette planche.
pub fn rasterize(
    primitives: &[egui::ClippedPrimitive],
    textures: &HashMap<TextureId, egui::ColorImage>,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut rgba = [20, 28, 31, 255].repeat(width * height);
    for primitive in primitives {
        let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        let Some(texture) = textures.get(&mesh.texture_id) else {
            continue;
        };
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let v = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            let edge = |a: egui::Pos2, b: egui::Pos2, p: egui::Pos2| {
                (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
            };
            let area = edge(v[0].pos, v[1].pos, v[2].pos);
            if area.abs() < 1e-6 {
                continue;
            }
            let min = v[0]
                .pos
                .min(v[1].pos)
                .min(v[2].pos)
                .max(primitive.clip_rect.min)
                .max(egui::Pos2::ZERO);
            let max = v[0]
                .pos
                .max(v[1].pos)
                .max(v[2].pos)
                .min(primitive.clip_rect.max)
                .min(egui::pos2(width as f32, height as f32));
            for y in min.y.floor() as usize..max.y.ceil() as usize {
                for x in min.x.floor() as usize..max.x.ceil() as usize {
                    let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let weights = [
                        edge(v[1].pos, v[2].pos, p) / area,
                        edge(v[2].pos, v[0].pos, p) / area,
                        edge(v[0].pos, v[1].pos, p) / area,
                    ];
                    if weights.iter().any(|w| *w < -1e-5) {
                        continue;
                    }
                    let uv = v[0].uv.to_vec2() * weights[0]
                        + v[1].uv.to_vec2() * weights[1]
                        + v[2].uv.to_vec2() * weights[2];
                    let tx = ((uv.x * texture.width() as f32) as usize).min(texture.width() - 1);
                    let ty = ((uv.y * texture.height() as f32) as usize).min(texture.height() - 1);
                    let texel = texture.pixels[tx + ty * texture.width()].to_array();
                    let color: [f32; 4] = std::array::from_fn(|c| {
                        (0..3)
                            .map(|i| v[i].color.to_array()[c] as f32 * weights[i])
                            .sum::<f32>()
                            / 255.0
                    });
                    let alpha = color[3] * texel[3] as f32 / 255.0;
                    let index = (x + y * width) * 4;
                    for c in 0..3 {
                        rgba[index + c] = (color[c] * texel[c] as f32
                            + rgba[index + c] as f32 * (1.0 - alpha))
                            .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    rgba
}

/// Orthographique CPU des modèles utilisés par les piles ; même géométrie et matériaux.
pub fn model_image(
    parts: &[engine_render::CubeInstance],
    textures: &[Vec<u8>],
    direction: glam::Vec3,
    light_factor: f32,
) -> Vec<u8> {
    scene_image(parts, textures, direction, light_factor, 43.0)
}

/// Une scène élargie pour la juxtaposition et le contact avec un mur.
pub fn scene_image(
    parts: &[engine_render::CubeInstance],
    textures: &[Vec<u8>],
    direction: glam::Vec3,
    light_factor: f32,
    zoom: f32,
) -> Vec<u8> {
    use glam::Vec3;
    let mut rgba = vec![0; 64 * 64 * 4];
    let direction = direction.normalize();
    let right = Vec3::Y.cross(-direction).normalize();
    let up = right.cross(direction).normalize();
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let srgb = |c: f32| {
        let c = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    for y in 0..64 {
        for x in 0..64 {
            let origin = -direction * 3.0
                + right * ((x as f32 + 0.5 - 32.0) / zoom)
                + up * ((32.0 - y as f32 - 0.5) / zoom);
            let mut nearest = f32::INFINITY;
            for part in parts {
                let center = Vec3::from_array(part.center);
                let size = Vec3::from_array(part.size);
                let min = center - size * 0.5;
                let max = center + size * 0.5;
                let a = (min - origin) / direction;
                let b = (max - origin) / direction;
                let near = a.min(b);
                let far = a.max(b);
                let t = near.max_element();
                if t < 0.0 || t > far.min_element() || t >= nearest {
                    continue;
                }
                let p = (origin + direction * t - min) / size;
                let uv = Vec3::from_array(part.uv_min)
                    + p * (Vec3::from_array(part.uv_max) - Vec3::from_array(part.uv_min));
                let axis = if near.x >= near.y && near.x >= near.z {
                    0
                } else if near.y >= near.z {
                    1
                } else {
                    2
                };
                let (coords, face, light) = match axis {
                    1 => ([uv.x, uv.z], 0, 1.0),
                    0 => ([uv.z, 1.0 - uv.y], 1, 0.82),
                    _ => ([uv.x, 1.0 - uv.y], 1, 0.64),
                };
                let texel = part.textures[face]
                    .checked_sub(1)
                    .and_then(|i| textures.get(i as usize))
                    .map_or([255; 4], |pixels| {
                        let x = (coords[0].rem_euclid(1.0) * 16.0) as usize;
                        let y = (coords[1].rem_euclid(1.0) * 16.0) as usize;
                        let p = (x + y * 16) * 4;
                        [pixels[p], pixels[p + 1], pixels[p + 2], pixels[p + 3]]
                    });
                if texel[3] < 128 {
                    continue;
                }
                let at = (x + y * 64) * 4;
                for c in 0..3 {
                    rgba[at + c] = srgb(linear(texel[c]) * part.color[c] * light * light_factor);
                }
                rgba[at + 3] = (texel[3] as f32 * part.opacity) as u8;
                nearest = t;
            }
        }
    }
    rgba
}
