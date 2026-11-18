//! Le terrain lointain montre les constructions et les creusements du joueur (lot 8) : un résumé
//! par colonne des chunks modifiés, lu paresseusement sur disque, reconstruisant les tuiles montrées.

use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, CHUNK_SIZE, Chunk, ChunkSurface, ChunkToSave, EditedSurfaces, FarConfig, FarEvent,
    FarStreamer, FarTile, SaveStore, TerrainGenerator, build_tile, build_tile_with,
};
use glam::{IVec3, Vec3};

fn generator() -> Arc<TerrainGenerator> {
    Arc::new(TerrainGenerator::new(42))
}

fn block(generator: &TerrainGenerator, key: &str) -> BlockId {
    generator
        .registry()
        .id(key)
        .unwrap_or_else(|| panic!("{key}"))
}

/// Un chunk avec une colonne de `key` de y = 0 à `height` (inclus) en (x, z) locaux.
fn pillar(generator: &TerrainGenerator, key: &str, x: usize, z: usize, height: usize) -> Chunk {
    let id = block(generator, key);
    let mut chunk = Chunk::new();
    for y in 0..=height {
        chunk.set(x, y, z, id);
    }
    chunk
}

#[test]
fn chunk_surface_keeps_ground_and_ignores_logs_leaves_and_air() {
    let g = generator();
    let registry = g.registry();
    let mut chunk = Chunk::new();
    // Colonne (1, 1) : pierre jusqu'à 5, puis une bûche et des feuilles par-dessus.
    for y in 0..=5 {
        chunk.set(1, y, 1, block(&g, "core:stone"));
    }
    chunk.set(1, 6, 1, block(&g, "core:oak_log"));
    chunk.set(1, 7, 1, block(&g, "core:oak_leaves"));
    // Colonne (2, 2) : que du bois.
    chunk.set(2, 3, 2, block(&g, "core:oak_log"));
    let surface = ChunkSurface::of(&chunk, registry);
    assert_eq!(surface.at(1, 1), Some((5, block(&g, "core:stone"))));
    assert_eq!(surface.at(2, 2), None);
    assert_eq!(surface.at(0, 0), None);
}

#[test]
fn surface_at_prefers_edited_chunks_at_or_above_the_generated_ground() {
    let g = generator();
    let registry = g.registry();
    let edits = EditedSurfaces::new();
    let (x, z) = (3, 5);
    let ground = g.column(x, z).height;
    let ground_chunk = ground.div_euclid(CHUNK_SIZE as i32);
    // Aucun chunk modifié : le générateur décide.
    assert_eq!(edits.surface_at(x, z, ground, registry), None);

    // Une tour de pierre dans le chunk au-dessus du sol : elle prime.
    let above = ground_chunk + 1;
    edits.record(
        IVec3::new(0, above, 0),
        &pillar(&g, "core:stone", 3, 5, 10),
        registry,
    );
    let (y, id) = edits.surface_at(x, z, ground, registry).expect("tour");
    assert_eq!(y, above * CHUNK_SIZE as i32 + 10);
    assert_eq!(id, block(&g, "core:stone"));

    // Un chunk modifié sous le sol généré, avec son chunk de sol intact : le générateur a raison.
    let edits = EditedSurfaces::new();
    if ground_chunk > 0 {
        edits.record(
            IVec3::new(0, ground_chunk - 1, 0),
            &pillar(&g, "core:stone", 3, 5, 10),
            registry,
        );
        assert_eq!(edits.surface_at(x, z, ground, registry), None);
    }

    // Le chunk du sol modifié : un fond de fosse à y local 3 remplace le sol généré.
    let edits = EditedSurfaces::new();
    edits.record(
        IVec3::new(0, ground_chunk, 0),
        &pillar(&g, "core:dirt", 3, 5, 3),
        registry,
    );
    let (y, _) = edits.surface_at(x, z, ground, registry).expect("fosse");
    assert_eq!(y, ground_chunk * CHUNK_SIZE as i32 + 3);
}

#[test]
fn far_tile_shows_an_edited_tower_that_the_generator_does_not_have() {
    let g = generator();
    let registry = g.registry();
    // La tuile fine à l'origine : 64 blocs de côté, une cellule tous les 2 blocs.
    let tile = FarTile {
        level: 0,
        x: 0,
        z: 0,
    };
    let plain = build_tile(&g, tile);
    let edits = EditedSurfaces::new();
    let ground_chunk = g.column(4, 4).height.div_euclid(CHUNK_SIZE as i32);
    let above = (ground_chunk + 1).min(5);
    edits.record(
        IVec3::new(0, above, 0),
        &pillar(&g, "core:stone", 4, 4, 20),
        registry,
    );
    let edited = build_tile_with(&g, tile, Some(&edits));
    // Les tuiles fines se posent 1 bloc sous le décor détaillé qu'elles recouvrent.
    let top = (above * CHUNK_SIZE as i32 + 20) as f32;
    assert!(
        edited.bounds.1.y >= top - 0.01,
        "tour absente : {}",
        edited.bounds.1.y
    );
    assert!(
        plain.bounds.1.y < top,
        "la tuile sans modification contient déjà la tour"
    );
    assert_ne!(plain, edited);
}

