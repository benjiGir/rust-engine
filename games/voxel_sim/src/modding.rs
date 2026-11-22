//! Les mods dans le jeu : leur ordre de chargement, les machines (natives et scriptées)
//! qui tournent à 20 ticks par seconde, et les événements transmis aux scripts.
//!
//! La simulation tourne à 60 pas par seconde ; les machines, comme dans Minecraft,
//! à 20 : un tick de machines tous les 3 pas.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use engine_mods::{ModContent, ModEvent, ModInfo, ModRuntime, ModSettings, ScriptHost};
use engine_voxel::{BlockRegistry, ChunkStreamer, Grids, MachineAction, Machines, TickReport};
use glam::{IVec3, Vec3};

/// Les ticks aléatoires touchent les chunks à moins de 4 chunks (128 blocs) du joueur...
const RANDOM_TICK_RADIUS: i32 = 4;
/// ... avec 24 blocs tirés par chunk et par tick (un bloc donné : toutes les 70 s en moyenne).
const RANDOM_TICKS_PER_CHUNK: u32 = 24;

/// Au plus ce nombre de chunks neufs confiés aux scripts par tick de machines.
const CHUNKS_PER_TICK: usize = 8;

/// Pas de simulation (60 Hz) par tick de machines (20 Hz).
const STEPS_PER_MACHINE_TICK: u32 = 3;

/// L'ordre de chargement des mods de `mods/`, `core` en premier. Un mod illisible est écarté et
/// expliqué dans les logs ; une erreur d'ensemble (dépendance manquante, cycle...) aussi, et le jeu
/// démarre sans les mods : mieux vaut un jeu sans mods qu'un jeu qui ne démarre pas.
pub fn resolve_order(assets: &Path) -> Vec<ModInfo> {
    let core = engine_mods::core_mod(&assets.join("core"));
    // Les mods du jeu (`paths::get().mods`), ou ceux qui sont à côté d'un autre jeu de base (tests).
    let paths = crate::paths::get();
    let mods_dir = if assets == paths.game {
        paths.mods.clone()
    } else {
        assets
            .parent()
            .map(|root| root.join("mods"))
            .unwrap_or_default()
    };
    let scan = engine_mods::scan(&[mods_dir]);
    for reason in &scan.skipped {
        log::warn!("mod écarté : {reason}");
    }
    let result = engine_mods::load_order(core.clone(), scan.mods);
    match result {
        Ok(order) => {
            let names: Vec<String> = order
                .iter()
                .map(|m| format!("{} {}", m.id(), m.manifest.version))
                .collect();
            log::info!("mods, dans l'ordre de chargement : {}", names.join(", "));
            order
        }
        Err(err) => {
            log::error!("mods désactivés : {err:#}");
            vec![core]
        }
    }
}

/// Les dossiers des mods (sans `core`), dans l'ordre : pour les registres.
pub fn mod_dirs(order: &[ModInfo]) -> Vec<(String, PathBuf)> {
    order
        .iter()
        .filter(|m| m.id() != engine_mods::CORE)
        .map(|m| (m.id().to_string(), m.dir.clone()))
        .collect()
}

