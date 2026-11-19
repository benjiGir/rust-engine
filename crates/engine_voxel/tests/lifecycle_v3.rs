//! Le cycle de vie d'un monde v3 (celui des nouveaux mondes) : création, modification, sauvegarde, réouverture, retour dans
//! une zone modifiée et chargement dans un ordre différent, avec le vrai streamer et un vrai dossier.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_voxel::{
    BlockId, ChunkStreamer, GenerationVersion, SaveStore, StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

const RADIUS: i32 = 2;

fn v3(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V3)
        .expect("v3")
}

fn streamer(generator: TerrainGenerator, store: Option<Arc<SaveStore>>) -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: RADIUS,
        unload_margin: 1,
        ..StreamingConfig::default()
    };
    let streamer = ChunkStreamer::new(generator, config);
    match store {
        Some(store) => streamer.with_save(store),
        None => streamer,
    }
}

/// Charge tout autour de `player` et attend que le streamer soit au repos.
fn settle(streamer: &mut ChunkStreamer, player: Vec3) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        streamer.update(player);
        while streamer.next_event().is_some() {}
        if streamer.is_idle() {
            return;
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Les blocs de tous les chunks chargés, par position.
fn snapshot(streamer: &ChunkStreamer) -> HashMap<IVec3, Vec<BlockId>> {
    streamer
        .world()
        .chunk_positions()
        .map(|pos| {
            (
                pos,
                streamer
                    .world()
                    .chunk(pos)
                    .expect("chargé")
                    .blocks()
                    .to_vec(),
            )
        })
        .collect()
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("voxel-lifecycle-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_v3_world_keeps_its_edits_across_save_and_reopen_and_the_rest_regenerates_identically() {
    let dir = temp("reopen");
    let here = Vec3::new(40.0, 90.0, 40.0);
    let far = Vec3::new(40.0 + 20.0 * 32.0, 90.0, 40.0);
    let dug = IVec3::new(41, 30, 41);

    // Première session : on creuse, on construit, on sauvegarde, on s'éloigne.
    let store = Arc::new(SaveStore::open(&dir).unwrap());
    let mut first = streamer(v3(2024), Some(store.clone()));
    settle(&mut first, here);
    let stone = first.world().registry().id("core:stone").unwrap();
    // Une construction dans l'air, juste au-dessus du sol (jamais de la pierre déjà là).
    let built = IVec3::new(45, first.generator().column(45, 45).height + 30, 45);
    assert_eq!(first.world().block(built), BlockId::AIR);
    let built_chunk = IVec3::new(1, built.y.div_euclid(32), 1);
    let original = first.world().block(dug);
    assert!(first.set_block(dug, BlockId::AIR));
    assert!(first.set_block(built, stone));
    first.save_blocking().unwrap();
    let before = snapshot(&first);
    settle(&mut first, far);
    assert!(
        first.world().chunk(IVec3::new(1, 0, 1)).is_none(),
        "déchargé"
    );
    drop(first);

    // Seconde session : un autre processus simulé, même dossier, même graine, même version.
    let store = Arc::new(SaveStore::open(&dir).unwrap());
    let mut second = streamer(v3(2024), Some(store));
    settle(&mut second, here);
    assert_eq!(second.world().block(dug), BlockId::AIR, "creusement perdu");
    assert_eq!(second.world().block(built), stone, "construction perdue");
    assert_ne!(original, BlockId::AIR, "le test creuse bien quelque chose");
    // Tout le reste est identique à la première session (régénéré ou relu).
    let after = snapshot(&second);
    for (pos, blocks) in &before {
        assert_eq!(after.get(pos), Some(blocks), "chunk {pos} différent");
    }
    // Un monde vierge de même graine donne les mêmes blocs partout sauf aux deux modifications.
    let mut fresh = streamer(v3(2024), None);
    settle(&mut fresh, here);
    let reference = snapshot(&fresh);
    // Un chunk entièrement vide n'est pas gardé dans le monde : on compare l'union des positions.
    let empty: Vec<BlockId> = Vec::new();
    let all_air = |blocks: &Vec<BlockId>| blocks.iter().all(|b| b.is_air());
    let positions: std::collections::HashSet<IVec3> =
        reference.keys().chain(after.keys()).copied().collect();
    let mut different = 0;
    for pos in positions {
        let (a, b) = (
            after.get(&pos).unwrap_or(&empty),
            reference.get(&pos).unwrap_or(&empty),
        );
        if a != b && !(all_air(a) && all_air(b)) {
            different += 1;
            assert!(
                pos == IVec3::new(1, 0, 1) || pos == built_chunk,
                "chunk {pos} modifié sans raison"
            );
        }
    }
    assert_eq!(
        different, 2,
        "exactement les deux chunks modifiés diffèrent"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loading_in_a_different_order_gives_the_same_chunks() {
    let a = Vec3::new(40.0, 90.0, 40.0);
    let b = Vec3::new(40.0 + 5.0 * 32.0, 90.0, 40.0 + 3.0 * 32.0);
    // Ordre A puis B, puis retour en A ; et ordre B puis A.
    let mut one = streamer(v3(7), None);
    settle(&mut one, a);
    settle(&mut one, b);
    let mut two = streamer(v3(7), None);
    settle(&mut two, b);
    settle(&mut two, a);
    settle(&mut two, b);
    let (x, y) = (snapshot(&one), snapshot(&two));
    assert!(!x.is_empty());
    for (pos, blocks) in &x {
        assert_eq!(y.get(pos), Some(blocks), "chunk {pos} dépend de l'ordre");
    }
}
