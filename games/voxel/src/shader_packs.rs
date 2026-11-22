//! Packs graphiques du client. La sélection est une préférence locale du joueur. Les packs sont
//! des dossiers de `shader_packs/` à la racine du jeu (`voxel_sim::paths`), à côté des packs de
//! textures : le joueur y dépose les siens. Une installation en lecture seule en a deux (celui du
//! joueur, puis celui de l'installation) : à identifiant égal, le premier passe devant.

use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use engine_render::{Renderer, ShaderEffects, ShaderSources, egui};
use serde::Deserialize;

const API: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    api: u32,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    terrain: Option<String>,
    #[serde(default)]
    sky: Option<String>,
    #[serde(default)]
    sun_shadows: bool,
    #[serde(default)]
    sun_rays: bool,
    #[serde(default)]
    moonlight: bool,
}

fn directory(packs: &[PathBuf], id: &str) -> Result<PathBuf> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_')),
        "identifiant de pack de shaders invalide : {id}"
    );
    // Le premier dossier qui a ce pack ; à défaut, le chemin dans le premier, pour l'erreur.
    let mut candidates = packs.iter().map(|dir| dir.join(id));
    let first = packs.first().map(|dir| dir.join(id));
    Ok(candidates
        .find(|dir| dir.is_dir())
        .or(first)
        .unwrap_or_else(|| PathBuf::from(id)))
}

fn manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("pack.ron");
    let text =
        fs::read_to_string(&path).with_context(|| format!("lecture de {}", path.display()))?;
    let data: Manifest =
        ron::from_str(&text).with_context(|| format!("description de {}", path.display()))?;
    ensure!(
        data.api == API,
        "version d'API {} non prise en charge (attendue : {API})",
        data.api
    );
    ensure!(!data.name.trim().is_empty(), "le nom du pack est vide");
    ensure!(
        data.terrain.is_some()
            || data.sky.is_some()
            || data.sun_shadows
            || data.sun_rays
            || data.moonlight,
        "le pack doit activer un effet de lumière ou fournir un effet de terrain ou de ciel"
    );
    Ok(data)
}

#[derive(Default)]
struct Effects {
    terrain: Option<String>,
    sky: Option<String>,
    sun_shadows: bool,
    sun_rays: bool,
    moonlight: bool,
}

impl Effects {
    fn load(packs: &[PathBuf], id: &str) -> Result<Self> {
        let dir = directory(packs, id)?;
        let data = manifest(&dir)?;
        let read = |name: String| -> Result<String> {
            let file = Path::new(&name);
            ensure!(
                file.file_name().and_then(|name| name.to_str()) == Some(name.as_str())
                    && file.extension().is_some_and(|ext| ext == "wgsl"),
                "nom de fichier WGSL invalide : {name}"
            );
            let path = dir.join(file);
            fs::read_to_string(&path).with_context(|| format!("lecture de {}", path.display()))
        };
        Ok(Self {
            terrain: data.terrain.map(read).transpose()?,
            sky: data.sky.map(read).transpose()?,
            sun_shadows: data.sun_shadows,
            sun_rays: data.sun_rays,
            moonlight: data.moonlight,
        })
    }

    fn sources(&self) -> ShaderEffects<'_> {
        ShaderEffects {
            terrain: self.terrain.as_deref(),
            sky: self.sky.as_deref(),
            sun_shadows: self.sun_shadows,
            sun_rays: self.sun_rays,
            moonlight: self.moonlight,
        }
    }
}

