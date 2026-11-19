//! Le monde haut (lot 0 du plan v3) : couches de chunks sous y = 0 et au-dessus de 191, avec le vrai
//! streamer, la lumière, la sauvegarde et les modifications. La génération v1 met de la pierre creusée
//! d'une salle fermée par chunk sous 0 quand on lui impose une plage plus grande (`with_chunk_range`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, ChunkStreamer, ChunkToSave, MAX_CHUNK_Y, MIN_CHUNK_Y, SaveStore, StreamEvent,
    StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

fn deep() -> TerrainGenerator {
    TerrainGenerator::new(7).with_chunk_range(MIN_CHUNK_Y..MAX_CHUNK_Y)
}

fn streamer(generator: TerrainGenerator, store: Option<Arc<SaveStore>>) -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: 2,
        unload_margin: 1,
        ..StreamingConfig::default()
    };
    let s = ChunkStreamer::new(generator, config);
    match store {
        Some(store) => s.with_save(store),
        None => s,
    }
}

fn settle(s: &mut ChunkStreamer, player: Vec3) -> Vec<StreamEvent> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut events = Vec::new();
    loop {
        s.update(player);
        events.extend(std::iter::from_fn(|| s.next_event()));
        if s.is_idle() {
            return events;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn the_world_range_follows_the_generator() {
    assert_eq!(TerrainGenerator::new(7).chunk_range(), 0..6, "v1 inchangée");
    assert_eq!(deep().chunk_range(), -4..8);
    // Bornée au stockage.
    assert_eq!(
        TerrainGenerator::new(7)
            .with_chunk_range(-50..50)
            .chunk_range(),
        MIN_CHUNK_Y..MAX_CHUNK_Y
    );
    let s = streamer(deep(), None);
    assert_eq!(s.world().vertical(), -4..8);
    assert_eq!(s.world().block_y_range(), -128..256);
}

#[test]
fn chunks_below_zero_load_light_and_mesh() {
    let mut s = streamer(deep(), None);
    let events = settle(&mut s, Vec3::new(16.0, 90.0, 16.0));
    // Les couches sous 0 sont chargées et pleines de pierre.
    assert!(s.is_generated(IVec3::new(0, -4, 0)));
    assert!(s.is_generated(IVec3::new(0, 7, 0)));
    let below = s.world().block(IVec3::new(5, -100, 5));
    assert!(!below.is_air(), "sous 0 : de la pierre");
    // Dans la roche, la lumière du ciel est nulle ; sous le monde, il fait noir.
    assert_eq!(s.world().light(IVec3::new(5, -100, 5)).0, 0);
    assert_eq!(s.world().light(IVec3::new(5, -129, 5)), (0, 0));
    // Un puits creusé jusqu'à -60 : le ciel y descend (mise à jour incrémentale de la lumière).
    // Depuis le haut du monde : un arbre au-dessus du sol couperait le ciel.
    for y in (-60..255).rev() {
        if !s.world().block(IVec3::new(5, y, 5)).is_air() {
            assert!(s.set_block(IVec3::new(5, y, 5), BlockId::AIR));
        }
    }
    while s.next_event().is_some() {}
    assert_eq!(
        s.world().light(IVec3::new(5, -60, 5)).0,
        15,
        "le ciel tombe au fond du puits"
    );
    // Des maillages existent pour des chunks sous 0 dès qu'ils ont des faces visibles : le puits.
    let mut meshed_below = events
        .iter()
        .any(|e| matches!(e, StreamEvent::Mesh(p, _) if p.y < 0));
    settle(&mut s, Vec3::new(16.0, 90.0, 16.0));
    meshed_below |= s.world().chunk(IVec3::new(0, -2, 0)).is_some();
    assert!(meshed_below);
}

#[test]
fn deep_dark_chunks_are_meshed_only_when_the_player_goes_down() {
    let mut s = streamer(deep(), None);
    let meshed_at = |events: &[StreamEvent], layer: i32| {
        events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Mesh(p, _) if p.y == layer))
            .count()
    };
    // En surface (couche 2) : rien de maillé dans la roche de la couche -4 (sans ciel, 6 couches plus bas).
    let surface = settle(&mut s, Vec3::new(16.0, 90.0, 16.0));
    assert!(meshed_at(&surface, 2) > 0, "la surface est maillée");
    assert_eq!(meshed_at(&surface, -4), 0, "la roche profonde attend");
    // Le joueur descend à y = -100 (couche -4) : elle est maillée à son tour.
    let deep_events = settle(&mut s, Vec3::new(16.0, -100.0, 16.0));
    assert!(
        meshed_at(&deep_events, -4) > 0,
        "la couche profonde est maillée en descendant"
    );
    // Sans limite, tout est maillé d'emblée.
    let config = StreamingConfig {
        view_radius: 2,
        mesh_depth: None,
        ..StreamingConfig::default()
    };
    let mut all = ChunkStreamer::new(deep(), config);
    let events = settle(&mut all, Vec3::new(16.0, 90.0, 16.0));
    assert!(meshed_at(&events, -4) > 0);
}

