//! Raccordement du rendu des transports aux mondes et aux structures mobiles.
use crate::VoxelGame;
use engine_render::CubeInstance;
use engine_voxel::GridId;
use glam::Vec3;
const MAX_INSTANCES: usize = 8192;
impl VoxelGame {
    pub(crate) fn draw_conduit_contents(&mut self, out: &mut Vec<CubeInstance>) {
        self.client
            .conduit_fx
            .ingest(self.server.mods.machines.take_visual_transfers());
        let streamer = self.client.view.replica();
        let Some(fluids) = streamer.world().fluids() else {
            return;
        };
        let limit = out.len() + MAX_INSTANCES;
        let start = out.len();
        self.client.conduit_fx.instances(
            GridId::WORLD,
            streamer,
            &self.server.content.items,
            fluids,
            self.client.camera.position,
            |p| p,
            0.0,
            out,
            limit,
        );
        let grids = self.server.mods.subgrids.borrow();
        for id in grids.ids() {
            let Some(cell) = grids.get(id) else {
                continue;
            };
            let grid = cell.borrow();
            self.client.conduit_fx.instances(
                id,
                &*grid,
                &self.server.content.items,
                fluids,
                self.client.camera.position,
                |p| grid.to_world(p),
                grid.yaw,
                out,
                limit,
            );
        }
        for part in &mut out[start..] {
            let pos = Vec3::from_array(part.center).floor().as_ivec3();
            let (mut sky, mut block) = streamer.world().light(pos);
            for d in engine_voxel::FACINGS {
                let (s, b) = streamer.world().light(pos + d);
                sky = sky.max(s);
                block = block.max(b);
            }
            part.light = [sky as f32 / 15.0, (block as f32 / 15.0).max(part.light[1])];
        }
    }
}
