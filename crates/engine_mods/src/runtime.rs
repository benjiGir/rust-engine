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
    BlockBehavior, BlockId, BlockRegistry, ChunkStreamer, GridPos, ItemRegistry, MachineAction,
    MachineContext, MachineRecipes, Machines,
};
use glam::{IVec3, Vec3};
use wasmtime::{
    Config, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, Trap,
    TypedFunc,
};

/// Le carburant d'un appel : environ 50 millions d'instructions simples (quelques dizaines
/// de millisecondes au plus). Au-delà, l'appel est interrompu.
pub const FUEL_PER_CALL: u64 = 50_000_000;
/// Le budget de temps d'un mod par tick de machines (20 par seconde) : des instructions
/// WebAssembly, tous ses appels confondus (événements, machines, tick). Passé ce budget, ses
/// événements du tick sont perdus et ses machines reportées d'un tick : un mod trop gourmand
/// se ralentit lui-même, pas le jeu. Plus petit que [`FUEL_PER_CALL`], qui n'arrête que les
/// boucles infinies.
pub const TICK_BUDGET: u64 = 20_000_000;
/// Un mod qui dépasse son budget pendant ce nombre de ticks de suite (5 secondes) est désactivé.
pub const MAX_OVERRUN_TICKS: u32 = 100;
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
    /// Le joueur a fait un clic droit avec cet objet `usable` en main.
    ItemUsed {
        item: engine_voxel::ItemId,
    },
    /// Une quête du livre vient de se terminer (son identifiant complet).
    QuestCompleted {
        quest: String,
    },
    /// Un chunk (coordonnées de chunk) vient d'être généré pour la première fois.
    ChunkGenerated {
        chunk: IVec3,
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
            Self::ChunkGenerated { .. } => abi::EVENT_CHUNK_GENERATED,
            Self::ItemUsed { .. } => abi::EVENT_ITEM_USED,
            Self::QuestCompleted { .. } => abi::EVENT_QUEST_COMPLETED,
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
            Self::ItemUsed { item } => int(item.0 as i32),
            Self::QuestCompleted { quest } => out.extend(quest.as_bytes()),
            Self::ChunkGenerated { chunk } => {
                int(0);
                chunk.to_array().into_iter().for_each(&mut int);
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

/// Les objets et les recettes que les scripts peuvent consulter.
#[derive(Clone, Default)]
pub struct ModContent {
    pub items: Arc<ItemRegistry>,
    pub machine_recipes: Arc<MachineRecipes>,
    /// Les tables de butin des blocs (`block_drops`).
    pub loot: Arc<engine_voxel::LootTables>,
    /// Les identifiants des types de créatures, dans l'ordre de leur numéro (`mob_kind`).
    pub mob_keys: Arc<Vec<String>>,
    /// La dimension où se trouve le joueur (`dimension` côté script).
    pub dimension: Arc<str>,
}

/// Ce que voient les fonctions hôtes pendant un appel : le monde, et les demandes en cours.
pub struct HostState {
    mod_id: String,
    streamer: Rc<RefCell<ChunkStreamer>>,
    registry: Arc<BlockRegistry>,
    content: ModContent,
    settings: Rc<RefCell<crate::ModSettings>>,
    world_data: Rc<RefCell<crate::ModSettings>>,
    peers: Peers,
    player: crate::player::SharedPlayer,
    /// Les quêtes terminées de ce monde (`quest_done`), que le jeu tient à jour.
    quests_done: crate::player::SharedQuests,
    /// Les commandes déclarées par le script (`register_command`) : (nom, aide).
    commands: Vec<(String, String)>,
    requests: Requests,
    subscriptions: Vec<i32>,
    limits: StoreLimits,
    rng: u64,
}

/// Le moteur WebAssembly et les fonctions que le jeu expose aux mods.
pub struct ScriptHost {
    engine: Engine,
    linker: Linker<HostState>,
    content: ModContent,
    settings: Rc<RefCell<crate::ModSettings>>,
    world_data: Rc<RefCell<crate::ModSettings>>,
    peers: Peers,
    player: crate::player::SharedPlayer,
    quests_done: crate::player::SharedQuests,
}

impl ScriptHost {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        // Chaque instruction consomme du carburant : c'est ce qui borne le temps d'un appel.
        config.consume_fuel(true);
        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        define_host_functions(&mut linker).map_err(anyhow::Error::from)?;
        Ok(Self {
            engine,
            linker,
            content: ModContent::default(),
            settings: Rc::default(),
            world_data: Rc::new(RefCell::new(crate::ModSettings::default().for_world())),
            peers: Peers::default(),
            player: Default::default(),
            quests_done: Default::default(),
        })
    }

    /// Les objets et recettes que les scripts chargés ensuite pourront consulter.
    pub fn set_content(&mut self, content: ModContent) {
        self.content = content;
    }

    /// Les réglages des mods, partagés avec le jeu (qui les écrit dans la page d'options).
    pub fn set_settings(&mut self, settings: Rc<RefCell<crate::ModSettings>>) {
        self.settings = settings;
    }

    /// L'instantané du joueur que le jeu rafraîchit et que les scripts lisent.
    pub fn set_player(&mut self, player: crate::player::SharedPlayer) {
        self.player = player;
    }

    /// Les quêtes terminées que le jeu rafraîchit et que les scripts lisent (`quest_done`).
    pub fn set_quests_done(&mut self, quests: crate::player::SharedQuests) {
        self.quests_done = quests;
    }

    /// Les scripts que les mods peuvent s'appeler (`ModRuntime::peers`).
    pub fn set_peers(&mut self, peers: Peers) {
        self.peers = peers;
    }

    /// Les données du monde des mods, sauvegardées avec le monde.
    pub fn set_world_data(&mut self, data: Rc<RefCell<crate::ModSettings>>) {
        self.world_data = data;
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
            content: self.content.clone(),
            settings: self.settings.clone(),
            world_data: self.world_data.clone(),
            peers: self.peers.clone(),
            player: self.player.clone(),
            quests_done: self.quests_done.clone(),
            commands: Vec::new(),
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
        let on_use = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "on_use")
            .ok();
        let on_ui = instance
            .get_typed_func::<UiArgs, ()>(&mut store, "on_ui")
            .ok();
        let on_call = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "on_call")
            .ok();
        let call_reply = instance
            .get_typed_func::<(), i32>(&mut store, "call_reply")
            .ok();
        let on_mobs = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "on_mobs")
            .ok();
        let on_command = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "on_command")
            .ok();
        let on_options = instance
            .get_typed_func::<(i32, i32, i32), ()>(&mut store, "on_options")
            .ok();
        let mut script = ScriptMod {
            id: mod_id.to_string(),
            store,
            _instance: instance,
            memory,
            alloc,
            on_event,
            on_machines,
            on_use,
            on_ui,
            on_options,
            on_call,
            on_command,
            on_mobs,
            call_reply,
            disabled: None,
            stats: ModStats::default(),
            commands: Vec::new(),
            spent: 0,
            consecutive_overruns: 0,
            tick_budget: TICK_BUDGET,
        };
        if let Ok(init) = script
            ._instance
            .get_typed_func::<(), ()>(&mut script.store, "mod_init")
        {
            script.guarded(|store| init.call(store, ()))?;
        }
        script.commands = std::mem::take(&mut script.store.data_mut().commands);
        Ok(script)
    }
}

