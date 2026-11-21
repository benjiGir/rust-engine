//! Les tests du streamer (voir le module parent).

use std::time::{Duration, Instant};

use super::*;
use crate::mesh::mesh_binary;
use crate::registry::test_blocks::{snow, stone};

const RADIUS: i32 = 2;

fn streamer() -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: RADIUS,
        unload_margin: 1,
        ..StreamingConfig::default()
    };
    ChunkStreamer::new(TerrainGenerator::new(7), config)
}

#[test]
fn changing_machine_ports_remeshes_the_neighbour_chunk_but_changing_amounts_does_not() {
    use crate::{EnergyStorage, GridPos, Grids, Sides};
    use std::cell::RefCell;
    let mut s = streamer();
    pump(&mut s, Vec3::new(4.0, 80.0, 4.0));
    let pos = IVec3::new(32, 80, 4);
    let battery = s.world().registry().id("core:battery").unwrap();
    assert!(s.set_block_quiet(pos, battery));
    s.remesh_pending.clear();
    let cell = RefCell::new(s);
    let grids = Grids::new(&cell);
    grids.write_or_create(GridPos::world(pos), |e| {
        e.caps_mut().energy = Some(EnergyStorage::new(100, 10))
    });
    let expected = HashSet::from_iter([IVec3::new(0, 2, 0), IVec3::new(1, 2, 0)]);
    assert_eq!(cell.borrow().remesh_pending, expected);
    cell.borrow_mut().remesh_pending.clear();
    grids.write(GridPos::world(pos), |e| {
        e.caps_mut().energy.as_mut().unwrap().amount = 42
    });
    assert!(
        cell.borrow().remesh_pending.is_empty(),
        "le contenu ne change pas la forme"
    );
    grids.write(GridPos::world(pos), |e| {
        e.caps_mut().energy.as_mut().unwrap().sides = Sides::NONE
    });
    assert_eq!(cell.borrow().remesh_pending, expected);
}

/// Charge tout autour de `player` (pour les tests d'autres modules).
pub(crate) fn pump_until_idle(streamer: &mut ChunkStreamer, player: Vec3) {
    pump(streamer, player);
}

/// Fait tourner le streamer jusqu'à ce qu'il n'ait plus rien à faire,
/// et renvoie tous les événements produits.
fn pump(streamer: &mut ChunkStreamer, player: Vec3) -> Vec<StreamEvent> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    loop {
        streamer.update(player);
        events.extend(std::iter::from_fn(|| streamer.next_event()));
        if streamer.is_idle() {
            return events;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn meshed(events: &[StreamEvent]) -> HashMap<IVec3, &ChunkMesh> {
    events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Mesh(pos, mesh) => Some((*pos, mesh)),
            StreamEvent::Unload(_) => None,
        })
        .collect()
}

#[test]
fn loads_every_chunk_in_radius() {
    let mut s = streamer();
    let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));

    // Rayon de génération = rayon de vue + 3.
    let r = RADIUS + 3;
    let columns = (-r..=r)
        .flat_map(|x| (-r..=r).map(move |z| x * x + z * z))
        .filter(|&d| d <= r * r)
        .count();
    assert_eq!(
        s.stats().loaded,
        columns * crate::WORLD_HEIGHT_CHUNKS as usize
    );

    // Tout chunk non vide dans le rayon de vue a reçu un maillage, et seulement eux.
    let meshes = meshed(&events);
    assert!(!meshes.is_empty());
    for pos in meshes.keys() {
        assert!(horizontal_distance_sq(*pos, IVec3::ZERO) <= RADIUS * RADIUS);
    }
    for pos in s.world().chunk_positions() {
        let inside = horizontal_distance_sq(pos, IVec3::ZERO) <= RADIUS * RADIUS;
        let has_faces = !mesh_binary(s.world(), pos).is_empty();
        assert_eq!(
            meshes.contains_key(&pos),
            inside && has_faces,
            "chunk {pos}"
        );
    }
}

