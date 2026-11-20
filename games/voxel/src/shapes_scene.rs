//! Les captures des formes (voir `screenshots.rs`) : une scène de pièces taillées (dalles de deux
//! matériaux, pentes qui montent, microblocs, verre, une pente retournée) et l'écran de l'établi de
//! taille.

use engine_voxel::Face;
use engine_voxel::shaped::{self, Piece, Shaped};
use glam::{IVec3, Vec3};

use crate::VoxelGame;
use crate::inventory::ItemStack;

/// Où construire la scène, en coordonnées de scène (autour du point d'apparition).
const SPOT: (i32, i32) = (-20, 12);
/// La scène flotte au-dessus des arbres : rien ne gêne la vue, quel que soit le biome.
const LIFT: i32 = 30;

fn layer(face: Face, offset: u8, thickness: u8) -> Piece {
    Piece::Layer {
        face,
        offset,
        thickness,
    }
}

fn slope(facing: u8, ceiling: bool, level: u8) -> Piece {
    Piece::Slope {
        facing,
        ceiling,
        level,
    }
}

impl VoxelGame {
    /// Le point de vue de la scène des formes : de biais, un peu au-dessus du sol.
    pub(crate) fn shapes_viewpoint(&self) -> (Vec3, f32, f32) {
        let g = (self.ground(SPOT.0, SPOT.1) + LIFT) as f32;
        (
            Vec3::new(SPOT.0 as f32 + 4.5, g + 4.2, SPOT.1 as f32 - 5.5),
            90.0,
            -22.0,
        )
    }

    /// Une plateforme de pierre et, dessus, une rangée de pièces : de gauche à droite, des dalles de
    /// pierre et de chêne, une pente de chêne, une pente de pierre qui monte jusqu'au niveau 10, une pente
    /// retournée sous un linteau, des microblocs, du verre.
    pub(crate) fn build_shapes(&mut self) {
        let Some(stone) = self.registry.id("core:stone") else {
            return;
        };
        let base = self.ground(SPOT.0, SPOT.1) + LIFT + 1;
        // Le sol : une plateforme plane de 10 × 5, un étage de pierre pour la rangée du fond.
        for x in -1..=9 {
            for z in -1..=4 {
                for y in base - 3..base {
                    self.put(IVec3::new(SPOT.0 + x, y, SPOT.1 + z), stone);
                }
                for y in base..base + 6 {
                    self.put(
                        IVec3::new(SPOT.0 + x, y, SPOT.1 + z),
                        engine_voxel::BlockId::AIR,
                    );
                }
            }
        }
        let origin = self.origin();
        let cell =
            move |x: i32, z: i32, y: i32| IVec3::new(SPOT.0 + x, base + y, SPOT.1 + z) + origin;
        let oak = "core:oak_planks";
        let rock = "core:stone";
        let put = |game: &mut Self, pos: IVec3, parts: &[(Piece, &str)]| {
            let mut shaped = Shaped::new();
            for (piece, material) in parts {
                if shaped.try_add(*piece, material).is_err() {
                    log::warn!("pièce refusée en {pos}");
                }
            }
            shaped::store(&mut game.streamer_mut(), pos, &shaped);
        };
        // Une dalle de chêne sur une dalle de pierre.
        put(
            self,
            cell(0, 0, 0),
            &[
                (layer(Face::NegY, 0, 8), rock),
                (layer(Face::NegY, 8, 8), oak),
            ],
        );
        // Une dalle de chêne seule, et une couche de pierre sur le côté d'un bloc.
        put(self, cell(1, 0, 0), &[(layer(Face::NegY, 0, 8), oak)]);
        // Des pentes pleines, côté haut vers le fond (sud), puis vers l'est.
        put(self, cell(2, 0, 0), &[(slope(2, false, 16), oak)]);
        put(self, cell(3, 0, 0), &[(slope(1, false, 16), rock)]);
        // Une pente de pierre montée couche par couche jusqu'au niveau 10.
        put(self, cell(4, 0, 0), &[(slope(2, false, 10), rock)]);
        // Une pente retournée, accrochée au plafond d'une dalle de chêne.
        put(
            self,
            cell(5, 0, 0),
            &[(layer(Face::NegY, 0, 4), oak), (slope(2, true, 12), rock)],
        );
        // Des microblocs : un cube de 1/8, un grain, deux cubes de deux matériaux.
        put(
            self,
            cell(6, 0, 0),
            &[
                (
                    Piece::Micro {
                        cells: shaped::micro_mask(0, 0, 0, 2),
                    },
                    oak,
                ),
                (
                    Piece::Micro {
                        cells: shaped::micro_mask(2, 0, 2, 2),
                    },
                    rock,
                ),
                (
                    Piece::Micro {
                        cells: shaped::micro_mask(3, 2, 0, 1),
                    },
                    oak,
                ),
            ],
        );
        // Du verre : une dalle translucide sur une dalle de pierre.
        put(
            self,
            cell(7, 0, 0),
            &[
                (layer(Face::NegY, 0, 6), rock),
                (layer(Face::NegY, 6, 6), "core:glass"),
            ],
        );
        // Un petit escalier de pentes, du sol au premier étage.
        for (i, level) in [16u8, 16, 16].into_iter().enumerate() {
            put(self, cell(i as i32, 2, 0), &[(slope(2, false, level), oak)]);
            put(
                self,
                cell(i as i32, 3, 0),
                &[(slope(2, false, level), rock)],
            );
        }
    }

    /// L'écran de l'établi de taille : du chêne dans la case d'entrée.
    pub(crate) fn build_shaping_screen(&mut self) {
        let items = self.content.items.clone();
        self.inventory.craft = [None; 9];
        if let Some(planks) = items.id("core:oak_planks") {
            self.inventory.craft[0] = Some(ItemStack::new(planks, 24));
        }
        for (key, count) in [("core:stone", 12), ("core:glass", 6)] {
            if let Some(item) = items.id(key) {
                self.inventory.add(item, count, &items);
            }
        }
        // Quelques pièces déjà taillées, pour voir leurs noms et leurs icônes.
        for (kind, material) in [
            ("core:shape_slab", "core:stone"),
            ("core:shape_slope", "core:oak_planks"),
        ] {
            if let Some(item) = items.id(kind) {
                let number = crate::shape_materials::number_of(material);
                self.inventory.add_stack(
                    ItemStack {
                        item,
                        count: 6,
                        damage: number,
                    },
                    &items,
                );
            }
        }
        self.ui.screen = crate::ui::Screen::Shaping;
    }
}
