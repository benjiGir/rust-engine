//! Les tests du serveur sans maillage, de la réplique et de l'expéditeur de chunks.

use std::time::{Duration, Instant};

use glam::Vec3;
use rustc_hash::FxHashMap as HashMap;

use super::*;
use crate::chunk_sender::ChunkSender;
use crate::generator::TerrainGenerator;
use crate::mesh::{ChunkMesh, mesh_binary};
use crate::registry::BlockRegistry;
use crate::registry::test_blocks::stone;
use crate::streaming::{StreamEvent, StreamingConfig, View};

const RADIUS: i32 = 2;

/// Deux endroits assez éloignés pour que leurs zones ne se touchent pas.
const HERE: Vec3 = Vec3::new(5.0, 40.0, 5.0);
const THERE: Vec3 = Vec3::new(1200.0, 80.0, 4.0);

fn config() -> StreamingConfig {
    StreamingConfig {
        view_radius: RADIUS,
        unload_margin: 1,
        ..StreamingConfig::default()
    }
}

fn view(position: Vec3, mesh: bool) -> View {
    View {
        position,
        forward: Vec3::ZERO,
        mesh,
    }
}

fn chunk_of(position: Vec3) -> IVec3 {
    World::chunk_containing(position)
}

fn toggled(s: &ChunkStreamer, pos: IVec3) -> crate::BlockId {
    if s.world().block(pos).is_air() {
        stone()
    } else {
        crate::BlockId::AIR
    }
}

/// Un serveur sans maillage, la réplique d'un client et l'expéditeur qui les relie.
struct Pair {
    server: ChunkStreamer,
    replica: ChunkStreamer,
    sender: ChunkSender,
}

impl Pair {
    fn new() -> Self {
        let server = ChunkStreamer::new(TerrainGenerator::new(7), config()).as_server();
        let replica = ChunkStreamer::replica_of(&server);
        Self {
            server,
            replica,
            sender: ChunkSender::new(RADIUS),
        }
    }

    /// Une image : le serveur avance, l'expéditeur envoie, la réplique applique et maille, puis on
    /// prend tout ce que la réplique donne au GPU.
    fn tick(&mut self, player: Vec3) -> Vec<StreamEvent> {
        self.server.update_views(&[view(player, false)]);
        let changes = self.server.take_changes();
        let mut updates = Vec::new();
        self.sender
            .step(&self.server, chunk_of(player), &changes, &mut updates);
        self.replica.apply_updates(updates);
        self.replica.update_views(&[view(player, true)]);
        std::iter::from_fn(|| self.replica.next_event()).collect()
    }

    fn idle(&self) -> bool {
        self.server.is_idle() && self.sender.pending() == 0 && self.replica.is_idle()
    }