/// Le maillage fait en tâche de fond doit être identique au maillage direct :
/// les faces de bordure tiennent bien compte des voisins.
#[test]
fn pipeline_timing_is_off_by_default_and_measures_every_uploaded_chunk_when_on() {
    let mut off = streamer();
    assert!(off.pipeline_report().is_none());
    off.note_uploaded(IVec3::ZERO); // sans effet, sans panique

    let mut on = streamer();
    on.set_pipeline_timing(true);
    let events = pump(&mut on, Vec3::new(0.5, 70.0, 0.5));
    let uploaded = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Mesh(..)))
        .count();
    assert!(uploaded > 0);
    for event in &events {
        if let StreamEvent::Mesh(pos, _) = event {
            on.note_uploaded(*pos);
        }
    }
    let report = on.pipeline_report().unwrap();
    let total = report.stage(crate::PipelineStage::Total);
    assert_eq!(total.count, uploaded as u64, "un chunk, une mesure");
    assert!(total.p50_ms <= total.max_ms);
    for stage in [
        crate::PipelineStage::Generation,
        crate::PipelineStage::Light,
        crate::PipelineStage::Meshing,
        crate::PipelineStage::Upload,
    ] {
        assert_eq!(report.stage(stage).count, total.count);
        assert!(report.stage(stage).max_ms <= total.max_ms + 0.01);
    }
    assert!(
        report
            .to_string()
            .contains(&format!("{uploaded} chunks mesurés"))
    );
}

#[test]
fn streamed_meshes_match_direct_meshing() {
    let mut s = streamer();
    let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let meshes = meshed(&events);
    for (pos, mesh) in meshes {
        assert_eq!(
            mesh.vertices,
            mesh_binary(s.world(), pos).vertices,
            "chunk {pos}"
        );
    }
}

#[test]
fn small_moves_unload_nothing() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    // Un chunk plus loin : la marge d'hystérésis garde tout.
    let events = pump(&mut s, Vec3::new(37.0, 40.0, 5.0));
    assert!(!events.iter().any(|e| matches!(e, StreamEvent::Unload(_))));
}

#[test]
fn far_chunks_are_unloaded() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let events = pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
    let unloaded: Vec<IVec3> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Unload(pos) => Some(*pos),
            StreamEvent::Mesh(..) => None,
        })
        .collect();
    assert!(unloaded.contains(&IVec3::new(0, 0, 0)));
    assert!(s.world().chunk(IVec3::new(0, 0, 0)).is_none());
    // Il ne reste que des chunks proches de la nouvelle position (rayon 2 + 3 + marge 1).
    assert!(
        s.slots
            .keys()
            .all(|p| horizontal_distance_sq(*p, IVec3::new(20, 0, 0)) <= 6 * 6)
    );
}

/// Le bloc opposé : de l'air devient de la pierre, un bloc plein devient de l'air.
fn toggled(s: &ChunkStreamer, pos: IVec3) -> BlockId {
    if s.world().block(pos).is_air() {
        stone()
    } else {
        BlockId::AIR
    }
}

#[test]
fn editing_a_border_block_remeshes_both_chunks_at_once() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    // x = 31 : dernier bloc du chunk x = 0, collé au chunk x = 1.
    let pos = IVec3::new(31, 40, 5);
    let block = toggled(&s, pos);
    assert!(s.set_block(pos, block));
    assert_eq!(s.world().block(pos), block);

    // Les deux maillages sont en tête de file, sans attendre de tâche de fond.
    let first: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).take(2).collect();
    let positions: Vec<IVec3> = first
        .iter()
        .map(|e| match e {
            StreamEvent::Mesh(p, mesh) => {
                assert_eq!(mesh, &mesh_binary(s.world(), *p), "maillage à jour");
                *p
            }
            StreamEvent::Unload(p) => panic!("déchargement inattendu de {p}"),
        })
        .collect();
    assert_eq!(positions, [IVec3::new(0, 1, 0), IVec3::new(1, 1, 0)]);
}

