//! Les formes (`core:shaped`) dans le monde : boîtes, visée, physique et maillage.
use engine_voxel::shaped::{Piece, Shaped};
use engine_voxel::{
    BlockEntity, BlockRegistry, ChunkMesh, Face, PlayerBody, World, mesh_binary, mesh_culled,
    mesh_greedy, raycast,
};
use glam::{IVec3, Vec3};
use std::{path::Path, sync::Arc};

fn registry() -> Arc<BlockRegistry> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap())
}

const CELL: IVec3 = IVec3::new(5, 5, 5);

/// Un monde où la case `CELL` contient `shaped`.
fn world_with(shaped: &Shaped) -> World {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    place(&mut world, CELL, shaped);
    world
}

fn place(world: &mut World, pos: IVec3, shaped: &Shaped) {
    let block = world.registry().id("core:shaped").expect("core:shaped");
    world.set_block(pos, block);
    let (chunk, [x, y, z]) = World::split(pos);
    world.chunk_mut(chunk).set_block_entity(
        x,
        y,
        z,
        Some(BlockEntity {
            data: shaped.encode(),
            ..BlockEntity::default()
        }),
    );
}

fn slab(face: Face, offset: u8, thickness: u8) -> Piece {
    Piece::Layer {
        face,
        offset,
        thickness,
    }
}

fn quads(mesh: &ChunkMesh) -> usize {
    mesh.triangle_count() / 2
}

fn meshes(world: &World) -> [ChunkMesh; 3] {
    let chunk = IVec3::ZERO;
    [
        mesh_culled(world, chunk),
        mesh_greedy(world, chunk),
        mesh_binary(world, chunk),
    ]
}

#[test]
fn the_boxes_come_from_the_content() {
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:stone").unwrap();
    let world = world_with(&s);
    let boxes = world.block_boxes(CELL);
    assert_eq!(boxes.len(), 1);
    assert_eq!((boxes[0].min.y, boxes[0].max.y), (0.0, 0.5));
    // Une case sans contenu lisible garde sa boîte pleine.
    let mut broken = world_with(&s);
    let (chunk, [x, y, z]) = World::split(CELL);
    broken
        .chunk_mut(chunk)
        .block_entity_mut(x, y, z)
        .unwrap()
        .data = vec![42];
    assert_eq!(broken.block_boxes(CELL).len(), 1);
    assert_eq!(broken.block_boxes(CELL)[0].max.y, 1.0);
}

#[test]
fn the_ray_hits_the_piece_not_the_empty_half() {
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:stone").unwrap();
    let world = world_with(&s);
    let from = CELL.as_vec3() + Vec3::new(0.5, 3.0, 0.5);
    let hit = raycast(&world, from, Vec3::NEG_Y, 6.0).unwrap();
    assert_eq!(hit.block_pos, CELL);
    assert_eq!(hit.normal, IVec3::Y);
    // La dalle est à mi-hauteur : on la touche 2,5 blocs plus bas, pas 2.
    assert!((hit.distance - 2.5).abs() < 1e-4, "{}", hit.distance);
    // Le contour de visée est celui de la pièce.
    let point = from + Vec3::NEG_Y * hit.distance;
    let (min, max) = world.selection_bounds(CELL, point);
    assert!((max.y - min.y - 0.5).abs() < 1e-5);
    // Un rayon qui passe au-dessus de la dalle, de côté, la manque.
    let side = raycast(
        &world,
        CELL.as_vec3() + Vec3::new(-2.0, 0.75, 0.5),
        Vec3::X,
        6.0,
    );
    assert!(side.is_none());
}

#[test]
fn the_player_walks_up_a_slope() {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    let stone = r.id("core:stone").unwrap();
    for x in 0..8 {
        for z in 0..4 {
            world.set_block(IVec3::new(x, 0, z), stone);
        }
    }
    // Une pente pleine dont le bord haut est à l'est (+X) : on la monte en marchant vers l'est.
    let mut s = Shaped::new();
    s.try_add(
        Piece::Slope {
            facing: 1,
            ceiling: false,
            level: 16,
        },
        "core:stone",
    )
    .unwrap();
    place(&mut world, IVec3::new(4, 1, 2), &s);
    for x in 5..8 {
        world.set_block(IVec3::new(x, 1, 2), stone);
    }
    let mut body = PlayerBody::new(Vec3::new(1.5, 1.0, 2.5));
    for _ in 0..40 {
        body.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
    }
    assert!(body.on_ground);
    for _ in 0..180 {
        body.step(&world, Vec3::new(2.0, 0.0, 0.0), false, 1.0 / 60.0);
    }
    // Arrêté par le mur de pierre au bout, mais monté sur la pente : plus haut que le sol.
    assert!(body.position.x > 4.0, "x = {}", body.position.x);
    assert!(body.position.y > 1.5, "y = {}", body.position.y);
}

