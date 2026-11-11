//! Le terrain lointain dans le jeu : les tuiles construites par `engine_voxel::far` en tâche de
//! fond deviennent des maillages GPU, dessinés avec les chunks détaillés.
//!
//! Les tuiles arrivent par petits groupes (une région change de niveau d'un coup) : on envoie
//! au GPU ce que `FarStreamer::update` rend, sans budget supplémentaire. Les maillages sont
//! toujours candidats au dessin : le frustum du renderer écarte ce qui est hors de vue.

use std::collections::HashMap;

use engine_render::{Aabb, MeshId};
use engine_voxel::{ChunkStreamer, FarConfig, FarEvent, FarStreamer, FarTile};
use glam::Vec3;

use super::*;

pub struct FarTerrain {
    streamer: FarStreamer,
    meshes: HashMap<FarTile, MeshId>,
}

impl FarTerrain {
    /// `near_radius` : le rayon des chunks détaillés ; `far_radius` : jusqu'où on va.
    pub fn new(chunks: &ChunkStreamer, near_radius: i32, far_radius: i32) -> Self {
        // Deux threads : le terrain lointain est peu coûteux et ne doit pas gêner les chunks.
        let streamer = FarStreamer::new(
            chunks.generator_arc(),
            FarConfig {
                near_radius,
                far_radius,
            },
            2,
        );
        Self {
            streamer,
            meshes: HashMap::new(),
        }
    }

    /// Les maillages, pour le culling des grottes (qui liste ceux à dessiner).
    pub fn ids(&self) -> impl Iterator<Item = MeshId> + '_ {
        self.meshes.values().copied()
    }

    pub fn stats(&self) -> engine_voxel::FarStats {
        self.streamer.stats()
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }
}

impl VoxelGame {
    /// À chaque image : met à jour les tuiles lointaines autour de la caméra.
    pub(crate) fn update_far(&mut self, eye: Vec3) {
        let Some(far) = &mut self.far else { return };
        for event in far.streamer.update(eye) {
            match event {
                FarEvent::Add(tile, mesh) => {
                    let (lo, hi) = mesh.bounds;
                    let id = self.renderer.add_mesh_scaled(
                        gpu_vertices(&mesh.vertices),
                        &mesh.indices,
                        &mesh.translucent_indices,
                        mesh.origin,
                        mesh.origin,
                        mesh.scale,
                        Aabb::new(lo, hi),
                    );
                    if let Some(id) = id
                        && let Some(old) = far.meshes.insert(tile, id)
                    {
                        self.renderer.remove_mesh(old);
                    }
                }
                FarEvent::Remove(tile) => {
                    if let Some(id) = far.meshes.remove(&tile) {
                        self.renderer.remove_mesh(id);
                    }
                }
            }
        }
    }
}
