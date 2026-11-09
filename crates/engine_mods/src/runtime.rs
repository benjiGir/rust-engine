//! Les scripts des mods, en WebAssembly, exécutés par `wasmtime` dans un bac à sable.
//!
//! # Pourquoi WebAssembly ?
//!
//! Un mod est du code écrit par quelqu'un d'autre. En WebAssembly, il tourne dans une
//! machine virtuelle qui ne voit **que ce qu'on lui donne** : pas de fichiers, pas de
//! réseau, pas la mémoire du jeu. Il ne peut que appeler les fonctions que le jeu lui
//! expose (les « fonctions hôtes » ci-dessous). Et on peut le borner :
//! - **mémoire** : au plus [`MEMORY_LIMIT`] octets ;
//! - **temps** : chaque appel reçoit du « carburant » ([`FUEL_PER_CALL`]) ; chaque
//!   instruction en consomme. Un mod qui boucle à l'infini tombe en panne sèche, l'appel
//!   s'arrête, et le jeu continue. Un mod qui plante (« trap ») est désactivé, avec un message.
//!
//! # L'interface (ABI)
//!
//! Le mod est un module WebAssembly « simple » (pas le Component Model) qui s'échange
//! uniquement des entiers avec le jeu ; les textes et les listes passent par sa mémoire.
//! Le détail est dans `docs/modding.md` ; la crate `mod_sdk` le cache aux auteurs de mods.
//!
//! Le mod exporte : `memory`, `sdk_alloc(taille) -> pointeur`, `mod_init()`,
//! `on_event(type, pointeur, taille)` et `on_machines(type, pointeur, nombre)`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use engine_voxel::{
    BlockBehavior, BlockId, BlockRegistry, ChunkStreamer, GridPos, MachineAction, MachineContext,
    Machines,
};
use glam::{IVec3, Vec3};
use wasmtime::{
    Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
    TypedFunc,
};

/// Le carburant d'un appel : environ 50 millions d'instructions simples (quelques dizaines
/// de millisecondes au plus). Au-delà, l'appel est interrompu.
pub const FUEL_PER_CALL: u64 = 50_000_000;
/// La mémoire maximale d'un mod.
pub const MEMORY_LIMIT: usize = 32 << 20;

/// Les numéros de l'ABI, partagés avec `mod_sdk`.
pub mod abi;

/// Un événement du jeu, tel que transmis aux mods.
#[derive(Debug, Clone, PartialEq)]
pub enum ModEvent {
    BlockBroken {
        at: GridPos,
        block: BlockId,
    },
    /// `facing` : la direction horizontale vers laquelle regardait le joueur.
    BlockPlaced {
        at: GridPos,
        block: BlockId,
        facing: IVec3,
    },
    EntityDied {
        kind: String,
        position: Vec3,
    },
    Tick(u64),
    PlayerDamaged {
        amount: f32,
        health_left: f32,
    },
}

impl ModEvent {
    fn kind(&self) -> i32 {
        match self {
            Self::BlockBroken { .. } => abi::EVENT_BLOCK_BROKEN,
            Self::BlockPlaced { .. } => abi::EVENT_BLOCK_PLACED,
            Self::EntityDied { .. } => abi::EVENT_ENTITY_DIED,
            Self::Tick(_) => abi::EVENT_TICK,
            Self::PlayerDamaged { .. } => abi::EVENT_PLAYER_DAMAGED,
        }
    }

