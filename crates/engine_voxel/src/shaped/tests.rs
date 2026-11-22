//! Les tests du modèle des formes : ajout, fusion, occupation, boîtes, format, rotation, pose.

use super::grid::{N, set, volume};
use super::*;
use crate::tech::capabilities::Face;

const OAK: &str = "core:oak_planks";
const STONE: &str = "core:stone";

fn bottom(offset: u8, thickness: u8) -> Piece {
    Piece::Layer {
        face: Face::NegY,
        offset,
        thickness,
    }
}

fn top(offset: u8, thickness: u8) -> Piece {
    Piece::Layer {
        face: Face::PosY,
        offset,
        thickness,
    }
}

#[test]
fn layers_of_the_same_material_merge() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 3), OAK).unwrap();
    s.try_add(bottom(3, 5), OAK).unwrap();
    assert_eq!(s.parts.len(), 1);
    assert_eq!(s.parts[0].piece, bottom(0, 8));
    // Une couche du même côté s'épaissit aussi par l'autre bout.
    s.try_add(bottom(8, 1), OAK).unwrap();
    assert_eq!(s.parts[0].piece, bottom(0, 9));
}

#[test]
fn different_materials_stay_separate() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    s.try_add(top(0, 8), OAK).unwrap();
    assert_eq!(s.parts.len(), 2);
    assert_eq!(s.materials, [STONE, OAK]);
    assert!(s.is_full());
    // Pleine, mais de deux matériaux : elle reste une case de formes.
    assert_eq!(s.simplified(), Simplified::Shaped);
}

#[test]
fn overlap_is_refused() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    assert_eq!(s.try_add(bottom(0, 1), OAK), Err(AddError::Overlap));
    assert_eq!(s.try_add(bottom(0, 1), STONE), Err(AddError::Overlap));
    assert_eq!(s.try_add(top(4, 8), STONE), Err(AddError::Overlap));
    // Le refus ne laisse pas un matériau inutile dans la liste.
    assert_eq!(s.materials, [STONE]);
    assert_eq!(s.try_add(top(0, 0), STONE), Err(AddError::Invalid));
    assert_eq!(s.try_add(top(10, 8), STONE), Err(AddError::Invalid));
}

#[test]
fn stacking_up_to_full_simplifies() {
    let mut s = Shaped::new();
    for i in 0..16 {
        assert!(s.simplified() == Simplified::Shaped || i == 0);
        s.try_add(bottom(i, 1), OAK).unwrap();
    }
    assert!(s.is_full());
    assert_eq!(s.simplified(), Simplified::Full(OAK));
    assert_eq!(Shaped::new().simplified(), Simplified::Empty);
}

#[test]
fn limits() {
    let mut s = Shaped::new();
    for i in 0..16 {
        s.try_add(bottom(i, 1), &format!("mod:m{i}")).unwrap();
    }
    assert_eq!(s.try_add(top(0, 0), "mod:autre"), Err(AddError::Invalid));
    assert_eq!(
        s.try_add(Piece::Micro { cells: 1 }, "mod:autre"),
        Err(AddError::TooManyMaterials)
    );
    assert_eq!(s.materials.len(), 16);
    // Trente-deux pièces au plus.
    let mut s = Shaped::new();
    s.materials.push(OAK.to_string());
    s.parts = (0..MAX_PARTS)
        .map(|i| Part {
            piece: Piece::Micro { cells: 1 << i },
            material: 0,
        })
        .collect();
    assert_eq!(
        s.try_add(Piece::Micro { cells: 1 << 40 }, STONE),
        Err(AddError::TooManyParts)
    );
    assert_eq!(s.materials, [OAK]);
}

#[test]
fn slope_occupancy_at_every_level() {
    for level in 1..=16u8 {
        let piece = Piece::floor_slope(0, false, level);
        let occ = piece.occupancy();
        // Contre le bord haut (z = 0) la colonne est la plus haute ; contre le bord bas, la plus basse.
        let column = |z: usize| (0..N).filter(|&y| is_set(&occ, 3, y, z)).count();
        assert_eq!(column(0), slope_height(level, 0));
        assert_eq!(column(15), slope_height(level, 15));
        assert!(column(0) >= column(8) && column(8) >= column(15));
        // Plein de bas en haut : aucune cellule en l'air.
        for z in 0..N {
            let h = column(z);
            assert!((0..h).all(|y| is_set(&occ, 3, y, z)));
        }
    }
    let full_slope = Piece::floor_slope(0, false, 16).occupancy();
    // Un peu plus d'un demi-bloc : 136 cellules sur 256 dans chaque rangée de x.
    assert_eq!(volume(&full_slope), 136 * 16);
    // Retournée, la pente occupe le haut.
    let ceiling = Piece::floor_slope(0, true, 16).occupancy();
    assert!(is_set(&ceiling, 0, 15, 15) && !is_set(&ceiling, 0, 0, 15));
    assert!(is_set(&ceiling, 0, 0, 0));
    // Chaque orientation a son bord haut de son côté.
    for (facing, x, z) in [(1u8, 15, 8), (2, 8, 15), (3, 0, 8)] {
        let occ = Piece::floor_slope(facing, false, 16).occupancy();
        assert!(is_set(&occ, x, 15, z), "orientation {facing}");
    }
}

#[test]
fn slope_boxes_are_a_staircase() {
    let piece = Piece::floor_slope(1, false, 16);
    let boxes = piece.int_boxes();
    assert_eq!(boxes.len(), 16);
    // Côté haut à l'est : la marche la plus haute touche x = 16.
    let tallest = boxes.iter().max_by_key(|b| b[4]).unwrap();
    assert_eq!((tallest[3], tallest[4]), (16, 16));
    // La somme des boîtes remplit exactement l'occupation.
    let mut occ = [0u16; 256];
    for b in &boxes {
        for y in b[1]..b[4] {
            for z in b[2]..b[5] {
                for x in b[0]..b[3] {
                    set(&mut occ, usize::from(x), usize::from(y), usize::from(z));
                }
            }
        }
    }
    assert_eq!(occ, piece.occupancy());
    // Un niveau bas fusionne les colonnes de même hauteur.
    let low = Piece::floor_slope(2, true, 2);
    assert!(low.int_boxes().len() <= 4);
}

#[test]
fn slopes_climb_level_by_level() {
    let mut s = Shaped::new();
    let slope = |level| Piece::floor_slope(3, false, level);
    for _ in 0..16 {
        s.try_add(slope(1), STONE).unwrap();
    }
    assert_eq!(s.parts.len(), 1);
    assert_eq!(s.parts[0].piece, slope(16));
    assert_eq!(s.try_add(slope(1), STONE), Err(AddError::Overlap));
    // Une pente d'un autre matériau, au même endroit, est refusée.
    assert_eq!(s.try_add(slope(1), OAK), Err(AddError::Overlap));
}

