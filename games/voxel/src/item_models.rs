//! Modèles des piles au sol : boîtes texturées des blocs ou sprites extrudés.
//! Les pixels voisins de même couleur sont réunis en bandes, une fois par objet.

use engine_render::CubeInstance;
use engine_voxel::{BlockBox, BlockRegistry, Icon, ItemId, ItemRegistry, Shape};
use glam::Vec3;

pub fn build(item: ItemId, items: &ItemRegistry, blocks: &BlockRegistry) -> Vec<CubeInstance> {
    let def = items.get(item);
    match &def.icon {
        Icon::Block(id) => engine_voxel::paired_block::display_parts(blocks, *id).map_or_else(
            || block_parts(blocks.def(*id), Vec3::ZERO, blocks),
            |parts| {
                parts
                    .into_iter()
                    .flat_map(|(id, offset)| block_parts(blocks.def(id), offset, blocks))
                    .collect()
            },
        ),
        Icon::Texture(key) => sprite(key, "items", blocks),
    }
}

fn block_parts(
    block: &engine_voxel::BlockDef,
    offset: Vec3,
    blocks: &BlockRegistry,
) -> Vec<CubeInstance> {
    if matches!(block.shape, Shape::Cross | Shape::Flat | Shape::Rail) {
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
        .enumerate()
        .map(|(i, b)| {
            let center = offset + (b.min + b.max) * 0.5 - Vec3::splat(0.5);
            let size = b.max - b.min;
            if block.glazing == Some(i) {
                let [r, g, b, alpha] = engine_voxel::door::GLAZING_COLOR;
                return CubeInstance::new(center, size, Vec3::new(r, g, b)).with_surface(
                    [0; 3],
                    Vec3::ZERO,
                    Vec3::ONE,
                    false,
                    alpha,
                );
            }
            CubeInstance::new(
                center,
                size,
                if block.face_textures == [0; 3] {
                    Vec3::from_array(block.color)
                } else {
                    Vec3::ONE
                },
            )
            .with_surface(block.face_textures, b.min, b.max, block.cutout, block.alpha)
            .with_ends(block.face_texture(2, false), block.face_texture(2, true))
        })
        .collect()
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
    let Some(side) = engine_render::texture_side(pixels).map(|s| s as usize) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    for y in 0..side {
        let mut x = 0;
        while x < side {
            let at = (x + y * side) * 4;
            let pixel = &pixels[at..at + 4];
            if pixel[3] < 128 {
                x += 1;
                continue;
            }
            let start = x;
            x += 1;
            while x < side && pixels[(x + y * side) * 4..(x + y * side) * 4 + 4] == *pixel {
                x += 1;
            }
            parts.push(CubeInstance::new(
                Vec3::new(
                    (start + x) as f32 / (2.0 * side as f32) - 0.5,
                    0.5 - (y as f32 + 0.5) / side as f32,
                    0.0,
                ),
                Vec3::new(
                    (x - start) as f32 / side as f32,
                    1.0 / side as f32,
                    1.0 / 16.0,
                ),
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
    fn the_32_pixel_sprite_uses_the_same_extent_and_depth_as_the_16_pixel_one() {
        let mut small = vec![0; 16 * 16 * 4];
        let mut large = vec![0; 32 * 32 * 4];
        for y in 3..7 {
            for x in 2..6 {
                small[(x + y * 16) * 4..(x + y * 16) * 4 + 4].copy_from_slice(&[160, 80, 40, 255]);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let at = ((x * 2 + dx) + (y * 2 + dy) * 32) * 4;
                        large[at..at + 4].copy_from_slice(&[160, 80, 40, 255]);
                    }
                }
            }
        }
        let bounds = |parts: Vec<CubeInstance>| {
            parts.into_iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(min, max), p| {
                    let c = Vec3::from_array(p.center);
                    let r = Vec3::from_array(p.size) * 0.5;
                    (min.min(c - r), max.max(c + r))
                },
            )
        };
        assert_eq!(bounds(extrude(&small)), bounds(extrude(&large)));
    }

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
