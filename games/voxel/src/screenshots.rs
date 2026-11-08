//! Le mode `VOXEL_SCREENSHOTS=dossier` : une visite guidée qui enregistre des images.
//!
//! La caméra se place tour à tour à midi, au coucher du soleil, la nuit, près de
//! torches, au-dessus d'un lac et dans une grotte éclairée par une torche. Pour chaque
//! vue, on attend que le monde soit chargé et maillé, puis on dessine l'image hors
//! écran (`Renderer::capture`) et on l'écrit en PNG. Ça marche même écran éteint, et
//! ça permet de vérifier le rendu sans être devant la machine.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result};
use engine_core::Context;
use engine_voxel::{BlockId, SEA_LEVEL};
use glam::{IVec3, Vec3};

use crate::VoxelGame;
use crate::daytime::TimeSpeed;
use crate::player::{self, MoveMode};

/// Où poser les créatures de la capture.
const MOB_SPOT: (i32, i32) = (12, -6);

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
/// Images à attendre une fois le monde prêt : le temps que tout soit envoyé au GPU.
const SETTLE_FRAMES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    Spawn,
    Sunset,
    Torches,
    Lake,
    Cave,
    Mobs,
    Inventory,
    Inspector,
}

struct Shot {
    name: &'static str,
    time: f32,
    scene: Scene,
}

const SHOTS: [Shot; 9] = [
    Shot {
        name: "1-midi",
        time: 0.5,
        scene: Scene::Spawn,
    },
    Shot {
        name: "2-coucher",
        time: 0.74,
        scene: Scene::Sunset,
    },
    Shot {
        name: "3-nuit",
        time: 0.0,
        scene: Scene::Spawn,
    },
    Shot {
        name: "4-torches-nuit",
        time: 0.0,
        scene: Scene::Torches,
    },
    Shot {
        name: "5-lac",
        time: 0.45,
        scene: Scene::Lake,
    },
    Shot {
        name: "6-grotte",
        time: 0.5,
        scene: Scene::Cave,
    },
    Shot {
        name: "7-creatures",
        time: 0.45,
        scene: Scene::Mobs,
    },
    Shot {
        name: "8-inventaire",
        time: 0.45,
        scene: Scene::Inventory,
    },
    Shot {
        name: "9-inspecteur",
        time: 0.45,
        scene: Scene::Inspector,
    },
];

/// Où en est la visite.
pub struct Tour {
    dir: PathBuf,
    index: usize,
    stage: Stage,
    started: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Placer la caméra.
    Move,
    /// Attendre le chargement, puis faire les modifications de la scène.
    Build,
    /// Attendre que tout soit remaillé, quelques images de plus, puis capturer.
    Settle(u32),
}

impl Tour {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            index: 0,
            stage: Stage::Move,
            started: Instant::now(),
        }
    }
}

