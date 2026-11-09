//! Le temps passé par le GPU sur la passe principale (le décor), mesuré par le GPU
//! lui-même avec deux « timestamps » : un au début de la passe, un à la fin.
//!
//! Le CPU ne doit jamais attendre ce résultat : on le fait recopier dans un tampon lisible,
//! on demande sa lecture (`map_async`), et on le récupère quelques images plus tard, quand
//! le GPU a fini. Trois tampons tournent pour qu'une mesure soit possible à chaque image.
//!
//! Il faut la fonctionnalité `TIMESTAMP_QUERY` (Metal, Vulkan, DX12 l'ont) : sans elle,
//! pas de mesure, et rien d'autre ne change.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// Deux valeurs de 8 octets : début et fin.
const SIZE: u64 = 2 * wgpu::QUERY_SIZE as u64;

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;

struct Readback {
    buffer: wgpu::Buffer,
    /// Mis à `READY` par le rappel de `map_async`, depuis un autre thread.
    state: Arc<AtomicU8>,
}

pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readbacks: Vec<Readback>,
    /// Le tampon qui recevra la mesure de l'image en cours, s'il y en a un de libre.
    current: Option<usize>,
    /// Nanosecondes par unité de timestamp.
    period: f64,
    last: Option<Duration>,
}

impl GpuTimer {
    /// `None` si le GPU ne sait pas mesurer.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("gpu timer"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu timer resolve"),
            size: SIZE,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readbacks = (0..3)
            .map(|_| Readback {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu timer readback"),
                    size: SIZE,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(FREE)),
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            readbacks,
            current: None,
            period: f64::from(queue.get_timestamp_period()),
            last: None,
        })
    }

    /// La dernière mesure arrivée (elle date de quelques images).
    pub fn last(&self) -> Option<Duration> {
        self.last
    }

    /// Récupère les mesures que le GPU a terminées, sans attendre.
    pub fn collect(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for readback in &self.readbacks {
            if readback.state.load(Ordering::Acquire) != READY {
                continue;
            }
            if let Ok(data) = readback.buffer.slice(..).get_mapped_range() {
                let start = u64::from_le_bytes(data[0..8].try_into().unwrap_or_default());
                let end = u64::from_le_bytes(data[8..16].try_into().unwrap_or_default());
                // Un compteur qui repart à zéro, ou une passe non mesurée : on ignore.
                if end > start {
                    let nanos = (end - start) as f64 * self.period;
                    self.last = Some(Duration::from_nanos(nanos as u64));
                }
            }
            readback.buffer.unmap();
            readback.state.store(FREE, Ordering::Release);
        }
    }

    /// Avant la passe : choisit le tampon qui recevra la mesure de cette image. Sans
    /// tampon libre (le GPU est en retard), cette image n'est pas mesurée.
    pub fn begin_frame(&mut self) {
        self.current = self
            .readbacks
            .iter()
            .position(|r| r.state.load(Ordering::Acquire) == FREE);
    }

    /// Les écritures de timestamps à donner à la passe mesurée.
    pub fn pass_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.current?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    /// Après la passe, dans le même encodeur : recopier les deux valeurs.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(index) = self.current else { return };
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readbacks[index].buffer, 0, SIZE);
    }

    /// Après l'envoi des commandes : demander la lecture du tampon.
    pub fn after_submit(&mut self) {
        let Some(index) = self.current.take() else {
            return;
        };
        let readback = &self.readbacks[index];
        readback.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&readback.state);
        readback
            .buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                // En cas d'échec, le tampon redevient libre sans mesure.
                state.store(if result.is_ok() { READY } else { FREE }, Ordering::Release);
            });
    }
}