#[test]
fn doors_place_toggle_and_break_as_a_pair_across_a_vertical_chunk_border() {
    let mut s = streamer();
    let bottom = IVec3::new(5, 127, 5);
    let top = bottom + IVec3::Y;
    let registry = s.world().registry().clone();
    let rock = registry.id("core:cobblestone").unwrap();
    let door = registry.id("core:wooden_glass_door").unwrap();
    let parts = crate::door::placement(&registry, door, bottom, Vec3::Z).unwrap();
    s.slots
        .insert(World::split(bottom).0, Slot::Generated(MeshState::Missing));
    s.set_block_quiet(bottom - IVec3::Y, rock);
    assert!(!s.place_pair(parts));
    assert!(s.world.block(bottom).is_air());
    s.slots
        .insert(World::split(top).0, Slot::Generated(MeshState::Missing));
    s.set_block_quiet(top, rock);
    assert!(!s.place_pair(parts));
    assert!(s.world.block(bottom).is_air());
    s.set_block_quiet(top, BlockId::AIR);
    assert!(s.place_pair(parts));

    let origin = top.as_vec3() + Vec3::new(0.5, 0.5, -1.0);
    assert_eq!(
        crate::raycast(s.world(), origin, Vec3::Z, 2.0)
            .unwrap()
            .block_pos,
        top
    );
    let mesh = crate::mesh_binary(s.world(), World::split(top).0);
    assert!(!mesh.indices.is_empty() && !mesh.translucent_indices.is_empty());
    assert!(s.toggle_door(top));
    for (p, _) in parts {
        assert!(crate::door::is_open(registry.def(s.world.block(p))));
        assert_eq!(registry.light_opacity(s.world.block(p)), 0);
    }
    assert!(crate::raycast(s.world(), origin, Vec3::Z, 2.0).is_none());
    assert!(s.toggle_door(bottom));
    assert_eq!(parts.map(|(p, _)| s.world.block(p)), parts.map(|(_, b)| b));
    let items = crate::ItemRegistry::builtin(&registry).unwrap();
    assert_eq!(
        items.of_block(s.world.block(top)),
        items.id("core:wooden_glass_door")
    );
    assert!(s.set_block(top, BlockId::AIR));
    assert!(s.world.block(bottom).is_air() && s.world.block(top).is_air());
    assert!(s.place_pair(parts));
    let floor = bottom - IVec3::Y;
    assert!(s.set_block(floor, BlockId::AIR));
    let settled = crate::settle(&mut s, floor);
    assert_eq!(settled.broken, vec![parts[0]]);
    assert!(s.world.block(bottom).is_air() && s.world.block(top).is_air());
}

#[test]
fn both_bed_halves_cross_chunk_borders_and_require_space_and_support() {
    let mut s = streamer();
    let foot = IVec3::new(31, 170, 31);
    let head = foot + IVec3::X;
    let registry = s.world().registry().clone();
    let rock = registry.id("core:cobblestone").unwrap();
    s.slots
        .insert(World::split(foot).0, Slot::Generated(MeshState::Missing));
    s.set_block_quiet(foot - IVec3::Y, rock);
    assert!(s.place_bed(foot, Vec3::X).is_none());
    assert!(s.world().block(foot).is_air());
    s.slots
        .insert(World::split(head).0, Slot::Generated(MeshState::Missing));
    assert!(s.place_bed(foot, Vec3::X).is_none());
    s.set_block_quiet(head - IVec3::Y, rock);
    s.set_block_quiet(head, rock);
    assert!(s.place_bed(foot, Vec3::X).is_none());
    assert!(s.world().block(foot).is_air());
    assert_eq!(s.world().block(head), rock);
    s.set_block_quiet(head, BlockId::AIR);

    let items = crate::ItemRegistry::builtin(&registry).unwrap();
    let bed = items.id(crate::bed::ITEM).unwrap();
    for broken in [head, foot] {
        let parts = s.place_bed(foot, Vec3::X).unwrap();
        assert_eq!(parts.map(|(p, _)| p), [foot, head]);
        for (p, block) in parts {
            assert_eq!(items.of_block(block), Some(bed));
            assert_eq!(crate::bed::head_at(s.world(), p), Some(head));
            let hit = crate::raycast(
                s.world(),
                p.as_vec3() + Vec3::new(0.5, 2.0, 0.5),
                Vec3::NEG_Y,
                3.0,
            )
            .unwrap();
            assert_eq!(hit.block_pos, p);
        }
        assert!(s.set_block(broken, BlockId::AIR));
        assert!(s.world().block(foot).is_air() && s.world().block(head).is_air());
    }

    s.place_bed(foot, Vec3::X).unwrap();
    let floor = head - IVec3::Y;
    s.set_block(floor, BlockId::AIR);
    let settled = crate::settle(&mut s, floor);
    assert_eq!(settled.broken.len(), 1);
    assert_eq!(items.of_block(settled.broken[0].1), Some(bed));
    assert!(s.world().block(foot).is_air() && s.world().block(head).is_air());
}