pub struct ModSystem {
    pub machines: Machines,
    runtime: ModRuntime,
    host: Option<ScriptHost>,
    /// Les mods qui ont un script : (identifiant, fichier `.wasm`).
    scripts: Vec<(String, PathBuf)>,
    world: Rc<RefCell<ChunkStreamer>>,
    /// Les structures mobiles : leurs machines tournent au même rythme que celles du monde.
    pub subgrids: Rc<RefCell<engine_voxel::SubGrids>>,
    registry: Arc<BlockRegistry>,
    steps: u32,
    pub last_tick: TickReport,
    /// Les chunks neufs en attente d'être décorés par les scripts qui s'y intéressent.
    pending_chunks: std::collections::VecDeque<IVec3>,
    /// Compte les ticks de machines : les créatures à comportement pensent un tick sur cinq.
    brain_clock: u32,
    brain_active: Vec<String>,
    /// L'instantané du joueur que les scripts lisent (`player_info`).
    player: engine_mods::player::SharedPlayer,
    /// Les quêtes terminées du monde, que les scripts lisent (`quest_done`).
    quests_done: engine_mods::player::SharedQuests,
    /// Les réglages des mods (`mod_settings.ron`), partagés avec leurs scripts.
    settings: Rc<RefCell<ModSettings>>,
    settings_path: PathBuf,
    /// Les données des mods propres au monde (`mod_data.ron` de son dossier ; `None` : monde éphémère).
    world_data: Rc<RefCell<ModSettings>>,
    world_data_path: Option<PathBuf>,
    /// Les mods qui ont une page d'options : (identifiant, nom), s'ils ont un script.
    pub options_mods: Vec<(String, String)>,
}

impl ModSystem {
    pub fn new(
        order: &[ModInfo],
        world: Rc<RefCell<ChunkStreamer>>,
        registry: Arc<BlockRegistry>,
        content: ModContent,
        root: &Path,
        world_dir: Option<&Path>,
    ) -> Self {
        // Les données des mods propres à ce monde, sauvegardées avec lui.
        let world_data_path = world_dir.map(|d| d.join("mod_data.ron"));
        let world_data = Rc::new(RefCell::new(
            world_data_path
                .as_deref()
                .map_or_else(ModSettings::default, ModSettings::load)
                .for_world(),
        ));
        let settings_path = root.join("mod_settings.ron");
        let settings = Rc::new(RefCell::new(ModSettings::load(&settings_path)));
        let options_mods = order
            .iter()
            .filter(|m| m.manifest.options && m.manifest.script.is_some())
            .map(|m| (m.id().to_string(), m.manifest.name.clone()))
            .collect();
        let player = engine_mods::player::SharedPlayer::default();
        let quests_done = engine_mods::player::SharedQuests::default();
        let mut machines = Machines::with_natives(&registry);
        machines.set_item_rules(content.items.clone(), content.machine_recipes.clone());
        let mut runtime = ModRuntime::default();
        let host = ScriptHost::new()
            .map_err(|err| log::error!("WebAssembly indisponible, scripts désactivés : {err:#}"))
            .ok()
            .map(|mut host| {
                host.set_content(content);
                host.set_settings(settings.clone());
                host.set_world_data(world_data.clone());
                host.set_peers(runtime.peers());
                host.set_player(player.clone());
                host.set_quests_done(quests_done.clone());
                host
            });
        let scripts: Vec<(String, PathBuf)> = order
            .iter()
            .filter_map(|m| Some((m.id().to_string(), m.script_path()?)))
            .collect();
        if let Some(host) = &host {
            for (id, path) in &scripts {
                match load_script(host, id, path, &world, &registry) {
                    Ok(script) => {
                        runtime.add(script);
                        log::info!("script du mod « {id} » chargé ({})", path.display());
                    }
                    Err(err) => log::error!("mod « {id} » : script non chargé : {err:#}"),
                }
            }
        }
        runtime.register_behaviors(&mut machines, &registry);
        Self {
            machines,
            runtime,
            host,
            scripts,
            world,
            subgrids: Rc::new(RefCell::new(engine_voxel::SubGrids::new(registry.clone()))),
            registry,
            steps: 0,
            last_tick: TickReport::default(),
            pending_chunks: std::collections::VecDeque::new(),
            brain_clock: 0,
            brain_active: Vec::new(),
            player,
            quests_done,
            settings,
            settings_path,
            world_data,
            world_data_path,
            options_mods,
        }
    }

    pub fn scripts_loaded(&self) -> usize {
        self.runtime.len()
    }