#[test]
fn a_slab_has_six_faces_and_hides_the_one_on_a_wall() {
    let r = registry();
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:stone").unwrap();
    let world = world_with(&s);
    for mesh in meshes(&world) {
        assert_eq!(quads(&mesh), 6);
        assert!(mesh.translucent_indices.is_empty());
    }
    // Posée sur de la pierre : le dessous disparaît.
    let mut world = world_with(&s);
    world.set_block(CELL - IVec3::Y, r.id("core:stone").unwrap());
    for mesh in meshes(&world) {
        // Cinq faces pour la dalle, six pour le bloc dessous : une case de formes est transparente,
        // le bloc garde sa face haute.
        assert_eq!(quads(&mesh), 5 + 6);
    }
}

#[test]
fn a_slope_has_an_inclined_face_a_wall_a_floor_and_two_triangles() {
    let mut s = Shaped::new();
    s.try_add(
        Piece::Slope {
            facing: 0,
            ceiling: false,
            level: 16,
        },
        "core:oak_planks",
    )
    .unwrap();
    let world = world_with(&s);
    for mesh in meshes(&world) {
        // Trois quads et deux triangles : 3 × 2 + 2 triangles.
        assert_eq!(mesh.triangle_count(), 8);
        // La face inclinée a une normale qui penche.
        let tilted = mesh
            .vertices
            .iter()
            .filter(|v| v.normal().y > 0.5 && v.normal().z > 0.5)
            .count();
        assert_eq!(tilted, 4);
        // Tous les sommets restent dans la case.
        for v in &mesh.vertices {
            let p = mesh.world_position(v) - CELL.as_vec3();
            assert!(p.cmpge(Vec3::splat(-1e-3)).all() && p.cmple(Vec3::splat(1.001)).all());
        }
    }
    // Le plan monte de 0 (bord bas, côté sud) à 1 (bord haut, côté nord).
    let mesh = &meshes(&world)[0];
    let top = mesh
        .vertices
        .iter()
        .filter(|v| v.normal().y > 0.5 && v.normal().z > 0.5);
    for v in top {
        let p = mesh.world_position(v) - CELL.as_vec3();
        assert!((p.y - (1.0 - p.z)).abs() < 1e-3, "{p:?}");
    }
}

#[test]
fn two_materials_fill_a_cell_without_inner_faces() {
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:stone").unwrap();
    s.try_add(slab(Face::PosY, 0, 8), "core:oak_planks")
        .unwrap();
    assert!(s.is_full());
    let world = world_with(&s);
    for mesh in meshes(&world) {
        // Deux dalles de 5 faces visibles chacune (la face de contact est cachée).
        assert_eq!(quads(&mesh), 10);
        // Chaque matériau a ses textures : au moins deux couches différentes.
        let mut layers: Vec<u32> = mesh
            .vertices
            .iter()
            .map(|v| v.texture & engine_voxel::TEXTURE_MASK)
            .collect();
        layers.sort_unstable();
        layers.dedup();
        assert!(layers.len() >= 2, "{layers:?}");
    }
}

#[test]
fn glass_goes_to_the_translucent_pass_and_unknown_materials_still_draw() {
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:glass").unwrap();
    let world = world_with(&s);
    let mesh = mesh_culled(&world, IVec3::ZERO);
    assert!(mesh.indices.is_empty() && !mesh.translucent_indices.is_empty());
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "mod:disparu").unwrap();
    let world = world_with(&s);
    assert_eq!(quads(&mesh_culled(&world, IVec3::ZERO)), 6);
}

#[test]
fn micro_blocks_are_small_boxes() {
    let mut s = Shaped::new();
    s.try_add(
        Piece::Micro {
            cells: engine_voxel::shaped::micro_mask(0, 0, 0, 2),
        },
        "core:stone",
    )
    .unwrap();
    let world = world_with(&s);
    for mesh in meshes(&world) {
        assert_eq!(quads(&mesh), 6);
        let (min, max) = mesh.bounds().unwrap();
        assert!(((max - min) - Vec3::splat(0.5)).abs().max_element() < 1e-3);
    }
}

#[test]
fn neighbouring_shaped_cells_hide_the_faces_between_them() {
    let mut s = Shaped::new();
    s.try_add(slab(Face::NegY, 0, 8), "core:stone").unwrap();
    let mut world = world_with(&s);
    place(&mut world, CELL + IVec3::X, &s);
    for mesh in meshes(&world) {
        // Deux dalles côte à côte : dix faces, pas douze.
        assert_eq!(quads(&mesh), 10);
    }
    // Une dalle haute contre une dalle basse : les deux faces se voient en partie, donc restent.
    let mut high = Shaped::new();
    high.try_add(slab(Face::PosY, 0, 8), "core:stone").unwrap();
    let mut world = world_with(&s);
    place(&mut world, CELL + IVec3::X, &high);
    for mesh in meshes(&world) {
        assert_eq!(quads(&mesh), 12);
    }
}