#[test]
fn breaking_a_leaf_two_blocks_from_the_border_remeshes_the_neighbour() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let leaves = BlockRegistry::builtin().id("core:oak_leaves").unwrap();
    // Un cube de feuilles 3 × 3 × 3 à cheval sur les chunks x = 0 et x = 1, dans le ciel :
    // son centre (32, 150, 5), entouré, est opaque, et la face +X de la feuille en
    // x = 31 (chunk 0) est cachée contre lui.
    for x in 31..34 {
        for y in 149..152 {
            for z in 4..7 {
                assert!(s.set_block(IVec3::new(x, y, z), leaves));
            }
        }
    }
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    // x = 33, deuxième bloc du chunk x = 1 : le centre n'est plus entouré, la face en
    // x = 31, à deux blocs, redevient visible. Le chunk 0 doit être remaillé.
    assert!(s.set_block(IVec3::new(33, 150, 5), BlockId::AIR));
    let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
    let remeshed = meshed(&events);
    let chunk = IVec3::new(0, 4, 0);
    let mesh = remeshed.get(&chunk).expect("le chunk voisin est remaillé");
    assert_eq!(*mesh, &mesh_binary(s.world(), chunk));
}

#[test]
fn interior_edit_remeshes_only_its_chunk() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let pos = IVec3::new(10, 40, 10);
    assert!(s.set_block(pos, toggled(&s, pos)));
    let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], StreamEvent::Mesh(p, _) if p == IVec3::new(0, 1, 0)));
}

#[test]
fn edits_outside_loaded_chunks_are_refused() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert!(!s.set_block(IVec3::new(5000, 40, 0), stone()));
}

#[test]
fn edits_survive_unloading() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let dug = IVec3::new(3, 20, 3); // sous terre : de la pierre (ou une grotte)
    let built = IVec3::new(8, 120, 8); // dans le ciel
    let dug_block = toggled(&s, dug);
    assert!(s.set_block(dug, dug_block));
    assert!(s.set_block(built, snow()));

    // On part loin : tout est déchargé...
    pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
    assert!(s.world().chunk(IVec3::ZERO).is_none());
    // ... puis on revient : les modifications sont toujours là.
    let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert_eq!(s.world().block(dug), dug_block);
    assert_eq!(s.world().block(built), snow());
    // Et le chunk du ciel, vide à la génération, a bien un maillage pour ce bloc.
    assert!(meshed(&events).contains_key(&IVec3::new(0, 3, 0)));
}

#[test]
fn unloading_an_edited_chunk_notes_its_surface_for_the_far_terrain() {
    let mut s = streamer();
    let surfaces = s.edited_surfaces();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let built = IVec3::new(8, 120, 8);
    assert!(s.set_block(built, stone()));
    // Tant que le chunk est chargé, rien n'est noté (le lointain ne le couvre pas).
    assert!(surfaces.is_empty());
    pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
    assert!(s.world().chunk(IVec3::ZERO).is_none());
    let ground = s.generator().column(8, 8).height;
    let found = surfaces.surface_at(8, 8, ground, s.world().registry());
    assert_eq!(found, Some((120, stone())));
    // Sa colonne est signalée une fois, pour reconstruire les tuiles lointaines qui la couvrent.
    assert_eq!(surfaces.take_changed(), vec![(0, 0)]);
    assert!(surfaces.take_changed().is_empty());
}

