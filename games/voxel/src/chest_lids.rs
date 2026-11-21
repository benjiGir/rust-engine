//! Le rendu des coffres suit les transitions du menu sans changer l'état des blocs.

use glam::IVec3;

use crate::{VoxelGame, audio, ui::Screen};

const OPEN_ANGLE: f32 = 1.45;
const OPEN_TIME: f32 = 0.28;
const CLOSE_TIME: f32 = 0.24;
const MAX_LIDS: usize = 4;

struct Transition {
    pos: IVec3,
    start: f32,
    goal: f32,
    since: f32,
}

impl Transition {
    fn angle(&self, now: f32) -> f32 {
        let duration = if self.goal > 0.0 {
            OPEN_TIME
        } else {
            CLOSE_TIME
        };
        let t = ((now - self.since) / duration).clamp(0.0, 1.0);
        self.start + (self.goal - self.start) * (t * t * (3.0 - 2.0 * t))
    }
}

#[derive(Default)]
pub(crate) struct ChestLids {
    /// Le coffre ouvert : une position, ou les deux moitiés d'un grand coffre.
    open: Vec<IVec3>,
    transitions: Vec<Transition>,
}

impl ChestLids {
    fn retarget(&mut self, pos: IVec3, goal: f32, now: f32) {
        let start = self
            .transitions
            .iter()
            .position(|lid| lid.pos == pos)
            .map_or(0.0, |index| self.transitions.remove(index).angle(now));
        if self.transitions.len() == MAX_LIDS {
            self.transitions.remove(0);
        }
        self.transitions.push(Transition {
            pos,
            start,
            goal,
            since: now,
        });
    }

    fn update(&mut self, target: &[IVec3], now: f32) -> (bool, bool) {
        self.transitions
            .retain(|lid| lid.goal != 0.0 || lid.angle(now) > 0.0);
        if self.open == target {
            return (false, false);
        }
        let closing = std::mem::take(&mut self.open);
        for &pos in &closing {
            self.retarget(pos, 0.0, now);
        }
        for &pos in target {
            self.retarget(pos, OPEN_ANGLE, now);
        }
        self.open = target.to_vec();
        (!closing.is_empty(), !target.is_empty())
    }

    fn sample(&self, now: f32) -> [[f32; 4]; MAX_LIDS] {
        let mut lids = [[0.0; 4]; MAX_LIDS];
        for (slot, lid) in lids.iter_mut().zip(&self.transitions) {
            *slot = lid.pos.as_vec3().extend(lid.angle(now)).to_array();
        }
        lids
    }
}

impl VoxelGame {
    pub(crate) fn update_chest_lids(&mut self) {
        let target = match self.client.ui.screen {
            Screen::Chest(pos) | Screen::Mod(pos) if self.client.ui.inventory_open => Some(pos),
            _ => None,
        }
        .filter(|&pos| {
            self.client
                .registry
                .def(self.view_world().block(pos))
                .lid
                .is_some()
        });
        // Les deux couvercles d'un grand coffre s'ouvrent ensemble.
        let other =
            target.and_then(|pos| engine_voxel::double_chest::paired(self.view_streamer(), pos));
        let targets: Vec<IVec3> = target.into_iter().chain(other).collect();
        let now = self.client.renderer.time;
        let (closing, opening) = self.client.chest_lids.update(&targets, now);
        if let Some(audio) = &mut self.client.audio {
            if closing {
                audio.play(audio::CHEST_CLOSE);
            }
            if opening {
                audio.play(audio::CHEST_OPEN);
            }
        }
        self.client.renderer.chest_lids = self.client.chest_lids.sample(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_and_closing_follow_the_menu_and_reverse_smoothly() {
        let pos = IVec3::new(1, 70, -4);
        let mut lids = ChestLids::default();
        assert_eq!(lids.update(&[pos], 0.0), (false, true));
        assert_eq!(lids.update(&[pos], 0.1), (false, false));
        let angle = lids.sample(0.1)[0][3];
        assert!(angle > 0.0 && angle < OPEN_ANGLE);
        assert_eq!(lids.update(&[], 0.1), (true, false));
        assert_eq!(lids.sample(0.1)[0][3], angle);
        let closing = lids.sample(0.15)[0][3];
        assert!(closing < angle);
        assert_eq!(lids.update(&[pos], 0.15), (false, true));
        assert_eq!(lids.sample(0.15)[0][3], closing);
        assert_eq!(lids.sample(1.0)[0][3], OPEN_ANGLE);
        assert_eq!(lids.update(&[], 1.0), (true, false));
        assert_eq!(lids.update(&[], 2.0), (false, false));
        assert_eq!(lids.sample(2.0), [[0.0; 4]; MAX_LIDS]);
    }

    #[test]
    fn both_halves_of_a_large_chest_open_and_close_together() {
        let (a, b) = (IVec3::new(0, 64, 0), IVec3::new(1, 64, 0));
        let mut lids = ChestLids::default();
        assert_eq!(lids.update(&[a, b], 0.0), (false, true));
        let open = lids.sample(1.0);
        assert_eq!((open[0][3], open[1][3]), (OPEN_ANGLE, OPEN_ANGLE));
        assert_eq!(lids.update(&[], 1.0), (true, false));
        assert_eq!(lids.update(&[], 2.0), (false, false));
        assert_eq!(lids.sample(2.0), [[0.0; 4]; MAX_LIDS]);
    }
}
