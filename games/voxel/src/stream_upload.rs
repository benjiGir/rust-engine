//! Les envois au GPU des chunks et du terrain lointain, sous **un seul budget par image** (octets et
//! temps, voir `engine_voxel::TransferBudget`).
//!
//! Compter des maillages (16 par image avant) ne dit rien de leur coût : un chunk de plaine pèse
//! quelques kilo-octets, un chunk de forêt dense des centaines. Le budget s'arrête au premier des deux
//! plafonds, mais le premier envoi de l'image passe toujours : une très grosse opération n'est jamais
//! bloquée, on mesure ce qu'elle coûte (`envois : ...` dans les logs).
//!
//! Les maillages viennent de la réplique du client (voir `client_world.rs`), jamais du serveur.
//!
//! Ordre dans l'image : les maillages d'un bloc modifié (hors budget, le joueur attend son coup), les
//! maillages arrivés du plus proche au plus lointain, les libérations (étalées), puis ce qui reste va
//! au terrain lointain, lot par lot. Pour que le lointain ne meure pas de faim pendant un long vol,
//! un lot passe quand même au bout de [`FAR_PATIENCE`] images sans rien recevoir.
//!
//! Réglages : `VOXEL_UPLOAD_MB` (octets par image, 3 par défaut) et `VOXEL_UPLOAD_MS` (temps, 3 ms).

use std::time::{Duration, Instant};

use engine_voxel::{StreamEvent, TransferBudget, TransferStats};

use super::*;

/// Octets envoyés au plus par image (hors premier envoi, toujours accepté).
const DEFAULT_BYTES: usize = 3 << 20;
/// Temps d'envoi au plus par image, mesuré (les libérations comprises).
const DEFAULT_TIME: Duration = Duration::from_millis(3);
/// Libérations de maillages au plus par image : une téléportation en libère des milliers, étalés sur
/// plusieurs images (ces chunks sont loin ou hors de vue, les garder un peu ne se voit pas).
const UNLOAD_COUNT: usize = 256;
/// Images sans lot lointain avant d'en forcer un.
const FAR_PATIENCE: u32 = 6;

/// L'état des envois : le budget choisi, les compteurs, la patience du lointain.
pub struct Uploads {
    bytes: usize,
    time: Duration,
    /// Depuis le dernier bilan des logs.
    window: TransferStats,
    far_starved: u32,
}

impl Uploads {
    pub fn new() -> Self {
        let bytes = env_parse::<f64>("VOXEL_UPLOAD_MB")
            .map_or(DEFAULT_BYTES, |mb| (mb.max(0.0) * 1_048_576.0) as usize);
        let time = env_parse::<f64>("VOXEL_UPLOAD_MS").map_or(DEFAULT_TIME, |ms| {
            Duration::from_secs_f64(ms.max(0.0) / 1000.0)
        });
        Self {
            bytes,
            time,
            window: TransferStats::default(),
            far_starved: 0,
        }
    }

    fn budget(&self) -> TransferBudget {
        TransferBudget::new(self.bytes, self.time, UNLOAD_COUNT)
    }
}

impl VoxelGame {
    /// Applique côté GPU ce que les streamers ont prêt, dans la limite du budget de l'image.
    /// Renvoie le nombre de maillages envoyés (chunks et tuiles lointaines).
    pub(crate) fn apply_stream_events(&mut self) -> usize {
        let _span = engine_trace::span("chunks : envoi au GPU");
        let mut budget = self.uploads.budget();
        let mut largest = 0usize;
        loop {
            // Une instruction à part : l'emprunt du streamer ne doit pas durer pendant l'envoi.
            let next = self.view.replica_mut().next_event_budgeted(&mut budget);
            let Some(event) = next else { break };
            let started = Instant::now();
            match event {
                StreamEvent::Unload(pos) => {
                    self.culling.graph.remove(pos);
                    if let Some(id) = self.meshes.remove(&pos) {
                        self.renderer.remove_mesh(id);
                    }
                    budget.record_unload(started.elapsed());
                }
                StreamEvent::Mesh(pos, mesh) => {
                    let bytes = mesh.byte_size();
                    self.culling.graph.set(pos, mesh.visibility);
                    self.view.replica_mut().note_uploaded(pos);
                    // `insert` et `remove` rendent l'ancienne valeur : on libère l'ancien maillage,
                    // qui est resté affiché jusqu'à ce que le nouveau soit prêt.
                    let old = match upload(&mut self.renderer, &mesh) {
                        Some(id) => self.meshes.insert(pos, id),
                        None => self.meshes.remove(&pos),
                    };
                    if let Some(old) = old {
                        self.renderer.remove_mesh(old);
                    }
                    budget.record_upload(bytes, started.elapsed());
                    largest = largest.max(bytes);
                }
            }
        }
        let near_uploads = budget.uploads();
        let far_uploads = self.send_far_batches(&mut budget, &mut largest);
        self.uploads.window.record_frame(&budget, largest);
        near_uploads + far_uploads
    }

    /// Ce qui reste du budget va au terrain lointain, un lot de région à la fois.
    fn send_far_batches(&mut self, budget: &mut TransferBudget, largest: &mut usize) -> usize {
        let Some(far) = &mut self.far else { return 0 };
        let mut sent = 0;
        let mut any = false;
        while let Some(batch) = far.next_batch(budget) {
            any = true;
            sent += far.apply(&mut self.renderer, batch, budget, largest);
        }
        // Un long vol peut remplir le budget de chunks à chaque image : un lot de temps en temps
        // passe quand même (le premier envoi d'un budget neuf n'est jamais refusé).
        if any || !far.has_waiting() {
            self.uploads.far_starved = 0;
        } else {
            self.uploads.far_starved += 1;
            if self.uploads.far_starved >= FAR_PATIENCE {
                self.uploads.far_starved = 0;
                let mut forced = TransferBudget::new(0, Duration::ZERO, 0);
                if let Some(batch) = far.next_batch(&forced) {
                    sent += far.apply(&mut self.renderer, batch, &mut forced, largest);
                }
            }
        }
        sent
    }

    /// Le bilan des envois depuis le dernier appel, dans les logs.
    pub(crate) fn log_transfers(&mut self) {
        let stats = std::mem::take(&mut self.uploads.window);
        let waiting = self.stream_stats();
        log::info!(
            "envois : {} maillages, {:.2} Mio | pire image {:.2} ms, {:.2} Mio | plus gros {:.0} Kio | \
             {} images au-dessus du budget ({:.1} Mio / {} ms) | en attente : {} maillages ({:.2} Mio), {} libérations",
            stats.uploads,
            stats.bytes as f64 / 1_048_576.0,
            stats.worst_frame.as_secs_f64() * 1000.0,
            stats.worst_frame_bytes as f64 / 1_048_576.0,
            stats.worst_item_bytes as f64 / 1024.0,
            stats.over_budget,
            self.uploads.bytes as f64 / 1_048_576.0,
            self.uploads.time.as_millis(),
            waiting.meshes_waiting,
            waiting.bytes_waiting as f64 / 1_048_576.0,
            waiting.unloads_waiting,
        );
    }
}
