//! Les tests de plusieurs centres de chargement (voir le module parent).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::*;
use crate::generator::TerrainGenerator;
use crate::streaming::{StreamEvent, StreamingConfig};

const RADIUS: i32 = 2;

/// Deux endroits assez éloignés pour que leurs zones (rayon de 2 chunks, plus 3 de génération) ne
/// se touchent pas.
const HERE: Vec3 = Vec3::new(4.0, 80.0, 4.0);
const THERE: Vec3 = Vec3::new(1200.0, 80.0, 4.0);

fn streamer() -> ChunkStreamer {
    let config = StreamingConfig {
        view_radius: RADIUS,
        unload_margin: 1,
        ..StreamingConfig::default()
    };
    ChunkStreamer::new(TerrainGenerator::new(7), config)
}

fn view(position: Vec3, mesh: bool) -> View {
    View {
        position,
        forward: Vec3::ZERO,
        mesh,
    }
}

/// Fait tourner le streamer jusqu'à ce qu'il n'ait plus rien à faire ; renvoie les positions
/// maillées et libérées.
fn pump(streamer: &mut ChunkStreamer, views: &[View]) -> (BTreeSet<[i32; 3]>, BTreeSet<[i32; 3]>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    let (mut meshed, mut unloaded) = (BTreeSet::new(), BTreeSet::new());
    loop {
        streamer.update_views(views);
        while let Some(event) = streamer.next_event() {
            match event {
                StreamEvent::Mesh(pos, _) => meshed.insert(pos.to_array()),
                StreamEvent::Unload(pos) => unloaded.insert(pos.to_array()),
            };
        }
        if streamer.is_idle() {
            return (meshed, unloaded);
        }
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn chunk_of(position: Vec3) -> IVec3 {
    World::chunk_containing(position)
}

#[test]
fn two_distant_views_load_both_zones() {
    let mut s = streamer();
    pump(&mut s, &[view(HERE, true), view(THERE, false)]);
    for position in [HERE, THERE] {
        let c = chunk_of(position);
        assert!(
            s.is_generated(c),
            "le chunk sous {position} n'est pas chargé"
        );
        assert!(s.is_lit(c), "le chunk sous {position} n'est pas éclairé");
    }
    // Entre les deux, rien.
    assert!(!s.is_generated(chunk_of(Vec3::new(600.0, 80.0, 4.0))));
}

#[test]
fn only_the_mesh_view_gets_meshes() {
    let mut s = streamer();
    let (meshed, _) = pump(&mut s, &[view(HERE, true), view(THERE, false)]);
    assert!(!meshed.is_empty(), "la vue locale doit être maillée");
    let near = chunk_of(HERE);
    for pos in &meshed {
        let d = IVec3::from_array(*pos) - near;
        assert!(
            d.x * d.x + d.z * d.z <= RADIUS * RADIUS,
            "chunk {pos:?} maillé loin de la vue locale"
        );
    }
}

#[test]
fn without_a_mesh_view_nothing_is_meshed_but_everything_loads() {
    let mut s = streamer();
    let (meshed, _) = pump(&mut s, &[view(HERE, false), view(THERE, false)]);
    assert!(meshed.is_empty());
    assert!(s.is_generated(chunk_of(HERE)) && s.is_generated(chunk_of(THERE)));
    assert!(s.is_lit(chunk_of(HERE)) && s.is_lit(chunk_of(THERE)));
}

#[test]
fn removing_a_view_frees_its_zone_and_only_it() {
    let mut s = streamer();
    pump(&mut s, &[view(HERE, true), view(THERE, false)]);
    let (_, unloaded) = pump(&mut s, &[view(HERE, true)]);
    assert!(
        !s.is_generated(chunk_of(THERE)),
        "la zone retirée reste chargée"
    );
    assert!(
        s.is_generated(chunk_of(HERE)),
        "la zone gardée a été libérée"
    );
    assert!(!unloaded.is_empty());
    let there = chunk_of(THERE);
    for pos in &unloaded {
        let d = IVec3::from_array(*pos) - there;
        assert!(
            d.x * d.x + d.z * d.z <= (RADIUS + 3 + 1).pow(2),
            "chunk {pos:?} libéré hors de la zone retirée"
        );
    }
}

#[test]
fn overlapping_views_share_chunks_and_free_one_only_when_both_are_gone() {
    let mut s = streamer();
    // Deux joueurs à 2 chunks l'un de l'autre : leurs disques se recouvrent.
    let a = HERE;
    let b = HERE + Vec3::new(64.0, 0.0, 0.0);
    pump(&mut s, &[view(a, true), view(b, false)]);
    let shared = chunk_of(a + Vec3::new(32.0, 0.0, 0.0));
    assert!(s.is_generated(shared));
    // Le second part : ce que le premier voit reste.
    pump(&mut s, &[view(a, true)]);
    assert!(s.is_generated(chunk_of(a)));
    assert!(s.is_generated(shared) || s.is_generated(chunk_of(a)));
    // Le premier part aussi : plus aucune vue ne tient ce chunk, mais `update_views(&[])` ne
    // libère rien (aucune vue, aucun ordre).
    pump(&mut s, &[]);
    assert!(s.is_generated(chunk_of(a)));
}

#[test]
fn one_view_behaves_exactly_like_update_view() {
    let mut old = streamer();
    let mut new = streamer();
    let forward = Vec3::new(1.0, 0.0, 0.3);
    // La même trajectoire (des sauts de plusieurs chunks), chaque fois jusqu'au repos.
    let (mut old_meshed, mut new_meshed) = (BTreeSet::new(), BTreeSet::new());
    for step in 0..4 {
        let player = HERE + Vec3::new(step as f32 * 150.0, 0.0, 0.0);
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            old.update_view(player, forward);
            while let Some(e) = old.next_event() {
                if let StreamEvent::Mesh(pos, _) = e {
                    old_meshed.insert(pos.to_array());
                }
            }
            if old.is_idle() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let views = [View {
            position: player,
            forward,
            mesh: true,
        }];
        let (meshed, _) = pump(&mut new, &views);
        new_meshed.extend(meshed);
    }
    assert_eq!(old_meshed, new_meshed);
    for x in -6..40 {
        for z in -6..6 {
            let c = IVec3::new(x, 2, z);
            assert_eq!(old.is_generated(c), new.is_generated(c), "chunk {c}");
            assert_eq!(old.is_lit(c), new.is_lit(c), "chunk {c}");
        }
    }
}
