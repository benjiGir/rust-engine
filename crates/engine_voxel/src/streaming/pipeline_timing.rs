//! Les délais du pipeline des chunks : demande → génération → lumière → maillage → envoi au GPU.
//!
//! Les spans de `engine_trace` mesurent la **durée d'un travail** sur un thread de fond. Ce module
//! mesure le **délai vu par un chunk** : le temps qui sépare deux étapes, files d'attente comprises.
//! C'est ce qui compte pour voir apparaître le décor, et ce que les optimisations du pipeline
//! doivent améliorer (voir `docs/plan-generation-rendu.md`).
//!
//! # Les étapes
//!
//! Pour chaque chunk, cinq instants, notés par le thread du streamer (celui qui appelle
//! [`ChunkStreamer::update`](crate::ChunkStreamer::update)) :
//!
//! 1. **demande** : le chunk entre dans la file de chargement ;
//! 2. **généré** : son résultat est reçu ;
//! 3. **maillage lancé** : sa lumière et celle de ses voisines sont prêtes, la tâche part ;
//! 4. **maillé** : le maillage est reçu ;
//! 5. **envoyé** : le jeu l'a transmis au GPU (voir `ChunkStreamer::note_uploaded`).
//!
//! D'où les étapes mesurées : [`Stage::Generation`] (1→2, file d'attente comprise),
//! [`Stage::Light`] (2→3, attente de la lumière des 9 colonnes), [`Stage::Meshing`] (3→4),
//! [`Stage::Upload`] (4→5, attente du budget d'envoi) et [`Stage::Total`] (1→5).
//!
//! Seule la **première** mise en place d'un chunk est mesurée, pas ses remaillages après une
//! modification. Éteint (par défaut), le suivi ne coûte qu'un test sur un `Option`.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use glam::IVec3;

/// Nombre de mesures gardées par étape : les plus récentes. Borne la mémoire et fait suivre
/// les centiles à l'activité du moment.
const WINDOW: usize = 4096;

/// Les étapes mesurées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Generation,
    Light,
    Meshing,
    Upload,
    Total,
}

impl Stage {
    const ALL: [Stage; 5] = [
        Stage::Generation,
        Stage::Light,
        Stage::Meshing,
        Stage::Upload,
        Stage::Total,
    ];

    /// Le nom affiché.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Generation => "génération",
            Stage::Light => "lumière",
            Stage::Meshing => "maillage",
            Stage::Upload => "envoi",
            Stage::Total => "total",
        }
    }
}

/// Les instants d'un chunk, au fil de son parcours.
#[derive(Debug, Clone, Copy)]
struct Times {
    requested: Instant,
    generated: Option<Instant>,
    mesh_started: Option<Instant>,
    meshed: Option<Instant>,
}

/// Les mesures d'une étape : une fenêtre glissante et un compteur total.
#[derive(Debug, Default)]
struct Samples {
    /// Les dernières durées, en microsecondes (tampon circulaire).
    recent: Vec<u32>,
    next: usize,
    count: u64,
}

impl Samples {
    fn push(&mut self, duration: Duration) {
        let micros = duration.as_micros().min(u32::MAX as u128) as u32;
        if self.recent.len() < WINDOW {
            self.recent.push(micros);
        } else {
            self.recent[self.next] = micros;
            self.next = (self.next + 1) % WINDOW;
        }
        self.count += 1;
    }

    fn report(&self) -> StageReport {
        let mut sorted = self.recent.clone();
        sorted.sort_unstable();
        let at = |q: f32| -> f32 {
            sorted
                .get(((sorted.len() as f32 * q) as usize).min(sorted.len().saturating_sub(1)))
                .map_or(0.0, |&us| us as f32 / 1000.0)
        };
        StageReport {
            count: self.count,
            p50_ms: at(0.5),
            p95_ms: at(0.95),
            max_ms: sorted.last().map_or(0.0, |&us| us as f32 / 1000.0),
        }
    }
}

/// Les chiffres d'une étape, en millisecondes. Les centiles portent sur les dernières mesures
/// (jusqu'à 4 096) ; `count` compte toutes les mesures depuis l'activation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StageReport {
    pub count: u64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub max_ms: f32,
}

/// Le bilan de toutes les étapes.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineReport {
    pub stages: Vec<(Stage, StageReport)>,
    /// Les chunks suivis qui n'ont pas encore été envoyés au GPU : en file, en cours, ou
    /// au-delà du rayon de vue (générés et éclairés, jamais maillés).
    pub in_progress: usize,
}

impl PipelineReport {
    /// Les chiffres d'une étape.
    pub fn stage(&self, stage: Stage) -> StageReport {
        self.stages
            .iter()
            .find(|(s, _)| *s == stage)
            .map(|(_, r)| *r)
            .unwrap_or_default()
    }
}

impl fmt::Display for PipelineReport {
    /// Une ligne : `génération p50 12.0 / p95 40.1 / max 80.0 ms | lumière ... | N chunks mesurés`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (stage, report) in &self.stages {
            write!(
                f,
                "{} p50 {:.1} / p95 {:.1} / max {:.1} ms | ",
                stage.name(),
                report.p50_ms,
                report.p95_ms,
                report.max_ms
            )?;
        }
        write!(
            f,
            "{} chunks mesurés, {} en cours",
            self.stage(Stage::Total).count,
            self.in_progress
        )
    }
}

