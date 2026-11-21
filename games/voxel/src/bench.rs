//! Le benchmark reproductible : `VOXEL_BENCH=résultats.csv` (ou `cargo xtask bench`).
//!
//! Même graine (42, ou `VOXEL_SEED`), mêmes trajets, même heure (midi, figée), sans VSync,
//! dans une fenêtre de 1280 × 720 : deux lancements sur la même machine donnent des
//! chiffres comparables, et on peut mesurer l'effet d'un changement du moteur.
//!
//! Chaque trajet commence par un temps de chargement (on attend que tout soit généré et
//! maillé autour du point de départ), puis la caméra suit le trajet pendant quelques
//! secondes en mesurant chaque image :
//! - **FPS moyen** et **1 % le plus lent** (la moyenne des 1 % d'images les plus longues,
//!   en FPS). Attention : sans VSync, une image lente est presque toujours une attente du
//!   système (ses 3 tampons d'écran sont pleins, il faut attendre le rafraîchissement
//!   suivant), pas du travail du jeu. Mesuré : sur 1 300 images, aucune ne dépassait 4,4 ms
//!   de travail. Les vraies saccades du jeu se voient dans le 99e centile du CPU, ou avec
//!   `VOXEL_BENCH_VSYNC=1` (comme en jeu) ;
//! - **CPU** : le temps médian du jeu par image (simulation, mise à jour, préparation du
//!   rendu), sans l'attente de l'écran, et le 99e centile ;
//! - **GPU** : le temps médian de la passe du décor, mesuré par le GPU (`gpu_timer.rs`) ;
//! - **chunks et triangles dessinés** : exactement les mêmes d'un lancement à l'autre.
//!
//! Fenêtre au premier plan, les mesures sont stables (±5 %). Cachée ou derrière une autre
//! fenêtre, macOS la ralentit : les FPS plafonnent au rythme de l'écran et les temps varient
//! de ±20 %. `cargo xtask bench` lance plusieurs passages et ne signale que les écarts plus
//! grands que le bruit mesuré.
//!
//! Idée reprise d'Horizon (`docs/benchmarks.md` : graine fixe, quatre trajets).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use engine_core::Context;
use engine_core::winit::dpi::LogicalSize;
use engine_voxel::SEA_LEVEL;
use glam::Vec3;

use crate::VoxelGame;
use crate::daytime::TimeSpeed;
use crate::player::MoveMode;

/// Images d'attente après le chargement, et au début de chaque mesure (préchauffage).
const SETTLE_FRAMES: u32 = 30;
/// Images passées loin avant chaque trajet (le streamer décharge au changement de chunk).
const RESET_FRAMES: u32 = 3;
/// Au-delà, on mesure quand même (le trajet est signalé comme pas chargé).
const LOAD_TIMEOUT: Duration = Duration::from_secs(90);

/// Un trajet : un point de départ, un cap, une vitesse, une durée.
#[derive(Debug, Clone)]
struct Route {
    name: &'static str,
    /// Départ (x, z), en blocs.
    start: (f32, f32),
    /// Hauteur des yeux : au-dessus du sol (ou de la mer) qui défile, ou fixe.
    height: Height,
    /// Cap et inclinaison, en degrés.
    yaw: f32,
    pitch: f32,
    /// Blocs par seconde, droit devant (à plat).
    speed: f32,
    /// Degrés par seconde de rotation sur place.
    turn: f32,
    seconds: f32,
}

#[derive(Debug, Clone, Copy)]
enum Height {
    AboveGround(f32),
    Fixed(f32),
}

/// Une image mesurée.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// Durée de l'image (de l'une à la suivante), en secondes.
    frame: f32,
    /// Travail du CPU, en secondes.
    cpu: f32,
    gpu: Option<f32>,
    triangles: usize,
    drawn: usize,
}

