//! Mesure, sans fenêtre, le délai « demande -> visible » du streaming dans trois scénarios : le
//! démarrage, une téléportation et un vol. Les images sont simulées à 60 par seconde en temps réel
//! (les threads de fond tournent pour de vrai) ; l'envoi au GPU est simulé : il coûte 25 µs plus
//! 0,7 ns par octet (une copie d'environ 1,4 Go/s), 11 µs par libération. C'est une hypothèse, pas
//! une mesure du pilote graphique : elle sert à comparer deux politiques sur la même charge.
//!
//! ```text
//! cargo run --release -p engine_voxel --example streaming_scenarios -- [--radius 10] [--seed 42]
//!     [--scenario demarrage|teleportation|vol|tout] [--far 0|48] [--vitesse 120]
//!     [--politique ancienne|colonnes|nouvelle|toutes]
//! ```
//!
//! `ancienne` : l'ordre par distance 3D de chaque chunk, 16 maillages par image, le terrain lointain
//! sans retenue. `colonnes` : comme `ancienne`, mais avec les colonnes complétées de la plus proche à la
//! plus lointaine (sans biais de vue). `nouvelle` : colonnes avec biais de vue, un budget d'octets et de
//! temps, le terrain lointain mis en veilleuse. Les
//! améliorations toujours actives (file des maillages du plus proche au plus lointain, remaillage
//! coalescé, lots de tuiles) ne sont pas isolées ici : elles ont leurs tests dans `streaming.rs` et
//! `far.rs`.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use engine_voxel::{
    ChunkStreamer, FarConfig, FarEvent, FarStreamer, StreamEvent, StreamingConfig,
    TerrainGenerator, TransferBudget, far_allowance, select_tiles,
};
use glam::{IVec3, Vec3};

const FRAME: Duration = Duration::from_micros(16_667);
/// Le coût simulé d'un envoi au GPU.
const UPLOAD_FIXED: Duration = Duration::from_micros(25);
const UPLOAD_PER_BYTE_NS: f64 = 0.7;
const UNLOAD_COST: Duration = Duration::from_micros(11);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Old,
    Columns,
    New,
}

impl Policy {
    fn name(self) -> &'static str {
        match self {
            Policy::Old => "ancienne",
            Policy::Columns => "colonnes",
            Policy::New => "nouvelle",
        }
    }

    fn column_order(self) -> bool {
        self != Policy::Old
    }

    fn view_bias(self) -> f32 {
        if self == Policy::New { 0.6 } else { 0.0 }
    }
}

struct Options {
    radius: i32,
    seed: u32,
    scenario: String,
    far: i32,
    speed: f32,
    policies: Vec<Policy>,
}

fn parse() -> Result<Options> {
    let mut o = Options {
        radius: 10,
        seed: 42,
        scenario: "tout".into(),
        far: 0,
        speed: 120.0,
        policies: vec![Policy::Old, Policy::Columns, Policy::New],
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| anyhow::anyhow!("{arg} attend une valeur"))
        };
        match arg.as_str() {
            "--radius" => o.radius = value()?.parse()?,
            "--seed" => o.seed = value()?.parse()?,
            "--scenario" => o.scenario = value()?,
            "--far" => o.far = value()?.parse()?,
            "--vitesse" => o.speed = value()?.parse()?,
            "--politique" => {
                o.policies = match value()?.as_str() {
                    "ancienne" => vec![Policy::Old],
                    "colonnes" => vec![Policy::Columns],
                    "nouvelle" => vec![Policy::New],
                    "toutes" => vec![Policy::Old, Policy::Columns, Policy::New],
                    other => bail!("politique inconnue « {other} »"),
                }
            }
            other => bail!("option inconnue « {other} »"),
        }
    }
    Ok(o)
}

struct Sim {
    policy: Policy,
    streamer: ChunkStreamer,
    far: Option<FarStreamer>,
    uploaded: HashSet<IVec3>,
    start: Instant,
    /// Les envois simulés : le pic d'octets et de temps par image.
    worst_frame_bytes: usize,
    worst_frame_sim: Duration,
    over_budget_frames: u32,
    frames: u32,
    far_tiles: usize,
}

