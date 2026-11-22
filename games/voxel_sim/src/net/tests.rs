//! Les tests de bout en bout du multijoueur : un serveur sans fenêtre et des bots (de vrais clients
//! TCP, voir `net/bot.rs`) dans le même processus, sur 127.0.0.1.

use std::time::Duration;

use glam::{IVec3, Vec3};

use crate::net::bot::tests::{WAIT, bot, pump};
use crate::net::protocol::ClientMsg;
use crate::server::VoxelServer;

/// Donne des objets à un joueur, directement au serveur (la mise en place du test).
fn give(
    server: &mut VoxelServer,
    player: crate::player::players::PlayerId,
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
    let mut server = crate::server::boot::tests::headless();
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
    // Le sol deux blocs devant A : son chunk peut arriver un peu après celui de ses pieds.
    let ahead = a.feet + Vec3::X * 2.0;
    a.wait_until(
        WAIT,
        || pump(&mut server),
        |a| a.ground_below(ahead).is_some(),
    )
    .unwrap();
    // A tient de la terre et vise le dessus du sol, deux blocs devant lui.
    let slot = give(&mut server, a.player, "core:dirt", 4);
    a.send(&ClientMsg::SelectSlot(slot as u8));
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
    let mut server = crate::server::boot::tests::headless();
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
    let mut server = crate::server::boot::tests::headless_in(Some(dir.clone()));
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

#[test]
fn a_player_sees_where_the_other_faces_what_he_holds_and_his_gestures() {
    let mut server = crate::server::boot::tests::headless();
    let (mut a, mut b) = two_players(&mut server);
    let ahead = a.feet + Vec3::X * 2.0;
    a.wait_until(
        WAIT,
        || pump(&mut server),
        |a| a.ground_below(ahead).is_some(),
    )
    .unwrap();
    // A tient une pioche, se tourne, puis frappe le sol devant lui.
    let slot = give(&mut server, a.player, "core:iron_pickaxe", 1);
    a.send(&ClientMsg::SelectSlot(slot as u8));
    a.send(&ClientMsg::Look {
        yaw: 1.0,
        pitch: -0.3,
    });
    a.use_on(surface_ahead(&a, 2.0));
    let pickaxe = server.content.items.id("core:iron_pickaxe").unwrap();
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| {
            b.players.values().any(|(pose, look)| {
                (pose.yaw - 1.0).abs() < 1e-3
                    && look.held.is_some_and(|s| s.item == pickaxe)
                    && look.swings > 0
            })
        },
    )
    .unwrap();
}

/// Deux bots, A et B, côte à côte, avec le terrain sous leurs pieds ; A au sol.
fn two_players(server: &mut VoxelServer) -> (crate::net::bot::Bot, crate::net::bot::Bot) {
    let port = server.open_lan(0).unwrap();
    let mut a = bot(server, port, "Alice", 1);
    let mut b = bot(server, port, "Bob", 2);
    for player in [&mut a, &mut b] {
        // Le sol sous ses pieds, et celui des deux blocs devant lui (voir `surface_ahead`) : un
        // chunk voisin peut arriver un peu plus tard.
        player
            .wait_until(
                WAIT,
                || pump(server),
                |p| (0..=2).all(|dx| p.ground_below(p.feet + Vec3::X * dx as f32).is_some()),
            )
            .unwrap();
        let ground = player.ground_below(player.feet).unwrap();
        player.move_to(Vec3::new(player.feet.x, ground, player.feet.z));
    }
    (a, b)
}

/// Le bloc de surface (le haut du sol) à `dx` blocs devant `bot`, côté x.
fn surface_ahead(bot: &crate::net::bot::Bot, dx: f32) -> IVec3 {
    let at = bot.feet + Vec3::X * dx;
    let top = bot.ground_below(at).expect("le sol devant est arrivé");
    IVec3::new(at.x.floor() as i32, top as i32 - 1, at.z.floor() as i32)
}

#[test]
fn a_door_opened_by_one_player_opens_for_the_other() {
    let mut server = crate::server::boot::tests::headless();
    let (mut a, mut b) = two_players(&mut server);
    let base = surface_ahead(&a, 2.0) + IVec3::Y;
    let door = server.registry.id("core:wooden_door").unwrap();
    let parts = server
        .paired_placement(base, door, Vec3::NEG_X)
        .expect("une porte se pose là");
    server.place_pair(parts).expect("la porte est posée");
    b.wait_until(WAIT, || pump(&mut server), |b| b.block(base) == parts[0].1)
        .unwrap();
    let closed = parts[0].1;
    a.wait_until(WAIT, || pump(&mut server), |a| a.block(base) == closed)
        .unwrap();
    a.use_on(base);
    // L'autre forme de la porte arrive chez les deux.
    for player in [&mut a, &mut b] {
        player
            .wait_until(WAIT, || pump(&mut server), |p| p.block(base) != closed)
            .unwrap();
    }
}

