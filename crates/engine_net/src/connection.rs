//! Une connexion TCP : un thread lit et recolle les trames, un autre écrit celles qu'on lui donne.
//!
//! Le jeu ne bloque jamais : [`Connection::send`] pose une trame dans une file bornée, et
//! [`Connection::try_recv`] prend ce qui est arrivé. [`Connection::queued_bytes`] dit ce qui attend
//! encore de partir : le serveur ralentit l'envoi des chunks quand le lien est lent, et coupe un pair
//! dont la file déborde.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use crate::frame::{Frame, FrameReader};

/// Combien de trames peuvent attendre l'écriture.
pub const QUEUE_FRAMES: usize = 65_536;
/// Au-delà, une écriture bloquée ferme la connexion.
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

/// Ce qui arrive d'un pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Une trame, décompressée.
    Frame(Vec<u8>),
    /// La connexion est fermée (la raison, pour le journal).
    Closed(String),
}

/// Une trame n'a pas pu partir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// La file est pleine : le pair ne lit pas assez vite.
    Full,
    Closed,
}

/// Une connexion à un pair.
pub struct Connection {
    peer: SocketAddr,
    stream: TcpStream,
    outgoing: SyncSender<Frame>,
    incoming: Receiver<Event>,
    queued: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
    /// Les octets arrivés sur le fil (trames compressées comprises).
    received: Arc<AtomicUsize>,
    /// Le retard simulé (voir [`Connection::simulate_lag`]) : les événements arrivés attendent ici
    /// leur heure, dans l'ordre.
    lag: Cell<(Duration, Duration)>,
    held: RefCell<VecDeque<(Instant, Event)>>,
    rng: Cell<u64>,
}