#[test]
fn micro_blocks_merge_and_box_up() {
    let mut s = Shaped::new();
    s.try_add(
        Piece::Micro {
            cells: micro_mask(0, 0, 0, 2),
        },
        OAK,
    )
    .unwrap();
    s.try_add(
        Piece::Micro {
            cells: micro_mask(2, 0, 0, 2),
        },
        OAK,
    )
    .unwrap();
    assert_eq!(s.parts.len(), 1);
    // Deux cubes voisins ne font qu'une boîte de 1/4 × 1/8... : 4 × 2 × 2 cellules.
    assert_eq!(s.boxes().len(), 1);
    assert_eq!(
        s.try_add(
            Piece::Micro {
                cells: micro_mask(1, 1, 1, 1)
            },
            STONE
        ),
        Err(AddError::Overlap)
    );
    s.try_add(
        Piece::Micro {
            cells: micro_mask(3, 3, 3, 1),
        },
        STONE,
    )
    .unwrap();
    assert_eq!(s.parts.len(), 2);
    // Un microbloc aligné rend un objet « microbloc », un grain seul un « grain ».
    assert_eq!(s.parts[0].piece.drops(), [(PieceKind::Micro, 2)]);
    assert_eq!(s.parts[1].piece.drops(), [(PieceKind::Nano, 1)]);
}

#[test]
fn drops_follow_the_piece() {
    assert_eq!(bottom(0, 5).drops(), [(PieceKind::Layer, 5)]);
    assert_eq!(
        bottom(0, 13).drops(),
        [(PieceKind::Slab, 1), (PieceKind::Layer, 5)]
    );
    let slope = |level| Piece::floor_slope(0, false, level);
    assert_eq!(slope(16).drops(), [(PieceKind::Slope, 1)]);
    assert_eq!(slope(7).drops(), [(PieceKind::SlopeLayer, 7)]);
}

#[test]
fn round_trip_keeps_unknown_materials() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 4), "mod:inconnu").unwrap();
    s.try_add(Piece::floor_slope(2, true, 6), OAK).unwrap();
    s.try_add(
        Piece::Micro {
            cells: 1 << 16 | 1 << 17,
        },
        STONE,
    )
    .unwrap();
    let bytes = s.encode();
    let back = Shaped::decode(&bytes).unwrap();
    assert_eq!(back, s);
    assert!(back.materials.contains(&"mod:inconnu".to_string()));
    // La lecture légère donne les mêmes boîtes.
    assert_eq!(Shaped::boxes_of(&bytes).unwrap(), s.boxes());
    // Des octets abîmés ou d'une autre version : refusés, sans panique.
    assert!(Shaped::decode(&[]).is_none());
    assert!(Shaped::decode(&[9, 0, 0]).is_none());
    for cut in 0..bytes.len() {
        assert!(Shaped::decode(&bytes[..cut]).is_none());
    }
    // Un matériau hors liste ou une pièce hors bornes.
    assert!(Shaped::decode(&[1, 0, 1, 5, 0, 0, 0, 1]).is_none());
    assert!(Shaped::decode(&[1, 1, 1, b'a', 1, 0, 0, 0, 20]).is_none());
}

#[test]
fn removing_the_aimed_piece() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    s.try_add(top(0, 8), OAK).unwrap();
    // Visée du dessus (la face haute de la dalle de chêne) : le chêne.
    let (piece, material) = s.remove_at(Vec3::new(0.5, 1.0, 0.5), Vec3::Y).unwrap();
    assert_eq!((piece, material.as_str()), (top(0, 8), OAK));
    // Le chêne n'est plus dans la liste.
    assert_eq!(s.materials, [STONE]);
    let (piece, _) = s.remove_at(Vec3::new(0.5, 0.5, 0.5), Vec3::Y).unwrap();
    assert_eq!(piece, bottom(0, 8));
    assert!(s.is_empty());
    assert!(s.remove_at(Vec3::splat(0.5), Vec3::Y).is_none());
}

#[test]
fn dominant_material_is_the_biggest_volume() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 3), STONE).unwrap();
    s.try_add(top(0, 5), OAK).unwrap();
    assert_eq!(s.dominant_material(), Some(OAK));
    assert_eq!(Shaped::new().dominant_material(), None);
}

#[test]
fn turning_four_times_is_the_identity() {
    let mut s = Shaped::new();
    s.try_add(
        Piece::Layer {
            face: Face::PosX,
            offset: 0,
            thickness: 1,
        },
        OAK,
    )
    .unwrap();
    s.try_add(Piece::floor_slope(3, true, 5), STONE).unwrap();
    s.try_add(
        Piece::Micro {
            cells: micro_mask(0, 0, 3, 1) | 1 << 20,
        },
        OAK,
    )
    .unwrap();
    let mut t = s.clone();
    for _ in 0..4 {
        t = t.turned(1);
    }
    assert_eq!(t, s);
    // Un quart de tour : +X devient +Z, la pente d'ouest regarde le nord.
    let once = s.turned(1);
    assert!(matches!(
        once.parts[0].piece,
        Piece::Layer {
            face: Face::PosZ,
            ..
        }
    ));
    assert_eq!(once.parts[1].piece, Piece::floor_slope(0, true, 5));
    // L'occupation tourne avec la case : la tranche x = 15 passe en z = 15.
    let occ = once.parts[0].piece.occupancy();
    assert!(is_set(&occ, 4, 4, 15) && !is_set(&occ, 15, 4, 4));
    // Les microblocs : la cellule (x, z) va en (3 - z, x).
    let m = Piece::Micro {
        cells: 1 << (4 * 3),
    }
    .turned(1);
    assert_eq!(m, Piece::Micro { cells: 1 });
    assert_eq!(Shaped::turned_data(&[0], 1), [0]);
    let data = s.encode();
    assert_eq!(Shaped::turned_data(&data, 1), once.encode());
}

#[test]
fn plans_for_a_new_neighbour_cell() {
    let look = Vec3::new(0.0, 0.0, 1.0);
    // Un mur à l'est de la case : la couche se colle à la face -X de la case voisine.
    let piece = plan_outer(PieceKind::Slab, Vec3::new(1.0, 0.4, 0.5), IVec3::X, look);
    assert_eq!(
        piece,
        Piece::Layer {
            face: Face::NegX,
            offset: 0,
            thickness: 8
        }
    );
    // Une pente au sol : son côté haut est celui du regard (la pente regarde le joueur).
    let piece = plan_outer(PieceKind::Slope, Vec3::new(0.5, 1.0, 0.5), IVec3::Y, look);
    assert_eq!(piece, Piece::floor_slope(2, false, 16));
    // Sous un plafond : retournée.
    let piece = plan_outer(
        PieceKind::SlopeLayer,
        Vec3::new(0.5, 0.0, 0.5),
        IVec3::NEG_Y,
        look,
    );
    assert!(matches!(
        piece,
        Piece::Slope {
            base: Face::PosY,
            level: 1,
            corner: Corner::None,
            ..
        }
    ));
    // Un microbloc sur le coin visé.
    let piece = plan_outer(PieceKind::Micro, Vec3::new(0.9, 1.0, 0.1), IVec3::Y, look);
    assert_eq!(
        piece,
        Piece::Micro {
            cells: micro_mask(2, 0, 0, 2)
        }
    );
}

