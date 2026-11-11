//! Un banc d'essai **sans fenêtre** pour tester un mod : le vrai script compilé, dans un
//! vrai petit monde, avec les blocs et les objets du jeu et de ses mods.
//!
//! ```ignore
//! let mut h = Harness::for_mod("example")?;
//! h.place(IVec3::new(0, 100, 0), "example:breaker");
//! let requests = h.event(&ModEvent::BlockBroken { at: GridPos::world(IVec3::ZERO), block: h.block_id("core:stone") })?;
//! assert!(requests.actions.contains(&MachineAction::Log("...".into())));
//! ```
//!
//! Le script doit être compilé (`cargo xtask build-mods`, fait par `cargo xtask test-mod <id>`).
//! Le monde est un terrain plat de graine fixe : les tests sont reproductibles.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use engine_voxel::{
    BlockAccess, BlockId, BlockRegistry, ChunkStreamer, GridPos, ItemRegistry, MachineRecipes,
    StreamingConfig, TerrainGenerator,
};
use glam::{IVec3, Vec3};

use crate::{ModContent, ModEvent, ModSettings, Requests, ScriptHost, ScriptMod, abi, manifest};

/// Un mod chargé dans un petit monde, prêt à recevoir des événements.
pub struct Harness {
    pub streamer: Rc<RefCell<ChunkStreamer>>,
    pub registry: Arc<BlockRegistry>,
    pub script: ScriptMod,
    pub settings: Rc<RefCell<ModSettings>>,
    pub mod_id: String,
}