fn read_base(path: &str, builtin: &'static str) -> Result<Cow<'static, str>> {
    // Hors du dépôt, les shaders embarqués : le chemin de compilation ne veut rien dire.
    if !voxel_sim::paths::get().from_repository() {
        return Ok(Cow::Borrowed(builtin));
    }
    match fs::read_to_string(path) {
        Ok(source) => Ok(Cow::Owned(source)),
        // Les éditions distribuées embarquent leurs shaders.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Cow::Borrowed(builtin)),
        Err(error) => Err(error).with_context(|| format!("lecture de {path}")),
    }
}

fn selected_effects(packs: &[PathBuf], id: Option<&str>) -> Result<Effects> {
    Ok(id
        .map(|id| Effects::load(packs, id))
        .transpose()?
        .unwrap_or_default())
}

/// Applique le pack `id` de `packs` (les dossiers des packs), ou le rendu standard.
pub fn apply(renderer: &mut Renderer, packs: &[PathBuf], id: Option<&str>) -> Result<()> {
    let effects = selected_effects(packs, id)?;
    renderer.reload_shaders(&ShaderSources::builtin(effects.sources())?)
}

pub fn reload(renderer: &mut Renderer, packs: &[PathBuf], id: Option<&str>) -> Result<()> {
    let effects = selected_effects(packs, id)?;
    let terrain = read_base(
        engine_render::CHUNK_SHADER_PATH,
        engine_render::CHUNK_SHADER_SOURCE,
    )?;
    let sky = read_base(
        engine_render::SKY_SHADER_PATH,
        engine_render::SKY_SHADER_SOURCE,
    )?;
    let sources = ShaderSources::compose(&terrain, &sky, effects.sources())?;
    renderer.reload_shaders(&sources)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pack {
    id: String,
    title: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    packs: Vec<Pack>,
    pub selected: Option<String>,
    pub error: Option<String>,
}

impl Selection {
    pub fn open(dirs: &[PathBuf], active: Option<&str>) -> Self {
        let mut packs: Vec<Pack> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(id) = entry.file_name().into_string() else {
                    continue;
                };
                let Ok(pack) = directory(std::slice::from_ref(dir), &id) else {
                    continue;
                };
                // Déjà vu dans un dossier précédent (celui du joueur) : celui-là passe devant,
                // comme pour `apply`.
                if !pack.is_dir() || !seen.insert(id.clone()) {
                    continue;
                }
                match manifest(&pack) {
                    Ok(data) => packs.push(Pack {
                        id,
                        title: data.name,
                        description: data.description,
                    }),
                    Err(error) => log::warn!("pack de shaders {id} : {error:#}"),
                }
            }
        }
        packs.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Self {
            packs,
            selected: active.map(str::to_owned),
            error: None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    #[default]
    None,
    Apply,
    Back,
}

pub fn panel(ui: &mut egui::Ui, selection: &mut Selection, active: Option<&str>) -> Action {
    let mut action = Action::None;
    let height = (ui.ctx().content_rect().height() - 145.0).max(36.0);
    egui::ScrollArea::vertical()
        .id_salt("shader_pack_list")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.vertical(|ui| {
                let standard = egui::RichText::new("Standard").strong();
                if ui
                    .selectable_label(selection.selected.is_none(), standard)
                    .clicked()
                {
                    selection.selected = None;
                    selection.error = None;
                }
                for pack in &selection.packs {
                    ui.separator();
                    let selected = selection.selected.as_deref() == Some(&pack.id);
                    let is_active = active == Some(&pack.id);
                    let title = if is_active {
                        format!("{} (actif)", pack.title)
                    } else {
                        pack.title.clone()
                    };
                    if ui
                        .selectable_label(selected, egui::RichText::new(title).strong())
                        .clicked()
                    {
                        selection.selected = Some(pack.id.clone());
                        selection.error = None;
                    }
                    if !pack.description.is_empty() {
                        ui.label(egui::RichText::new(&pack.description).color(crate::skin::MUTED));
                    }
                }
                if let Some(error) = &selection.error {
                    ui.separator();
                    ui.label(egui::RichText::new(error).color(crate::skin::RED));
                }
            });
        });
    ui.add_space(8.0);
    let (apply, back) = crate::ui_layout::paired_buttons(ui, "Appliquer", "Retour");
    if apply {
        action = Action::Apply;
    } else if back {
        action = Action::Back;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les packs livrés, dans le dépôt.
    fn packs() -> Vec<PathBuf> {
        vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../shader_packs")]
    }

    #[test]
    fn the_bundled_pack_and_each_effect_compile_as_wgsl() {
        let packs = packs();
        let selection = Selection::open(&packs, None);
        assert!(selection.packs.iter().any(|pack| pack.id == "crepuscule"));
        let entries = Effects::load(&packs, "crepuscule").unwrap();
        let (terrain, sky) = (entries.terrain.as_deref(), entries.sky.as_deref());
        assert!(terrain.is_some() && sky.is_some());
        assert!(entries.sun_shadows && entries.sun_rays && entries.moonlight);
        for effects in [
            ShaderEffects::default(),
            entries.sources(),
            ShaderEffects {
                terrain,
                ..Default::default()
            },
            ShaderEffects {
                sky,
                ..Default::default()
            },
        ] {
            ShaderSources::builtin(effects).unwrap().validate().unwrap();
        }
    }

    #[test]
    fn manifest_paths_and_invalid_wgsl_are_reported() {
        let root =
            std::env::temp_dir().join(format!("voxel-shader-manifest-{}", std::process::id()));
        let pack = root.join("test");
        fs::create_dir_all(&pack).unwrap();
        let manifest = pack.join("pack.ron");
        fs::write(
            &manifest,
            r#"(api: 1, name: "Test", terrain: Some("../test.wgsl"))"#,
        )
        .unwrap();
        let error = Effects::load(std::slice::from_ref(&root), "test")
            .err()
            .unwrap();
        assert!(error.to_string().contains("nom de fichier WGSL invalide"));
        fs::write(
            &manifest,
            r#"(api: 2, name: "Test", sky: Some("sky.wgsl"))"#,
        )
        .unwrap();
        assert!(Effects::load(std::slice::from_ref(&root), "test").is_err());
        fs::write(
            &manifest,
            r#"(api: 1, name: "Test", sky: Some("sky.wgsl"))"#,
        )
        .unwrap();
        assert!(Effects::load(std::slice::from_ref(&root), "test").is_err());
        fs::write(pack.join("sky.wgsl"), "fn voxel_sky(input: SyntaxError) {}").unwrap();
        let bad = Effects::load(std::slice::from_ref(&root), "test").unwrap();
        assert!(
            ShaderSources::builtin(bad.sources())
                .unwrap()
                .validate()
                .is_err()
        );
        assert!(Effects::load(std::slice::from_ref(&root), "..").is_err());
        fs::write(&manifest, r#"(api: 1, name: "Ombres", sun_shadows: true)"#).unwrap();
        let shadows = Effects::load(std::slice::from_ref(&root), "test").unwrap();
        assert!(shadows.sun_shadows);
        assert!(shadows.terrain.is_none() && shadows.sky.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_player_pack_comes_before_the_shipped_one() {
        let base = std::env::temp_dir().join(format!("voxel-shader-layers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let write = |dir: &str, id: &str, name: &str| {
            let pack = base.join(dir).join(id);
            fs::create_dir_all(&pack).unwrap();
            fs::write(
                pack.join("pack.ron"),
                format!(r#"(api: 1, name: "{name}", sun_shadows: true)"#),
            )
            .unwrap();
        };
        write("joueur", "commun", "Du joueur");
        write("livres", "commun", "Livré");
        write("livres", "autre", "Autre");
        let dirs = [
            base.join("joueur"),
            base.join("livres"),
            base.join("absent"),
        ];
        let selection = Selection::open(&dirs, None);
        let titles: Vec<&str> = selection.packs.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Autre", "Du joueur"]);
        assert_eq!(
            directory(&dirs, "commun").unwrap(),
            base.join("joueur/commun")
        );
        assert_eq!(
            directory(&dirs, "autre").unwrap(),
            base.join("livres/autre")
        );
        assert!(Effects::load(&dirs, "autre").unwrap().sun_shadows);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_shader_menu_can_select_and_apply_at_small_and_large_sizes() {
        for resolution in [egui::vec2(320.0, 240.0), egui::vec2(1280.0, 720.0)] {
            let ctx = egui::Context::default();
            crate::skin::apply(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, resolution);
            let mut selection = Selection::open(&packs(), None);
            let (mut target, mut apply) = (None, None);
            let mut applied = false;
            for frame in 0..10 {
                let click = match frame {
                    3 | 4 => target,
                    7 | 8 => apply,
                    _ => None,
                };
                let events = click.map_or_else(Vec::new, |pos| {
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: matches!(frame, 3 | 7),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                });
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        crate::ui_layout::menu(ui.ctx(), "Packs de shaders", 640., |ui| {
                            applied |= panel(ui, &mut selection, None) == Action::Apply;
                        })
                    },
                );
                output.textures_delta.clear();
                for entry in output.shapes {
                    if let egui::Shape::Text(text) = entry.shape {
                        let bounds = text.visual_bounding_rect();
                        if entry.clip_rect.contains_rect(bounds) && screen.contains_rect(bounds) {
                            match text.galley.text() {
                                "Crépuscule" => target = Some(bounds.center()),
                                "Appliquer" => apply = Some(bounds.center()),
                                _ => (),
                            }
                        }
                    }
                }
            }
            assert_eq!(
                selection.selected.as_deref(),
                Some("crepuscule"),
                "{resolution:?}"
            );
            assert!(applied, "{resolution:?}");
        }
    }
}
