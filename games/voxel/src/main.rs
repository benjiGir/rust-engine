//! Le futur mini-Minecraft.
//!
//! Étape actuelle : un monde infini, généré et maillé sur des threads de fond
//! autour de la caméra.
//! - Clic gauche : capturer la souris. Échap : la libérer (deuxième Échap : quitter).
//! - ZQSD (AZERTY) ou WASD (QWERTY) : se déplacer. Espace / Maj : monter / descendre.
//! - Ctrl gauche : aller plus vite.
//! - F1 : basculer entre maillage « culling » et « greedy ».
//! - F2 : activer / désactiver le frustum culling.
//!
//! Variables d'environnement :
//! - `VOXEL_SEED=123` : la graine du monde (42 par défaut).
//! - `VOXEL_RADIUS=24` : le rayon de vue en chunks (16 par défaut).
//! - `VOXEL_AUTOFLY=1` : la caméra avance seule, à la vitesse du sprint, pour mesurer
//!   le chargement sans toucher au clavier. `VOXEL_AUTOFLY=100` : 100 blocs par seconde.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use engine_core::winit::dpi::PhysicalSize;
use engine_core::winit::event::MouseButton;
use engine_core::winit::keyboard::KeyCode;
use engine_core::{AppConfig, Context, Game};
use engine_render::{Aabb, Camera, ColorVertex, Fog, MeshId, RenderStats, Renderer};
use engine_voxel::{
    CHUNK_SIZE, ChunkMesh, ChunkStreamer, MeshVertex, StreamEvent, StreamingConfig,
    TerrainGenerator,
};
use glam::{IVec3, Vec2, Vec3};

const MOUSE_SENSITIVITY: f32 = 0.0025;
const MOVE_SPEED: f32 = 12.0;
const SPRINT_FACTOR: f32 = 3.0;
const DEFAULT_SEED: u32 = 42;
/// Maillages envoyés au GPU au maximum par image. Chaque envoi crée deux tampons :
/// les étaler sur plusieurs images évite une saccade quand 50 chunks arrivent d'un coup.
const UPLOAD_BUDGET: usize = 16;
/// Hauteur de vol au départ et en mode `VOXEL_AUTOFLY`, au-dessus du sol.
const FLY_ALTITUDE: f32 = 30.0;
const STATS_LOG_INTERVAL: Duration = Duration::from_secs(2);

struct VoxelGame {
    renderer: Renderer,
    camera: Camera,
    mouse_captured: bool,
    streamer: ChunkStreamer,
    /// Le maillage GPU de chaque chunk affiché.
    meshes: HashMap<IVec3, MeshId>,
    /// `Some(vitesse)` en mode vol automatique.
    autofly: Option<f32>,
    frame_stats: FrameStats,
    last_render: RenderStats,
}