/// Le bilan d'un trajet.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteResult {
    pub name: String,
    pub frames: usize,
    pub fps: f64,
    pub fps_low: f64,
    pub cpu_ms: f64,
    pub cpu_p99_ms: f64,
    pub gpu_ms: Option<f64>,
    pub triangles_m: f64,
    pub chunks: f64,
    /// Le chargement n'était pas fini au début de la mesure.
    pub not_loaded: bool,
}

enum Stage {
    /// Pas encore commencé (les trajets dépendent du monde chargé).
    Start,
    /// La caméra part très loin quelques images : tous les chunks sont déchargés, et
    /// chaque trajet commence dans un monde vide, quels que soient les trajets d'avant.
    Reset {
        frames: u32,
    },
    Load {
        since: Instant,
        quiet: u32,
    },
    Measure {
        time: f32,
        warmup: u32,
    },
}

pub struct Bench {
    output: PathBuf,
    routes: Vec<Route>,
    index: usize,
    stage: Stage,
    samples: Vec<Sample>,
    not_loaded: bool,
    results: Vec<RouteResult>,
    started: Instant,
}

impl Bench {
    /// `VOXEL_BENCH=fichier.csv` (`1` : `bench.csv`).
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("VOXEL_BENCH").ok()?;
        let output = if value == "1" {
            "bench.csv".into()
        } else {
            value
        };
        Some(Self {
            output: PathBuf::from(output),
            routes: Vec::new(),
            index: 0,
            stage: Stage::Start,
            samples: Vec::new(),
            not_loaded: false,
            results: Vec::new(),
            started: Instant::now(),
        })
    }
}

impl VoxelGame {
    /// Une image du benchmark ; quitte le jeu à la fin.
    pub(crate) fn run_bench(&mut self, ctx: &mut Context, dt: f32) -> Result<()> {
        let Some(mut bench) = self.client.bench.take() else {
            return Ok(());
        };
        let done = self.bench_step(&mut bench, ctx, dt);
        let finished = matches!(done, Ok(true));
        if finished {
            write_results(&bench)?;
            ctx.exit();
        }
        self.client.bench = Some(bench);
        done.map(|_| ())
    }

