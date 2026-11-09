//! Le futur mini-Minecraft.
//!
//! Étape actuelle (« mini-Minecraft v1 ») : un monde infini où l'on marche, casse et
//! pose des blocs avec un inventaire, au milieu de créatures. Le joueur et les créatures
//! sont des entités de l'ECS (`engine_ecs`). Les blocs et les créatures viennent de
//! `assets/core/` et des mods (`mods/<nom>/`).
//! - Clic gauche : capturer la souris, puis casser le bloc visé ou frapper une créature.
//!   Clic droit : poser le bloc choisi. Touches 1 à 9 : choisir la case de la barre.
//! - E : ouvrir l'inventaire (déplacer les piles à la souris). F3 : l'inspecteur.
//! - ZQSD (AZERTY) ou WASD (QWERTY) : se déplacer. Espace : sauter (en vol : monter).
//!   Maj : descendre (en vol). Ctrl gauche : courir.
//! - F : basculer entre marche et vol libre. Dans l'eau, Espace fait remonter.
//! - T : temps normal, accéléré (×40) ou figé.
//! - F1 : maillage « culling » ou « greedy ». F2 : frustum culling.
//! - F5 : recharger shaders, textures, sons et blocs (c'est aussi automatique quand
//!   un fichier change dans `assets/`, `mods/` ou le dossier des shaders).
//! - Échap : libérer la souris (deuxième Échap : quitter).
//!
//! Variables d'environnement :
//! - `VOXEL_SEED=123` : la graine du monde (42 par défaut).
//! - `VOXEL_RADIUS=24` : le rayon de vue en chunks (16 par défaut).
//! - `VOXEL_ASSETS=chemin` : le dossier `assets` (les mods sont dans `../mods`).
//! - `VOXEL_AUTOFLY=1` : vol automatique, pour mesurer le chargement.
//!   `VOXEL_AUTOFLY=100` : à 100 blocs par seconde.
//! - `VOXEL_TIME=0.75` : l'heure de départ (0 minuit, 0,25 lever, 0,5 midi, 0,75 coucher).
//! - `VOXEL_DAY_LENGTH=120` : la durée d'une journée en secondes (600 par défaut).
//! - `VOXEL_WORLD=nom` : le monde à ouvrir dans `saves/` (« world » par défaut). Il est
//!   sauvegardé toutes les 30 secondes et à la fermeture.
//! - `VOXEL_MUTE=1` : couper le son. `VOXEL_VOLUME_EFFECTS=0.8`, `VOXEL_VOLUME_AMBIENCE=0.35` :
//!   volumes des effets et de l'ambiance, de 0 à 1.
//! - `VOXEL_QUIT_AFTER=20` : quitter proprement après 20 secondes (pour les tests).
//! - `VOXEL_SCREENSHOTS=dossier` : enregistre des captures (midi, coucher, nuit, torches,
//!   lac, grotte) dans ce dossier, puis quitte. Marche même écran éteint.
//! - `VOXEL_DEMO=1` : le joueur marche, saute, casse et pose des blocs tout seul
//!   (macOS empêche les outils de test d'envoyer des touches).

mod assets;
mod audio;
mod bench;
mod components;
mod content;
mod control;
mod culling;
mod daytime;
mod drops;
mod events;
mod hand;
mod hot_reload;
mod inventory;
mod menus;
mod mobs;
mod modding;
mod persistence;
mod player;
mod screenshots;
mod settings;
mod shared;
mod stats;
mod stress;
mod ui;

use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use engine_core::winit::dpi::PhysicalSize;
use engine_core::winit::event::MouseButton;
use engine_core::winit::event::WindowEvent;
use engine_core::winit::keyboard::KeyCode;
use engine_core::{AppConfig, Context, Game};
use engine_ecs::{Entity, EventReader, Events, Schedule, Stage};
use engine_mods::ModEvent;
use engine_render::{Aabb, Camera, ColorVertex, Fog, MeshId, RenderStats, Renderer, Sun};
use engine_voxel::{
    BlockId, BlockRegistry, CHUNK_SIZE, ChunkMesh, ChunkStreamer, GridPos, MachineAction,
    MeshVertex, RayHit, StreamEvent, StreamingConfig, TerrainGenerator, raycast,
};
use glam::{IVec3, Vec2, Vec3};

use components::{Collider, Health, Transform, Velocity};
use daytime::DayCycle;
use events::{BlockBroken, BlockPlaced, EntityDied, PlayerDamaged};
use inventory::Inventory;
use mobs::MobRegistry;
use player::{FLY_SPEED, FLY_SPRINT_FACTOR, MoveMode, SPRINT_SPEED, WALK_SPEED};
use settings::Action;
use shared::StreamerExt;
use stats::FrameStats;

const MOUSE_SENSITIVITY: f32 = 0.0025;
const DEFAULT_SEED: u32 = 42;
/// Maillages envoyés au GPU au maximum par image. Chaque envoi crée deux tampons :
/// les étaler sur plusieurs images évite une saccade quand 50 chunks arrivent d'un coup.
/// Les maillages d'un bloc modifié passent en tête de file : ils sont envoyés tout de suite.
const UPLOAD_BUDGET: usize = 16;
/// Hauteur de vol au départ et en mode `VOXEL_AUTOFLY`, au-dessus du sol.
const FLY_ALTITUDE: f32 = 30.0;
/// Portée du joueur, en blocs.
const REACH: f32 = 6.0;
/// Dégâts d'un coup de poing.
const PUNCH_DAMAGE: f32 = 4.0;
const HOTBAR_KEYS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

struct VoxelGame {
    renderer: Renderer,
    camera: Camera,
    mouse_captured: bool,
    /// Le monde ECS : le joueur, les créatures, et des ressources partagées par les
    /// systèmes (le streamer des chunks, le registre des créatures, les événements).
    ecs: engine_ecs::World,
    schedule: Schedule,
    registry: Arc<BlockRegistry>,
    /// Les objets, recettes, fonte et butin.
    content: content::Content,
    /// Le bloc en cours de minage (bouton gauche maintenu) et l'avancement, de 0 à 1.
    mining: Option<(IVec3, f32)>,
    /// L'état du hasard du jeu (butin, objets lâchés).
    random: u64,
    /// Le bras à la première personne.
    hand: hand::Hand,
    /// L'eau et la lave qui coulent.
    fluids: engine_voxel::FluidSim,
    /// Pas de simulation depuis la dernière étape des fluides (une étape tous les 6 pas :
    /// 10 par seconde).
    fluid_steps: u32,
    /// Le bilan de la dernière étape des fluides.
    fluid_report: engine_voxel::FluidReport,
    /// Menu principal, jeu ou pause.
    mode: Mode,
    settings: settings::Settings,
    /// Ce que le jeu demande à l'application (nouveau monde, menu, quitter).
    request: Option<Request>,
    /// Les options attendent une touche pour cette action.
    waiting_key: Option<settings::Action>,
    /// Les mondes sauvegardés, listés à l'ouverture de l'écran de choix.
    worlds: Vec<menus::SavedWorld>,
    /// Le monde de blocs, partagé avec l'ECS (ressource `SharedStreamer`) et les mods.
    world: Rc<RefCell<ChunkStreamer>>,
    /// Les machines et les scripts des mods.
    mods: modding::ModSystem,
    /// Les dossiers des mods, dans l'ordre de chargement.
    mod_dirs: Vec<(String, PathBuf)>,
    /// Les lecteurs d'événements qui les transmettent aux mods.
    mod_readers: EventReaders,
    /// `VOXEL_STRESS=N` : le test de charge.
    stress: Option<stress::StressTest>,
    /// Le point d'apparition (et de réapparition) : la surface d'une colonne de terre ferme.
    spawn: IVec3,
    /// Le maillage GPU de chaque chunk affiché.
    meshes: HashMap<IVec3, MeshId>,
    /// L'entité du joueur dans `ecs`.
    player: Entity,
    inventory: Inventory,
    /// Les pixels des textures de blocs (pour les icônes de l'inventaire).
    block_textures: Vec<Vec<u8>>,
    ui: ui::GameUi,
    debug: ui::DebugInfo,
    /// Le jeu s'abonne lui-même à ses événements (pour les sons) : un lecteur par type.
    readers: EventReaders,
    /// Le bloc visé, mis à jour à chaque image.
    target: Option<RayHit>,
    /// `Some(vitesse)` en mode vol automatique.
    autofly: Option<f32>,
    demo: Option<Demo>,
    /// `Some` en mode `VOXEL_SCREENSHOTS` : la visite guidée en cours.
    tour: Option<screenshots::Tour>,
    day: DayCycle,
    /// La sauvegarde (absente en mode démo, vol automatique ou captures, sauf si
    /// `VOXEL_WORLD` est donné : ces modes modifient le monde).
    store: Option<Arc<engine_voxel::SaveStore>>,
    last_autosave: Instant,
    save_started: Instant,
    /// Le lancement du jeu (pour animer le vent).
    started: Instant,
    /// `VOXEL_QUIT_AFTER=secondes` : quitter proprement (avec sauvegarde) après ce délai.
    quit_at: Option<Instant>,
    /// La surveillance des fichiers (shaders, textures, sons, blocs).
    hot_reload: Option<hot_reload::HotReload>,
    /// `None` si le son est coupé ou sans carte son.
    audio: Option<audio::Audio>,
    /// Distance marchée depuis le dernier bruit de pas.
    step_distance: f32,
    was_in_liquid: bool,
    frame_stats: FrameStats,
    last_render: RenderStats,
    /// `VOXEL_CONTROL` : la marche et les `wait` commandés (voir `control.rs`).
    remote: control::Remote,
    /// Le culling des grottes : quels chunks sont visibles depuis la caméra.
    culling: culling::CaveCulling,
    /// `VOXEL_BENCH` : le benchmark reproductible (voir `bench.rs`).
    bench: Option<bench::Bench>,
}

