//! Géométrie et articulations des créatures, communes au jeu et aux planches de contrôle.
use engine_voxel::BlockRegistry;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MobPart {
    pub offset: (f32, f32, f32),
    pub size: (f32, f32, f32),
    pub color: (f32, f32, f32),
    #[serde(default)]
    pub bone: Bone,
    /// Inclinaison de repos autour de l'axe latéral, en radians.
    #[serde(default)]
    pub pitch: f32,
    /// Texture optionnelle, qualifiée par l'espace de noms de la créature.
    #[serde(default)]
    pub texture: Option<String>,
    /// Les pixels transparents sont découpés, par exemple aux bords d'une marque du pelage.
    #[serde(default)]
    pub cutout: bool,
    #[serde(skip)]
    pub texture_layer: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Bone {
    #[default]
    Fixed,
    Leg,
    Arm,
    Head,
    /// Queue et nageoire caudale : pivot commun, oscillation latérale pendant la nage.
    Tail,
}

pub fn bind_textures(parts: &mut [MobPart], namespace: &str, blocks: &mut BlockRegistry) {
    for part in parts {
        part.texture_layer = part.texture.as_ref().map_or(0, |name| {
            let key = if name.contains(':') {
                name.clone()
            } else {
                format!("{namespace}:{name}")
            };
            blocks.register_texture(&key)
        });
    }
}