    /// Un pas de simulation (60 Hz) : relance les machines des chunks chargés, et fait
    /// un tick de machines tous les 3 pas. Renvoie ce que les machines demandent au jeu.
    pub fn fixed_step(&mut self, player: Vec3) -> (Vec<MachineAction>, Option<TickReport>) {
        let _span = engine_trace::span("mods et machines");
        let mut actions = Vec::new();
        let loaded = self.world.borrow_mut().take_loaded_block_entities();
        if !loaded.is_empty() {
            self.machines
                .blocks_loaded(&self.world, &loaded, &mut actions);
        }
        self.steps += 1;
        if self.steps < STEPS_PER_MACHINE_TICK {
            return (actions, None);
        }
        self.steps = 0;
        // Un nouveau tick de machines : chaque mod retrouve son budget de temps.
        self.runtime.begin_tick();
        let report = {
            let subs = self.subgrids.borrow();
            let grids = Grids::with_subgrids(&self.world, &subs);
            self.machines.run_grids_tick(grids, &mut actions)
        };
        self.last_tick = report;
        // La nature autour du joueur : l'herbe, les pousses, les feuilles.
        let center = engine_voxel::World::chunk_containing(player);
        self.machines.run_random_ticks(
            &self.world,
            center,
            RANDOM_TICK_RADIUS,
            RANDOM_TICKS_PER_CHUNK,
            &mut actions,
        );
        // Les chunks neufs : les scripts abonnés les décorent, au plus quelques-uns par tick et
        // seulement quand ils ont du budget (un chunk ne doit pas rester sans sa décoration).
        let fresh = self.world.borrow_mut().take_fresh_chunks();
        if self
            .runtime
            .has_subscribers(engine_mods::abi::EVENT_CHUNK_GENERATED)
        {
            self.pending_chunks.extend(fresh);
            for _ in 0..CHUNKS_PER_TICK {
                if !self
                    .runtime
                    .subscribers_ready(engine_mods::abi::EVENT_CHUNK_GENERATED)
                {
                    break;
                }
                let Some(chunk) = self.pending_chunks.pop_front() else {
                    break;
                };
                actions.extend(self.dispatch(&ModEvent::ChunkGenerated { chunk }));
            }
        } else {
            self.pending_chunks.clear();
        }
        // L'événement « tick » pour les scripts qui s'y sont abonnés.
        let tick = self.machines.scheduler().tick();
        actions.extend(self.dispatch(&ModEvent::Tick(tick)));
        (actions, Some(report))
    }

    /// Change la puissance émise par un bloc actif du monde (un rail détecteur, un chargeur) et
    /// prévient ses voisins. `true` si elle a changé.
    pub fn set_power(&mut self, pos: IVec3, power: u8) -> bool {
        let subs = self.subgrids.borrow();
        let grids = Grids::with_subgrids(&self.world, &subs);
        self.machines
            .set_power(grids, engine_voxel::GridPos::world(pos), power)
    }

    /// Le joueur a changé un bloc.
    pub fn block_changed(&mut self, pos: IVec3) -> Vec<MachineAction> {
        let mut actions = Vec::new();
        self.machines.block_changed(&self.world, pos, &mut actions);
        actions
    }

    /// Le joueur clique sur un bloc actif (levier, bouton, répéteur) : `true` s'il réagit.
    pub fn use_block(&mut self, pos: IVec3) -> (bool, Vec<MachineAction>) {
        let mut actions = Vec::new();
        let used = self.machines.use_block(&self.world, pos, &mut actions);
        (used, actions)
    }

    /// Remplace la liste des quêtes terminées que les scripts lisent (`quest_done`).
    pub fn set_quests_done(&mut self, done: impl IntoIterator<Item = String>) {
        let mut set = self.quests_done.borrow_mut();
        set.clear();
        set.extend(done);
    }

    /// Rafraîchit l'instantané du joueur que les scripts lisent.
    pub fn set_player(&mut self, snapshot: engine_mods::player::PlayerSnapshot) {
        self.player.set(snapshot);
    }

