//! Chargement des assets décrits par le registre : les textures des blocs.

use engine_render::TEXTURE_SIZE;
use engine_voxel::BlockRegistry;

/// Une texture de remplacement, damier magenta et noir : bien visible, pour repérer
/// une texture manquante sans faire planter le jeu.
fn missing_texture() -> Vec<u8> {
    let size = TEXTURE_SIZE as usize;
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

/// Lit un PNG et le ramène à 16 × 16 pixels RGBA.
pub(crate) fn load_png(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    let image = image::open(path)?.to_rgba8();
    let image = if image.dimensions() == (TEXTURE_SIZE, TEXTURE_SIZE) {
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
    registry
        .textures()
        .iter()
        .map(|key| match registry.asset_path(key, "textures", "png") {
            Some(path) => load_png(&path).unwrap_or_else(|err| {
                log::warn!("texture « {key} » ({}) : {err:#}", path.display());
                missing_texture()
            }),
            None => {
                log::warn!("texture « {key} » : pas de dossier pour cet espace de noms");
                missing_texture()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_core_and_mod_textures_load() {
        let registry = crate::test_registry();
        let textures = load_textures(&registry);
        assert_eq!(textures.len(), registry.textures().len());
        for (key, pixels) in registry.textures().iter().zip(&textures) {
            assert_eq!(pixels.len(), 16 * 16 * 4);
            assert_ne!(pixels, &missing_texture(), "texture « {key} » manquante");
        }
    }
}
