//! Les captures des formes (voir `screenshots.rs`) : une scène de pièces taillées (dalles de deux
//! matériaux, pentes qui montent, microblocs, verre, une pente retournée) et l'écran de l'établi de
//! taille.

use engine_voxel::Face;
use engine_voxel::shaped::{self, Corner, Link, Piece, PieceKind, Shaped};
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
    Piece::floor_slope(facing, ceiling, level)
}

impl VoxelGame {
    /// Le point de vue de la scène des formes : de biais, un peu au-dessus du sol.
    pub(crate) fn shapes_viewpoint(&self) -> (Vec3, f32, f32) {
        let g = (self.ground(SPOT.0, SPOT.1) + LIFT) as f32;
        (
            Vec3::new(SPOT.0 as f32 + 4.5, g + 7.5, SPOT.1 as f32 - 6.5),
            90.0,
            -33.0,
        )
    }

    /// Une plateforme de pierre et, dessus, une rangée de pièces : de gauche à droite, des dalles de
    /// pierre et de chêne, une pente de chêne, une pente de pierre qui monte jusqu'au niveau 10, une pente
    /// retournée sous un linteau, des microblocs, du verre.
    pub(crate) fn build_shapes(&mut self) {
        let Some(stone) = self.server.registry.id("core:stone") else {
            return;
        };
        let base = self.ground(SPOT.0, SPOT.1) + LIFT + 1;
        // Le sol : une plateforme plane de 10 × 5, un étage de pierre pour la rangée du fond.
        for x in -1..=9 {
            for z in -1..=9 {
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
        self.build_shapes_wave_two(cell);
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

    /// Deuxième vague : une rangée de pièces en boîtes et de pentes en coin (z = 5), une rangée de formes
    /// raccordées (z = 7 : un muret en L, une barrière et son portillon ouvert, des vitres).
    fn build_shapes_wave_two(&mut self, cell: impl Fn(i32, i32, i32) -> IVec3) {
        let oak = "core:oak_planks";
        let rock = "core:stone";
        let glass = "core:glass";
        let put = |game: &mut Self, pos: IVec3, parts: &[(Piece, &str)]| {
            let mut shaped = Shaped::new();
            for (piece, material) in parts {
                if shaped.try_add(*piece, material).is_err() {
                    log::warn!("pièce refusée en {pos}");
                }
            }
            shaped::store(&mut game.streamer_mut(), pos, &shaped);
        };
        let sample = Piece::sample;
        let cuboid = |kind, min, max, count| Piece::cuboid(kind, min, max, count);
        let row = 5;
        put(self, cell(0, row, 0), &[(sample(PieceKind::Stairs), oak)]);
        put(
            self,
            cell(1, row, 0),
            &[(sample(PieceKind::VerticalStairs), rock)],
        );
        // Une marche, une poutre et deux tranches qui ont grandi (3 et 4), dans la même case.
        put(
            self,
            cell(2, row, 0),
            &[
                (cuboid(PieceKind::Step, [0, 0, 0], [8, 16, 8], 1), oak),
                (cuboid(PieceKind::Slice, [13, 0, 0], [16, 16, 3], 9), rock),
                (
                    cuboid(PieceKind::CornerSlice, [12, 0, 12], [16, 4, 16], 8),
                    rock,
                ),
            ],
        );
        let up = shaped::corner_up(Face::NegY, Face::NegZ, Face::NegX).unwrap_or(Face::NegZ);
        let corner = |corner, level| Piece::Slope {
            base: Face::NegY,
            up,
            level,
            corner,
        };
        put(self, cell(3, row, 0), &[(corner(Corner::Outer, 16), rock)]);
        put(self, cell(4, row, 0), &[(corner(Corner::Inner, 16), oak)]);
        // Une pente murale : posée sur le mur ouest de sa case, qui monte vers le haut.
        put(
            self,
            cell(5, row, 0),
            &[(
                Piece::Slope {
                    base: Face::NegX,
                    up: Face::PosY,
                    level: 16,
                    corner: Corner::None,
                },
                oak,
            )],
        );
        // Des planchettes sur trois faces, un demi-panneau et un panneau d'un quart.
        let board = |min, max| cuboid(PieceKind::Board, min, max, 1);
        put(
            self,
            cell(6, row, 0),
            &[
                (board([0, 0, 0], [16, 1, 16]), oak),
                (board([0, 1, 0], [1, 16, 16]), oak),
                (board([1, 1, 0], [16, 16, 1]), oak),
            ],
        );
        put(
            self,
            cell(7, row, 0),
            &[
                (cuboid(PieceKind::HalfPanel, [0, 0, 0], [16, 8, 2], 1), rock),
                (
                    cuboid(PieceKind::QuarterPanel, [8, 8, 8], [16, 16, 10], 1),
                    oak,
                ),
                (
                    cuboid(PieceKind::HalfLayer, [0, 0, 8], [16, 2, 16], 2),
                    glass,
                ),
            ],
        );
        // Les formes raccordées : elles se raccordent toutes seules.
        let line = 7;
        let connected = |kind, open| Piece::Connected { kind, open };
        for (x, z) in [(0, 0), (1, 0), (2, 0), (2, 1), (2, 2)] {
            put(
                self,
                cell(x, line + z, 0),
                &[(connected(Link::Wall, false), rock)],
            );
        }
        for x in 4..8 {
            let (link, open) = if x == 6 {
                (Link::Gate, true)
            } else {
                (Link::Fence, false)
            };
            put(self, cell(x, line, 0), &[(connected(link, open), oak)]);
        }
        put(
            self,
            cell(4, line + 1, 0),
            &[(connected(Link::Fence, false), oak)],
        );
        for x in 8..10 {
            put(
                self,
                cell(x, line, 0),
                &[(connected(Link::Pane, false), glass)],
            );
        }
        put(
            self,
            cell(9, line + 1, 0),
            &[(connected(Link::Pane, false), glass)],
        );
        put(
            self,
            cell(8, line + 1, 0),
            &[(connected(Link::FlatPane, false), glass)],
        );
        put(
            self,
            cell(7, line + 1, 0),
            &[(connected(Link::FlatPane, false), glass)],
        );
    }

    /// L'écran de l'établi de taille : du chêne dans la case d'entrée.
    pub(crate) fn build_shaping_screen(&mut self) {
        let items = self.server.content.items.clone();
        self.server.players.me_mut().inventory.craft = [None; 9];
        if let Some(planks) = items.id("core:oak_planks") {
            self.server.players.me_mut().inventory.craft[0] = Some(ItemStack::new(planks, 24));
        }
        for (key, count) in [("core:stone", 12), ("core:glass", 6)] {
            if let Some(item) = items.id(key) {
                self.server
                    .players
                    .me_mut()
                    .inventory
                    .add(item, count, &items);
            }
        }
        // Quelques pièces déjà taillées, pour voir leurs noms et leurs icônes.
        for (kind, material) in [
            ("core:shape_slab", "core:stone"),
            ("core:shape_slope", "core:oak_planks"),
        ] {
            if let Some(item) = items.id(kind) {
                let number = crate::shape_materials::number_of(material);
                self.server.players.me_mut().inventory.add_stack(
                    ItemStack {
                        item,
                        count: 6,
                        damage: number,
                    },
                    &items,
                );
            }
        }
        self.client.ui.screen = crate::ui::Screen::Shaping;
    }
}
