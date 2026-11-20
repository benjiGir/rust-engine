//! Les formes (`core:shaped`) dans le monde : boîtes, visée, physique et maillage.
use engine_voxel::shaped::{Corner, Link, Piece, PieceKind, Shaped};
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
    s.try_add(Piece::floor_slope(1, false, 16), "core:stone")
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
    s.try_add(Piece::floor_slope(0, false, 16), "core:oak_planks")
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

// ----- Escaliers, pavés, pentes en coin -----

fn mesh_of(shaped: &Shaped) -> [ChunkMesh; 3] {
    meshes(&world_with(shaped))
}

#[test]
fn stairs_are_two_boxes_with_hidden_contact_faces() {
    let mut s = Shaped::new();
    s.try_add(Piece::sample(PieceKind::Stairs), "core:stone")
        .unwrap();
    for mesh in mesh_of(&s) {
        // La dalle : ses six faces (le dessus reste entier sous la marche) ; la marche : cinq (son
        // dessous touche la dalle).
        assert_eq!(quads(&mesh), 11);
    }
    // Un escalier vertical : une colonne de 8 × 16 × 16 et une de 8 × 16 × 8, dont les faces de contact
    // disparaissent en partie.
    let mut v = Shaped::new();
    v.try_add(Piece::sample(PieceKind::VerticalStairs), "core:stone")
        .unwrap();
    for mesh in mesh_of(&v) {
        assert_eq!(quads(&mesh), 11);
    }
    // Une poutre, une marche, une tranche : une boîte, six faces.
    for kind in [
        PieceKind::Beam,
        PieceKind::Step,
        PieceKind::Slice,
        PieceKind::CornerSlice,
        PieceKind::HalfPanel,
        PieceKind::QuarterPanel,
        PieceKind::Board,
        PieceKind::HalfLayer,
    ] {
        let mut s = Shaped::new();
        s.try_add(Piece::sample(kind), "core:oak_planks").unwrap();
        for mesh in mesh_of(&s) {
            assert_eq!(quads(&mesh), 6, "{kind:?}");
        }
    }
}

#[test]
fn corner_slopes_are_made_of_triangles() {
    let up = engine_voxel::shaped::corner_up(Face::NegY, Face::NegZ, Face::NegX).unwrap();
    let slope = |corner| Piece::Slope {
        base: Face::NegY,
        up,
        level: 16,
        corner,
    };
    let triangles = |corner| {
        let mut s = Shaped::new();
        s.try_add(slope(corner), "core:oak_planks").unwrap();
        mesh_of(&s).map(|m| m.triangle_count())
    };
    // Sortant : deux triangles inclinés, deux triangles de côté, un fond en quad.
    assert_eq!(triangles(Corner::Outer), [6, 6, 6]);
    // Rentrant : deux triangles inclinés, deux murs en quads, deux triangles de côté, un fond.
    assert_eq!(triangles(Corner::Inner), [10, 10, 10]);
    // Les sommets restent dans la case, et le plan incliné d'un coin sortant monte jusqu'à la pointe.
    let mut s = Shaped::new();
    s.try_add(slope(Corner::Outer), "core:oak_planks").unwrap();
    let mesh = &mesh_of(&s)[0];
    let mut apex = f32::MIN;
    for v in &mesh.vertices {
        let p = mesh.world_position(v) - CELL.as_vec3();
        assert!(p.cmpge(Vec3::splat(-1e-3)).all() && p.cmple(Vec3::splat(1.001)).all());
        if v.normal().y > 0.5 {
            apex = apex.max(p.y);
        }
    }
    assert!((apex - 1.0).abs() < 1e-3, "{apex}");
    // Les normales du plan incliné penchent vers deux directions.
    let tilted: Vec<_> = mesh
        .vertices
        .iter()
        .filter(|v| v.normal().y > 0.5)
        .collect();
    assert!(tilted.iter().any(|v| v.normal().x.abs() > 0.3));
    assert!(tilted.iter().any(|v| v.normal().z.abs() > 0.3));
}

