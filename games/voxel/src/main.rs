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
//! - F2 : une capture d'écran dans `screenshots/`.
//! - F1 : maillage « culling » ou « greedy ». F7 : frustum culling.
//! - F5 : recharger shaders, textures, sons et blocs (c'est aussi automatique quand
//!   un fichier change dans `assets/`, `mods/`, `texture_packs/`, `shader_packs/` ou le dossier
//!   des shaders).
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
mod assets;
mod audio;
mod client;
mod control;
mod enchanting;
mod key_codes;
mod modding;
mod quests;
mod render;
mod shaping;
mod storage;
mod ui;
mod vehicles;
mod world;
pub use launch::{Launch, Request};

use crate::key_codes::KeyCodes;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
#[cfg(test)]
pub(crate) use voxel_sim::test_registry;
#[allow(unused_imports)]
pub(crate) use voxel_sim::{
    DEFAULT_SEED, EventReaders, FLY_ALTITUDE, assets_dir, env_parse, load_registry, project_root,
};

use voxel_sim::{
    components, launch, mobs, net::bot_swarm, net::dedicated, net::predicted, net::wire, player,
    player::inventory, player::players, server, server::hot_reload, server::stats, settings,
    shared, world::soundscape, world::weather,
};

use anyhow::Result;
use engine_core::winit::dpi::PhysicalSize;
use engine_core::winit::event::MouseButton;
use engine_core::winit::event::WindowEvent;
use engine_core::winit::keyboard::KeyCode;
use engine_core::{AppConfig, Context, Game};
use engine_ecs::{Entity, Stage};

use engine_render::{Aabb, Camera, ColorVertex, Fog, MeshId, RenderStats, Renderer, Sun};
use engine_voxel::{CHUNK_SIZE, ChunkMesh, MeshVertex, raycast};
use glam::{IVec3, Vec2, Vec3};

use crate::client::ClientSide;
use components::{Collider, Transform};
use mobs::MobRegistry;
use player::{FLY_SPEED, FLY_SPRINT_FACTOR, MoveMode, SPRINT_SPEED, WALK_SPEED};
use server::VoxelServer;
use settings::Action;
use shared::StreamerExt;
use stats::FrameStats;