    /// Renvoie `true` quand tous les trajets sont mesurés.
    fn bench_step(&mut self, bench: &mut Bench, ctx: &mut Context, dt: f32) -> Result<bool> {
        if let Stage::Start = bench.stage {
            self.setup_bench(ctx);
            bench.routes = self.bench_routes();
            log::info!(
                "benchmark : {} trajets ({})",
                bench.routes.len(),
                bench
                    .routes
                    .iter()
                    .map(|r| r.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            self.start_route(bench);
            return Ok(false);
        }
        let Some(route) = bench.routes.get(bench.index).cloned() else {
            return Ok(true);
        };
        match &mut bench.stage {
            Stage::Start => {}
            Stage::Reset { frames } => {
                *frames += 1;
                if *frames >= RESET_FRAMES {
                    log::info!("benchmark : chargement de « {} »", route.name);
                    self.place_on_route(&route, 0.0);
                    bench.stage = Stage::Load {
                        since: Instant::now(),
                        quiet: 0,
                    };
                }
            }
            Stage::Load { since, quiet } => {
                let ready = self.world_idle() && self.stream_stats().meshes_waiting == 0;
                *quiet = if ready { *quiet + 1 } else { 0 };
                let timeout = since.elapsed() > LOAD_TIMEOUT;
                if *quiet >= SETTLE_FRAMES || timeout {
                    if timeout {
                        log::warn!(
                            "benchmark : {} pas chargé après {LOAD_TIMEOUT:?}",
                            route.name
                        );
                    }
                    // Le délai de chargement (démarrage ou téléportation) : le temps jusqu'à ce que tout
                    // soit généré, maillé et envoyé, moins les images d'attente de la fin.
                    log::info!(
                        "benchmark : « {} » chargé en {:.2} s ({} chunks)",
                        route.name,
                        since.elapsed().as_secs_f64(),
                        self.stream_stats().loaded
                    );
                    bench.not_loaded = timeout;
                    bench.samples.clear();
                    bench.stage = Stage::Measure {
                        time: 0.0,
                        warmup: SETTLE_FRAMES,
                    };
                }
            }
            Stage::Measure { time, warmup } => {
                if *warmup > 0 {
                    *warmup -= 1;
                } else {
                    // Les mesures de l'image précédente : elle est finie.
                    let render = self.client.last_render;
                    let cpu = ctx.frame_work.saturating_sub(render.wait);
                    bench.samples.push(Sample {
                        frame: dt,
                        cpu: cpu.as_secs_f32(),
                        gpu: render.gpu_scene.map(|d| d.as_secs_f32()),
                        triangles: render.triangles,
                        drawn: render.meshes_drawn,
                    });
                    *time += dt;
                }
                let time = *time;
                self.place_on_route(&route, time);
                if time >= route.seconds {
                    let result = summarize(&route, &bench.samples, bench.not_loaded);
                    log::info!(
                        "benchmark {} : {:.0} FPS, 1 % le plus lent {:.0} FPS, CPU {:.2} ms, GPU {} ms",
                        result.name,
                        result.fps,
                        result.fps_low,
                        result.cpu_ms,
                        result.gpu_ms.map_or("?".into(), |g| format!("{g:.2}"))
                    );
                    bench.results.push(result);
                    bench.index += 1;
                    if bench.index == bench.routes.len() {
                        return Ok(true);
                    }
                    self.start_route(bench);
                }
            }
        }
        Ok(false)
    }

    /// Mêmes conditions à chaque lancement : vol, midi figé, sans VSync, 1280 × 720
    /// (`VOXEL_BENCH_SIZE=640x360` pour une autre taille : si le GPU va bien plus vite en
    /// plus petit, ce sont les pixels qui coûtent, pas les sommets).
    fn setup_bench(&mut self, ctx: &mut Context) {
        let (width, height) = std::env::var("VOXEL_BENCH_SIZE")
            .ok()
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?))
            })
            .unwrap_or((1280.0, 720.0));
        let _ = ctx
            .window
            .request_inner_size(LogicalSize::new(width, height));
        // `VOXEL_BENCH_VSYNC=1` : comme en jeu, pour voir les images ratées. Sans VSync,
        // le « 1 % le plus lent » mesure surtout l'attente d'une image libre auprès du
        // système (les 3 tampons de l'écran sont pleins), pas le travail du jeu.
        self.client
            .renderer
            .set_vsync(std::env::var("VOXEL_BENCH_VSYNC").is_ok_and(|v| v == "1"));
        self.server.day.time = 0.5;
        self.server.day.speed = TimeSpeed::Frozen;
        if self.client.body.mode != MoveMode::Fly {
            self.toggle_fly();
        }
    }

    fn start_route(&mut self, bench: &mut Bench) {
        let Some(route) = bench.routes.get(bench.index).cloned() else {
            return;
        };
        // Bien au-delà du rayon de déchargement.
        let far = Route {
            start: (route.start.0 + 100_000.0, route.start.1),
            height: Height::Fixed(150.0),
            ..route
        };
        self.place_on_route(&far, 0.0);
        bench.stage = Stage::Reset { frames: 0 };
    }

    /// Place la caméra là où le trajet en est après `time` secondes.
    fn place_on_route(&mut self, route: &Route, time: f32) {
        let yaw = (route.yaw + route.turn * time).to_radians();
        let direction = Vec3::new(
            route.yaw.to_radians().cos(),
            0.0,
            route.yaw.to_radians().sin(),
        );
        let flat = Vec3::new(route.start.0, 0.0, route.start.1) + direction * route.speed * time;
        let y = match route.height {
            Height::Fixed(y) => y,
            Height::AboveGround(h) => self.surface(flat.x, flat.z) + h,
        };
        self.client.camera.yaw = yaw;
        self.client.camera.pitch = route.pitch.to_radians();
        self.place_eye(Vec3::new(flat.x, y, flat.z));
    }

    /// Le sol, ou la mer si elle est plus haute.
    fn surface(&self, x: f32, z: f32) -> f32 {
        let column = self
            .server
            .streamer()
            .generator()
            .column(x.floor() as i32, z.floor() as i32);
        column.height.max(SEA_LEVEL) as f32 + 1.0
    }

    /// Les trajets, calculés depuis le point d'apparition : les mêmes pour une graine.
    fn bench_routes(&self) -> Vec<Route> {
        let spawn = (
            self.server.spawn.x as f32 + 0.5,
            self.server.spawn.z as f32 + 0.5,
        );
        let biome = |key: &str| {
            self.find_biome(key).map(|(x, z)| {
                (
                    (self.server.spawn.x + x) as f32 + 0.5,
                    (self.server.spawn.z + z) as f32 + 0.5,
                )
            })
        };
        let route = |name, start, height, speed, seconds| Route {
            name,
            start,
            height,
            yaw: 0.0,
            pitch: -15.0,
            speed,
            turn: 0.0,
            seconds,
        };
        let mut routes = vec![
            // Une vue large qui défile vite : le chargement et le rendu ensemble.
            route("survol", spawn, Height::AboveGround(40.0), 30.0, 12.0),
            // Le rendu seul : rien ne se charge, la caméra tourne sur place.
            Route {
                turn: 30.0,
                pitch: -5.0,
                ..route("immobile", spawn, Height::AboveGround(2.0), 0.0, 12.0)
            },
        ];
        for (name, key, height, speed) in [
            ("foret", "core:forest", 4.0, 6.0),
            ("montagnes", "core:mountains", 12.0, 10.0),
            ("ocean", "core:ocean", 3.0, 10.0),
        ] {
            match biome(key) {
                Some(start) => routes.push(Route {
                    pitch: 0.0,
                    ..route(name, start, Height::AboveGround(height), speed, 10.0)
                }),
                None => log::warn!("benchmark : pas de biome {key} près du départ"),
            }
        }
        // Sous terre : le culling des grottes doit retirer presque tout.
        routes.push(Route {
            pitch: 0.0,
            ..route("sous-sol", spawn, Height::Fixed(20.0), 6.0, 10.0)
        });
        // `VOXEL_BENCH_ONLY=foret,ocean` : seulement ces trajets.
        if let Ok(only) = std::env::var("VOXEL_BENCH_ONLY") {
            routes.retain(|r| only.split(',').any(|name| name.trim() == r.name));
        }
        routes
    }
}