impl Game for VoxelGame {
    fn new(ctx: &mut Context) -> Result<Self> {
        // `?` remonte l'erreur au moteur, qui l'affichera à la fermeture.
        let mut renderer = Renderer::new(ctx.window.clone())?;

        let seed = env_parse("VOXEL_SEED").unwrap_or(DEFAULT_SEED);
        let config = StreamingConfig {
            view_radius: env_parse("VOXEL_RADIUS").unwrap_or(16),
            ..StreamingConfig::default()
        };
        // Le brouillard termine juste avant le bord du rayon de vue.
        let view_distance = (config.view_radius * CHUNK_SIZE as i32) as f32;
        renderer.fog = Some(Fog {
            start: view_distance * 0.6,
            end: view_distance * 0.95,
        });
        log::info!(
            "graine {seed}, rayon de vue {} chunks, {} threads de fond",
            config.view_radius,
            config.worker_threads
        );
        let streamer = ChunkStreamer::new(TerrainGenerator::new(seed), config);

        // On démarre au-dessus du sol, quelle que soit la graine.
        let ground = streamer.generator().column(0, 0).height as f32;
        let camera = Camera {
            position: Vec3::new(0.5, ground + FLY_ALTITUDE, 0.5),
            yaw: 45f32.to_radians(),
            pitch: -20f32.to_radians(),
            far: view_distance * 1.5,
            ..Camera::default()
        };

        // `std::env::var` renvoie `Err` si la variable n'existe pas : `.ok()` en fait un `Option`.
        let autofly = std::env::var("VOXEL_AUTOFLY")
            .ok()
            .map(|v| v.parse().unwrap_or(MOVE_SPEED * SPRINT_FACTOR))
            // `VOXEL_AUTOFLY=1` veut dire « oui », pas « 1 bloc par seconde ».
            .map(|speed: f32| {
                if speed <= 1.0 {
                    MOVE_SPEED * SPRINT_FACTOR
                } else {
                    speed
                }
            });
        if let Some(speed) = autofly {
            log::info!("vol automatique à {speed} blocs/s");
        }

        Ok(Self {
            renderer,
            camera,
            mouse_captured: false,
            streamer,
            meshes: HashMap::new(),
            autofly,
            frame_stats: FrameStats::new(),
            last_render: RenderStats::default(),
        })
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        self.handle_mouse_capture(ctx);
        if self.mouse_captured {
            self.look(ctx.input.mouse_delta());
        }
        match self.autofly {
            Some(speed) => self.autofly(speed, dt),
            None => self.fly(ctx, dt),
        }

        if ctx.input.key_pressed(KeyCode::F1) {
            let mesher = self.streamer.config().mesher.other();
            log::info!("maillage {} : on remaille tout", mesher.name());
            self.streamer.set_mesher(mesher);
        }
        if ctx.input.key_pressed(KeyCode::F2) {
            self.renderer.frustum_culling = !self.renderer.frustum_culling;
            let state = if self.renderer.frustum_culling {
                "activé"
            } else {
                "désactivé"
            };
            log::info!("frustum culling {state}");
        }

        // Tout le travail du monde côté thread principal : on le chronomètre.
        let start = Instant::now();
        self.streamer.update(self.camera.position);
        let streamed = Instant::now();
        let uploads = self.apply_stream_events();
        self.frame_stats
            .record_streaming(streamed - start, streamed.elapsed(), uploads);
    }

    fn render(&mut self, ctx: &mut Context, _alpha: f32) {
        let start = Instant::now();
        let result = self.renderer.render(&self.camera);
        self.frame_stats.record_render(start.elapsed());
        match result {
            Ok(Some(stats)) => self.last_render = stats,
            // Image sautée : on garde les statistiques précédentes.
            Ok(None) => {}
            Err(err) => log::error!("erreur de rendu : {err:#}"),
        }
        self.show_stats(ctx);
    }

    fn resized(&mut self, _ctx: &mut Context, size: PhysicalSize<u32>) {
        self.renderer.resize(size.width, size.height);
    }
}

impl VoxelGame {
    /// Applique les résultats du streamer côté GPU, dans la limite du budget.
    /// Renvoie le nombre de maillages envoyés.
    fn apply_stream_events(&mut self) -> usize {
        let mut uploads = 0;
        while uploads < UPLOAD_BUDGET {
            let Some(event) = self.streamer.next_event() else {
                break;
            };
            match event {
                // Libérer est presque gratuit : ça ne compte pas dans le budget.
                StreamEvent::Unload(pos) => {
                    if let Some(id) = self.meshes.remove(&pos) {
                        self.renderer.remove_mesh(id);
                    }
                }
                StreamEvent::Mesh(pos, mesh) => {
                    uploads += 1;
                    // `insert` et `remove` rendent l'ancienne valeur : on libère l'ancien maillage.
                    let old = match upload(&mut self.renderer, &mesh) {
                        Some(id) => self.meshes.insert(pos, id),
                        None => self.meshes.remove(&pos),
                    };
                    if let Some(old) = old {
                        self.renderer.remove_mesh(old);
                    }
                }
            }
        }
        uploads
    }