#[test]
fn chunks_below_zero_are_saved_and_reloaded() {
    let dir = std::env::temp_dir().join(format!("voxel-vertical-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Directement par le stockage : une position négative.
    let store = Arc::new(SaveStore::open(&dir).unwrap());
    let generator = deep();
    let registry = generator.registry().clone();
    let chunk = generator
        .generate(IVec3::new(2, -3, -1))
        .expect("chunk de pierre");
    store
        .save_chunks(
            &[ChunkToSave {
                pos: IVec3::new(2, -3, -1),
                chunk: Arc::new(chunk.clone()),
                unknown: Default::default(),
            }],
            &registry,
        )
        .unwrap();
    let back = store
        .load_chunk(IVec3::new(2, -3, -1), &registry)
        .unwrap()
        .unwrap();
    assert_eq!(back.chunk.blocks(), chunk.blocks());
    assert_eq!(store.saved_heights(2, -1), vec![-3]);

    // Par le streamer : une modification sous 0 survit au déchargement et à la réouverture.
    let dir2 = dir.join("monde");
    let store = Arc::new(SaveStore::open(&dir2).unwrap());
    let mut first = streamer(deep(), Some(store));
    let here = Vec3::new(16.0, 90.0, 16.0);
    settle(&mut first, here);
    let pos = IVec3::new(4, -77, 9);
    let glass = first.world().registry().id("core:glass").unwrap();
    assert!(first.set_block(pos, glass));
    first.save_blocking().unwrap();
    drop(first);
    let store = Arc::new(SaveStore::open(&dir2).unwrap());
    let mut second = streamer(deep(), Some(store));
    settle(&mut second, here);
    assert_eq!(second.world().block(pos), glass);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Le coût d'un monde de 12 couches contre 6 : chargement complet autour d'un point, même générateur.
/// `cargo test -p engine_voxel --test vertical_range print_deep_world_cost --release -- --ignored --nocapture`
#[test]
#[ignore = "mesure"]
fn print_deep_world_cost() {
    for radius in [6, 10] {
        for (name, generator) in [
            ("6 couches", TerrainGenerator::new(7)),
            ("12 couches", deep()),
        ] {
            let config = StreamingConfig {
                view_radius: radius,
                ..StreamingConfig::default()
            };
            let mut s = ChunkStreamer::new(generator, config);
            let started = Instant::now();
            let events = settle(&mut s, Vec3::new(16.0, 90.0, 16.0));
            let meshes = events
                .iter()
                .filter(|e| matches!(e, StreamEvent::Mesh(..)))
                .count();
            println!(
                "rayon {radius}, {name} : {:.2} s, {} chunks chargés, {} gardés en mémoire, {meshes} maillages",
                started.elapsed().as_secs_f64(),
                s.stats().loaded,
                s.world().chunk_positions().count(),
            );
        }
    }
}