impl Sim {
    fn new(seed: u32, radius: i32, far: i32, policy: Policy) -> Self {
        let config = StreamingConfig {
            view_radius: radius,
            column_order: policy.column_order(),
            view_bias: policy.view_bias(),
            ..StreamingConfig::default()
        };
        let streamer = ChunkStreamer::new(TerrainGenerator::new(seed), config);
        let far = (far > 0).then(|| {
            FarStreamer::new(
                streamer.generator_arc(),
                FarConfig {
                    near_radius: radius,
                    far_radius: far,
                },
                2,
            )
        });
        Self {
            policy,
            streamer,
            far,
            uploaded: HashSet::new(),
            start: Instant::now(),
            worst_frame_bytes: 0,
            worst_frame_sim: Duration::ZERO,
            over_budget_frames: 0,
            frames: 0,
            far_tiles: 0,
        }
    }

    /// Une image : le streamer, le lointain, les envois (simulés) de la politique choisie.
    fn step(&mut self, eye: Vec3, forward: Vec3) {
        let begun = Instant::now();
        self.frames += 1;
        let mut bytes_this_frame = 0usize;
        let mut sim_this_frame = Duration::ZERO;
        match self.policy {
            Policy::Old | Policy::Columns => {
                self.streamer.update(eye);
                if let Some(far) = &mut self.far {
                    for event in far.update(eye) {
                        if let FarEvent::Add(..) = event {
                            self.far_tiles += 1;
                        }
                    }
                }
                let (mut uploads, mut unloads) = (0, 0);
                while uploads < 16 && unloads < 256 {
                    let Some(event) = self.streamer.next_event() else {
                        break;
                    };
                    match event {
                        StreamEvent::Mesh(pos, mesh) => {
                            uploads += 1;
                            let bytes = mesh.byte_size();
                            bytes_this_frame += bytes;
                            sim_this_frame += upload_cost(bytes);
                            self.uploaded.insert(pos);
                        }
                        StreamEvent::Unload(pos) => {
                            unloads += 1;
                            sim_this_frame += UNLOAD_COST;
                            self.uploaded.remove(&pos);
                        }
                    }
                }
            }
            Policy::New => {
                self.streamer.update_view(eye, forward);
                if let Some(far) = &mut self.far {
                    far.set_allowance(far_allowance(self.streamer.stats().backlog(), 2));
                    far.refresh(eye);
                }
                let mut budget = TransferBudget::new(3 << 20, Duration::from_millis(3), 256);
                while let Some(event) = self.streamer.next_event_budgeted(&mut budget) {
                    match event {
                        StreamEvent::Mesh(pos, mesh) => {
                            let bytes = mesh.byte_size();
                            bytes_this_frame += bytes;
                            let cost = upload_cost(bytes);
                            sim_this_frame += cost;
                            budget.record_upload(bytes, cost);
                            self.uploaded.insert(pos);
                        }
                        StreamEvent::Unload(pos) => {
                            sim_this_frame += UNLOAD_COST;
                            budget.record_unload(UNLOAD_COST);
                            self.uploaded.remove(&pos);
                        }
                    }
                }
                if let Some(far) = &mut self.far {
                    while let Some(batch) = far.next_batch(&budget) {
                        for event in batch {
                            if let FarEvent::Add(_, mesh) = event {
                                let bytes = mesh.byte_size();
                                bytes_this_frame += bytes;
                                let cost = upload_cost(bytes);
                                sim_this_frame += cost;
                                budget.record_upload(bytes, cost);
                                self.far_tiles += 1;
                            }
                        }
                    }
                }
                if budget.exceeded() {
                    self.over_budget_frames += 1;
                }
            }
        }
        self.worst_frame_bytes = self.worst_frame_bytes.max(bytes_this_frame);
        self.worst_frame_sim = self.worst_frame_sim.max(sim_this_frame);
        // Le reste de l'image : les threads de fond travaillent.
        if let Some(rest) = FRAME.checked_sub(begun.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    fn ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// Une colonne est visible quand ses six chunks sont générés et que chaque chunk non vide a son
    /// maillage envoyé.
    fn column_visible(&self, cx: i32, cz: i32) -> bool {
        (0..6).all(|y| {
            let pos = IVec3::new(cx, y, cz);
            self.streamer.is_generated(pos)
                && (self.streamer.world().chunk(pos).is_none() || self.uploaded.contains(&pos))
        })
    }

    /// Les colonnes à moins de `radius` chunks de `eye` qui ne sont pas encore visibles : (devant,
    /// derrière) la direction `forward`.
    fn missing(&self, eye: Vec3, forward: Vec3, radius: i32) -> (usize, usize) {
        let (cx, cz) = ((eye.x / 32.0).floor() as i32, (eye.z / 32.0).floor() as i32);
        let (mut front, mut back) = (0, 0);
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dz * dz > radius * radius || self.column_visible(cx + dx, cz + dz) {
                    continue;
                }
                if dx as f32 * forward.x + dz as f32 * forward.z >= 0.0 {
                    front += 1;
                } else {
                    back += 1;
                }
            }
        }
        (front, back)
    }
}