    /// Fait tourner jusqu'à ce que tout soit au repos ; renvoie tous les événements.
    fn pump(&mut self, player: Vec3) -> Vec<StreamEvent> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut events = Vec::new();
        loop {
            events.extend(self.tick(player));
            if self.idle() {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "le chargement ne se termine pas : serveur {:?} (au repos : {}), expéditeur {} en attente, réplique {:?} (au repos : {})",
                self.server.stats(),
                self.server.is_idle(),
                self.sender.pending(),
                self.replica.stats(),
                self.replica.is_idle(),
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Le dernier maillage de chaque chunk.
fn latest(events: &[StreamEvent]) -> HashMap<IVec3, &ChunkMesh> {
    let mut meshes = HashMap::default();
    for event in events {
        match event {
            StreamEvent::Mesh(pos, mesh) => {
                meshes.insert(*pos, mesh);
            }
            StreamEvent::Unload(pos) => {
                meshes.remove(pos);
            }
        }
    }
    meshes
}

#[test]
fn a_server_loads_and_lights_but_never_meshes() {
    let mut server = ChunkStreamer::new(TerrainGenerator::new(7), config()).as_server();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !server.is_idle() || server.stats().loaded == 0 {
        server.update_views(&[view(HERE, true)]);
        assert!(server.next_event().is_none(), "un serveur ne maille pas");
        assert!(Instant::now() < deadline, "le chargement ne se termine pas");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(server.stats().lit_columns > 0);
    assert_eq!(server.stats().meshes_waiting, 0);
    assert!(server.is_lit(IVec3::ZERO));

    // Une édition ne maille rien non plus : elle est notée pour les expéditeurs.
    let pos = IVec3::new(31, 40, 5);
    let block = toggled(&server, pos);
    assert!(server.set_block(pos, block));
    assert!(server.next_event().is_none());
    let changes = server.take_changes();
    let edited = changes
        .iter()
        .find(|c| c.pos == IVec3::new(0, 1, 0))
        .expect("le chunk modifié est noté");
    assert_eq!(edited.edits, [pos]);
    assert!(
        server.take_changes().is_empty(),
        "les changements sont rendus une fois"
    );
}

#[test]
fn the_replica_meshes_exactly_what_a_standalone_streamer_meshes() {
    let mut alone = ChunkStreamer::new(TerrainGenerator::new(7), config());
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut expected = Vec::new();
    loop {
        alone.update(HERE);
        expected.extend(std::iter::from_fn(|| alone.next_event()));
        if alone.is_idle() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let expected = latest(&expected);

    let mut pair = Pair::new();
    let events = pair.pump(HERE);
    let got = latest(&events);

    assert!(!got.is_empty());
    let mut a: Vec<IVec3> = expected.keys().copied().collect();
    let mut b: Vec<IVec3> = got.keys().copied().collect();
    a.sort_by_key(|p| p.to_array());
    b.sort_by_key(|p| p.to_array());
    assert_eq!(b, a, "les mêmes chunks sont maillés");
    for (pos, mesh) in got {
        assert_eq!(mesh, expected[&pos], "chunk {pos} : même maillage");
    }
    // La réplique ne génère ni n'éclaire rien : elle a ce qu'on lui a envoyé.
    let stats = pair.replica.stats();
    assert_eq!(stats.queued, 0);
    assert_eq!(stats.loaded, pair.sender.sent_chunks());
}

#[test]
fn editing_a_border_block_remeshes_both_chunks_in_the_same_update() {
    let mut pair = Pair::new();
    pair.pump(HERE);
    let pos = IVec3::new(31, 40, 5);
    let block = toggled(&pair.server, pos);
    assert!(pair.server.set_block(pos, block));
    // Une seule image : le serveur change, la réplique applique et remaille tout de suite.
    let events = pair.tick(HERE);
    assert_eq!(pair.replica.world().block(pos), block);
    let positions: Vec<IVec3> = events
        .iter()
        .take(2)
        .map(|e| match e {
            StreamEvent::Mesh(p, mesh) => {
                assert_eq!(
                    mesh,
                    &mesh_binary(pair.replica.world(), *p),
                    "maillage à jour"
                );
                *p
            }
            StreamEvent::Unload(p) => panic!("déchargement inattendu de {p}"),
        })
        .collect();
    assert_eq!(positions, [IVec3::new(0, 1, 0), IVec3::new(1, 1, 0)]);
}

#[test]
fn a_block_placed_in_an_empty_chunk_is_sent_and_meshed() {
    let mut pair = Pair::new();
    pair.pump(HERE);
    // En plein ciel : le chunk n'existait pas.
    let pos = IVec3::new(5, 175, 5);
    assert!(pair.server.world().chunk(chunk_of(pos.as_vec3())).is_none());
    assert!(pair.server.set_block(pos, stone()));
    let mut events = pair.tick(HERE);
    events.extend(pair.pump(HERE));
    assert_eq!(pair.replica.world().block(pos), stone());
    let meshes = latest(&events);
    let mesh = meshes[&chunk_of(pos.as_vec3())];
    assert!(!mesh.is_empty());
    for (pos, mesh) in meshes {
        assert_eq!(mesh, &mesh_binary(pair.replica.world(), pos), "chunk {pos}");
    }
}

#[test]
fn torch_light_reaches_and_remeshes_the_next_chunk_on_the_replica() {
    let mut pair = Pair::new();
    pair.pump(HERE);
    let torch = BlockRegistry::builtin().id("core:torch").unwrap();
    // Dans le ciel, à 12 blocs du chunk x = 1 : sa lumière y déborde.
    let pos = IVec3::new(20, 120, 5);
    assert!(pair.server.set_block(pos, torch));
    let events = pair.pump(HERE);
    assert_eq!(pair.replica.world().light(pos).1, 14);
    assert_eq!(pair.replica.world().light(IVec3::new(33, 120, 5)).1, 1);
    // Le chunk voisin n'a changé que de lumière : il est remaillé, à jour.
    let meshes = latest(&events);
    assert!(meshes.contains_key(&IVec3::new(1, 3, 0)));
    for (pos, mesh) in meshes {
        assert_eq!(mesh, &mesh_binary(pair.replica.world(), pos), "chunk {pos}");
    }
}

#[test]
fn the_replica_follows_the_player_and_unloads_what_is_far() {
    let mut pair = Pair::new();
    pair.pump(HERE);
    let before = pair.replica.stats().loaded;
    assert!(before > 0);
    let events = pair.pump(THERE);
    assert!(
        events.iter().any(|e| matches!(e, StreamEvent::Unload(_))),
        "les maillages d'avant sont libérés"
    );
    // Tout est de l'autre côté : plus rien près de l'ancienne place.
    assert!(pair.replica.world().chunk(IVec3::new(0, 1, 0)).is_none());
    assert!(pair.replica.world().chunk_count() > 0);
    assert_eq!(pair.replica.stats().loaded, pair.sender.sent_chunks());
    // Les maillages encore vivants sont autour de la nouvelle place.
    let centre = chunk_of(THERE);
    for pos in latest(&events).keys() {
        assert!(super::super::horizontal_distance_sq(*pos, centre) <= RADIUS * RADIUS);
    }
}

#[test]
fn the_sender_goes_from_the_nearest_column_to_the_farthest_under_its_budget() {
    let mut server = ChunkStreamer::new(TerrainGenerator::new(7), config()).as_server();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !server.is_idle() || server.stats().loaded == 0 {
        server.update_views(&[view(HERE, false)]);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut sender = ChunkSender::new(RADIUS).with_budget(1);
    let player = chunk_of(HERE);
    let mut distances = Vec::new();
    let mut steps = 0;
    while sender.pending() > 0 || steps == 0 {
        let mut out = Vec::new();
        sender.step(&server, player, &[], &mut out);
        // Au plus une colonne entière par pas.
        let columns: std::collections::BTreeSet<[i32; 2]> = out
            .iter()
            .map(|u| match u {
                ChunkUpdate::Chunk { pos, .. } => [pos.x, pos.z],
                other => panic!("message inattendu : {other:?}"),
            })
            .collect();
        assert!(columns.len() <= 1, "{columns:?}");
        for [x, z] in columns {
            assert!(
                server.is_lit(IVec3::new(x, 0, z)),
                "une colonne non éclairée est partie"
            );
            distances.push((x - player.x).pow(2) + (z - player.z).pow(2));
        }
        steps += 1;
        assert!(steps < 1000);
    }
    assert!(distances.len() > 10);
    assert!(
        distances.windows(2).all(|w| w[0] <= w[1]),
        "du plus proche au plus lointain : {distances:?}"
    );
}

#[test]
fn the_sender_waits_for_the_light() {
    let mut server = ChunkStreamer::new(TerrainGenerator::new(7), config()).as_server();
    let mut sender = ChunkSender::new(RADIUS);
    let mut out = Vec::new();
    // Rien n'est chargé : rien à envoyer.
    sender.step(&server, chunk_of(HERE), &[], &mut out);
    assert!(out.is_empty());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        server.update_views(&[view(HERE, false)]);
        out.clear();
        sender.step(&server, chunk_of(HERE), &[], &mut out);
        for update in &out {
            let ChunkUpdate::Chunk { pos, .. } = update else {
                panic!("message inattendu");
            };
            assert!(server.is_lit(*pos), "chunk {pos} envoyé avant sa lumière");
        }
        if server.is_idle() && sender.pending() == 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_changed_chunk_goes_only_to_the_players_who_have_it() {
    let mut server = ChunkStreamer::new(TerrainGenerator::new(7), config()).as_server();
    let deadline = Instant::now() + Duration::from_secs(60);
    let views = [view(HERE, false), view(THERE, false)];
    while !server.is_idle() || server.stats().loaded == 0 {
        server.update_views(&views);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut here = ChunkSender::new(RADIUS);
    let mut there = ChunkSender::new(RADIUS);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    here.step(&server, chunk_of(HERE), &[], &mut a);
    there.step(&server, chunk_of(THERE), &[], &mut b);
    // Deux envois indépendants : chacun a sa zone, et seulement elle.
    assert!(here.sent_chunks() > 0 && there.sent_chunks() > 0);
    assert!(here.has(IVec3::new(0, 1, 0)) && !there.has(IVec3::new(0, 1, 0)));
    let far = chunk_of(THERE);
    assert!(there.has(IVec3::new(far.x, 1, far.z)) && !here.has(IVec3::new(far.x, 1, far.z)));
    server.take_changes();

    let pos = IVec3::new(5, 40, 5);
    let block = toggled(&server, pos);
    assert!(server.set_block(pos, block));
    let changes = server.take_changes();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    here.step(&server, chunk_of(HERE), &changes, &mut a);
    there.step(&server, chunk_of(THERE), &changes, &mut b);
    let changed_here = a.iter().any(|u| {
        matches!(u, ChunkUpdate::Changed { pos: p, edits, .. }
            if *p == IVec3::new(0, 1, 0) && edits == &[pos])
    });
    assert!(
        changed_here,
        "celui qui a le chunk le reçoit, avec la position éditée"
    );
    assert!(b.is_empty(), "celui qui ne l'a pas ne reçoit rien : {b:?}");
}

#[test]
fn a_change_shares_the_chunk_without_copying_it() {
    let mut pair = Pair::new();
    pair.pump(HERE);
    let pos = IVec3::new(5, 40, 5);
    let block = toggled(&pair.server, pos);
    pair.server.set_block(pos, block);
    pair.tick(HERE);
    // Après l'envoi, le serveur et la réplique tiennent le même chunk.
    let chunk = IVec3::new(0, 1, 0);
    let a = pair.server.world().chunk_arc(chunk).unwrap();
    let b = pair.replica.world().chunk_arc(chunk).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
}

#[test]
fn quiet_changes_remesh_in_the_background_and_data_changes_do_not_remesh() {
    let mut pair = Pair::new();
    pair.pump(HERE);

    // Des données seules (le contenu d'un bloc actif) : le chunk est renvoyé, sans remaillage.
    let at = IVec3::new(5, 40, 5);
    assert!(pair.server.set_block(at, stone()));
    pair.tick(HERE);
    assert!(
        pair.server
            .set_block_entity(at, Some(crate::BlockEntity::default()))
    );
    let changes = pair.server.take_changes();
    let change = changes
        .iter()
        .find(|c| c.pos == IVec3::new(0, 1, 0))
        .unwrap();
    assert!(change.edits.is_empty() && !change.remesh);

    // Une machine pose un bloc au bord d'un chunk : tout part en tâche de fond, voisin compris.
    let border = IVec3::new(31, 41, 5);
    let block = toggled(&pair.server, border);
    assert!(pair.server.set_block_quiet(border, block));
    let changes = pair.server.take_changes();
    for chunk in [IVec3::new(0, 1, 0), IVec3::new(1, 1, 0)] {
        let change = changes.iter().find(|c| c.pos == chunk).expect("chunk noté");
        assert!(change.edits.is_empty() && change.remesh, "chunk {chunk}");
    }

    // Et la réplique, une fois à jour, a les maillages de ses voisins comme un maillage direct.
    let back = toggled(&pair.server, border);
    assert!(pair.server.set_block_quiet(border, back));
    let events = pair.pump(HERE);
    for (pos, mesh) in latest(&events) {
        assert_eq!(mesh, &mesh_binary(pair.replica.world(), pos), "chunk {pos}");
    }
    assert_eq!(pair.replica.world().block(border), back);
}
