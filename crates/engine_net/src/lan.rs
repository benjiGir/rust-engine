//! L'annonce des parties sur le réseau local (UDP).
//!
//! Un hôte envoie toutes les 1,5 s un petit datagramme « VOXEL1 », son port de jeu et le nom de la
//! partie, en diffusion (255.255.255.255) ou, sans réseau, sur la boucle locale (une fenêtre de la même machine).
//! L'écran « Rejoindre » écoute le port d'annonce et garde les parties entendues depuis moins de
//! 5 s.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Le port où les parties s'annoncent.
pub const ANNOUNCE_PORT: u16 = 25_599;
/// Le début de chaque annonce.
const MAGIC: &str = "VOXEL1";
/// L'intervalle entre deux annonces.
pub const ANNOUNCE_EVERY: Duration = Duration::from_millis(1500);
/// Une partie qu'on n'entend plus depuis ce temps disparaît de la liste.
const FORGET_AFTER: Duration = Duration::from_secs(5);

/// L'annonce d'un hôte.
pub struct Announcer {
    socket: UdpSocket,
    message: Vec<u8>,
    targets: Vec<SocketAddr>,
    last: Option<Instant>,
}

impl Announcer {
    /// Annonce la partie `name` qui écoute sur `game_port`, vers le port d'annonce `to`.
    pub fn new(game_port: u16, name: &str, to: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_broadcast(true)?;
        let name: String = name.chars().filter(|c| *c != '\n').take(64).collect();
        Ok(Self {
            socket,
            message: format!("{MAGIC}\n{game_port}\n{name}").into_bytes(),
            targets: vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), to),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), to),
            ],
            last: None,
        })
    }

    /// Envoie l'annonce si c'est le moment.
    pub fn tick(&mut self) {
        if self.last.is_some_and(|t| t.elapsed() < ANNOUNCE_EVERY) {
            return;
        }
        self.last = Some(Instant::now());
        // La diffusion revient aussi à la machine elle-même ; la boucle locale ne sert que si elle
        // échoue (pas de réseau), pour une fenêtre de la même machine.
        for target in &self.targets {
            if self.socket.send_to(&self.message, target).is_ok() {
                return;
            }
        }
    }
}

/// Une partie entendue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanGame {
    /// L'adresse où la rejoindre (celle de l'annonceur, avec le port de jeu annoncé).
    pub address: SocketAddr,
    pub name: String,
}

/// L'écoute des annonces.
pub struct Discovery {
    socket: UdpSocket,
    games: HashMap<SocketAddr, (LanGame, Instant)>,
}

impl Discovery {
    /// Écoute le port d'annonce `port`.
    pub fn new(port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            games: HashMap::new(),
        })
    }

    /// Les parties entendues récemment, par nom.
    pub fn poll(&mut self) -> Vec<LanGame> {
        let mut buffer = [0u8; 256];
        while let Ok((n, from)) = self.socket.recv_from(&mut buffer) {
            if let Some(game) = parse(&buffer[..n], from.ip()) {
                self.games.insert(game.address, (game, Instant::now()));
            }
        }
        self.games
            .retain(|_, (_, seen)| seen.elapsed() < FORGET_AFTER);
        let mut games: Vec<LanGame> = self.games.values().map(|(g, _)| g.clone()).collect();
        games.sort_by(|a, b| a.name.cmp(&b.name).then(a.address.cmp(&b.address)));
        games
    }
}

fn parse(bytes: &[u8], from: IpAddr) -> Option<LanGame> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.splitn(3, '\n');
    if lines.next()? != MAGIC {
        return None;
    }
    let port: u16 = lines.next()?.parse().ok()?;
    let name = lines.next().unwrap_or("").to_string();
    Some(LanGame {
        address: SocketAddr::new(from, port),
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_announced_game_is_heard_then_forgotten_when_silent() {
        let mut discovery = Discovery::new(0).unwrap();
        let port = discovery.socket.local_addr().unwrap().port();
        let mut announcer = Announcer::new(25_600, "La partie de Bob", port).unwrap();
        announcer.tick();
        let deadline = Instant::now() + Duration::from_secs(5);
        let games = loop {
            let games = discovery.poll();
            if !games.is_empty() {
                break games;
            }
            assert!(Instant::now() < deadline, "aucune annonce entendue");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(games[0].name, "La partie de Bob");
        assert_eq!(games[0].address.port(), 25_600);
        assert_eq!(
            games.len(),
            1,
            "diffusion et boucle locale font une seule partie"
        );
    }

    #[test]
    fn foreign_datagrams_are_ignored() {
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert_eq!(parse(b"bonjour", local), None);
        assert_eq!(parse(b"VOXEL1\nabc\nx", local), None);
        assert_eq!(
            parse(b"VOXEL1\n25601\nMonde", local)
                .unwrap()
                .address
                .port(),
            25_601
        );
    }
}