#[test]
fn mod_blocks_can_be_placed_and_meshed() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = BlockRegistry::load(&repo.join("assets"), Some(&repo.join("mods"))).unwrap();
    let ruby = registry.id("example:ruby_slab").unwrap();
    let ruby_texture = registry.def(ruby).face_textures[0];
    assert_ne!(ruby_texture, 0);
    let generator = TerrainGenerator::with_registry(7, Arc::new(registry)).unwrap();
    let config = StreamingConfig {
        view_radius: RADIUS,
        ..StreamingConfig::default()
    };
    let mut s = ChunkStreamer::new(generator, config);
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));

    // Une dalle de rubis seule dans le ciel : 6 faces.
    let pos = IVec3::new(8, 120, 8);
    assert!(s.set_block(pos, ruby));
    let Some(StreamEvent::Mesh(_, mesh)) = s.next_event() else {
        panic!("pas de maillage");
    };
    assert_eq!(mesh.quad_count(), 6);
    // La dalle a une texture : ses faces portent sa couche.
    assert!(mesh.vertices.iter().all(|v| v.texture == ruby_texture));
}

#[test]
fn torch_light_reaches_and_remeshes_the_next_chunk() {
    let mut s = streamer();
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    let torch = BlockRegistry::builtin().id("core:torch").unwrap();
    // Dans le ciel, à 12 blocs du chunk x = 1 : sa lumière y déborde.
    let pos = IVec3::new(20, 120, 5);
    assert!(s.set_block(pos, torch));
    let events = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert_eq!(s.world().light(pos).1, 14);
    assert_eq!(s.world().light(IVec3::new(33, 120, 5)).1, 1);
    // Le chunk voisin n'a pas changé de blocs, seulement de lumière : il est remaillé
    // en tâche de fond.
    assert!(meshed(&events).contains_key(&IVec3::new(1, 3, 0)));
    // Et tout reste cohérent avec un maillage direct.
    for (pos, mesh) in meshed(&events) {
        assert_eq!(mesh, &mesh_binary(s.world(), pos), "chunk {pos}");
    }
}

fn saving_streamer(dir: &std::path::Path) -> ChunkStreamer {
    let store = Arc::new(crate::save::SaveStore::open(dir).unwrap());
    streamer().with_save(store)
}

/// Le test d'intégration de la sauvegarde : on modifie, on sauvegarde, on « relance
/// le jeu » avec un streamer neuf sur le même dossier, et le monde doit être identique.
#[test]
fn reloaded_world_has_the_same_blocks() {
    let dir = crate::save::tests::temp_dir("streamer");
    let player = Vec3::new(5.0, 40.0, 5.0);
    let mut first = saving_streamer(&dir);
    pump(&mut first, player);
    // Du verre : il ne peut pas déjà se trouver là (le terrain n'en génère pas).
    let glass = BlockRegistry::builtin().id("core:glass").unwrap();
    let edits = [
        (IVec3::new(3, 20, 3), glass),
        (IVec3::new(8, 120, 8), snow()), // dans un chunk de ciel, vide à la génération
        (IVec3::new(31, 40, 5), glass),  // en bordure de chunk
        (IVec3::new(-20, 39, -7), glass),
    ];
    for (pos, block) in edits {
        assert!(first.set_block(pos, block));
    }
    assert!(first.has_unsaved_changes());
    let report = first.save_blocking().unwrap();
    assert_eq!(report.chunks, 4);
    assert!(!first.has_unsaved_changes());

    let mut second = saving_streamer(&dir);
    pump(&mut second, player);
    for (pos, block) in edits {
        assert_eq!(second.world().block(pos), block, "en {pos}");
    }
    // Tous les chunks chargés sont identiques, bloc pour bloc.
    for pos in first.world().chunk_positions() {
        let a = first.world().chunk(pos).map(|c| c.blocks().to_vec());
        let b = second.world().chunk(pos).map(|c| c.blocks().to_vec());
        assert_eq!(a, b, "chunk {pos}");
    }
}

