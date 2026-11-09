//! `engine_trace` : chronométrer le moteur, thread par thread.
//!
//! On entoure une étape d'un « span » : `let _span = engine_trace::span("maillage");`.
//! Quand la variable sort de portée, sa durée est notée dans le tampon de **ce** thread
//! (le thread principal, un thread de chunks, celui de la sauvegarde...). Deux usages :
//!
//! - **Trace complète** ([`start_recording`] puis [`write_chrome_trace`]) : tous les spans
//!   de tous les threads, écrits au format « Chrome trace ». On ouvre le fichier dans
//!   <https://ui.perfetto.dev> : une ligne par thread, une barre par étape, imbriquées.
//!   On y voit tout de suite qui attend qui, et si les threads de fond travaillent.
//! - **Images lentes** ([`set_spike_threshold`]) : le thread de la boucle appelle
//!   [`begin_frame`] et [`end_frame`] ; une image plus longue que le seuil renvoie le détail
//!   de ses étapes, à afficher dans les logs.
//!
//! Éteint, un span ne coûte qu'une lecture atomique : on peut en mettre partout.
//! Allumé, environ 50 ns (deux lectures de l'horloge et un verrou que personne d'autre
//! ne prend, sauf à l'écriture du fichier).
//!
//! Idée reprise d'Horizon (`horizon-trace`, licence MIT), réécrite ici.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Au-delà, un thread arrête de noter (environ 50 Mo par thread) : une trace laissée
/// allumée des heures ne doit pas remplir la mémoire.
const MAX_EVENTS_PER_THREAD: usize = 1_000_000;

/// La trace complète est allumée (tous les threads notent).
static RECORDING: AtomicBool = AtomicBool::new(false);
/// Seuil des images lentes en microsecondes ; 0 = éteint.
static SPIKE_US: AtomicU64 = AtomicU64::new(0);
/// Numéro du prochain thread qui note quelque chose.
static NEXT_TID: AtomicU32 = AtomicU32::new(1);
/// Les tampons de tous les threads, pour les rassembler à l'écriture. Un tampon survit à
/// son thread : la trace garde le travail d'un thread de sauvegarde déjà terminé.
static THREADS: Mutex<Vec<Arc<ThreadBuffer>>> = Mutex::new(Vec::new());

/// L'instant zéro de la trace : les temps sont notés à partir de lui.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Une étape terminée.
struct Event {
    name: Cow<'static, str>,
    /// Début, depuis [`epoch`].
    start: Duration,
    duration: Duration,
    /// Nombre de spans ouverts autour de celui-ci sur le même thread.
    depth: u16,
}

/// Les étapes d'un thread. Seul ce thread y écrit ; le verrou ne sert qu'au moment où
/// [`write_chrome_trace`] les lit depuis un autre thread.
struct ThreadBuffer {
    tid: u32,
    name: String,
    events: Mutex<Vec<Event>>,
    /// Étapes perdues parce que le tampon était plein.
    dropped: AtomicU64,
}

/// L'image en cours sur le thread de la boucle.
struct Frame {
    start: Instant,
    /// Indice, dans le tampon du thread, de la première étape de l'image.
    first_event: usize,
}

thread_local! {
    /// Le tampon de ce thread, créé à sa première étape notée.
    static LOCAL: RefCell<Option<Arc<ThreadBuffer>>> = const { RefCell::new(None) };
    /// Profondeur des spans ouverts.
    static DEPTH: Cell<u16> = const { Cell::new(0) };
    /// `Some` sur le thread de la boucle, entre [`begin_frame`] et [`end_frame`].
    static FRAME: Cell<Option<Frame>> = const { Cell::new(None) };
}

/// Allume la trace complète : à partir de maintenant, tous les threads notent leurs étapes.
pub fn start_recording() {
    epoch();
    RECORDING.store(true, Ordering::Relaxed);
}