fn upload_cost(bytes: usize) -> Duration {
    UPLOAD_FIXED + Duration::from_nanos((bytes as f64 * UPLOAD_PER_BYTE_NS) as u64)
}

/// Attend que les colonnes à moins de chacun des `radii` soient visibles ; renvoie, pour chaque
/// rayon, le temps (ms) où toutes celles qui sont devant `forward` le sont, puis celui où toutes le
/// sont, ou `None` après `limit`.
fn time_to_visible(
    sim: &mut Sim,
    eye: Vec3,
    forward: Vec3,
    radii: &[i32],
    limit: Duration,
) -> Vec<(Option<f64>, Option<f64>)> {
    let started = sim.ms();
    let mut done: Vec<(Option<f64>, Option<f64>)> = vec![(None, None); radii.len()];
    while done.iter().any(|d| d.1.is_none()) && sim.ms() - started < limit.as_secs_f64() * 1000.0 {
        sim.step(eye, forward);
        for (i, &r) in radii.iter().enumerate() {
            if done[i].1.is_none() {
                let (f, b) = sim.missing(eye, forward, r);
                if f == 0 && done[i].0.is_none() {
                    done[i].0 = Some(sim.ms() - started);
                }
                if f + b == 0 {
                    done[i].1 = Some(sim.ms() - started);
                }
            }
        }
    }
    done
}

