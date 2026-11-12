//! Raccords contextuels : géométrie, vraie visée, compatibilité et sauvegardes.
use engine_voxel::{
    BlockEntity, BlockRegistry, Chunk, EnergyStorage, FACINGS, Face, FluidTank, Sides, World,
    mesh_culled, mesh_greedy, raycast,
};
use glam::{IVec3, Vec3};
use std::{path::Path, sync::Arc};

fn registry() -> Arc<BlockRegistry> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap())
}

fn entity(world: &mut World, pos: IVec3, e: BlockEntity) {
    let (chunk, [x, y, z]) = World::split(pos);
    world.chunk_mut(chunk).set_block_entity(x, y, z, Some(e));
}

#[test]
fn all_64_shapes_follow_neighbours_on_the_six_faces_across_chunk_borders() {
    let r = registry();
    let p = IVec3::splat(31);
    for key in ["core:cable", "core:pipe"] {
        let id = r.id(key).unwrap();
        for mask in 0u8..64 {
            let mut world = World::with_registry(r.clone());
            world.set_block(p, id);
            for (i, d) in FACINGS.into_iter().enumerate() {
                if mask & (1 << i) != 0 {
                    world.set_block(p + d, id);
                }
            }
            let boxes = world.block_boxes(p);
            assert_eq!(
                boxes.len(),
                1 + mask.count_ones() as usize,
                "{key}, {mask:06b}"
            );
            for face in Face::ALL {
                let axis = face as usize / 2;
                let edge = if face as u8 & 1 == 0 {
                    boxes.iter().map(|b| b.max[axis]).reduce(f32::max).unwrap()
                } else {
                    1.0 - boxes.iter().map(|b| b.min[axis]).reduce(f32::min).unwrap()
                };
                assert_eq!(
                    edge == 1.0,
                    mask & (1 << face as u8) != 0,
                    "{key}, {mask:06b}, {face:?}"
                );
            }
            // Le rayon passe dans la cellule, hors du noyau ; seule la branche -X l'arrête.
            drop(boxes);
            world.set_block(p - IVec3::Y * 2, r.id("core:stone").unwrap());
            let hit = raycast(
                &world,
                p.as_vec3() + Vec3::new(0.1, 2.0, 0.5),
                Vec3::NEG_Y,
                4.0,
            )
            .unwrap();
            assert_eq!(
                hit.block_pos == p,
                mask & (1 << Face::NegX as u8) != 0,
                "{key}, {mask:06b}"
            );
            // Les deux chemins de maillage prennent exactement les mêmes formes connectées.
            let a = mesh_culled(&world, IVec3::ZERO);
            let b = mesh_greedy(&world, IVec3::ZERO);
            // La pierre peut être greedy ; ses six faces sont isolées ici.
            assert_eq!(a.vertices.len(), b.vertices.len());
        }
    }
}

#[test]
fn no_phantom_arm_in_selection_collision_or_rendering_after_breaking_a_neighbour() {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    let cable = r.id("core:cable").unwrap();
    world.set_block(IVec3::ZERO, cable);
    let isolated = mesh_culled(&world, IVec3::ZERO);
    assert_eq!(
        isolated.vertices.len(),
        24,
        "un noyau isolé, pas de croix à six branches"
    );
    assert!(
        isolated
            .vertices
            .iter()
            .all(|v| v.local_position().cmpge(Vec3::splat(6.0 / 16.0)).all()
                && v.local_position().cmple(Vec3::splat(10.0 / 16.0)).all())
    );
    world.set_block(-IVec3::X, cable);
    assert_eq!(world.block_bounds(IVec3::ZERO).0.x, 0.0);
    let ray = Vec3::new(0.1, 2.0, 0.5);
    assert_eq!(
        raycast(&world, ray, Vec3::NEG_Y, 4.0).unwrap().block_pos,
        IVec3::ZERO
    );
    let mut player = engine_voxel::PlayerBody::new(ray);
    player.width = 0.05;
    for _ in 0..120 {
        player.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
    }
    assert!(player.on_ground);
    world.set_block(-IVec3::X, engine_voxel::BlockId::AIR);
    assert!(raycast(&world, ray, Vec3::NEG_Y, 4.0).is_none());
    for _ in 0..120 {
        player.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
    }
    assert!(!player.on_ground);
    assert!(player.position.y < 0.0);
    assert_eq!(mesh_culled(&world, IVec3::ZERO).vertices, isolated.vertices);
}

