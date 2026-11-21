//! Se déplacer en suivant un chemin (phase O5) : la recherche est dans
//! `engine_voxel::find_path` (un A* sur les blocs), ici on décide quand la relancer et
//! comment marcher d'une case à l'autre.
//!
//! - Un chemin est recalculé quand le but a bougé (le joueur poursuivi), quand la
//!   créature est coincée, ou quand elle choisit un nouveau but ; au plus toutes les
//!   [`REPATH_INTERVAL`] secondes par créature, et au plus [`SEARCHES_PER_STEP`] recherches
//!   par pas de simulation pour toutes (un pic de créatures ne fait pas ramer).
//! - Pour suivre le chemin : marcher vers le centre de la case suivante, sauter si elle
//!   est plus haute, passer à la suivante une fois dessus.

use engine_voxel::{BlockId, PathRules, World, find_path};
use glam::{IVec3, Vec3};

/// Au plus tant de recherches de chemin par pas de simulation (60 par seconde).
pub const SEARCHES_PER_STEP: usize = 6;
/// Au plus une recherche toutes les tant de secondes par créature.
const REPATH_INTERVAL: f32 = 0.5;
/// Les cases explorées au plus par recherche.
const MAX_NODES: usize = 600;
/// La plus grande chute acceptée.
const MAX_DROP: i32 = 3;
/// Assez près du centre d'une case pour passer à la suivante.
const REACHED: f32 = 0.35;
/// Sans avancer de [`STUCK_DISTANCE`] en ce temps, la créature est coincée.
const STUCK_TIME: f32 = 1.5;
const STUCK_DISTANCE: f32 = 0.3;

/// Les dégâts par seconde de chaque bloc (les fluides qui blessent, la lave), indexés par
/// bloc : une créature n'y va jamais, et s'y brûle si elle y tombe.
#[derive(Debug, Clone, Default)]
pub struct DangerBlocks(pub Vec<f32>);

impl DangerBlocks {
    pub fn damage(&self, block: BlockId) -> f32 {
        self.0.get(block.0 as usize).copied().unwrap_or(0.0)
    }

    pub fn is_danger(&self, block: BlockId) -> bool {
        self.damage(block) > 0.0
    }
}

/// Le chemin qu'une créature suit.
#[derive(Debug, Clone, Default)]
pub struct PathFollow {
    /// Le point vers lequel elle erre, choisi par l'IA (`None` : elle reste là).
    pub wander_goal: Option<IVec3>,
    cells: Vec<IVec3>,
    next: usize,
    /// Le but du chemin actuel.
    goal: Option<IVec3>,
    /// Temps avant de pouvoir relancer une recherche.
    cooldown: f32,
    /// Pour voir si elle avance.
    stuck_timer: f32,
    last_position: Vec3,
}

impl PathFollow {
    /// Faut-il chercher un nouveau chemin vers `goal` ? (Le but a changé, ou elle est
    /// coincée, et le délai est passé.)
    pub fn wants(&self, goal: IVec3, position: Vec3) -> bool {
        if self.cooldown > 0.0 {
            return false;
        }
        let moved_goal = self.goal.is_none_or(|g| (g - goal).abs().max_element() > 1);
        let stuck = self.stuck_timer >= STUCK_TIME;
        let finished = self.next >= self.cells.len() && cell_of(position) != goal;
        moved_goal || stuck || finished
    }

    /// Cherche un chemin vers `goal` et le suit désormais.
    pub fn plan(
        &mut self,
        world: &World,
        danger: &DangerBlocks,
        position: Vec3,
        height: f32,
        goal: IVec3,
    ) {
        let rules = PathRules {
            height: height.ceil().max(1.0) as i32,
            max_drop: MAX_DROP,
            max_nodes: MAX_NODES,
            danger: |block| danger.is_danger(block),
        };
        let path = find_path(world, cell_of(position), goal, &rules);
        self.cells = path.cells;
        self.next = 0;
        self.goal = Some(goal);
        self.cooldown = REPATH_INTERVAL;
        self.stuck_timer = 0.0;
        self.last_position = position;
    }

    /// Oublie le chemin (elle reste sur place).
    pub fn clear(&mut self) {
        self.cells.clear();
        self.next = 0;
        self.goal = None;
    }

    /// La direction (horizontale, unitaire) et le saut pour ce pas, ou `None` si le
    /// chemin est fini.
    pub fn steer(&mut self, position: Vec3, dt: f32) -> Option<(Vec3, bool)> {
        self.cooldown = (self.cooldown - dt).max(0.0);
        // Coincée ? (elle a un chemin mais n'avance pas)
        if self.next < self.cells.len() {
            if position.distance(self.last_position) < STUCK_DISTANCE {
                self.stuck_timer += dt;
            } else {
                self.stuck_timer = 0.0;
                self.last_position = position;
            }
        }
        while let Some(&cell) = self.cells.get(self.next) {
            let target = Vec3::new(cell.x as f32 + 0.5, cell.y as f32, cell.z as f32 + 0.5);
            let flat = Vec3::new(target.x - position.x, 0.0, target.z - position.z);
            let feet = position.y.floor() as i32;
            if flat.length() < REACHED && (cell.y - feet).abs() <= 1 {
                self.next += 1;
                continue;
            }
            // Plus haute que les pieds : un saut.
            let jump = cell.y > feet;
            return Some((flat.normalize_or_zero(), jump));
        }
        None
    }

    /// Les cases qui restent à parcourir (pour les tests).
    #[cfg(test)]
    pub fn remaining(&self) -> &[IVec3] {
        &self.cells[self.next.min(self.cells.len())..]
    }
}

/// La case des pieds.
pub fn cell_of(position: Vec3) -> IVec3 {
    IVec3::new(
        position.x.floor() as i32,
        (position.y + 0.01).floor() as i32,
        position.z.floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_follows_the_cells_and_jumps_up() {
        let mut follow = PathFollow {
            cells: vec![IVec3::new(1, 1, 0), IVec3::new(2, 2, 0)],
            ..PathFollow::default()
        };
        // Vers la première case, à plat.
        let (dir, jump) = follow.steer(Vec3::new(0.5, 1.0, 0.5), 0.016).unwrap();
        assert!(dir.x > 0.9 && !jump);
        // Sur la première : on passe à la seconde, plus haute (saut).
        let (_, jump) = follow.steer(Vec3::new(1.5, 1.0, 0.5), 0.016).unwrap();
        assert!(jump);
        // Arrivée.
        assert_eq!(follow.steer(Vec3::new(2.5, 2.0, 0.5), 0.016), None);
    }

    #[test]
    fn a_new_path_is_wanted_when_the_goal_moves_or_when_stuck() {
        let mut follow = PathFollow {
            cells: vec![IVec3::new(5, 1, 0)],
            goal: Some(IVec3::new(5, 1, 0)),
            ..PathFollow::default()
        };
        let here = Vec3::new(0.5, 1.0, 0.5);
        assert!(!follow.wants(IVec3::new(5, 1, 0), here));
        assert!(
            !follow.wants(IVec3::new(6, 1, 1), here),
            "un bloc de plus : pareil"
        );
        assert!(follow.wants(IVec3::new(9, 1, 0), here), "le but a bougé");
        // Coincée 2 s sur place.
        for _ in 0..120 {
            follow.steer(here, 1.0 / 60.0);
        }
        assert!(follow.wants(IVec3::new(5, 1, 0), here));
    }
}