/// Comment lancer un monde : depuis quel dossier, avec quelle graine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launch {
    /// Le dossier de sauvegarde (`None` : un monde jetable, rien n'est écrit).
    pub world: Option<PathBuf>,
    /// La graine d'un nouveau monde (un monde existant garde la sienne).
    pub seed: Option<u32>,
    /// Le monde de fond du menu principal : survol de caméra, sans joueur.
    pub title: bool,
}

impl Launch {
    /// Les modes automatiques (captures, démo, vol, test de charge) et `VOXEL_WORLD`
    /// lancent un monde directement, sans menu, comme avant.
    fn from_env() -> Option<Self> {
        let automatic = [
            "VOXEL_DEMO",
            "VOXEL_AUTOFLY",
            "VOXEL_SCREENSHOTS",
            "VOXEL_STRESS",
            "VOXEL_BENCH",
        ]
        .iter()
        .any(|v| std::env::var(v).is_ok());
        let named = std::env::var("VOXEL_WORLD").is_ok();
        if !automatic && !named {
            return None;
        }
        Some(Self {
            // Les modes automatiques n'écrivent pas dans le monde du joueur, sauf demande explicite.
            world: named.then(|| persistence::world_dir(&assets_dir())),
            seed: env_parse("VOXEL_SEED"),
            title: false,
        })
    }

    /// Le monde de fond du menu principal.
    pub fn title() -> Self {
        Self {
            world: None,
            seed: Some(TITLE_SEED),
            title: true,
        }
    }
}

/// La graine du monde de fond du menu.
const TITLE_SEED: u32 = 2024;

/// Ce que le jeu demande à l'application : changer de monde ou quitter.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Play(Launch),
    /// Retour au menu principal (le monde est sauvegardé et déchargé).
    Title,
    Quit,
}

/// Où en est le jeu : le menu principal (un monde de fond), le jeu, ou la pause.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Title(menus::TitlePage),
    Playing,
    Paused(menus::PausePage),
}