#[test]
fn a_wall_slope_leans_on_its_wall() {
    // Une pente de base -X (collée au mur ouest de la case), qui monte vers le haut.
    let mut s = Shaped::new();
    s.try_add(
        Piece::Slope {
            base: Face::NegX,
            up: Face::PosY,
            level: 16,
            corner: Corner::None,
        },
        "core:stone",
    )
    .unwrap();
    for mesh in mesh_of(&s) {
        assert_eq!(mesh.triangle_count(), 8);
        let tilted = mesh
            .vertices
            .iter()
            .filter(|v| v.normal().x > 0.5 && v.normal().y < -0.5)
            .count();
        // Le plan incliné regarde vers +X et vers le bas (le bord haut est en haut, côté -X).
        assert!(tilted == 0 || tilted == 4);
    }
    let world = world_with(&s);
    let boxes = world.block_boxes(CELL);
    // Le mur occupe la moitié de la case : ses boîtes sont des marches contre x = 0.
    assert_eq!(boxes.len(), 16);
    assert!(boxes.iter().all(|b| b.min.x == 0.0));
}

// ----- Les formes raccordées -----

const GLASS: &str = "core:glass";
const STONE: &str = "core:stone";

fn connected(link: Link, open: bool) -> Shaped {
    let mut s = Shaped::new();
    s.try_add(Piece::Connected { kind: link, open }, STONE)
        .unwrap();
    s
}

/// Un monde où le muret `link` est en `CELL`, entouré des cellules `around` : (décalage, case).
fn world_around(centre: &Shaped, around: &[(IVec3, &Shaped)]) -> World {
    let mut world = world_with(centre);
    for (offset, shaped) in around {
        place(&mut world, CELL + *offset, shaped);
    }
    world
}

#[test]
fn a_wall_follows_its_neighbours() {
    let wall = connected(Link::Wall, false);
    let count = |around: &[(IVec3, &Shaped)]| world_around(&wall, around).block_boxes(CELL).len();
    // 0 voisin : le poteau.
    assert_eq!(count(&[]), 1);
    // 1 voisin : poteau et bras.
    assert_eq!(count(&[(IVec3::X, &wall)]), 2);
    // 2 en ligne : poteau et deux bras.
    assert_eq!(count(&[(IVec3::X, &wall), (IVec3::NEG_X, &wall)]), 3);
    // 2 en coin : idem, avec un poteau plus haut.
    let corner = world_around(&wall, &[(IVec3::X, &wall), (IVec3::Z, &wall)]);
    let boxes = corner.block_boxes(CELL);
    assert_eq!(boxes.len(), 3);
    assert_eq!(boxes[0].max.y, 1.0);
    let line = world_around(&wall, &[(IVec3::X, &wall), (IVec3::NEG_X, &wall)]);
    assert_eq!(line.block_boxes(CELL)[0].max.y, 14.0 / 16.0);
    // 4 voisins : quatre bras.
    assert_eq!(
        count(&[
            (IVec3::X, &wall),
            (IVec3::NEG_X, &wall),
            (IVec3::Z, &wall),
            (IVec3::NEG_Z, &wall)
        ]),
        5
    );
    // Un cube de pierre plein s'accroche aussi ; une planche aussi ; l'air et le verre, non.
    let r = registry();
    let mut world = world_with(&wall);
    world.set_block(CELL + IVec3::X, r.id("core:stone").unwrap());
    assert_eq!(world.block_boxes(CELL).len(), 2);
    world.set_block(CELL + IVec3::X, r.id("core:glass").unwrap());
    assert_eq!(world.block_boxes(CELL).len(), 1);
    // Une barrière à côté d'un muret : pas de raccord (autre famille).
    let fence = connected(Link::Fence, false);
    assert_eq!(count(&[(IVec3::X, &fence)]), 1);
    // Un portillon, si : un muret se raccorde à lui par l'axe que le portillon ferme.
    let gate = connected(Link::Gate, false);
    assert_eq!(count(&[(IVec3::X, &gate)]), 2);
    assert_eq!(count(&[(IVec3::Z, &gate), (IVec3::Z * 2, &wall)]), 2);
}

