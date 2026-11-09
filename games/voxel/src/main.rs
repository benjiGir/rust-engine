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
mod components;
mod daytime;
mod events;
mod hot_reload;
mod inventory;
mod mobs;
mod modding;
mod persistence;
mod player;
mod screenshots;
mod shared;
mod stats;
mod stress;
mod ui;

use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::path::PathBuf;
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
}

impl Game for VoxelGame {
    fn new(ctx: &mut Context) -> Result<Self> {
        // `?` remonte l'erreur au moteur, qui l'affichera à la fermeture.
        let mut renderer = Renderer::new(ctx.window.clone())?;
        renderer.crosshair = true;

        // L'ordre des mods (chacun après ses dépendances) fixe l'ordre des registres.
        let assets = assets_dir();
        let mod_order = modding::resolve_order(&assets);
        let mod_dirs = modding::mod_dirs(&mod_order);
        let registry = Arc::new(load_registry(&mod_dirs)?);
        let block_textures = assets::load_textures(&registry);
        renderer.set_block_textures(&block_textures);
        let mob_registry =
            MobRegistry::load(&assets, &mod_dirs, &registry).context("chargement des créatures")?;
        log::info!("{} types de créatures", mob_registry.len());

        // Les modes automatiques n'écrivent pas dans le monde du joueur, sauf demande explicite.
        let automatic = ["VOXEL_DEMO", "VOXEL_AUTOFLY", "VOXEL_SCREENSHOTS"]
            .iter()
            .any(|v| std::env::var(v).is_ok());
        let (store, meta) = if automatic && std::env::var("VOXEL_WORLD").is_err() {
            log::info!("mode automatique : pas de sauvegarde");
            (None, None)
        } else {
            let (store, meta) = persistence::open_world(persistence::world_dir(&assets_dir()))?;
            (Some(store), meta)
        };
        if let Some(meta) = &meta {
            persistence::check_mods(meta, &registry);
        }

        // Un monde existant garde sa graine.
        let seed = meta
            .as_ref()
            .map(|m| m.seed)
            .or_else(|| env_parse("VOXEL_SEED"))
            .unwrap_or(DEFAULT_SEED);
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
        let generator = TerrainGenerator::with_registry(seed, registry.clone())?;
        let mut streamer = ChunkStreamer::new(generator, config);
        if let Some(store) = &store {
            streamer = streamer.with_save(store.clone());
        }

        // `std::env::var` renvoie `Err` si la variable n'existe pas : `.ok()` en fait un `Option`.
        let autofly = std::env::var("VOXEL_AUTOFLY").ok().map(|v| {
            let speed: f32 = v.parse().unwrap_or(0.0);
            // `VOXEL_AUTOFLY=1` veut dire « oui », pas « 1 bloc par seconde ».
            if speed <= 1.0 {
                FLY_SPEED * FLY_SPRINT_FACTOR
            } else {
                speed
            }
        });
        let demo = std::env::var("VOXEL_DEMO").ok().map(|_| Demo::new());
        let tour = std::env::var("VOXEL_SCREENSHOTS")
            .ok()
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
        let mods = modding::ModSystem::new(&mod_order, world.clone(), registry.clone());
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
            pitch,
            far: view_distance * 1.5,
            ..Camera::default()
        };
        // L'inventaire sauvegardé, ou un petit nécessaire de départ pour un nouveau monde.
        let mut inventory = match meta.as_ref().filter(|m| !m.inventory.is_empty()) {
            Some(meta) => Inventory::from_save(&meta.inventory, &registry),
            None => starter_inventory(&registry),
        };
        if let Some(key) = meta
            .as_ref()
            .and_then(|m| m.player.as_ref()?.selected_block.as_deref())
            && let Some(id) = registry.id(key)
            && let Some(slot) =
                (0..inventory::HOTBAR).find(|&i| inventory.slots[i].is_some_and(|s| s.item == id))
        {
            inventory.selected = slot;
        }
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