impl VoxelGame {
    /// Crée un monde. Le renderer est prêté par l'application : il passe d'un monde à
    /// l'autre (on ne recrée pas la fenêtre ni le GPU).
    fn create(
        ctx: &mut Context,
        mut renderer: Renderer,
        launch: Launch,
        settings: settings::Settings,
    ) -> Result<Self> {
        renderer.crosshair = !launch.title;

        // L'ordre des mods (chacun après ses dépendances) fixe l'ordre des registres.
        let assets = assets_dir();
        let mod_order = modding::resolve_order(&assets);
        let mod_dirs = modding::mod_dirs(&mod_order);
        let registry = Arc::new(load_registry(&mod_dirs)?);
        let block_textures = assets::load_textures(&registry);
        renderer.set_block_textures(&block_textures);
        let mob_registry =
            MobRegistry::load(&assets, &mod_dirs, &registry).context("chargement des créatures")?;
        let content = content::Content::load(&registry, &assets, &mod_dirs)
            .context("chargement des objets")?;
        log::info!("{} types de créatures", mob_registry.len());

        let (store, meta) = match &launch.world {
            Some(dir) => {
                let (store, meta) = persistence::open_world(dir.clone())?;
                (Some(store), meta)
            }
            None => {
                log::info!("monde sans sauvegarde");
                (None, None)
            }
        };
        if let Some(meta) = &meta {
            persistence::check_mods(meta, &registry);
        }

        // Un monde existant garde sa graine.
        let seed = meta
            .as_ref()
            .map(|m| m.seed)
            .or(launch.seed)
            .unwrap_or(DEFAULT_SEED);
        let config = StreamingConfig {
            view_radius: env_parse("VOXEL_RADIUS").unwrap_or(settings.render_distance.clamp(4, 32)),
            // `VOXEL_LEAF_CULLING=0` : garder toutes les faces de feuilles, pour comparer.
            leaf_culling: std::env::var("VOXEL_LEAF_CULLING").map_or(true, |v| v != "0"),
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
        let generator = TerrainGenerator::with_registry(seed, registry.clone())?;
        let mut streamer = ChunkStreamer::new(generator, config);
        // Les fluides : ceux du jeu et des mods (viscosité, densité...), pour la nage et
        // la simulation.
        let owned: Vec<(String, PathBuf)> =
            std::iter::once(("core".to_string(), assets.join("core")))
                .chain(mod_dirs.iter().cloned())
                .collect();
        let fluid_dirs: Vec<(String, &Path)> = owned
            .iter()
            .map(|(ns, dir)| (ns.clone(), dir.as_path()))
            .collect();
        let fluid_registry = Arc::new(
            engine_voxel::FluidRegistry::load(&fluid_dirs).unwrap_or_else(|err| {
                log::error!("fluides : {err:#}");
                engine_voxel::FluidRegistry::builtin()
            }),
        );
        streamer.set_fluids(fluid_registry.clone());
        let fluids = engine_voxel::FluidSim::new(&registry, &fluid_registry);
        if let Some(store) = &store {
            streamer = streamer.with_save(store.clone());
        }

        // `std::env::var` renvoie `Err` si la variable n'existe pas : `.ok()` en fait un `Option`.
        let autofly = std::env::var("VOXEL_AUTOFLY")
            .ok()
            .filter(|_| !launch.title)
            .map(|v| {
                let speed: f32 = v.parse().unwrap_or(0.0);
                // `VOXEL_AUTOFLY=1` veut dire « oui », pas « 1 bloc par seconde ».
                if speed <= 1.0 {
                    FLY_SPEED * FLY_SPRINT_FACTOR
                } else {
                    speed
                }
            });
        // Le menu principal : un lent survol du monde de fond.
        let autofly = if launch.title {
            Some(TITLE_FLY_SPEED)
        } else {
            autofly
        };
        let automatic = !launch.title;
        let demo = std::env::var("VOXEL_DEMO")
            .ok()
            .filter(|_| automatic)
            .map(|_| Demo::new());
        let tour = std::env::var("VOXEL_SCREENSHOTS")
            .ok()
            .filter(|_| automatic)
            .map(|dir| screenshots::Tour::new(dir.into()));

        // On démarre sur la terre ferme la plus proche de l'origine (elle peut être en
        // pleine mer), au-dessus du sol. En marche, le joueur tombe jusqu'au sol dès que
        // le terrain est chargé.
        let (spawn_x, spawn_z) = streamer.generator().find_spawn();
        let spawn = IVec3::new(
            spawn_x,
            streamer.generator().column(spawn_x, spawn_z).height,
            spawn_z,
        );
        log::info!("point d'apparition : ({spawn_x}, {spawn_z}), biome {}", {
            let generator = streamer.generator();
            generator
                .biome(generator.column(spawn_x, spawn_z).biome)
                .name
                .clone()
        });
        let ground = spawn.y as f32;
        let mode = if autofly.is_some() {
            MoveMode::Fly
        } else {
            MoveMode::Walk
        };
        let altitude = if mode == MoveMode::Walk {
            3.0
        } else {
            FLY_ALTITUDE
        };
        let default_view = (
            Vec3::new(
                spawn.x as f32 + 0.5,
                ground + altitude,
                spawn.z as f32 + 0.5,
            ),
            45f32.to_radians(),
            if demo.is_some() { -35f32 } else { -20f32 }.to_radians(),
            mode,
        );
        // Un monde sauvegardé reprend là où on l'a laissé.
        let (eye, yaw, pitch, mode) = persistence::saved_eye(meta.as_ref()).unwrap_or(default_view);
        // Le monde ECS : le streamer et les registres deviennent des ressources, que les
        // systèmes (physique, IA) lisent sans que le jeu ait à les leur passer.
        let world = Rc::new(RefCell::new(streamer));
        let mut mods = modding::ModSystem::new(&mod_order, world.clone(), registry.clone());
        // Le coffre et le four : des machines natives qui ont besoin des objets et de la fonte.
        engine_voxel::register_containers(
            &mut mods.machines,
            &registry,
            content.items.clone(),
            content.smelting.clone(),
        );
        let mut ecs = engine_ecs::World::new();
        events::register(&mut ecs);
        ecs.insert_resource(shared::SharedStreamer(world.clone()));
        ecs.insert_resource(Arc::new(mob_registry));
        ecs.insert_resource(mobs::FixedDt(ctx.fixed_dt));
        ecs.insert_resource(mobs::GameRng(0x2545_F491_4F6C_DD1D ^ seed as u64));
        ecs.insert_resource(mobs::SimClock::default());
        let player = player::spawn(&mut ecs, eye, mode);
        ecs.insert_resource(mobs::PlayerEntity(player));
        let mut schedule = Schedule::new();
        schedule
            .add(Stage::FixedUpdate, "tick", mobs::tick)
            .add(Stage::FixedUpdate, "créatures : apparition", mobs::spawn)
            .add(Stage::FixedUpdate, "créatures : IA", mobs::wander)
            .add(Stage::FixedUpdate, "physique", mobs::physics)
            .add(Stage::FixedUpdate, "créatures : attaque", mobs::attack)
            .add(Stage::FixedUpdate, "créatures : mort", mobs::cleanup);
        let readers = EventReaders::new(&ecs);
        let mod_readers = EventReaders::new(&ecs);

        let camera = Camera {
            position: player::eye(&ecs, player, 1.0),
            yaw,
            pitch: if launch.title { -0.25 } else { pitch },
            far: view_distance * 1.5,
            fov_y: settings.fov.to_radians(),
            ..Camera::default()
        };
        renderer.set_vsync(settings.vsync);
        // L'inventaire sauvegardé, ou un petit nécessaire de départ pour un nouveau monde.
        let items = &content.items;
        let inventory = match meta.as_ref() {
            Some(meta) => match &meta.items {
                Some(saved) => Inventory::from_save(saved, items),
                // Un monde d'avant les objets : son inventaire de blocs.
                None if !meta.inventory.is_empty() => {
                    Inventory::from_old_save(&meta.inventory, items)
                }
                None => starter_inventory(items),
            },
            None => starter_inventory(items),
        };
        let time = meta
            .as_ref()
            .map(|m| m.time_of_day)
            .or_else(|| env_parse("VOXEL_TIME"))
            .unwrap_or(0.3);
        if let Some(speed) = autofly {
            log::info!("vol automatique à {speed} blocs/s");
        }
        if demo.is_some() {
            log::info!("démo : le joueur marche, saute, casse et pose des blocs tout seul");
        }

        let (effects, music) = settings.volumes();
        let audio = audio::Audio::new(
            &registry,
            audio::AudioSettings {
                effects,
                ambience: music,
                ..audio::AudioSettings::from_env()
            },
        );
        let mut hand = hand::Hand::new(hand::ArmColors::load(&assets));
        hand.visible &= settings.show_arm && !launch.title;
        Ok(Self {
            renderer,
            camera,
            mouse_captured: false,
            ecs,
            schedule,
            registry,
            content,
            mining: None,
            random: 0x853C_49E6_748F_EA9B ^ seed as u64,
            hand,
            fluids,
            fluid_steps: 0,
            fluid_report: engine_voxel::FluidReport::default(),
            mode: if launch.title {
                Mode::Title(menus::TitlePage::Main)
            } else {
                Mode::Playing
            },
            settings,
            request: None,
            waiting_key: None,
            worlds: Vec::new(),
            world,
            mods,
            mod_dirs,
            mod_readers,
            stress: stress::StressTest::from_env(),
            spawn,
            meshes: HashMap::new(),
            player,
            inventory,
            block_textures,
            ui: ui::GameUi::new(&ctx.window),
            debug: ui::DebugInfo::default(),
            readers,
            target: None,
            autofly,
            demo,
            tour,
            day: DayCycle::new(time, env_parse("VOXEL_DAY_LENGTH").unwrap_or(600.0)),
            store,
            last_autosave: Instant::now(),
            save_started: Instant::now(),
            started: Instant::now(),
            hot_reload: start_hot_reload(),
            audio,
            step_distance: 0.0,
            was_in_liquid: false,
            quit_at: env_parse::<u64>("VOXEL_QUIT_AFTER")
                .map(|s| Instant::now() + std::time::Duration::from_secs(s)),
            frame_stats: FrameStats::new(),
            last_render: RenderStats::default(),
            remote: control::Remote::default(),
            culling: culling::CaveCulling::from_env(),
            bench: bench::Bench::from_env(),
        })
    }

    /// La simulation, à 60 Hz exactement : les entrées du joueur, puis les systèmes.
    fn fixed_update(&mut self, ctx: &mut Context) {
        match self.mode {
            // En pause, le monde s'arrête.
            Mode::Paused(_) => return,
            // Le monde du menu vit (créatures, machines), sans joueur aux commandes.
            Mode::Title(_) => {
                player::set_intent(&self.ecs, self.player, Vec3::ZERO, false);
                self.schedule.run(Stage::FixedUpdate, &mut self.ecs);
                self.step_fluids();
                return;
            }
            Mode::Playing => {}
        }
        // `VOXEL_CONTROL` : une marche commandée passe avant le clavier.
        let remote = self.remote_intent(ctx.fixed_dt);
        let input = &ctx.input;
        let keys = &self.settings.keys;
        let busy = self.ui.inventory_open;
        let (walk, jump) = match &self.demo {
            _ if remote.is_some() => remote.unwrap_or_default(),
            Some(_) => Demo::walk(&self.ecs, self.player, &self.camera),
            None if busy => (Vec3::ZERO, false),
            None => {
                let speed = if input.key_held(keys.key(Action::Sprint)) {
                    SPRINT_SPEED
                } else {
                    WALK_SPEED
                };
                let jump = input.key_held(keys.key(Action::Jump));
                let direction = self.wish_direction(ctx, false);
                (direction * speed, jump)
            }
        };
        player::set_intent(&self.ecs, self.player, walk, jump);
        self.schedule.run(Stage::FixedUpdate, &mut self.ecs);
        // Les machines : un tick tous les 3 pas (20 par seconde).
        let eye = player::eye(&self.ecs, self.player, 1.0);
        self.pick_up_drops(ctx.fixed_dt);
        let (actions, machine_tick) = self.mods.fixed_step(eye);
        self.apply_machine_actions(actions);
        if let Some(stress) = &mut self.stress {
            let eye = player::eye(&self.ecs, self.player, 1.0);
            let ready = self.world.borrow().is_idle();
            stress.build_if_ready(ready, &mut self.ecs, &mut self.mods, eye);
            let start = Instant::now();
            stress::conveyors(&mut self.ecs);
            stress.record(machine_tick.map(|t| t.duration), start.elapsed());
        }
        self.step_fluids();
        self.fluid_damage(ctx.fixed_dt);
        self.footsteps(ctx.fixed_dt);
        self.respawn_if_dead();
    }

    /// Une étape des fluides tous les 6 pas (10 par seconde), dans un budget de temps.
    fn step_fluids(&mut self) {
        self.fluid_steps += 1;
        if self.fluid_steps < FLUID_STEP_EVERY {
            return;
        }
        self.fluid_steps = 0;
        let report = self.fluids.step(&mut self.world.borrow_mut(), FLUID_BUDGET);
        self.fluid_report = report;
    }

    /// La lave brûle : des dégâts par seconde tant qu'on y est.
    fn fluid_damage(&mut self, dt: f32) {
        let body = player::body(&self.ecs, self.player);
        let damage = {
            let streamer = self.streamer();
            (0..2)
                .map(|dy| {
                    streamer.world().block(
                        (body.position + Vec3::Y * (0.2 + dy as f32))
                            .floor()
                            .as_ivec3(),
                    )
                })
                .filter_map(|block| self.fluids.kind_of(block))
                .map(|kind| kind.damage)
                .fold(0.0, f32::max)
        };
        if damage > 0.0
            && let Some(mut health) = self.ecs.get_mut::<Health>(self.player)
        {
            health.current = (health.current - damage * dt).max(0.0);
        }
    }

    fn window_event(&mut self, ctx: &mut Context, event: &WindowEvent) -> bool {
        self.ui.on_window_event(&ctx.window, event)
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        // Les options attendent une touche : la prochaine pressée est la nouvelle
        // (Échap annule).
        let mut escape = ctx.input.key_pressed(KeyCode::Escape);
        if let Some(action) = self.waiting_key
            && let Some(key) = ctx.input.keys_pressed().next()
        {
            if key != KeyCode::Escape {
                self.settings.keys.set(action, key);
            }
            self.waiting_key = None;
            escape = false;
        }
        let interactive = self.mode == Mode::Playing;
        match &self.mode {
            Mode::Playing if escape && !self.ui.inventory_open => {
                self.mode = Mode::Paused(menus::PausePage::Main);
                escape = false;
            }
            Mode::Paused(menus::PausePage::Main) if escape => {
                self.mode = Mode::Playing;
                escape = false;
            }
            Mode::Paused(_) if escape => {
                self.mode = Mode::Paused(menus::PausePage::Main);
                escape = false;
            }
            _ => {}
        }
        if !interactive || self.mode != Mode::Playing {
            // Un menu est ouvert : la souris est libre.
            if self.mouse_captured {
                self.mouse_captured = false;
                ctx.set_cursor_captured(false);
            }
            self.mining = None;
        }
        if let Mode::Title(_) = self.mode {
            // Le survol du menu principal tourne lentement.
            self.camera.yaw += dt * 0.03;
        }
        let interactive = self.mode == Mode::Playing;
        self.ui.menu_open = !interactive;
        // E : l'inventaire. Ouvert, il libère la souris ; fermé, la pile tenue est rangée.
        let close = self.ui.inventory_open && escape;
        let toggle = interactive
            && ctx
                .input
                .key_pressed(self.settings.keys.key(Action::Inventory));
        if toggle || close {
            self.ui.inventory_open = !self.ui.inventory_open;
            if self.ui.inventory_open {
                self.mouse_captured = false;
                ctx.set_cursor_captured(false);
            } else {
                self.close_screen();
            }
            self.ui.screen = ui::Screen::Inventory;
        }
        let click_used = if self.ui.inventory_open || !interactive {
            true
        } else {
            self.handle_mouse_capture(ctx)
        };
        if self.mouse_captured {
            self.look(ctx.input.mouse_delta());
        }
        if interactive {
            self.handle_keys(ctx);
        }
        self.day.update(dt);
        self.update_remote(dt);
        if let Some(audio) = &mut self.audio {
            audio.update_ambience(self.day.daylight());
        }

        if player::mode(&self.ecs, self.player) == MoveMode::Fly {
            match self.autofly {
                Some(speed) => self.autofly(speed, dt),
                None => {
                    let mut speed = FLY_SPEED;
                    if ctx.input.key_held(self.settings.keys.key(Action::Sprint)) {
                        speed *= FLY_SPRINT_FACTOR;
                    }
                    let direction = if self.ui.inventory_open || !interactive {
                        Vec3::ZERO
                    } else {
                        self.wish_direction(ctx, true)
                    };
                    player::fly(&self.ecs, self.player, direction, speed, dt);
                }
            }
        }

        // Le bloc visé, depuis la caméra de la dernière image.
        // Le `Ref` du streamer doit être rendu avant d'écrire dans `self.target` : on
        // calcule d'abord dans une variable locale.
        let target = raycast(
            self.streamer().world(),
            self.camera.position,
            self.camera.forward(),
            REACH,
        );
        self.target = target;
        // Le minage : le bouton gauche maintenu sur le même bloc.
        let mining = self.mouse_captured && !click_used && ctx.input.mouse_held(MouseButton::Left);
        if !mining {
            self.mining = None;
        }
        if self.mouse_captured && !click_used {
            if mining && !ctx.input.mouse_pressed(MouseButton::Left) {
                self.mine(dt);
            }
            if ctx.input.mouse_pressed(MouseButton::Left) {
                // Une créature devant le bloc visé ? On la frappe plutôt que de casser.
                let block_distance = self.target.map_or(REACH, |hit| hit.distance);
                match mobs::hit_mob(
                    &self.ecs,
                    self.camera.position,
                    self.camera.forward(),
                    block_distance,
                ) {
                    Some((mob, _)) => {
                        // Les dégâts de l'arme ou de l'outil en main, sinon un coup de poing.
                        let damage = self
                            .held_tool()
                            .map_or(PUNCH_DAMAGE, |t| t.damage.max(PUNCH_DAMAGE));
                        mobs::damage_mob(&mut self.ecs, mob, damage, self.camera.forward());
                        self.hand.punch();
                        if self.held_tool().is_some() {
                            self.inventory.wear_selected(&self.content.items);
                        }
                        if let Some(audio) = &mut self.audio {
                            audio.play("core:break_soft");
                        }
                    }
                    None => self.mine(dt),
                }
            }
            if ctx.input.mouse_pressed(MouseButton::Right) && !self.open_container() {
                self.place_at_target();
            }
        }
        if interactive {
            self.animate_hand(ctx, dt);
        }
        // Un écran vient de s'ouvrir (établi, four, coffre) : on libère la souris.
        if self.ui.inventory_open && self.mouse_captured {
            self.mouse_captured = false;
            ctx.set_cursor_captured(false);
        }
        if self.demo.is_some() {
            self.run_demo(dt);
        }
        self.autosave();
        let changes = self.hot_reload.as_mut().and_then(|h| h.poll());
        if ctx.input.key_pressed(KeyCode::F5) {
            log::info!("F5 : rechargement de tous les assets");
            self.reload(hot_reload::Changes::ALL);
        } else if let Some(changes) = changes {
            self.reload(changes);
        }
        if self.quit_at.is_some_and(|t| Instant::now() >= t) {
            ctx.exit();
        }
        if let Err(err) = self.run_tour(ctx) {
            log::error!("captures : {err:#}");
            ctx.exit();
        }
        if let Err(err) = self.run_bench(ctx, dt) {
            log::error!("benchmark : {err:#}");
            ctx.exit();
        }

        self.schedule.run(Stage::Update, &mut self.ecs);
        self.play_event_sounds();
        self.forward_events_to_mods();
        // Fin de l'image : les événements lus tournent leurs tampons.
        events::update(&self.ecs);

        // Tout le travail du monde côté thread principal : on le chronomètre.
        let start = Instant::now();
        let eye = player::eye(&self.ecs, self.player, 1.0);
        self.streamer_mut().update(eye);
        let streamed = Instant::now();
        let uploads = self.apply_stream_events();
        self.frame_stats
            .record_streaming(streamed - start, streamed.elapsed(), uploads);
    }

    fn render(&mut self, ctx: &mut Context, alpha: f32) {
        // Entre deux pas de physique : la caméra est interpolée (voir `player.rs`).
        self.camera.position = player::eye(&self.ecs, self.player, alpha);
        // Le vent dans les feuilles suit le temps réel depuis le lancement.
        self.renderer.time = self.started.elapsed().as_secs_f32();
        let mut mob_boxes = mobs::instances(&self.ecs, alpha);
        // Les objets au sol : de petits cubes qui tournent.
        mob_boxes.extend(drops::instances(
            &self.ecs,
            &self.content.items,
            alpha,
            self.renderer.time,
        ));
        self.renderer.set_instances(&mob_boxes);
        // Le bras, éclairé par la lumière là où se tient le joueur.
        let (sky, block) = self
            .streamer()
            .world()
            .light(self.camera.position.floor().as_ivec3());
        let held = self.inventory.selected_item();
        let arm = self
            .hand
            .instances(held, &self.content.items, &self.registry);
        self.renderer
            .set_hand(&arm, [sky as f32 / 15.0, block as f32 / 15.0]);
        // L'heure décide du soleil et de la couleur du ciel, donc du brouillard.
        self.renderer.sun = Sun {
            direction: self.day.sun_direction(),
            daylight: self.day.daylight(),
            color: self.day.sun_color(),
        };
        let sky = self.day.sky_color().as_dvec3();
        self.renderer.clear_color = engine_render::wgpu_color(sky.x, sky.y, sky.z);
        self.renderer.outline = self.target.map(|hit| {
            let min = hit.block_pos.as_vec3();
            Aabb::new(min, min + Vec3::ONE)
        });

        // L'interface de cette image.
        let raw = self.ui.take_input(&ctx.window);
        let (ui_frame, platform) = self.build_ui(raw);
        self.ui.handle_output(&ctx.window, platform);

        self.update_visibility(self.renderer.aspect());
        let start = Instant::now();
        let result = self.renderer.render(&self.camera, Some(ui_frame));
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

    fn exit(&mut self, _ctx: &mut Context) {
        self.save_now();
    }

    /// Sauvegarde et décharge le monde ; rend le renderer pour le monde suivant.
    fn shutdown(mut self) -> Renderer {
        self.save_now();
        self.renderer.clear_meshes();
        self.renderer.set_visible_meshes(None);
        self.renderer.set_instances(&[]);
        self.renderer.set_hand(&[], [1.0, 0.0]);
        self.renderer.outline = None;
        self.renderer
    }
}

/// Une étape des fluides tous les 6 pas de simulation (60 Hz) : 10 par seconde.
const FLUID_STEP_EVERY: u32 = 6;
/// Le temps maximal d'une étape des fluides : au-delà, la suite attend l'étape suivante.
const FLUID_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

/// Vitesse du survol du menu principal, en blocs par seconde.
const TITLE_FLY_SPEED: f32 = 4.0;

/// L'application : un monde à la fois (celui du menu, ou celui du joueur). Elle
/// change de monde quand le jeu le demande, sans recréer la fenêtre.
struct App {
    game: Option<VoxelGame>,
    /// `VOXEL_MENU_DEMO=1` : un scénario automatique pour vérifier les changements de
    /// monde sans cliquer (menu → monde → menu → quitter). Le temps écoulé et l'étape.
    menu_demo: Option<(Instant, u32)>,
    /// `VOXEL_CONTROL=port` : le jeu piloté par commandes texte (voir `control.rs`).
    control: Option<control::ControlServer>,
}

impl App {
    fn game(&mut self) -> &mut VoxelGame {
        self.game.as_mut().expect("un monde est toujours chargé")
    }

    fn transition(&mut self, ctx: &mut Context, request: Request) -> Result<()> {
        let Some(game) = self.game.take() else {
            return Ok(());
        };
        let settings = game.settings.clone();
        if request == Request::Quit {
            self.game = Some(game);
            ctx.exit();
            return Ok(());
        }
        let renderer = game.shutdown();
        let launch = match request {
            Request::Play(launch) => launch,
            _ => Launch::title(),
        };
        log::info!("chargement : {launch:?}");
        self.game = Some(VoxelGame::create(ctx, renderer, launch, settings)?);
        Ok(())
    }
}

impl Game for App {
    fn new(ctx: &mut Context) -> Result<Self> {
        // `?` remonte l'erreur au moteur, qui l'affichera à la fermeture.
        let renderer = Renderer::new(ctx.window.clone())?;
        let settings = settings::Settings::load(&project_root());
        let launch = Launch::from_env().unwrap_or_else(Launch::title);
        Ok(Self {
            game: Some(VoxelGame::create(ctx, renderer, launch, settings)?),
            menu_demo: std::env::var("VOXEL_MENU_DEMO")
                .ok()
                .map(|_| (Instant::now(), 0)),
            control: control::ControlServer::from_env(),
        })
    }

    fn fixed_update(&mut self, ctx: &mut Context) {
        self.game().fixed_update(ctx);
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        self.game().update(ctx, dt);
        if let (Some(control), Some(game)) = (&self.control, self.game.as_mut()) {
            for request in control.poll() {
                game.run_command(ctx, request);
            }
        }
        if let Some((start, step)) = &mut self.menu_demo {
            // Chaque étape quand le monde courant est chargé et quelques secondes passées.
            let ready = start.elapsed().as_secs_f32() > 4.0
                && self.game.as_ref().is_some_and(|g| g.streamer().is_idle());
            if ready {
                let request = match step {
                    0 => Request::Play(Launch {
                        world: Some(project_root().join("saves").join("menu-demo")),
                        seed: Some(7),
                        title: false,
                    }),
                    1 => Request::Title,
                    _ => Request::Quit,
                };
                log::info!("démo des menus, étape {step} : {request:?}");
                *step += 1;
                *start = Instant::now();
                self.game().request = Some(request);
            }
        }
        if let Some(request) = self.game().request.take()
            && let Err(err) = self.transition(ctx, request)
        {
            log::error!("changement de monde impossible : {err:#}");
            ctx.exit();
        }
    }

    fn render(&mut self, ctx: &mut Context, alpha: f32) {
        self.game().render(ctx, alpha);
    }

    fn resized(&mut self, ctx: &mut Context, size: PhysicalSize<u32>) {
        self.game().resized(ctx, size);
    }

    fn window_event(&mut self, ctx: &mut Context, event: &WindowEvent) -> bool {
        self.game().window_event(ctx, event)
    }

    fn exit(&mut self, ctx: &mut Context) {
        self.game().exit(ctx);
    }
}

/// La racine du projet (le dossier qui contient `assets/` et `saves/`).
fn project_root() -> PathBuf {
    assets_dir()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

impl VoxelGame {
    /// Direction voulue d'après les touches. Horizontale en marche ; en vol,
    /// Espace et Maj ajoutent la verticale.
    fn wish_direction(&self, ctx: &Context, vertical: bool) -> Vec3 {
        let input = &ctx.input;
        let keys = &self.settings.keys;
        let mut bindings = vec![
            (keys.key(Action::Forward), self.camera.forward_flat()),
            (keys.key(Action::Back), -self.camera.forward_flat()),
            (keys.key(Action::Right), self.camera.right()),
            (keys.key(Action::Left), -self.camera.right()),
        ];
        if vertical {
            bindings.push((keys.key(Action::Jump), Vec3::Y));
            bindings.push((keys.key(Action::Descend), -Vec3::Y));
        }
        // Chaque touche ajoute une direction ; on somme puis on normalise.
        // `normalize_or_zero` évite une division par zéro quand aucune touche n'est tenue.
        bindings
            .iter()
            .filter(|(key, _)| input.key_held(*key))
            .map(|(_, dir)| *dir)
            .sum::<Vec3>()
            .normalize_or_zero()
    }

    fn handle_keys(&mut self, ctx: &Context) {
        let input = &ctx.input;
        if input.key_pressed(self.settings.keys.key(Action::Time)) {
            self.day.cycle_speed();
            log::info!("temps {} ({})", self.day.speed.name(), self.day.clock());
        }
        if input.key_pressed(self.settings.keys.key(Action::Fly)) {
            player::toggle_mode(&mut self.ecs, self.player);
            log::info!("mode {}", player::mode(&self.ecs, self.player).name());
        }
        for (i, key) in HOTBAR_KEYS.iter().enumerate() {
            if input.key_pressed(*key) {
                self.inventory.selected = i;
            }
        }
        if input.key_pressed(KeyCode::F3) {
            self.ui.inspector_open = !self.ui.inspector_open;
        }
        if input.key_pressed(KeyCode::F4) {
            self.hand.visible = !self.hand.visible;
        }
        if input.key_pressed(KeyCode::F1) {
            let mesher = self.streamer().config().mesher.other();
            log::info!("maillage {} : on remaille tout", mesher.name());
            self.streamer_mut().set_mesher(mesher);
        }
        if input.key_pressed(KeyCode::F6) {
            self.toggle_cave_culling();
        }
        if input.key_pressed(KeyCode::F2) {
            self.renderer.frustum_culling = !self.renderer.frustum_culling;
            let state = if self.renderer.frustum_culling {
                "activé"
            } else {
                "désactivé"
            };
            log::info!("frustum culling {state}");
        }
    }

    /// Recharge ce qui a changé sur le disque. Une erreur est affichée dans les logs,
    /// et l'ancienne version reste en place : le jeu ne plante jamais pour un asset.
    fn reload(&mut self, changes: hot_reload::Changes) {
        if changes.scripts {
            self.mods.reload_scripts();
        }
        if changes.shader {
            let result = std::fs::read_to_string(engine_render::CHUNK_SHADER_PATH)
                .map_err(anyhow::Error::from)
                .and_then(|source| self.renderer.reload_chunk_shader(&source));
            match result {
                Ok(()) => log::info!("shader des chunks rechargé"),
                Err(err) => log::error!("{err:#}"),
            }
        }
        let mut textures = changes.textures;
        if changes.blocks {
            match hot_reload::reload_registry(&self.registry, &assets_dir(), &self.mod_dirs) {
                hot_reload::RegistryReload::Applied(fresh) => {
                    self.registry = Arc::new(fresh);
                    let registry = self.registry.clone();
                    self.streamer_mut().replace_registry(registry);
                    textures = true;
                    log::info!("blocs rechargés : lumière et maillages recalculés");
                }
                hot_reload::RegistryReload::Incompatible(reason) => log::warn!("{reason}"),
                hot_reload::RegistryReload::Failed(err) => {
                    log::error!("blocks.ron invalide, l'ancien est gardé : {err:#}")
                }
            }
        }
        if (changes.sounds || textures)
            && let Some(audio) = &mut self.audio
        {
            audio.reload(&self.registry);
        }
        if textures {
            self.block_textures = assets::load_textures(&self.registry);
            self.renderer.set_block_textures(&self.block_textures);
            self.ui.clear_icons();
            log::info!("{} textures rechargées", self.registry.textures().len());
        }
    }

    /// Casse le bloc visé. Renvoie sa position si c'est fait.
    fn break_target(&mut self) -> Option<IVec3> {
        let hit = self.target?;
        self.break_block(hit.block_pos).then_some(hit.block_pos)
    }

    /// Un clic droit sur un établi, un four ou un coffre ouvre son écran. `true` si c'est fait.
    fn open_container(&mut self) -> bool {
        let Some(hit) = self.target else { return false };
        let block = self.streamer().world().block(hit.block_pos);
        let screen = match self.registry.def(block).behavior.as_deref() {
            Some("core:chest") => ui::Screen::Chest(hit.block_pos),
            Some("core:furnace") => ui::Screen::Furnace(hit.block_pos),
            _ if self.registry.def(block).key == "core:workbench" => ui::Screen::Workbench,
            _ => return false,
        };
        // Un conteneur ancien (ou posé par un mod) n'a peut-être pas encore son inventaire.
        if let Some(slots) = engine_voxel::container_slots(&self.registry, block) {
            let mut streamer = self.streamer_mut();
            if streamer
                .block_entity(hit.block_pos)
                .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
                .is_none()
            {
                let mut entity = streamer
                    .block_entity(hit.block_pos)
                    .cloned()
                    .unwrap_or_default();
                entity.caps_mut().inventory = Some(engine_voxel::Inventory::new(slots));
                streamer.set_block_entity(hit.block_pos, Some(entity));
            }
        }
        self.ui.screen = screen;
        self.ui.inventory_open = true;
        true
    }

    /// Ferme l'écran ouvert : la grille d'artisanat et la main retournent dans
    /// l'inventaire (ce qui n'y tient pas tombe par terre).
    fn close_screen(&mut self) {
        self.ui.split = None;
        let loose = self.inventory.put_away(&self.content.items);
        let at = player::body(&self.ecs, self.player)
            .position
            .floor()
            .as_ivec3()
            + IVec3::Y;
        for stack in loose {
            self.drop_stack(at, stack);
        }
        self.ui.inventory_open = false;
        self.ui.screen = ui::Screen::Inventory;
    }

    /// Les objets d'un coffre ou d'un four (vide pour un autre bloc).
    fn container_contents(&self, pos: IVec3) -> Vec<inventory::ItemStack> {
        let streamer = self.streamer();
        let Some(inventory) = streamer
            .block_entity(pos)
            .and_then(|e| e.caps.as_ref()?.inventory.clone())
        else {
            return Vec::new();
        };
        inventory
            .slots
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(inventory::ItemStack {
                    item: self.content.items.id(&s.item)?,
                    count: s.count,
                    damage: s.damage,
                })
            })
            .collect()
    }

    /// Les animations du bras : marche, minage, changement d'objet, inertie.
    fn animate_hand(&mut self, ctx: &Context, dt: f32) {
        let velocity = self
            .ecs
            .get::<Velocity>(self.player)
            .map_or(Vec3::ZERO, |v| v.0);
        let on_ground = self
            .ecs
            .get::<Collider>(self.player)
            .is_some_and(|c| c.on_ground)
            || player::mode(&self.ecs, self.player) == MoveMode::Fly;
        self.hand.update(hand::HandInput {
            dt,
            speed: Vec2::new(velocity.x, velocity.z).length(),
            sprinting: ctx.input.key_held(KeyCode::ControlLeft),
            on_ground,
            mining: self.mining.is_some(),
            selected: (self.inventory.selected, self.inventory.selected_item()),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
        });
    }

    /// Le minage, à chaque image où le bouton gauche est tenu : l'avancement grandit selon
    /// la dureté du bloc et l'outil en main ; à 1, le bloc casse.
    fn mine(&mut self, dt: f32) {
        let Some(hit) = self.target else {
            self.mining = None;
            return;
        };
        let pos = hit.block_pos;
        let block = self.streamer().world().block(pos);
        let tool = self.held_tool();
        let Some(time) = engine_voxel::break_time(self.registry.def(block), tool.as_ref()) else {
            self.mining = None;
            return; // Incassable.
        };
        let progress = match self.mining {
            Some((p, progress)) if p == pos => progress,
            _ => 0.0,
        };
        let progress = if time <= 0.0 {
            1.0
        } else {
            progress + dt / time
        };
        if progress >= 1.0 {
            self.mining = None;
            self.break_block(pos);
        } else {
            self.mining = Some((pos, progress));
        }
    }

    /// L'outil tenu en main.
    fn held_tool(&self) -> Option<engine_voxel::ToolDef> {
        self.inventory
            .selected_item()
            .and_then(|i| self.content.items.get(i).tool)
    }

    /// Casse le bloc en `pos` : il lâche son butin (selon l'outil), l'outil s'use, et les
    /// blocs qui en dépendaient (une fleur dessus, du sable) suivent.
    fn break_block(&mut self, pos: IVec3) -> bool {
        let block = self.streamer().world().block(pos);
        let def = self.registry.def(block).clone();
        if def.hardness < 0.0 {
            return false;
        }
        let tool = self.held_tool();
        // Un coffre ou un four cassé lâche aussi ce qu'il contient.
        let contents = self.container_contents(pos);
        if !self.edit(pos, BlockId::AIR) {
            return false;
        }
        let harvest = engine_voxel::can_harvest(&def, tool.as_ref());
        self.drop_loot(pos, block, tool.map(|t| t.kind), harvest);
        for stack in contents {
            self.drop_stack(pos, stack);
        }
        if tool.is_some() && def.hardness > 0.0 && self.inventory.wear_selected(&self.content.items)
        {
            log::info!("l'outil s'est cassé");
            if let Some(audio) = &mut self.audio {
                audio.play("core:break_glass");
            }
        }
        true
    }

    /// Ce que lâche un bloc, par terre à sa place.
    fn drop_loot(
        &mut self,
        pos: IVec3,
        block: BlockId,
        tool: Option<engine_voxel::ToolKind>,
        harvest: bool,
    ) {
        let seed = self.next_random();
        let mut rng = engine_voxel::Rng64::new(seed);
        let loot = self
            .content
            .loot
            .roll(block, tool, harvest, &self.content.items, &mut rng);
        for (item, count) in loot {
            self.drop_stack(pos, inventory::ItemStack::new(item, count));
        }
    }

    fn drop_stack(&mut self, pos: IVec3, stack: inventory::ItemStack) {
        let seed = self.next_random();
        drops::spawn(&mut self.ecs, stack, pos.as_vec3() + Vec3::splat(0.5), seed);
    }

    /// Un nombre pseudo-aléatoire pour le jeu (butin, rebonds).
    fn next_random(&mut self) -> u64 {
        self.random = self
            .random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.random ^ (self.random >> 29)
    }

    /// Ramasse les objets au sol tout proches.
    fn pick_up_drops(&mut self, dt: f32) {
        let feet = player::body(&self.ecs, self.player).position;
        for (entity, stack) in drops::update(&mut self.ecs, feet, dt) {
            let left = self.inventory.add_stack(stack, &self.content.items);
            if left < stack.count {
                if let Some(audio) = &mut self.audio {
                    audio.play("core:place_soft");
                }
                if left == 0 {
                    self.ecs.despawn(entity);
                } else if let Some(mut drop) = self.ecs.get_mut::<drops::DroppedItem>(entity) {
                    drop.stack.count = left;
                }
            }
        }
    }

    /// Pose le bloc de la case choisie contre la face visée. Renvoie sa position si c'est fait.
    fn place_at_target(&mut self) -> Option<IVec3> {
        let item = self.inventory.selected_item()?;
        if self.content.items.get(item).key.ends_with("bucket") {
            self.use_bucket(item);
            return None;
        }
        let hit = self.target?;
        let block = self.content.items.get(item).block?;
        let pos = hit.block_pos + hit.normal;
        // Normale nulle : le rayon part de l'intérieur d'un bloc, pas de face où poser.
        // On peut poser dans l'air ou à la place d'un liquide (l'eau).
        let target = self.streamer().world().block(pos);
        let replaceable = target.is_air() || self.registry.def(target).liquid;
        if hit.normal == IVec3::ZERO || !replaceable {
            return None;
        }
        if player::body(&self.ecs, self.player).overlaps_block(pos) || self.mob_overlaps(pos) {
            log::debug!("impossible de poser un bloc dans une entité");
            return None;
        }
        // Une plante ou une torche a besoin d'un support.
        let support = self.registry.def(block).support;
        if !engine_voxel::is_supported(&self.streamer(), &self.registry, pos, support) {
            return None;
        }
        let placed = self.edit(pos, block);
        if placed {
            self.inventory.consume_selected();
            self.hand.punch();
            // Des feuilles posées par le joueur ne se décomposent pas : une entité de bloc
            // les marque comme « posées ».
            if self.registry.def(block).behavior.as_deref() == Some("core:leaves") {
                self.streamer_mut()
                    .set_block_entity(pos, Some(engine_voxel::BlockEntity::default()));
            }
        }
        placed.then_some(pos)
    }

    /// Le seau : vide, il prend le fluide de la première cellule visée ; plein, il le
    /// reverse dans la cellule devant le bloc visé. La quantité transférée est celle de
    /// la cellule (jusqu'à un bloc) : elle est gardée dans l'objet (son champ `damage`).
    fn use_bucket(&mut self, item: engine_voxel::ItemId) {
        let items = self.content.items.clone();
        let key = items.get(item).key.clone();
        let slot = self.inventory.selected;
        if key == "core:bucket" {
            // Le long du rayon, la première cellule de fluide.
            let found = {
                let streamer = self.streamer();
                (1..(REACH * 10.0) as i32)
                    .map(|k| {
                        (self.camera.position + self.camera.forward() * (k as f32 * 0.1))
                            .floor()
                            .as_ivec3()
                    })
                    .find(|&p| self.fluids.kind_of(streamer.world().block(p)).is_some())
            };
            let Some(pos) = found else { return };
            let Some((kind, amount)) = self.fluids.take(&mut self.world.borrow_mut(), pos) else {
                return;
            };
            let fluid = self.fluids.kinds()[kind].key.clone();
            let Some(full) = items.id(&format!("{fluid}_bucket")) else {
                return;
            };
            self.inventory.consume_selected();
            let filled = inventory::ItemStack {
                item: full,
                count: 1,
                damage: amount as u32,
            };
            if self.inventory.slots[slot].is_none() {
                self.inventory.slots[slot] = Some(filled);
            } else if self.inventory.add_stack(filled, &items) > 0 {
                self.drop_stack(pos, filled);
            }
            if let Some(audio) = &mut self.audio {
                audio.play(audio::SPLASH);
            }
            self.hand.punch();
        } else {
            let Some(hit) = self.target else { return };
            let Some(stack) = self.inventory.selected_stack() else {
                return;
            };
            let fluid_key = key.trim_end_matches("_bucket");
            let Some(kind) = self.fluids.kinds().iter().position(|k| k.key == fluid_key) else {
                return;
            };
            let pos = hit.block_pos + hit.normal;
            let poured =
                self.fluids
                    .pour(&mut self.world.borrow_mut(), pos, kind, stack.damage as i32);
            if poured == 0 {
                return;
            }
            let left = stack.damage as i32 - poured;
            self.inventory.slots[slot] = Some(if left > 0 {
                inventory::ItemStack {
                    damage: left as u32,
                    ..stack
                }
            } else {
                inventory::ItemStack::new(items.id("core:bucket").unwrap_or(item), 1)
            });
            if let Some(audio) = &mut self.audio {
                audio.play(audio::SPLASH);
            }
            self.hand.punch();
        }
    }

    /// Une créature occupe-t-elle ce bloc ?
    fn mob_overlaps(&self, pos: IVec3) -> bool {
        let mut overlap = false;
        self.ecs
            .query::<(&Transform, &Collider, &components::Mob)>()
            .for_each(|_, (transform, collider, _)| {
                let body = engine_voxel::PlayerBody::with_size(
                    transform.position,
                    collider.width,
                    collider.height,
                );
                overlap |= body.overlaps_block(pos);
            });
        overlap
    }

    fn edit(&mut self, pos: IVec3, block: BlockId) -> bool {
        let old = self.streamer().world().block(pos);
        let start = Instant::now();
        let done = self.streamer_mut().set_block(pos, block);
        if done {
            self.frame_stats.record_edit(start.elapsed());
            // Les machines voisines (et la nouvelle, si c'en est une) se réveillent.
            let actions = self.mods.block_changed(pos);
            self.apply_machine_actions(actions);
            // Le bloc a changé : le rayon doit être relancé à la prochaine image.
            self.target = None;
            // On annonce le changement : le son (et plus tard les mods) s'y abonnent.
            if block.is_air() {
                events::send(&self.ecs, BlockBroken { pos, block: old });
            } else {
                events::send(&self.ecs, BlockPlaced { pos, block });
            }
            // Les fluides voisins se réveillent (un trou au bord d'un lac se remplit).
            self.fluids.wake(pos);
            // Le support et la gravité : une fleur sans sol tombe, le sable s'écroule.
            let settled = engine_voxel::settle(&mut self.streamer_mut(), pos);
            for (p, broken) in settled.broken {
                let harvest = self.registry.def(broken).tier == 0;
                self.drop_loot(p, broken, None, harvest);
                let actions = self.mods.block_changed(p);
                self.apply_machine_actions(actions);
            }
            for (from, to, _) in settled.fallen {
                for p in [from, to] {
                    self.fluids.wake(p);
                    let actions = self.mods.block_changed(p);
                    self.apply_machine_actions(actions);
                }
            }
        }
        done
    }

    /// Le jeu écoute ses propres événements pour jouer les sons.
    fn play_event_sounds(&mut self) {
        let Some(audio) = &mut self.audio else {
            return;
        };
        for event in self
            .readers
            .broken
            .read(&self.ecs.resource::<Events<BlockBroken>>())
        {
            audio.play_block(&self.registry, event.block, audio::BlockSound::Break);
        }
        for event in self
            .readers
            .placed
            .read(&self.ecs.resource::<Events<BlockPlaced>>())
        {
            audio.play_block(&self.registry, event.block, audio::BlockSound::Place);
        }
        for event in self
            .readers
            .died
            .read(&self.ecs.resource::<Events<EntityDied>>())
        {
            log::info!("{} est mort", event.kind);
            audio.play("core:break_soft");
        }
        for event in self
            .readers
            .damaged
            .read(&self.ecs.resource::<Events<PlayerDamaged>>())
        {
            log::info!("aïe : -{} ({} restants)", event.amount, event.health_left);
            audio.play("core:place_hard");
        }
    }

    /// Mort : on réapparaît au point de départ, en pleine forme.
    fn respawn_if_dead(&mut self) {
        let health = player::health(&self.ecs, self.player);
        if health.current > 0.0 {
            return;
        }
        let spawn = self.spawn;
        player::set_eye(
            &self.ecs,
            self.player,
            Vec3::new(
                spawn.x as f32 + 0.5,
                spawn.y as f32 + 3.0,
                spawn.z as f32 + 0.5,
            ),
        );
        if let Some(mut health) = self.ecs.get_mut::<Health>(self.player) {
            health.current = health.max;
        }
        log::info!("le joueur est mort : retour au point de départ");
    }

    /// Le streamer des chunks (partagé avec l'ECS et les mods).
    fn streamer(&self) -> Ref<'_, ChunkStreamer> {
        self.world.borrow()
    }