#[test]
fn a_fence_blocks_up_to_a_block_and_a_half_and_an_open_gate_does_not() {
    let fence = connected(Link::Fence, false);
    let world = world_with(&fence);
    // La visée reste dans la case ; la collision monte à 1,5.
    assert_eq!(world.block_boxes(CELL)[0].max.y, 1.0);
    let collision = world.collision_boxes(CELL).unwrap();
    assert_eq!(collision[0].max.y, 1.5);
    let gate = world_with(&connected(Link::Gate, false));
    assert!(gate.collision_boxes(CELL).is_some_and(|b| !b.is_empty()));
    let open = world_with(&connected(Link::Gate, true));
    // Ouvert : on traverse, mais on peut toujours le viser (pour le refermer).
    assert!(open.collision_boxes(CELL).is_none());
    assert!(!open.block_boxes(CELL).is_empty());
    // Un cube ordinaire garde ses boîtes.
    let mut plain = World::with_registry(registry());
    let stone = plain.registry().id(STONE).unwrap();
    plain.set_block(CELL, stone);
    assert!(plain.collision_boxes(CELL).is_some_and(|b| b.is_empty()));
}

/// Un sol de pierre et une rangée de cases `piece` à x = 4, sur toute la profondeur.
fn field_with_row(piece: &Shaped) -> World {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    let stone = r.id(STONE).unwrap();
    for x in 0..10 {
        for z in 0..6 {
            world.set_block(IVec3::new(x, 0, z), stone);
        }
    }
    for z in 0..6 {
        place(&mut world, IVec3::new(4, 1, z), piece);
    }
    world
}

fn run_east(world: &World, jump: bool) -> f32 {
    let mut body = PlayerBody::new(Vec3::new(1.5, 1.0, 2.5));
    for _ in 0..40 {
        body.step(world, Vec3::ZERO, false, 1.0 / 60.0);
    }
    for _ in 0..240 {
        body.step(world, Vec3::new(3.0, 0.0, 0.0), jump, 1.0 / 60.0);
    }
    body.position.x
}

#[test]
fn nobody_jumps_over_a_fence_but_a_wall_and_an_open_gate_are_crossed() {
    // Une barrière (24 seizièmes) arrête même un joueur qui saute sans cesse.
    let fence = field_with_row(&connected(Link::Fence, false));
    // (Le poteau est au milieu de la case, à x = 4,4 : le joueur, large de 0,6, s'arrête à 4,1.)
    assert!(
        run_east(&fence, true) < 4.5,
        "x = {}",
        run_east(&fence, true)
    );
    assert!(run_east(&fence, false) < 4.5);
    // Un muret (14 seizièmes) se franchit en sautant, pas en marchant.
    let wall = field_with_row(&connected(Link::Wall, false));
    assert!(run_east(&wall, true) > 5.0, "x = {}", run_east(&wall, true));
    assert!(run_east(&wall, false) < 4.5);
    // Un portillon fermé arrête ; ouvert, on passe en marchant.
    let closed = field_with_row(&connected(Link::Gate, false));
    assert!(run_east(&closed, true) < 4.5);
    let open = field_with_row(&connected(Link::Gate, true));
    assert!(
        run_east(&open, false) > 5.0,
        "x = {}",
        run_east(&open, false)
    );
}