#[test]
fn plans_inside_a_shaped_cell() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    let look = Vec3::Z;
    // Le dessus de la dalle de pierre est à l'intérieur de la case : la dalle de chêne s'y pose.
    let top_face = Vec3::new(0.5, 0.5, 0.5);
    let piece = plan_inner(PieceKind::Slab, &s, top_face, IVec3::Y, look).unwrap();
    assert_eq!(piece, bottom(8, 8));
    s.try_add(piece, OAK).unwrap();
    assert!(s.is_full());
    // Le bord de la case : rien à faire ici, la pièce va dans la case voisine.
    assert!(
        plan_inner(
            PieceKind::Slab,
            &s,
            Vec3::new(0.5, 1.0, 0.5),
            IVec3::Y,
            look
        )
        .is_none()
    );
    // Un grain sur le dessus de la dalle.
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    let piece = plan_inner(
        PieceKind::Nano,
        &s,
        Vec3::new(0.6, 0.5, 0.1),
        IVec3::Y,
        look,
    )
    .unwrap();
    assert_eq!(
        piece,
        Piece::Micro {
            cells: micro_mask(2, 2, 0, 1)
        }
    );
    s.try_add(piece, OAK).unwrap();
}

#[test]
fn a_slope_aimed_gets_its_orientation() {
    let mut s = Shaped::new();
    let slope = Piece::floor_slope(3, false, 4);
    s.try_add(slope, STONE).unwrap();
    // On regarde vers le nord, mais la pente visée est tournée à l'ouest : le niveau s'ajoute.
    let piece = plan_inner(
        PieceKind::SlopeLayer,
        &s,
        Vec3::new(0.1, 0.25, 0.5),
        IVec3::Y,
        Vec3::NEG_Z,
    )
    .unwrap();
    s.try_add(piece, STONE).unwrap();
    assert_eq!(s.parts.len(), 1);
    assert_eq!(s.parts[0].piece, Piece::floor_slope(3, false, 5));
}

#[test]
fn what_can_be_shaped() {
    let registry = crate::BlockRegistry::builtin();
    let shapeable = |key: &str| is_shapeable(registry.def(registry.id(key).unwrap()));
    assert!(shapeable("core:oak_planks"));
    assert!(shapeable("core:stone"));
    assert!(shapeable("core:glass"));
    assert!(!shapeable("core:chest"));
    assert!(!shapeable("core:furnace"));
    assert!(!shapeable("core:water"));
    assert!(!shapeable(BEHAVIOR));
}

#[test]
fn the_registry_knows_the_shaped_block() {
    let registry = crate::BlockRegistry::builtin();
    let id = registry.id(BEHAVIOR).expect("core:shaped");
    let def = registry.def(id);
    assert!(is_shaped_block(def));
    assert_eq!(def.shape, Shape::Model);
    assert!(!def.placeable);
}

#[test]
fn storing_a_cell_simplifies_it() {
    use crate::streaming::{StreamingConfig, tests::pump_until_idle};
    use crate::worldgen::generator::TerrainGenerator;
    let config = StreamingConfig {
        view_radius: 1,
        ..StreamingConfig::default()
    };
    let mut s = crate::ChunkStreamer::new(TerrainGenerator::new(7), config);
    pump_until_idle(&mut s, Vec3::new(5.0, 100.0, 5.0));
    let id = |s: &crate::ChunkStreamer, key: &str| s.world().registry().id(key).unwrap();
    let pos = IVec3::new(8, 140, 8);
    // Une dalle : une case de formes, avec son contenu dans l'entité.
    let mut cell = Shaped::new();
    cell.try_add(bottom(0, 8), STONE).unwrap();
    assert!(store(&mut s, pos, &cell));
    assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
    assert_eq!(read(s.block_entity(pos)), cell);
    // Deux dalles de pierre : un vrai bloc de pierre, sans entité.
    cell.try_add(bottom(8, 8), STONE).unwrap();
    assert!(store(&mut s, pos, &cell));
    assert_eq!(s.world().block(pos), id(&s, STONE));
    assert!(s.block_entity(pos).is_none());
    // Pleine de deux matériaux : toujours une case de formes.
    let mut mixed = Shaped::new();
    mixed.try_add(bottom(0, 8), STONE).unwrap();
    mixed.try_add(bottom(8, 8), OAK).unwrap();
    assert!(store(&mut s, pos, &mixed));
    assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
    assert_eq!(s.world().block_boxes(pos).len(), 2);
    // Vide : de l'air.
    assert!(store(&mut s, pos, &Shaped::new()));
    assert!(s.world().block(pos).is_air());
    // Un matériau retiré du registre : la case pleine reste lisible comme case de formes.
    let mut lost = Shaped::new();
    lost.try_add(bottom(0, 16), "mod:disparu").unwrap();
    assert!(store(&mut s, pos, &lost));
    assert_eq!(s.world().block(pos), id(&s, BEHAVIOR));
    assert_eq!(read(s.block_entity(pos)).materials, ["mod:disparu"]);
}

#[test]
fn stackable_pieces() {
    assert!(stacks_by_damage("core:shape_slab"));
    assert!(!stacks_by_damage("core:iron_pickaxe"));
}

// ----- Les pièces en boîtes -----

/// L'occupation d'une case après `k` quarts de tour : la cellule (x, z) va en (15 - z, x).
fn rotated(occ: &Occupancy, k: u8) -> Occupancy {
    let mut out = [0u16; 256];
    for y in 0..N {
        for z in 0..N {
            for x in 0..N {
                if !is_set(occ, x, y, z) {
                    continue;
                }
                let (mut nx, mut nz) = (x, z);
                for _ in 0..k {
                    (nx, nz) = (15 - nz, nx);
                }
                set(&mut out, nx, y, nz);
            }
        }
    }
    out
}

#[test]
fn every_kind_has_a_unique_key_and_code() {
    for (i, kind) in PieceKind::ALL.into_iter().enumerate() {
        assert_eq!(usize::from(kind.code()), i);
        assert_eq!(PieceKind::from_code(kind.code()), Some(kind));
        assert_eq!(PieceKind::from_key(kind.key()), Some(kind));
        assert_eq!(PieceKind::from_short_name(kind.short_name()), Some(kind));
        assert!(stacks_by_damage(kind.key()));
        assert!(kind.per_block() >= 1);
        // Chaque sorte est rangée dans une famille de l'établi.
        assert!(PieceKind::of_family(kind.family()).any(|k| k == kind));
    }
    assert_eq!(PieceKind::ALL.len(), 23);
    assert_eq!(PieceKind::from_code(23), None);
    assert_eq!(PieceKind::Slab.short_name(), "slab");
    assert_eq!(PieceKind::CornerSlice.short_name(), "corner_slice");
}