    /// Au moins un script est abonné à ce type d'événement.
    pub fn has_subscribers(&self, kind: i32) -> bool {
        self.runtime.has_subscribers(kind)
    }

    /// Vrai un tick de machines sur cinq (4 fois par seconde) : le moment de confier leurs
    /// créatures aux scripts.
    pub fn brain_due(&mut self, machine_tick: bool) -> bool {
        if !machine_tick {
            return false;
        }
        self.brain_clock = (self.brain_clock + 1) % 5;
        self.brain_clock == 0
    }

    /// Les mods dont le script dirige des créatures.
    /// Les mods qui avaient des créatures dans leur dernier lot : si ce n'est plus le cas, le
    /// script reçoit un lot vide une fois (il peut retirer sa jauge de boss, par exemple).
    pub fn take_brain_leavers(&mut self, now: &[String]) -> Vec<String> {
        let leavers: Vec<String> = self
            .brain_active
            .iter()
            .filter(|m| !now.contains(m))
            .cloned()
            .collect();
        self.brain_active = now.to_vec();
        leavers
    }

    pub fn brain_mods(&self) -> Vec<String> {
        self.runtime.mob_brains()
    }

    /// Confie un lot de créatures au script de `mod_id` : ses ordres.
    pub fn mob_brains(&mut self, mod_id: &str, bytes: &[u8], count: usize) -> Vec<MachineAction> {
        let requests = self.runtime.mobs(mod_id, bytes, count);
        self.settle(requests)
    }

    /// Les commandes `/nom` des mods : (mod, nom, aide).
    pub fn commands(&self) -> Vec<(String, String, String)> {
        self.runtime.commands()
    }

    /// Le joueur tape la commande `/name args` du mod `mod_id` : sa réponse, et ce qu'il demande
    /// au jeu.
    pub fn command(
        &mut self,
        mod_id: &str,
        name: &str,
        args: &str,
    ) -> (Option<String>, Vec<MachineAction>) {
        let (reply, requests) = self.runtime.command(mod_id, name, args);
        (reply, self.settle(requests))
    }

    /// Une action du joueur dans le menu d'une machine du mod `mod_id`.
    pub fn ui_event(
        &mut self,
        mod_id: &str,
        kind: i32,
        at: engine_voxel::GridPos,
        id: i32,
        value: i32,
    ) -> Vec<MachineAction> {
        let requests = self.runtime.ui_event(mod_id, kind, at, id, value);
        self.settle(requests)
    }

    /// Une action du joueur dans la page d'options du mod `mod_id`.
    pub fn options_event(
        &mut self,
        mod_id: &str,
        kind: i32,
        id: i32,
        value: i32,
    ) -> Vec<MachineAction> {
        let requests = self.runtime.options_event(mod_id, kind, id, value);
        self.settle(requests)
    }

    /// Écrit les données du monde des mods si elles ont changé (avec la sauvegarde du monde).
    pub fn save_world_data(&self) {
        if let Some(path) = &self.world_data_path {
            self.world_data.borrow_mut().save_if_dirty(path);
        }
    }

    /// Écrit les réglages des mods s'ils ont changé.
    pub fn save_settings(&self) {
        self.settings
            .borrow_mut()
            .save_if_dirty(&self.settings_path);
    }

    /// Applique les ticks et les voisins demandés par un script ; le reste revient au jeu.
    fn settle(&mut self, requests: engine_mods::Requests) -> Vec<MachineAction> {
        for (at, delay) in requests.schedules {
            self.machines.schedule(at, delay);
        }
        for at in requests.notify {
            self.machines.notify_neighbors(Grids::new(&self.world), at);
        }
        requests.actions
    }

