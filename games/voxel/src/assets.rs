//! Chargement des assets décrits par le registre : les textures des blocs.

use engine_render::TEXTURE_SIZE;
use engine_voxel::BlockRegistry;

/// Une texture de remplacement, damier magenta et noir : bien visible, pour repérer
/// une texture manquante sans faire planter le jeu.
fn missing_texture_at(size: u32) -> Vec<u8> {
    let size = size as usize;
    (0..size * size)
        .flat_map(|i| {
            let (x, y) = (i % size, i / size);
            if (x / 4 + y / 4) % 2 == 0 {
                [255, 0, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        })
        .collect()
}

/// Lit un PNG RGBA, en conservant sa résolution 16 × 16, 32 × 32 ou 64 × 64.
pub(crate) fn load_png(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let image = image::open(path)?.to_rgba8();
    let image = if matches!(image.dimensions(), (16, 16) | (32, 32) | (64, 64)) {
        image
    } else {
        log::warn!(
            "{} fait {:?} pixels, redimensionnée en {TEXTURE_SIZE} × {TEXTURE_SIZE}",
            path.display(),
            image.dimensions()
        );
        // `Nearest` : on garde l'aspect pixel art, sans flou.
        image::imageops::resize(
            &image,
            TEXTURE_SIZE,
            TEXTURE_SIZE,
            image::imageops::FilterType::Nearest,
        )
    };
    Ok(image.into_raw())
}

/// Les pixels de chaque texture du registre, dans l'ordre de ses couches.
pub fn load_textures(registry: &BlockRegistry) -> Vec<Vec<u8>> {
    let side = registry.texture_size();
    registry
        .textures()
        .iter()
        .map(|key| match registry.asset_path(key, "textures", "png") {
            Some(path) => load_png_at(&path, side).unwrap_or_else(|err| {
                log::warn!("texture « {key} » ({}) : {err:#}", path.display());
                missing_texture_at(side)
            }),
            None => {
                log::warn!("texture « {key} » : pas de dossier pour cet espace de noms");
                missing_texture_at(side)
            }
        })
        .collect()
}

/// Les couches GPU ont toutes la résolution choisie ; les ressources absentes du pack
/// gardent leurs dessins d'origine, agrandis au plus proche voisin.
pub(crate) fn load_png_at(path: &std::path::Path, side: u32) -> anyhow::Result<Vec<u8>> {
    let pixels = load_png(path)?;
    let original = engine_render::texture_side(&pixels).unwrap();
    if original == side {
        return Ok(pixels);
    }
    let image = image::RgbaImage::from_raw(original, original, pixels).unwrap();
    Ok(
        image::imageops::resize(&image, side, side, image::imageops::FilterType::Nearest)
            .into_raw(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_64_pack_loads_native_blocks_plants_and_items() {
        use crate::settings::TexturePack;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut registry = crate::test_registry();
        crate::texture_packs::configure(
            &mut registry,
            TexturePack::Earth64,
            &[root.join("texture_packs")],
        );
        let pack = root.join("texture_packs/earth-copper-64");
        assert_eq!(registry.texture_size(), 64);
        for (key, kind) in [("core:stone", "textures"), ("core:iron_axe", "items")] {
            let path = registry.asset_path(key, kind, "png").unwrap();
            assert!(path.starts_with(&pack));
            assert_eq!(load_png(&path).unwrap().len(), 64 * 64 * 4);
        }
        for key in ["core:cactus_side", "core:tall_grass", "core:poppy"] {
            let path = registry.asset_path(key, "textures", "png").unwrap();
            assert!(path.starts_with(&pack));
            assert_eq!(load_png(&path).unwrap().len(), 64 * 64 * 4);
        }
        assert!(
            load_textures(&registry)
                .iter()
                .all(|p| p.len() == 64 * 64 * 4)
        );
    }

    #[test]
    fn the_32_pack_keeps_native_layers_and_missing_overrides_fall_back() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut registry = crate::test_registry();
        registry
            .set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)
            .unwrap();
        let packed = load_textures(&registry);
        assert!(
            packed
                .iter()
                .all(|p| p.len() == 32 * 32 * 4 && *p != missing_texture_at(32))
        );
        let temporary =
            std::env::temp_dir().join(format!("voxel-pack-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&temporary).unwrap();
        registry
            .set_visual_pack(Some(temporary.clone()), 32)
            .unwrap();
        let fallback = load_textures(&registry);
        assert!(
            fallback
                .iter()
                .all(|p| p.len() == 32 * 32 * 4 && *p != missing_texture_at(32))
        );
        registry.set_visual_pack(None, 16).unwrap();
        let base = load_textures(&registry);
        for (small, large) in base.iter().zip(fallback) {
            for y in 0..32 {
                for x in 0..32 {
                    assert_eq!(
                        &large[(x + y * 32) * 4..(x + y * 32) * 4 + 4],
                        &small[(x / 2 + y / 2 * 16) * 4..(x / 2 + y / 2 * 16) * 4 + 4]
                    );
                }
            }
        }
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn all_core_and_mod_textures_load() {
        let registry = crate::test_registry();
        let textures = load_textures(&registry);
        assert_eq!(textures.len(), registry.textures().len());
        for (key, pixels) in registry.textures().iter().zip(&textures) {
            assert_eq!(pixels.len(), 16 * 16 * 4);
            assert_ne!(
                pixels,
                &missing_texture_at(16),
                "texture « {key} » manquante"
            );
        }
    }
}
