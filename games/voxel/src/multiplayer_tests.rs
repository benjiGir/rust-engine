//! Les tests de bout en bout du multijoueur : un serveur sans fenêtre et des bots (de vrais clients
//! TCP, voir `bot.rs`) dans le même processus, sur 127.0.0.1.

use std::time::Duration;

use glam::{IVec3, Vec3};

use crate::bot::tests::{WAIT, bot, pump};
use crate::protocol::ClientMsg;
use crate::server_side::VoxelServer;

/// Donne des objets à un joueur, directement au serveur (la mise en place du test).
fn give(
    server: &mut VoxelServer,
    player: crate::players::PlayerId,
    key: &str,
    count: u32,
) -> usize {
    let item = server.content.items.id(key).expect("objet connu");
    let items = server.content.items.clone();
    let inventory = &mut server.players.get_mut(player).unwrap().inventory;
    let left = inventory.add(item, count, &items);
    assert_eq!(left, 0);
    inventory
        .slots
        .iter()
        .position(|s| s.is_some_and(|s| s.item == item))
        .unwrap()
}

#[test]
fn a_block_placed_by_one_player_is_seen_by_the_other() {
    let mut server = crate::server_boot::tests::headless();
    let port = server.open_lan(0).unwrap();
    let mut a = bot(&mut server, port, "Alice", 1);
    let mut b = bot(&mut server, port, "Bob", 2);
    // Les deux reçoivent le terrain sous leurs pieds.
    for player in [&mut a, &mut b] {
        player
            .wait_until(
                WAIT,
                || pump(&mut server),
                |p| p.ground_below(p.feet).is_some(),
            )
            .unwrap();
    }
    let ground = a.ground_below(a.feet).unwrap();
    a.move_to(Vec3::new(a.feet.x, ground, a.feet.z));
    // A tient de la terre et vise le dessus du sol, deux blocs devant lui.
    let slot = give(&mut server, a.player, "core:dirt", 4);
    a.send(&ClientMsg::SelectSlot(slot as u8));
    let ahead = a.feet + Vec3::X * 2.0;
    let top = a.ground_below(ahead).expect("le sol devant est arrivé");
    let target = IVec3::new(
        ahead.x.floor() as i32,
        top as i32 - 1,
        ahead.z.floor() as i32,
    );
    a.use_on(target);
    let placed = target + IVec3::Y;
    let dirt = server.registry.id("core:dirt").unwrap();
    // Le serveur pose le bloc…
    let deadline = std::time::Instant::now() + WAIT;
    while server.streamer().world().block(placed) != dirt {
        pump(&mut server);
        a.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "le bloc n'est pas posé en {placed}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // … et B le voit dans son monde.
    b.wait_until(WAIT, || pump(&mut server), |b| b.block(placed) == dirt)
        .unwrap();
    // A en a un de moins.
    a.wait_until(
        WAIT,
        || pump(&mut server),
        |a| a.inventory.slots[slot].is_some_and(|s| s.count == 3),
    )
    .unwrap();
}

#[test]
fn a_chat_line_reaches_everyone() {
    let mut server = crate::server_boot::tests::headless();
    let port = server.open_lan(0).unwrap();
    let a = bot(&mut server, port, "Alice", 1);
    let mut b = bot(&mut server, port, "Bob", 2);
    a.say("bonjour");
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| b.chat.iter().any(|l| l == "<Alice> bonjour"),
    )
    .unwrap();
}

#[test]
fn a_player_who_comes_back_finds_his_inventory() {
    let dir = std::env::temp_dir().join(format!("voxel-retour-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut server = crate::server_boot::tests::headless_in(Some(dir.clone()));
    let port = server.open_lan(0).unwrap();
    let b = bot(&mut server, port, "Bob", 2);
    let first = b.player;
    let slot = give(&mut server, first, "core:dirt", 7);
    b.leave();
    let deadline = std::time::Instant::now() + WAIT;
    while server.players.get(first).is_some() {
        pump(&mut server);
        assert!(
            std::time::Instant::now() < deadline,
            "le départ n'est pas vu"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Il revient : même identifiant, un autre numéro, le même inventaire.
    let mut b = bot(&mut server, port, "Bob", 2);
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| b.inventory.slots[slot].is_some_and(|s| s.count == 7),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