    fn show_stats(&mut self, ctx: &mut Context) {
        let stream = self.streamer.stats();
        let render = self.last_render;
        let pending = stream.queued + stream.in_flight + stream.meshes_waiting;
        let triangles = render.triangles as f64 / 1e6;
        ctx.status = format!(
            "{} | {} chunks chargés | {pending} en attente | {}/{} dessinés | {triangles:.2} M triangles",
            self.streamer.config().mesher.name(),
            stream.loaded,
            render.meshes_drawn,
            render.meshes_total,
        );

        if let Some(report) = self.frame_stats.tick() {
            log::info!(
                "image : moy {:.2} ms, max {:.2} ms | streaming : max {:.2} ms, envois GPU : max {:.2} ms ({} au total) | rendu : max {:.2} ms | \
                 chunks : {} chargés, {} à générer, {} en cours, {} à envoyer | \
                 dessinés {}/{}, {triangles:.2} M triangles | caméra {:.0?}",
                report.avg_frame_ms,
                report.max_frame_ms,
                report.max_streaming_ms,
                report.max_upload_ms,
                report.uploads,
                report.max_render_ms,
                stream.loaded,
                stream.queued,
                stream.in_flight,
                stream.meshes_waiting,
                render.meshes_drawn,
                render.meshes_total,
                self.camera.position.to_array(),
            );
        }
    }

    fn handle_mouse_capture(&mut self, ctx: &mut Context) {
        let input = &ctx.input;
        if !self.mouse_captured && input.mouse_pressed(MouseButton::Left) {
            self.mouse_captured = true;
            ctx.set_cursor_captured(true);
        } else if input.key_pressed(KeyCode::Escape) {
            if self.mouse_captured {
                self.mouse_captured = false;
                ctx.set_cursor_captured(false);
            } else {
                ctx.exit();
            }
        }
    }

    fn look(&mut self, delta: Vec2) {
        self.camera.yaw += delta.x * MOUSE_SENSITIVITY;
        self.camera.pitch = (self.camera.pitch - delta.y * MOUSE_SENSITIVITY)
            .clamp(-Camera::MAX_PITCH, Camera::MAX_PITCH);
    }

    fn fly(&mut self, ctx: &Context, dt: f32) {
        let input = &ctx.input;
        // Chaque touche ajoute une direction ; on somme puis on normalise.
        let bindings = [
            (KeyCode::KeyW, self.camera.forward_flat()),
            (KeyCode::KeyS, -self.camera.forward_flat()),
            (KeyCode::KeyD, self.camera.right()),
            (KeyCode::KeyA, -self.camera.right()),
            (KeyCode::Space, Vec3::Y),
            (KeyCode::ShiftLeft, -Vec3::Y),
        ];
        let direction: Vec3 = bindings
            .iter()
            .filter(|(key, _)| input.key_held(*key))
            .map(|(_, dir)| *dir)
            .sum();

        let mut speed = MOVE_SPEED;
        if input.key_held(KeyCode::ControlLeft) {
            speed *= SPRINT_FACTOR;
        }
        // `normalize_or_zero` évite une division par zéro quand aucune touche n'est tenue.
        self.camera.position += direction.normalize_or_zero() * speed * dt;
    }

    /// Avance tout droit en suivant le relief, pour mesurer le chargement.
    fn autofly(&mut self, speed: f32, dt: f32) {
        self.camera.position += self.camera.forward_flat() * speed * dt;
        let p = self.camera.position;
        let ground = self
            .streamer
            .generator()
            .column(p.x.floor() as i32, p.z.floor() as i32)
            .height as f32;
        // On rejoint doucement l'altitude visée au lieu de sauter d'un coup.
        let target = ground + FLY_ALTITUDE;
        self.camera.position.y += (target - p.y) * (dt * 2.0).min(1.0);
    }
}

/// Envoie un maillage au GPU. `None` s'il est vide.
fn upload(renderer: &mut Renderer, mesh: &ChunkMesh) -> Option<MeshId> {
    let (min, max) = mesh.bounds()?;
    renderer.add_mesh(
        gpu_vertices(&mesh.vertices),
        &mesh.indices,
        Aabb::new(min, max),
    )
}