#[test]
fn different_resources_and_channels_do_not_connect_but_same_class_mod_conduits_do() {
    let mut r = (*registry()).clone();
    r.add_source(
        "test",
        r#"[
      (id:"test:a",name:"A",color:(1,1,1),conduit:Energy,channel:"a",connections:(core:4,arm:4)),
      (id:"test:b",name:"B",color:(1,1,1),conduit:Energy,channel:"b",connections:(core:4,arm:4)),
      (id:"test:c",name:"C",color:(1,1,1),conduit:Energy,channel:"a",connections:(core:6,arm:4)),
    ]"#,
        "test",
    )
    .unwrap();
    let r = Arc::new(r);
    let mut world = World::with_registry(r.clone());
    for (a, b, connected) in [
        ("core:cable", "core:pipe", false),
        ("test:a", "test:b", false),
        ("test:a", "core:cable", false),
        ("test:a", "test:c", true),
    ] {
        world.set_block(IVec3::ZERO, r.id(a).unwrap());
        world.set_block(IVec3::X, r.id(b).unwrap());
        assert_eq!(
            world.block_boxes(IVec3::ZERO).len(),
            if connected { 2 } else { 1 },
            "{a} / {b}"
        );
        assert_eq!(
            world.block_boxes(IVec3::X).len(),
            if connected { 2 } else { 1 }
        );
    }
}

#[test]
fn the_selection_outline_follows_the_hit_segment_and_its_normal_extends_the_chain() {
    let r = registry();
    let id = r.id("core:cable").unwrap();
    let mut world = World::with_registry(r);
    world.set_block(IVec3::ZERO, id);
    world.set_block(-IVec3::X, id);
    let origin = Vec3::new(0.1, 2.0, 0.5);
    let hit = raycast(&world, origin, Vec3::NEG_Y, 4.0).unwrap();
    let (min, max) = world.selection_bounds(hit.block_pos, origin - Vec3::Y * hit.distance);
    assert_eq!(max.x, 6.0 / 16.0, "contour du bras -X uniquement");
    assert_eq!(min.x, 0.0);
    assert_eq!(hit.normal, IVec3::Y);
    world.set_block(hit.block_pos + hit.normal, id);
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        3,
        "un coude se forme à la pose au-dessus"
    );
    assert_eq!(world.block_boxes(IVec3::Y).len(), 2);
}

#[test]
fn malformed_connection_models_are_rejected_instead_of_creating_invalid_boxes() {
    for field in [
        "connections:(core:0,arm:0)",
        "connections:(core:4,arm:6)",
        "connections:(core:16,arm:4)",
        "connections:(core:4,arm:4),states:[Axis]",
        "connections:(core:4,arm:4),boxes:[(from:(0,0,0),to:(8,8,8))]",
    ] {
        let mut r = BlockRegistry::empty();
        let source =
            format!("[(id:\"test:bad\",name:\"Bad\",color:(1,1,1),conduit:Energy,{field})]");
        assert!(r.add_source("test", &source, "test").is_err(), "{field}");
    }
    let mut r = BlockRegistry::empty();
    assert!(
        r.add_source(
            "test",
            r#"[(id:"test:bad",name:"Bad",color:(1,1,1),connections:(core:4,arm:4))]"#,
            "test"
        )
        .is_err()
    );
}