        let audio = audio::Audio::new(&registry, audio::AudioSettings::from_env());
        Ok(Self {
            renderer,
            camera,
            mouse_captured: false,
            ecs,
            schedule,
            registry,
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
            hot_reload: start_hot_reload(),
            audio,
            step_distance: 0.0,
            was_in_liquid: false,
            quit_at: env_parse::<u64>("VOXEL_QUIT_AFTER")
                .map(|s| Instant::now() + std::time::Duration::from_secs(s)),
            frame_stats: FrameStats::new(),
            last_render: RenderStats::default(),
        })
    }

    /// La simulation, à 60 Hz exactement : les entrées du joueur, puis les systèmes.
    fn fixed_update(&mut self, ctx: &mut Context) {
        let input = &ctx.input;
        let busy = self.ui.inventory_open;
        let (walk, jump) = match &self.demo {
            Some(_) => Demo::walk(&self.ecs, self.player, &self.camera),
            None if busy => (Vec3::ZERO, false),
            None => {
                let speed = if input.key_held(KeyCode::ControlLeft) {
                    SPRINT_SPEED
                } else {
                    WALK_SPEED
                };
                let direction = self.wish_direction(ctx, false);
                (direction * speed, input.key_held(KeyCode::Space))
            }
        };
        player::set_intent(&self.ecs, self.player, walk, jump);
        self.schedule.run(Stage::FixedUpdate, &mut self.ecs);
        // Les machines : un tick tous les 3 pas (20 par seconde).
        let (actions, machine_tick) = self.mods.fixed_step();
        self.apply_machine_actions(actions);
        if let Some(stress) = &mut self.stress {
            let eye = player::eye(&self.ecs, self.player, 1.0);
            let ready = self.world.borrow().is_idle();
            stress.build_if_ready(ready, &mut self.ecs, &mut self.mods, eye);
            let start = Instant::now();
            stress::conveyors(&mut self.ecs);
            stress.record(machine_tick.map(|t| t.duration), start.elapsed());
        }
        self.footsteps(ctx.fixed_dt);
        self.respawn_if_dead();
    }

    fn window_event(&mut self, ctx: &mut Context, event: &WindowEvent) -> bool {
        self.ui.on_window_event(&ctx.window, event)
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        // E : l'inventaire. Ouvert, il libère la souris ; fermé, la pile tenue est rangée.
        let close = self.ui.inventory_open && ctx.input.key_pressed(KeyCode::Escape);
        if ctx.input.key_pressed(KeyCode::KeyE) || close {
            self.ui.inventory_open = !self.ui.inventory_open;
            if self.ui.inventory_open {
                self.mouse_captured = false;
                ctx.set_cursor_captured(false);
            } else if let Some(held) = self.inventory.cursor.take() {
                self.inventory.add(held.item, held.count);
            }
        }
        let click_used = if self.ui.inventory_open {
            true
        } else {
            self.handle_mouse_capture(ctx)
        };
        if self.mouse_captured {
            self.look(ctx.input.mouse_delta());
        }
        self.handle_keys(ctx);
        self.day.update(dt);
        if let Some(audio) = &mut self.audio {
            audio.update_ambience(self.day.daylight());
        }

        if player::mode(&self.ecs, self.player) == MoveMode::Fly {
            match self.autofly {
                Some(speed) => self.autofly(speed, dt),
                None => {
                    let mut speed = FLY_SPEED;
                    if ctx.input.key_held(KeyCode::ControlLeft) {
                        speed *= FLY_SPRINT_FACTOR;
                    }
                    let direction = if self.ui.inventory_open {
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
        if self.mouse_captured && !click_used {
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
                        mobs::damage_mob(&mut self.ecs, mob, PUNCH_DAMAGE, self.camera.forward());
                        if let Some(audio) = &mut self.audio {
                            audio.play("core:break_soft");
                        }
                    }
                    None => {
                        self.break_target();
                    }
                }
            }
            if ctx.input.mouse_pressed(MouseButton::Right) {
                self.place_at_target();
            }
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
        let mob_boxes = mobs::instances(&self.ecs, alpha);
        self.renderer.set_instances(&mob_boxes);
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
}

impl VoxelGame {
    /// Direction voulue d'après les touches. Horizontale en marche ; en vol,
    /// Espace et Maj ajoutent la verticale.
    fn wish_direction(&self, ctx: &Context, vertical: bool) -> Vec3 {
        let input = &ctx.input;
        let mut bindings = vec![
            (KeyCode::KeyW, self.camera.forward_flat()),
            (KeyCode::KeyS, -self.camera.forward_flat()),
            (KeyCode::KeyD, self.camera.right()),
            (KeyCode::KeyA, -self.camera.right()),
        ];
        if vertical {
            bindings.push((KeyCode::Space, Vec3::Y));
            bindings.push((KeyCode::ShiftLeft, -Vec3::Y));
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
        if input.key_pressed(KeyCode::KeyT) {
            self.day.cycle_speed();
            log::info!("temps {} ({})", self.day.speed.name(), self.day.clock());
        }
        if input.key_pressed(KeyCode::KeyF) {
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
        if input.key_pressed(KeyCode::F1) {
            let mesher = self.streamer().config().mesher.other();
            log::info!("maillage {} : on remaille tout", mesher.name());
            self.streamer_mut().set_mesher(mesher);
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
        self.edit(hit.block_pos, BlockId::AIR)
            .then_some(hit.block_pos)
    }

    /// Pose le bloc de la case choisie contre la face visée. Renvoie sa position si c'est fait.
    fn place_at_target(&mut self) -> Option<IVec3> {
        let hit = self.target?;
        let block = self.inventory.selected_item()?;
        if !self.registry.def(block).placeable {
            return None;
        }
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
        let placed = self.edit(pos, block);
        if placed {
            self.inventory.consume_selected();
        }
        placed.then_some(pos)
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
                // Le bloc cassé va dans l'inventaire (s'il se pose : pas l'eau).
                if self.registry.def(old).placeable {
                    self.inventory.add(old, 1);
                }
            } else {
                events::send(&self.ecs, BlockPlaced { pos, block });
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
                    let left = self.inventory.add(block, count);
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
        let mut uploads = 0;
        while uploads < UPLOAD_BUDGET {
            let Some(event) = self.streamer_mut().next_event() else {
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
            "{} | {}{frozen} | {} entités | {} chunks | {pending} en attente | {}/{} dessinés | {triangles:.2} M tri",
            self.day.clock(),
            player::mode(&self.ecs, self.player).name(),
            self.ecs.len(),
            stream.loaded,
            render.meshes_drawn,
            render.meshes_total,
        );

        if let Some(report) = self.frame_stats.tick() {
            self.debug = ui::DebugInfo {
                fps: 1000.0 / report.avg_frame_ms.max(0.001),
                frame_ms: report.avg_frame_ms,
                max_frame_ms: report.max_frame_ms,
            };
            log::info!(
                "image : moy {:.2} ms, max {:.2} ms | streaming : max {:.2} ms, envois GPU : max {:.2} ms ({} au total) | \
                 rendu : max {:.2} ms | modifs : {} (max {:.2} ms) | chunks : {} chargés, {} à générer, {} en cours | \
                 dessinés {}/{}, {triangles:.2} M triangles",
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
                render.meshes_drawn,
                render.meshes_total,
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
        self.camera.yaw += delta.x * MOUSE_SENSITIVITY;
        self.camera.pitch = (self.camera.pitch - delta.y * MOUSE_SENSITIVITY)
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
            .map_or("?", |b| self.registry.def(b).key.as_str());
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
fn starter_inventory(registry: &BlockRegistry) -> Inventory {
    let mut inventory = Inventory::default();
    for (key, count) in [
        ("core:torch", 32),
        ("core:glass", 16),
        ("core:glowstone", 8),
        ("core:stone", 32),
        ("example:ruby", 4),
        // De quoi construire un mécanisme : horloge → lampe, horloge → casseur.
        ("core:clock", 8),
        ("core:lamp", 8),
        ("example:breaker", 4),
    ] {
        if let Some(id) = registry.id(key) {
            inventory.add(id, count);
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