#[test]
fn background_save_finishes_and_frees_unloaded_chunks() {
    let dir = crate::save::tests::temp_dir("background");
    let mut s = saving_streamer(&dir);
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert!(!s.begin_save(), "rien à sauvegarder");
    assert!(s.set_block(IVec3::new(3, 20, 3), BlockId::AIR));
    // On part loin : le chunk modifié est gardé en mémoire, pas encore sauvegardé.
    pump(&mut s, Vec3::new(20.0 * 32.0, 40.0, 5.0));
    assert!(s.saved_edits.contains_key(&IVec3::new(0, 0, 0)));

    assert!(s.begin_save());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let report = loop {
        if let Some(result) = s.poll_save() {
            break result.unwrap();
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(report.chunks, 1);
    // Écrit sur disque : plus besoin de le garder en mémoire...
    assert!(s.saved_edits.is_empty());
    // ... et en revenant, il est relu depuis le disque.
    pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert!(s.world().block(IVec3::new(3, 20, 3)).is_air());
}

#[test]
fn replacing_the_registry_relights_and_remeshes() {
    let mut s = streamer();
    let first = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    // Même blocs, propriétés changées : la pierre devient une source de lumière.
    let text = include_str!("../../../../assets/core/blocks.ron").replacen(
        r#"id: "core:stone","#,
        r#"id: "core:stone", light_emission: 5,"#,
        1,
    );
    let mut registry = BlockRegistry::empty();
    registry.add_source("core", &text, "test").unwrap();
    assert_eq!(registry.light_emission(stone()), 5);
    s.replace_registry(Arc::new(registry));
    let second = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert_eq!(
        meshed(&first).len(),
        meshed(&second).len(),
        "tout est remaillé"
    );
    // Une pierre à l'air libre éclaire maintenant ses voisins.
    let lit = s.world().chunk_positions().any(|c| {
        s.world()
            .chunk(c)
            .is_some_and(|ch| ch.light_data().iter().any(|l| l & 0x0F > 0))
    });
    assert!(lit);
}

#[test]
fn changing_mesher_remeshes_everything() {
    let mut s = streamer();
    let first = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    s.set_mesher(Mesher::Culling);
    let second = pump(&mut s, Vec3::new(5.0, 40.0, 5.0));
    assert_eq!(meshed(&first).len(), meshed(&second).len());
    // En culling, il y a forcément au moins autant de triangles.
    let triangles = |events: &[StreamEvent]| -> usize {
        meshed(events).values().map(|m| m.triangle_count()).sum()
    };
    assert!(triangles(&second) > triangles(&first));
}

// ---------- Priorités, file des maillages et budget (lot 6) ----------

/// Les positions de la file de chargement, dans l'ordre où elles partiront.
fn load_order(s: &ChunkStreamer) -> Vec<IVec3> {
    s.load_queue.iter().copied().collect()
}

fn streamer_with(column_order: bool, view_bias: f32) -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: 6,
        unload_margin: 1,
        column_order,
        view_bias,
        ..StreamingConfig::default()
    };
    ChunkStreamer::new(TerrainGenerator::new(7), config)
}

/// La distance horizontale au carré d'un chunk au centre (0, y, 0).
fn flat_sq(p: IVec3) -> i32 {
    p.x * p.x + p.z * p.z
}

#[test]
fn columns_are_completed_from_the_nearest_to_the_farthest() {
    let mut s = streamer_with(true, 0.0);
    s.update(Vec3::new(5.0, 40.0, 5.0));
    let order = load_order(&s);
    assert!(order.len() > 500);
    // La distance horizontale ne diminue jamais : une colonne est finie avant la suivante.
    assert!(order.windows(2).all(|w| flat_sq(w[0]) <= flat_sq(w[1])));
    // La colonne du joueur est partie la première : ses chunks sont déjà en cours de génération.
    assert!(!order.iter().take(3).any(|p| p.x == 0 && p.z == 0));
    assert!(s.stats().in_flight >= 6);
}