impl Harness {
    /// Le mod `mod_id` du dépôt de ce moteur (le dossier `mods/` à côté de `assets/`).
    pub fn for_mod(mod_id: &str) -> Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self::load(&root, mod_id)
    }

    /// Charge `core`, les mods de `root/mods` (dans l'ordre de dépendance) et le script de `mod_id`.
    pub fn load(root: &Path, mod_id: &str) -> Result<Self> {
        let assets = root.join("assets");
        let core = manifest::core_mod(&assets.join("core"));
        let order = manifest::load_order(core, manifest::discover(&root.join("mods"))?)?;
        let info = order.iter().find(|m| m.id() == mod_id).with_context(|| {
            format!(
                "mod « {mod_id} » introuvable dans {}",
                root.join("mods").display()
            )
        })?;
        let script_path: PathBuf = info
            .script_path()
            .with_context(|| format!("le mod « {mod_id} » n'a pas de script"))?;
        let dirs: Vec<(String, PathBuf)> = order
            .iter()
            .filter(|m| m.id() != manifest::CORE)
            .map(|m| (m.id().to_string(), m.dir.clone()))
            .collect();

        let registry = Arc::new(BlockRegistry::load_mods(&assets, &dirs)?);
        let items = ItemRegistry::load_mods(&registry, &assets, &dirs)?;
        let mut recipes = MachineRecipes::default();
        for (ns, dir) in std::iter::once(&("core".to_string(), assets.join("core"))).chain(&dirs) {
            let path = dir.join("machine_recipes.ron");
            if path.exists() {
                let text = std::fs::read_to_string(&path)?;
                recipes.add_source(ns, &text, &path.display().to_string(), &items)?;
            }
        }

        let generator = TerrainGenerator::with_registry(1, registry.clone())?;
        let mut streamer = ChunkStreamer::new(
            generator,
            StreamingConfig {
                view_radius: 2,
                ..StreamingConfig::default()
            },
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            streamer.update(Vec3::new(5.0, 40.0, 5.0));
            while streamer.next_event().is_some() {}
            if streamer.is_idle() {
                break;
            }
            if Instant::now() > deadline {
                bail!("le monde de test ne se charge pas");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let streamer = Rc::new(RefCell::new(streamer));

        let bytes = std::fs::read(&script_path).with_context(|| {
            format!(
                "{} manquant : lancer `cargo xtask build-mods`",
                script_path.display()
            )
        })?;
        let settings = Rc::new(RefCell::new(ModSettings::default()));
        let mut host = ScriptHost::new()?;
        host.set_content(ModContent {
            items: Arc::new(items),
            machine_recipes: Arc::new(recipes),
        });
        host.set_settings(settings.clone());
        let script = host.load(mod_id, &bytes, streamer.clone(), registry.clone())?;
        Ok(Self {
            streamer,
            registry,
            script,
            settings,
            mod_id: mod_id.to_string(),
        })
    }

    /// Le numéro d'un bloc (`core:stone`, `example:ruby_stairs[facing=east]`...).
    pub fn block_id(&self, key: &str) -> BlockId {
        self.registry
            .id(key)
            .unwrap_or_else(|| panic!("bloc inconnu : {key}"))
    }

    /// Pose un bloc dans le monde de test (sans prévenir les voisins).
    pub fn place(&mut self, pos: IVec3, key: &str) {
        let block = self.block_id(key);
        self.streamer.borrow_mut().set_block(pos, block);
    }

    /// L'identifiant du bloc à cette position.
    pub fn block(&self, pos: IVec3) -> String {
        let block = self.streamer.borrow().world().block(pos);
        self.registry.def(block).key.clone()
    }

    /// Envoie un événement au script ; renvoie ce qu'il demande au jeu.
    pub fn event(&mut self, event: &ModEvent) -> Result<Requests> {
        self.script.send_event(event)
    }

    /// Réveille des machines du mod : `kind` vaut `abi::MACHINES_PLACED`, `_LOADED`, `_NEIGHBOR`
    /// ou `_TICK`.
    pub fn machines(&mut self, kind: i32, positions: &[IVec3]) -> Result<Requests> {
        let positions: Vec<GridPos> = positions.iter().map(|&p| GridPos::world(p)).collect();
        self.script.machines(kind, &positions)
    }

    /// Un clic droit du joueur sur la machine en `pos` : (le script l'a pris, ses demandes).
    pub fn use_block(&mut self, pos: IVec3) -> Result<(bool, Requests)> {
        self.script.used(GridPos::world(pos))
    }

    /// Une action du joueur dans le menu de la machine en `pos` (voir `menu::UI_*`).
    pub fn ui(&mut self, pos: IVec3, kind: i32, id: i32, value: i32) -> Result<Requests> {
        self.script.ui_event(kind, GridPos::world(pos), id, value)
    }

    /// Met `count` objets `item` dans la case `slot` de l'inventaire du bloc (le crée au besoin).
    pub fn put(&mut self, pos: IVec3, slot: usize, item: &str, count: u32) {
        let mut streamer = self.streamer.borrow_mut();
        let entity = streamer
            .ensure_block_entity(pos)
            .expect("bloc hors du monde de test");
        let caps = entity.caps_mut();
        let inventory = caps
            .inventory
            .get_or_insert_with(|| engine_voxel::Inventory::new(slot + 1));
        if inventory.slots.len() <= slot {
            inventory.slots.resize(slot + 1, None);
        }
        inventory.slots[slot] = Some(engine_voxel::ItemStack {
            item: item.to_string(),
            count,
            damage: 0,
        });
    }

    /// Le contenu d'une case de l'inventaire du bloc : (objet, nombre).
    pub fn slot_of(&self, pos: IVec3, slot: usize) -> Option<(String, u32)> {
        let streamer = self.streamer.borrow();
        let stack = streamer
            .block_entity(pos)?
            .caps
            .as_ref()?
            .inventory
            .as_ref()?
            .slots
            .get(slot)?
            .as_ref()?;
        Some((stack.item.clone(), stack.count))
    }

    /// Impose la vitesse de rotation du bloc, comme le ferait un réseau qui lui en donne.
    pub fn set_speed(&mut self, pos: IVec3, speed: f32) {
        let mut streamer = self.streamer.borrow_mut();
        if let Some(k) = streamer
            .block_entity_mut(pos)
            .and_then(|e| e.caps.as_mut())
            .and_then(|c| c.kinetic.as_mut())
        {
            k.speed = speed;
        }
    }

    /// La vitesse de rotation actuelle du bloc.
    pub fn speed_of(&self, pos: IVec3) -> f32 {
        let streamer = self.streamer.borrow();
        streamer
            .block_entity(pos)
            .and_then(|e| e.caps.as_ref()?.kinetic.as_ref().map(|k| k.speed))
            .unwrap_or(0.0)
    }

    /// Un tick de machines : le budget de temps du mod est rendu.
    pub fn next_tick(&mut self) {
        self.script.begin_tick();
    }
}

/// Les numéros des appels de machines, pour `Harness::machines`.
pub use abi::{MACHINES_LOADED, MACHINES_NEIGHBOR, MACHINES_PLACED, MACHINES_TICK};