    fn streamer_mut(&self) -> RefMut<'_, ChunkStreamer> {
        self.world.borrow_mut()
    }

    /// Ce que les machines et les scripts demandent au jeu.
    fn apply_machine_actions(&mut self, actions: Vec<MachineAction>) {
        for action in actions {
            match action {
                MachineAction::GiveItem { block, count } => {
                    let Some(item) = self.content.items.of_block(block) else {
                        continue;
                    };
                    let left = self.inventory.add(item, count, &self.content.items);
                    if left > 0 {
                        log::info!("inventaire plein : {left} objet(s) perdu(s)");
                    }
                }
                MachineAction::SpawnEntity { kind, position } => {
                    let registry = self.ecs.resource::<Arc<MobRegistry>>().clone();
                    match registry.id(&kind) {
                        Some(id) => {
                            mobs::spawn_mob(&mut self.ecs, id, position);
                        }
                        None => log::warn!("créature inconnue demandée par un mod : {kind}"),
                    }
                }
                MachineAction::BlockBroken { pos, block } => {
                    events::send(&self.ecs, BlockBroken { pos, block });
                }
                MachineAction::Log(message) => log::info!("{message}"),
            }
        }
    }

    /// Transmet les événements du jeu aux scripts des mods.
    fn forward_events_to_mods(&mut self) {
        let _span = engine_trace::span("mods : événements");
        let mut events = Vec::new();
        for e in self
            .mod_readers
            .broken
            .read(&self.ecs.resource::<Events<BlockBroken>>())
        {
            events.push(ModEvent::BlockBroken {
                at: GridPos::world(e.pos),
                block: e.block,
            });
        }
        // La direction du regard, ramenée à l'axe horizontal le plus proche : pour
        // orienter les machines posées (un casseur regarde dans la direction du joueur).
        let forward = self.camera.forward_flat();
        let facing = if forward.x.abs() > forward.z.abs() {
            IVec3::new(forward.x.signum() as i32, 0, 0)
        } else {
            IVec3::new(0, 0, forward.z.signum() as i32)
        };
        for e in self
            .mod_readers
            .placed
            .read(&self.ecs.resource::<Events<BlockPlaced>>())
        {
            events.push(ModEvent::BlockPlaced {
                at: GridPos::world(e.pos),
                block: e.block,
                facing,
            });
        }
        for e in self
            .mod_readers
            .died
            .read(&self.ecs.resource::<Events<EntityDied>>())
        {
            events.push(ModEvent::EntityDied {
                kind: e.kind.clone(),
                position: e.position,
            });
        }
        for e in self
            .mod_readers
            .damaged
            .read(&self.ecs.resource::<Events<PlayerDamaged>>())
        {
            events.push(ModEvent::PlayerDamaged {
                amount: e.amount,
                health_left: e.health_left,
            });
        }
        for event in events {
            let actions = self.mods.dispatch(&event);
            self.apply_machine_actions(actions);
        }
    }

    /// Les bruits de pas (selon le bloc sous les pieds) et l'éclaboussure en entrant dans l'eau.
    fn footsteps(&mut self, dt: f32) {
        if player::mode(&self.ecs, self.player) != MoveMode::Walk
            || player::frozen(&self.ecs, self.player)
        {
            return;
        }
        let (Some(collider), Some(velocity), Some(transform)) = (
            self.ecs.get::<Collider>(self.player).map(|c| *c),
            self.ecs.get::<Velocity>(self.player).map(|v| v.0),
            self.ecs.get::<Transform>(self.player).map(|t| t.position),
        ) else {
            return;
        };
        let below_block = self
            .streamer()
            .world()
            .block((transform - Vec3::Y * 0.05).floor().as_ivec3());
        let Some(audio) = &mut self.audio else {
            return;
        };
        if collider.in_liquid && !self.was_in_liquid && velocity.y < -1.0 {
            audio.play(audio::SPLASH);
        }
        self.was_in_liquid = collider.in_liquid;
        if !collider.on_ground || collider.in_liquid {
            return;
        }
        // Un pas tous les 1,7 bloc parcourus à l'horizontale.
        self.step_distance += Vec2::new(velocity.x, velocity.z).length() * dt;
        if self.step_distance >= 1.7 {
            self.step_distance = 0.0;
            audio.play_block(&self.registry, below_block, audio::BlockSound::Step);
        }
    }

    /// Applique les résultats du streamer côté GPU, dans la limite du budget.
    /// Renvoie le nombre de maillages envoyés.
    fn apply_stream_events(&mut self) -> usize {
        let _span = engine_trace::span("chunks : envoi au GPU");
        let mut uploads = 0;
        while uploads < UPLOAD_BUDGET {
            let Some(event) = self.streamer_mut().next_event() else {
                break;
            };
            match event {
                // Libérer est presque gratuit : ça ne compte pas dans le budget.
                StreamEvent::Unload(pos) => {
                    self.culling.graph.remove(pos);
                    if let Some(id) = self.meshes.remove(&pos) {
                        self.renderer.remove_mesh(id);
                    }
                }
                StreamEvent::Mesh(pos, mesh) => {
                    uploads += 1;
                    self.culling.graph.set(pos, mesh.visibility);
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
        let stream = self.streamer().stats();
        let render = self.last_render;
        let pending = stream.queued + stream.in_flight + stream.meshes_waiting;
        let triangles = render.triangles as f64 / 1e6;
        let frozen = if player::frozen(&self.ecs, self.player) {
            " (chargement…)"
        } else {
            ""
        };
        ctx.status = format!(
            "{} | {}{frozen} | {} entités | {} chunks | {pending} en attente | {}/{} dessinés ({} cachés) | {triangles:.2} M tri",
            self.day.clock(),
            player::mode(&self.ecs, self.player).name(),
            self.ecs.len(),
            stream.loaded,
            render.meshes_drawn,
            render.meshes_total,
            render.meshes_hidden,
        );

        if let Some(report) = self.frame_stats.tick() {
            self.debug = ui::DebugInfo {
                fps: 1000.0 / report.avg_frame_ms.max(0.001),
                frame_ms: report.avg_frame_ms,
                max_frame_ms: report.max_frame_ms,
            };
            log::info!(
                "image : moy {:.2} ms, max {:.2} ms | streaming : max {:.2} ms, envois GPU : max {:.2} ms ({} au total) | \
                 rendu : max {:.2} ms | modifs : {} (max {:.2} ms) | chunks : {} chargés, {} à générer, {} en cours, {} tâches jetées sur {} | \
                 dessinés {}/{} ({} cachés), {triangles:.2} M triangles",
                report.avg_frame_ms,
                report.max_frame_ms,
                report.max_streaming_ms,
                report.max_upload_ms,
                report.uploads,
                report.max_render_ms,
                report.edits,
                report.max_edit_ms,
                stream.loaded,
                stream.queued,
                stream.in_flight,
                stream.jobs_discarded,
                stream.jobs_done,
                render.meshes_drawn,
                render.meshes_total,
                render.meshes_hidden,
            );
            let fluids = self.fluid_report;
            log::info!(
                "fluides : {} cellules calculées, {} actives, {} modifiées, {:.2} ms",
                fluids.processed,
                fluids.active,
                fluids.changed,
                modding::ms(fluids.duration)
            );
            let tick = self.mods.last_tick;
            log::info!(
                "machines : tick {} ({} programmés, {} voisins, {:.2} ms), {} en attente | scripts : {} chargé(s){}",
                self.mods.machines.scheduler().tick(),
                tick.scheduled,
                tick.neighbor_updates,
                modding::ms(tick.duration),
                self.mods.machines.scheduler().pending(),
                self.mods.scripts_loaded(),
                self.mods
                    .disabled()
                    .iter()
                    .map(|(id, why)| format!(", « {id} » désactivé : {why}"))
                    .collect::<String>(),
            );
        }
    }

    /// Renvoie `true` si ce clic a servi à capturer la souris (il ne casse donc rien).
    fn handle_mouse_capture(&mut self, ctx: &mut Context) -> bool {
        let input = &ctx.input;
        if !self.mouse_captured && input.mouse_pressed(MouseButton::Left) {
            self.mouse_captured = true;
            ctx.set_cursor_captured(true);
            return true;
        }
        if input.key_pressed(KeyCode::Escape) {
            if self.mouse_captured {
                self.mouse_captured = false;
                ctx.set_cursor_captured(false);
            } else {
                ctx.exit();
            }
        }
        false
    }

    fn look(&mut self, delta: Vec2) {
        let sensitivity = MOUSE_SENSITIVITY * self.settings.mouse_sensitivity;
        self.camera.yaw += delta.x * sensitivity;
        self.camera.pitch = (self.camera.pitch - delta.y * sensitivity)
            .clamp(-Camera::MAX_PITCH, Camera::MAX_PITCH);
    }

    /// Avance tout droit en suivant le relief, pour mesurer le chargement.
    fn autofly(&mut self, speed: f32, dt: f32) {
        let mut eye =
            player::eye(&self.ecs, self.player, 1.0) + self.camera.forward_flat() * speed * dt;
        let ground = self
            .streamer()
            .generator()
            .column(eye.x.floor() as i32, eye.z.floor() as i32)
            .height as f32;
        // On rejoint doucement l'altitude visée au lieu de sauter d'un coup.
        eye.y += (ground + FLY_ALTITUDE - eye.y) * (dt * 2.0).min(1.0);
        player::set_eye(&self.ecs, self.player, eye);
    }

    /// Une action de démo par seconde : casser, puis poser, en alternance.
    fn run_demo(&mut self, dt: f32) {
        let Some(demo) = self.demo.as_mut() else {
            return;
        };
        demo.timer += dt;
        if demo.timer < 1.0 || player::frozen(&self.ecs, self.player) {
            return;
        }
        demo.timer = 0.0;
        demo.breaking = !demo.breaking;
        let breaking = demo.breaking;
        // On essaie aussi les blocs des mods : à chaque pose, le suivant de la barre.
        if !breaking {
            // La case suivante qui contient quelque chose.
            for _ in 0..inventory::HOTBAR {
                self.inventory.selected = (self.inventory.selected + 1) % inventory::HOTBAR;
                if self.inventory.selected_item().is_some() {
                    break;
                }
            }
        }
        let before = Instant::now();
        let result = if breaking {
            self.break_target()
        } else {
            self.place_at_target()
        };
        let elapsed = before.elapsed();
        let body = engine_voxel::PlayerBody {
            on_ground: self
                .ecs
                .get::<Collider>(self.player)
                .is_some_and(|c| c.on_ground),
            ..player::body(&self.ecs, self.player)
        };
        let action = if breaking { "cassé" } else { "posé" };
        let block = self
            .inventory
            .selected_item()
            .map_or("?", |i| self.content.items.get(i).key.as_str());
        match result {
            Some(pos) => log::info!(
                "démo : {action} {} en {:.2} ms ({}) | pieds {:.1?} au sol : {} | {} maillages en attente",
                pos,
                elapsed.as_secs_f32() * 1000.0,
                if breaking { "-" } else { block },
                body.position.to_array(),
                body.on_ground,
                self.streamer().stats().meshes_waiting,
            ),
            None => log::info!(
                "démo : rien à {} (cible {:?}) | pieds {:.1?}",
                if breaking { "casser" } else { "poser" },
                self.target.map(|t| t.block_pos),
                body.position.to_array()
            ),
        }
    }
}

/// Le mode `VOXEL_DEMO` : un joueur automatique pour tester sans clavier.
struct Demo {
    timer: f32,
    breaking: bool,
}

impl Demo {
    fn new() -> Self {
        Self {
            timer: 0.0,
            breaking: false,
        }
    }

    /// Avance tout droit en courant ; saute quand un bloc barre le chemin.
    fn walk(ecs: &engine_ecs::World, player: Entity, camera: &Camera) -> (Vec3, bool) {
        let forward = camera.forward_flat();
        let feet = player::body(ecs, player).position;
        let ahead = feet + forward * 0.8 + Vec3::Y * 0.5;
        let blocked = ecs
            .streamer()
            .borrow()
            .world()
            .is_solid(ahead.floor().as_ivec3());
        (forward * SPRINT_SPEED, blocked)
    }
}

/// Les curseurs du jeu sur ses propres événements.
struct EventReaders {
    broken: EventReader,
    placed: EventReader,
    died: EventReader,
    damaged: EventReader,
}

impl EventReaders {
    fn new(ecs: &engine_ecs::World) -> Self {
        Self {
            broken: ecs.resource::<Events<BlockBroken>>().reader(),
            placed: ecs.resource::<Events<BlockPlaced>>().reader(),
            died: ecs.resource::<Events<EntityDied>>().reader(),
            damaged: ecs.resource::<Events<PlayerDamaged>>().reader(),
        }
    }
}

/// Un petit nécessaire pour un nouveau monde : de quoi s'éclairer et construire.
fn starter_inventory(items: &engine_voxel::ItemRegistry) -> Inventory {
    let mut inventory = Inventory::default();
    for (key, count) in [
        ("core:stone_pickaxe", 1),
        ("core:stone_axe", 1),
        ("core:stone_shovel", 1),
        ("core:torch", 32),
        ("core:oak_log", 16),
        ("core:workbench", 1),
        ("core:furnace", 1),
        ("core:chest", 1),
        ("core:coal", 8),
        ("core:raw_iron", 6),
        ("core:shears", 1),
        ("core:oak_sapling", 4),
        ("core:glass", 16),
        ("core:glowstone", 8),
        // De quoi construire un mécanisme : horloge → lampe, horloge → casseur.
        ("core:clock", 4),
        ("core:lamp", 4),
        ("example:breaker", 2),
        ("example:ruby", 4),
    ] {
        if let Some(id) = items.id(key) {
            inventory.add(id, count, items);
        }
    }
    inventory
}

/// Lit le registre des blocs : `assets/core` puis les mods de `mods/`.
fn load_registry(mods: &[(String, PathBuf)]) -> Result<BlockRegistry> {
    let assets = assets_dir();
    let registry = BlockRegistry::load_mods(&assets, mods)
        .with_context(|| format!("chargement des blocs depuis {}", assets.display()))?;
    let keys: Vec<&str> = registry
        .iter()
        .skip(1)
        .map(|(_, def)| def.key.as_str())
        .collect();
    log::info!("{} blocs chargés : {}", keys.len(), keys.join(", "));
    Ok(registry)
}

/// Le registre complet (core et mods, dans l'ordre), pour les tests.
#[cfg(test)]
fn test_registry() -> BlockRegistry {
    load_registry(&modding::mod_dirs(&modding::resolve_order(&assets_dir()))).unwrap()
}

/// Démarre la surveillance des assets, des mods et des shaders (si les dossiers existent).
fn start_hot_reload() -> Option<hot_reload::HotReload> {
    let assets = assets_dir();
    let mut dirs = vec![assets.clone()];
    if let Some(root) = assets.parent() {
        dirs.push(root.join("mods"));
    }
    if let Some(shaders) = std::path::Path::new(engine_render::CHUNK_SHADER_PATH).parent() {
        dirs.push(shaders.to_path_buf());
    }
    hot_reload::HotReload::new(&dirs)
        .map_err(|err| log::warn!("rechargement à chaud indisponible : {err:#}"))
        .ok()
}

/// `VOXEL_ASSETS`, sinon `./assets`, sinon le dossier du dépôt (connu à la compilation).
fn assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VOXEL_ASSETS") {
        return dir.into();
    }
    let local = PathBuf::from("assets");
    if local.is_dir() {
        return local;
    }
    // `env!` lit une variable au moment de la compilation : ici, le dossier de cette crate.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Envoie un maillage au GPU. `None` s'il est vide.
fn upload(renderer: &mut Renderer, mesh: &ChunkMesh) -> Option<MeshId> {
    let (min, max) = mesh.bounds()?;
    renderer.add_mesh(
        gpu_vertices(&mesh.vertices),
        &mesh.indices,
        &mesh.translucent_indices,
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

fn main() -> Result<()> {
    // Logs visibles avec `RUST_LOG=info cargo run -p voxel`.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,voxel=info"))
        .init();

    let trace = start_tracing();
    let result = engine_core::run::<App>(AppConfig {
        title: "Voxel".into(),
        ..AppConfig::default()
    });
    if let Some(path) = trace {
        match engine_trace::write_chrome_trace(&path) {
            Ok(s) => log::info!(
                "trace : {} étapes sur {} threads dans {} ({} perdues) ; à ouvrir dans https://ui.perfetto.dev",
                s.events,
                s.threads,
                path.display(),
                s.dropped
            ),
            Err(err) => log::error!("trace : écriture de {} impossible : {err}", path.display()),
        }
    }
    result
}

/// `VOXEL_TRACE=fichier.json` : chronomètre tous les threads et écrit la trace à la
/// fermeture. `VOXEL_SPIKES=ms` : détaille dans les logs chaque image plus longue.
fn start_tracing() -> Option<PathBuf> {
    if let Some(ms) = env_parse::<f64>("VOXEL_SPIKES").filter(|ms| *ms > 0.0) {
        engine_trace::set_spike_threshold(Some(std::time::Duration::from_secs_f64(ms / 1000.0)));
        log::info!("images lentes : détail au-delà de {ms} ms");
    }
    let path = PathBuf::from(std::env::var_os("VOXEL_TRACE")?);
    engine_trace::start_recording();
    log::info!(
        "trace allumée, écrite à la fermeture dans {}",
        path.display()
    );
    Some(path)
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
            color: [0.5, 0.25, 0.125, 0.6],
            light: [1.0, 0.5, 0.25],
            texture: 7,
        };
        let gpu = gpu_vertices(std::slice::from_ref(&vertex))[0];
        assert_eq!(gpu.position, vertex.position);
        assert_eq!(gpu.normal, vertex.normal);
        assert_eq!(gpu.color, vertex.color);
        assert_eq!(gpu.light, vertex.light);
        assert_eq!(gpu.texture, vertex.texture);
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

    #[test]
    fn game_finds_core_and_mod_blocks() {
        let order = modding::resolve_order(&assets_dir());
        let registry = load_registry(&modding::mod_dirs(&order)).unwrap();
        let hotbar: Vec<&str> = registry
            .placeable()
            .map(|id| registry.def(id).key.as_str())
            .collect();
        assert_eq!(hotbar.first(), Some(&"core:grass"));
        assert!(hotbar.contains(&"example:ruby"), "{hotbar:?}");
    }
}
