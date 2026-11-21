//! Construire le serveur seul, sans fenêtre (lot 4b du multijoueur) : registres, contenu, monde,
//! dimensions, générateur, chunks, fluides, mods, structures mobiles, ECS et systèmes, joueur local.
//! `VoxelGame::create` s'en sert puis construit la fenêtre ; les tests et, plus tard, le serveur
//! dédié l'utilisent tels quels.

use crate::DEFAULT_SEED;
use crate::EventReaders;
use crate::FLY_ALTITUDE;
use crate::assets_dir;
use crate::daytime::DayCycle;
use crate::env_parse;
use crate::inventory::Inventory;
use crate::launch::Launch;
use crate::load_registry;
use crate::mobs::MobRegistry;
use crate::player::MoveMode;
use crate::project_root;
use crate::server_side::VoxelServer;
use crate::{
    adventure, adventure_data, attributes, backpacks, breeding, carts, components, content,
    dimensions, elites, enchant_library, events, gear, intents, inventory, item_data, mob_brains,
    mobs, modding, navigation, persistence, player, players, potions, projectiles, settings,
    shape_materials, shared, ships, spawning, stress, survival, texture_packs, trading, villagers,
    weather,
};
use anyhow::{Context as _, Result};
use engine_ecs::Schedule;
use engine_ecs::Stage;
use engine_voxel::BlockId;
use engine_voxel::ChunkStreamer;
use engine_voxel::StreamingConfig;
use glam::IVec3;
use glam::Vec3;
use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Le serveur construit, et ce que la fenêtre en reprend pour démarrer.
pub(crate) struct Boot {
    pub server: VoxelServer,
    /// Le regard de départ du joueur (sauvegardé, ou par défaut).
    pub yaw: f32,
    pub pitch: f32,
    pub seed: u32,
}

