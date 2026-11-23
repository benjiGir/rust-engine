use std::time::Instant;

use engine_voxel::{BlockId, PlayerBody, blocks::bed, blocks::door, blocks::tall_plant};
use glam::{IVec3, Vec3};

use crate::{BlockPlaced, events, player};

impl crate::server::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    fn overlaps_door(&self, parts: [(IVec3, BlockId); 2]) -> bool {
        let occupied = |body: &PlayerBody| {
            let (min, max) = body.bounds();
            parts.iter().any(|&(pos, block)| {
                let origin = pos.as_vec3();
                self.registry
                    .def(block)
                    .boxes
                    .iter()
                    .any(|b| min.cmplt(origin + b.max).all() && max.cmpgt(origin + b.min).all())
            })
        };
        occupied(&player::body(&self.ecs, self.players.entity())) || self.any_mob_body(occupied)
    }

    pub fn use_door(&mut self, pos: IVec3) -> bool {
        // Un portillon (une forme raccordée) s'ouvre comme une porte.
        if self.use_gate(pos) {
            return true;
        }
        let block = self.streamer().world().block(pos);
        if !door::is_door(self.registry.def(block)) {
            return false;
        }
        let parts = door::toggled(self.streamer().world(), pos);
        let Some(parts) = parts else {
            return true;
        };
        if self.overlaps_door(parts) {
            self.toast("Le passage du battant est encombré");
            return true;
        }
        if self.streamer_mut().toggle_door(pos) {
            for (p, _) in parts {
                self.notify_edit(p);
            }
            self.sound_near(
                crate::net::player_view::Sound::Block {
                    block: block.0,
                    kind: crate::world::sound_ids::BlockSound::Place,
                },
                pos.as_vec3() + glam::Vec3::splat(0.5),
            );
            self.punch();
        }
        true
    }

    pub fn place_pair(&mut self, parts: [(IVec3, BlockId); 2]) -> Option<()> {
        let door = door::is_door(self.registry.def(parts[0].1));
        let occupied = if door {
            self.overlaps_door(parts)
        } else {
            let body = player::body(&self.ecs, self.players.entity());
            parts
                .iter()
                .any(|&(pos, _)| body.overlaps_block(pos) || self.mob_overlaps(pos))
        };
        if occupied {
            return None;
        }
        let start = Instant::now();
        if !self.streamer_mut().place_pair(parts) {
            return None;
        }
        self.edit_times.push(start.elapsed());
        let (pos, block) = parts[0];
        events::send(
            &self.ecs,
            BlockPlaced {
                pos,
                block,
                player: Some(self.players.current_id()),
            },
        );
        for (pos, _) in parts {
            self.after_edit(pos);
        }
        Some(())
    }

    pub fn paired_placement(
        &self,
        pos: IVec3,
        block: BlockId,
        look: Vec3,
    ) -> Option<[(IVec3, BlockId); 2]> {
        if self.registry.def(block).key == bed::ITEM {
            bed::placement(&self.registry, pos, look)
        } else if let Some(parts) = tall_plant::placement(&self.registry, block, pos) {
            Some(parts)
        } else {
            door::placement(&self.registry, block, pos, look)
        }
    }
}