/// Réinterprète les sommets d'`engine_voxel` comme ceux d'`engine_render`, sans copie.
///
/// Les deux structs sont `#[repr(C)]` + `Pod` avec les mêmes champs (3 × `[f32; 3]`) :
/// `bytemuck::cast_slice` vérifie que tailles et alignements sont compatibles
/// (sinon il panique) et renvoie la même mémoire vue avec un autre type. Le test
/// `vertex_layouts_match` ci-dessous vérifie que les champs correspondent bien.
fn gpu_vertices(vertices: &[MeshVertex]) -> &[ColorVertex] {
    bytemuck::cast_slice(vertices)
}

/// Lit une variable d'environnement et la convertit (`u32`, `i32`...).
/// `T: FromStr` : n'importe quel type qui sait se lire depuis du texte.
fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}

/// Temps d'image moyen et maximum, pour vérifier qu'il n'y a pas de saccade.
struct FrameStats {
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
    uploads: usize,
}

struct FrameReport {
    avg_frame_ms: f32,
    max_frame_ms: f32,
    max_streaming_ms: f32,
    max_upload_ms: f32,
    max_render_ms: f32,
    uploads: usize,
}

impl FrameStats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            window_start: now,
            last_frame: now,
            frames: 0,
            max_frame: Duration::ZERO,
            max_streaming: Duration::ZERO,
            max_upload: Duration::ZERO,
            max_render: Duration::ZERO,
            uploads: 0,
        }
    }

    fn record_streaming(&mut self, streaming: Duration, upload: Duration, uploads: usize) {
        self.max_streaming = self.max_streaming.max(streaming);
        self.max_upload = self.max_upload.max(upload);
        self.uploads += uploads;
    }

    fn record_render(&mut self, time: Duration) {
        self.max_render = self.max_render.max(time);
    }

    /// À appeler une fois par image. Renvoie un bilan toutes les `STATS_LOG_INTERVAL`.
    fn tick(&mut self) -> Option<FrameReport> {
        let now = Instant::now();
        self.max_frame = self.max_frame.max(now - self.last_frame);
        self.last_frame = now;
        self.frames += 1;

        let elapsed = now - self.window_start;
        if elapsed < STATS_LOG_INTERVAL {
            return None;
        }
        let report = FrameReport {
            avg_frame_ms: elapsed.as_secs_f32() * 1000.0 / self.frames as f32,
            max_frame_ms: self.max_frame.as_secs_f32() * 1000.0,
            max_streaming_ms: self.max_streaming.as_secs_f32() * 1000.0,
            max_upload_ms: self.max_upload.as_secs_f32() * 1000.0,
            max_render_ms: self.max_render.as_secs_f32() * 1000.0,
            uploads: self.uploads,
        };
        // On repart à zéro ; `..Self::new()` remplit les champs non cités.
        *self = Self {
            last_frame: now,
            ..Self::new()
        };
        Some(report)
    }
}

fn main() -> Result<()> {
    // Logs visibles avec `RUST_LOG=info cargo run -p voxel`.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,voxel=info"))
        .init();

    engine_core::run::<VoxelGame>(AppConfig {
        title: "Voxel".into(),
        ..AppConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::mesh_greedy;

    #[test]
    fn vertex_layouts_match() {
        let vertex = MeshVertex {
            position: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            color: [0.5, 0.25, 0.125],
        };
        let gpu = gpu_vertices(std::slice::from_ref(&vertex))[0];
        assert_eq!(gpu.position, vertex.position);
        assert_eq!(gpu.normal, vertex.normal);
        assert_eq!(gpu.color, vertex.color);
    }

    #[test]
    fn terrain_meshes_convert_without_copy() {
        let world = TerrainGenerator::new(1).generate_region(1, 1);
        let mesh = mesh_greedy(&world, IVec3::new(0, 1, 0));
        let gpu = gpu_vertices(&mesh.vertices);
        assert_eq!(gpu.len(), mesh.vertices.len());
        // Même adresse mémoire : c'est une vue, pas une copie.
        assert_eq!(
            gpu.as_ptr().cast::<u8>(),
            mesh.vertices.as_ptr().cast::<u8>()
        );
    }
}