impl VoxelServer {
    /// Ouvre le monde de `launch` (ou un monde sans sauvegarde). `fly` : le joueur commence en vol
    /// (survol du menu, vol automatique) ; `demo` : le regard de départ de la démo.
    pub(crate) fn boot(
        launch: &Launch,
        settings: &settings::Settings,
        fixed_dt: f32,
        fly: bool,
        demo: bool,
    ) -> Result<Boot> {
        // L'ordre des mods (chacun après ses dépendances) fixe l'ordre des registres.
        let assets = assets_dir();
        let mod_order = modding::resolve_order(&assets);
        let mod_dirs = modding::mod_dirs(&mod_order);
        let mut registry = load_registry(&mod_dirs)?;
        texture_packs::configure(&mut registry, settings.texture_pack, &project_root());
        let mut mob_registry =
            MobRegistry::load(&assets, &mod_dirs, &registry).context("chargement des créatures")?;
        mob_registry.bind_textures(&mut registry);
        let registry = Arc::new(registry);
        let content = content::Content::load(&registry, &assets, &mod_dirs)
            .context("chargement des objets")?;
        let trades =
            trading::TradeBook::builtin(&content.items).context("chargement des échanges")?;
        let potions =
            potions::Potions::builtin(&content.items).context("chargement des potions")?;
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
        let (mut dims, store) = dimensions::open(
            &registry,
            &mod_dirs,
            store,
            meta.as_ref(),
            launch.dimension.as_deref(),
        )?;
        // Le joueur local vient de `players/<identifiant>.ron`, ou de `world.ron` pour un monde plus
        // ancien (voir `persistence::with_player_file`).
        let local_uuid = players::parse_uuid(&settings.player_uuid).unwrap_or_default();
        let meta = meta.map(|m| {
            persistence::with_player_file(
                m,
                dims.meta_store.as_deref().or(store.as_deref()),
                local_uuid,
            )
        });
        if let Some(meta) = &meta {
            persistence::check_mods(meta, &registry);
        }

        // Un monde existant garde sa graine.
        let seed = meta
            .as_ref()
            .map(|m| m.seed)
            .or(launch.seed)
            .unwrap_or(DEFAULT_SEED);
        dims.base_seed = seed;
        // Un monde existant garde sa difficulté.
        let difficulty = meta
            .as_ref()
            .map(|m| m.difficulty)
            .or(launch.difficulty)
            .unwrap_or(1)
            .min(2);
        let config = StreamingConfig {
            view_radius: env_parse("VOXEL_RADIUS").unwrap_or(settings.render_distance.clamp(4, 32)),
            // `VOXEL_LEAF_CULLING=0` : garder toutes les faces de feuilles, pour comparer.
            leaf_culling: std::env::var("VOXEL_LEAF_CULLING").map_or(true, |v| v != "0"),
            ..StreamingConfig::default()
        };
        log::info!(
            "graine {seed}, rayon de vue {} chunks, {} threads de fond",
            config.view_radius,
            config.worker_threads
        );
        let generator = modding::terrain(
            seed,
            &registry,
            &mod_dirs,
            &dims.current,
            dims.generation.generation_version()?,
        )?;
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
        let mode = if fly { MoveMode::Fly } else { MoveMode::Walk };
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
            if demo { -35f32 } else { -20f32 }.to_radians(),
            mode,
        );
        // Un monde sauvegardé reprend là où on l'a laissé.
        let (eye, yaw, pitch, mode) = persistence::saved_eye(meta.as_ref()).unwrap_or(default_view);
        // Arrivé par un portail : au sol, aux mêmes x et z (voir `dimensions.rs`).
        let eye = dims.arrive(eye, meta.as_ref(), streamer.generator());
        // Le monde ECS : le streamer et les registres deviennent des ressources, que les
        // systèmes (physique, IA) lisent sans que le jeu ait à les leur passer.
        // Le serveur ne maille pas ; la fenêtre en a la réplique (après les fluides, qu'elle copie).
        let streamer = streamer.as_server();
        let world = Rc::new(RefCell::new(streamer));
        let mut mods = modding::ModSystem::new(
            &mod_order,
            world.clone(),
            registry.clone(),
            engine_mods::ModContent {
                items: content.items.clone(),
                machine_recipes: content.machine_recipes.clone(),
                loot: content.loot.clone(),
                mob_keys: Arc::new(mob_registry.keys()),
                dimension: dims.current.key.as_str().into(),
            },
            &project_root(),
            launch.world.as_deref(),
        );
        // Le coffre et le four : des machines natives qui ont besoin des objets et de la fonte.
        engine_voxel::register_containers(
            &mut mods.machines,
            &registry,
            content.items.clone(),
            content.smelting.clone(),
            content.brewing.clone(),
            &content.recipes,
        );
        enchant_library::register(&mut mods.machines, &registry);
        // Les structures mobiles du monde : relues, et leurs machines redémarrent.
        if let Some(store) = &store {
            match engine_voxel::load_grids(store.dir(), &registry) {
                Ok(loaded) if !loaded.is_empty() => {
                    log::info!("{} structure(s) mobile(s) chargée(s)", loaded.len());
                    *mods.subgrids.borrow_mut() = loaded;
                    let shared = mods.subgrids.clone();
                    let subs = shared.borrow();
                    let mut actions = Vec::new();
                    for id in subs.ids() {
                        // L'emprunt de la grille doit finir avant de démarrer ses machines.
                        let active: Vec<engine_voxel::GridPos> = {
                            let grid = subs.get(id).expect("identifiant connu").borrow();
                            grid.cells()
                                .filter(|c| mods.machines.has_behavior(grid.world().block(*c)))
                                .map(|c| engine_voxel::GridPos::new(id, c))
                                .collect()
                        };
                        let grids = engine_voxel::Grids::with_subgrids(&world, &subs);
                        mods.machines
                            .grid_blocks_loaded(grids, &active, &mut actions);
                    }
                }
                Ok(_) => {}
                Err(err) => log::error!("structures mobiles non chargées : {err:#}"),
            }
        }
        let mut ecs = engine_ecs::World::new();
        events::register(&mut ecs);
        ecs.insert_resource(shared::SharedStreamer(world.clone()));
        ecs.insert_resource(shared::SharedSubGrids(mods.subgrids.clone()));
        ecs.insert_resource(ships::ShipControl::default());
        ecs.insert_resource(ships::ShipEvents::default());
        ecs.insert_resource(Arc::new(mob_registry));
        ecs.insert_resource(components::GravityScale(dims.current.gravity));
        // Les blocs où les créatures ne vont jamais : les fluides qui blessent (la lave).
        ecs.insert_resource(Arc::new(navigation::DangerBlocks(
            (0..registry.len())
                .map(|i| {
                    fluids
                        .kind_of(BlockId(i as u16))
                        .map_or(0.0, |kind| kind.damage)
                })
                .collect(),
        )));
        // Les règles de survie, aussi pour les créatures (chute, lave, feu).
        ecs.insert_resource(Arc::new(survival::load_rules(&assets).0));
        ecs.insert_resource(mobs::FixedDt(fixed_dt));
        ecs.insert_resource(mobs::GameRng(0x2545_F491_4F6C_DD1D ^ seed as u64));
        ecs.insert_resource(mobs::SimClock::default());
        carts::setup(&mut ecs, settings, &content, store.as_deref());
        ecs.insert_resource(spawning::Difficulty(difficulty));
        let adventure = adventure::Adventure::load(
            meta.as_ref().map(|m| &m.adventure),
            &adventure_data::current(),
        );
        ecs.insert_resource(adventure.effects(&adventure_data::current()));
        ecs.insert_resource(spawning::Sky::default());
        let player = player::spawn(&mut ecs, eye, mode);
        // Le joueur de la fenêtre se simule chez elle (voir `predicted.rs`).
        ecs.insert(player, components::Predicted);
        let mut schedule = Schedule::new();
        schedule
            .add(Stage::FixedUpdate, "tick", mobs::tick)
            .add(
                Stage::FixedUpdate,
                "créatures : apparition",
                spawning::spawn,
            )
            .add(Stage::FixedUpdate, "créatures : soleil", spawning::dissolve)
            .add(Stage::FixedUpdate, "créatures : élevage", breeding::update)
            .add(Stage::FixedUpdate, "créatures : IA", mobs::wander)
            .add(
                Stage::FixedUpdate,
                "créatures : trajets des scripts",
                mob_brains::follow_goals,
            )
            .add(Stage::FixedUpdate, "structures mobiles", ships::update)
            .add(Stage::FixedUpdate, "physique", mobs::physics)
            .add(Stage::FixedUpdate, "wagons", carts::update)
            .add(Stage::FixedUpdate, "créatures : animation", mobs::animate)
            .add(
                Stage::FixedUpdate,
                "créatures : environnement",
                mobs::environment,
            )
            .add(Stage::FixedUpdate, "créatures : attaque", mobs::attack)
            .add(Stage::FixedUpdate, "créatures : tirs", projectiles::shoot)
            .add(Stage::FixedUpdate, "projectiles", projectiles::fly)
            .add(Stage::FixedUpdate, "élites : régénération", elites::regen)
            .add(Stage::FixedUpdate, "créatures : mort", mobs::cleanup);
        let readers = EventReaders::new(&ecs);
        let mod_readers = EventReaders::new(&ecs);

