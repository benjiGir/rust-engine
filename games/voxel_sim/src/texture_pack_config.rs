//! Quel pack de textures le registre des blocs utilise (la simulation le configure au lancement ;
//! le choix du pack se fait dans `texture_packs.rs`, côté fenêtre).

use std::path::Path;

use engine_voxel::BlockRegistry;
use serde::Deserialize;

use crate::settings::TexturePack;

pub fn details(pack: TexturePack) -> Option<(&'static str, u32)> {
    match pack {
        TexturePack::Classic => None,
        TexturePack::Earth32 => Some(("earth-copper-32", 32)),
        TexturePack::Earth64 => Some(("earth-copper-64", 64)),
    }
}

#[derive(Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub resolution: u32,
    pub description: String,
}
pub fn manifest(folder: &Path) -> Option<Manifest> {
    ron::from_str(&std::fs::read_to_string(folder.join("pack.ron")).ok()?).ok()
}
pub fn available(root: &Path, pack: TexturePack) -> bool {
    details(pack).is_none_or(|(id, size)| {
        let folder = root.join("texture_packs").join(id);
        manifest(&folder).is_some_and(|p| p.id == id && p.resolution == size)
            && folder.join("assets/core/textures/grass_top.png").is_file()
    })
}
pub fn configure(registry: &mut BlockRegistry, pack: TexturePack, root: &Path) {
    let (path, size) = match details(pack) {
        Some((id, size)) if available(root, pack) => (
            Some(root.join("texture_packs").join(id).join("assets")),
            size,
        ),
        Some((id, _)) => {
            log::warn!("pack {id} absent ou invalide : textures classiques utilisées");
            (None, 16)
        }
        None => (None, 16),
    };
    if let Err(err) = registry.set_visual_pack(path, size) {
        log::warn!("pack graphique : {err}");
    }
}
