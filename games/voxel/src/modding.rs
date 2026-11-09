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

use engine_mods::{ModEvent, ModInfo, ModRuntime, ScriptHost};
use engine_voxel::{BlockRegistry, ChunkStreamer, Grids, MachineAction, Machines, TickReport};
use glam::IVec3;

/// Pas de simulation (60 Hz) par tick de machines (20 Hz).
const STEPS_PER_MACHINE_TICK: u32 = 3;

/// L'ordre de chargement des mods de `mods/`, `core` en premier. Une erreur (dépendance
/// manquante, cycle...) est expliquée dans les logs, et le jeu démarre sans les mods :
/// mieux vaut un jeu sans mods qu'un jeu qui ne démarre pas.
pub fn resolve_order(assets: &Path) -> Vec<ModInfo> {
    let core = engine_mods::core_mod(&assets.join("core"));
    let mods_dir = assets
        .parent()
        .map(|root| root.join("mods"))
        .unwrap_or_default();
    let result = engine_mods::discover(&mods_dir)
        .and_then(|mods| engine_mods::load_order(core.clone(), mods));
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
    registry: Arc<BlockRegistry>,
    steps: u32,
    pub last_tick: TickReport,
}

impl ModSystem {
    pub fn new(
        order: &[ModInfo],
        world: Rc<RefCell<ChunkStreamer>>,
        registry: Arc<BlockRegistry>,
    ) -> Self {
        let mut machines = Machines::with_natives(&registry);
        let mut runtime = ModRuntime::default();
        let host = ScriptHost::new()
            .map_err(|err| log::error!("WebAssembly indisponible, scripts désactivés : {err:#}"))
            .ok();
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
            registry,
            steps: 0,
            last_tick: TickReport::default(),
        }
    }

    pub fn scripts_loaded(&self) -> usize {
        self.runtime.len()
    }

    /// Un pas de simulation (60 Hz) : relance les machines des chunks chargés, et fait
    /// un tick de machines tous les 3 pas. Renvoie ce que les machines demandent au jeu.
    pub fn fixed_step(&mut self) -> (Vec<MachineAction>, Option<TickReport>) {
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
        let report = self.machines.run_tick(&self.world, &mut actions);
        self.last_tick = report;
        // L'événement « tick » pour les scripts qui s'y sont abonnés.
        let tick = self.machines.scheduler().tick();
        actions.extend(self.dispatch(&ModEvent::Tick(tick)));
        (actions, Some(report))
    }

    /// Le joueur a changé un bloc.
    pub fn block_changed(&mut self, pos: IVec3) -> Vec<MachineAction> {
        let mut actions = Vec::new();
        self.machines.block_changed(&self.world, pos, &mut actions);
        actions
    }

    /// Transmet un événement aux scripts. Ce qu'ils demandent (ticks, voisins à prévenir)
    /// passe par l'ordonnanceur des machines ; le reste revient au jeu.
    pub fn dispatch(&mut self, event: &ModEvent) -> Vec<MachineAction> {
        if self.runtime.is_empty() {
            return Vec::new();
        }
        let requests = self.runtime.dispatch(event);
        for (at, delay) in requests.schedules {
            self.machines.schedule(at, delay);
        }
        for at in requests.notify {
            self.machines.notify_neighbors(Grids::new(&self.world), at);
        }
        requests.actions
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