        // L'inventaire sauvegardé, ou un petit nécessaire de départ pour un nouveau monde.
        let items = &content.items;
        let inventory = match meta.as_ref() {
            Some(meta) => match &meta.items {
                Some(saved) => Inventory::from_save(saved, items),
                // Un monde d'avant les objets : son inventaire de blocs.
                None if !meta.inventory.is_empty() => {
                    Inventory::from_old_save(&meta.inventory, items)
                }
                None => inventory::starter_inventory(items),
            },
            None => inventory::starter_inventory(items),
        };
        let time = meta
            .as_ref()
            .map(|m| m.time_of_day)
            .or_else(|| env_parse("VOXEL_TIME"))
            .unwrap_or(0.3);
        let (survival, hunger_rules) = survival::load_rules(&assets);
        // Les fiches d'objets du monde ouvert (voir `item_data.rs`).
        // Ces deux tables sont globales au processus : un test qui construit un serveur (avec ou sans
        // sauvegarde) ne les change pas sous les pieds des autres tests qui tournent en même temps.
        if !cfg!(test) {
            item_data::replace_all(
                dims.meta_store
                    .as_ref()
                    .map_or_else(Default::default, |s| item_data::ItemStore::load(s.dir())),
            );
            // Les matériaux des pièces taillées du monde ouvert (voir `shape_materials.rs`).
            shape_materials::replace_all(
                dims.meta_store.as_ref().map_or_else(Default::default, |s| {
                    shape_materials::Palette::load(s.dir())
                }),
            );
        }
        let mut local = players::PlayerState::new(
            players::PlayerIdentity {
                id: players::PlayerId(0),
                name: settings.player_name.clone(),
                uuid: local_uuid,
            },
            player,
            inventory,
            gear::Gear::new(attributes::load_base(&assets)),
            &survival,
            &hunger_rules,
        );
        local.adventure = adventure;
        if let Some(saved) = meta.as_ref().and_then(|m| m.player.as_ref()) {
            local.apply_save(saved);
        }
        let players = players::Players::new(local);
        players.sync_ecs(&mut ecs);
        let server = VoxelServer {
            sends: Default::default(),
            charm_clock: 0.0,
            charm_beats: 0,
            allow_mod_menu: false,
            edit_times: Vec::new(),
            travel: None,
            pending_commands: Vec::new(),
            registry,
            content,
            survival,
            hunger_rules,
            ecs,
            schedule,
            random: 0x853C_49E6_748F_EA9B ^ seed as u64,
            fluids,
            fluid_steps: 0,
            fluid_report: engine_voxel::FluidReport::default(),
            world,
            mods,
            mod_dirs,
            mod_readers,
            stress: stress::StressTest::from_env(),
            spawn,
            players,
            readers,
            day: DayCycle::new(time, env_parse("VOXEL_DAY_LENGTH").unwrap_or(600.0)),
            weather: weather::Weather::new(seed),
            helm: None,
            steer: 0.0,
            net: None,
            operators: Vec::new(),
            mod_settings_saved: std::time::Instant::now(),
            difficulty,
            potions,
            enchant_seed: seed ^ 0xE5C4,
            villages: villagers::Villages::default(),
            trades,
            store,
            backpacks: dims.meta_store.as_ref().map_or_else(Default::default, |s| {
                backpacks::BackpackStore::load(s.dir())
            }),
            dims,
            last_autosave: Instant::now(),
            save_started: Instant::now(),
            acting: None,
            replies: Vec::new(),
            outcome: intents::Outcome::Nothing,
            command_reply: None,
            check_moves: std::env::var("VOXEL_CHECK_MOVES").is_ok_and(|v| v == "1"),
            sim_step: 0,
        };
        Ok(Boot {
            server,
            yaw,
            pitch,
            seed,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::players::PlayerId;
    use crate::protocol::{ClientMsg, ServerMsg};

    /// Un serveur sans fenêtre ni sauvegarde, sur une graine fixe.
    pub(crate) fn headless() -> VoxelServer {
        headless_in(None)
    }

    /// Le même, qui sauvegarde dans `world` s'il est donné.
    pub(crate) fn headless_in(world: Option<std::path::PathBuf>) -> VoxelServer {
        let launch = Launch {
            world,
            seed: Some(7),
            title: false,
            difficulty: None,
            dimension: None,
            join: None,
        };
        let settings = settings::Settings::default();
        VoxelServer::boot(&launch, &settings, 1.0 / 60.0, false, false)
            .expect("le serveur se construit sans fenêtre")
            .server
    }

    #[test]
    fn a_server_without_a_window_serves_two_players() {
        let mut server = headless();
        assert_eq!(server.players.iter().count(), 1, "le joueur local");
        let host = server.players.local_id();
        let feet = player::body(&server.ecs, server.players.entity()).position;
        let guest = server.add_ghost("Invité", feet + Vec3::X * 2.0);
        assert_ne!(guest, host);

        // Chaque joueur a sa case choisie : le message de l'invité ne touche que lui.
        let before = server.players.local().inventory.selected;
        server.handle(guest, ClientMsg::SelectSlot(4));
        assert_eq!(server.players.get(guest).unwrap().inventory.selected, 4);
        assert_eq!(server.players.local().inventory.selected, before);
        server.handle(host, ClientMsg::SelectSlot(7));
        assert_eq!(server.players.local().inventory.selected, 7);
        assert_eq!(server.players.get(guest).unwrap().inventory.selected, 4);

        // Le chat d'un joueur part aux deux, avec son nom.
        server.replies.clear();
        server.handle(guest, ClientMsg::Chat("bonjour".into()));
        let heard: Vec<PlayerId> = server
            .replies
            .iter()
            .filter(|(_, m)| matches!(m, ServerMsg::Chat(line) if line == "<Invité> bonjour"))
            .map(|(p, _)| *p)
            .collect();
        assert!(heard.contains(&host) && heard.contains(&guest), "{heard:?}");

        // Le regard d'un joueur est le sien.
        server.handle(
            guest,
            ClientMsg::Look {
                yaw: 1.0,
                pitch: 0.5,
            },
        );
        assert_eq!(server.players.get(guest).unwrap().yaw, 1.0);
        assert_ne!(server.players.local().yaw, 1.0);

        // Un joueur inconnu n'a aucun effet.
        server.replies.clear();
        server.handle(PlayerId(99), ClientMsg::Chat("personne".into()));
        assert!(server.replies.is_empty());
    }

    #[test]
    fn the_views_of_each_player_are_built_on_the_server() {
        let mut server = headless();
        let feet = player::body(&server.ecs, server.players.entity()).position;
        let guest = server.add_ghost("Invité", feet + Vec3::Z * 2.0);
        // Un objet pour l'invité seulement : seule sa vue d'inventaire le montre.
        let stone = server.content.items.id("core:stone").expect("pierre");
        let items = server.content.items.clone();
        server
            .players
            .get_mut(guest)
            .unwrap()
            .inventory
            .add(stone, 5, &items);
        server.replies.clear();
        server.replicate_player_views();
        let inventory_of = |who: PlayerId| {
            server.replies.iter().find_map(|(p, m)| match m {
                ServerMsg::Inventory(view) if *p == who => Some(view.clone()),
                _ => None,
            })
        };
        let host = server.players.local_id();
        let has_stone = |view: Option<crate::player_view::InventoryView>| {
            view.is_some_and(|v| v.slots.iter().flatten().any(|s| s.item == stone))
        };
        assert!(has_stone(inventory_of(guest)));
        assert!(!has_stone(inventory_of(host)));
    }

    #[test]
    fn a_server_without_a_window_steps_the_world_for_two_players() {
        let mut server = headless();
        let feet = player::body(&server.ecs, server.players.entity()).position;
        let guest = server.add_ghost("Invité", feet + Vec3::X * 3.0);
        // Une seconde de jeu : le monde se charge autour des deux joueurs, les envois suivent.
        let time = server.day.time;
        for _ in 0..60 {
            server.frame(1.0 / 60.0);
            server.stream_world();
            server.send_world();
            server.fixed_step(1.0 / 60.0);
        }
        assert_ne!(server.day.time, time, "l'heure avance sans fenêtre");
        assert_eq!(server.sends.tick, 60, "un compteur de pas par pas fixe");
        assert!(server.sends.senders.contains_key(&guest));
        assert!(
            server
                .sends
                .trackers
                .contains_key(&server.players.local_id())
        );
        assert!(
            server.players.get(guest).is_some(),
            "le fantôme reste en vie"
        );
    }

    #[test]
    fn the_window_player_moves_by_its_messages_and_a_stale_move_is_corrected() {
        use crate::protocol::{ClientMsg, ServerMsg};
        let mut server = headless();
        let local = server.players.local_id();
        let feet = player::body(&server.ecs, server.players.entity()).position;
        let moved = |position: Vec3| ClientMsg::Move {
            position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            in_liquid: false,
            walk: Vec3::ZERO,
            jump: false,
            frozen: false,
            mode: player::MoveMode::Walk,
            step: 1,
        };
        // Le serveur ne simule pas ce joueur : il range ce que sa fenêtre a joué.
        server.handle(local, moved(feet + Vec3::X));
        server.fixed_step(1.0 / 60.0);
        let here = player::body(&server.ecs, server.players.entity()).position;
        assert_eq!(here, feet + Vec3::X, "ni gravité ni physique au serveur");
        assert!(
            server
                .replies
                .iter()
                .all(|(_, m)| !matches!(m, ServerMsg::Correct { .. }))
        );
        // Le serveur le déplace (une réapparition) : le mouvement suivant, joué ailleurs, est périmé.
        let spawn = feet + Vec3::Z * 5.0;
        player::set_eye(
            &server.ecs,
            server.players.entity(),
            spawn + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
        );
        server.replies.clear();
        server.handle(local, moved(feet + Vec3::X * 2.0));
        assert_eq!(
            player::body(&server.ecs, server.players.entity()).position,
            spawn
        );
        assert!(server.replies.iter().any(|(id, m)| *id == local
            && matches!(m, ServerMsg::Correct { position, carried: false, .. } if *position == spawn)));
        // D'accord sur la nouvelle place, le mouvement suivant est rangé.
        server.handle(local, moved(spawn + Vec3::X));
        assert_eq!(
            player::body(&server.ecs, server.players.entity()).position,
            spawn + Vec3::X
        );
    }
}
