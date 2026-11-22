//! La part fenêtre de `ships.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `ships.rs`).

use crate::key_codes::KeyCodes;
use crate::*;
#[allow(unused_imports)]
use voxel_sim::vehicles::ships::*;

impl VoxelGame {
    /// La barre : gauche et droite, envoyées au serveur quand elles changent (voir `update_helm`).
    pub(crate) fn update_ships(&mut self, ctx: &Context) {
        let keys = &self.settings.keys;
        let steer = f32::from(ctx.input.key_held(keys.key(Action::Right)) as u8)
            - f32::from(ctx.input.key_held(keys.key(Action::Left)) as u8);
        if steer != self.client.steer_sent {
            self.client.steer_sent = steer;
            self.send_now(voxel_sim::net::protocol::ClientMsg::Steer(steer));
        }
    }
}

use crate::gpu_vertices;
use engine_render::{Aabb, MeshId, Renderer};
use engine_voxel::{CHUNK_SIZE, GridId, SubGrid};

/// Les maillages des structures mobiles sur le GPU.
#[derive(Default)]
pub struct GridMeshes {
    built: HashMap<GridId, Built>,
}

struct Built {
    revision: u64,
    /// Les chunks locaux : leur origine locale et leur maillage.
    chunks: Vec<(IVec3, MeshId)>,
}

impl GridMeshes {
    /// Tous les maillages (pour le culling : ils sont toujours dessinés).
    pub fn ids(&self) -> impl Iterator<Item = MeshId> + '_ {
        self.built
            .values()
            .flat_map(|b| b.chunks.iter().map(|c| c.1))
    }

    /// Met les maillages à jour : remaille ce qui a changé, retire ce qui a disparu, et place
    /// chaque maillage selon la transformation actuelle de sa structure.
    pub fn sync(&mut self, renderer: &mut Renderer, grids: &[&SubGrid]) {
        let live: Vec<GridId> = grids.iter().map(|grid| grid.id).collect();
        let gone: Vec<GridId> = self
            .built
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in gone {
            if let Some(built) = self.built.remove(&id) {
                for (_, mesh) in built.chunks {
                    renderer.remove_mesh(mesh);
                }
            }
        }
        for grid in grids {
            let id = grid.id;
            let stale = self
                .built
                .get(&id)
                .is_none_or(|b| b.revision != grid.revision());
            if stale {
                if let Some(old) = self.built.remove(&id) {
                    for (_, mesh) in old.chunks {
                        renderer.remove_mesh(mesh);
                    }
                }
                self.built.insert(id, build(renderer, grid));
            }
            if let Some(built) = self.built.get(&id) {
                for &(origin, mesh) in &built.chunks {
                    let (world_origin, bounds) = place(grid, origin);
                    renderer.set_mesh_transform(mesh, world_origin, grid.yaw, bounds);
                }
            }
        }
    }
}

/// Où est l'origine d'un chunk local dans le monde, et sa boîte englobante dans le monde.
fn place(grid: &SubGrid, chunk_origin: IVec3) -> (Vec3, Aabb) {
    let size = CHUNK_SIZE as f32;
    let base = chunk_origin.as_vec3();
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for corner in 0..8 {
        // Une pièce peut dépasser de son cube pendant sa rotation (au plus
        // sqrt(0.5) - 0.5 bloc). Garder cette marge aussi sur les structures mobiles.
        let offset = Vec3::new(
            (corner & 1) as f32,
            ((corner >> 1) & 1) as f32,
            ((corner >> 2) & 1) as f32,
        ) * (size + 0.5)
            - Vec3::splat(0.25);
        let p = grid.to_world(base + offset);
        min = min.min(p);
        max = max.max(p);
    }
    (grid.to_world(base), Aabb::new(min, max))
}

fn build(renderer: &mut Renderer, grid: &SubGrid) -> Built {
    let mut chunks = Vec::new();
    let mut positions: Vec<IVec3> = grid.world().chunk_positions().collect();
    positions.sort_by_key(|p| p.to_array());
    for pos in positions {
        let mesh = engine_voxel::mesh_binary(grid.world(), pos);
        let Some((min, max)) = mesh.bounds() else {
            continue;
        };
        let _ = (min, max);
        let (origin, bounds) = place(grid, mesh.origin);
        if let Some(id) = renderer.add_mesh_in_frame(
            gpu_vertices(&mesh.vertices),
            &mesh.indices,
            &mesh.translucent_indices,
            origin,
            mesh.origin.as_vec3(),
            bounds,
        ) {
            chunks.push((mesh.origin, id));
        }
    }
    Built {
        revision: grid.revision(),
        chunks,
    }
}