/// Vrai si la trace complète est allumée.
pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Le seuil des images lentes (`None` : éteint). Seul le thread de la boucle note alors
/// ses étapes, et les oublie à la fin de chaque image si la trace complète est éteinte.
pub fn set_spike_threshold(threshold: Option<Duration>) {
    epoch();
    let micros = threshold.map_or(0, |t| (t.as_micros() as u64).max(1));
    SPIKE_US.store(micros, Ordering::Relaxed);
}

/// Ce thread doit-il noter ? La trace complète, ou une image en cours avec les images
/// lentes allumées.
fn active() -> bool {
    if RECORDING.load(Ordering::Relaxed) {
        return true;
    }
    SPIKE_US.load(Ordering::Relaxed) != 0 && FRAME.with(|f| with_cell(f, |f| f.is_some()))
}

/// Lit une `Cell<Option<Frame>>` sans la copier (`Frame` n'est pas `Copy`).
fn with_cell<R>(cell: &Cell<Option<Frame>>, read: impl FnOnce(&Option<Frame>) -> R) -> R {
    let value = cell.take();
    let result = read(&value);
    cell.set(value);
    result
}

/// Le tampon de ce thread, créé et enregistré au premier appel.
fn local_buffer() -> Arc<ThreadBuffer> {
    LOCAL.with(|local| {
        local
            .borrow_mut()
            .get_or_insert_with(|| {
                let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);
                let name = std::thread::current()
                    .name()
                    .map_or_else(|| format!("thread-{tid}"), str::to_string);
                let buffer = Arc::new(ThreadBuffer {
                    tid,
                    name,
                    events: Mutex::new(Vec::new()),
                    dropped: AtomicU64::new(0),
                });
                lock(&THREADS).push(Arc::clone(&buffer));
                buffer
            })
            .clone()
    })
}