impl Connection {
    /// Se connecte à `address` (« hôte:port »).
    pub fn connect(address: &str, timeout: Duration) -> io::Result<Self> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "adresse introuvable");
        for addr in address.to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => return Self::from_stream(stream),
                Err(err) => last = err,
            }
        }
        Err(last)
    }

    /// Prend en main un flux déjà ouvert (une connexion acceptée).
    pub fn from_stream(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        // Un pair qui ne lit plus ne bloque pas le thread d'écriture pour toujours.
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let peer = stream.peer_addr()?;
        let reader = stream.try_clone()?;
        let writer = stream.try_clone()?;
        let (out_tx, out_rx) = mpsc::sync_channel::<Frame>(QUEUE_FRAMES);
        let (in_tx, in_rx) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let received = Arc::new(AtomicUsize::new(0));
        {
            let (in_tx, closed, received) = (in_tx.clone(), closed.clone(), received.clone());
            std::thread::Builder::new()
                .name(format!("net lecture {peer}"))
                .spawn(move || read_loop(reader, in_tx, closed, received))?;
        }
        {
            let (queued, closed) = (queued.clone(), closed.clone());
            std::thread::Builder::new()
                .name(format!("net écriture {peer}"))
                .spawn(move || write_loop(writer, out_rx, in_tx, queued, closed))?;
        }
        Ok(Self {
            peer,
            stream,
            outgoing: out_tx,
            incoming: in_rx,
            queued,
            closed,
            received,
            lag: Cell::new((Duration::ZERO, Duration::ZERO)),
            held: RefCell::new(VecDeque::new()),
            rng: Cell::new(0x9E37_79B9_7F4A_7C15 ^ u64::from(peer.port())),
        })
    }

    /// Simule un lien lent : chaque événement reçu attend `base` plus un tirage entre 0 et `jitter`
    /// avant d'être rendu, sans jamais doubler le précédent. Sur TCP, une perte se voit comme un retard :
    /// la gigue en tient lieu. Pour éprouver la prédiction et l'interpolation.
    pub fn simulate_lag(&self, base: Duration, jitter: Duration) {
        self.lag.set((base, jitter));
    }

    /// Le délai d'un événement qui arrive maintenant.
    fn delay(&self) -> Duration {
        let (base, jitter) = self.lag.get();
        if jitter.is_zero() {
            return base;
        }
        // xorshift : un tirage entre 0 et 1.
        let mut x = self.rng.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng.set(x);
        base + jitter.mul_f64((x >> 11) as f64 / (1u64 << 53) as f64)
    }

    /// Range ce qui est arrivé, avec son heure de remise.
    fn hold_arrivals(&self) {
        let now = Instant::now();
        let mut held = self.held.borrow_mut();
        while let Ok(event) = self.incoming.try_recv() {
            let at = now + self.delay();
            let at = held.back().map_or(at, |(last, _)| at.max(*last));
            held.push_back((at, event));
        }
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Pose une trame dans la file d'envoi.
    pub fn send(&self, frame: Frame) -> Result<(), SendError> {
        if self.is_closed() {
            return Err(SendError::Closed);
        }
        let len = frame.len();
        self.queued.fetch_add(len, Ordering::Relaxed);
        match self.outgoing.try_send(frame) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.queued.fetch_sub(len, Ordering::Relaxed);
                Err(match err {
                    TrySendError::Full(_) => SendError::Full,
                    TrySendError::Disconnected(_) => SendError::Closed,
                })
            }
        }
    }

    /// Ce qui est arrivé, sans attendre.
    pub fn try_recv(&self) -> Option<Event> {
        if self.lag.get() == (Duration::ZERO, Duration::ZERO) && self.held.borrow().is_empty() {
            return self.incoming.try_recv().ok();
        }
        self.hold_arrivals();
        let mut held = self.held.borrow_mut();
        if held.front().is_some_and(|(at, _)| *at <= Instant::now()) {
            held.pop_front().map(|(_, event)| event)
        } else {
            None
        }
    }

    /// Attend ce qui arrive, au plus `timeout` (pour la poignée de main et les tests).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        if self.lag.get() == (Duration::ZERO, Duration::ZERO) && self.held.borrow().is_empty() {
            return self.incoming.recv_timeout(timeout).ok();
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.try_recv() {
                return Some(event);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Les octets arrivés sur le fil depuis l'ouverture.
    pub fn received_bytes(&self) -> usize {
        self.received.load(Ordering::Relaxed)
    }

    /// Les octets qui attendent encore de partir.
    pub fn queued_bytes(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Ferme la connexion : les deux threads s'arrêtent.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

// Lâcher une connexion ne la coupe pas tout de suite : la file d'envoi se vide (un refus doit
// arriver), puis le thread d'écriture ferme le flux, ce qui arrête celui de lecture.

/// Signale la fermeture une seule fois, quel que soit le thread qui la voit le premier.
fn report_closed(events: &Sender<Event>, closed: &AtomicBool, reason: String) {
    if !closed.swap(true, Ordering::Relaxed) {
        let _ = events.send(Event::Closed(reason));
    }
}

fn read_loop(
    mut stream: TcpStream,
    events: Sender<Event>,
    closed: Arc<AtomicBool>,
    received: Arc<AtomicUsize>,
) {
    let mut reader = FrameReader::default();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(0) => {
                report_closed(&events, &closed, "fermée par le pair".to_string());
                return;
            }
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => {
                report_closed(&events, &closed, err.to_string());
                return;
            }
        };
        received.fetch_add(n, Ordering::Relaxed);
        reader.push(&buffer[..n]);
        loop {
            match reader.next_frame() {
                Ok(Some(frame)) => {
                    if events.send(Event::Frame(frame)).is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    report_closed(&events, &closed, err.to_string());
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
            }
        }
    }
}

fn write_loop(
    mut stream: TcpStream,
    frames: Receiver<Frame>,
    events: Sender<Event>,
    queued: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
) {
    // La file se vide jusqu'à ce que la connexion soit lâchée (l'émetteur disparaît).
    for frame in frames {
        if let Err(err) = stream.write_all(&frame) {
            report_closed(&events, &closed, err.to_string());
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
        queued.fetch_sub(frame.len(), Ordering::Relaxed);
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// L'écoute d'un hôte : elle accepte les connexions sans bloquer.
pub struct Listener {
    inner: TcpListener,
}

impl Listener {
    /// Écoute sur `port`, ou sur l'un des `tries - 1` suivants s'il est pris (0 : un port libre).
    pub fn bind(port: u16, tries: u16) -> io::Result<Self> {
        let mut last = None;
        for offset in 0..tries.max(1) {
            let candidate = if port == 0 {
                0
            } else {
                port.saturating_add(offset)
            };
            match TcpListener::bind(("0.0.0.0", candidate)) {
                Ok(inner) => {
                    inner.set_nonblocking(true)?;
                    return Ok(Self { inner });
                }
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("aucun port")))
    }

    pub fn port(&self) -> u16 {
        self.inner.local_addr().map_or(0, |a| a.port())
    }

    /// Une connexion en attente, s'il y en a une.
    pub fn accept(&self) -> io::Result<Option<Connection>> {
        match self.inner.accept() {
            Ok((stream, _)) => Connection::from_stream(stream).map(Some),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::encode;

    const WAIT: Duration = Duration::from_secs(5);

    fn pair() -> (Connection, Connection) {
        let listener = Listener::bind(0, 1).unwrap();
        let client = Connection::connect(&format!("127.0.0.1:{}", listener.port()), WAIT).unwrap();
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            if let Some(server) = listener.accept().unwrap() {
                return (server, client);
            }
            assert!(std::time::Instant::now() < deadline, "pas de connexion");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn frames_go_both_ways_in_order() {
        let (server, client) = pair();
        let big = vec![3u8; 200_000];
        client.send(encode(b"un")).unwrap();
        client.send(encode(&big)).unwrap();
        server.send(encode(b"deux")).unwrap();
        assert_eq!(
            server.recv_timeout(WAIT),
            Some(Event::Frame(b"un".to_vec()))
        );
        assert_eq!(server.recv_timeout(WAIT), Some(Event::Frame(big)));
        assert_eq!(
            client.recv_timeout(WAIT),
            Some(Event::Frame(b"deux".to_vec()))
        );
        // Tout est parti : plus rien n'attend.
        let deadline = std::time::Instant::now() + WAIT;
        while client.queued_bytes() > 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn one_frame_encoded_once_reaches_every_peer() {
        let (a_server, a_client) = pair();
        let (b_server, b_client) = pair();
        let frame = encode(&[9u8; 1000]);
        a_server.send(frame.clone()).unwrap();
        b_server.send(frame).unwrap();
        for client in [&a_client, &b_client] {
            assert_eq!(
                client.recv_timeout(WAIT),
                Some(Event::Frame(vec![9u8; 1000]))
            );
        }
    }

    #[test]
    fn closing_one_side_is_seen_by_the_other() {
        let (server, client) = pair();
        drop(client);
        match server.recv_timeout(WAIT) {
            Some(Event::Closed(_)) => {}
            other => panic!("attendu une fermeture, reçu {other:?}"),
        }
        assert!(server.is_closed());
        assert_eq!(server.send(encode(b"x")), Err(SendError::Closed));
    }

    #[test]
    fn a_simulated_lag_delays_frames_and_keeps_their_order() {
        let (server, client) = pair();
        client.simulate_lag(Duration::from_millis(80), Duration::from_millis(40));
        let sent = std::time::Instant::now();
        for i in 0..5u8 {
            server.send(encode(&[i])).unwrap();
        }
        let mut got = Vec::new();
        while got.len() < 5 {
            match client.recv_timeout(WAIT) {
                Some(Event::Frame(bytes)) => {
                    if got.is_empty() {
                        assert!(sent.elapsed() >= Duration::from_millis(80), "trop tôt");
                    }
                    got.push(bytes[0]);
                }
                other => panic!("attendu une trame, reçu {other:?}"),
            }
        }
        assert_eq!(got, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_taken_port_moves_to_the_next_one() {
        let first = Listener::bind(0, 1).unwrap();
        let port = first.port();
        // Les autres tests ouvrent des ports en même temps : une marge large.
        let second = Listener::bind(port, 50).unwrap();
        assert_ne!(second.port(), port);
    }
}
