//! Phase 0 : le jeu de la vie de Conway en terminal.
//!
//! Ce qui est déjà là : une grille et son affichage.
//! À toi d'écrire `Grid::live_neighbours` et `Grid::step`, puis de retirer
//! les `#[ignore]` des tests en bas du fichier et de lancer `cargo test`.

use std::fmt;

/// Une grille 2D stockée à plat dans un `Vec<bool>`, comme un tableau C.
/// La même idée servira pour les chunks voxel en 3D.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<bool>,
}

impl Grid {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![false; width * height],
        }
    }

    /// Construit une grille depuis du texte : `#` = vivante, `.` = morte.
    pub fn parse(text: &str) -> Self {
        let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        let height = lines.len();
        let width = lines.first().map_or(0, |l| l.len());
        let mut grid = Self::new(width, height);
        for (y, line) in lines.iter().enumerate() {
            for (x, c) in line.chars().enumerate() {
                grid.set(x, y, c == '#');
            }
        }
        grid
    }

    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// `None` si (x, y) est hors de la grille : pas de lecture hors limites comme en C.
    pub fn get(&self, x: usize, y: usize) -> Option<bool> {
        if x < self.width && y < self.height {
            Some(self.cells[self.index(x, y)])
        } else {
            None
        }
    }

    pub fn set(&mut self, x: usize, y: usize, alive: bool) {
        let i = self.index(x, y);
        self.cells[i] = alive;
    }

    /// Nombre de voisines vivantes parmi les 8 cases autour de (x, y).
    /// Les cases hors de la grille comptent comme mortes.
    /// Indice : `x.checked_sub(1)` renvoie `None` au lieu de déborder.
    pub fn live_neighbours(&self, x: usize, y: usize) -> usize {
        let _ = (x, y);
        todo!("phase 0 : compter les voisines vivantes")
    }

    /// Calcule la génération suivante, sans modifier `self`.
    /// Règles : une cellule vivante survit avec 2 ou 3 voisines ;
    /// une cellule morte naît avec exactement 3 voisines.
    pub fn step(&self) -> Grid {
        todo!("phase 0 : appliquer les règles de Conway")
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for y in 0..self.height {
            for x in 0..self.width {
                let alive = self.get(x, y).unwrap_or(false);
                write!(f, "{}", if alive { '#' } else { '.' })?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

fn main() {
    let glider = Grid::parse(
        "
.#......
..#.....
###.....
........
........
",
    );
    println!("Génération 0 :\n{glider}");
    // Quand `step` est écrit, affiche quelques générations ici.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display_round_trip() {
        let text = ".#.\n.#.\n.#.\n";
        assert_eq!(Grid::parse(text).to_string(), text);
    }

    #[test]
    fn get_outside_grid_is_none() {
        let grid = Grid::new(3, 3);
        assert_eq!(grid.get(3, 0), None);
        assert_eq!(grid.get(0, 0), Some(false));
    }

    #[test]
    #[ignore = "retire cette ligne quand live_neighbours est écrit"]
    fn counts_neighbours() {
        let grid = Grid::parse("###\n#.#\n###\n");
        assert_eq!(grid.live_neighbours(1, 1), 8);
        assert_eq!(grid.live_neighbours(0, 0), 2);
    }

    #[test]
    #[ignore = "retire cette ligne quand step est écrit"]
    fn blinker_oscillates() {
        let vertical = Grid::parse(".#.\n.#.\n.#.\n");
        let horizontal = Grid::parse("...\n###\n...\n");
        assert_eq!(vertical.step(), horizontal);
        assert_eq!(horizontal.step(), vertical);
    }

    #[test]
    #[ignore = "retire cette ligne quand step est écrit"]
    fn block_is_stable() {
        let block = Grid::parse("....\n.##.\n.##.\n....\n");
        assert_eq!(block.step(), block);
    }
}