fn show(times: &[(Option<f64>, Option<f64>)]) -> String {
    let one = |t: Option<f64>| t.map_or("   —".to_string(), |t| format!("{t:5.0}"));
    times
        .iter()
        .map(|(front, all)| format!("{}/{}", one(*front), one(*all)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn report_transfers(sim: &Sim) {
    println!(
        "    pic de transfert : {:.2} Mio et {:.1} ms simulés dans une image ; {} images au-dessus du budget ; {} tuiles lointaines",
        sim.worst_frame_bytes as f64 / 1_048_576.0,
        sim.worst_frame_sim.as_secs_f64() * 1000.0,
        sim.over_budget_frames,
        sim.far_tiles,
    );
}

fn main() -> Result<()> {
    let o = parse()?;
    let radii = [2, 4, 6, o.radius];
    println!(
        "rayon de vue {} chunks, graine {}, lointain {} | délais en ms « devant / tout » jusqu'à ce que les colonnes soient visibles dans un rayon de {radii:?} chunks (devant : le demi-plan que la caméra regarde)",
        o.radius,
        o.seed,
        if o.far > 0 {
            format!("jusqu'à {} chunks", o.far)
        } else {
            "éteint".into()
        },
    );
    let wants = |name: &str| o.scenario == "tout" || o.scenario == name;
    let eye = Vec3::new(8.0, 100.0, 8.0);
    let forward = Vec3::X;

    if wants("demarrage") {
        println!("\n## Démarrage (monde vide, caméra immobile)");
        for &policy in &o.policies {
            let mut sim = Sim::new(o.seed, o.radius, o.far, policy);
            let t = time_to_visible(&mut sim, eye, forward, &radii, Duration::from_secs(90));
            println!("  {:9} | {}", policy.name(), show(&t));
            report_transfers(&sim);
            if o.far > 0 {
                // Le lointain finit-il, et quand, une fois les chunks proches visibles ?
                let wanted = select_tiles(
                    eye,
                    FarConfig {
                        near_radius: o.radius,
                        far_radius: o.far,
                    },
                )
                .len();
                let near_done = sim.ms();
                while sim.far_tiles < wanted && sim.ms() - near_done < 60_000.0 {
                    sim.step(eye, forward);
                }
                println!(
                    "    lointain : {}/{} tuiles montrées, {} ms après les chunks proches",
                    sim.far_tiles.min(wanted),
                    wanted,
                    (sim.ms() - near_done) as i64
                );
            }
        }
    }

    if wants("teleportation") {
        println!("\n## Téléportation à 5 000 blocs (tout est à recharger)");
        for &policy in &o.policies {
            let mut sim = Sim::new(o.seed, o.radius, o.far, policy);
            time_to_visible(&mut sim, eye, forward, &[o.radius], Duration::from_secs(90));
            sim.worst_frame_bytes = 0;
            sim.worst_frame_sim = Duration::ZERO;
            sim.over_budget_frames = 0;
            let away = Vec3::new(5008.0, 100.0, 8.0);
            let t = time_to_visible(&mut sim, away, forward, &radii, Duration::from_secs(90));
            println!("  {:9} | {}", policy.name(), show(&t));
            report_transfers(&sim);
        }
    }

    if wants("vol") {
        println!(
            "\n## Vol à {} blocs/s pendant 25 s (colonnes manquantes dans un rayon de {} chunks)",
            o.speed,
            o.radius - 2
        );
        for &policy in &o.policies {
            let mut sim = Sim::new(o.seed, o.radius, o.far, policy);
            time_to_visible(&mut sim, eye, forward, &[o.radius], Duration::from_secs(90));
            sim.worst_frame_bytes = 0;
            sim.worst_frame_sim = Duration::ZERO;
            sim.over_budget_frames = 0;
            let mut position = eye;
            let (mut front, mut back): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
            let mut first_seen: std::collections::HashMap<(i32, i32), f64> = Default::default();
            let mut latency: Vec<f64> = Vec::new();
            let frames = (25.0 / FRAME.as_secs_f64()) as usize;
            for frame in 0..frames {
                position += forward * (o.speed * FRAME.as_secs_f32());
                sim.step(position, forward);
                if frame < 150 {
                    continue; // les 2,5 premières secondes : l'accélération
                }
                let (f, b) = sim.missing(position, forward, o.radius - 2);
                front.push(f);
                back.push(b);
                // La latence de chaque colonne : de son entrée dans le rayon de 8 à sa visibilité.
                let (cx, cz) = (
                    (position.x / 32.0).floor() as i32,
                    (position.z / 32.0).floor() as i32,
                );
                let rr = o.radius - 2;
                for dz in -rr..=rr {
                    for dx in -rr..=rr {
                        if dx * dx + dz * dz > rr * rr {
                            continue;
                        }
                        let key = (cx + dx, cz + dz);
                        let now = sim.ms();
                        let seen = *first_seen.entry(key).or_insert(now);
                        if seen >= 0.0 && sim.column_visible(key.0, key.1) {
                            latency.push(now - seen);
                            first_seen.insert(key, f64::NEG_INFINITY);
                        }
                    }
                }
            }
            front.sort_unstable();
            back.sort_unstable();
            latency.sort_by(|a, b| a.total_cmp(b));
            let pct = |v: &[usize], p: f64| v[((v.len() - 1) as f64 * p) as usize];
            let mean = |v: &[usize]| v.iter().sum::<usize>() as f64 / v.len() as f64;
            let lat = |p: f64| {
                latency
                    .get(((latency.len().max(1) - 1) as f64 * p) as usize)
                    .copied()
                    .unwrap_or(f64::NAN)
            };
            println!(
                "  {:9} | devant : moy {:5.1}, p95 {:3}, max {:3} | derrière : moy {:5.1}, p95 {:3}, max {:3} | entrée -> visible : p50 {:5.0} ms, p95 {:5.0} ms, max {:5.0} ms",
                policy.name(),
                mean(&front),
                pct(&front, 0.95),
                front.last().copied().unwrap_or(0),
                mean(&back),
                pct(&back, 0.95),
                back.last().copied().unwrap_or(0),
                lat(0.5),
                lat(0.95),
                latency.last().copied().unwrap_or(f64::NAN),
            );
            report_transfers(&sim);
        }
    }
    Ok(())
}
