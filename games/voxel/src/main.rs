//! Le futur mini-Minecraft.
//!
//! Étape actuelle (« mini-Minecraft v1 ») : un monde infini où l'on marche, casse et
//! pose des blocs avec un inventaire, au milieu de créatures. Le joueur et les créatures
//! sont des entités de l'ECS (`engine_ecs`). Les blocs et les créatures viennent de
//! `assets/core/` et des mods (`mods/<nom>/`).
//! - Clic gauche : capturer la souris, puis casser le bloc visé ou frapper une créature.
//!   Clic droit : poser le bloc choisi. Touches 1 à 9 : choisir la case de la barre.
//! - E : ouvrir l'inventaire (déplacer les piles à la souris). B : le sac à dos. F3 : l'inspecteur.
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
//! - `VOXEL_TEXTURE_PACK=classic|earth32|earth64` : impose le pack de textures au lancement.
//! - `VOXEL_RADIUS=24` : le rayon de vue en chunks (16 par défaut).
//! - `VOXEL_ASSETS=chemin` : le dossier `assets` (les mods sont dans `../mods`).
//! - `VOXEL_AUTOFLY=1` : vol automatique, pour mesurer le chargement.
//!   `VOXEL_AUTOFLY=100` : à 100 blocs par seconde.
//! - `VOXEL_TIME=0.75` : l'heure de départ (0 minuit, 0,25 lever, 0,5 midi, 0,75 coucher).
//! - `VOXEL_DAY_LENGTH=120` : la durée d'une journée en secondes (600 par défaut).
//! - `VOXEL_WORLD=nom` : le monde à ouvrir dans `saves/` (« world » par défaut). Il est
//!   sauvegardé toutes les 30 secondes et à la fermeture.
//! - `VOXEL_PIPELINE=1` : journalise toutes les 2 secondes les délais des chunks (génération,
//!   lumière, maillage, envoi au GPU : médiane, 95e centile et maximum).
//! - `VOXEL_SAVES=dossier` : la racine des sauvegardes à la place de `saves/` (isoler une séance
//!   de test des vraies sauvegardes). Ne déplace pas `settings.ron` ni `mod_settings.ron`.
//! - `VOXEL_MUTE=1` : couper le son. `VOXEL_VOLUME_EFFECTS=0.8`, `VOXEL_VOLUME_AMBIENCE=0.35` :
//!   volumes des effets et de l'ambiance, de 0 à 1.
//! - `VOXEL_QUIT_AFTER=20` : quitter proprement après 20 secondes (pour les tests).
//! - `VOXEL_SCREENSHOTS=dossier` : enregistre des captures (midi, coucher, nuit, torches,
//!   lac, grotte) dans ce dossier, puis quitte. Marche même écran éteint.
//! - `VOXEL_DEMO=1` : le joueur marche, saute, casse et pose des blocs tout seul
//!   (macOS empêche les outils de test d'envoyer des touches).

mod adventure;
mod adventure_ctl;
mod adventure_data;
mod affixes;
mod aquatic;
mod assets;
mod attributes;
mod attributes_ui;
mod audio;
mod backpack_block;
mod backpacks;
mod bench;
mod book_play;
mod breeding;
mod bucket_play;
mod cart_play;
mod carts;
mod charm_play;
mod charms;
mod chest_lids;
mod chest_preview;
mod client_entities;
mod client_side;
mod client_world;
mod components;
mod console;
mod container_view;
mod content;
mod control;
mod culling;
mod daytime;
mod death;
mod dimensions;
mod drops;
mod elites;
mod enchant_books;
mod enchant_ctl;
mod enchant_data;
mod enchant_effects;
mod enchant_library;
mod enchant_table;
mod enchant_ui;
mod enchanting;
mod entities;
mod entity_render;
mod events;
mod far_terrain;
mod forge_tables;
mod forge_ui;
mod gear;
mod gem_sockets;
mod gems;
mod ghosts;
mod guide;
mod guide_ui;
mod hand;
mod hand_model;
mod hot_reload;
mod intents;
mod interact;
mod item_ctl;
mod launch;
mod net_ctl;
mod net_host;
mod predicted;
mod server_boot;
mod server_sends;
mod server_side;
mod server_step;
mod sky_view;
mod stream_upload;
mod texture_packs;
mod wire;
pub use launch::{Launch, Request};
mod conduit_fx;
mod conduit_render;
mod inventory;
mod item_art;
mod item_data;
mod item_models;
mod menus;
mod mob_brains;
mod mob_model;
mod mobs;
mod mod_menu;
mod modding;
mod movement;
mod navigation;
mod paired_play;
mod persistence;
mod play_input;
mod player;
mod player_api;
mod player_view;
mod players;
mod potions;
mod projectiles;
mod protocol;
mod quest_ctl;
mod quest_play;
mod quest_progress;
mod quest_ui;
mod quests;
mod recipe_browser;
mod screen_actions;
mod screenshots;
mod settings;
mod shader_packs;
mod shape_ctl;
mod shape_materials;
mod shaped_play;
mod shapes_scene;
mod shaping_ui;
mod shared;
mod sheep_colors;
mod ships;
mod skin;
mod soundscape;
mod spawning;
mod stats;
mod storage_ctl;
mod storage_ui;
mod stress;
mod survival;
mod tiers;
mod tiers_ui;
mod trading;
mod transfer;
mod ui;
mod ui_info;
mod ui_input;
mod ui_layout;
mod ui_options;
mod ui_target;
mod ui_widgets;
mod villagers;
mod water_fx;
mod weather;

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

use engine_render::{Aabb, Camera, ColorVertex, Fog, MeshId, RenderStats, Renderer, Sun};
use engine_voxel::{
    BlockId, BlockRegistry, CHUNK_SIZE, ChunkMesh, ChunkStreamer, MachineAction, MeshVertex,
    RayHit, StreamingConfig, raycast,
};
use glam::{IVec3, Vec2, Vec3};

use client_side::ClientSide;
use components::{Collider, Transform};
use daytime::DayCycle;
use events::{BlockBroken, BlockPlaced, EntityDied, PlayerDamaged};
use inventory::Inventory;
use mobs::MobRegistry;
use player::{FLY_SPEED, FLY_SPRINT_FACTOR, MoveMode, SPRINT_SPEED, WALK_SPEED};
use server_side::VoxelServer;
use settings::Action;
use shared::StreamerExt;
use stats::FrameStats;

