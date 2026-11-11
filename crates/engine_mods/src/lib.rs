//! Les mods : leur manifeste (`mod.ron`), l'ordre dans lequel les charger, et leurs
//! scripts WebAssembly exécutés dans un bac à sable.
//!
//! Voir `docs/modding.md` pour écrire un mod.

pub mod manifest;
pub mod menu;
pub mod runtime;

pub use manifest::{
    CORE, Dependency, ModInfo, ModManifest, core_mod, discover, load_order, read_manifest,
};
pub use menu::{ModMenu, Widget};
pub use runtime::{
    FUEL_PER_CALL, MEMORY_LIMIT, ModContent, ModEvent, ModRuntime, Requests, ScriptBehavior,
    ScriptHost, ScriptMod, abi,
};