#[test]
fn a_chest_filled_by_one_player_changes_for_the_other() {
    use crate::net::protocol::{ClickButton, SlotTarget};
    use crate::player::inventory::Slot;
    let mut server = crate::server::boot::tests::headless();
    let (mut a, mut b) = two_players(&mut server);
    let chest_at = surface_ahead(&a, 2.0) + IVec3::Y;
    let chest = server.registry.id("core:chest").unwrap();
    assert!(server.edit(chest_at, chest), "le coffre est posé");
    for player in [&mut a, &mut b] {
        player
            .wait_until(WAIT, || pump(&mut server), |p| p.block(chest_at) == chest)
            .unwrap();
    }
    // Les deux l'ouvrent.
    b.move_to(a.feet);
    for player in [&mut a, &mut b] {
        player.use_on(chest_at);
        player
            .wait_until(WAIT, || pump(&mut server), |p| p.container.is_some())
            .unwrap();
    }
    // B prend sa terre et la pose dans la première case du coffre.
    let slot = give(&mut server, b.player, "core:dirt", 5);
    for target in [SlotTarget::Player(Slot::Main(slot)), SlotTarget::Open(0)] {
        b.send(&ClientMsg::SlotClick {
            slot: target,
            button: ClickButton::Primary,
            shift: false,
        });
    }
    // A voit le contenu changer.
    a.wait_until(
        WAIT,
        || pump(&mut server),
        |a| {
            a.container
                .as_ref()
                .is_some_and(|c| c.raw.first().is_some_and(|s| s.is_some()))
        },
    )
    .unwrap();
}

#[test]
fn a_creature_chases_the_nearest_player() {
    let mut server = crate::server::boot::tests::headless();
    // La nuit : les créatures hostiles ne se dissipent pas.
    server.day.time = 0.0;
    let (a, mut b) = two_players(&mut server);
    let mobs = server
        .ecs
        .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
        .clone();
    let kind = (0..mobs.len() as u32)
        .find(|&k| mobs.get(k).is_some_and(|d| d.hostile))
        .expect("une créature hostile");
    // B s'éloigne de A ; la créature apparaît près de B, hors de portée de A.
    let far = b.feet + Vec3::X * 24.0;
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| b.ground_below(far).is_some(),
    )
    .unwrap();
    let ground = b.ground_below(far).unwrap();
    b.move_to(Vec3::new(far.x, ground, far.z));
    let spot = b.feet + Vec3::X * 7.0;
    let spot = Vec3::new(spot.x, b.ground_below(spot).unwrap(), spot.z);
    let mob = crate::mobs::spawn_mob(&mut server.ecs, kind, spot).expect("la créature apparaît");
    let start = spot.distance(b.feet);
    for _ in 0..300 {
        pump(&mut server);
        b.poll().unwrap();
    }
    let at = crate::player::body(&server.ecs, mob).position;
    assert!(server.ecs.is_alive(mob), "la créature est encore là");
    assert!(
        at.distance(b.feet) < start - 1.0,
        "elle s'est approchée de B ({at} → {})",
        b.feet
    );
    assert!(
        at.distance(b.feet) < at.distance(a.feet),
        "elle va vers le plus proche"
    );
}

#[test]
fn a_guest_operator_runs_commands_on_the_server() {
    let mut server = crate::server::boot::tests::headless();
    let port = server.open_lan(0).unwrap();
    let mut b = bot(&mut server, port, "Bob", 2);
    b.send(&ClientMsg::Command("give dirt 2".into()));
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| {
            b.chat
                .iter()
                .any(|l| l == "Commande réservée aux opérateurs")
        },
    )
    .unwrap();
    server.operators.push("Bob".into());
    b.send(&ClientMsg::Command("/give dirt 2".into()));
    let dirt = server.content.items.id("core:dirt").unwrap();
    b.wait_until(
        WAIT,
        || pump(&mut server),
        |b| {
            b.inventory
                .slots
                .iter()
                .flatten()
                .any(|s| s.item == dirt && s.count == 2)
        },
    )
    .unwrap();
    assert!(b.chat.iter().any(|l| l == "core:dirt donné"));
}
