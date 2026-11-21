//! Temps d'image moyen et maximum, pour vérifier qu'il n'y a pas de saccade.

use std::time::{Duration, Instant};

const LOG_INTERVAL: Duration = Duration::from_secs(2);

pub struct FrameStats {
    window_start: Instant,
    last_frame: Instant,
    frames: u32,
    max_frame: Duration,
    /// Travail du streamer sur le thread principal (tri, réception, lancement des tâches).
    max_streaming: Duration,
    /// Création des tampons GPU.
    max_upload: Duration,
    /// `Renderer::render` (attente de l'image suivante comprise).
    max_render: Duration,
    /// Modification d'un bloc, remaillage immédiat compris.
    max_edit: Duration,
    uploads: usize,
    edits: usize,
}

pub struct FrameReport {
    pub avg_frame_ms: f32,
    pub max_frame_ms: f32,
    pub max_streaming_ms: f32,
    pub max_upload_ms: f32,
    pub max_render_ms: f32,
    pub max_edit_ms: f32,
    pub uploads: usize,
    pub edits: usize,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameStats {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            window_start: now,
            last_frame: now,
            frames: 0,
            max_frame: Duration::ZERO,
            max_streaming: Duration::ZERO,
            max_upload: Duration::ZERO,
            max_render: Duration::ZERO,
            max_edit: Duration::ZERO,
            uploads: 0,
            edits: 0,
        }
    }

    pub fn record_streaming(&mut self, streaming: Duration, upload: Duration, uploads: usize) {
        self.max_streaming = self.max_streaming.max(streaming);
        self.max_upload = self.max_upload.max(upload);
        self.uploads += uploads;
    }

    pub fn record_render(&mut self, time: Duration) {
        self.max_render = self.max_render.max(time);
    }

    pub fn record_edit(&mut self, time: Duration) {
        self.max_edit = self.max_edit.max(time);
        self.edits += 1;
    }

    /// À appeler une fois par image. Renvoie un bilan toutes les 2 secondes.
    pub fn tick(&mut self) -> Option<FrameReport> {
        let now = Instant::now();
        self.max_frame = self.max_frame.max(now - self.last_frame);
        self.last_frame = now;
        self.frames += 1;

        let elapsed = now - self.window_start;
        if elapsed < LOG_INTERVAL {
            return None;
        }
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        let report = FrameReport {
            avg_frame_ms: ms(elapsed) / self.frames as f32,
            max_frame_ms: ms(self.max_frame),
            max_streaming_ms: ms(self.max_streaming),
            max_upload_ms: ms(self.max_upload),
            max_render_ms: ms(self.max_render),
            max_edit_ms: ms(self.max_edit),
            uploads: self.uploads,
            edits: self.edits,
        };
        // On repart à zéro ; `..Self::new()` remplit les champs non cités.
        *self = Self {
            last_frame: now,
            ..Self::new()
        };
        Some(report)
    }
}
