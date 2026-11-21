//! Le culling des grottes côté jeu : à chaque image, demander au graphe de visibilité
//! (`engine_voxel::VisibilityGraph`) quels chunks on voit, et ne donner que leurs
//! maillages au renderer. F6 ou `VOXEL_CAVE_CULLING=0` l'éteignent, pour comparer.

use engine_render::{Aabb, Frustum, MeshId};
use engine_voxel::{CHUNK_SIZE, VisibilityGraph};
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
        if !self.client.culling.enabled {
            self.client.renderer.set_visible_meshes(None);
            return;
        }
        let frustum = Frustum::from_view_proj(self.client.camera.view_projection(aspect));
        let size = CHUNK_SIZE as f32;
        let in_view = |chunk: IVec3| {
            let min = chunk.as_vec3() * size;
            frustum.intersects(&Aabb::new(min, min + Vec3::splat(size)))
        };
        let radius = self.view_streamer().config().view_radius + 1;
        let vertical = self.view_world().vertical();
        let culling = &mut self.client.culling;
        culling.chunks.clear();
        culling.graph.visible(
            self.client.camera.position,
            radius,
            vertical,
            in_view,
            &mut culling.chunks,
        );
        culling.ids.clear();
        culling.ids.extend(
            culling
                .chunks
                .iter()
                .filter_map(|c| self.client.meshes.get(c)),
        );
        // Les structures mobiles sont toujours dessinées (le frustum les écarte au besoin).
        culling.ids.extend(self.client.grid_meshes.ids());
        if let Some(far) = &self.client.far {
            culling.ids.extend(far.ids());
        }
        self.client.renderer.set_visible_meshes(Some(&culling.ids));
    }

    /// F6 : allume ou éteint le culling des grottes.
    pub(crate) fn toggle_cave_culling(&mut self) {
        self.client.culling.enabled = !self.client.culling.enabled;
        let state = if self.client.culling.enabled {
            "activé"
        } else {
            "désactivé"
        };
        log::info!("culling des grottes {state}");
    }
}