    /// Les données de l'événement : des entiers 32 bits en petit-boutiste, puis
    /// éventuellement un texte UTF-8.
    fn payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut int = |v: i32| out.extend(v.to_le_bytes());
        match self {
            Self::BlockBroken { at, block } => {
                int(at.grid.0 as i32);
                at.pos.to_array().into_iter().for_each(&mut int);
                int(block.0 as i32);
            }
            Self::BlockPlaced { at, block, facing } => {
                int(at.grid.0 as i32);
                at.pos.to_array().into_iter().for_each(&mut int);
                int(block.0 as i32);
                facing.to_array().into_iter().for_each(&mut int);
            }
            Self::EntityDied { kind, position } => {
                position
                    .floor()
                    .as_ivec3()
                    .to_array()
                    .into_iter()
                    .for_each(&mut int);
                out.extend(kind.as_bytes());
            }
            Self::Tick(tick) => {
                int(*tick as i32);
                int((*tick >> 32) as i32);
            }
            Self::PlayerDamaged {
                amount,
                health_left,
            } => {
                int(amount.to_bits() as i32);
                int(health_left.to_bits() as i32);
            }
        }
        out
    }
}

/// Ce que le script a demandé pendant un appel, que le jeu applique ensuite.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Requests {
    /// Ticks à programmer : (position, délai en ticks).
    pub schedules: Vec<(GridPos, u32)>,
    /// Blocs dont les voisins doivent être prévenus (un bloc ou une puissance a changé).
    pub notify: Vec<GridPos>,
    pub actions: Vec<MachineAction>,
}

impl Requests {
    fn append(&mut self, other: Requests) {
        self.schedules.extend(other.schedules);
        self.notify.extend(other.notify);
        self.actions.extend(other.actions);
    }

    /// Applique les demandes dans un tick de machines.
    pub fn apply(self, ctx: &mut MachineContext) {
        for (pos, delay) in self.schedules {
            ctx.schedule(pos, delay);
        }
        for pos in self.notify {
            ctx.notify_neighbors(pos);
        }
        for action in self.actions {
            ctx.act(action);
        }
    }
}

/// Ce que voient les fonctions hôtes pendant un appel : le monde, et les demandes en cours.
pub struct HostState {
    mod_id: String,
    streamer: Rc<RefCell<ChunkStreamer>>,
    registry: Arc<BlockRegistry>,
    requests: Requests,
    subscriptions: Vec<i32>,
    limits: StoreLimits,
    rng: u64,
}

/// Le moteur WebAssembly et les fonctions que le jeu expose aux mods.
pub struct ScriptHost {
    engine: Engine,
    linker: Linker<HostState>,
}

impl ScriptHost {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        // Chaque instruction consomme du carburant : c'est ce qui borne le temps d'un appel.
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        define_host_functions(&mut linker).map_err(anyhow::Error::from)?;
        Ok(Self { engine, linker })
    }

    /// Charge un script : compile, instancie, puis appelle `mod_init`.
    pub fn load(
        &self,
        mod_id: &str,
        bytes: &[u8],
        streamer: Rc<RefCell<ChunkStreamer>>,
        registry: Arc<BlockRegistry>,
    ) -> Result<ScriptMod> {
        let module = Module::new(&self.engine, bytes)
            .map_err(anyhow::Error::from)
            .context("module WebAssembly invalide")?;
        let state = HostState {
            mod_id: mod_id.to_string(),
            streamer,
            registry,
            requests: Requests::default(),
            subscriptions: Vec::new(),
            // Le bac à sable mémoire : une seule mémoire, de taille bornée.
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .instances(1)
                .build(),
            rng: 0x9E37_79B9_7F4A_7C15 ^ mod_id.len() as u64,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_PER_CALL)?;
        // Seules nos fonctions hôtes sont fournies : pas de WASI, donc ni fichiers, ni réseau.
        let instance = self
            .linker
            .instantiate(&mut store, &module)
            .map_err(anyhow::Error::from)
            .context("le module demande une fonction que le jeu ne fournit pas")?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("le module n'exporte pas sa mémoire")?;
        let alloc = instance.get_typed_func::<i32, i32>(&mut store, "sdk_alloc")?;
        let on_event = instance
            .get_typed_func::<(i32, i32, i32), ()>(&mut store, "on_event")
            .ok();
        let on_machines = instance
            .get_typed_func::<(i32, i32, i32), ()>(&mut store, "on_machines")
            .ok();
        let mut script = ScriptMod {
            id: mod_id.to_string(),
            store,
            _instance: instance,
            memory,
            alloc,
            on_event,
            on_machines,
            disabled: None,
        };
        if let Ok(init) = script
            ._instance
            .get_typed_func::<(), ()>(&mut script.store, "mod_init")
        {
            script.guarded(|store| init.call(store, ()))?;
        }
        Ok(script)
    }
}