#[test]
fn a_pane_alone_and_connected_has_the_right_faces() {
    let mut glass = Shaped::new();
    glass
        .try_add(
            Piece::Connected {
                kind: Link::Pane,
                open: false,
            },
            GLASS,
        )
        .unwrap();
    // Seule : un poteau de six faces, dans la passe translucide.
    let alone = world_with(&glass);
    for mesh in meshes(&alone) {
        assert_eq!(quads(&mesh), 6);
        assert!(mesh.indices.is_empty() && !mesh.translucent_indices.is_empty());
    }
    // Avec un voisin de chaque côté : le poteau et deux bras, dont les bouts touchent le voisin.
    let middle = world_around(&glass, &[(IVec3::X, &glass), (IVec3::NEG_X, &glass)]);
    // Trois vitres en ligne : 9 + 12 + 9 faces, contre 18 si elles ne se raccordaient pas.
    for mesh in meshes(&middle) {
        assert_eq!(quads(&mesh), 9 + 12 + 9);
    }
    // Les vitres en opaque sont une simple grille pleine : ici de la pierre, dans la passe opaque.
    let stone = connected(Link::Pane, false);
    for mesh in meshes(&world_with(&stone)) {
        assert!(!mesh.indices.is_empty() && mesh.translucent_indices.is_empty());
    }
}

#[test]
fn two_neighbouring_walls_hide_the_ends_of_their_arms() {
    let wall = connected(Link::Wall, false);
    let world = world_around(&wall, &[(IVec3::X, &wall)]);
    for mesh in meshes(&world) {
        // Un poteau (6 faces) et un bras (4 : le bout touche l'autre bras, la face contre le poteau est
        // cachée), deux fois : 20 faces, pas 22.
        assert_eq!(quads(&mesh), 20);
    }
    // Un muret seul : six faces.
    for mesh in meshes(&world_with(&wall)) {
        assert_eq!(quads(&mesh), 6);
    }
}

#[test]
fn a_flat_pane_is_a_thin_sheet() {
    let pane = connected(Link::FlatPane, false);
    let mesh = mesh_culled(&world_with(&pane), IVec3::ZERO);
    let (min, max) = mesh.bounds().unwrap();
    assert!((max.y - min.y - 2.0 / 16.0).abs() < 1e-3);
    assert!((max.x - min.x - 0.5).abs() < 1e-3);
    // Avec deux voisines en ligne, la nappe s'étend jusqu'au bout de leurs bras : 3 cases moins le
    // quart de case que chaque bout laisse libre.
    let row = world_around(&pane, &[(IVec3::X, &pane), (IVec3::NEG_X, &pane)]);
    let (min, max) = mesh_culled(&row, IVec3::ZERO).bounds().unwrap();
    assert!((max.x - min.x - 2.5).abs() < 1e-3, "{}", max.x - min.x);
}

#[test]
fn the_player_climbs_a_corner_slope() {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    let stone = r.id(STONE).unwrap();
    for x in 0..8 {
        for z in 0..6 {
            world.set_block(IVec3::new(x, 0, z), stone);
        }
    }
    // Un coin rentrant plein dont les bords hauts sont au nord et à l'ouest, et un mur à l'ouest qui
    // arrête le joueur au sommet.
    let up = engine_voxel::shaped::corner_up(Face::NegY, Face::NegZ, Face::NegX).unwrap();
    let mut s = Shaped::new();
    s.try_add(
        Piece::Slope {
            base: Face::NegY,
            up,
            level: 16,
            corner: Corner::Inner,
        },
        STONE,
    )
    .unwrap();
    place(&mut world, IVec3::new(4, 1, 2), &s);
    for y in 1..4 {
        world.set_block(IVec3::new(3, y, 2), stone);
    }
    // Le long du bord sud de la case : chaque cran reste sous les 0,6 de la marche automatique.
    let mut body = PlayerBody::new(Vec3::new(7.5, 1.0, 2.95));
    for _ in 0..40 {
        body.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
    }
    for _ in 0..240 {
        body.step(&world, Vec3::new(-2.0, 0.0, 0.0), false, 1.0 / 60.0);
    }
    // Monté sur le coin : plus haut que le sol, arrêté contre le mur.
    assert!(body.position.x < 5.0, "x = {}", body.position.x);
    assert!(body.position.y > 1.7, "y = {}", body.position.y);
}