#[test]
fn the_legacy_order_interleaves_the_columns() {
    // L'ancien tri par distance 3D mélange les colonnes : le haut d'une colonne proche passe
    // après le bas d'une colonne plus loin (ce que `column_order` corrige).
    let mut s = streamer_with(false, 0.0);
    s.update(Vec3::new(5.0, 40.0, 5.0));
    let order = load_order(&s);
    assert!(order.windows(2).any(|w| flat_sq(w[0]) > flat_sq(w[1])));
}

#[test]
fn what_the_camera_looks_at_loads_first_without_starving_the_back() {
    let mut s = streamer_with(true, 0.6);
    s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::X);
    let order = load_order(&s);
    let index = |x: i32, z: i32| {
        order
            .iter()
            .position(|p| p.x == x && p.z == z && p.y == 2)
            .unwrap_or_else(|| panic!("({x}, {z}) manque"))
    };
    // À même distance, devant avant derrière.
    assert!(index(5, 0) < index(-5, 0));
    // Mais pas de famine : une colonne proche de derrière passe avant une lointaine de devant.
    assert!(index(-3, 0) < index(8, 0));
    assert!(index(-5, 0) < index(8, 0));
    // Aucun biais sous 3 chunks : le sol sous les pieds ne dépend pas du regard.
    assert!(index(-2, 0) < index(3, 0));
}

#[test]
fn turning_the_camera_resorts_the_queue_and_a_small_turn_does_not() {
    let mut s = streamer_with(true, 0.6);
    s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::X);
    let ahead = load_order(&s);
    // Un petit pas de côté (moins de 25 degrés) ne retrie rien.
    s.update_view(Vec3::new(5.0, 40.0, 5.0), Vec3::new(1.0, 0.0, 0.2));
    assert_eq!(load_order(&s)[..50], ahead[..50]);
    // Un demi-tour renverse les préférences.
    s.update_view(Vec3::new(5.0, 40.0, 5.0), -Vec3::X);
    let behind = load_order(&s);
    let index =
        |order: &[IVec3], x: i32| order.iter().position(|p| p.x == x && p.z == 0 && p.y == 2);
    assert!(index(&ahead, 5) < index(&ahead, -5));
    assert!(index(&behind, -5) < index(&behind, 5));
}

/// Fait tourner le streamer sans rien envoyer : tout reste en attente dans `waiting`.
fn settle_without_draining(s: &mut ChunkStreamer, player: Vec3) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        s.update(player);
        let working =
            s.load_queue.len() + s.in_flight + s.light_candidates.len() + s.mesh_candidates.len();
        if working == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn waiting_meshes_leave_nearest_first_and_the_budget_limits_each_frame() {
    let mut s = streamer();
    let player = Vec3::new(5.0, 40.0, 5.0);
    settle_without_draining(&mut s, player);
    let stats = s.stats();
    assert!(
        stats.meshes_waiting > 10 && stats.bytes_waiting > 0,
        "{stats:?}"
    );
    // Un budget d'un seul octet : un maillage par image (le premier passe toujours).
    let centre = World::chunk_containing(player);
    let mut distances = Vec::new();
    let mut frames = 0;
    while s.stats().meshes_waiting > 0 && frames < 10_000 {
        frames += 1;
        let mut budget = TransferBudget::new(1, Duration::MAX, 0);
        let mut sent = 0;
        while let Some(event) = s.next_event_budgeted(&mut budget) {
            if let StreamEvent::Mesh(pos, mesh) = event {
                budget.record_upload(mesh.byte_size(), Duration::ZERO);
                distances.push(flat_sq(pos - centre));
                sent += 1;
            }
        }
        assert!(
            sent <= 1,
            "plus d'un maillage dans une image à budget minimal"
        );
    }
    assert!(
        distances.windows(2).all(|w| w[0] <= w[1]),
        "du plus proche au plus lointain"
    );
    assert!(distances.len() > 10);
}