/// Un mod chargé.
pub struct ScriptMod {
    id: String,
    store: Store<HostState>,
    _instance: Instance,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    on_event: Option<TypedFunc<(i32, i32, i32), ()>>,
    on_machines: Option<TypedFunc<(i32, i32, i32), ()>>,
    /// La raison de sa désactivation, s'il a planté.
    disabled: Option<String>,
}

impl ScriptMod {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn disabled(&self) -> Option<&str> {
        self.disabled.as_deref()
    }

    pub fn subscribed(&self, kind: i32) -> bool {
        self.store.data().subscriptions.contains(&kind)
    }

    /// Le carburant restant après le dernier appel (pour les mesures).
    pub fn fuel_left(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }

    /// Lance `call` avec un plein de carburant. Si le mod plante ou boucle, il est
    /// désactivé : ses appels suivants ne font plus rien.
    fn guarded(
        &mut self,
        call: impl FnOnce(&mut Store<HostState>) -> wasmtime::Result<()>,
    ) -> Result<Requests> {
        if let Some(reason) = &self.disabled {
            bail!("mod « {} » désactivé : {reason}", self.id);
        }
        self.store.set_fuel(FUEL_PER_CALL)?;
        let result = call(&mut self.store);
        let requests = std::mem::take(&mut self.store.data_mut().requests);
        match result {
            Ok(()) => Ok(requests),
            Err(err) => {
                let reason = match err.downcast_ref::<Trap>() {
                    Some(Trap::OutOfFuel) => {
                        "trop long (boucle infinie ?) : appel interrompu".to_string()
                    }
                    Some(trap) => format!("plantage : {trap}"),
                    None => format!("{err:#}"),
                };
                log::error!("mod « {} » désactivé — {reason}", self.id);
                self.disabled = Some(reason.clone());
                Err(anyhow!("mod « {} » désactivé : {reason}", self.id))
            }
        }
    }

    /// Copie des octets dans la mémoire du mod (via son `sdk_alloc`) ; renvoie le pointeur.
    fn write_guest(&mut self, bytes: &[u8]) -> Result<i32> {
        let alloc = self.alloc.clone();
        let mut ptr = 0;
        self.guarded(|store| {
            ptr = alloc.call(&mut *store, bytes.len() as i32)?;
            Ok(())
        })?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .context("écriture dans la mémoire du mod")?;
        Ok(ptr)
    }

    /// Transmet un événement, si le mod s'y est abonné.
    pub fn send_event(&mut self, event: &ModEvent) -> Result<Requests> {
        let kind = event.kind();
        let Some(on_event) = self.on_event.clone() else {
            return Ok(Requests::default());
        };
        if !self.subscribed(kind) || self.disabled.is_some() {
            return Ok(Requests::default());
        }
        let payload = event.payload();
        let ptr = self.write_guest(&payload)?;
        self.guarded(|store| on_event.call(store, (kind, ptr, payload.len() as i32)))
    }

    /// Réveille les machines du mod : **toutes en un seul appel** (grille, x, y, z à la suite).
    pub fn machines(&mut self, kind: i32, positions: &[GridPos]) -> Result<Requests> {
        let Some(on_machines) = self.on_machines.clone() else {
            return Ok(Requests::default());
        };
        if positions.is_empty() || self.disabled.is_some() {
            return Ok(Requests::default());
        }
        let bytes: Vec<u8> = positions
            .iter()
            .flat_map(|p| [p.grid.0 as i32, p.pos.x, p.pos.y, p.pos.z])
            .flat_map(i32::to_le_bytes)
            .collect();
        let ptr = self.write_guest(&bytes)?;
        self.guarded(|store| on_machines.call(store, (kind, ptr, positions.len() as i32)))
    }
}

