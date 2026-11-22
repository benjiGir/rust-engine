//! Composition des effets WGSL sur les passes du moteur.

use std::borrow::Cow;

use anyhow::{Context, Result, ensure};

use crate::{CHUNK_SHADER_SOURCE, SKY_SHADER_SOURCE};

const EFFECT_START: &str = "// VOXEL_PACK_BEGIN";
const EFFECT_END: &str = "// VOXEL_PACK_END";

/// Les effets d'un pack. Une passe absente utilise son effet intégré.
#[derive(Default, Clone, Copy)]
pub struct ShaderEffects<'a> {
    pub terrain: Option<&'a str>,
    pub sky: Option<&'a str>,
    pub sun_shadows: bool,
    pub sun_rays: bool,
    pub moonlight: bool,
}

/// Une paire de shaders prête à compiler avec les pipelines du moteur.
pub struct ShaderSources<'a> {
    pub(crate) terrain: Cow<'a, str>,
    pub(crate) sky: Cow<'a, str>,
    pub(crate) lighting: Lighting,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Lighting {
    pub sun_shadows: bool,
    pub sun_rays: bool,
    pub moonlight: bool,
}

impl Lighting {
    pub fn needs_shadows(self) -> bool {
        self.sun_shadows || self.sun_rays || self.moonlight
    }
}

impl<'a> ShaderSources<'a> {
    /// Compose les effets avec les passes intégrées.
    pub fn builtin(effects: ShaderEffects<'_>) -> Result<ShaderSources<'static>> {
        ShaderSources::compose(CHUNK_SHADER_SOURCE, SKY_SHADER_SOURCE, effects)
    }

    /// Compose les effets avec les passes fournies, notamment lors d'un rechargement à chaud.
    pub fn compose(terrain: &'a str, sky: &'a str, effects: ShaderEffects<'_>) -> Result<Self> {
        Ok(Self {
            terrain: compose(terrain, effects.terrain).context("effet du terrain")?,
            sky: compose(sky, effects.sky).context("effet du ciel")?,
            lighting: Lighting {
                sun_shadows: effects.sun_shadows,
                sun_rays: effects.sun_rays,
                moonlight: effects.moonlight,
            },
        })
    }

    /// Contrôle la syntaxe et les types WGSL sur le CPU.
    pub fn validate(&self) -> Result<()> {
        let terrain = crate::renderer::terrain_with_flame(&self.terrain);
        for (name, source) in [("terrain", terrain.as_str()), ("ciel", self.sky.as_ref())] {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .map_err(|error| anyhow::anyhow!("{name} :\n{}", error.emit_to_string(source)))?;
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .map_err(|error| anyhow::anyhow!("{name} :\n{}", error.emit_to_string(source)))?;
        }
        Ok(())
    }
}

fn compose<'a>(base: &'a str, effect: Option<&str>) -> Result<Cow<'a, str>> {
    let Some(effect) = effect else {
        return Ok(Cow::Borrowed(base));
    };
    let (before, hook) = base
        .split_once(EFFECT_START)
        .context("début de l'effet intégré introuvable")?;
    let (_, after) = hook
        .split_once(EFFECT_END)
        .context("fin de l'effet intégré introuvable")?;
    ensure!(!effect.trim().is_empty(), "fichier WGSL vide");
    Ok(Cow::Owned(format!("{before}{effect}\n{after}")))
}