#[test]
fn a_big_budget_sends_everything_in_one_frame_and_unloads_are_spread() {
    let mut s = streamer();
    let player = Vec3::new(5.0, 40.0, 5.0);
    settle_without_draining(&mut s, player);
    let waiting = s.stats().meshes_waiting;
    let mut budget = TransferBudget::unlimited();
    let sent = std::iter::from_fn(|| s.next_event_budgeted(&mut budget))
        .filter(|e| matches!(e, StreamEvent::Mesh(..)))
        .count();
    assert_eq!(sent, waiting);
    // Une téléportation : beaucoup de libérations, étalées par le budget.
    s.update(Vec3::new(30.0 * 32.0, 40.0, 5.0));
    let queued = s.stats().unloads_waiting;
    assert!(queued > 30, "{queued}");
    let mut frame = TransferBudget::new(usize::MAX, Duration::MAX, 10);
    let unloaded = std::iter::from_fn(|| {
        let event = s.next_event_budgeted(&mut frame)?;
        if let StreamEvent::Unload(_) = event {
            frame.record_unload(Duration::ZERO);
        }
        Some(event)
    })
    .filter(|e| matches!(e, StreamEvent::Unload(_)))
    .count();
    assert_eq!(unloaded, 10);
    assert_eq!(s.stats().unloads_waiting, queued - 10);
}

#[test]
fn a_mesh_that_comes_back_cancels_its_pending_unload() {
    let mut s = streamer();
    let home = Vec3::new(5.0, 40.0, 5.0);
    settle_without_draining(&mut s, home);
    // On s'éloigne (toutes les libérations en attente), puis on revient avant de les envoyer.
    s.update(Vec3::new(40.0 * 32.0, 40.0, 5.0));
    assert!(s.stats().unloads_waiting > 20);
    settle_without_draining(&mut s, home);
    let events: Vec<StreamEvent> = std::iter::from_fn(|| s.next_event()).collect();
    let meshed: HashSet<IVec3> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Mesh(p, _) => Some(*p),
            _ => None,
        })
        .collect();
    assert!(!meshed.is_empty());
    // Un Unload après un Mesh du même chunk l'effacerait côté GPU : il n'y en a jamais.
    let mut seen = HashSet::default();
    for event in &events {
        match event {
            StreamEvent::Mesh(p, _) => {
                seen.insert(*p);
            }
            StreamEvent::Unload(p) => {
                assert!(!seen.contains(p), "{p} libéré après son maillage")
            }
        }
    }
}

#[test]
fn a_chunk_edited_while_meshing_is_remeshed_once_more_not_once_per_edit() {
    let mut s = streamer();
    let player = Vec3::new(5.0, 40.0, 5.0);
    pump(&mut s, player);
    let target = IVec3::new(0, 1, 0);
    let (done, discarded) = (s.jobs_done, s.jobs_discarded);
    // Dix modifications visuelles d'affilée, bien plus vite qu'un maillage ne tourne.
    for _ in 0..10 {
        s.invalidate_block_visuals(IVec3::new(5, 40, 5));
        s.update(player);
    }
    assert!(s.in_flight <= 8, "{}", s.in_flight);
    let events = pump(&mut s, player);
    // Au plus deux maillages par chunk touché (le premier, puis un seul de rattrapage), et aucun
    // résultat jeté : avant, chacune des dix modifications lançait sa tâche et jetait la précédente.
    let touched = [target, IVec3::new(0, 2, 0), IVec3::new(0, 0, 0)];
    let jobs = s.jobs_done - done;
    assert!(
        jobs <= 2 * touched.len() as u64,
        "{jobs} tâches pour dix modifications"
    );
    assert_eq!(s.jobs_discarded - discarded, 0);
    let latest = meshed(&events);
    assert!(
        latest.contains_key(&target),
        "le chunk modifié a été remaillé"
    );
}
