//! Le culling des grottes côté jeu : à chaque image, demander au graphe de visibilité
//! (`engine_voxel::VisibilityGraph`) quels chunks on voit, et ne donner que leurs
//! maillages au renderer. F6 ou `VOXEL_CAVE_CULLING=0` l'éteignent, pour comparer.

use engine_render::{Aabb, Frustum, MeshId};
use engine_voxel::{CHUNK_SIZE, VisibilityGraph, WORLD_HEIGHT_CHUNKS};
use glam::{IVec3, Vec3};

use crate::VoxelGame;

pub struct CaveCulling {
    pub enabled: bool,
    /// Les connexions de chaque chunk maillé, tenues à jour avec les maillages.
    pub graph: VisibilityGraph,
    /// Réutilisés d'une image à l'autre.
    chunks: Vec<IVec3>,
    ids: Vec<MeshId>,
}

impl CaveCulling {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("VOXEL_CAVE_CULLING").map_or(true, |v| v != "0"),
            graph: VisibilityGraph::default(),
            chunks: Vec::new(),
            ids: Vec::new(),
        }
    }
}

impl VoxelGame {
    /// Choisit les maillages à dessiner pour la caméra actuelle, vue avec ce rapport
    /// largeur / hauteur (celui de l'écran, ou d'une capture).
    pub(crate) fn update_visibility(&mut self, aspect: f32) {
        if !self.culling.enabled {
            self.renderer.set_visible_meshes(None);
            return;
        }
        let frustum = Frustum::from_view_proj(self.camera.view_projection(aspect));
        let size = CHUNK_SIZE as f32;
        let in_view = |chunk: IVec3| {
            let min = chunk.as_vec3() * size;
            frustum.intersects(&Aabb::new(min, min + Vec3::splat(size)))
        };
        let radius = self.streamer().config().view_radius + 1;
        let culling = &mut self.culling;
        culling.chunks.clear();
        culling.graph.visible(
            self.camera.position,
            radius,
            0..WORLD_HEIGHT_CHUNKS,
            in_view,
            &mut culling.chunks,
        );
        culling.ids.clear();
        culling
            .ids
            .extend(culling.chunks.iter().filter_map(|c| self.meshes.get(c)));
        self.renderer.set_visible_meshes(Some(&culling.ids));
    }

    /// F6 : allume ou éteint le culling des grottes.
    pub(crate) fn toggle_cave_culling(&mut self) {
        self.culling.enabled = !self.culling.enabled;
        let state = if self.culling.enabled {
            "activé"
        } else {
            "désactivé"
        };
        log::info!("culling des grottes {state}");
    }
}