/// Le bilan d'un trajet à partir de ses images.
fn summarize(route: &Route, samples: &[Sample], not_loaded: bool) -> RouteResult {
    let n = samples.len().max(1) as f64;
    let total: f64 = samples.iter().map(|s| f64::from(s.frame)).sum();
    let mut frames: Vec<f32> = samples.iter().map(|s| s.frame).collect();
    frames.sort_by(|a, b| b.total_cmp(a));
    let worst = (frames.len() / 100).max(1).min(frames.len());
    let worst_mean =
        frames[..worst].iter().map(|&f| f64::from(f)).sum::<f64>() / worst.max(1) as f64;
    // Des médianes : une image retardée par le système ne fausse pas tout le trajet.
    let mut cpu: Vec<f32> = samples.iter().map(|s| s.cpu).collect();
    cpu.sort_by(f32::total_cmp);
    let p99 = cpu
        .get((cpu.len() * 99 / 100).min(cpu.len().saturating_sub(1)))
        .copied();
    let mut gpu: Vec<f32> = samples.iter().filter_map(|s| s.gpu).collect();
    gpu.sort_by(f32::total_cmp);
    RouteResult {
        name: route.name.to_string(),
        frames: samples.len(),
        fps: if total > 0.0 { n / total } else { 0.0 },
        fps_low: if worst_mean > 0.0 {
            1.0 / worst_mean
        } else {
            0.0
        },
        cpu_ms: f64::from(median(&cpu)) * 1000.0,
        cpu_p99_ms: f64::from(p99.unwrap_or(0.0)) * 1000.0,
        gpu_ms: (!gpu.is_empty()).then(|| f64::from(median(&gpu)) * 1000.0),
        triangles_m: samples.iter().map(|s| s.triangles as f64).sum::<f64>() / n / 1e6,
        chunks: samples.iter().map(|s| s.drawn as f64).sum::<f64>() / n,
        not_loaded,
    }
}