impl VoxelGame {
    /// Une étape de la visite, à chaque image.
    pub(crate) fn run_tour(&mut self, ctx: &mut Context) -> Result<()> {
        let Some(tour) = self.tour.as_mut() else {
            return Ok(());
        };
        let Some(shot) = SHOTS.get(tour.index) else {
            log::info!(
                "{} captures enregistrées dans {} en {:.0?}",
                SHOTS.len(),
                tour.dir.display(),
                tour.started.elapsed()
            );
            ctx.exit();
            return Ok(());
        };
        let stage = tour.stage;
        let ready = self.streamer().is_idle();
        match stage {
            Stage::Move => {
                self.day.time = shot.time;
                self.day.speed = TimeSpeed::Frozen;
                if player::mode(&self.ecs, self.player) != MoveMode::Fly {
                    player::toggle_mode(&mut self.ecs, self.player);
                }
                let (eye, yaw, pitch) = self.viewpoint(shot.scene);
                player::set_eye(&self.ecs, self.player, eye);
                self.ui.inventory_open = shot.scene == Scene::Inventory;
                self.ui.inspector_open = shot.scene == Scene::Inspector;
                self.camera.yaw = yaw.to_radians();
                self.camera.pitch = pitch.to_radians();
                self.set_stage(Stage::Build);
            }
            Stage::Build if ready => {
                self.build_scene(shot.scene);
                self.set_stage(Stage::Settle(0));
            }
            Stage::Settle(frames) if ready => {
                if frames < SETTLE_FRAMES {
                    self.set_stage(Stage::Settle(frames + 1));
                } else {
                    let start = Instant::now();
                    // L'interface aussi : la barre d'objets, l'inventaire ou l'inspecteur.
                    let (ui, _) = self.build_ui(crate::ui::offscreen_input(WIDTH, HEIGHT));
                    let capture = self
                        .renderer
                        .capture(&self.camera, WIDTH, HEIGHT, Some(ui))?;
                    let elapsed = start.elapsed();
                    let dir = self
                        .tour
                        .as_ref()
                        .map(|t| t.dir.clone())
                        .unwrap_or_default();
                    let path = dir.join(format!("{}.png", shot.name));
                    save_png(&path, capture.width, capture.height, &capture.rgba)?;
                    log::info!(
                        "capture {} : {} chunks dessinés, {:.2} M triangles, dessin + lecture {:.1} ms",
                        path.display(),
                        capture.stats.meshes_drawn,
                        capture.stats.triangles as f64 / 1e6,
                        elapsed.as_secs_f32() * 1000.0
                    );
                    if tour_index(&self.tour) == 0 {
                        self.measure_gpu()?;
                    }
                    if let Some(tour) = self.tour.as_mut() {
                        tour.index += 1;
                    }
                    self.set_stage(Stage::Move);
                }
            }
            // Pas encore prêt : on attend l'image suivante.
            Stage::Build | Stage::Settle(_) => {}
        }
        Ok(())
    }

    /// Mesure le temps d'une image complète à la résolution native d'une fenêtre
    /// 1280 × 720 sur écran Retina (2560 × 1440), lecture des pixels comprise :
    /// une borne haute du travail du GPU par image, même écran éteint.
    fn measure_gpu(&mut self) -> Result<()> {
        const FRAMES: u32 = 10;
        self.renderer.capture(&self.camera, 2560, 1440, None)?; // préchauffage
        let start = Instant::now();
        for _ in 0..FRAMES {
            self.renderer.capture(&self.camera, 2560, 1440, None)?;
        }
        log::info!(
            "rendu hors écran 2560 × 1440 : {:.2} ms par image (lecture des pixels comprise)",
            start.elapsed().as_secs_f32() * 1000.0 / FRAMES as f32
        );
        Ok(())
    }

    fn set_stage(&mut self, stage: Stage) {
        if let Some(tour) = self.tour.as_mut() {
            tour.stage = stage;
        }
    }

    fn ground(&self, x: i32, z: i32) -> i32 {
        self.streamer().generator().column(x, z).height
    }

    /// Position des yeux, cap et inclinaison (en degrés) pour chaque scène.
    fn viewpoint(&self, scene: Scene) -> (Vec3, f32, f32) {
        let spawn = self.ground(0, 0) as f32;
        match scene {
            Scene::Spawn => (Vec3::new(0.5, spawn + 20.0, 0.5), 45.0, -20.0),
            // Le soleil se couche à l'ouest (-x) : on regarde vers lui.
            Scene::Sunset => (Vec3::new(0.5, spawn + 20.0, 0.5), 180.0, -12.0),
            Scene::Torches => {
                let g = self.ground(20, 20) as f32;
                (Vec3::new(8.5, g + 12.0, 8.5), 45.0, -35.0)
            }
            Scene::Lake => {
                let lake = self.find_lake().unwrap_or(IVec3::new(0, SEA_LEVEL, 0));
                let eye = Vec3::new(
                    lake.x as f32 - 18.0,
                    SEA_LEVEL as f32 + 14.0,
                    lake.z as f32 - 18.0,
                );
                (eye, 45.0, -28.0)
            }
            Scene::Cave => {
                let floor = self.cave_floor();
                (floor.as_vec3() + Vec3::new(-2.5, 1.7, -2.5), 45.0, -15.0)
            }
            Scene::Mobs | Scene::Inspector => {
                let (x, z) = MOB_SPOT;
                let g = self.ground(x, z) as f32;
                (
                    Vec3::new(x as f32 - 7.0, g + 5.0, z as f32 - 7.0),
                    45.0,
                    -22.0,
                )
            }
            Scene::Inventory => (Vec3::new(0.5, spawn + 20.0, 0.5), 45.0, -20.0),
        }
    }