#[test]
fn machines_connect_only_on_an_open_compatible_face_even_when_the_store_is_empty() {
    let r = registry();
    let mut world = World::with_registry(r.clone());
    world.set_block(IVec3::ZERO, r.id("core:cable").unwrap());
    world.set_block(IVec3::X, r.id("core:battery").unwrap());
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        1,
        "capacités pas encore créées"
    );
    let mut e = BlockEntity::default();
    let mut energy = EnergyStorage::new(100, 10);
    energy.sides = Sides(1 << Face::PosX as u8);
    e.caps_mut().energy = Some(energy);
    entity(&mut world, IVec3::X, e.clone());
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        1,
        "face opposée fermée"
    );
    e.caps_mut().energy.as_mut().unwrap().sides = Sides(1 << Face::NegX as u8);
    // Les mêmes données passent par le format existant de sauvegarde.
    let e = postcard::from_bytes(&postcard::to_stdvec(&e).unwrap()).unwrap();
    entity(&mut world, IVec3::X, e);
    assert_eq!(world.block_boxes(IVec3::ZERO).len(), 2);
    let ports = world.block_boxes(IVec3::X);
    assert!(
        ports
            .iter()
            .any(|b| b.min.x == 0.0 && b.max.x == 2.0 / 16.0),
        "raccord jusqu'au corps de batterie en retrait"
    );
    drop(ports);
    world.set_block(IVec3::ZERO, r.id("core:pipe").unwrap());
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        1,
        "un tuyau ne se branche pas à une batterie"
    );
    let mut e = BlockEntity::default();
    let mut closed = FluidTank::new(100);
    closed.sides = Sides::NONE;
    let mut open = FluidTank::new(100);
    open.sides = Sides(1 << Face::NegX as u8);
    e.caps_mut().tanks = vec![closed, open];
    world.set_block(IVec3::X, r.id("core:tank").unwrap());
    entity(&mut world, IVec3::X, e);
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        2,
        "union des faces des réservoirs, comme le réseau"
    );
    world.set_block(IVec3::X, engine_voxel::BlockId::AIR);
    assert_eq!(
        world.block_boxes(IVec3::ZERO).len(),
        1,
        "aucun raccord vers l'air même si une écriture brute a laissé une ancienne entité"
    );
}

#[test]
fn chunk_unloading_and_reloading_recompute_the_connection_without_saved_variants() {
    let r = registry();
    let cable = r.id("core:cable").unwrap();
    let mut world = World::with_registry(r.clone());
    let p = IVec3::new(31, 4, 4);
    world.set_block(p, cable);
    world.set_block(p + IVec3::X, cable);
    assert_eq!(world.block_boxes(p).len(), 2);
    let chunk = world.remove(IVec3::X).unwrap();
    assert_eq!(world.block_boxes(p).len(), 1);
    let restored = Chunk::from_blocks(chunk.blocks().to_vec()).unwrap();
    world.insert(IVec3::X, Arc::new(restored));
    assert_eq!(world.block_boxes(p).len(), 2);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let expected = std::fs::read_to_string(root.join("docs/visual/registre-avant.tsv")).unwrap();
    let ids: Vec<_> = expected
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields[0].parse::<u16>().unwrap(), fields[1].to_string())
        })
        .collect();
    // Les identifiants du jeu de base ne bougent jamais (une passe visuelle n'en réordonne aucun) ;
    // ceux des mods se décalent quand un mod gagne un bloc : les sauvegardes les nomment par leur
    // identifiant texte (palette), pas par leur numéro.
    let core = |list: &[(u16, String)]| -> Vec<(u16, String)> {
        list.iter()
            .filter(|(_, key)| key.starts_with("core:"))
            .cloned()
            .collect()
    };
    let now: Vec<(u16, String)> = r.iter().map(|(id, def)| (id.0, def.key.clone())).collect();
    assert_eq!(
        core(&now),
        core(&ids),
        "un identifiant du jeu de base a changé de place"
    );
}