/// Le suivi des chunks en cours de mise en place.
#[derive(Debug, Default)]
pub(crate) struct PipelineTimings {
    chunks: HashMap<IVec3, Times>,
    samples: [Samples; 5],
}

impl PipelineTimings {
    /// Le chunk est demandé. Une seconde demande ne remet pas l'horloge à zéro.
    pub fn requested(&mut self, pos: IVec3) {
        self.chunks.entry(pos).or_insert_with(|| Times {
            requested: Instant::now(),
            generated: None,
            mesh_started: None,
            meshed: None,
        });
    }

    pub fn generated(&mut self, pos: IVec3) {
        if let Some(t) = self.chunks.get_mut(&pos) {
            t.generated.get_or_insert_with(Instant::now);
        }
    }

    pub fn mesh_started(&mut self, pos: IVec3) {
        if let Some(t) = self.chunks.get_mut(&pos) {
            t.mesh_started.get_or_insert_with(Instant::now);
        }
    }

    pub fn meshed(&mut self, pos: IVec3) {
        if let Some(t) = self.chunks.get_mut(&pos) {
            t.meshed.get_or_insert_with(Instant::now);
        }
    }

    /// Le chunk est au GPU : on note les délais et on l'oublie (un remaillage ultérieur n'est
    /// pas mesuré).
    pub fn uploaded(&mut self, pos: IVec3) {
        let Some(t) = self.chunks.remove(&pos) else {
            return;
        };
        let now = Instant::now();
        let (Some(generated), Some(started), Some(meshed)) =
            (t.generated, t.mesh_started, t.meshed)
        else {
            return;
        };
        let gap = |a: Instant, b: Instant| b.saturating_duration_since(a);
        self.samples[0].push(gap(t.requested, generated));
        self.samples[1].push(gap(generated, started));
        self.samples[2].push(gap(started, meshed));
        self.samples[3].push(gap(meshed, now));
        self.samples[4].push(gap(t.requested, now));
    }

    /// Le chunk ne sera pas envoyé (vide, ou déchargé) : on l'oublie sans mesure.
    pub fn forget(&mut self, pos: IVec3) {
        self.chunks.remove(&pos);
    }

    /// Oublie les chunks que `keep` refuse (déchargés avant leur envoi).
    pub fn retain(&mut self, mut keep: impl FnMut(IVec3) -> bool) {
        self.chunks.retain(|&pos, _| keep(pos));
    }

    #[cfg(test)]
    fn in_progress(&self) -> usize {
        self.chunks.len()
    }

    pub fn report(&self) -> PipelineReport {
        PipelineReport {
            stages: Stage::ALL
                .iter()
                .zip(&self.samples)
                .map(|(&stage, samples)| (stage, samples.report()))
                .collect(),
            in_progress: self.chunks.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_walks_through_every_stage_and_is_then_forgotten() {
        let mut timings = PipelineTimings::default();
        let pos = IVec3::new(1, 2, 3);
        timings.requested(pos);
        timings.generated(pos);
        timings.mesh_started(pos);
        timings.meshed(pos);
        assert_eq!(timings.in_progress(), 1);
        timings.uploaded(pos);
        assert_eq!(timings.in_progress(), 0);
        let report = timings.report();
        for stage in Stage::ALL {
            assert_eq!(report.stage(stage).count, 1, "{}", stage.name());
        }
        assert!(report.stage(Stage::Total).max_ms >= report.stage(Stage::Meshing).max_ms);
        // Un second envoi (remaillage) n'est plus mesuré.
        timings.uploaded(pos);
        assert_eq!(timings.report().stage(Stage::Total).count, 1);
    }

    #[test]
    fn an_unfinished_chunk_adds_no_measure() {
        let mut timings = PipelineTimings::default();
        let pos = IVec3::ZERO;
        timings.requested(pos);
        timings.generated(pos);
        timings.uploaded(pos); // jamais maillé : pas de mesure, et oublié
        assert_eq!(timings.in_progress(), 0);
        assert_eq!(timings.report().stage(Stage::Total).count, 0);
    }

    #[test]
    fn the_window_is_bounded_and_percentiles_follow_the_recent_values() {
        let mut samples = Samples::default();
        for _ in 0..WINDOW {
            samples.push(Duration::from_millis(100));
        }
        for _ in 0..WINDOW {
            samples.push(Duration::from_millis(2));
        }
        assert_eq!(samples.recent.len(), WINDOW, "mémoire bornée");
        let report = samples.report();
        assert_eq!(report.count, 2 * WINDOW as u64);
        assert!((report.p50_ms - 2.0).abs() < 0.01, "{report:?}");
        assert!(
            (report.max_ms - 2.0).abs() < 0.01,
            "les vieilles valeurs sont sorties"
        );
    }

    #[test]
    fn retain_and_forget_drop_chunks_without_measuring() {
        let mut timings = PipelineTimings::default();
        for x in 0..4 {
            timings.requested(IVec3::new(x, 0, 0));
        }
        timings.forget(IVec3::new(0, 0, 0));
        timings.retain(|p| p.x >= 2);
        assert_eq!(timings.in_progress(), 2);
        assert_eq!(timings.report().stage(Stage::Total).count, 0);
    }
}