/// La valeur du milieu d'une liste triée (0 si vide).
fn median(sorted: &[f32]) -> f32 {
    sorted.get(sorted.len() / 2).copied().unwrap_or(0.0)
}

/// L'en-tête du fichier : `cargo xtask bench` lit les mêmes colonnes.
pub const CSV_HEADER: &str =
    "trajet,images,fps_moyen,fps_1pct,cpu_ms,cpu_p99_ms,gpu_ms,triangles_M,chunks_dessines,charge";

fn to_csv(results: &[RouteResult]) -> String {
    let mut csv = String::from(CSV_HEADER);
    csv.push('\n');
    for r in results {
        csv.push_str(&format!(
            "{},{},{:.1},{:.1},{:.3},{:.3},{},{:.3},{:.0},{}\n",
            r.name,
            r.frames,
            r.fps,
            r.fps_low,
            r.cpu_ms,
            r.cpu_p99_ms,
            r.gpu_ms.map_or(String::new(), |g| format!("{g:.3}")),
            r.triangles_m,
            r.chunks,
            if r.not_loaded { "non" } else { "oui" },
        ));
    }
    csv
}

fn write_results(bench: &Bench) -> Result<()> {
    if let Some(dir) = bench.output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&bench.output, to_csv(&bench.results))?;
    log::info!(
        "benchmark terminé en {:.0?} : {}",
        bench.started.elapsed(),
        bench.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(frame_ms: f32, cpu_ms: f32) -> Sample {
        Sample {
            frame: frame_ms / 1000.0,
            cpu: cpu_ms / 1000.0,
            gpu: Some(0.002),
            triangles: 1_000_000,
            drawn: 500,
        }
    }

    #[test]
    fn slow_frames_show_in_the_one_percent_low() {
        let route = Route {
            name: "test",
            start: (0.0, 0.0),
            height: Height::Fixed(0.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 0.0,
            turn: 0.0,
            seconds: 1.0,
        };
        // 198 images de 5 ms, 2 de 50 ms : la moyenne bouge peu, le 1 % beaucoup.
        let mut samples = vec![sample(5.0, 3.0); 198];
        samples.extend([sample(50.0, 40.0); 2]);
        let r = summarize(&route, &samples, false);
        assert_eq!(r.frames, 200);
        assert!((r.fps - 200.0 / 1.09).abs() < 0.5, "{}", r.fps);
        assert!((r.fps_low - 20.0).abs() < 0.01, "{}", r.fps_low);
        assert!((r.cpu_ms - 3.0).abs() < 0.01, "médiane : {}", r.cpu_ms);
        assert!((r.cpu_p99_ms - 40.0).abs() < 0.01, "{}", r.cpu_p99_ms);
        assert_eq!(r.gpu_ms.map(|g| g.round()), Some(2.0));
        assert!((r.triangles_m - 1.0).abs() < 1e-9);

        let csv = to_csv(&[r]);
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(CSV_HEADER));
        let row: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(row.len(), CSV_HEADER.split(',').count());
        assert_eq!(row[0], "test");
    }
}