    /// Transmet un événement aux scripts. Ce qu'ils demandent (ticks, voisins à prévenir)
    /// passe par l'ordonnanceur des machines ; le reste revient au jeu.
    pub fn dispatch(&mut self, event: &ModEvent) -> Vec<MachineAction> {
        if self.runtime.is_empty() {
            return Vec::new();
        }
        let requests = self.runtime.dispatch(event);
        self.settle(requests)
    }

    /// Recharge les scripts des mods (un `.wasm` a changé). Un script invalide est
    /// refusé et l'ancien reste en place.
    pub fn reload_scripts(&mut self) {
        let Some(host) = &self.host else { return };
        for (id, path) in &self.scripts {
            match load_script(host, id, path, &self.world, &self.registry) {
                Ok(script) => {
                    if self.runtime.replace(script) {
                        log::info!("script du mod « {id} » rechargé");
                    }
                }
                Err(err) => log::error!(
                    "mod « {id} » : nouveau script refusé, l'ancien est gardé : {err:#}"
                ),
            }
        }
    }

    /// La consommation de chaque script, pour `ctl mods`.
    pub fn mod_stats(&self) -> Vec<(String, engine_mods::ModStats)> {
        self.runtime.stats()
    }

    pub fn disabled(&self) -> Vec<(String, String)> {
        self.runtime.disabled()
    }
}

fn load_script(
    host: &ScriptHost,
    id: &str,
    path: &Path,
    world: &Rc<RefCell<ChunkStreamer>>,
    registry: &Arc<BlockRegistry>,
) -> anyhow::Result<engine_mods::ScriptMod> {
    let bytes = std::fs::read(path)?;
    host.load(id, &bytes, world.clone(), registry.clone())
}

/// Le mode `VOXEL_STRESS=N` : N horloges au rythme le plus rapide (un basculement par
/// tick) dans le ciel autour du point de départ, pour mesurer le coût des machines en jeu.
pub fn build_stress_factory(system: &mut ModSystem, n: usize, around: IVec3) -> usize {
    let Some(clock) = system.registry.id("core:clock") else {
        return 0;
    };
    let mut placed = 0;
    let mut actions = Vec::new();
    let base = around + IVec3::new(-48, 20, -48);
    'fill: for y in (0..64).step_by(2) {
        for x in (0..96).step_by(2) {
            for z in (0..96).step_by(2) {
                if placed == n {
                    break 'fill;
                }
                let pos = base + IVec3::new(x, y, z);
                let mut world = system.world.borrow_mut();
                if !world.set_block_quiet(pos, clock) {
                    continue;
                }
                world.set_block_entity(
                    pos,
                    Some(engine_voxel::BlockEntity {
                        data: vec![1],
                        ..Default::default()
                    }),
                );
                drop(world);
                system
                    .machines
                    .block_changed(&system.world, pos, &mut actions);
                placed += 1;
            }
        }
    }
    placed
}

/// Une durée en millisecondes, pour les logs.
pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Le terrain du jeu et des mods (leurs minerais, biomes et structures), généré selon `version`
/// (celle de `world.ron` : un monde v1 reste v1, un monde v3 reste v3). Un fichier de mod
/// invalide est signalé dans les logs, et le monde se génère sans ce qu'apportent les mods.
pub fn terrain(
    seed: u32,
    registry: &Arc<BlockRegistry>,
    mods: &[(String, PathBuf)],
    dimension: &engine_voxel::DimensionDef,
    version: engine_voxel::GenerationVersion,
) -> anyhow::Result<engine_voxel::TerrainGenerator> {
    match engine_voxel::TerrainGenerator::with_dimension_versioned(
        seed,
        registry.clone(),
        mods,
        dimension,
        version,
    ) {
        Ok(generator) => Ok(generator),
        Err(err) => {
            log::error!("génération du monde des mods refusée : {err:#}");
            engine_voxel::TerrainGenerator::with_registry(seed, registry.clone())?
                .with_version(version)
        }
    }
}
