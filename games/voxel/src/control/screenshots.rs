//! Le mode `VOXEL_SCREENSHOTS=dossier` : une visite guidée qui enregistre des images.
//!
//! La caméra se place tour à tour à midi, au coucher du soleil, la nuit, près de
//! torches, au-dessus d'un lac, dans une grotte éclairée par une torche, puis au-dessus
//! de chaque biome et d'une coupe du sous-sol. Pour chaque vue, on attend que le monde
//! soit chargé et maillé, puis on dessine l'image hors écran (`Renderer::capture`) et on
//! l'écrit en PNG. Ça marche même écran éteint, et ça permet de vérifier le rendu sans
//! être devant la machine.
//!
//! Les positions des scènes sont relatives au point d'apparition (le centre du monde
//! peut être en pleine mer). `VOXEL_SHOTS=biome` ne garde que les captures dont le nom
//! contient « biome ».

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context as _, Result};
use engine_core::Context;
use engine_voxel::{BlockId, SEA_LEVEL};
use glam::{IVec3, Vec3};

use crate::VoxelGame;
use voxel_sim::player::MoveMode;
use voxel_sim::world::daytime::TimeSpeed;

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
    /// Les écrans d'artisanat : l'établi, le four (en marche), un coffre rempli.
    Workbench,
    Furnace,
    Chest,
    /// L'échange avec un habitant (un fermier), de l'or et du blé dans l'inventaire.
    Trade,
    /// Les formes : une rangée de pièces taillées dans le monde, puis l'écran de l'établi de taille
    /// (voir `control/shapes_scene.rs`).
    Shapes,
    ShapingTable,
    Inspector,
    Machines,
    /// Au-dessus d'un biome (son identifiant).
    Biome(&'static str),
    /// Dans une grotte d'un biome souterrain (génération v3).
    CaveBiome(&'static str),
    /// Une fosse creusée jusqu'au socle : les couches, les minerais, les grottes.
    Underground,
    /// Le bras à la première personne : en marchant les mains vides, en minant à la
    /// pioche, en tenant un bloc.
    ArmWalk,
    ArmMine,
    ArmTool,
    ArmSword,
    ArmBlock,
    /// Les menus, par-dessus le monde.
    Menu(MenuShot),
    /// Les fluides : une cascade, un tube en U, l'eau contre la lave, un lac qui se vide.
    Fluid(FluidShot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FluidShot {
    Waterfall,
    UTube,
    WaterLava,
    Lake,
}

impl FluidShot {
    /// Où construire la scène (en coordonnées de scène).
    fn spot(self) -> (i32, i32) {
        match self {
            Self::Waterfall => (-40, 0),
            Self::UTube => (-40, 30),
            Self::WaterLava => (-40, 60),
            Self::Lake => (-40, 90),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuShot {
    Main,
    Worlds,
    NewWorld,
    Options,
    Pause,
}

struct Shot {
    name: &'static str,
    time: f32,
    scene: Scene,
}

/// Taille de la fosse de la coupe du sous-sol (x, z).
const PIT: (i32, i32) = (24, 16);

/// Où construire le mécanisme de la capture.
const MACHINE_SPOT: (i32, i32) = (-10, 14);

/// Une capture par biome.
const fn biome(name: &'static str, key: &'static str) -> Shot {
    Shot {
        name,
        time: 0.42,
        scene: Scene::Biome(key),
    }
}

const fn cave_biome(name: &'static str, key: &'static str) -> Shot {
    Shot {
        name,
        time: 0.42,
        scene: Scene::CaveBiome(key),
    }
}

const fn menu(name: &'static str, which: MenuShot) -> Shot {
    Shot {
        name,
        time: 0.3,
        scene: Scene::Menu(which),
    }
}

const fn fluid(name: &'static str, which: FluidShot) -> Shot {
    Shot {
        name,
        time: 0.45,
        scene: Scene::Fluid(which),
    }
}

const SHOTS: [Shot; 65] = [
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
        name: "8b-etabli",
        time: 0.45,
        scene: Scene::Workbench,
    },
    Shot {
        name: "8c-four",
        time: 0.45,
        scene: Scene::Furnace,
    },
    Shot {
        name: "8d-coffre",
        time: 0.45,
        scene: Scene::Chest,
    },
    Shot {
        name: "8e-echange",
        time: 0.45,
        scene: Scene::Trade,
    },
    Shot {
        name: "8f-formes",
        time: 0.45,
        scene: Scene::Shapes,
    },
    Shot {
        name: "8g-etabli-de-taille",
        time: 0.45,
        scene: Scene::ShapingTable,
    },
    Shot {
        name: "9-inspecteur",
        time: 0.45,
        scene: Scene::Inspector,
    },
    Shot {
        name: "10-machines",
        time: 0.8,
        scene: Scene::Machines,
    },
    Shot {
        name: "11-sous-sol",
        time: 0.5,
        scene: Scene::Underground,
    },
    Shot {
        name: "12-bras-marche",
        time: 0.45,
        scene: Scene::ArmWalk,
    },
    Shot {
        name: "13-bras-minage",
        time: 0.45,
        scene: Scene::ArmMine,
    },
    Shot {
        name: "14-bras-bloc",
        time: 0.3,
        scene: Scene::ArmBlock,
    },
    Shot {
        name: "14-bras-outil-repos",
        time: 0.45,
        scene: Scene::ArmTool,
    },
    Shot {
        name: "14-bras-epee",
        time: 0.45,
        scene: Scene::ArmSword,
    },
    menu("15-menu-principal", MenuShot::Main),
    menu("16-menu-mondes", MenuShot::Worlds),
    menu("17-menu-nouveau-monde", MenuShot::NewWorld),
    menu("18-menu-options", MenuShot::Options),
    menu("19-menu-pause", MenuShot::Pause),
    fluid("20-fluide-cascade", FluidShot::Waterfall),
    fluid("21-fluide-tube-en-u", FluidShot::UTube),
    fluid("22-fluide-eau-lave", FluidShot::WaterLava),
    fluid("23-fluide-lac-qui-se-vide", FluidShot::Lake),
    biome("biome-plaine", "core:plains"),
    biome("biome-foret", "core:forest"),
    biome("biome-foret-bouleaux", "core:birch_forest"),
    biome("biome-taiga", "core:taiga"),
    biome("biome-toundra", "core:snowy_tundra"),
    biome("biome-jungle", "core:jungle"),
    biome("biome-savane", "core:savanna"),
    biome("biome-desert", "core:desert"),
    biome("biome-marais", "core:swamp"),
    biome("biome-montagnes", "core:mountains"),
    biome("biome-pics", "core:snowy_peaks"),
    biome("biome-plage", "core:beach"),
    biome("biome-riviere", "core:river"),
    biome("biome-ocean", "core:ocean"),
    biome("biome-ocean-profond", "core:deep_ocean"),
    // La génération v3, vague 1 (ces biomes n'existent pas dans les mondes v1).
    biome("biome-sequoias", "core:redwood_forest"),
    biome("biome-bayou", "core:bayou"),
    biome("biome-lavande", "core:lavender_field"),
    biome("biome-prairie-seche", "core:prairie"),
    biome("biome-verger", "core:orchard"),
    biome("biome-erables", "core:maple_woods"),
    biome("biome-toundra-rousse", "core:tundra"),
    biome("biome-sapiniere", "core:snowy_fir_forest"),
    biome("biome-hautes-terres", "core:highland"),
    biome("biome-landes", "core:moor"),
    biome("biome-escarpements", "core:crag"),
    biome("biome-volcan", "core:volcano"),
    biome("biome-plaines-volcaniques", "core:volcanic_plains"),
    biome("biome-sources-chaudes", "core:hot_springs"),
    biome("biome-maquis", "core:maquis"),
    biome("biome-desert-fleuri", "core:flowering_desert"),
    cave_biome("biome-gouffre-cristallin", "core:crystal_cavern"),
    cave_biome("biome-grotte-luminescente", "core:glowing_grotto"),
    cave_biome("biome-grotte-fongique", "core:fungal_cave"),
];

/// Où en est la visite.
pub struct Tour {
    dir: PathBuf,
    index: usize,
    stage: Stage,
    started: Instant,
    /// `VOXEL_SHOTS` : ne garder que les captures dont le nom contient ce texte.
    filter: Option<String>,
    taken: usize,
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
            filter: std::env::var("VOXEL_SHOTS").ok(),
            taken: 0,
        }
    }
}

impl VoxelGame {
    /// Une étape de la visite, à chaque image.
    pub(crate) fn run_tour(&mut self, ctx: &mut Context) -> Result<()> {
        let Some(tour) = self.client.tour.as_mut() else {
            return Ok(());
        };
        // Les captures écartées par `VOXEL_SHOTS` sont sautées.
        while let Some(shot) = SHOTS.get(tour.index)
            && tour
                .filter
                .as_deref()
                .is_some_and(|f| !shot.name.contains(f))
        {
            tour.index += 1;
        }
        let Some(shot) = SHOTS.get(tour.index) else {
            log::info!(
                "{} captures enregistrées dans {} en {:.0?}",
                tour.taken,
                tour.dir.display(),
                tour.started.elapsed()
            );
            ctx.exit();
            return Ok(());
        };
        let stage = tour.stage;
        let ready = self.world_idle();
        match stage {
            Stage::Move => {
                self.server.day.time = shot.time;
                self.server.day.speed = TimeSpeed::Frozen;
                if self.client.body.mode != MoveMode::Fly {
                    self.toggle_fly();
                }
                let (eye, yaw, pitch) = self.viewpoint(shot.scene);
                self.place_eye(eye + self.origin().as_vec3());
                self.client.ui.inventory_open = matches!(
                    shot.scene,
                    Scene::Inventory
                        | Scene::Workbench
                        | Scene::Furnace
                        | Scene::Chest
                        | Scene::Trade
                        | Scene::ShapingTable
                );
                self.client.ui.screen = voxel_sim::server::screens::Screen::Inventory;
                // Le bras n'apparaît que dans ses propres captures.
                let arm = matches!(
                    shot.scene,
                    Scene::ArmWalk
                        | Scene::ArmMine
                        | Scene::ArmBlock
                        | Scene::ArmTool
                        | Scene::ArmSword
                );
                self.client.hand.visible = arm;
                self.client.hand.frozen = false;
                self.mode = match shot.scene {
                    Scene::Menu(MenuShot::Main) => {
                        crate::Mode::Title(crate::ui::menus::TitlePage::Main)
                    }
                    Scene::Menu(MenuShot::Worlds) => {
                        self.client.worlds = crate::ui::menus::saved_worlds();
                        crate::Mode::Title(crate::ui::menus::TitlePage::Worlds { confirm: None })
                    }
                    Scene::Menu(MenuShot::NewWorld) => {
                        crate::Mode::Title(crate::ui::menus::TitlePage::NewWorld {
                            name: "Ma vallée".into(),
                            seed: "montagne".into(),
                            difficulty: 1,
                            error: None,
                        })
                    }
                    Scene::Menu(MenuShot::Options) => {
                        crate::Mode::Title(crate::ui::menus::TitlePage::Options)
                    }
                    Scene::Menu(MenuShot::Pause) => {
                        crate::Mode::Paused(crate::ui::menus::PausePage::Main)
                    }
                    _ => crate::Mode::Playing,
                };
                self.client.ui.inspector_open = shot.scene == Scene::Inspector;
                self.client.camera.yaw = yaw.to_radians();
                self.client.camera.pitch = pitch.to_radians();
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
                    self.update_visibility(WIDTH as f32 / HEIGHT as f32);
                    let capture = self.client.renderer.capture(
                        &self.client.camera,
                        WIDTH,
                        HEIGHT,
                        Some(ui),
                    )?;
                    let elapsed = start.elapsed();
                    let dir = self
                        .client
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
                    if tour_index(&self.client.tour) == 0 {
                        self.measure_gpu()?;
                    }
                    if let Some(tour) = self.client.tour.as_mut() {
                        tour.index += 1;
                        tour.taken += 1;
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
        self.client
            .renderer
            .capture(&self.client.camera, 2560, 1440, None)?; // préchauffage
        let start = Instant::now();
        for _ in 0..FRAMES {
            self.client
                .renderer
                .capture(&self.client.camera, 2560, 1440, None)?;
        }
        log::info!(
            "rendu hors écran 2560 × 1440 : {:.2} ms par image (lecture des pixels comprise)",
            start.elapsed().as_secs_f32() * 1000.0 / FRAMES as f32
        );
        Ok(())
    }

    fn set_stage(&mut self, stage: Stage) {
        if let Some(tour) = self.client.tour.as_mut() {
            tour.stage = stage;
        }
    }

    /// Les scènes sont décrites autour de (0, 0) : on les place au point d'apparition.
    pub(crate) fn origin(&self) -> IVec3 {
        IVec3::new(self.server.spawn.x, 0, self.server.spawn.z)
    }

    /// La hauteur du sol, en coordonnées de scène.
    pub(crate) fn ground(&self, x: i32, z: i32) -> i32 {
        let o = self.origin();
        self.server
            .streamer()
            .generator()
            .column(o.x + x, o.z + z)
            .height
    }

    /// Pose un bloc, en coordonnées de scène.
    pub(crate) fn put(&mut self, pos: IVec3, block: BlockId) -> bool {
        let pos = pos + self.origin();
        self.server.streamer_mut().set_block(pos, block)
    }

    /// Le centre d'une zone du biome `key` (en coordonnées de scène) : une colonne dont
    /// les voisines à 24 blocs sont du même biome, pour bien le voir.
    pub(crate) fn find_biome(&self, key: &str) -> Option<(i32, i32)> {
        let o = self.origin();
        let streamer = self.server.streamer();
        let generator = streamer.generator();
        let id = generator.biomes().id(key)?;
        let inside = |x: i32, z: i32| generator.column(x, z).biome == id;
        let found = generator.find_column(o.x, o.z, 32, 40_000, |c| c.biome == id)?;
        // On cherche autour de la première trouvée un point bien à l'intérieur.
        let interior = |x: i32, z: i32| {
            [(0, 0), (24, 0), (-24, 0), (0, 24), (0, -24)]
                .iter()
                .all(|(dx, dz)| inside(x + dx, z + dz))
        };
        let center = (0..=400)
            .step_by(8)
            .flat_map(|r| {
                (-r..=r)
                    .step_by(8)
                    .flat_map(move |d| [(d, -r), (d, r), (-r, d), (r, d)])
            })
            .map(|(dx, dz)| (found.0 + dx, found.1 + dz))
            .find(|&(x, z)| interior(x, z))
            .unwrap_or(found);
        Some((center.0 - o.x, center.1 - o.z))
    }

    /// Le centre du volcan le plus proche (génération v3), en coordonnées de scène.
    fn nearest_volcano(&self) -> Option<(i32, i32)> {
        let o = self.origin();
        let sites = self
            .server
            .streamer()
            .generator()
            .volcano_sites((o.x - 6000, o.z - 6000), (o.x + 6000, o.z + 6000));
        sites
            .into_iter()
            .min_by_key(|&(x, z)| (x - o.x).pow(2) + (z - o.z).pow(2))
            .map(|(x, z)| (x - o.x, z - o.z))
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
            Scene::Inventory
            | Scene::Workbench
            | Scene::Furnace
            | Scene::Chest
            | Scene::Trade
            | Scene::ShapingTable => (Vec3::new(0.5, spawn + 20.0, 0.5), 45.0, -20.0),
            Scene::Shapes => self.shapes_viewpoint(),
            // À hauteur d'homme, face au paysage.
            Scene::Fluid(which) => {
                let (x, z) = which.spot();
                let base = self.fluid_base(x, z) as f32;
                // De côté, un peu au-dessus, pour voir dans les bassins.
                (
                    Vec3::new(x as f32 + 5.0, base + 9.0, z as f32 - 14.0),
                    90.0,
                    -25.0,
                )
            }
            Scene::Menu(_) => {
                let g = self.ground(0, 0) as f32;
                (Vec3::new(0.5, g + 30.0, 0.5), 200.0, -15.0)
            }
            Scene::ArmWalk
            | Scene::ArmMine
            | Scene::ArmBlock
            | Scene::ArmTool
            | Scene::ArmSword => {
                let g = self.ground(0, 0) as f32;
                (Vec3::new(0.5, g + 22.0, 0.5), 30.0, -15.0)
            }
            Scene::Machines => {
                let (x, z) = MACHINE_SPOT;
                let g = self.ground(x, z) as f32;
                (
                    Vec3::new(x as f32 - 5.5, g + 5.5, z as f32 - 5.5),
                    45.0,
                    -30.0,
                )
            }
            // Le volcan : de loin et d'en haut, vers le cratère du plus proche.
            Scene::Biome("core:volcano") if self.nearest_volcano().is_some() => {
                let (x, z) = self.nearest_volcano().unwrap_or_default();
                let g = self.ground(x, z) as f32;
                (
                    Vec3::new(x as f32 - 60.0, g + 42.0, z as f32 - 60.0),
                    45.0,
                    -22.0,
                )
            }
            Scene::Biome(key) => {
                let (x, z) = self.find_biome(key).unwrap_or_else(|| {
                    log::warn!("biome {key} introuvable");
                    (0, 0)
                });
                // Au-dessus de l'eau ou du sol, assez haut pour voir le paysage.
                let g = self.ground(x, z).max(SEA_LEVEL) as f32;
                (
                    Vec3::new(x as f32 - 26.0, g + 24.0, z as f32 - 26.0),
                    45.0,
                    -24.0,
                )
            }
            Scene::CaveBiome(key) => {
                let spot = self.find_cave_biome(key).unwrap_or_else(|| {
                    log::warn!("biome souterrain {key} introuvable");
                    self.cave_floor()
                });
                (spot.as_vec3() + Vec3::new(0.5, 1.6, 0.5), 45.0, -8.0)
            }
            Scene::Underground => {
                // Au-dessus du bord sud de la fosse, regardant vers le nord (-z) et vers le bas :
                // la paroi d'en face, du sol jusqu'au socle.
                let (x, z) = self.pit_spot();
                let g = self.ground(x, z) as f32;
                (
                    Vec3::new(
                        x as f32 + PIT.0 as f32 / 2.0,
                        g + 20.0,
                        (z + PIT.1 - 2) as f32,
                    ),
                    -90.0,
                    -70.0,
                )
            }
        }
    }

    /// Les blocs à poser pour la scène (une fois le terrain chargé).
    fn build_scene(&mut self, scene: Scene) {
        let torch = self.client.registry.id("core:torch");
        let glow = self.client.registry.id("core:glowstone");
        let stone = self.client.registry.id("core:stone");
        match scene {
            Scene::Torches => {
                for (dx, dz) in [(-6, -6), (6, -6), (-6, 6), (6, 6), (0, 0)] {
                    let (x, z) = (20 + dx, 20 + dz);
                    let pos = IVec3::new(x, self.ground(x, z) + 1, z);
                    let block = if (dx, dz) == (0, 0) { glow } else { torch };
                    if let Some(block) = block {
                        self.put(pos, block);
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
                                self.put(pos, block);
                            }
                        }
                    }
                }
                if let Some(torch) = torch {
                    self.put(floor + IVec3::new(1, 0, 1), torch);
                }
            }
            Scene::Mobs => {
                // Quelques créatures posées devant la caméra (en plus de celles du hasard).
                let registry = self
                    .server
                    .ecs
                    .resource::<std::sync::Arc<voxel_sim::mobs::MobRegistry>>()
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
                    let o = self.origin().as_vec3();
                    voxel_sim::mobs::spawn_mob(
                        &mut self.server.ecs,
                        kind,
                        o + Vec3::new(px as f32 + 0.5, y, pz as f32 + 0.5),
                    );
                }
            }
            Scene::Machines => self.build_machines(),
            Scene::Shapes => self.build_shapes(),
            Scene::ShapingTable => self.build_shaping_screen(),
            Scene::Inventory | Scene::Workbench | Scene::Furnace | Scene::Chest | Scene::Trade => {
                self.build_screen(scene);
            }
            Scene::Underground => self.dig_pit(),
            Scene::Fluid(which) => self.build_fluid(which),
            Scene::ArmWalk
            | Scene::ArmMine
            | Scene::ArmBlock
            | Scene::ArmTool
            | Scene::ArmSword => self.pose_arm(scene),
            Scene::Spawn
            | Scene::Sunset
            | Scene::Lake
            | Scene::Inspector
            | Scene::Biome(_)
            | Scene::CaveBiome(_)
            | Scene::Menu(_) => {}
        }
    }

    /// Remplit l'écran de la capture : une grille d'artisanat, un four en marche, un coffre.
    fn build_screen(&mut self, scene: Scene) {
        use voxel_sim::player::inventory::ItemStack;
        let items = self.client.content.items.clone();
        let stack = |key: &str, count: u32| items.id(key).map(|i| ItemStack::new(i, count));
        self.server.players.me_mut().inventory.craft = [None; 9];
        match scene {
            Scene::Inventory => {
                self.server.players.me_mut().inventory.craft[0] = stack("core:oak_log", 3);
                self.server.players.me_mut().inventory.armor[0] = stack("core:iron_helmet", 1);
                self.server.players.me_mut().inventory.armor[1] =
                    stack("core:copper_chestplate", 1);
                self.client.ui.screen = voxel_sim::server::screens::Screen::Inventory;
            }
            Scene::Workbench => {
                // Une pioche en bois : trois planches en haut, deux bâtons au milieu.
                for i in [0, 1, 2] {
                    self.server.players.me_mut().inventory.craft[i] = stack("core:oak_planks", 2);
                }
                for i in [4, 7] {
                    self.server.players.me_mut().inventory.craft[i] = stack("core:stick", 2);
                }
                self.server.players.me_mut().inventory.add(
                    items.id("core:oak_planks").unwrap(),
                    12,
                    &items,
                );
                self.client.ui.screen = voxel_sim::server::screens::Screen::Workbench;
            }
            Scene::Furnace | Scene::Chest => {
                let (x, z) = (3, 3);
                let pos = IVec3::new(x, self.ground(x, z) + 1, z) + self.origin();
                let key = if scene == Scene::Furnace {
                    "core:furnace"
                } else {
                    "core:chest"
                };
                let Some(block) = self.client.registry.id(key) else {
                    return;
                };
                self.server.streamer_mut().set_block(pos, block);
                let actions = self.server.mods.block_changed(pos);
                self.server.apply_machine_actions(actions);
                let fill: Vec<(usize, &str, u32)> = if scene == Scene::Furnace {
                    vec![(0, "core:raw_iron", 6), (1, "core:coal", 3)]
                } else {
                    vec![
                        (0, "core:oak_log", 32),
                        (1, "core:cobblestone", 64),
                        (2, "core:iron_ingot", 9),
                        (4, "core:sapphire", 3),
                        (9, "core:iron_pickaxe", 1),
                        (10, "core:golden_sword", 1),
                        (13, "core:torch", 16),
                        (20, "core:coal", 21),
                    ]
                };
                if let Some(entity) = self.server.streamer_mut().block_entity_mut(pos)
                    && let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut())
                {
                    for (slot, item, count) in fill {
                        inventory.slots[slot] = Some(engine_voxel::ItemStack {
                            item: item.into(),
                            count,
                            damage: 0,
                        });
                    }
                }
                if scene == Scene::Furnace {
                    // Quelques secondes de cuisson : un lingot sorti, le suivant en cours.
                    self.server
                        .mods
                        .machines
                        .schedule(engine_voxel::GridPos::world(pos), 1);
                    for _ in 0..3 * 300 {
                        let (actions, _) = self.server.mods.fixed_step(self.client.camera.position);
                        self.server.apply_machine_actions(actions);
                    }
                    self.client.ui.screen = voxel_sim::server::screens::Screen::Furnace(pos);
                } else {
                    self.client.ui.screen = voxel_sim::server::screens::Screen::Chest(pos);
                }
            }
            Scene::Trade => {
                // Un fermier tout près, de quoi payer certaines offres et pas d'autres.
                let registry = self
                    .server
                    .ecs
                    .resource::<std::sync::Arc<voxel_sim::mobs::MobRegistry>>()
                    .clone();
                let Some(kind) = registry.id("core:farmer") else {
                    return;
                };
                let (x, z) = (3, 3);
                let at = self.origin().as_vec3()
                    + Vec3::new(
                        x as f32 + 0.5,
                        self.ground(x, z) as f32 + 1.0,
                        z as f32 + 0.5,
                    );
                let Some(mob) = voxel_sim::mobs::spawn_mob(&mut self.server.ecs, kind, at) else {
                    return;
                };
                let uses: Vec<u32> = self
                    .server
                    .trades
                    .for_mob("core:farmer")
                    .iter()
                    .enumerate()
                    .map(|(i, t)| if i == 2 { 0 } else { t.uses })
                    .collect();
                self.server
                    .ecs
                    .insert(mob, voxel_sim::components::Trader { uses });
                self.server.players.me_mut().inventory.add(
                    items.id("core:gold_ingot").unwrap(),
                    3,
                    &items,
                );
                self.server.players.me_mut().inventory.add(
                    items.id("core:wheat").unwrap(),
                    45,
                    &items,
                );
                voxel_sim::net::entities::NetIds::assign(&mut self.server.ecs);
                if let Some(id) = self
                    .server
                    .ecs
                    .get::<voxel_sim::net::entities::NetId>(mob)
                    .map(|id| *id)
                {
                    self.client.ui.screen = voxel_sim::server::screens::Screen::Trade(id);
                }
                self.client.ui.trade_selected = 4;
            }
            _ => {}
        }
    }

    /// L'altitude des constructions de fluides : au-dessus du plus haut sol de la zone.
    fn fluid_base(&self, x: i32, z: i32) -> i32 {
        (-2..14)
            .flat_map(|dx| (-2..6).map(move |dz| (dx, dz)))
            .map(|(dx, dz)| self.ground(x + dx, z + dz))
            .max()
            .unwrap_or(SEA_LEVEL)
            + 14
    }

    /// Construit une scène de fluides et fait couler quelques secondes.
    fn build_fluid(&mut self, which: FluidShot) {
        let (x, z) = which.spot();
        let o = self.origin() + IVec3::new(x, self.fluid_base(x, z), z);
        let id = |key: &str| self.client.registry.id(key).unwrap_or(BlockId::AIR);
        let (stone, glass, water, lava) = (
            id("core:stone"),
            id("core:glass"),
            id("core:water"),
            id("core:lava"),
        );
        let full = engine_voxel::FLUID_FULL as u16;
        let put = |game: &mut Self, p: IVec3, b: BlockId| {
            game.server.streamer_mut().set_block(o + p, b);
        };
        let steps = match which {
            FluidShot::Waterfall => {
                // Un bassin perché à 8 blocs de haut, plein d'eau (finie), dont un côté
                // s'ouvre : l'eau tombe en cascade sur une dalle de pierre et s'y étale.
                for dx in 0..14 {
                    for dz in -3..6 {
                        put(self, IVec3::new(dx, -1, dz), stone);
                    }
                }
                for dx in -1..=4 {
                    for dz in -1..=3 {
                        put(self, IVec3::new(dx, 7, dz), stone);
                        if dx == -1 || dz == -1 || dz == 3 {
                            put(self, IVec3::new(dx, 8, dz), stone);
                            put(self, IVec3::new(dx, 9, dz), stone);
                        }
                    }
                }
                for dx in 0..=4 {
                    for dz in 0..3 {
                        for dy in 8..10 {
                            self.server.streamer_mut().set_fluid(
                                o + IVec3::new(dx, dy, dz),
                                water,
                                Some(full),
                            );
                            self.server.fluids.wake(o + IVec3::new(dx, dy, dz));
                        }
                    }
                }
                25
            }
            FluidShot::UTube => {
                // Un tube en U en verre : deux puits de 8 blocs reliés par le fond.
                for dx in -1..=7 {
                    for dz in -1..=1 {
                        for dy in -1..=9 {
                            put(self, IVec3::new(dx, dy, dz), glass);
                        }
                    }
                }
                for dy in 0..9 {
                    put(self, IVec3::new(0, dy, 0), BlockId::AIR);
                    put(self, IVec3::new(6, dy, 0), BlockId::AIR);
                }
                for dx in 0..=6 {
                    put(self, IVec3::new(dx, 0, 0), BlockId::AIR);
                }
                for dy in 0..9 {
                    self.server.streamer_mut().set_fluid(
                        o + IVec3::new(0, dy, 0),
                        water,
                        Some(full),
                    );
                    self.server.fluids.wake(o + IVec3::new(0, dy, 0));
                }
                for dx in 1..4 {
                    self.server.streamer_mut().set_fluid(
                        o + IVec3::new(dx, 0, 0),
                        water,
                        Some(full),
                    );
                }
                2500
            }
            FluidShot::WaterLava => {
                // Un bassin de 10 × 3 : de l'eau à gauche, de la lave à droite.
                for dx in -1..=10 {
                    for dz in -1..=3 {
                        put(self, IVec3::new(dx, -1, dz), stone);
                        if dx == -1 || dx == 10 || dz == -1 || dz == 3 {
                            put(self, IVec3::new(dx, 0, dz), stone);
                            put(self, IVec3::new(dx, 1, dz), stone);
                        }
                    }
                }
                for dz in 0..3 {
                    for dx in 0..3 {
                        self.server.streamer_mut().set_fluid(
                            o + IVec3::new(dx, 0, dz),
                            water,
                            Some(full),
                        );
                        self.server.fluids.wake(o + IVec3::new(dx, 0, dz));
                        self.server.streamer_mut().set_fluid(
                            o + IVec3::new(9 - dx, 0, dz),
                            lava,
                            Some(full),
                        );
                        self.server.fluids.wake(o + IVec3::new(9 - dx, 0, dz));
                    }
                }
                150
            }
            FluidShot::Lake => {
                // Un lac fini de 10 × 6 sur 2 de profondeur, et un trou dans le fond qui
                // le vide vers un bassin plus bas.
                for dx in -1..=10 {
                    for dz in -1..=6 {
                        put(self, IVec3::new(dx, 2, dz), stone);
                        if dx == -1 || dx == 10 || dz == -1 || dz == 6 {
                            for dy in 3..6 {
                                put(self, IVec3::new(dx, dy, dz), stone);
                            }
                        }
                        put(self, IVec3::new(dx, -3, dz), stone);
                    }
                }
                for dx in 0..10 {
                    for dz in 0..6 {
                        for dy in 3..5 {
                            self.server.streamer_mut().set_fluid(
                                o + IVec3::new(dx, dy, dz),
                                water,
                                Some(full),
                            );
                        }
                    }
                }
                put(self, IVec3::new(5, 2, 3), BlockId::AIR);
                self.server.fluids.wake(o + IVec3::new(5, 2, 3));
                self.server.fluids.wake(o + IVec3::new(5, 3, 3));
                60
            }
        };
        let start = Instant::now();
        let mut processed = 0;
        for _ in 0..steps {
            let report = self.server.fluids.step(
                &mut self.server.world.borrow_mut(),
                std::time::Duration::from_millis(50),
            );
            processed += report.processed;
        }
        log::info!(
            "fluides ({which:?}) : {steps} étapes, {processed} cellules calculées en {:.1} ms, {} encore actives",
            start.elapsed().as_secs_f64() * 1000.0,
            self.server.fluids.active()
        );
    }

    /// Une pose du bras, figée pour la capture.
    fn pose_arm(&mut self, scene: Scene) {
        let items = self.client.content.items.clone();
        let wanted = match scene {
            Scene::ArmMine | Scene::ArmTool => items.id("core:stone_pickaxe"),
            Scene::ArmSword => items.id("core:iron_sword"),
            Scene::ArmBlock => items.id("core:oak_log"),
            _ => None,
        };
        // La case de la barre qui contient l'objet voulu (ou une case vide).
        let slot = (0..voxel_sim::player::inventory::HOTBAR)
            .find(|&i| self.server.players.me_mut().inventory.slots[i].map(|s| s.item) == wanted)
            .unwrap_or(0);
        if wanted.is_none() {
            self.server.players.me_mut().inventory.slots[8] = None;
        } else {
            self.server.players.me_mut().inventory.slots[slot] =
                wanted.map(|item| voxel_sim::player::inventory::ItemStack::new(item, 1));
        }
        self.server.players.me_mut().inventory.selected = if wanted.is_none() { 8 } else { slot };
        self.client.hand.equip = 0.0;
        self.client.hand.swing = None;
        self.client.hand.walk_amount = 0.0;
        match scene {
            Scene::ArmWalk => {
                self.client.hand.walk_phase = 1.1;
                self.client.hand.walk_amount = 1.6;
            }
            Scene::ArmMine => self.client.hand.swing = Some(0.42),
            _ => {}
        }
        self.client.hand.frozen = true;
    }

    /// Un mécanisme sur une dalle de pierre : une horloge entourée de quatre lampes, et une
    /// seconde horloge qui alimente un casseur tourné vers une colonne de pierre.
    fn build_machines(&mut self) {
        let id = |key: &str| self.client.registry.id(key);
        let (Some(stone), Some(clock), Some(lamp)) =
            (id("core:stone"), id("core:clock"), id("core:lamp"))
        else {
            return;
        };
        let breaker = id("example:breaker");
        let (x, z) = MACHINE_SPOT;
        let base = IVec3::new(x, self.ground(x, z) + 1, z) + self.origin();
        let place = |game: &mut Self, pos: IVec3, block: BlockId| {
            if game.server.streamer_mut().set_block(pos, block) {
                let actions = game.server.mods.block_changed(pos);
                game.server.apply_machine_actions(actions);
            }
        };
        for dx in -2..=5 {
            for dz in -2..=2 {
                place(self, base + IVec3::new(dx, -1, dz), stone);
                for dy in 0..3 {
                    place(self, base + IVec3::new(dx, dy, dz), BlockId::AIR);
                }
            }
        }
        place(self, base, clock);
        // Une horloge lente (une bascule toutes les 20 ticks, 1 s) : les lampes restent
        // allumées le temps de la capture.
        self.server.streamer_mut().set_block_entity(
            base,
            Some(engine_voxel::BlockEntity {
                data: vec![20],
                ..Default::default()
            }),
        );
        for offset in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            place(self, base + offset, lamp);
        }
        if let Some(breaker) = breaker {
            let at = base + IVec3::new(3, 0, 0);
            place(self, at, breaker);
            let actions = self
                .server
                .mods
                .dispatch(&engine_mods::ModEvent::BlockPlaced {
                    at: engine_voxel::GridPos::world(at),
                    block: breaker,
                    facing: IVec3::X,
                });
            self.server.apply_machine_actions(actions);
            place(self, at + IVec3::NEG_X, clock);
            for dy in 0..3 {
                place(self, at + IVec3::new(1, dy, 0), stone);
            }
        }
        // Une seconde et quart de machines (25 ticks) : l'horloge s'allume (tick 20),
        // ses lampes aussi.
        for _ in 0..75 {
            let (actions, _) = self.server.mods.fixed_step(self.client.camera.position);
            self.server.apply_machine_actions(actions);
        }
    }

    /// Un coin de terre ferme près du départ, loin de l'eau, pour la fosse.
    fn pit_spot(&self) -> (i32, i32) {
        let dry = |x: i32, z: i32| {
            (-4..PIT.0 + 8)
                .step_by(4)
                .flat_map(|dx| (-4..PIT.1 + 8).step_by(4).map(move |dz| (dx, dz)))
                .all(|(dx, dz)| self.ground(x + dx, z + dz) > SEA_LEVEL + 2)
        };
        (0..20)
            .flat_map(|r| (-r..=r).flat_map(move |d| [(d, -r), (d, r), (-r, d), (r, d)]))
            .map(|(i, j)| (40 + i * 16, -30 + j * 16))
            .find(|&(x, z)| dry(x, z))
            .unwrap_or((40, -30))
    }

    /// Une fosse creusée jusqu'au socle : ses parois montrent les
    /// couches (terre, pierre, pierre profonde), les filons et les grottes coupées.
    fn dig_pit(&mut self) {
        let (x0, z0) = self.pit_spot();
        let o = self.origin();
        let top = (0..PIT.0)
            .flat_map(|dx| (0..PIT.1).map(move |dz| (dx, dz)))
            .map(|(dx, dz)| self.ground(x0 + dx, z0 + dz))
            .max()
            .unwrap_or(SEA_LEVEL);
        let mut streamer = self.server.streamer_mut();
        // Jusqu'au socle : le bas du monde (y = 0 en v1, -128 en v3), le socle lui-même gardé.
        let bottom = streamer.world().block_y_range().start + 1;
        let mut dug = 0;
        for dx in 0..PIT.0 {
            for dz in 0..PIT.1 {
                for y in (bottom..=top).rev() {
                    // `set_block_quiet` : le maillage est refait une fois, à la fin.
                    dug += streamer
                        .set_block_quiet(IVec3::new(o.x + x0 + dx, y, o.z + z0 + dz), BlockId::AIR)
                        as usize;
                }
            }
        }
        log::info!("fosse : {dug} blocs creusés depuis y = {top}");
    }

    /// Le sol de la salle souterraine : 20 blocs sous la surface du point (0, 0).
    fn cave_floor(&self) -> IVec3 {
        IVec3::new(0, (self.ground(0, 0) - 20).max(SEA_LEVEL - 30), 0)
    }

    /// Un point d'une grotte du biome souterrain `key` (en coordonnées de scène) : de l'air sur un
    /// sol, avec de la place au-dessus et autour, pour voir la salle.
    fn find_cave_biome(&self, key: &str) -> Option<IVec3> {
        let o = self.origin();
        let streamer = self.server.streamer();
        let generator = streamer.generator();
        let id = generator.biomes().id(key)?;
        let open = |chunk: &engine_voxel::Chunk, x: i32, y: i32, z: i32| {
            (0..32).contains(&x)
                && (0..32).contains(&y)
                && (0..32).contains(&z)
                && chunk.get(x as usize, y as usize, z as usize).is_air()
        };
        for ring in 0..24 {
            let r = ring * 64;
            let points: Vec<(i32, i32)> = if r == 0 {
                vec![(0, 0)]
            } else {
                (-r..=r)
                    .step_by(64)
                    .flat_map(|d| [(d, -r), (d, r), (-r, d), (r, d)])
                    .collect()
            };
            for (dx, dz) in points {
                for y in [-90, -60, -30, 0, 20] {
                    let (x, z) = (o.x + dx, o.z + dz);
                    if generator.biome_at(x, y, z) != id {
                        continue;
                    }
                    let chunk_pos = IVec3::new(x, y, z).div_euclid(IVec3::splat(32));
                    let Some(chunk) = generator.generate(chunk_pos) else {
                        continue;
                    };
                    let base = chunk_pos * 32;
                    // L'air le plus dégagé du chunk : un sol dessous, 4 blocs libres au-dessus, et
                    // de l'air à 3 blocs dans les quatre directions.
                    let spot = (2..26).find_map(|ly| {
                        (3..29).find_map(|lz| {
                            (3..29).find_map(|lx| {
                                let ok = !open(&chunk, lx, ly - 1, lz)
                                    && (0..4).all(|k| open(&chunk, lx, ly + k, lz))
                                    && [(3, 0), (-3, 0), (0, 3), (0, -3)]
                                        .iter()
                                        .all(|&(ax, az)| open(&chunk, lx + ax, ly + 1, lz + az));
                                let p = base + IVec3::new(lx, ly, lz);
                                (ok && generator.biome_at(p.x, p.y, p.z) == id).then_some(p)
                            })
                        })
                    });
                    if let Some(p) = spot {
                        return Some(p - o);
                    }
                }
            }
        }
        None
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
pub(crate) fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::SHOTS;

    /// Deux vues du même nom s'écraseraient : la seconde remplacerait la première sans bruit.
    #[test]
    fn shot_names_are_unique() {
        let mut names = std::collections::BTreeSet::new();
        for shot in &SHOTS {
            assert!(names.insert(shot.name), "deux vues nommées {}", shot.name);
        }
    }
}