/// Ce qu'un mod a consommé (voir [`TICK_BUDGET`]), pour la mesure et le débogage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModStats {
    /// Instructions du dernier tick terminé, et le pire tick.
    pub fuel_last_tick: u64,
    pub fuel_peak_tick: u64,
    /// Appels faits au script, et appels reportés ou perdus faute de budget.
    pub calls: u64,
    pub skipped_calls: u64,
    /// Ticks où le budget a été dépassé.
    pub overrun_ticks: u64,
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
    on_use: Option<TypedFunc<(i32, i32, i32, i32), i32>>,
    on_ui: Option<TypedFunc<UiArgs, ()>>,
    on_options: Option<TypedFunc<(i32, i32, i32), ()>>,
    on_call: Option<TypedFunc<(i32, i32), i32>>,
    on_command: Option<TypedFunc<(i32, i32), i32>>,
    on_mobs: Option<TypedFunc<(i32, i32), ()>>,
    call_reply: Option<TypedFunc<(), i32>>,
    /// La raison de sa désactivation, s'il a planté.
    disabled: Option<String>,
    stats: ModStats,
    /// Ses commandes `/nom` : (nom, aide).
    commands: Vec<(String, String)>,
    /// Instructions dépensées dans le tick en cours, et ticks de dépassement consécutifs.
    spent: u64,
    consecutive_overruns: u32,
    tick_budget: u64,
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
        let used = FUEL_PER_CALL.saturating_sub(self.store.get_fuel().unwrap_or(0));
        self.spent += used;
        self.stats.calls += 1;
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

    /// Le budget de ce tick est épuisé : les appels groupés du jeu attendent le suivant.
    pub fn over_budget(&self) -> bool {
        self.spent >= self.tick_budget
    }

    /// Change le budget par tick (les tests ; le jeu garde [`TICK_BUDGET`]).
    pub fn set_tick_budget(&mut self, budget: u64) {
        self.tick_budget = budget;
    }

    pub fn stats(&self) -> ModStats {
        self.stats
    }

    /// Un nouveau tick de machines : le budget est rendu. Un mod qui dépasse son budget
    /// [`MAX_OVERRUN_TICKS`] ticks de suite est désactivé.
    pub fn begin_tick(&mut self) {
        self.stats.fuel_last_tick = self.spent;
        self.stats.fuel_peak_tick = self.stats.fuel_peak_tick.max(self.spent);
        if self.spent >= self.tick_budget {
            self.stats.overrun_ticks += 1;
            self.consecutive_overruns += 1;
            if self.consecutive_overruns >= MAX_OVERRUN_TICKS && self.disabled.is_none() {
                let reason = format!(
                    "dépasse son budget de temps ({} instructions par tick) depuis {} ticks",
                    self.tick_budget, MAX_OVERRUN_TICKS
                );
                log::error!("mod « {} » désactivé — {reason}", self.id);
                self.disabled = Some(reason);
            }
        } else {
            self.consecutive_overruns = 0;
        }
        self.spent = 0;
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
        if self.over_budget() {
            self.stats.skipped_calls += 1;
            return Ok(Requests::default());
        }
        let payload = event.payload();
        let ptr = self.write_guest(&payload)?;
        self.guarded(|store| on_event.call(store, (kind, ptr, payload.len() as i32)))
    }

    /// Une action du joueur dans le menu de la machine en `at` (`kind` : voir `menu::UI_*`,
    /// `value` : 0 ou 1 pour une case, les bits d'un décimal pour un curseur).
    pub fn ui_event(&mut self, kind: i32, at: GridPos, id: i32, value: i32) -> Result<Requests> {
        let Some(on_ui) = self.on_ui.clone() else {
            return Ok(Requests::default());
        };
        if self.disabled.is_some() {
            return Ok(Requests::default());
        }
        self.guarded(|store| {
            on_ui.call(
                store,
                (
                    kind,
                    at.grid.0 as i32,
                    at.pos.x,
                    at.pos.y,
                    at.pos.z,
                    id,
                    value,
                ),
            )
        })
    }

    /// Un autre mod appelle la fonction `name` de ce mod. Renvoie sa réponse (`None` : il ne
    /// répond pas, ou n'a pas de `on_call`) et ce qu'il a demandé au jeu pendant l'appel.
    pub fn call(&mut self, name: &str, args: &[u8]) -> Result<(Option<Vec<u8>>, Requests)> {
        let (Some(on_call), Some(call_reply)) = (self.on_call.clone(), self.call_reply.clone())
        else {
            return Ok((None, Requests::default()));
        };
        if self.disabled.is_some() || self.over_budget() {
            self.stats.skipped_calls += 1;
            return Ok((None, Requests::default()));
        }
        let mut payload = (name.len() as u16).to_le_bytes().to_vec();
        payload.extend(name.as_bytes());
        payload.extend(args);
        let ptr = self.write_guest(&payload)?;
        let mut len = -1;
        let mut reply_ptr = 0;
        let requests = self.guarded(|store| {
            len = on_call.call(&mut *store, (ptr, payload.len() as i32))?;
            if len >= 0 {
                reply_ptr = call_reply.call(&mut *store, ())?;
            }
            Ok(())
        })?;
        if len < 0 || len as usize > MAX_CALL_BYTES {
            return Ok((None, requests));
        }
        let mut reply = vec![0u8; len as usize];
        self.memory
            .read(&self.store, reply_ptr as u32 as usize, &mut reply)
            .context("lecture de la réponse du mod")?;
        Ok((Some(reply), requests))
    }

    /// Le script a un `on_mobs` (le jeu ne lui envoie des créatures que dans ce cas).
    pub fn has_mobs(&self) -> bool {
        self.on_mobs.is_some()
    }

    /// Confie un lot de créatures (`count` enregistrements, voir `mobs::encode`) au script.
    /// Hors budget, le lot est ignoré : les créatures gardent leurs derniers ordres.
    pub fn mobs(&mut self, bytes: &[u8], count: usize) -> Result<Requests> {
        let Some(on_mobs) = self.on_mobs.clone() else {
            return Ok(Requests::default());
        };
        if self.disabled.is_some() {
            return Ok(Requests::default());
        }
        if self.over_budget() {
            self.stats.skipped_calls += 1;
            return Ok(Requests::default());
        }
        let ptr = self.write_guest(bytes)?;
        self.guarded(|store| on_mobs.call(store, (ptr, count as i32)))
    }

    /// Les commandes `/nom` que ce script a déclarées dans `init` : (nom, aide).
    pub fn commands(&self) -> &[(String, String)] {
        &self.commands
    }

    /// Le joueur tape `/name args`. Renvoie la réponse du script (`None` : rien à dire) et ce
    /// qu'il demande au jeu. Une commande est une action du joueur : elle ne dépend pas du
    /// budget par tick.
    pub fn command(&mut self, name: &str, args: &str) -> Result<(Option<String>, Requests)> {
        let (Some(on_command), Some(call_reply)) =
            (self.on_command.clone(), self.call_reply.clone())
        else {
            return Ok((None, Requests::default()));
        };
        if self.disabled.is_some() {
            return Ok((None, Requests::default()));
        }
        let mut payload = (name.len() as u16).to_le_bytes().to_vec();
        payload.extend(name.as_bytes());
        payload.extend(args.as_bytes());
        let ptr = self.write_guest(&payload)?;
        let mut len = -1;
        let mut reply_ptr = 0;
        let requests = self.guarded(|store| {
            len = on_command.call(&mut *store, (ptr, payload.len() as i32))?;
            if len >= 0 {
                reply_ptr = call_reply.call(&mut *store, ())?;
            }
            Ok(())
        })?;
        if len < 0 || len as usize > MAX_CALL_BYTES {
            return Ok((None, requests));
        }
        let mut reply = vec![0u8; len as usize];
        self.memory
            .read(&self.store, reply_ptr as u32 as usize, &mut reply)
            .context("lecture de la réponse du mod")?;
        Ok((Some(String::from_utf8_lossy(&reply).into_owned()), requests))
    }

    /// La page d'options : `kind` vaut `menu::UI_OPEN` à l'ouverture, puis les actions du
    /// joueur et `menu::UI_CLOSED`. Le script répond en rappelant `options_show`.
    pub fn options_event(&mut self, kind: i32, id: i32, value: i32) -> Result<Requests> {
        let Some(on_options) = self.on_options.clone() else {
            return Ok(Requests::default());
        };
        if self.disabled.is_some() {
            return Ok(Requests::default());
        }
        self.guarded(|store| on_options.call(store, (kind, id, value)))
    }

    /// Le joueur fait un clic droit sur une machine du mod. Renvoie si le script a pris le
    /// clic en compte (sinon le jeu continue : poser un bloc contre elle, par exemple).
    pub fn used(&mut self, at: GridPos) -> Result<(bool, Requests)> {
        let Some(on_use) = self.on_use.clone() else {
            return Ok((false, Requests::default()));
        };
        if self.disabled.is_some() {
            return Ok((false, Requests::default()));
        }
        let mut handled = 0;
        let requests = self.guarded(|store| {
            handled = on_use.call(store, (at.grid.0 as i32, at.pos.x, at.pos.y, at.pos.z))?;
            Ok(())
        })?;
        Ok((handled != 0, requests))
    }

    /// Réveille les machines du mod : **toutes en un seul appel** (grille, x, y, z à la suite).
    pub fn machines(&mut self, kind: i32, positions: &[GridPos]) -> Result<Requests> {
        let Some(on_machines) = self.on_machines.clone() else {
            return Ok(Requests::default());
        };
        if positions.is_empty() || self.disabled.is_some() {
            return Ok(Requests::default());
        }
        if self.over_budget() {
            // Hors budget : les machines sont réveillées au tick suivant (par un tick programmé).
            self.stats.skipped_calls += 1;
            return Ok(Requests {
                schedules: positions.iter().map(|&p| (p, 1)).collect(),
                ..Requests::default()
            });
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

/// Les scripts chargés, avec leur identifiant : ce que voit un script qui en appelle un autre.
/// L'identifiant est gardé à côté pour chercher sans emprunter un script en cours d'appel.
pub type Peers = Rc<RefCell<Vec<(String, Rc<RefCell<ScriptMod>>)>>>;

/// Au plus ce nombre d'octets pour les arguments et la réponse d'un appel entre mods.
pub const MAX_CALL_BYTES: usize = 64 * 1024;

/// Les arguments de `on_ui` : type, grille, x, y, z, identifiant, valeur.
type UiArgs = (i32, i32, i32, i32, i32, i32, i32);

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
    fn used(&self, ctx: &mut MachineContext, at: GridPos) -> bool {
        match self.script.borrow_mut().used(at) {
            Ok((handled, requests)) => {
                requests.apply(ctx);
                handled
            }
            Err(_) => false,
        }
    }
}

/// Tous les scripts chargés.
#[derive(Default)]
pub struct ModRuntime {
    scripts: Vec<Rc<RefCell<ScriptMod>>>,
    peers: Peers,
}

impl ModRuntime {
    pub fn add(&mut self, script: ScriptMod) -> Rc<RefCell<ScriptMod>> {
        let id = script.id().to_string();
        let script = Rc::new(RefCell::new(script));
        self.scripts.push(script.clone());
        self.peers.borrow_mut().push((id, script.clone()));
        script
    }

    /// La liste que les scripts consultent pour s'appeler : à donner au `ScriptHost`
    /// (`set_peers`) avant de charger les scripts.
    pub fn peers(&self) -> Peers {
        self.peers.clone()
    }

    /// Appelle la fonction `name` du mod `mod_id` (pour le jeu et les tests ; entre mods, voir
    /// `call_mod`). Renvoie sa réponse et ce qu'il demande au jeu.
    pub fn call(&self, mod_id: &str, name: &str, args: &[u8]) -> (Option<Vec<u8>>, Requests) {
        let Some(script) = self.scripts.iter().find(|s| s.borrow().id() == mod_id) else {
            return (None, Requests::default());
        };
        let result = script.borrow_mut().call(name, args);
        result.unwrap_or_default()
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

    /// Transmet une action de la page d'options au script du mod `mod_id`.
    pub fn options_event(&self, mod_id: &str, kind: i32, id: i32, value: i32) -> Requests {
        let Some(script) = self.scripts.iter().find(|s| s.borrow().id() == mod_id) else {
            return Requests::default();
        };
        let result = script.borrow_mut().options_event(kind, id, value);
        result.unwrap_or_default()
    }

    /// Transmet une action de menu au script du mod `mod_id`.
    pub fn ui_event(&self, mod_id: &str, kind: i32, at: GridPos, id: i32, value: i32) -> Requests {
        let Some(script) = self.scripts.iter().find(|s| s.borrow().id() == mod_id) else {
            return Requests::default();
        };
        let result = script.borrow_mut().ui_event(kind, at, id, value);
        result.unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    /// Au moins un script est abonné à ce type d'événement.
    pub fn has_subscribers(&self, kind: i32) -> bool {
        self.scripts.iter().any(|s| s.borrow().subscribed(kind))
    }

    /// Tous les scripts abonnés à ce type d'événement ont encore du budget ce tick-ci : un
    /// événement qui ne doit pas se perdre (un chunk à décorer) ne part que dans ce cas.
    pub fn subscribers_ready(&self, kind: i32) -> bool {
        self.scripts.iter().all(|s| {
            let s = s.borrow();
            !s.subscribed(kind) || !s.over_budget()
        })
    }

    /// Confie un lot de créatures au script du mod `mod_id`.
    pub fn mobs(&self, mod_id: &str, bytes: &[u8], count: usize) -> Requests {
        let Some(script) = self.scripts.iter().find(|s| s.borrow().id() == mod_id) else {
            return Requests::default();
        };
        let result = script.borrow_mut().mobs(bytes, count);
        result.unwrap_or_default()
    }

    /// Les mods dont le script dirige des créatures (il a un `on_mobs`).
    pub fn mob_brains(&self) -> Vec<String> {
        self.scripts
            .iter()
            .filter(|s| s.borrow().has_mobs())
            .map(|s| s.borrow().id().to_string())
            .collect()
    }

    /// Les commandes de tous les scripts : (mod, nom, aide).
    pub fn commands(&self) -> Vec<(String, String, String)> {
        self.scripts
            .iter()
            .flat_map(|s| {
                let s = s.borrow();
                let id = s.id().to_string();
                s.commands()
                    .iter()
                    .map(|(name, help)| (id.clone(), name.clone(), help.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Exécute la commande `/name args` du mod `mod_id`.
    pub fn command(&self, mod_id: &str, name: &str, args: &str) -> (Option<String>, Requests) {
        let Some(script) = self.scripts.iter().find(|s| s.borrow().id() == mod_id) else {
            return (None, Requests::default());
        };
        let result = script.borrow_mut().command(name, args);
        result.unwrap_or_default()
    }

    /// Un nouveau tick de machines : chaque mod retrouve son budget de temps.
    pub fn begin_tick(&self) {
        for script in &self.scripts {
            script.borrow_mut().begin_tick();
        }
    }

    /// La consommation de chaque mod, pour la mesure.
    pub fn stats(&self) -> Vec<(String, ModStats)> {
        self.scripts
            .iter()
            .map(|s| {
                let s = s.borrow();
                (s.id().to_string(), s.stats())
            })
            .collect()
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
