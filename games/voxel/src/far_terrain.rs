//! Le terrain lointain dans le jeu : les tuiles construites par `engine_voxel::far` en tâche de
//! fond deviennent des maillages GPU, dessinés avec les chunks détaillés.
//!
//! Les changements arrivent par lots (une région change de niveau d'un coup) : le jeu les envoie
//! avec ce qui reste du budget d'envoi de l'image (voir `stream_upload.rs`), un lot entier à la fois,
//! du plus proche au plus lointain. Les maillages sont toujours candidats au dessin : le frustum du
//! renderer écarte ce qui est hors de vue. Le terrain lointain se fait discret pendant un gros
//! chargement de chunks proches : `update_far` règle son allocation d'après leur file d'attente.

use std::collections::HashMap;

use engine_render::{Aabb, MeshId};
use engine_voxel::{
    ChunkStreamer, FarConfig, FarEvent, FarStreamer, FarTile, TransferBudget, far_allowance,
};
use glam::Vec3;

use super::*;

pub struct FarTerrain {
    streamer: FarStreamer,
    meshes: HashMap<FarTile, MeshId>,
    threads: usize,
}

impl FarTerrain {
    /// `near_radius` : le rayon des chunks détaillés ; `far_radius` : jusqu'où on va.
    pub fn new(chunks: &ChunkStreamer, near_radius: i32, far_radius: i32) -> Self {
        // Peu de threads : le terrain lointain ne doit pas gêner les chunks (et il se met en veilleuse
        // pendant un gros chargement : voir `update_far`).
        let threads = 2;
        let streamer = FarStreamer::with_debug(
            chunks.generator_arc(),
            FarConfig {
                near_radius,
                far_radius,
            },
            threads,
            Some(chunks.edited_surfaces()),
            // `VOXEL_FAR_DEBUG=levels|sources` : teinte les tuiles pour voir niveaux et sources.
            engine_voxel::FarDebug::from_env(),
        );
        Self {
            streamer,
            meshes: HashMap::new(),
            threads,
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

impl FarTerrain {
    /// Le prochain lot de changements qui tient dans le budget (voir `FarStreamer::next_batch`).
    pub fn next_batch(&mut self, budget: &TransferBudget) -> Option<Vec<FarEvent>> {
        self.streamer.next_batch(budget)
    }

    /// Des lots attendent d'être envoyés.
    pub fn has_waiting(&self) -> bool {
        self.streamer.stats().waiting_batches > 0
    }

    /// Envoie un lot au GPU et note chaque envoi dans le budget. Renvoie le nombre de tuiles ajoutées.
    pub fn apply(
        &mut self,
        renderer: &mut Renderer,
        batch: Vec<FarEvent>,
        budget: &mut TransferBudget,
        largest: &mut usize,
    ) -> usize {
        let mut added = 0;
        for event in batch {
            let started = std::time::Instant::now();
            match event {
                FarEvent::Add(tile, mesh) => {
                    let (lo, hi) = mesh.bounds;
                    let bytes = mesh.byte_size();
                    let id = renderer.add_mesh_scaled(
                        gpu_vertices(&mesh.vertices),
                        &mesh.indices,
                        &mesh.translucent_indices,
                        mesh.origin,
                        mesh.origin,
                        mesh.scale,
                        Aabb::new(lo, hi),
                    );
                    if let Some(id) = id
                        && let Some(old) = self.meshes.insert(tile, id)
                    {
                        renderer.remove_mesh(old);
                    }
                    budget.record_upload(bytes, started.elapsed());
                    *largest = (*largest).max(bytes);
                    added += 1;
                }
                FarEvent::Remove(tile) => {
                    if let Some(id) = self.meshes.remove(&tile) {
                        renderer.remove_mesh(id);
                    }
                    budget.record_unload(started.elapsed());
                }
            }
        }
        added
    }
}

impl VoxelGame {
    /// À chaque image : règle l'allocation du terrain lointain d'après le travail des chunks proches
    /// (tous ses threads au calme, un seul pendant un chargement soutenu, aucun pendant un gros), puis
    /// met à jour les tuiles voulues autour de la caméra. L'envoi au GPU se fait avec celui des chunks
    /// (voir `stream_upload.rs`).
    pub(crate) fn update_far(&mut self, eye: Vec3) {
        let backlog = self.stream_stats().backlog();
        let Some(far) = &mut self.far else { return };
        far.streamer
            .set_allowance(far_allowance(backlog, far.threads));
        far.streamer.refresh(eye);
    }
}