const MOUSE_SENSITIVITY: f32 = 0.0025;
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
    Title(crate::ui::menus::TitlePage),
    Playing,
    Paused(crate::ui::menus::PausePage),
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
        joined: Option<crate::client::join::Joined>,
    ) -> Result<Self> {
        let mut launch = launch;
        let mut joined = joined;
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
            .map(|dir| crate::control::screenshots::Tour::new(dir.into()));

        // `VOXEL_SERVER_THREAD=1` : le monde tourne dans son propre thread (un serveur intégré,
        // voir `voxel_sim::net::dedicated::spawn_integrated`), et la fenêtre le rejoint comme un
        // invité, sur la boucle locale. Les outils de l'hôte (`ctl`, captures, démo) n'ont alors
        // que la fenêtre.
        let mut integrated = None;
        let threaded = std::env::var("VOXEL_SERVER_THREAD").is_ok_and(|v| v == "1");
        if threaded
            && !launch.title
            && launch.join.is_none()
            && joined.is_none()
            && let Some(world) = launch.world.clone()
        {
            let (name, uuid) = crate::client::join::identity(&settings);
            let mut started =
                voxel_sim::net::dedicated::spawn_integrated(world, &settings, &name, uuid)?;
            // Déjà accueilli par un lien en mémoire : la fenêtre suit le chemin d'un invité.
            joined = started.joined.take();
            launch.join = Some("serveur intégré".into());
            launch.world = None;
            integrated = Some(started);
        }

        // Le serveur d'abord, sans fenêtre (voir `server/boot.rs`), puis ce que la fenêtre en tire.
        let boot = VoxelServer::boot(
            &launch,
            &settings,
            ctx.fixed_dt,
            autofly.is_some(),
            demo.is_some(),
        )?;
        let (mut server, mut yaw, mut pitch, mut seed) =
            (boot.server, boot.yaw, boot.pitch, boot.seed);
        // Un invité : la poignée de main avec l'hôte, puis la coquille refaite sur sa graine (le
        // terrain lointain en dépend) et le joueur placé où l'hôte l'accueille (voir
        // `client/join.rs`).
        let mut remote = None;
        // La portée de l'hôte : il n'envoie pas de chunks au-delà (voir plus bas, le terrain
        // lointain).
        let mut host_radius = None;
        if let Some(address) = &launch.join {
            // Déjà accueilli depuis le menu, ou (`VOXEL_JOIN`) la poignée de main maintenant.
            let joined = match joined {
                Some(joined) => joined,
                None => {
                    let (name, uuid) = crate::client::join::identity(&settings);
                    crate::client::join::join(address, &server, &name, uuid)?
                }
            };
            if joined.seed != seed {
                let shell = Launch {
                    seed: Some(joined.seed),
                    world: None,
                    join: None,
                    ..launch.clone()
                };
                server = VoxelServer::boot(&shell, &settings, ctx.fixed_dt, false, false)?.server;
                seed = joined.seed;
            }
            let entity = server.players.entity();
            player::set_eye(
                &server.ecs,
                entity,
                joined.feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
            );
            if joined.fly != (player::mode(&server.ecs, entity) == MoveMode::Fly) {
                player::toggle_mode(&mut server.ecs, entity);
            }
            (yaw, pitch) = (joined.yaw, joined.pitch);
            host_radius = Some(joined.view_radius);
            wire::set_registry(server.registry.clone());
            log::info!(
                "partie de {address} rejointe (joueur {:?})",
                joined.link.player
            );
            remote = Some(joined.link);
        }
        let registry = server.registry.clone();
        let assets = assets_dir();
        let block_textures = assets::load_textures(&registry);
        renderer.set_block_textures(&block_textures);
        let config = server.world.borrow().config().clone();
        // Les chunks détaillés vont jusqu'à notre portée, ou celle de l'hôte pour un invité : c'est
        // lui qui les envoie. Le terrain lointain commence là, sinon un anneau resterait vide entre
        // les deux.
        let near_radius = host_radius.unwrap_or(config.view_radius);
        // Le terrain lointain (simplifié) prolonge le monde au-delà des chunks détaillés ;
        // `VOXEL_FAR=0` le coupe, pour comparer. Pas dans le décor du menu principal.
        let far_chunks = if launch.title {
            0
        } else {
            env_parse("VOXEL_FAR")
                .unwrap_or(settings.far_distance)
                .clamp(0, 192)
        };
        let far_chunks = if far_chunks > near_radius {
            far_chunks
        } else {
            0
        };
        // Le brouillard termine juste avant le bord de ce qu'on dessine.
        let drawn_chunks = if far_chunks > 0 {
            far_chunks
        } else {
            near_radius
        };
        let view_distance = (drawn_chunks * CHUNK_SIZE as i32) as f32;
        renderer.fog = Some(Fog {
            start: view_distance * 0.6,
            end: view_distance * 0.95,
        });
        let far = (far_chunks > 0).then(|| {
            let far = crate::render::far_terrain::FarTerrain::new(
                &server.world.borrow(),
                near_radius,
                far_chunks,
            );
            log::info!("terrain lointain : de {near_radius} à {far_chunks} chunks");
            far
        });
        let mut view = crate::client::world::ClientWorld::new(&server.world.borrow());
        // `VOXEL_PIPELINE=1` : les délais d'un chunk reçu → maillage → envoi (la réplique).
        view.replica_mut()
            .set_pipeline_timing(std::env::var("VOXEL_PIPELINE").is_ok_and(|v| v == "1"));
        // La fenêtre simule son joueur à partir de là où le serveur l'a placé (voir
        // `net/predicted.rs`).
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
        let mut hand =
            crate::render::hand::Hand::new(crate::render::hand::ArmColors::load(&assets));
        hand.visible &= settings.show_arm && !launch.title;
        Ok(Self {
            mode: if launch.title {
                Mode::Title(crate::ui::menus::TitlePage::Main)
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
                grid_meshes: crate::vehicles::ships::GridMeshes::default(),
                soundscape: soundscape::Soundscape::new(seed),
                enchant_ui: crate::enchanting::ui::EnchantUi::default(),
                music_until: None,
                base_fog: (view_distance * 0.6, view_distance * 0.95),
                audio,
                chest_lids: crate::render::chest_lids::ChestLids::default(),
                step_distance: 0.0,
                was_in_liquid: false,
                quit_at: env_parse::<u64>("VOXEL_QUIT_AFTER")
                    .map(|s| Instant::now() + std::time::Duration::from_secs(s)),
                frame_stats: FrameStats::new(),
                uploads: crate::client::stream_upload::Uploads::new(),
                last_render: RenderStats::default(),
                culling: crate::client::culling::CaveCulling::from_env(),
                bench: crate::control::bench::Bench::from_env(),
                outbox: Vec::new(),
                frame_dt: 0.0,
                eating_sent: false,
                body,
                remote,
                integrated,
                lan: None,
                online: Vec::new(),
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
            // En pause, le monde s'arrête ; ouvert au réseau local, il continue pour les autres
            // joueurs (le joueur de la fenêtre ne bouge plus).
            Mode::Paused(_) if self.server.net.is_some() => {
                self.server.fixed_step(ctx.fixed_dt);
                self.apply_replies();
                return;
            }
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
                if self.hosting() {
                    self.server
                        .schedule
                        .run(Stage::FixedUpdate, &mut self.server.ecs);
                    self.server.step_fluids();
                }
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
                // Affamé, on ne court plus (les jauges viennent du serveur, voir
                // `net/player_view.rs`).
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
        // La fenêtre joue le mouvement de son joueur et l'envoie (voir `net/predicted.rs`).
        self.predict_move(walk, jump, ctx.fixed_dt);
        if self.hosting() {
            self.server.fixed_step(ctx.fixed_dt);
        }
        self.footsteps(ctx.fixed_dt);
        self.apply_replies();
    }

    fn window_event(&mut self, ctx: &mut Context, event: &WindowEvent) -> bool {
        // La fenêtre perd le focus en pleine partie : le jeu se met en pause et rend la souris,
        // comme Minecraft (on la reprend d'un clic sur « Reprendre », jamais par surprise).
        // (Pas dans les modes automatiques : captures, banc d'essai, démo, qui tournent sans
        // focus.)
        let automatic =
            self.client.tour.is_some() || self.client.bench.is_some() || self.client.demo.is_some();
        if *event == WindowEvent::Focused(false) && self.mode == Mode::Playing && !automatic {
            self.mode = Mode::Paused(crate::ui::menus::PausePage::Main);
            self.client.mouse_captured = false;
            ctx.set_cursor_captured(false);
        }
        self.client.ui.on_window_event(&ctx.window, event)
    }

    fn update(&mut self, ctx: &mut Context, dt: f32) {
        self.client.frame_dt = dt;
        // Les joueurs du réseau local : arrivées, messages, départs (voir `net/host.rs`).
        if self.hosting() {
            self.server.poll_net();
        }
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
                self.mode = Mode::Paused(crate::ui::menus::PausePage::Main);
                escape = false;
            }
            Mode::Paused(crate::ui::menus::PausePage::Main) if escape => {
                self.mode = Mode::Playing;
                escape = false;
            }
            Mode::Paused(_) if escape => {
                self.leave_mod_options();
                self.mode = Mode::Paused(crate::ui::menus::PausePage::Main);
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
        // (voir `client/input.rs` et `server/intents.rs`).
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
        if self.hosting() {
            self.server.autosave();
        }
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
        // l'image et les événements (voir `server/step.rs`).
        // En pause, le monde s'arrête : l'heure, les potions et les villages aussi (sauf ouvert au
        // réseau local).
        let paused = matches!(self.mode, Mode::Paused(_)) && self.server.net.is_none();
        if self.hosting() && !paused {
            self.server.frame(dt);
        }

        // Tout le travail du monde côté thread principal : on le chronomètre.
        let start = Instant::now();
        let eye = self.client.body.eye(1.0);
        if self.hosting() {
            self.server.stream_world();
        }
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
        let wind = self.client.weather.wind();
        self.client.renderer.wind = engine_render::Wind {
            direction: wind.direction,
            strength: wind.strength,
        };
        // Les entités viennent de la réplique du client : créatures, autres joueurs, projectiles,
        // wagons et objets au sol (voir `render/entities.rs`).
        self.client.view.entities.begin_frame(alpha);
        let mobs = self.server.ecs.resource::<Arc<MobRegistry>>().clone();
        let mut mob_boxes = Vec::new();
        crate::render::entities::draw(
            &self.client.view.entities,
            &crate::render::entities::DrawEnv {
                mobs: &mobs,
                items: &self.client.content.items,
                blocks: &self.client.registry,
                world: self.client.view.world(),
                alpha,
                time: self.client.renderer.time,
            },
            &mut mob_boxes,
        );
        crate::render::weather::instances(&self.client.weather, &mut mob_boxes);
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
            .then(|| crate::render::weather::sky(&self.client.weather));
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
        // Le serveur intégré sauvegarde à son arrêt (le joueur de la fenêtre compris).
        if let Some(integrated) = &mut self.client.integrated {
            integrated.stop();
        }
    }

    /// Sauvegarde et décharge le monde ; rend le renderer pour le monde suivant.
    fn shutdown(mut self) -> Renderer {
        self.server.save_now();
        if let Some(integrated) = &mut self.client.integrated {
            integrated.stop();
        }
        self.client.renderer.clear_meshes();
        self.client.renderer.set_visible_meshes(None);
        self.client.renderer.set_instances(&[]);
        self.client.renderer.set_hand(&[], [1.0, 0.0]);
        self.client.renderer.outline = None;
        self.client.renderer
    }
}

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
        let launch = match request {
            Request::Play(launch) => launch,
            _ => Launch::title(),
        };
        // Rejoindre : la poignée de main avant de quitter ce monde (même contenu, même empreinte) ;
        // un refus laisse au menu, avec la raison.
        let mut joined = None;
        if let Some(address) = &launch.join {
            let (name, uuid) = crate::client::join::identity(&settings);
            match crate::client::join::join(address, &game.server, &name, uuid) {
                Ok(welcome) => joined = Some(welcome),
                Err(err) => {
                    log::warn!("{err:#}");
                    let mut game = game;
                    game.mode = Mode::Title(crate::ui::menus::TitlePage::Join {
                        address: address.clone(),
                        error: Some(format!("{err:#}")),
                    });
                    self.game = Some(game);
                    return Ok(());
                }
            }
        }
        let renderer = game.shutdown();
        log::info!("chargement : {launch:?}");
        self.game = Some(VoxelGame::create(ctx, renderer, launch, settings, joined)?);
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
        // `VOXEL_TEXTURE_PACK` impose un pack pour cette exécution (captures de recette),
        // sans toucher à `settings.ron` : seules les actions du menu des options l'écrivent.
        match std::env::var("VOXEL_TEXTURE_PACK").as_deref() {
            Ok("earth64") => settings.texture_pack = settings::TexturePack::Earth64,
            Ok("earth32") => settings.texture_pack = settings::TexturePack::Earth32,
            Ok("classic") => settings.texture_pack = settings::TexturePack::Classic,
            _ => {}
        }
        if let Some(pack) = settings.shader_pack.as_deref()
            && let Err(error) = crate::render::shader_packs::apply(
                &mut renderer,
                &voxel_sim::paths::get().shader_pack_dirs(),
                Some(pack),
            )
        {
            log::warn!("pack de shaders {pack} : {error:#}. Rendu standard utilisé.");
            settings.shader_pack = None;
        }
        let launch = Launch::from_env().unwrap_or_else(Launch::title);
        Ok(Self {
            game: Some(VoxelGame::create(ctx, renderer, launch, settings, None)?),
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
                        world: Some(voxel_sim::paths::get().saves.join("menu-demo")),
                        seed: Some(7),
                        title: false,
                        difficulty: None,
                        dimension: None,
                        join: None,
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

impl VoxelGame {
    /// Recharge ce qui a changé sur le disque. Une erreur est affichée dans les logs,
    /// et l'ancienne version reste en place : le jeu ne plante jamais pour un asset.
    fn reload(&mut self, changes: hot_reload::Changes) {
        if changes.scripts {
            self.server.mods.reload_scripts();
        }
        if changes.shader {
            let result = crate::render::shader_packs::reload(
                &mut self.client.renderer,
                &voxel_sim::paths::get().shader_pack_dirs(),
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
                    crate::render::texture_packs::configure(
                        &mut fresh,
                        self.settings.texture_pack,
                        &voxel_sim::paths::get().texture_pack_dirs(),
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
                .reload_assets(crate::render::hand::ArmColors::load(&assets_dir()));
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
        // Le corps prédit (voir `net/predicted.rs`).
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
            audio.play_block(
                &self.client.registry,
                below_block,
                voxel_sim::world::sound_ids::BlockSound::Step,
            );
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
                voxel_sim::modding::ms(fluids.duration)
            );
            let tick = self.server.mods.last_tick;
            log::info!(
                "machines : tick {} ({} programmés, {} voisins, {:.2} ms), {} en attente | scripts : {} chargé(s){}",
                self.server.mods.machines.scheduler().tick(),
                tick.scheduled,
                tick.neighbor_updates,
                voxel_sim::modding::ms(tick.duration),
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

/// Démarre la surveillance des assets, des mods et des shaders (si les dossiers existent).
fn start_hot_reload() -> Option<hot_reload::HotReload> {
    let paths = voxel_sim::paths::get();
    let mut dirs = vec![paths.game.clone()];
    dirs.extend(paths.mods_dirs());
    dirs.extend(paths.texture_pack_dirs());
    dirs.extend(paths.shader_pack_dirs());
    // Les shaders du moteur : seulement en développement (une installation les embarque, et le
    // dépôt de la machine qui l'a compilée n'a pas à compter).
    if paths.from_repository()
        && let Some(shaders) = std::path::Path::new(engine_render::CHUNK_SHADER_PATH).parent()
    {
        dirs.push(shaders.to_path_buf());
    }
    hot_reload::HotReload::new(&dirs)
        .map_err(|err| log::warn!("rechargement à chaud indisponible : {err:#}"))
        .ok()
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

fn main() -> Result<()> {
    // Les dossiers du jeu (voir `voxel_sim::paths`) : `--data-dir` d'abord, avant tout chemin.
    let args: Vec<String> = std::env::args().collect();
    voxel_sim::paths::init(voxel_sim::paths::data_dir_arg(&args));
    // Le journal, dans la console et `logs/latest.log` ; plus détaillé avec `RUST_LOG=info`.
    voxel_sim::logs::init(voxel_sim::logs::Program::Game);

    // `voxel --server` : le serveur dédié, sans fenêtre (voir `net/dedicated.rs`).
    if std::env::args().any(|a| a == "--server") {
        return dedicated::run();
    }
    // `voxel --bots N adresse [idle|walk|explore]` : des bots sans fenêtre (voir
    // `net/bot_swarm.rs`).
    if args.iter().any(|a| a == "--bots") {
        return bot_swarm::run(&args);
    }

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
        let order = voxel_sim::modding::resolve_order(&assets_dir());
        let registry = load_registry(&voxel_sim::modding::mod_dirs(&order)).unwrap();
        let hotbar: Vec<&str> = registry
            .placeable()
            .map(|id| registry.def(id).key.as_str())
            .collect();
        assert_eq!(hotbar.first(), Some(&"core:grass"));
        assert!(hotbar.contains(&"example:ruby_slab"), "{hotbar:?}");
    }
}
