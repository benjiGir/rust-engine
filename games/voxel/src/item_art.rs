//! Miniatures isométriques des blocs, calculées une fois et mises en cache par l'UI.
//! Les boîtes du modèle restent visibles : une dalle ou un engrenage n'est pas un cube.

use engine_voxel::{BlockBox, BlockDef};
use glam::Vec3;

pub const ICON_SIZE: usize = 32;

pub fn block_icon(def: &BlockDef, textures: &[Vec<u8>]) -> Vec<u8> {
    let mut pixels = vec![0; ICON_SIZE * ICON_SIZE * 4];
    let boxes = if def.boxes.is_empty() {
        &[BlockBox::FULL][..]
    } else {
        &def.boxes
    };
    let direction = Vec3::new(-1.0, -0.82, -1.0).normalize();
    let right = Vec3::new(1.0, 0.0, -1.0).normalize();
    let up = right.cross(direction).normalize();
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let origin = Vec3::splat(0.5) - direction * 3.0
                + right * ((x as f32 + 0.5 - 16.0) / 17.0)
                + up * ((16.0 - y as f32 - 0.5) / 17.0);
            let mut nearest = f32::INFINITY;
            for b in boxes {
                let a = (b.min - origin) / direction;
                let z = (b.max - origin) / direction;
                let near = a.min(z);
                let far = a.max(z);
                let entry = near.max_element();
                if entry < 0.0 || entry > far.min_element() || entry >= nearest {
                    continue;
                }
                let p = origin + direction * entry;
                let axis = if near.x >= near.y && near.x >= near.z {
                    0
                } else if near.y >= near.z {
                    1
                } else {
                    2
                };
                let (uv, face, shade) = match axis {
                    1 => ([p.x, p.z], 0, 1.0),
                    0 => ([p.z, 1.0 - p.y], 1, 0.84),
                    _ => ([p.x, 1.0 - p.y], 1, 0.65),
                };
                let texture = def.face_textures[face] as usize;
                let texel = texture
                    .checked_sub(1)
                    .and_then(|i| textures.get(i))
                    .map(|image| {
                        let u = (uv[0].clamp(0.0, 0.999) * 16.0) as usize;
                        let v = (uv[1].clamp(0.0, 0.999) * 16.0) as usize;
                        let index = (u + v * 16) * 4;
                        [
                            image[index],
                            image[index + 1],
                            image[index + 2],
                            image[index + 3],
                        ]
                    })
                    .unwrap_or_else(|| {
                        let color = def.color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8);
                        [color[0], color[1], color[2], 255]
                    });
                if def.cutout && texel[3] < 128 {
                    continue;
                }
                let index = (x + y * ICON_SIZE) * 4;
                for c in 0..3 {
                    pixels[index + c] = (texel[c] as f32 * shade) as u8;
                }
                pixels[index + 3] = texel[3];
                nearest = entry;
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_show_the_top_and_sides_on_a_transparent_background() {
        let registry = engine_voxel::BlockRegistry::builtin();
        let mut def = registry.def(registry.id("core:stone").unwrap()).clone();
        def.face_textures = [1, 2, 2];
        let textures = vec![[255, 0, 0, 255].repeat(256), [0, 0, 255, 255].repeat(256)];
        let pixels = block_icon(&def, &textures);
        assert_eq!(pixels.len(), ICON_SIZE * ICON_SIZE * 4);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 200 && p[2] == 0)
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[2] > 100 && p[0] == 0)
        );
        assert_eq!(pixels[3], 0);
        let full = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .count();
        def.boxes = vec![BlockBox {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 0.5, 1.0),
        }];
        let slab = block_icon(&def, &textures);
        let half = slab
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .count();
        assert!(
            half > 0 && half < full,
            "la dalle doit montrer sa forme, pas un cube"
        );
    }
}
