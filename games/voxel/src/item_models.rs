//! Modèles des piles au sol : boîtes texturées des blocs ou sprites extrudés.
//! Les pixels voisins de même couleur sont réunis en bandes, une fois par objet.

use engine_render::CubeInstance;
use engine_voxel::{BlockBox, BlockRegistry, Icon, ItemId, ItemRegistry, Shape};
use glam::Vec3;

pub fn build(item: ItemId, items: &ItemRegistry, blocks: &BlockRegistry) -> Vec<CubeInstance> {
    let def = items.get(item);
    match &def.icon {
        Icon::Block(id) => {
            let block = blocks.def(*id);
            if matches!(block.shape, Shape::Cross | Shape::Flat) {
                return block.face_textures[1]
                    .checked_sub(1)
                    .and_then(|i| blocks.textures().get(i as usize))
                    .map_or_else(Vec::new, |key| sprite(key, "textures", blocks));
            }
            let boxes = if block.boxes.is_empty() {
                &[BlockBox::FULL][..]
            } else {
                &block.boxes
            };
            boxes
                .iter()
                .map(|b| {
                    CubeInstance::new(
                        (b.min + b.max) * 0.5 - Vec3::splat(0.5),
                        b.max - b.min,
                        if block.face_textures == [0; 3] {
                            Vec3::from_array(block.color)
                        } else {
                            Vec3::ONE
                        },
                    )
                    .with_surface(
                        block.face_textures,
                        b.min,
                        b.max,
                        block.cutout,
                        block.alpha,
                    )
                })
                .collect()
        }
        Icon::Texture(key) => sprite(key, "items", blocks),
    }
}

fn sprite(key: &str, folder: &str, blocks: &BlockRegistry) -> Vec<CubeInstance> {
    let Some(pixels) = blocks
        .asset_path(key, folder, "png")
        .and_then(|p| crate::assets::load_png(&p).ok())
    else {
        return Vec::new();
    };
    extrude(&pixels)
}

fn extrude(pixels: &[u8]) -> Vec<CubeInstance> {
    let mut parts = Vec::new();
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    for y in 0..16 {
        let mut x = 0;
        while x < 16 {
            let at = (x + y * 16) * 4;
            let pixel = &pixels[at..at + 4];
            if pixel[3] < 128 {
                x += 1;
                continue;
            }
            let start = x;
            x += 1;
            while x < 16 && pixels[(x + y * 16) * 4..(x + y * 16) * 4 + 4] == *pixel {
                x += 1;
            }
            parts.push(CubeInstance::new(
                Vec3::new(
                    (start + x) as f32 / 32.0 - 0.5,
                    0.5 - (y as f32 + 0.5) / 16.0,
                    0.0,
                ),
                Vec3::new((x - start) as f32 / 16.0, 1.0 / 16.0, 1.0 / 16.0),
                Vec3::new(linear(pixel[0]), linear(pixel[1]), linear(pixel[2])),
            ));
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprites_keep_holes_colours_and_thickness_and_merge_matching_pixels() {
        let mut pixels = vec![0; 16 * 16 * 4];
        for x in 2..6 {
            pixels[(x + 3 * 16) * 4..(x + 3 * 16) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        let parts = extrude(&pixels);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].size, [4.0 / 16.0, 1.0 / 16.0, 1.0 / 16.0]);
        assert_eq!(parts[0].color, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn block_models_preserve_faces_and_shape() {
        let registry = crate::test_registry();
        let items = ItemRegistry::builtin(&registry).unwrap();
        let grass = build(items.id("core:grass").unwrap(), &items, &registry);
        assert_eq!(grass.len(), 1);
        assert_eq!(
            grass[0].textures,
            registry
                .def(registry.id("core:grass").unwrap())
                .face_textures
        );
        assert_ne!(grass[0].textures[0], grass[0].textures[1]);
        let glass = build(items.id("core:glass").unwrap(), &items, &registry);
        assert!(glass[0].opacity < 1.0);
        let slab = build(items.id("example:ruby_slab").unwrap(), &items, &registry);
        assert_eq!(slab.len(), 1);
        assert_eq!(slab[0].size[1], 0.5);
        assert_eq!(slab[0].uv_max[1], 0.5);
    }
}