const MOUSE_SENSITIVITY: f32 = 0.0025;
const DEFAULT_SEED: u32 = 42;
/// Hauteur de vol au départ et en mode `VOXEL_AUTOFLY`, au-dessus du sol.
const FLY_ALTITUDE: f32 = 30.0;
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
    /// Les objets, recettes, fonte et butin.
    /// Menu principal, jeu ou pause.
    mode: Mode,
    settings: settings::Settings,
    /// Le lancement du jeu (pour animer le vent).
    started: Instant,
    /// La surveillance des fichiers (shaders, textures, sons, blocs).
    hot_reload: Option<hot_reload::HotReload>,
    /// `VOXEL_CONTROL` : la marche et les `wait` commandés (voir `control.rs`).
    remote: control::Remote,
    /// Ce qui est à la fenêtre (rendu, interface, son, réplique, prédiction) : voir `ClientSide`.
    client: ClientSide,
    /// Ce qui fait vivre le monde (ECS, chunks, mods, joueurs...) : voir `VoxelServer`.
    server: VoxelServer,
}

/// Où en est le jeu : le menu principal (un monde de fond), le jeu, ou la pause.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Title(menus::TitlePage),
    Playing,
    Paused(menus::PausePage),
    /// L'écran de mort, avec sa cause.
    Dead(String),
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

        // Le serveur d'abord, sans fenêtre (voir `server_boot.rs`), puis ce que la fenêtre en tire.
        let boot = VoxelServer::boot(
            &launch,
            &settings,
            ctx.fixed_dt,
            autofly.is_some(),
            demo.is_some(),
        )?;
        let (server, yaw, pitch, seed) = (boot.server, boot.yaw, boot.pitch, boot.seed);
        let registry = server.registry.clone();
        let assets = assets_dir();
        let block_textures = assets::load_textures(&registry);
        renderer.set_block_textures(&block_textures);
        let config = server.world.borrow().config().clone();
        // Le terrain lointain (simplifié) prolonge le monde au-delà des chunks détaillés ;
        // `VOXEL_FAR=0` le coupe, pour comparer. Pas dans le décor du menu principal.
        let far_chunks = if launch.title {
            0
        } else {
            env_parse("VOXEL_FAR")
                .unwrap_or(settings.far_distance)
                .clamp(0, 192)
        };
        let far_chunks = if far_chunks > config.view_radius {
            far_chunks
        } else {
            0
        };
        // Le brouillard termine juste avant le bord de ce qu'on dessine.
        let drawn_chunks = if far_chunks > 0 {
            far_chunks
        } else {
            config.view_radius
        };
        let view_distance = (drawn_chunks * CHUNK_SIZE as i32) as f32;
        renderer.fog = Some(Fog {
            start: view_distance * 0.6,
            end: view_distance * 0.95,
        });
        let near_radius = config.view_radius;
        let far = (far_chunks > 0).then(|| {
            let far = far_terrain::FarTerrain::new(&server.world.borrow(), near_radius, far_chunks);
            log::info!("terrain lointain : jusqu'à {far_chunks} chunks");
            far
        });
        let mut view = client_world::ClientWorld::new(&server.world.borrow());
        // `VOXEL_PIPELINE=1` : les délais d'un chunk reçu → maillage → envoi (la réplique).
        view.replica_mut()
            .set_pipeline_timing(std::env::var("VOXEL_PIPELINE").is_ok_and(|v| v == "1"));
        // La fenêtre simule son joueur à partir de là où le serveur l'a placé (voir `predicted.rs`).
        let body = predicted::PredictedBody::new(
            player::body(&server.ecs, server.players.entity()).position,
            player::mode(&server.ecs, server.players.entity()),
        );
        let camera = Camera {
            position: body.eye(1.0),
            yaw,
            pitch: if launch.title { -0.25 } else { pitch },
            far: view_distance * 1.5,
            fov_y: settings.fov.to_radians(),
            ..Camera::default()
        };
        renderer.set_vsync(settings.vsync);
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
            mode: if launch.title {
                Mode::Title(menus::TitlePage::Main)
            } else {
                Mode::Playing
            },
            settings,
            started: Instant::now(),
            hot_reload: start_hot_reload(),
            remote: control::Remote::default(),
            client: ClientSide {
                last_look: (f32::NAN, f32::NAN),
                renderer,
                camera,
                mouse_captured: false,
                hand,
                request: None,
                waiting_key: None,
                worlds: Vec::new(),
                view,
                meshes: HashMap::new(),
                block_textures,
                ui: ui::GameUi::new(&ctx.window),
                debug: ui::DebugInfo::default(),
                target: None,
                autofly,
                demo,
                tour,
                water_fx: Default::default(),
                conduit_fx: Default::default(),
                far,
                grid_meshes: ships::GridMeshes::default(),
                soundscape: soundscape::Soundscape::new(seed),
                enchant_ui: enchant_ui::EnchantUi::default(),
                music_until: None,
                base_fog: (view_distance * 0.6, view_distance * 0.95),
                audio,
                chest_lids: chest_lids::ChestLids::default(),
                step_distance: 0.0,
                was_in_liquid: false,
                quit_at: env_parse::<u64>("VOXEL_QUIT_AFTER")
                    .map(|s| Instant::now() + std::time::Duration::from_secs(s)),
                frame_stats: FrameStats::new(),
                uploads: stream_upload::Uploads::new(),
                last_render: RenderStats::default(),
                culling: culling::CaveCulling::from_env(),
                bench: bench::Bench::from_env(),
                outbox: Vec::new(),
                frame_dt: 0.0,
                eating_sent: false,
                body,
                mining: None,
                air_swing: false,
                steer_sent: 0.0,
                day: server.day.clone(),
                weather: weather::Weather::follower(
                    seed,
                    server.weather.sky,
                    server.weather.overcast,
                ),
                registry: server.registry.clone(),
                content: server.content.clone(),
            },
            server,
        })
    }

    /// La simulation, à 60 Hz exactement : les entrées du joueur, puis les systèmes.
    fn fixed_update(&mut self, ctx: &mut Context) {
        match self.mode {
            // En pause, le monde s'arrête.
            Mode::Paused(_) => return,
            // Le monde du menu vit (créatures, machines), sans joueur aux commandes ; de
            // même quand le joueur est mort.
            Mode::Title(_) | Mode::Dead(_) => {
                player::set_intent(
                    &self.server.ecs,
                    self.server.players.entity(),
                    Vec3::ZERO,
                    false,
                );
                self.server
                    .schedule
                    .run(Stage::FixedUpdate, &mut self.server.ecs);
                self.server.step_fluids();
                self.replicate_entities();
                return;
            }
            Mode::Playing => {}
        }
        self.server.sim_step += 1;
        // `VOXEL_CONTROL` : une marche commandée passe avant le clavier.
        let remote = self.remote_intent(ctx.fixed_dt);
        let input = &ctx.input;
        let keys = &self.settings.keys;
        let busy = self.client.ui.inventory_open;
        let (walk, jump) = match &self.client.demo {
            _ if remote.is_some() => remote.unwrap_or_default(),
            Some(_) => Demo::walk(
                &self.server.ecs,
                self.server.players.entity(),
                &self.client.camera,
            ),
            None if busy => (Vec3::ZERO, false),
            None => {
                // Affamé, on ne court plus (les jauges viennent du serveur, voir `player_view.rs`).
                let vitals = self.client.view.me.vitals.as_ref();
                let can_sprint = vitals.is_none_or(|v| v.can_sprint);
                let speed = if input.key_held(keys.key(Action::Sprint)) && can_sprint {
                    SPRINT_SPEED
                } else {
                    WALK_SPEED
                } * vitals.map_or(1.0, |v| v.move_speed);
                let jump = input.key_held(keys.key(Action::Jump));
                let direction = self.wish_direction(ctx, false);
                (direction * speed, jump)
            }
        };
        // La fenêtre joue le mouvement de son joueur et l'envoie (voir `predicted.rs`).
        self.predict_move(walk, jump, ctx.fixed_dt);
        self.server.fixed_step(ctx.fixed_dt);
        self.footsteps(ctx.fixed_dt);
        self.apply_replies();
    }

    fn window_event(&mut self, ctx: &mut Context, event: &WindowEvent) -> bool {
        // La fenêtre perd le focus en pleine partie : le jeu se met en pause et rend la souris, comme
        // Minecraft (on la reprend d'un clic sur « Reprendre », jamais par surprise).
        // (Pas dans les modes automatiques : captures, banc d'essai, démo, qui tournent sans focus.)
        let automatic =
            self.client.tour.is_some() || self.client.bench.is_some() || self.client.demo.is_some();
        if *event == WindowEvent::Focused(false) && self.mode == Mode::Playing && !automatic {
            self.mode = Mode::Paused(menus::PausePage::Main);
            self.client.mouse_captured = false;
            ctx.set_cursor_captured(false);
        }
        self.client.ui.on_window_event(&ctx.window, event)
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        self.client.frame_dt = dt;
        // Les joueurs du réseau local : arrivées, messages, départs (voir `net_host.rs`).
        self.server.poll_net();
        // Les options attendent une touche : la prochaine pressée est la nouvelle
        // (Échap annule).
        let mut escape = ctx.input.key_pressed(KeyCode::Escape);
        if let Some(action) = self.client.waiting_key
            && let Some(key) = ctx.input.keys_pressed().next()
        {
            if key != KeyCode::Escape {
                self.settings.keys.set(action, key);
            }
            self.client.waiting_key = None;
            escape = false;
        }
        let interactive = self.mode == Mode::Playing;
        match &self.mode {
            Mode::Playing if escape && !self.client.ui.inventory_open => {
                self.mode = Mode::Paused(menus::PausePage::Main);
                escape = false;
            }
            Mode::Paused(menus::PausePage::Main) if escape => {
                self.mode = Mode::Playing;
                escape = false;
            }
            Mode::Paused(_) if escape => {
                self.leave_mod_options();
                self.mode = Mode::Paused(menus::PausePage::Main);
                escape = false;
            }
            _ => {}
        }
        if !interactive || self.mode != Mode::Playing {
            // Un menu est ouvert : la souris est libre.
            if self.client.mouse_captured {
                self.client.mouse_captured = false;
                ctx.set_cursor_captured(false);
            }
            self.stop_mining();
        }
        if let Mode::Title(_) = self.mode {
            // Le survol du menu principal tourne lentement.
            self.client.camera.yaw += dt * 0.03;
        }
        let interactive = self.mode == Mode::Playing;
        self.client.ui.menu_open = !interactive;
        self.update_console(ctx);
        // E : l'inventaire. Ouvert, il libère la souris ; fermé, la pile tenue est rangée.
        // Échap ferme d'abord la fenêtre des recettes, puis l'écran.
        if escape && self.client.ui.inventory_open && self.client.ui.browser.view.is_some() {
            self.client.ui.browser.close();
            escape = false;
        }
        // On tape dans un champ (la recherche des recettes) : les touches ne sont pas des
        // commandes du jeu.
        let typing = self.client.ui.ctx.egui_wants_keyboard_input();
        let close = self.client.ui.inventory_open && escape;
        let toggle = interactive
            && !self.console_open()
            && !typing
            && ctx
                .input
                .key_pressed(self.settings.keys.key(Action::Inventory));
        if toggle || close {
            self.client.ui.inventory_open = !self.client.ui.inventory_open;
            if self.client.ui.inventory_open {
                self.client.mouse_captured = false;
                ctx.set_cursor_captured(false);
            } else {
                self.close_screen();
            }
            self.client.ui.screen = ui::Screen::Inventory;
        }
        let click_used = if self.client.ui.inventory_open || !interactive {
            true
        } else {
            self.handle_mouse_capture(ctx)
        };
        if self.client.mouse_captured {
            self.look(ctx.input.mouse_delta());
        }
        if interactive && !self.console_open() && !typing {
            self.handle_keys(ctx);
        }
        self.update_water_fx(dt);
        self.client.conduit_fx.advance(dt);
        self.update_soundscape(dt);
        self.update_ships(ctx);
        self.update_carts(ctx);
        self.update_sky(dt);
        self.update_remote(dt);
        if let Some(audio) = &mut self.client.audio {
            audio.update_ambience(self.client.day.daylight());
        }

        if self.client.body.mode == MoveMode::Fly {
            match self.client.autofly {
                Some(speed) => self.autofly(speed, dt),
                None => {
                    let mut speed = FLY_SPEED;
                    if ctx.input.key_held(self.settings.keys.key(Action::Sprint)) {
                        speed *= FLY_SPRINT_FACTOR;
                    }
                    let direction = if self.client.ui.inventory_open || !interactive {
                        Vec3::ZERO
                    } else {
                        self.wish_direction(ctx, true)
                    };
                    self.client.body.fly(direction, speed, dt);
                }
            }
        }

        // Les entrées de la souris : le bloc visé, le minage, les coups, le clic droit, le repas.
        // Elles ne font que fabriquer des messages, appliqués avant la fin de cette image
        // (voir `play_input.rs` et `intents.rs`).
        self.play_input(ctx, dt, click_used);
        if interactive {
            self.animate_hand(ctx, dt);
        }
        // Un écran vient de s'ouvrir (établi, four, coffre) : on libère la souris.
        if self.client.ui.inventory_open && self.client.mouse_captured {
            self.client.mouse_captured = false;
            ctx.set_cursor_captured(false);
        }
        if self.client.demo.is_some() {
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
        if self.client.quit_at.is_some_and(|t| Instant::now() >= t) {
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

        // Le travail du serveur à chaque image : l'heure, la météo, les villages, les systèmes de
        // l'image et les événements (voir `server_step.rs`).
        // En pause, le monde s'arrête : l'heure, les potions et les villages aussi.
        if !matches!(self.mode, Mode::Paused(_)) {
            self.server.frame(dt);
        }

        // Tout le travail du monde côté thread principal : on le chronomètre.
        let start = Instant::now();
        let eye = self.client.body.eye(1.0);
        self.server.stream_world();
        self.update_far(eye);
        self.sync_world_view();
        let streamed = Instant::now();
        let uploads = self.apply_stream_events();
        self.client
            .frame_stats
            .record_streaming(streamed - start, streamed.elapsed(), uploads);
        self.server.end_net_frame();
    }

    fn render(&mut self, ctx: &mut Context, alpha: f32) {
        // Entre deux pas de physique : la caméra est interpolée (voir `player.rs`).
        self.client.camera.position = self.client.body.eye(alpha);
        // Le vent dans les feuilles suit le temps réel depuis le lancement.
        self.client.renderer.time = self.started.elapsed().as_secs_f32();
        // Les entités viennent de la réplique du client : créatures, autres joueurs, projectiles,
        // wagons et objets au sol (voir `entity_render.rs`).
        self.client.view.entities.begin_frame(alpha);
        let mobs = self.server.ecs.resource::<Arc<MobRegistry>>().clone();
        let mut mob_boxes = Vec::new();
        entity_render::draw(
            &self.client.view.entities,
            &entity_render::DrawEnv {
                mobs: &mobs,
                items: &self.client.content.items,
                blocks: &self.client.registry,
                world: self.client.view.world(),
                alpha,
                time: self.client.renderer.time,
            },
            &mut mob_boxes,
        );
        self.client.weather.instances(&mut mob_boxes);
        self.client
            .water_fx
            .instances(self.client.camera.position, &mut mob_boxes);
        self.draw_conduit_contents(&mut mob_boxes);
        self.direction_hint(&mut mob_boxes);
        self.client
            .renderer
            .set_instances_at(&mob_boxes, self.client.camera.position);
        // Le bras, éclairé par la lumière là où se tient le joueur.
        let (sky, block) = self
            .view_world()
            .light(self.client.camera.position.floor().as_ivec3());
        let held = self
            .client
            .view
            .me
            .inventory
            .selected_stack()
            .map(|s| s.item);
        let size = ctx.window.inner_size();
        self.client
            .hand
            .set_aspect_ratio(size.width as f32 / size.height.max(1) as f32);
        // Mort : plus de bras.
        let arm = if matches!(self.mode, Mode::Dead(_)) {
            Vec::new()
        } else {
            self.client
                .hand
                .instances(held, &self.client.content.items, &self.client.registry)
        };
        self.client
            .renderer
            .set_hand(&arm, [sky as f32 / 15.0, block as f32 / 15.0]);
        // L'heure décide du soleil et de la couleur du ciel, donc du brouillard.
        self.client.renderer.sun = Sun {
            direction: self.client.day.sun_direction(),
            daylight: self.client.day.daylight() * self.client.weather.daylight_factor(),
            color: self.client.day.sun_color(),
        };
        self.client.renderer.sky = self
            .server
            .day
            .has_daylight_cycle()
            .then(|| self.client.weather.sky());
        let fog_scale = self.client.weather.fog_scale();
        self.client.renderer.fog = Some(Fog {
            start: self.client.base_fog.0 * fog_scale,
            end: self.client.base_fog.1 * fog_scale,
        });
        let sky = self
            .server
            .weather
            .tint_sky(self.client.day.sky_color())
            .as_dvec3();
        // Sous l'eau, le « ciel » (le fond de ce qu'on voit) est le bleu sombre de l'eau.
        let (underwater, surface) = self.client.water_fx.underwater();
        let day = f64::from(0.25 + 0.75 * self.client.day.daylight());
        let deep = glam::DVec3::new(0.015, 0.11, 0.17) * day;
        let sky = sky.lerp(deep, f64::from(underwater));
        self.client.renderer.underwater = underwater;
        self.client.renderer.water_surface = surface;
        self.client.renderer.clear_color = engine_render::wgpu_color(sky.x, sky.y, sky.z);
        self.client.renderer.outline = self.client.target.map(|hit| {
            let point = self.client.camera.position + self.client.camera.forward() * hit.distance;
            let (min, max) = self.view_world().selection_bounds(hit.block_pos, point);
            Aabb::new(min, max)
        });

        // L'interface de cette image.
        let raw = self
            .client
            .ui
            .take_input(&ctx.window, self.settings.ui_scale);
        let (ui_frame, platform) = self.build_ui(raw);
        self.flush_mod_ui();
        self.client.ui.handle_output(&ctx.window, platform);
        self.update_chest_lids();

        // Les structures mobiles (bateaux, contraptions) viennent aussi de la réplique.
        self.client.grid_meshes.sync(
            &mut self.client.renderer,
            &self.client.view.entities.grids(),
        );
        self.update_visibility(self.client.renderer.aspect());
        let start = Instant::now();
        let result = self
            .client
            .renderer
            .render(&self.client.camera, Some(ui_frame));
        self.client.frame_stats.record_render(start.elapsed());
        match result {
            Ok(Some(stats)) => self.client.last_render = stats,
            // Image sautée : on garde les statistiques précédentes.
            Ok(None) => {}
            Err(err) => log::error!("erreur de rendu : {err:#}"),
        }
        self.show_stats(ctx);
    }

    fn resized(&mut self, _ctx: &mut Context, size: PhysicalSize<u32>) {
        self.client.renderer.resize(size.width, size.height);
    }

    fn exit(&mut self, _ctx: &mut Context) {
        self.server.save_now();
    }

    /// Sauvegarde et décharge le monde ; rend le renderer pour le monde suivant.
    fn shutdown(mut self) -> Renderer {
        self.server.save_now();
        self.client.renderer.clear_meshes();
        self.client.renderer.set_visible_meshes(None);
        self.client.renderer.set_instances(&[]);
        self.client.renderer.set_hand(&[], [1.0, 0.0]);
        self.client.renderer.outline = None;
        self.client.renderer
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
        let mut renderer = Renderer::new(ctx.window.clone())?;
        let mut settings = settings::Settings::load(&project_root());
        if players::ensure_identity(&mut settings) {
            settings.save(&project_root());
        }
        // `VOXEL_TEXTURE_PACK=classic|earth32` : un pack imposé pour cette exécution (captures de recette),
        // sans toucher à `settings.ron` : seules les actions du menu des options l'écrivent.
        match std::env::var("VOXEL_TEXTURE_PACK").as_deref() {
            Ok("earth64") => settings.texture_pack = settings::TexturePack::Earth64,
            Ok("earth32") => settings.texture_pack = settings::TexturePack::Earth32,
            Ok("classic") => settings.texture_pack = settings::TexturePack::Classic,
            _ => {}
        }
        if let Some(pack) = settings.shader_pack.as_deref()
            && let Err(error) = shader_packs::apply(&mut renderer, &assets_dir(), Some(pack))
        {
            log::warn!("pack de shaders {pack} : {error:#}. Rendu standard utilisé.");
            settings.shader_pack = None;
        }
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
            let mut ran = false;
            for request in control.poll() {
                game.run_command(request);
                ran = true;
            }
            if ran {
                // Une commande a pu éditer le monde : la fenêtre le voit dans cette image.
                game.flush_world_view();
            }
        }
        if let Some((start, step)) = &mut self.menu_demo {
            // Chaque étape quand le monde courant est chargé et quelques secondes passées.
            let ready = start.elapsed().as_secs_f32() > 4.0
                && self.game.as_ref().is_some_and(|g| g.world_idle());
            if ready {
                let request = match step {
                    0 => Request::Play(Launch {
                        world: Some(persistence::saves_root(&project_root()).join("menu-demo")),
                        seed: Some(7),
                        title: false,
                        difficulty: None,
                        dimension: None,
                    }),
                    1 => Request::Title,
                    _ => Request::Quit,
                };
                log::info!("démo des menus, étape {step} : {request:?}");
                *step += 1;
                *start = Instant::now();
                self.game().client.request = Some(request);
            }
        }
        if let Some(request) = self.game().client.request.take()
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
    /// Recharge ce qui a changé sur le disque. Une erreur est affichée dans les logs,
    /// et l'ancienne version reste en place : le jeu ne plante jamais pour un asset.
    fn reload(&mut self, changes: hot_reload::Changes) {
        if changes.scripts {
            self.server.mods.reload_scripts();
        }
        if changes.shader {
            let result = shader_packs::reload(
                &mut self.renderer,
                &assets_dir(),
                self.settings.shader_pack.as_deref(),
            );
            match result {
                Ok(()) => log::info!("shaders rechargés"),
                Err(err) => log::error!("{err:#}"),
            }
        }
        let mut textures = changes.textures;
        if changes.blocks {
            match hot_reload::reload_registry(
                &self.client.registry,
                &assets_dir(),
                &self.server.mod_dirs,
            ) {
                hot_reload::RegistryReload::Applied(mut fresh) => {
                    texture_packs::configure(
                        &mut fresh,
                        self.settings.texture_pack,
                        &project_root(),
                    );
                    let mut creatures = self
                        .server
                        .ecs
                        .resource::<Arc<MobRegistry>>()
                        .as_ref()
                        .clone();
                    creatures.bind_textures(&mut fresh);
                    self.server.ecs.insert_resource(Arc::new(creatures));
                    self.server.registry = Arc::new(fresh);
                    let registry = self.server.registry.clone();
                    self.client.registry = registry.clone();
                    self.server
                        .streamer_mut()
                        .replace_registry(registry.clone());
                    self.client.view.replica_mut().replace_registry(registry);
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
            && let Some(audio) = &mut self.client.audio
        {
            audio.reload(&self.client.registry);
            self.client.music_until = None;
            self.client.soundscape.reloaded();
        }
        if textures {
            self.client.block_textures = assets::load_textures(&self.client.registry);
            self.client
                .renderer
                .set_block_textures(&self.client.block_textures);
            self.client.ui.clear_icons();
            self.client.view.entities.clear_models();
            self.client.conduit_fx.clear_models();
            self.client
                .hand
                .reload_assets(hand::ArmColors::load(&assets_dir()));
            log::info!(
                "{} textures rechargées",
                self.client.registry.textures().len()
            );
        }
    }

    /// Casse le bloc visé. Renvoie sa position si c'est fait.
    fn break_target(&mut self) -> Option<IVec3> {
        let hit = self.server.aim().hit?;
        self.server
            .break_block(hit.block_pos)
            .then_some(hit.block_pos)
    }

    /// Les bruits de pas (selon le bloc sous les pieds) et l'éclaboussure en entrant dans l'eau.
    fn footsteps(&mut self, dt: f32) {
        if self.client.body.mode != MoveMode::Walk || self.client.body.frozen {
            return;
        }
        // Le corps prédit (voir `predicted.rs`).
        let collider = self.client.body.body;
        let (velocity, transform) = (collider.velocity, collider.position);
        let below_block = self
            .view_world()
            .block((transform - Vec3::Y * 0.05).floor().as_ivec3());
        let Some(audio) = &mut self.client.audio else {
            return;
        };
        if collider.in_liquid && !self.client.was_in_liquid && velocity.y < -1.0 {
            audio.play(audio::SPLASH);
        }
        self.client.was_in_liquid = collider.in_liquid;
        if !collider.on_ground || collider.in_liquid {
            return;
        }
        // Un pas tous les 1,7 bloc parcourus à l'horizontale.
        self.client.step_distance += Vec2::new(velocity.x, velocity.z).length() * dt;
        if self.client.step_distance >= 1.7 {
            self.client.step_distance = 0.0;
            audio.play_block(&self.client.registry, below_block, audio::BlockSound::Step);
        }
    }

    fn show_stats(&mut self, ctx: &mut Context) {
        let stream = self.stream_stats();
        let render = self.client.last_render;
        let pending = stream.queued + stream.in_flight + stream.meshes_waiting;
        let triangles = render.triangles as f64 / 1e6;
        let frozen = if self.client.body.frozen {
            " (chargement…)"
        } else {
            ""
        };
        ctx.status = format!(
            "{} | {}{frozen} | {} entités | {} chunks | {pending} en attente | {}/{} dessinés ({} cachés) | {triangles:.2} M tri",
            self.client.day.clock(),
            self.client.body.mode.name(),
            self.server.ecs.len(),
            stream.loaded,
            render.meshes_drawn,
            render.meshes_total,
            render.meshes_hidden,
        );
        if let Some(far) = &self.client.far {
            let stats = far.stats();
            ctx.status.push_str(&format!(
                " | lointain {} tuiles ({} en cours)",
                far.mesh_count(),
                stats.pending
            ));
        }

        if let Some(report) = self.client.frame_stats.tick() {
            self.client.debug = ui::DebugInfo {
                fps: 1000.0 / report.avg_frame_ms.max(0.001),
                frame_ms: report.avg_frame_ms,
                max_frame_ms: report.max_frame_ms,
            };
            log::info!(
                "image : moy {:.2} ms, max {:.2} ms | streaming : max {:.2} ms, envois GPU : max {:.2} ms ({} au total) | \
                 rendu : max {:.2} ms | modifs : {} (max {:.2} ms) | chunks : {} chargés, {} à générer, {} en cours, {} tâches jetées sur {} | \
                 dessinés {}/{} ({} cachés), {triangles:.2} M triangles | arène : {} pages, {:.1}/{:.1} Mio, {} origines, {} dessins, {} liaisons de page",
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
                render.arena_pages,
                render.arena_used_bytes as f64 / 1_048_576.0,
                render.arena_reserved_bytes as f64 / 1_048_576.0,
                render.arena_origin_slots,
                render.chunk_draws,
                render.page_binds,
            );
            if let Some(pipeline) = self.client.view.replica().pipeline_report() {
                log::info!("délais des chunks : {pipeline}");
            }
            self.log_transfers();
            let fluids = self.server.fluid_report;
            log::info!(
                "fluides : {} cellules calculées, {} actives, {} modifiées, {:.2} ms",
                fluids.processed,
                fluids.active,
                fluids.changed,
                modding::ms(fluids.duration)
            );
            let tick = self.server.mods.last_tick;
            log::info!(
                "machines : tick {} ({} programmés, {} voisins, {:.2} ms), {} en attente | scripts : {} chargé(s){}",
                self.server.mods.machines.scheduler().tick(),
                tick.scheduled,
                tick.neighbor_updates,
                modding::ms(tick.duration),
                self.server.mods.machines.scheduler().pending(),
                self.server.mods.scripts_loaded(),
                self.server
                    .mods
                    .disabled()
                    .iter()
                    .map(|(id, why)| format!(", « {id} » désactivé : {why}"))
                    .collect::<String>(),
            );
        }
    }

    /// Avance tout droit en suivant le relief, pour mesurer le chargement.
    fn autofly(&mut self, speed: f32, dt: f32) {
        let mut eye = self.client.body.eye(1.0) + self.client.camera.forward_flat() * speed * dt;
        let ground = self
            .server
            .streamer()
            .generator()
            .column(eye.x.floor() as i32, eye.z.floor() as i32)
            .height as f32;
        // On rejoint doucement l'altitude visée au lieu de sauter d'un coup.
        eye.y += (ground + FLY_ALTITUDE - eye.y) * (dt * 2.0).min(1.0);
        self.place_eye(eye);
    }

    /// Une action de démo par seconde : casser, puis poser, en alternance.
    fn run_demo(&mut self, dt: f32) {
        let Some(demo) = self.client.demo.as_mut() else {
            return;
        };
        demo.timer += dt;
        if demo.timer < 1.0 || self.client.body.frozen {
            return;
        }
        demo.timer = 0.0;
        demo.breaking = !demo.breaking;
        let breaking = demo.breaking;
        // On essaie aussi les blocs des mods : à chaque pose, le suivant de la barre.
        if !breaking {
            // La case suivante qui contient quelque chose.
            for _ in 0..inventory::HOTBAR {
                self.server.players.me_mut().inventory.selected =
                    (self.server.players.me_mut().inventory.selected + 1) % inventory::HOTBAR;
                if self
                    .server
                    .players
                    .me_mut()
                    .inventory
                    .selected_item()
                    .is_some()
                {
                    break;
                }
            }
        }
        let before = Instant::now();
        let result = if breaking {
            self.break_target()
        } else {
            self.server.place_at_target()
        };
        let elapsed = before.elapsed();
        let body = self.client.body.body;
        let action = if breaking { "cassé" } else { "posé" };
        let block = self
            .server
            .players
            .me_mut()
            .inventory
            .selected_item()
            .map_or("?", |i| self.client.content.items.get(i).key.as_str());
        match result {
            Some(pos) => log::info!(
                "démo : {action} {} en {:.2} ms ({}) | pieds {:.1?} au sol : {} | {} maillages en attente",
                pos,
                elapsed.as_secs_f32() * 1000.0,
                if breaking { "-" } else { block },
                body.position.to_array(),
                body.on_ground,
                self.stream_stats().meshes_waiting,
            ),
            None => log::info!(
                "démo : rien à {} (cible {:?}) | pieds {:.1?}",
                if breaking { "casser" } else { "poser" },
                self.client.target.map(|t| t.block_pos),
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
    attacked: EventReader,
    /// Les morts de créatures, pour leur butin (`died` sert aux sons).
    loot: EventReader,
}

impl EventReaders {
    fn new(ecs: &engine_ecs::World) -> Self {
        Self {
            broken: ecs.resource::<Events<BlockBroken>>().reader(),
            placed: ecs.resource::<Events<BlockPlaced>>().reader(),
            died: ecs.resource::<Events<EntityDied>>().reader(),
            damaged: ecs.resource::<Events<PlayerDamaged>>().reader(),
            attacked: ecs.resource::<Events<events::PlayerAttacked>>().reader(),
            loot: ecs.resource::<Events<EntityDied>>().reader(),
        }
    }
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
        dirs.push(root.join("texture_packs"));
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
        mesh.origin.as_vec3(),
        Aabb::new(min, max),
    )
}

/// Réinterprète les sommets d'`engine_voxel` comme ceux d'`engine_render`, sans copie.
///
/// Les deux structs sont `#[repr(C)]` + `Pod` avec les mêmes champs (24 octets) :
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

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
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

    /// Ramasse les objets au sol tout proches.
    fn pick_up_drops(&mut self, dt: f32) {
        let feet = player::body(&self.ecs, self.players.entity()).position;
        for (entity, stack) in drops::update(&mut self.ecs, feet, dt) {
            let left = self.collect_stack(stack);
            if left < stack.count {
                self.sound("core:place_soft");
                if left == 0 {
                    self.ecs.despawn(entity);
                } else if let Some(mut drop) = self.ecs.get_mut::<drops::DroppedItem>(entity) {
                    drop.stack.count = left;
                }
            }
        }
    }

    fn notify_edit(&mut self, pos: IVec3) {
        self.fluids.wake(pos);
        let actions = self.mods.block_changed(pos);
        self.apply_machine_actions(actions);
    }

    fn after_edit(&mut self, pos: IVec3) {
        self.notify_edit(pos);
        // Une fleur sans sol tombe, le sable s'écroule.
        let settled = engine_voxel::settle(&mut self.streamer_mut(), pos);
        for (p, broken) in settled.broken {
            let harvest = self.registry.def(broken).tier == 0;
            self.drop_loot(p, broken, None, harvest);
            self.notify_edit(p);
            if let Some((other, _)) = engine_voxel::paired_block::partner(&self.registry, p, broken)
            {
                self.notify_edit(other);
            }
        }
        for (from, to, _) in settled.fallen {
            for p in [from, to] {
                self.notify_edit(p);
            }
        }
    }

    fn edit(&mut self, pos: IVec3, block: BlockId) -> bool {
        let old = self.streamer().world().block(pos);
        let partner = engine_voxel::paired_block::partner(&self.registry, pos, old)
            .filter(|&(p, expected)| old != block && self.streamer().world().block(p) == expected);
        let half = (old != block).then(|| self.split_chest(pos)).flatten();
        // Un bloc à inventaire (coffre, four, machine de mod) lâche son contenu quand il change.
        let contents = if old != block {
            self.container_contents(pos)
        } else {
            Vec::new()
        };
        let start = Instant::now();
        let done = self.streamer_mut().set_block(pos, block);
        if done {
            for stack in contents {
                self.drop_stack(pos, stack);
            }
            self.edit_times.push(start.elapsed());
            // On annonce le changement : le son (et plus tard les mods) s'y abonnent.
            if block.is_air() {
                events::send(
                    &self.ecs,
                    BlockBroken {
                        pos,
                        block: old,
                        player: Some(self.players.current_id()),
                    },
                );
            } else {
                events::send(
                    &self.ecs,
                    BlockPlaced {
                        pos,
                        block,
                        player: Some(self.players.current_id()),
                    },
                );
            }
            let joined = (!block.is_air()).then(|| self.join_chest(pos)).flatten();
            self.after_edit(pos);
            for other in partner
                .map(|(p, _)| p)
                .into_iter()
                .chain(half)
                .chain(joined)
            {
                self.after_edit(other);
            }
        }
        done
    }

    fn any_mob_body(&self, predicate: impl Fn(&engine_voxel::PlayerBody) -> bool) -> bool {
        let mut overlap = false;
        self.ecs
            .query::<(&Transform, &Collider, &components::Mob)>()
            .for_each(|_, (transform, collider, _)| {
                let body = engine_voxel::PlayerBody::with_size(
                    transform.position,
                    collider.width,
                    collider.height,
                );
                overlap |= predicate(&body);
            });
        overlap
    }

    /// Une créature occupe-t-elle ce bloc ?
    fn mob_overlaps(&self, pos: IVec3) -> bool {
        self.any_mob_body(|body| body.overlaps_block(pos))
    }

    /// L'objet en main s'use d'un point (Solidité peut l'épargner). `true` s'il se brise.
    fn wear_held(&mut self) -> bool {
        let roll = engine_voxel::Rng64::new(self.next_random()).next_f32();
        self.players
            .me_mut()
            .inventory
            .wear_selected(&self.content.items, roll)
    }

    fn streamer_mut(&self) -> RefMut<'_, ChunkStreamer> {
        self.world.borrow_mut()
    }

    fn place_at_target(&mut self) -> Option<IVec3> {
        let item = self.players.me_mut().inventory.selected_item()?;
        if self.content.items.get(item).key == "core:flask" {
            self.fill_flask(item);
            return None;
        }
        if self.content.items.get(item).key.ends_with("bucket") {
            self.use_bucket(item);
            return None;
        }
        // Une pièce taillée : elle s'ajoute à une case de formes (voir `shaped_play.rs`).
        if let Some(done) = self.place_shaped_piece() {
            return done;
        }
        let aim = self.aim();
        let hit = aim.hit?;
        let def = self.content.items.get(item);
        // Une graine plante son premier stade ; sinon l'objet pose son bloc.
        let block = def
            .block
            .or_else(|| def.plants.as_deref().and_then(|key| self.registry.id(key)))
            .or_else(|| def.places.as_deref().and_then(|key| self.registry.id(key)))?;
        // Un bloc à états (escalier, dalle, tuyau...) se pose selon le regard et la face visée.
        let forward = aim.forward;
        let hit_y = (aim.eye + forward * hit.distance).y - hit.block_pos.y as f32;
        let pos = hit.block_pos + hit.normal;
        if hit.normal == IVec3::ZERO {
            return None;
        }
        if let Some(parts) = self.paired_placement(pos, block, forward) {
            self.place_pair(parts)?;
            self.players.me_mut().inventory.consume_selected();
            self.punch();
            return Some(pos);
        }
        let block = self
            .registry
            .placement_variant(block, forward, hit.normal, hit_y);
        // On peut poser dans l'air ou à la place d'un liquide (l'eau).
        let target = self.streamer().world().block(pos);
        let replaceable = target.is_air() || self.registry.def(target).liquid;
        if !replaceable {
            return None;
        }
        if player::body(&self.ecs, self.players.entity()).overlaps_block(pos)
            || self.mob_overlaps(pos)
        {
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
            self.players.me_mut().inventory.consume_selected();
            self.punch();
            // Des feuilles posées par le joueur ne se décomposent pas : une entité de bloc
            // les marque comme « posées ».
            // Un piston, un inverseur, un répéteur : ils visent où le joueur regarde.
            if let Some(state) = self
                .registry
                .def(block)
                .behavior
                .as_deref()
                .and_then(|b| engine_voxel::placement_state(b, forward))
            {
                self.streamer_mut().set_block_entity(pos, Some(state));
            }
            if self.registry.def(block).behavior.as_deref() == Some("core:leaves") {
                self.streamer_mut()
                    .set_block_entity(pos, Some(engine_voxel::BlockEntity::default()));
            }
        }
        placed.then_some(pos)
    }

    /// L'outil tenu en main.
    fn held_tool(&self) -> Option<engine_voxel::ToolDef> {
        self.players
            .me()
            .inventory
            .selected_item()
            .and_then(|i| self.content.items.get(i).tool)
    }

    /// Ce que les machines et les scripts demandent au jeu.
    fn apply_machine_actions(&mut self, actions: Vec<MachineAction>) {
        for action in actions {
            match action {
                MachineAction::GiveItem { block, count } => {
                    let Some(item) = self.content.items.of_block(block) else {
                        continue;
                    };
                    let left =
                        self.players
                            .me_mut()
                            .inventory
                            .add(item, count, &self.content.items);
                    if left > 0 {
                        log::info!("inventaire plein : {left} objet(s) perdu(s)");
                    }
                }
                MachineAction::GiveStack { item, count } => {
                    let left =
                        self.players
                            .me_mut()
                            .inventory
                            .add(item, count, &self.content.items);
                    if left > 0 {
                        log::info!("inventaire plein : {left} objet(s) perdu(s)");
                    }
                }
                MachineAction::ShowMenu { mod_id, at, data } => {
                    self.show_mod_menu(mod_id, at, &data);
                }
                MachineAction::CloseMenu { mod_id } => self.close_mod_menu_of(&mod_id),
                action @ (MachineAction::PlayerHeal { .. }
                | MachineAction::PlayerTeleport { .. }
                | MachineAction::PlayerEffect { .. }
                | MachineAction::HurtArea { .. }
                | MachineAction::Say { .. }
                | MachineAction::QuestSignal { .. }
                | MachineAction::PlayerHurt { .. }
                | MachineAction::ConsumeHeld { .. }
                | MachineAction::HudBar { .. }
                | MachineAction::HudClear { .. }) => self.apply_player_action(action),
                MachineAction::Travel { dimension } => {
                    self.travel(&dimension);
                }
                MachineAction::ShowOptions { mod_id, data } => {
                    // Une page d'options (réglages du jeu) : la fenêtre la montre si elle est ouverte.
                    let id = self.players.current_id();
                    self.replies
                        .push((id, protocol::ServerMsg::ModOptions { mod_id, data }));
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
                MachineAction::DropStack {
                    pos,
                    item,
                    count,
                    damage,
                } => {
                    if let Some(item) = self.content.items.id(&item) {
                        self.drop_stack(
                            pos,
                            inventory::ItemStack {
                                item,
                                count,
                                damage,
                            },
                        );
                    }
                }
                MachineAction::BlockBroken { pos, block } => {
                    events::send(
                        &self.ecs,
                        BlockBroken {
                            pos,
                            block,
                            player: None,
                        },
                    );
                }
                MachineAction::Log(message) => log::info!("{message}"),
                // Les ordres aux créatures ne valent que pendant leur lot (`mob_brains.rs`).
                MachineAction::MobMove { .. }
                | MachineAction::MobMoveTo { .. }
                | MachineAction::MobJump { .. }
                | MachineAction::MobAttack { .. }
                | MachineAction::MobState { .. } => {}
            }
        }
    }

    /// Casse le bloc en `pos` : il lâche son butin (selon l'outil), l'outil s'use, et les
    /// blocs qui en dépendaient (une fleur dessus, du sable) suivent.
    fn break_block(&mut self, pos: IVec3) -> bool {
        // Une case de formes perd une pièce à la fois (voir `shaped_play.rs`).
        if let Some(done) = self.break_shaped_piece(pos) {
            return done;
        }
        let block = self.streamer().world().block(pos);
        let def = self.registry.def(block).clone();
        if def.hardness < 0.0 {
            return false;
        }
        let tool = self.held_tool();
        // (`edit` fait tomber ce que contenait un coffre, un four ou une machine.)
        if !self.edit(pos, BlockId::AIR) {
            return false;
        }
        let harvest = engine_voxel::can_harvest(&def, tool.as_ref());
        self.drop_loot(pos, block, tool.map(|t| t.kind), harvest);
        // Un minerai cassé avec le bon outil : de l'expérience.
        if harvest {
            let points = self.roll_range(def.xp);
            self.grant_xp(points);
        }
        if tool.is_some() && def.hardness > 0.0 && self.wear_held() {
            log::info!("l'outil s'est cassé");
            self.sound("core:break_glass");
        }
        self.exert(self.hunger_rules.mine);
        true
    }

    /// Mort : on réapparaît au point de départ, en pleine forme.
    /// Le streamer des chunks (partagé avec l'ECS et les mods).
    fn streamer(&self) -> Ref<'_, ChunkStreamer> {
        self.world.borrow()
    }

    /// Un nombre pseudo-aléatoire pour le jeu (butin, rebonds).
    fn next_random(&mut self) -> u64 {
        self.random = self
            .random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.random ^ (self.random >> 29)
    }

    pub(crate) fn drop_stack(&mut self, pos: IVec3, stack: inventory::ItemStack) {
        let seed = self.next_random();
        drops::spawn(&mut self.ecs, stack, pos.as_vec3() + Vec3::splat(0.5), seed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_voxel::mesh_greedy;

    #[test]
    fn vertex_layouts_match() {
        let vertex = MeshVertex::new(
            Vec3::new(1.0, 2.0, 3.0),
            [0.0, 1.0, 0.0],
            [0.5, 0.25, 0.125, 0.6],
            [1.0, 0.5, 0.25],
            7,
        );
        let gpu = gpu_vertices(std::slice::from_ref(&vertex))[0];
        assert_eq!(gpu.position, vertex.position);
        assert_eq!(gpu.normal, vertex.normal);
        assert_eq!(gpu.color, vertex.color);
        assert_eq!(gpu.light, vertex.light);
        assert_eq!(gpu.texture, vertex.texture);
    }

    #[test]
    fn terrain_meshes_convert_without_copy() {
        let world = engine_voxel::TerrainGenerator::new(1).generate_region(1, 1);
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
        assert!(hotbar.contains(&"example:ruby_slab"), "{hotbar:?}");
    }
}
