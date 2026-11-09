//! Le registre des fluides : eau, lave, et ceux des mods (huile, vapeur...).
//!
//! Comme les blocs, les fluides sont décrits par des données (`assets/core/fluids.ron`,
//! `mods/<id>/fluids.ron`) et désignés par un identifiant texte avec espace de noms.
//! Un fluide existe sous deux formes :
//! - **dans un réservoir** ([`crate::FluidTank`]) : une quantité en millibuckets ;
//! - **dans le monde** : un bloc qui le référence (`core:water` a `fluid: "water"`).
//!   L'écoulement (un automate cellulaire avec des ticks programmés) viendra en phase 9c.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

const BUILTIN_CORE: &str = include_str!("../../../assets/core/fluids.ron");

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FluidDef {
    pub id: String,
    pub name: String,
    pub color: (f32, f32, f32, f32),
    /// Ticks entre deux avancées de l'écoulement.
    pub viscosity: u32,
    /// kg/m³, négatif pour un gaz.
    pub density: i32,
    /// Kelvins.
    pub temperature: u32,
    #[serde(default)]
    pub light_emission: u8,
    /// Le bloc de ce fluide dans le monde (sans espace de noms : celui du fichier).
    #[serde(default)]
    pub block: Option<String>,
    /// Surpression admise par cellule de profondeur (voir `fluid_sim.rs`).
    #[serde(default = "default_compressibility")]
    pub compressibility: f32,
    /// Dégâts par seconde pour qui s'y trouve.
    #[serde(default)]
    pub damage: f32,
    /// Une règle de source infinie (désactivée par défaut).
    #[serde(default)]
    pub infinite: bool,
}

fn default_compressibility() -> f32 {
    0.02
}

#[derive(Debug, Clone, Default)]
pub struct FluidRegistry {
    defs: Vec<FluidDef>,
    by_id: HashMap<String, usize>,
}

impl FluidRegistry {
    /// Les fluides de base, intégrés au binaire.
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry
            .add_source("core", BUILTIN_CORE, "core intégré")
            .expect("assets/core/fluids.ron invalide");
        registry
    }

    /// Charge `fluids.ron` de chaque mod, dans l'ordre donné (`(espace de noms, dossier)`).
    pub fn load(mods: &[(String, &Path)]) -> Result<Self> {
        let mut registry = Self::default();
        for (namespace, dir) in mods {
            let path = dir.join("fluids.ron");
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("lecture de {}", path.display()))?;
                registry.add_source(namespace, &text, &path.display().to_string())?;
            }
        }
        Ok(registry)
    }

    pub fn add_source(&mut self, namespace: &str, source: &str, origin: &str) -> Result<()> {
        let options = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let defs: Vec<FluidDef> = options
            .from_str(source)
            .with_context(|| format!("{origin} : fichier RON invalide"))?;
        for mut def in defs {
            // `water` dans le fichier de `core` : le bloc `core:water`.
            def.block = def.block.map(|b| {
                if b.contains(':') {
                    b
                } else {
                    format!("{namespace}:{b}")
                }
            });
            if def.id.split_once(':').map(|(ns, _)| ns) != Some(namespace) {
                bail!(
                    "{origin} : le fluide « {} » doit commencer par « {namespace}: »",
                    def.id
                );
            }
            if def.light_emission > 15 {
                bail!("{origin} : « {} » : la lumière va de 0 à 15", def.id);
            }
            if self.by_id.contains_key(&def.id) {
                bail!("{origin} : le fluide « {} » est défini deux fois", def.id);
            }
            self.by_id.insert(def.id.clone(), self.defs.len());
            self.defs.push(def);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&FluidDef> {
        self.by_id.get(id).map(|&i| &self.defs[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &FluidDef> {
        self.defs.iter()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockRegistry;

    #[test]
    fn core_has_water_and_lava() {
        let fluids = FluidRegistry::builtin();
        assert_eq!(fluids.len(), 2);
        assert!(
            fluids.get("core:lava").unwrap().temperature
                > fluids.get("core:water").unwrap().temperature
        );
    }

    #[test]
    fn the_water_block_is_linked_to_the_water_fluid() {
        let blocks = BlockRegistry::builtin();
        let fluids = FluidRegistry::builtin();
        let water = blocks.def(blocks.id("core:water").unwrap());
        let fluid = water
            .fluid
            .as_deref()
            .expect("core:water doit référencer son fluide");
        assert!(fluids.get(fluid).is_some());
    }

    #[test]
    fn foreign_namespaces_are_refused() {
        let mut fluids = FluidRegistry::default();
        let source = r#"[(id: "core:oil", name: "x", color: (0,0,0,1), viscosity: 1, density: 1, temperature: 1)]"#;
        assert!(fluids.add_source("mymod", source, "test").is_err());
    }
}