#[test]
fn saved_chunks_are_read_lazily_from_the_world_folder() {
    let g = generator();
    let registry = g.registry();
    let dir = std::env::temp_dir().join(format!("voxel-far-edits-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(SaveStore::open(&dir).unwrap());
    let ground_chunk = g.column(3, 5).height.div_euclid(CHUNK_SIZE as i32);
    let above = (ground_chunk + 1).min(5);
    let chunk = pillar(&g, "core:stone", 3, 5, 7);
    store
        .save_chunks(
            &[ChunkToSave {
                pos: IVec3::new(0, above, 0),
                chunk: Arc::new(chunk),
                unknown: Default::default(),
            }],
            registry,
        )
        .unwrap();
    // Un résumé neuf, sans rien noté : il lit le dossier la première fois qu'on l'interroge.
    let edits = EditedSurfaces::new();
    edits.attach_store(store);
    let found = edits.surface_at(3, 5, g.column(3, 5).height, registry);
    assert_eq!(found.map(|(y, _)| y), Some(above * CHUNK_SIZE as i32 + 7));
    // Une autre colonne de chunks, jamais sauvegardée : rien.
    assert_eq!(
        edits.surface_at(100, 100, g.column(100, 100).height, registry),
        None
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shown_tiles_are_rebuilt_in_place_when_a_chunk_summary_changes() {
    let g = generator();
    let registry = g.registry().clone();
    let edits = Arc::new(EditedSurfaces::new());
    let config = FarConfig {
        near_radius: 2,
        far_radius: 6,
    };
    let mut far = FarStreamer::with_edits(g.clone(), config, 2, Some(edits.clone()));
    let camera = Vec3::new(16.0, 80.0, 16.0);
    let deadline = Instant::now() + Duration::from_secs(60);
    // On attend que tout soit montré.
    let mut tiles = std::collections::HashSet::new();
    loop {
        for event in far.update(camera) {
            match event {
                FarEvent::Add(t, _) => {
                    tiles.insert(t);
                }
                FarEvent::Remove(t) => {
                    tiles.remove(&t);
                }
            }
        }
        let stats = far.stats();
        if stats.pending == 0
            && stats.staged == 0
            && stats.waiting_batches == 0
            && !tiles.is_empty()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "le terrain lointain n'est pas prêt"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Un chunk modifié sous une tuile montrée (hors du rayon proche).
    let (cx, cz) = (4, 4);
    let covering: Vec<FarTile> = tiles
        .iter()
        .copied()
        .filter(|t| {
            let s = t.size_chunks();
            (t.x * s..(t.x + 1) * s).contains(&cx) && (t.z * s..(t.z + 1) * s).contains(&cz)
        })
        .collect();
    assert_eq!(covering.len(), 1, "une tuile couvre la colonne de chunks");
    let (wx, wz) = (cx * 32 + 4, cz * 32 + 4);
    let ground_chunk = g.column(wx, wz).height.div_euclid(32);
    let above = (ground_chunk + 1).min(5);
    edits.record(
        IVec3::new(cx, above, cz),
        &pillar(&g, "core:stone", 4, 4, 20),
        &registry,
    );
    let top = (above * 32 + 20) as f32;
    let mut rebuilt = None;
    while rebuilt.is_none() {
        for event in far.update(camera) {
            if let FarEvent::Add(t, mesh) = event
                && t == covering[0]
            {
                rebuilt = Some(mesh);
            }
        }
        assert!(
            Instant::now() < deadline,
            "la tuile n'a pas été reconstruite"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rebuilt.unwrap().bounds.1.y >= top - 0.01);
}

#[test]
fn the_debug_tint_tells_observed_cells_from_estimated_ones() {
    use engine_voxel::{FarDebug, build_tile_debug};
    let g = generator();
    let registry = g.registry();
    let tile = FarTile {
        level: 0,
        x: 0,
        z: 0,
    };
    let magenta = |mesh: &engine_voxel::FarMesh| {
        mesh.vertices
            .iter()
            .filter(|v| v.color[3] == 255)
            .any(|v| v.color[0] > 200 && v.color[1] < 60 && v.color[2] > 160)
    };
    // Sans chunk modifié, rien n'est « observé ».
    let none = EditedSurfaces::new();
    assert!(!magenta(&build_tile_debug(
        &g,
        tile,
        Some(&none),
        FarDebug::Sources
    )));
    // Avec une tour, la cellule observée est magenta ; sans diagnostic, elle garde sa couleur.
    let edits = EditedSurfaces::new();
    let above = (g.column(4, 4).height.div_euclid(32) + 1).min(5);
    edits.record(
        IVec3::new(0, above, 0),
        &pillar(&g, "core:stone", 4, 4, 20),
        registry,
    );
    assert!(magenta(&build_tile_debug(
        &g,
        tile,
        Some(&edits),
        FarDebug::Sources
    )));
    assert!(!magenta(&build_tile_debug(
        &g,
        tile,
        Some(&edits),
        FarDebug::Off
    )));
    // Les niveaux se distinguent : la même cellule n'a pas la même teinte à deux niveaux.
    let level0 = build_tile_debug(
        &g,
        FarTile {
            level: 0,
            x: 0,
            z: 0,
        },
        None,
        FarDebug::Levels,
    );
    let level2 = build_tile_debug(
        &g,
        FarTile {
            level: 2,
            x: 0,
            z: 0,
        },
        None,
        FarDebug::Levels,
    );
    assert_ne!(level0.vertices[0].color, level2.vertices[0].color);
}
