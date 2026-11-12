//! Vérifie les conséquences réelles des formes des postes de travail, sans GPU.
use std::{path::Path, sync::Arc};

use engine_voxel::{BlockRegistry, PlayerBody, World, raycast};
use glam::{IVec3, Vec3};

#[test]
fn workstations_can_be_targeted_removed_and_walked_on_at_their_actual_height() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry =
        Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap());
    for key in [
        "core:chest",
        "core:furnace",
        "core:workbench",
        "core:bed",
        "core:enchanting_table",
        "core:brewing_stand",
        "entrepot:vault",
        "forge:anvil",
        "forge:alloy_furnace",
        "magie:rune_table",
    ] {
        let id = registry.id(key).unwrap();
        let def = registry.def(id);
        assert!(!def.boxes.is_empty(), "{key} : une vraie forme");
        let mut world = World::with_registry(registry.clone());
        world.set_block(IVec3::ZERO, id);
        let hit = raycast(&world, Vec3::new(0.5, 3.0, 0.5), Vec3::NEG_Y, 4.0).unwrap();
        assert_eq!(hit.block_pos, IVec3::ZERO, "{key} : sélection");
        let mut body = PlayerBody::new(Vec3::new(0.5, 3.0, 0.5));
        for _ in 0..180 {
            body.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
        }
        assert!(body.on_ground, "{key} : appui");
        // La largeur du joueur peut toucher une pièce plus haute que le rayon central.
        assert!(
            body.position.y + 0.005 >= 3.0 - hit.distance,
            "{key} : collision sous la surface"
        );
        assert!(body.position.y <= 1.005, "{key} : reste dans sa cellule");
        world.set_block(IVec3::ZERO, registry.id("core:air").unwrap());
        assert!(
            raycast(&world, Vec3::new(0.5, 3.0, 0.5), Vec3::NEG_Y, 4.0).is_none(),
            "{key} : retrait"
        );
    }
}

#[test]
fn ray_passes_through_the_open_space_below_a_workbench() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = Arc::new(BlockRegistry::load(&root.join("assets"), None).unwrap());
    let mut world = World::with_registry(registry.clone());
    world.set_block(IVec3::new(0, 0, 2), registry.id("core:workbench").unwrap());
    world.set_block(IVec3::new(0, 0, 4), registry.id("core:stone").unwrap());
    let hit = raycast(&world, Vec3::new(0.5, 0.4, 0.5), Vec3::Z, 6.0).unwrap();
    assert_eq!(hit.block_pos, IVec3::new(0, 0, 4));
}

#[test]
fn every_selectable_model_and_its_variants_can_be_hit_and_support_a_player_if_solid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry =
        Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap());
    for (id, def) in registry
        .iter()
        .filter(|(_, d)| !d.boxes.is_empty() && d.selectable)
    {
        // Choisir une surface réelle, y compris pour une cage ou une herse creuse.
        let mut world = World::with_registry(registry.clone());
        world.set_block(IVec3::ZERO, id);
        let boxes = world.block_boxes(IVec3::ZERO);
        let b = boxes
            .iter()
            .max_by(|a, b| {
                let a = a.max - a.min;
                let b = b.max - b.min;
                (a.x * a.z).total_cmp(&(b.x * b.z))
            })
            .unwrap();
        let center = (b.min + b.max) * 0.5;
        let origin = Vec3::new(center.x, 3.0, center.z);
        let hit = raycast(&world, origin, Vec3::NEG_Y, 4.0)
            .unwrap_or_else(|| panic!("{} : sélection", def.key));
        assert_eq!(hit.block_pos, IVec3::ZERO, "{}", def.key);
        if def.solid {
            let mut body = PlayerBody::new(origin);
            for _ in 0..180 {
                body.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
            }
            assert!(body.on_ground, "{} : appui", def.key);
            assert!(
                body.position.y + 0.005 >= 3.0 - hit.distance,
                "{} : surface de collision",
                def.key
            );
        }
        world.set_block(IVec3::ZERO, registry.id("core:air").unwrap());
        assert!(
            raycast(&world, origin, Vec3::NEG_Y, 4.0).is_none(),
            "{} : retrait",
            def.key
        );
    }
}

#[test]
fn farmland_has_a_lower_surface_and_cactus_has_a_margin() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = Arc::new(BlockRegistry::load(&root.join("assets"), None).unwrap());
    let mut world = World::with_registry(registry.clone());
    for key in ["core:farmland", "core:farmland_wet"] {
        world.set_block(IVec3::ZERO, registry.id(key).unwrap());
        let hit = raycast(&world, Vec3::new(0.5, 2.0, 0.5), Vec3::NEG_Y, 4.0).unwrap();
        assert!((hit.distance - (2.0 - 15.0 / 16.0)).abs() < 1e-5, "{key}");
    }
    world.set_block(IVec3::ZERO, registry.id("core:cactus").unwrap());
    assert!(raycast(&world, Vec3::new(0.02, 0.5, -1.0), Vec3::Z, 4.0).is_none());
    assert!(raycast(&world, Vec3::new(0.5, 0.5, -1.0), Vec3::Z, 4.0).is_some());
}

#[test]
fn a_wall_keeps_its_face_behind_every_model_and_adjacent_models_keep_their_faces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry =
        Arc::new(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap());
    let stone = registry.id("core:stone").unwrap();
    for (id, def) in registry.iter().filter(|(_, d)| !d.boxes.is_empty()) {
        let mut world = World::with_registry(registry.clone());
        world.set_block(IVec3::ZERO, id);
        let single = engine_voxel::mesh_culled(&world, IVec3::ZERO)
            .vertices
            .len();
        world.set_block(IVec3::X, id);
        // Les modèles fixes gardent toutes leurs faces ; un conduit connecté ajoute
        // ses branches, vérifiées séparément sur les 64 combinaisons de voisins.
        if def.connections.is_none() {
            assert_eq!(
                engine_voxel::mesh_culled(&world, IVec3::ZERO)
                    .vertices
                    .len(),
                single * 2,
                "{} : modèles voisins",
                def.key
            );
        }
        world.set_block(IVec3::X, stone);
        let mesh = engine_voxel::mesh_culled(&world, IVec3::ZERO);
        let wall_face = mesh
            .vertices
            .iter()
            .filter(|v| v.normal() == Vec3::NEG_X && (v.local_position().x - 1.0).abs() < 1e-5)
            .count();
        assert_eq!(wall_face, 4, "{} : face de mur conservée", def.key);
    }
}