/// Un verrou, même si un thread a paniqué en le tenant : une trace n'est pas une raison
/// de faire planter le jeu.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Une étape en cours. Sa durée est notée quand elle est détruite (fin de portée).
#[must_use = "le span mesure jusqu'à sa destruction : nommez-le `_span`, pas `_`"]
pub struct Span {
    /// `None` : rien à noter (trace éteinte au début du span).
    open: Option<(Cow<'static, str>, Instant, u16)>,
}

/// Commence une étape au nom fixe. Garder le résultat dans une variable :
/// `let _span = engine_trace::span("lumière");`.
#[inline]
pub fn span(name: &'static str) -> Span {
    if !active() {
        return Span { open: None };
    }
    open(Cow::Borrowed(name))
}

/// Comme [`span`], pour un nom connu seulement à l'exécution (un système ECS, un mod).
/// Le nom n'est copié que si la trace est allumée.
#[inline]
pub fn span_dyn(name: &str) -> Span {
    if !active() {
        return Span { open: None };
    }
    open(Cow::Owned(name.to_string()))
}

fn open(name: Cow<'static, str>) -> Span {
    let depth = DEPTH.with(|d| {
        let depth = d.get();
        d.set(depth.saturating_add(1));
        depth
    });
    Span {
        open: Some((name, Instant::now(), depth)),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some((name, start, depth)) = self.open.take() else {
            return;
        };
        let duration = start.elapsed();
        DEPTH.with(|d| d.set(depth));
        let buffer = local_buffer();
        let mut events = lock(&buffer.events);
        if events.len() >= MAX_EVENTS_PER_THREAD {
            buffer.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        events.push(Event {
            name,
            start: start.saturating_duration_since(epoch()),
            duration,
            depth,
        });
    }
}

/// Début d'une image, sur le thread de la boucle de jeu.
pub fn begin_frame() {
    if !RECORDING.load(Ordering::Relaxed) && SPIKE_US.load(Ordering::Relaxed) == 0 {
        return;
    }
    let first_event = lock(&local_buffer().events).len();
    FRAME.with(|f| {
        f.set(Some(Frame {
            start: Instant::now(),
            first_event,
        }));
    });
}

/// Fin d'une image. Si elle a dépassé le seuil des images lentes, renvoie le détail de
/// ses étapes (une par ligne, indentées selon leur imbrication), à afficher.
pub fn end_frame() -> Option<String> {
    let frame = FRAME.with(Cell::take)?;
    let threshold = SPIKE_US.load(Ordering::Relaxed);
    let elapsed = frame.start.elapsed();
    let buffer = local_buffer();
    let mut events = lock(&buffer.events);
    let first = frame.first_event.min(events.len());
    let report = (threshold != 0 && elapsed.as_micros() as u64 >= threshold)
        .then(|| spike_report(elapsed, &events[first..]));
    // Sans trace complète, les étapes de l'image ne servaient qu'à ce rapport.
    if !RECORDING.load(Ordering::Relaxed) {
        events.truncate(first);
    }
    report
}

/// Les étapes d'une image lente, de la plus haute à la plus imbriquée, dans l'ordre.
fn spike_report(elapsed: Duration, events: &[Event]) -> String {
    /// Les étapes plus courtes ne font que du bruit.
    const MIN: Duration = Duration::from_micros(100);
    /// Une image peut contenir des milliers d'étapes : on n'affiche que les premières.
    const MAX_LINES: usize = 40;

    // Un span est noté à sa fin, donc après ceux qu'il contient : on trie par début.
    let mut order: Vec<&Event> = events.iter().filter(|e| e.duration >= MIN).collect();
    order.sort_by_key(|e| (e.start, e.depth));
    let mut text = format!("image lente : {:.1} ms", ms(elapsed));
    for event in order.iter().take(MAX_LINES) {
        let indent = "  ".repeat(event.depth as usize + 1);
        let _ = write!(
            text,
            "\n{indent}{} {:.2} ms",
            event.name,
            ms(event.duration)
        );
    }
    if order.len() > MAX_LINES {
        let _ = write!(text, "\n  ... et {} autres étapes", order.len() - MAX_LINES);
    }
    text
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Ce que [`write_chrome_trace`] a écrit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceSummary {
    pub threads: usize,
    pub events: usize,
    /// Étapes perdues, tampons pleins.
    pub dropped: u64,
}

/// Écrit toutes les étapes notées dans `path`, au format « Chrome trace » (JSON),
/// à ouvrir dans <https://ui.perfetto.dev> ou `chrome://tracing`.
pub fn write_chrome_trace(path: &Path) -> io::Result<TraceSummary> {
    let file = std::fs::File::create(path)?;
    let mut out = io::BufWriter::new(file);
    let summary = write_events(&mut out)?;
    out.flush()?;
    Ok(summary)
}

/// Le format : `{"traceEvents": [...]}`. Une étape est un événement « complet »
/// (`"ph":"X"`, début et durée en microsecondes) ; le nom de chaque thread est un
/// événement de métadonnées (`"ph":"M"`).
fn write_events(out: &mut impl io::Write) -> io::Result<TraceSummary> {
    let threads: Vec<Arc<ThreadBuffer>> = lock(&THREADS).clone();
    let mut summary = TraceSummary {
        threads: threads.len(),
        events: 0,
        dropped: 0,
    };
    out.write_all(b"{\"displayTimeUnit\":\"ms\",\"traceEvents\":[\n")?;
    let mut first = true;
    for thread in &threads {
        let separator = if first { "" } else { ",\n" };
        first = false;
        write!(
            out,
            "{separator}{{\"name\":\"thread_name\",\"ph\":\"M\",\"pid\":1,\"tid\":{},\"args\":{{\"name\":\"{}\"}}}}",
            thread.tid,
            escape(&thread.name)
        )?;
        let events = lock(&thread.events);
        for event in events.iter() {
            write!(
                out,
                ",\n{{\"name\":\"{}\",\"ph\":\"X\",\"pid\":1,\"tid\":{},\"ts\":{:.3},\"dur\":{:.3}}}",
                escape(&event.name),
                thread.tid,
                event.start.as_secs_f64() * 1e6,
                event.duration.as_secs_f64() * 1e6,
            )?;
        }
        summary.events += events.len();
        summary.dropped += thread.dropped.load(Ordering::Relaxed);
    }
    out.write_all(b"\n]}\n")?;
    Ok(summary)
}

/// Un texte pour une chaîne JSON : guillemets, barres obliques inverses et caractères
/// de contrôle échappés.
fn escape(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            }
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les étapes notées par le thread qui appelle.
    fn my_events() -> Vec<(String, u16)> {
        lock(&local_buffer().events)
            .iter()
            .map(|e| (e.name.to_string(), e.depth))
            .collect()
    }

    #[test]
    fn spans_are_recorded_with_their_nesting() {
        start_recording();
        // Un thread neuf : son tampon ne contient que ce test.
        std::thread::spawn(|| {
            {
                let _outer = span("dehors");
                let _inner = span_dyn("dedans");
            }
            // Un span est noté à sa fin : l'intérieur d'abord.
            assert_eq!(
                my_events(),
                [("dedans".to_string(), 1), ("dehors".to_string(), 0)]
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn trace_is_json_with_thread_names() {
        start_recording();
        std::thread::Builder::new()
            .name("ouvrier \"test\"".into())
            .spawn(|| drop(span("tâche")))
            .unwrap()
            .join()
            .unwrap();
        let mut out = Vec::new();
        let summary = write_events(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("{\"displayTimeUnit\""));
        assert!(text.trim_end().ends_with("]}"));
        assert!(
            text.contains(r#""args":{"name":"ouvrier \"test\""}"#),
            "{text}"
        );
        assert!(text.contains(r#""name":"tâche","ph":"X""#), "{text}");
        assert!(summary.events >= 1);
    }

    #[test]
    fn slow_frames_report_their_steps() {
        let events = [
            Event {
                name: Cow::Borrowed("rendu"),
                start: Duration::from_millis(10),
                duration: Duration::from_millis(5),
                depth: 1,
            },
            Event {
                name: Cow::Borrowed("image"),
                start: Duration::from_millis(1),
                duration: Duration::from_millis(30),
                depth: 0,
            },
            Event {
                name: Cow::Borrowed("minuscule"),
                start: Duration::from_millis(2),
                duration: Duration::from_micros(3),
                depth: 1,
            },
        ];
        let report = spike_report(Duration::from_millis(31), &events);
        assert_eq!(
            report,
            "image lente : 31.0 ms\n  image 30.00 ms\n    rendu 5.00 ms"
        );
    }

    #[test]
    fn frames_are_reported_only_when_slow() {
        // Un seul test change le seuil (global) : pas de course entre tests.
        std::thread::spawn(|| {
            set_spike_threshold(Some(Duration::from_secs(60)));
            begin_frame();
            drop(span("vite"));
            assert_eq!(end_frame(), None);
            assert_eq!(end_frame(), None, "pas d'image en cours");

            set_spike_threshold(Some(Duration::from_micros(1)));
            for _ in 0..2 {
                begin_frame();
                {
                    let _frame = span("image");
                    let _render = span("render");
                    std::thread::sleep(Duration::from_millis(1));
                }
                let report = end_frame().expect("image lente");
                let lines: Vec<&str> = report.lines().skip(1).collect();
                assert!(lines[0].starts_with("  image "), "{report}");
                assert!(lines[1].starts_with("    render "), "{report}");
            }
            set_spike_threshold(None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn escape_handles_quotes_and_control_characters() {
        assert_eq!(escape("simple"), "simple");
        assert_eq!(escape("a\"b\\c\n"), "a\\\"b\\\\c\\u000a");
    }
}