/// Le comportement d'un bloc défini par le script d'un mod.
pub struct ScriptBehavior {
    script: Rc<RefCell<ScriptMod>>,
}

impl ScriptBehavior {
    pub fn new(script: Rc<RefCell<ScriptMod>>) -> Self {
        Self { script }
    }

    fn run(&self, ctx: &mut MachineContext, kind: i32, positions: &[GridPos]) {
        // Pendant l'appel, aucun emprunt du streamer n'est tenu : les fonctions hôtes
        // peuvent l'emprunter à leur tour.
        let result = self.script.borrow_mut().machines(kind, positions);
        if let Ok(requests) = result {
            requests.apply(ctx);
        }
    }
}

impl BlockBehavior for ScriptBehavior {
    fn placed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.run(ctx, abi::MACHINES_PLACED, positions);
    }
    fn loaded(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.run(ctx, abi::MACHINES_LOADED, positions);
    }
    fn neighbor_changed(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.run(ctx, abi::MACHINES_NEIGHBOR, positions);
    }
    fn scheduled_tick(&self, ctx: &mut MachineContext, positions: &[GridPos]) {
        self.run(ctx, abi::MACHINES_TICK, positions);
    }
}

/// Tous les scripts chargés.
#[derive(Default)]
pub struct ModRuntime {
    scripts: Vec<Rc<RefCell<ScriptMod>>>,
}

impl ModRuntime {
    pub fn add(&mut self, script: ScriptMod) -> Rc<RefCell<ScriptMod>> {
        let script = Rc::new(RefCell::new(script));
        self.scripts.push(script.clone());
        script
    }

    /// Remplace le script d'un mod (rechargement à chaud) ; renvoie `false` s'il n'existait pas.
    /// Les comportements de blocs gardent le même `Rc` : ils utilisent aussitôt le nouveau script.
    pub fn replace(&mut self, script: ScriptMod) -> bool {
        match self.scripts.iter().find(|s| s.borrow().id() == script.id()) {
            Some(slot) => {
                *slot.borrow_mut() = script;
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    /// Les mods désactivés et pourquoi.
    pub fn disabled(&self) -> Vec<(String, String)> {
        self.scripts
            .iter()
            .filter_map(|s| {
                let s = s.borrow();
                s.disabled().map(|r| (s.id().to_string(), r.to_string()))
            })
            .collect()
    }

    /// Relie chaque bloc dont le comportement appartient à un mod scripté (`example:breaker`
    /// → mod `example`) à ce script.
    pub fn register_behaviors(&self, machines: &mut Machines, registry: &BlockRegistry) {
        for script in &self.scripts {
            let id = script.borrow().id().to_string();
            for (block, def) in registry.iter() {
                let owner = def
                    .behavior
                    .as_deref()
                    .and_then(|b| b.split_once(':'))
                    .map(|(ns, _)| ns);
                if owner == Some(id.as_str()) {
                    machines.register(
                        block,
                        Rc::new(ScriptBehavior {
                            script: script.clone(),
                        }),
                    );
                }
            }
        }
    }

    /// Transmet un événement à tous les mods abonnés ; renvoie leurs demandes.
    pub fn dispatch(&self, event: &ModEvent) -> Requests {
        let mut all = Requests::default();
        for script in &self.scripts {
            if let Ok(requests) = script.borrow_mut().send_event(event) {
                all.append(requests);
            }
        }
        all
    }
}

mod host;
use host::define_host_functions;

#[cfg(test)]
mod tests;