    /// Les blocs à poser pour la scène (une fois le terrain chargé).
    fn build_scene(&mut self, scene: Scene) {
        let torch = self.registry.id("core:torch");
        let glow = self.registry.id("core:glowstone");
        let stone = self.registry.id("core:stone");
        match scene {
            Scene::Torches => {
                for (dx, dz) in [(-6, -6), (6, -6), (-6, 6), (6, 6), (0, 0)] {
                    let (x, z) = (20 + dx, 20 + dz);
                    let pos = IVec3::new(x, self.ground(x, z) + 1, z);
                    let block = if (dx, dz) == (0, 0) { glow } else { torch };
                    if let Some(block) = block {
                        self.streamer_mut().set_block(pos, block);
                    }
                }
            }
            Scene::Cave => {
                // Une salle de 7 × 3 × 7 creusée sous terre, fermée de pierre de tous
                // côtés (une grotte naturelle pourrait la percer), avec une torche au milieu.
                let floor = self.cave_floor();
                for x in -4..=4 {
                    for z in -4..=4 {
                        for y in -1..=3 {
                            let pos = floor + IVec3::new(x, y, z);
                            let wall = x.abs() == 4 || z.abs() == 4 || y == -1 || y == 3;
                            let block = if wall { stone } else { Some(BlockId::AIR) };
                            if let Some(block) = block {
                                self.streamer_mut().set_block(pos, block);
                            }
                        }
                    }
                }
                if let Some(torch) = torch {
                    self.streamer_mut()
                        .set_block(floor + IVec3::new(1, 0, 1), torch);
                }
            }
            Scene::Mobs => {
                // Quelques créatures posées devant la caméra (en plus de celles du hasard).
                let registry = self
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .clone();
                let (x, z) = MOB_SPOT;
                for (i, (dx, dz)) in [(0, 0), (2, -1), (-1, 2), (3, 2), (1, 4)]
                    .into_iter()
                    .enumerate()
                {
                    let key = if i % 2 == 0 {
                        "core:pig"
                    } else {
                        "example:slime"
                    };
                    let Some(kind) = registry.id(key) else {
                        continue;
                    };
                    let (px, pz) = (x + dx, z + dz);
                    let y = self.ground(px, pz) as f32 + 1.0;
                    crate::mobs::spawn_mob(
                        &mut self.ecs,
                        kind,
                        Vec3::new(px as f32 + 0.5, y, pz as f32 + 0.5),
                    );
                }
            }
            Scene::Spawn | Scene::Sunset | Scene::Lake | Scene::Inventory | Scene::Inspector => {}
        }
    }

    /// Le sol de la salle souterraine : 20 blocs sous la surface du point (0, 0).
    fn cave_floor(&self) -> IVec3 {
        IVec3::new(0, (self.ground(0, 0) - 20).max(4), 0)
    }

    /// Un point d'eau profond près du départ, en tournant en spirale.
    fn find_lake(&self) -> Option<IVec3> {
        (1..40).find_map(|ring| {
            let r = ring * 16;
            (-r..=r).step_by(16).find_map(|d| {
                [(d, -r), (d, r), (-r, d), (r, d)]
                    .into_iter()
                    .find(|&(x, z)| self.ground(x, z) < SEA_LEVEL - 3)
                    .map(|(x, z)| IVec3::new(x, SEA_LEVEL, z))
            })
        })
    }
}

/// Écrit une image RGBA en PNG.
fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::create(path).with_context(|| format!("création de {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    // Le canal alpha n'a pas de sens pour une capture d'écran : tout opaque.
    let mut pixels = rgba.to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    writer.write_image_data(&pixels)?;
    Ok(())
}

fn tour_index(tour: &Option<Tour>) -> usize {
    tour.as_ref().map_or(usize::MAX, |t| t.index)
}