#[test]
fn every_sample_piece_is_valid_and_its_boxes_fill_its_occupancy() {
    for kind in PieceKind::ALL {
        let piece = Piece::sample(kind);
        assert!(piece.is_valid(), "{kind:?}");
        let boxes = piece.int_boxes_with(&Neighbours::sample());
        assert!(!boxes.is_empty(), "{kind:?}");
        let occ = piece.occupancy_with(&Neighbours::sample());
        assert!(volume(&occ) > 0, "{kind:?}");
        // Les boîtes d'une pièce hors raccords ne sont que celles de son occupation.
        if !kind.is_connected() {
            let mut filled = [0u16; 256];
            for b in &piece.int_boxes() {
                grid::fill_box(&mut filled, b);
            }
            assert_eq!(filled, piece.occupancy(), "{kind:?}");
        }
        // Tout ce qu'une pièce rend en objets existe.
        assert!(!piece.drops().is_empty(), "{kind:?}");
    }
}

#[test]
fn cuboid_presets_have_the_documented_sizes() {
    let size = |kind| {
        let Piece::Cuboid { min, max, .. } = Piece::sample(kind) else {
            panic!("{kind:?} devrait être un pavé");
        };
        [0, 1, 2].map(|a| max[a] - min[a])
    };
    assert_eq!(size(PieceKind::Step), [8, 16, 8]);
    assert_eq!(size(PieceKind::Beam), [8, 16, 8]);
    assert_eq!(size(PieceKind::HalfPanel), [16, 8, 2]);
    assert_eq!(size(PieceKind::QuarterPanel), [8, 8, 2]);
    assert_eq!(size(PieceKind::Board), [16, 16, 1]);
    // Chaque forme vaut son volume en objets : rendement par bloc = 4096 / volume d'un objet.
    for (kind, item_volume) in [
        (PieceKind::Step, 1024u32),
        (PieceKind::Beam, 1024),
        (PieceKind::HalfPanel, 256),
        (PieceKind::QuarterPanel, 128),
        (PieceKind::Board, 256),
        (PieceKind::Slice, 16),
        (PieceKind::CornerSlice, 8),
    ] {
        assert_eq!(kind.per_block() * item_volume, 4096, "{kind:?}");
    }
}

#[test]
fn stairs_keep_three_quarters_of_a_block_in_every_orientation() {
    let straight = Piece::Stairs {
        facing: 0,
        ceiling: false,
        vertical: false,
    };
    let base = straight.occupancy();
    assert_eq!(volume(&base), 4096 * 3 / 4);
    for facing in 0..4 {
        for ceiling in [false, true] {
            let piece = Piece::Stairs {
                facing,
                ceiling,
                vertical: false,
            };
            assert_eq!(volume(&piece.occupancy()), 3072);
            assert_eq!(piece.int_boxes().len(), 2);
        }
        let vertical = Piece::Stairs {
            facing,
            ceiling: false,
            vertical: true,
        };
        assert_eq!(volume(&vertical.occupancy()), 3072);
    }
    // La haute marche est du côté de `facing`, retournée au plafond la dalle est en haut.
    let north = Piece::Stairs {
        facing: 0,
        ceiling: false,
        vertical: false,
    }
    .occupancy();
    assert!(is_set(&north, 8, 12, 2) && !is_set(&north, 8, 12, 13));
    assert!(is_set(&north, 8, 2, 13));
    let hanging = Piece::Stairs {
        facing: 0,
        ceiling: true,
        vertical: false,
    }
    .occupancy();
    assert!(
        is_set(&hanging, 8, 13, 13) && is_set(&hanging, 8, 3, 2) && !is_set(&hanging, 8, 3, 13)
    );
    // L'escalier vertical : le quart `facing` est vide, sur toute la hauteur.
    for (facing, x, z) in [(0u8, 2, 2), (1, 13, 2), (2, 13, 13), (3, 2, 13)] {
        let occ = Piece::Stairs {
            facing,
            ceiling: false,
            vertical: true,
        }
        .occupancy();
        assert!(!is_set(&occ, x, 8, z), "quart vide {facing}");
        assert!(is_set(&occ, 15 - x, 8, z) && is_set(&occ, x, 8, 15 - z));
        assert!(is_set(&occ, 15 - x, 8, 15 - z));
    }
    // Un escalier vertical ne se retourne pas.
    assert!(
        !Piece::Stairs {
            facing: 0,
            ceiling: true,
            vertical: true
        }
        .is_valid()
    );
    assert!(
        !Piece::Stairs {
            facing: 4,
            ceiling: false,
            vertical: false
        }
        .is_valid()
    );
}

#[test]
fn every_piece_turns_with_its_occupancy() {
    for kind in PieceKind::ALL {
        let piece = Piece::sample(kind);
        // Un portillon s'oriente selon ses voisins, pas selon sa case : voir le test suivant.
        if kind == PieceKind::Gate {
            continue;
        }
        for k in 0..4u8 {
            assert_eq!(
                piece.turned(k).occupancy(),
                rotated(&piece.occupancy(), k),
                "{kind:?} tourné de {k}"
            );
        }
        assert_eq!(piece.turned(4), piece);
        assert_eq!(piece.turned(1).turned(3), piece);
    }
    // Aussi pour des pièces moins symétriques que les échantillons.
    for piece in [
        Piece::cuboid(PieceKind::Slice, [1, 0, 0], [3, 16, 2], 2),
        Piece::cuboid(PieceKind::Step, [8, 0, 8], [16, 16, 16], 1),
        Piece::cuboid(PieceKind::CornerSlice, [14, 14, 0], [16, 16, 2], 1),
        Piece::Stairs {
            facing: 1,
            ceiling: true,
            vertical: false,
        },
        Piece::Stairs {
            facing: 2,
            ceiling: false,
            vertical: true,
        },
    ] {
        for k in 0..4u8 {
            assert_eq!(piece.turned(k).occupancy(), rotated(&piece.occupancy(), k));
        }
    }
    // Les 24 orientations de pente, avec leurs coins.
    for base in Face::ALL {
        for up in Face::ALL {
            for corner in [Corner::None, Corner::Outer, Corner::Inner] {
                let piece = Piece::Slope {
                    base,
                    up,
                    level: 9,
                    corner,
                };
                if !piece.is_valid() {
                    continue;
                }
                for k in 0..4u8 {
                    assert_eq!(
                        piece.turned(k).occupancy(),
                        rotated(&piece.occupancy(), k),
                        "{base:?} {up:?} {corner:?} {k}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_gate_turns_by_changing_its_axis() {
    let gate = Piece::Connected {
        kind: Link::Gate,
        open: false,
    };
    let along_x = gate.occupancy_with(&Neighbours {
        axis: 0,
        ..Neighbours::NONE
    });
    let along_z = gate.occupancy_with(&Neighbours {
        axis: 1,
        ..Neighbours::NONE
    });
    assert_eq!(rotated(&along_x, 1), along_z);
    assert_eq!(rotated(&along_z, 1), along_x);
}

#[test]
fn half_layers_merge_and_stack_like_layers() {
    let half =
        |min: [u8; 3], max: [u8; 3]| Piece::cuboid(PieceKind::HalfLayer, min, max, max[1] - min[1]);
    let mut s = Shaped::new();
    s.try_add(half([0, 0, 0], [16, 1, 8]), OAK).unwrap();
    s.try_add(half([0, 1, 0], [16, 2, 8]), OAK).unwrap();
    s.try_add(half([0, 2, 0], [16, 3, 8]), OAK).unwrap();
    assert_eq!(s.parts.len(), 1);
    assert_eq!(s.parts[0].piece, half([0, 0, 0], [16, 3, 8]));
    assert_eq!(s.parts[0].piece.drops(), [(PieceKind::HalfLayer, 3)]);
    // L'autre moitié, d'un autre matériau, ne fusionne pas mais tient à côté.
    s.try_add(half([0, 0, 8], [16, 3, 16]), STONE).unwrap();
    assert_eq!(s.parts.len(), 2);
    // Une moitié d'un autre matériau sur la même moitié : refusée.
    assert_eq!(
        s.try_add(half([0, 2, 0], [16, 3, 8]), STONE),
        Err(AddError::Overlap)
    );
    // Les deux moitiés du même matériau, côte à côte, refont une couche entière, qui rend ses deux
    // demi-couches : rien ne se perd.
    let mut t = Shaped::new();
    t.try_add(half([0, 0, 0], [16, 1, 8]), OAK).unwrap();
    t.try_add(half([0, 0, 8], [16, 1, 16]), OAK).unwrap();
    assert_eq!(t.parts.len(), 1);
    assert_eq!(t.parts[0].piece.drops(), [(PieceKind::HalfLayer, 2)]);
    // Deux moitiés qui ne se touchent pas ne fusionnent pas.
    let mut u = Shaped::new();
    u.try_add(half([0, 0, 0], [16, 1, 8]), OAK).unwrap();
    u.try_add(half([0, 2, 0], [16, 3, 8]), OAK).unwrap();
    assert_eq!(u.parts.len(), 2);
}

#[test]
fn boards_on_several_faces_are_trimmed_at_the_edges() {
    let board = |face: Face| {
        let (axis, positive) = grid::face_axis(face);
        let mut min = [0u8; 3];
        let mut max = [16u8; 3];
        (min[axis], max[axis]) = if positive { (15, 16) } else { (0, 1) };
        Piece::cuboid(PieceKind::Board, min, max, 1)
    };
    let mut s = Shaped::new();
    for face in [
        Face::NegY,
        Face::PosY,
        Face::NegX,
        Face::PosX,
        Face::NegZ,
        Face::PosZ,
    ] {
        s.try_add(board(face), OAK).unwrap();
    }
    assert_eq!(s.parts.len(), 6);
    // Six planchettes, qui ne se recouvrent pas : la coque de la case, 16³ - 14³.
    let total: u32 = s.parts.iter().map(|p| volume(&p.piece.occupancy())).sum();
    assert_eq!(total, 4096 - 14 * 14 * 14);
    assert_eq!(volume(&s.occupancy()), total);
    // Une planchette posée deux fois sur la même face : refusée.
    assert_eq!(s.try_add(board(Face::NegY), OAK), Err(AddError::Overlap));
    // Chacune rend sa planchette.
    assert!(
        s.parts
            .iter()
            .all(|p| p.piece.drops() == [(PieceKind::Board, 1)])
    );
}

#[test]
fn slices_grow_without_losing_or_creating_items() {
    for (kind, start, per_step) in [
        (
            PieceKind::Slice,
            Piece::cuboid(PieceKind::Slice, [0, 0, 0], [1, 16, 1], 1),
            slice_cost as fn(u32) -> u32,
        ),
        (
            PieceKind::CornerSlice,
            Piece::cuboid(PieceKind::CornerSlice, [0, 0, 0], [1, 1, 1], 1),
            corner_slice_cost,
        ),
    ] {
        let mut s = Shaped::new();
        s.try_add(start, STONE).unwrap();
        let mut spent = 1u32;
        for n in 2..=8u32 {
            spent += s.try_grow(0, 1_000).unwrap();
            assert_eq!(spent, per_step(n), "{kind:?} à {n}");
            assert_eq!(s.parts[0].piece.drops(), [(kind, spent)], "{kind:?} à {n}");
            assert!(s.parts[0].piece.is_valid());
        }
        // Au plus 8 : au-delà, rien.
        assert_eq!(s.try_grow(0, 1_000), Err(AddError::Invalid));
        assert_eq!(spent, 64);
    }
    // Il faut avoir assez d'objets.
    let mut s = Shaped::new();
    s.try_add(
        Piece::cuboid(PieceKind::Slice, [0, 0, 0], [1, 16, 1], 1),
        STONE,
    )
    .unwrap();
    assert_eq!(s.try_grow(0, 2), Err(AddError::NotEnough));
    // 1 → 2 coûte 3 : (n + 1)² - n².
    assert_eq!(s.try_grow(0, 3), Ok(3));
    // Une tranche de l'arête opposée grandit vers l'intérieur.
    let mut s = Shaped::new();
    s.try_add(
        Piece::cuboid(PieceKind::Slice, [15, 0, 15], [16, 16, 16], 1),
        STONE,
    )
    .unwrap();
    s.try_grow(0, 9).unwrap();
    assert_eq!(
        s.parts[0].piece,
        Piece::cuboid(PieceKind::Slice, [14, 0, 14], [16, 16, 16], 4)
    );
    // Une pièce qui gêne la croissance la refuse.
    s.try_add(
        Piece::cuboid(PieceKind::Slice, [13, 0, 14], [14, 16, 16], 1),
        OAK,
    )
    .unwrap();
    assert_eq!(s.try_grow(0, 99), Err(AddError::Overlap));
    // Ce qui n'est pas une tranche ne grandit pas.
    let mut s = Shaped::new();
    s.try_add(bottom(0, 4), STONE).unwrap();
    assert_eq!(s.try_grow(0, 9), Err(AddError::Invalid));
}

#[test]
fn a_connected_piece_is_alone_in_its_cell() {
    let wall = Piece::Connected {
        kind: Link::Wall,
        open: false,
    };
    let mut s = Shaped::new();
    s.try_add(wall, STONE).unwrap();
    assert_eq!(s.link(), Some(Link::Wall));
    assert_eq!(s.try_add(bottom(0, 4), STONE), Err(AddError::Alone));
    assert_eq!(s.try_add(wall, STONE), Err(AddError::Alone));
    let mut t = Shaped::new();
    t.try_add(bottom(0, 4), STONE).unwrap();
    assert_eq!(t.try_add(wall, STONE), Err(AddError::Alone));
    assert_eq!(t.link(), None);
    // Les octets d'une case avec une forme raccordée et autre chose sont refusés à la lecture.
    let mut bad = t.clone();
    bad.parts.push(Part {
        piece: wall,
        material: 0,
    });
    assert!(Shaped::decode(&bad.encode()).is_none());
    // La lecture légère retrouve la forme.
    assert_eq!(Shaped::link_of(&s.encode()), Some(Link::Wall));
    assert_eq!(Shaped::link_of(&t.encode()), None);
}

#[test]
fn a_connected_piece_changes_its_boxes_with_its_neighbours() {
    let mut s = Shaped::new();
    s.try_add(
        Piece::Connected {
            kind: Link::Wall,
            open: false,
        },
        STONE,
    )
    .unwrap();
    let alone = s.boxes();
    let line = s.boxes_with(&Neighbours::sample());
    assert_eq!(alone.len(), 1);
    assert_eq!(line.len(), 3);
    // La collision d'une barrière est plus haute que sa case.
    let mut f = Shaped::new();
    f.try_add(
        Piece::Connected {
            kind: Link::Fence,
            open: false,
        },
        OAK,
    )
    .unwrap();
    assert_eq!(f.boxes()[0].max.y, 1.0);
    assert_eq!(f.collision_with(&Neighbours::NONE)[0].max.y, 1.5);
}

// ----- Les pentes avancées -----

fn slopes_of_every_orientation(level: u8) -> Vec<Piece> {
    let mut out = Vec::new();
    for base in Face::ALL {
        for up in Face::ALL {
            for corner in [Corner::None, Corner::Outer, Corner::Inner] {
                let piece = Piece::Slope {
                    base,
                    up,
                    level,
                    corner,
                };
                if piece.is_valid() {
                    out.push(piece);
                }
            }
        }
    }
    out
}

#[test]
fn there_are_24_slope_orientations_per_corner_kind() {
    assert_eq!(slopes_of_every_orientation(16).len(), 24 * 3);
    // Deux orientations différentes ne remplissent jamais la même case. (À 45°, une pente est la
    // même que celle dont la base et le haut sont échangés : on prend un autre niveau.)
    let all: Vec<Occupancy> = slopes_of_every_orientation(8)
        .into_iter()
        .filter(|p| {
            matches!(
                p,
                Piece::Slope {
                    corner: Corner::None,
                    ..
                }
            )
        })
        .map(|p| p.occupancy())
        .collect();
    for (i, a) in all.iter().enumerate() {
        assert!(!all[i + 1..].contains(a), "deux pentes identiques");
    }
}

#[test]
fn slope_boxes_cover_the_occupancy_in_every_orientation() {
    for level in [1, 5, 16] {
        for piece in slopes_of_every_orientation(level) {
            let mut filled = [0u16; 256];
            for b in &piece.int_boxes() {
                grid::fill_box(&mut filled, b);
            }
            assert_eq!(filled, piece.occupancy(), "{piece:?}");
            let Piece::Slope { corner, .. } = piece else {
                unreachable!()
            };
            // Au plus 16 groupes de colonnes, deux boîtes chacun en coin.
            let limit = if corner == Corner::None { 16 } else { 32 };
            assert!(piece.int_boxes().len() <= limit, "{piece:?}");
        }
    }
}

#[test]
fn corner_slopes_climb_to_the_apex_and_fill_by_volume() {
    let up = corner_up(Face::NegY, Face::NegZ, Face::NegX).unwrap();
    let corner = |corner, level| Piece::Slope {
        base: Face::NegY,
        up,
        level,
        corner,
    };
    let outer = corner(Corner::Outer, 16).occupancy();
    let inner = corner(Corner::Inner, 16).occupancy();
    let straight = corner(Corner::None, 16).occupancy();
    // Le coin sortant a sa pointe au coin nord-ouest et zéro au coin opposé.
    let height =
        |occ: &Occupancy, x: usize, z: usize| (0..N).filter(|&y| is_set(occ, x, y, z)).count();
    assert_eq!(height(&outer, 0, 0), 16);
    assert_eq!(height(&outer, 15, 15), 1);
    assert_eq!(height(&inner, 15, 15), 1);
    assert_eq!(height(&inner, 15, 0), 16);
    assert_eq!(height(&inner, 0, 15), 16);
    // Le sortant remplit moins que la pente droite, le rentrant plus.
    assert!(volume(&outer) < volume(&straight) && volume(&straight) < volume(&inner));
    // Un tiers et deux tiers d'un bloc, environ.
    let third = 4096.0 / 3.0;
    assert!(
        (volume(&outer) as f32 - third).abs() < 400.0,
        "{}",
        volume(&outer)
    );
    assert!((volume(&inner) as f32 - 2.0 * third).abs() < 400.0);
    // Chaque niveau empile : 16 niveaux de coin sortant font la pente pleine.
    let mut s = Shaped::new();
    for _ in 0..16 {
        s.try_add(corner(Corner::Outer, 1), STONE).unwrap();
    }
    assert_eq!(s.parts.len(), 1);
    assert_eq!(s.parts[0].piece, corner(Corner::Outer, 16));
    assert_eq!(
        s.parts[0].piece.drops(),
        [(PieceKind::CornerSlopeLayer, 16)]
    );
    assert_eq!(
        corner(Corner::Inner, 3).drops(),
        [(PieceKind::InnerSlopeLayer, 3)]
    );
    // Un niveau de coin ne s'empile pas sur une pente droite.
    let mut t = Shaped::new();
    t.try_add(corner(Corner::None, 4), STONE).unwrap();
    assert!(t.try_add(corner(Corner::Outer, 1), STONE).is_err());
}

#[test]
fn version_1_slopes_read_as_floor_slopes() {
    // Une case écrite par la première version : une couche, une pente (est, au sol, niveau 7), une
    // pente (nord, plafond, niveau 3) et un microbloc.
    let mut bytes = vec![1u8, 1, 10];
    bytes.extend(b"core:stone");
    bytes.push(4);
    bytes.extend([0, 0, 3, 0, 2]); // couche : matériau 0, étiquette 0, face 3 (-Y), décalage 0, épaisseur 2
    bytes.extend([0, 1, 1, 0, 7]);
    bytes.extend([0, 1, 0, 1, 3]);
    bytes.push(0);
    bytes.push(2);
    bytes.extend(1u64.to_le_bytes());
    let read = Shaped::decode(&bytes).expect("la version 1 se relit");
    assert_eq!(read.materials, [STONE]);
    assert_eq!(read.parts.len(), 4);
    assert_eq!(read.parts[0].piece, bottom(0, 2));
    assert_eq!(read.parts[1].piece, Piece::floor_slope(1, false, 7));
    assert_eq!(read.parts[2].piece, Piece::floor_slope(0, true, 3));
    assert_eq!(read.parts[3].piece, Piece::Micro { cells: 1 });
    // L'occupation et les boîtes sont celles de la première version : une pente à l'est monte vers +X.
    let occ = read.parts[1].piece.occupancy();
    assert!(is_set(&occ, 15, 6, 8) && !is_set(&occ, 0, 6, 8));
    // On réécrit en version 2, et la relecture est identique.
    let rewritten = read.encode();
    assert_eq!(rewritten[0], 2);
    assert_eq!(Shaped::decode(&rewritten).unwrap(), read);
    // La version 2 ne connaît plus l'étiquette 1 des pentes anciennes.
    let mut v2 = rewritten.clone();
    v2[0] = 2;
    let mut bogus = vec![2u8, 1, 1, b'a', 1, 0, 1, 0, 1, 3];
    assert!(Shaped::decode(&bogus).is_none());
    bogus[0] = 1;
    assert!(Shaped::decode(&bogus).is_some());
}

#[test]
fn version_2_round_trips_every_piece() {
    let mut s = Shaped::new();
    s.try_add(bottom(0, 2), OAK).unwrap();
    s.try_add(
        Piece::Slope {
            base: Face::PosX,
            up: Face::NegY,
            level: 6,
            corner: Corner::None,
        },
        STONE,
    )
    .unwrap_or(());
    for kind in PieceKind::ALL {
        if kind.is_connected() {
            continue;
        }
        let mut one = Shaped::new();
        one.try_add(Piece::sample(kind), "mod:inconnu").unwrap();
        let bytes = one.encode();
        assert_eq!(Shaped::decode(&bytes).unwrap(), one, "{kind:?}");
        assert_eq!(Shaped::boxes_of(&bytes).unwrap(), one.boxes(), "{kind:?}");
        for cut in 0..bytes.len() {
            assert!(Shaped::decode(&bytes[..cut]).is_none(), "{kind:?} {cut}");
        }
    }
    for link in Link::ALL {
        for open in [false, true] {
            let mut one = Shaped::new();
            one.try_add(Piece::Connected { kind: link, open }, STONE)
                .unwrap();
            assert_eq!(Shaped::decode(&one.encode()).unwrap(), one);
        }
    }
    // Des valeurs hors bornes ne se lisent pas.
    for bad in [
        vec![2u8, 1, 1, b'a', 1, 0, 4, 8, 8, 8, 4, 4, 4, 9, 1], // pavé de min > max
        vec![2, 1, 1, b'a', 1, 0, 4, 0, 0, 0, 4, 4, 4, 200, 1], // sorte inconnue
        vec![2, 1, 1, b'a', 1, 0, 4, 0, 0, 0, 4, 4, 4, 9, 0],   // aucun objet
        vec![2, 1, 1, b'a', 1, 0, 4, 0, 0, 0, 17, 4, 4, 9, 1],  // hors de la case
        vec![2, 1, 1, b'a', 1, 0, 6, 9, 0],                     // forme raccordée inconnue
        vec![2, 1, 1, b'a', 1, 0, 3, 3, 3, 5, 0], // pente dont base et haut sont alignés
        vec![2, 1, 1, b'a', 1, 0, 3, 3, 0, 5, 7], // coin inconnu
        vec![2, 1, 1, b'a', 1, 0, 5, 7, 0, 0],    // escalier de facing 7
    ] {
        assert!(Shaped::decode(&bad).is_none(), "{bad:?}");
    }
}

#[test]
fn turned_data_works_on_the_new_pieces() {
    let mut s = Shaped::new();
    s.try_add(Piece::sample(PieceKind::Stairs), OAK).unwrap();
    s.try_add(
        Piece::cuboid(PieceKind::Beam, [4, 0, 4], [12, 16, 12], 1),
        STONE,
    )
    .unwrap_err();
    let data = s.encode();
    assert_eq!(
        Shaped::decode(&Shaped::turned_data(&data, 1))
            .unwrap()
            .parts[0]
            .piece,
        Piece::Stairs {
            facing: 1,
            ceiling: false,
            vertical: false
        }
    );
}

// ----- Poser -----

#[test]
fn plans_pick_the_quarter_of_the_aimed_cell() {
    let look = Vec3::new(0.0, 0.0, 1.0);
    // Une tranche sur le dessus d'un bloc, vers le coin nord-est : une barre debout dans ce coin.
    let slice = plan_outer(PieceKind::Slice, Vec3::new(0.9, 1.0, 0.1), IVec3::Y, look);
    assert_eq!(
        slice,
        Piece::cuboid(PieceKind::Slice, [15, 0, 0], [16, 16, 1], 1)
    );
    // Une tranche de coin au même endroit : le cube du coin (haut est, bas, nord).
    let corner = plan_outer(
        PieceKind::CornerSlice,
        Vec3::new(0.9, 1.0, 0.1),
        IVec3::Y,
        look,
    );
    assert_eq!(
        corner,
        Piece::cuboid(PieceKind::CornerSlice, [15, 0, 0], [16, 1, 1], 1)
    );
    // Une marche verticale : un quart de colonne dans le coin visé.
    let step = plan_outer(PieceKind::Step, Vec3::new(0.2, 1.0, 0.8), IVec3::Y, look);
    assert_eq!(
        step,
        Piece::cuboid(PieceKind::Step, [0, 0, 8], [8, 16, 16], 1)
    );
    // Une poutre : toujours au centre.
    let beam = plan_outer(PieceKind::Beam, Vec3::new(0.2, 1.0, 0.8), IVec3::Y, look);
    assert_eq!(
        beam,
        Piece::cuboid(PieceKind::Beam, [4, 0, 4], [12, 16, 12], 1)
    );
    // Visant le côté d'un mur (normale +X), la poutre se couche le long du mur (axe Z).
    let lying = plan_outer(PieceKind::Beam, Vec3::new(1.0, 0.5, 0.5), IVec3::X, look);
    assert_eq!(
        lying,
        Piece::cuboid(PieceKind::Beam, [4, 4, 0], [12, 12, 16], 1)
    );
    // Un escalier : la haute marche est du côté du regard, retourné sous un plafond.
    let stairs = plan_outer(PieceKind::Stairs, Vec3::new(0.5, 1.0, 0.5), IVec3::Y, look);
    assert_eq!(
        stairs,
        Piece::Stairs {
            facing: 2,
            ceiling: false,
            vertical: false
        }
    );
    let hanging = plan_outer(
        PieceKind::Stairs,
        Vec3::new(0.5, 0.0, 0.5),
        IVec3::NEG_Y,
        look,
    );
    assert!(matches!(hanging, Piece::Stairs { ceiling: true, .. }));
    // Un escalier vertical : le quart visé est le quart vide.
    let vertical = plan_outer(
        PieceKind::VerticalStairs,
        Vec3::new(0.9, 1.0, 0.1),
        IVec3::Y,
        look,
    );
    assert_eq!(
        vertical,
        Piece::Stairs {
            facing: 1,
            ceiling: false,
            vertical: true
        }
    );
    // Les panneaux se collent à la face de la case voisine, du côté où l'on a visé.
    let half = plan_outer(
        PieceKind::HalfPanel,
        Vec3::new(1.0, 0.5, 0.8),
        IVec3::X,
        look,
    );
    assert_eq!(
        half,
        Piece::cuboid(PieceKind::HalfPanel, [0, 0, 8], [2, 16, 16], 1)
    );
    let quarter = plan_outer(
        PieceKind::QuarterPanel,
        Vec3::new(1.0, 0.2, 0.8),
        IVec3::X,
        look,
    );
    assert_eq!(
        quarter,
        Piece::cuboid(PieceKind::QuarterPanel, [0, 0, 8], [2, 8, 16], 1)
    );
    let board = plan_outer(PieceKind::Board, Vec3::new(1.0, 0.5, 0.5), IVec3::X, look);
    assert_eq!(
        board,
        Piece::cuboid(PieceKind::Board, [0, 0, 0], [1, 16, 16], 1)
    );
    // Une demi-couche sur le dessus d'une dalle, côté sud : une moitié d'une couche.
    let mut s = Shaped::new();
    s.try_add(bottom(0, 8), STONE).unwrap();
    let layer = plan_inner(
        PieceKind::HalfLayer,
        &s,
        Vec3::new(0.5, 0.5, 0.9),
        IVec3::Y,
        look,
    )
    .unwrap();
    assert_eq!(
        layer,
        Piece::cuboid(PieceKind::HalfLayer, [0, 8, 8], [16, 9, 16], 1)
    );
    s.try_add(layer, OAK).unwrap();
    // Une autre demi-couche posée sur la précédente la rend plus épaisse.
    let more = plan_inner(
        PieceKind::HalfLayer,
        &s,
        Vec3::new(0.5, 9.0 / 16.0, 0.9),
        IVec3::Y,
        look,
    )
    .unwrap();
    s.try_add(more, OAK).unwrap();
    assert_eq!(s.parts.len(), 2);
    assert_eq!(
        s.parts[1].piece,
        Piece::cuboid(PieceKind::HalfLayer, [0, 8, 8], [16, 10, 16], 2)
    );
    // Les formes raccordées vont toujours dans la case voisine.
    assert!(
        plan_inner(
            PieceKind::Wall,
            &s,
            Vec3::new(0.5, 0.5, 0.5),
            IVec3::Y,
            look
        )
        .is_none()
    );
    assert_eq!(
        plan_outer(PieceKind::Gate, Vec3::new(0.5, 1.0, 0.5), IVec3::Y, look),
        Piece::Connected {
            kind: Link::Gate,
            open: false
        }
    );
}

#[test]
fn slopes_stick_to_the_aimed_wall_or_the_floor() {
    let look = Vec3::new(0.0, 0.0, 1.0);
    // Sur un mur (normale +X), au milieu de la face : une pente murale collée au mur (base -X de la
    // case voisine), qui monte vers le bord le plus proche du point visé (ici le haut).
    let wall = plan_outer(PieceKind::Slope, Vec3::new(1.0, 0.6, 0.5), IVec3::X, look);
    assert_eq!(
        wall,
        Piece::Slope {
            base: Face::NegX,
            up: Face::PosY,
            level: 16,
            corner: Corner::None
        }
    );
    // Vers le bas de la face : une pente au sol, comme avant.
    let floor = plan_outer(PieceKind::Slope, Vec3::new(1.0, 0.1, 0.5), IVec3::X, look);
    assert_eq!(floor, Piece::floor_slope(2, false, 16));
    // Vers le haut de la face : au plafond.
    let ceiling = plan_outer(PieceKind::Slope, Vec3::new(1.0, 0.9, 0.5), IVec3::X, look);
    assert_eq!(ceiling, Piece::floor_slope(2, true, 16));
    // Un coin sortant sur le dessus d'un bloc : sa pointe est dans le quart visé.
    let corner = plan_outer(
        PieceKind::CornerSlopeLayer,
        Vec3::new(0.9, 1.0, 0.1),
        IVec3::Y,
        look,
    );
    let Piece::Slope {
        base,
        up,
        level,
        corner: kind,
    } = corner
    else {
        panic!("une pente");
    };
    assert_eq!((base, level, kind), (Face::NegY, 1, Corner::Outer));
    // Les deux bords hauts sont l'est et le nord.
    assert_eq!(corner_up(Face::NegY, Face::PosX, Face::NegZ), Some(up));
    let inner = plan_outer(
        PieceKind::InnerSlopeLayer,
        Vec3::new(0.1, 0.0, 0.9),
        IVec3::NEG_Y,
        look,
    );
    assert!(matches!(
        inner,
        Piece::Slope {
            base: Face::PosY,
            corner: Corner::Inner,
            level: 1,
            ..
        }
    ));
    // Un coin sur un mur : la base est le mur, les deux bords hauts sont ceux du quart visé.
    let wall_corner = plan_outer(
        PieceKind::CornerSlopeLayer,
        Vec3::new(1.0, 0.6, 0.9),
        IVec3::X,
        look,
    );
    let Piece::Slope { base, up, .. } = wall_corner else {
        panic!("une pente");
    };
    assert_eq!(base, Face::NegX);
    assert_eq!(corner_up(base, Face::PosY, Face::PosZ), Some(up));
}

#[test]
fn a_neighbour_that_changes_remeshes_the_connected_cell_even_across_a_chunk() {
    use crate::streaming::{StreamEvent, StreamingConfig};
    use crate::worldgen::generator::TerrainGenerator;
    let config = StreamingConfig {
        view_radius: 1,
        ..StreamingConfig::default()
    };
    let mut s = crate::ChunkStreamer::new(TerrainGenerator::new(7), config);
    let player = Vec3::new(5.0, 100.0, 5.0);
    // Les maillages envoyés pour un chunk, jusqu'à ce que le streamer n'ait plus rien à faire.
    fn settle(
        s: &mut crate::ChunkStreamer,
        player: Vec3,
        chunk: IVec3,
    ) -> Option<crate::ChunkMesh> {
        let mut last = None;
        for _ in 0..20_000 {
            s.update(player);
            while let Some(event) = s.next_event() {
                if let StreamEvent::Mesh(p, mesh) = event
                    && p == chunk
                {
                    last = Some(mesh);
                }
            }
            if s.is_idle() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        last
    }
    settle(&mut s, player, IVec3::ZERO);
    // Deux murets de part et d'autre de la frontière des chunks x = 31 / x = 32.
    let (a, b) = (IVec3::new(31, 140, 8), IVec3::new(32, 140, 8));
    let chunk = IVec3::new(0, 4, 0);
    let mut wall = Shaped::new();
    wall.try_add(
        Piece::Connected {
            kind: Link::Wall,
            open: false,
        },
        STONE,
    )
    .unwrap();
    assert!(store(&mut s, a, &wall));
    let alone = settle(&mut s, player, chunk).expect("le chunk est maillé");
    // Seul : le poteau, six faces.
    assert_eq!(alone.triangle_count(), 12);
    assert!(store(&mut s, b, &wall));
    let joined =
        settle(&mut s, player, chunk).expect("le chunk est remaillé quand son voisin change");
    // Le muret a maintenant un bras vers l'est : le poteau (6 faces) et le bras (dessus, dessous et
    // deux côtés ; le bout touche le bras de l'autre muret, dans l'autre chunk).
    assert_eq!(joined.triangle_count(), 20);
    // Et quand le voisin disparaît, le bras aussi.
    assert!(store(&mut s, b, &Shaped::new()));
    let alone_again = settle(&mut s, player, chunk).expect("remaillé");
    assert_eq!(alone_again.triangle_count(), 12);
}
