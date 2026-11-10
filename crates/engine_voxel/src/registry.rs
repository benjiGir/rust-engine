//! Le registre des blocs : quels blocs existent et quelles sont leurs propriétés.
//!
//! Les blocs ne sont pas écrits dans le code : ils sont décrits dans des fichiers de
//! données (`assets/core/blocks.ron`, puis `mods/<nom>/blocks.ron`), lus au démarrage.
//! Ajouter un bloc ne demande donc pas de recompiler le jeu : c'est la base des mods.
//!
//! # Deux identifiants par bloc
//!
//! - Un **identifiant texte** avec espace de noms, comme Minecraft : `core:stone`,
//!   `example:ruby`. L'espace de noms (le dossier du mod) évite les collisions :
//!   deux mods peuvent chacun avoir un bloc `ruby` sans se gêner. C'est lui qui est
//!   stable dans le temps et qu'on utilise dans les fichiers.
//! - Un **identifiant numérique** [`BlockId`] (`u16`), attribué au chargement dans
//!   l'ordre de lecture (core, puis les mods par ordre alphabétique). C'est lui qu'on
//!   stocke dans les chunks : 2 octets par bloc, comparaisons instantanées.
//!
//! Le numéro dépend des mods installés : ajouter ou retirer un mod décale les numéros
//! des suivants. Il ne faut donc **jamais** écrire ces numéros sur disque tels quels.
//! Une sauvegarde (phase 8) devra écrire aussi la table « numéro → identifiant texte »,
//! et la retraduire au chargement avec le registre du moment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;
use crate::items::ToolKind;

/// L'espace de noms du jeu lui-même. Aucun mod ne peut l'utiliser.
pub const CORE_NAMESPACE: &str = "core";

/// Les blocs de base, intégrés au binaire : utilisés par les tests et si le dossier
/// `assets` est introuvable. `include_str!` lit le fichier à la compilation.
const BUILTIN_CORE: &str = include_str!("../../../assets/core/blocks.ron");

/// Les propriétés d'un bloc.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockDef {
    /// Identifiant texte, par exemple `core:stone`.
    pub key: String,
    /// Nom affiché.
    pub name: String,
    pub color: [f32; 3],
    /// Le joueur ne peut pas le traverser.
    pub solid: bool,
    /// On voit à travers : les faces des blocs voisins restent dessinées.
    pub transparent: bool,
    /// Le joueur peut le viser et le casser.
    pub selectable: bool,
    /// Il apparaît dans la barre de blocs.
    pub placeable: bool,
    /// Opacité du rendu : 1 = opaque. En dessous de 1, le bloc est dessiné dans la
    /// passe translucide (eau, verre).
    pub alpha: f32,
    /// Un liquide : on peut y nager, et poser un bloc à sa place.
    pub liquid: bool,
    /// Lumière émise, de 0 (rien) à 15 (le maximum, comme le soleil).
    pub light_emission: u8,
    /// Combien la lumière perd en traversant ce bloc : 0 = rien (air, verre),
    /// 15 = tout (bloc opaque). L'eau en absorbe un peu.
    pub light_opacity: u8,
    /// Couche de texture de chaque face : dessus, côtés, dessous.
    /// 0 = pas de texture (la couleur seule).
    pub face_textures: [u32; 3],
    /// Sons du bloc (identifiants namespacés, par exemple `core:step_soft`).
    pub sounds: BlockSounds,
    /// Le comportement du bloc s'il est actif (une machine) : `core:lamp`, `example:breaker`...
    pub behavior: Option<String>,
    /// Le fluide que ce bloc représente dans le monde (`core:water`), voir `FluidRegistry`.
    pub fluid: Option<String>,
    /// Indestructible (le socle du monde).
    pub unbreakable: bool,
    pub shape: Shape,
    /// Des pixels transparents dans la texture (feuilles, plantes) : ils sont découpés
    /// (« alpha cutout ») au lieu d'être mélangés. Dessiné dans la passe opaque.
    pub cutout: bool,
    pub tint: Tint,
    /// Se balance au vent (feuilles, herbes).
    pub sway: bool,
    /// Secondes de minage à la main (avant les bonus d'outil). Négatif : incassable.
    pub hardness: f32,
    /// L'outil adapté, qui va plus vite.
    pub tool: Option<ToolKind>,
    /// Le niveau d'outil minimal pour que le bloc lâche son butin (0 : à la main).
    pub tier: u8,
    pub support: Support,
    /// Tombe s'il n'a rien dessous.
    pub gravity: bool,
    /// L'expérience gagnée en le cassant (min, max), s'il lâche son butin : les minerais.
    pub xp: (u32, u32),
    /// D'où vient la définition, pour les messages d'erreur.
    pub source: String,
}

/// Les sons d'un bloc. `None` : silence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockSounds {
    #[serde(default)]
    pub step: Option<String>,
    /// `break` est un mot réservé de Rust : le champ s'appelle `break_` dans le code,
    /// et `rename` lui garde le nom `break` dans les fichiers.
    #[serde(default, rename = "break")]
    pub break_: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
}

/// La forme d'un bloc.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Shape {
    /// Un cube plein (ou transparent).
    #[default]
    Cube,
    /// Deux panneaux en X qui se croisent : herbes, fleurs, pousses, champignons.
    Cross,
    /// Un panneau horizontal posé juste au-dessus du bas du bloc : un nénuphar.
    Flat,
}

/// La teinte qui dépend du biome (multipliée à la couleur de la texture).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Tint {
    #[default]
    None,
    /// La couleur de l'herbe du biome (le dessus de l'herbe, les herbes hautes).
    Grass,
    /// La couleur du feuillage du biome (les feuilles).
    Foliage,
    /// La couleur de l'eau du biome (l'eau verte des marais, bleu profond des océans).
    Water,
}

/// Ce dont un bloc a besoin pour tenir en place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Support {
    /// Rien : il tient tout seul.
    #[default]
    None,
    /// Un bloc plein dessous (les plantes, les pousses).
    Floor,
    /// Un bloc plein sur une des six faces (la torche).
    Any,
    /// De l'eau dessous (le nénuphar).
    Water,
}

/// Les textures d'un bloc dans un fichier : une par face (dessus, côtés, dessous).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaceTextures {
    top: String,
    side: String,
    bottom: String,
}

/// Un bloc tel qu'écrit dans un fichier `blocks.ron`.
///
/// `Deserialize` (de serde) génère le code qui lit cette struct depuis le fichier.
/// `deny_unknown_fields` refuse un champ mal orthographié au lieu de l'ignorer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFileEntry {
    id: String,
    name: String,
    color: (f32, f32, f32),
    #[serde(default = "yes")]
    solid: bool,
    #[serde(default)]
    transparent: bool,
    #[serde(default = "yes")]
    selectable: bool,
    #[serde(default = "yes")]
    placeable: bool,
    #[serde(default = "one")]
    alpha: f32,
    #[serde(default)]
    liquid: bool,
    #[serde(default)]
    light_emission: u8,
    /// `None` si absent : on le déduit de `transparent` (0 ou 15).
    #[serde(default)]
    light_opacity: Option<u8>,
    /// Une texture pour toutes les faces...
    #[serde(default)]
    texture: Option<String>,
    /// ... ou une par face.
    #[serde(default)]
    textures: Option<FaceTextures>,
    #[serde(default)]
    sounds: BlockSounds,
    #[serde(default)]
    behavior: Option<String>,
    #[serde(default)]
    fluid: Option<String>,
    #[serde(default)]
    unbreakable: bool,
    #[serde(default)]
    shape: Shape,
    #[serde(default)]
    cutout: bool,
    #[serde(default)]
    tint: Tint,
    #[serde(default)]
    sway: bool,
    #[serde(default = "one")]
    hardness: f32,
    #[serde(default)]
    tool: Option<ToolKind>,
    #[serde(default)]
    tier: u8,
    #[serde(default)]
    support: Support,
    #[serde(default)]
    gravity: bool,
    #[serde(default)]
    xp: (u32, u32),
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// Tous les blocs connus. L'indice dans `defs` est le `BlockId`.
#[derive(Debug, Clone)]
pub struct BlockRegistry {
    defs: Vec<BlockDef>,
    by_key: HashMap<String, BlockId>,
    /// Les textures, dans l'ordre de leurs couches (la couche 0, blanche, n'y est pas).
    textures: Vec<String>,
    /// Le dossier de chaque espace de noms : `assets/core`, `mods/example`...
    /// Les fichiers d'un mod sont cherchés dans le sien.
    asset_dirs: HashMap<String, PathBuf>,
}

impl BlockRegistry {
    /// Un registre qui ne contient que l'air (id 0, réservé).
    pub fn empty() -> Self {
        let air = BlockDef {
            key: format!("{CORE_NAMESPACE}:air"),
            name: "Air".into(),
            color: [0.0; 3],
            solid: false,
            transparent: true,
            selectable: false,
            placeable: false,
            alpha: 0.0,
            liquid: false,
            light_emission: 0,
            light_opacity: 0,
            face_textures: [0; 3],
            sounds: BlockSounds::default(),
            behavior: None,
            fluid: None,
            unbreakable: false,
            shape: Shape::Cube,
            cutout: false,
            tint: Tint::None,
            sway: false,
            hardness: 0.0,
            tool: None,
            tier: 0,
            support: Support::None,
            gravity: false,
            xp: (0, 0),
            source: "intégré".into(),
        };
        Self {
            by_key: HashMap::from([(air.key.clone(), BlockId::AIR)]),
            defs: vec![air],
            textures: Vec::new(),
            asset_dirs: HashMap::new(),
        }
    }

    /// Les blocs de base intégrés au binaire, sans mod.
    ///
    /// `OnceLock` : le registre est construit au premier appel puis partagé.
    /// Les appels suivants (de n'importe quel thread) reçoivent le même `Arc`.
    pub fn builtin() -> Arc<Self> {
        static BUILTIN: OnceLock<Arc<BlockRegistry>> = OnceLock::new();
        BUILTIN
            .get_or_init(|| {
                let mut registry = Self::empty();
                registry
                    .add_source(CORE_NAMESPACE, BUILTIN_CORE, "core intégré")
                    .expect("assets/core/blocks.ron invalide");
                Arc::new(registry)
            })
            .clone()
    }

    /// Charge `assets/core/blocks.ron`, puis `mods/<nom>/blocks.ron` pour chaque
    /// dossier de `mods_dir` (s'il existe), par ordre alphabétique.
    ///
    /// Le jeu utilise plutôt [`BlockRegistry::load_mods`] avec l'ordre de chargement
    /// calculé depuis les dépendances des mods (`engine_mods::load_order`).
    pub fn load(assets_dir: &Path, mods_dir: Option<&Path>) -> Result<Self> {
        let mut mods = Vec::new();
        if let Some(mods_dir) = mods_dir.filter(|dir| dir.is_dir()) {
            // `collect::<Result<Vec<_>, _>>()` : la première erreur interrompt tout.
            let mut dirs = std::fs::read_dir(mods_dir)
                .with_context(|| format!("lecture de {}", mods_dir.display()))?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()?;
            dirs.retain(|path| path.is_dir());
            // Trier rend l'ordre (donc les numéros) identique sur toutes les machines :
            // `read_dir` ne garantit aucun ordre.
            dirs.sort();
            for dir in dirs {
                let namespace = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .with_context(|| format!("nom de mod illisible : {}", dir.display()))?
                    .to_string();
                mods.push((namespace, dir));
            }
        }
        Self::load_mods(assets_dir, &mods)
    }

    /// Charge `assets/core/blocks.ron`, puis le `blocks.ron` de chaque mod **dans l'ordre
    /// donné** : `(espace de noms, dossier)`. L'ordre fixe les numéros des blocs.
    pub fn load_mods(assets_dir: &Path, mods: &[(String, PathBuf)]) -> Result<Self> {
        let mut registry = Self::empty();
        registry.add_file(CORE_NAMESPACE, &assets_dir.join("core").join("blocks.ron"))?;
        for (namespace, dir) in mods {
            if namespace == CORE_NAMESPACE {
                bail!(
                    "le mod {} ne peut pas s'appeler « core », ce nom est réservé",
                    dir.display()
                );
            }
            let file = dir.join("blocks.ron");
            if file.exists() {
                registry.add_file(namespace, &file)?;
            }
            // Sinon : un mod sans nouveaux blocs.
        }
        Ok(registry)
    }

    fn add_file(&mut self, namespace: &str, path: &Path) -> Result<()> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        // Le dossier qui contient `blocks.ron` contient aussi `textures/` et `sounds/`.
        if let Some(dir) = path.parent() {
            self.asset_dirs
                .insert(namespace.to_string(), dir.to_path_buf());
        }
        self.add_source(namespace, &source, &path.display().to_string())
    }

    /// Ajoute les blocs décrits par `source` (texte RON). `origin` sert aux messages d'erreur.
    pub fn add_source(&mut self, namespace: &str, source: &str, origin: &str) -> Result<()> {
        // `IMPLICIT_SOME` : un champ facultatif s'écrit `light_opacity: 2`
        // au lieu de `light_opacity: Some(2)`, plus simple pour les auteurs de mods.
        let options = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let entries: Vec<BlockFileEntry> = options
            .from_str(source)
            .with_context(|| format!("{origin} : fichier RON invalide"))?;
        for entry in entries {
            validate_key(&entry.id, namespace).with_context(|| origin.to_string())?;
            if let Some(&existing) = self.by_key.get(&entry.id) {
                bail!(
                    "le bloc « {} » est défini deux fois : dans {} puis dans {origin}",
                    entry.id,
                    self.def(existing).source
                );
            }
            // `try_from` échoue au-delà de 65 535 : pas de débordement silencieux.
            let id = u16::try_from(self.defs.len())
                .map(BlockId)
                .context("trop de blocs (65 535 au maximum)")?;
            let (r, g, b) = entry.color;
            // Textures : `texture` (toutes les faces) ou `textures` (dessus, côtés, dessous).
            let names = match (&entry.texture, &entry.textures) {
                (Some(_), Some(_)) => {
                    bail!(
                        "{origin} : « {} » : `texture` ou `textures`, pas les deux",
                        entry.id
                    )
                }
                (Some(all), None) => Some([all.clone(), all.clone(), all.clone()]),
                (None, Some(f)) => Some([f.top.clone(), f.side.clone(), f.bottom.clone()]),
                (None, None) => None,
            };
            let face_textures = match names {
                Some(names) => names.map(|name| self.texture_layer(&qualify(&name, namespace))),
                None => [0; 3],
            };
            let q = |name: &Option<String>| name.as_ref().map(|n| qualify(n, namespace));
            let sounds = BlockSounds {
                step: q(&entry.sounds.step),
                break_: q(&entry.sounds.break_),
                place: q(&entry.sounds.place),
            };
            if entry.light_emission > 15 || entry.light_opacity.is_some_and(|o| o > 15) {
                bail!("{origin} : « {} » : la lumière va de 0 à 15", entry.id);
            }
            if !(0.0..=1.0).contains(&entry.alpha) {
                bail!("{origin} : « {} » : alpha va de 0 à 1", entry.id);
            }
            // Par défaut, un bloc transparent laisse passer la lumière, un opaque l'arrête.
            let light_opacity =
                entry
                    .light_opacity
                    .unwrap_or(if entry.transparent { 0 } else { 15 });
            self.by_key.insert(entry.id.clone(), id);
            self.defs.push(BlockDef {
                key: entry.id,
                name: entry.name,
                color: [r, g, b],
                solid: entry.solid,
                transparent: entry.transparent,
                selectable: entry.selectable,
                placeable: entry.placeable,
                alpha: entry.alpha,
                liquid: entry.liquid,
                light_emission: entry.light_emission,
                light_opacity,
                face_textures,
                sounds,
                behavior: entry.behavior.as_ref().map(|b| qualify(b, namespace)),
                fluid: entry.fluid.as_ref().map(|f| qualify(f, namespace)),
                unbreakable: entry.unbreakable,
                shape: entry.shape,
                cutout: entry.cutout,
                tint: entry.tint,
                sway: entry.sway,
                // Le socle incassable : une dureté négative.
                hardness: if entry.unbreakable {
                    -1.0
                } else {
                    entry.hardness
                },
                tool: entry.tool,
                tier: entry.tier,
                support: entry.support,
                gravity: entry.gravity,
                xp: entry.xp,
                source: origin.to_string(),
            });
        }
        Ok(())
    }

    /// La couche d'une texture (créée au premier usage). La couche 0 est réservée au blanc.
    fn texture_layer(&mut self, key: &str) -> u32 {
        let index = match self.textures.iter().position(|t| t == key) {
            Some(index) => index,
            None => {
                self.textures.push(key.to_string());
                self.textures.len() - 1
            }
        };
        index as u32 + 1
    }

    /// Les textures dans l'ordre de leurs couches : la couche `i + 1` est `textures()[i]`.
    pub fn textures(&self) -> &[String] {
        &self.textures
    }

    /// Le fichier d'un asset : `core:dirt` en `textures` → `assets/core/textures/dirt.png`.
    /// `None` si l'espace de noms n'a pas de dossier (registre intégré, tests).
    pub fn asset_path(&self, key: &str, kind: &str, extension: &str) -> Option<PathBuf> {
        let (namespace, name) = key.split_once(':')?;
        let dir = self.asset_dirs.get(namespace)?;
        Some(dir.join(kind).join(format!("{name}.{extension}")))
    }

    /// Le numéro d'un bloc à partir de son identifiant texte.
    pub fn id(&self, key: &str) -> Option<BlockId> {
        self.by_key.get(key).copied()
    }

    /// Les propriétés d'un bloc. Un numéro inconnu donne celles de l'air.
    #[inline]
    pub fn def(&self, id: BlockId) -> &BlockDef {
        self.defs.get(id.0 as usize).unwrap_or(&self.defs[0])
    }

    #[inline]
    pub fn is_solid(&self, id: BlockId) -> bool {
        self.def(id).solid
    }

    #[inline]
    pub fn is_transparent(&self, id: BlockId) -> bool {
        self.def(id).transparent
    }

    #[inline]
    pub fn is_selectable(&self, id: BlockId) -> bool {
        self.def(id).selectable
    }

    /// Dessiné dans la passe translucide (alpha < 1).
    #[inline]
    pub fn is_translucent(&self, id: BlockId) -> bool {
        self.def(id).alpha < 1.0
    }

    /// Un cube (plein ou transparent), par opposition aux plantes.
    #[inline]
    pub fn is_cube(&self, id: BlockId) -> bool {
        self.def(id).shape == Shape::Cube
    }

    #[inline]
    pub fn light_emission(&self, id: BlockId) -> u8 {
        self.def(id).light_emission
    }

    #[inline]
    pub fn light_opacity(&self, id: BlockId) -> u8 {
        self.def(id).light_opacity
    }

    /// Les blocs de la barre de blocs, dans l'ordre du registre.
    pub fn placeable(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.iter()
            .filter(|(_, def)| def.placeable)
            .map(|(id, _)| id)
    }

    /// Tous les blocs, air compris, avec leur numéro.
    pub fn iter(&self) -> impl Iterator<Item = (BlockId, &BlockDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, def)| (BlockId(i as u16), def))
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// `stone` dans le fichier de `core` devient `core:stone` ; `other:stone` reste tel quel
/// (un mod peut réutiliser la texture d'un autre).
fn qualify(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

/// Un identifiant valide ressemble à `espace:nom`, en minuscules, chiffres et `_`,
/// et son espace de noms est celui du fichier qui le définit.
fn validate_key(key: &str, namespace: &str) -> Result<()> {
    let Some((ns, name)) = key.split_once(':') else {
        bail!("« {key} » : il manque l'espace de noms (attendu « {namespace}:nom »)");
    };
    if ns != namespace {
        bail!("« {key} » : ce fichier ne peut définir que des blocs « {namespace}:... »");
    }
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    if !valid(name) {
        bail!("« {key} » : le nom ne doit contenir que a-z, 0-9 et _");
    }
    Ok(())
}

/// Des raccourcis pour les tests de la crate : le numéro des blocs de base.
#[cfg(test)]
pub(crate) mod test_blocks {
    use super::*;

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }
    pub fn grass() -> BlockId {
        id("core:grass")
    }
    pub fn dirt() -> BlockId {
        id("core:dirt")
    }
    pub fn stone() -> BlockId {
        id("core:stone")
    }
    pub fn sand() -> BlockId {
        id("core:sand")
    }
    pub fn snow() -> BlockId {
        id("core:snow")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// La racine du dépôt, où sont `assets/` et `mods/`.
    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Un dossier temporaire neuf, propre à ce test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voxel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: PathBuf, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn builtin_has_the_core_blocks() {
        let registry = BlockRegistry::builtin();
        assert_eq!(registry.id("core:air"), Some(BlockId::AIR));
        let stone = registry.id("core:stone").unwrap();
        assert_eq!(registry.def(stone).name, "Pierre");
        assert!(registry.is_solid(stone) && !registry.is_transparent(stone));
        assert!(!registry.is_solid(BlockId::AIR) && registry.is_transparent(BlockId::AIR));
        assert_eq!(
            registry.placeable().count(),
            63,
            "sans l'air, l'eau, la lave, le socle, l'inconnu et la lampe allumée"
        );
        let water = registry.id("core:water").unwrap();
        assert!(registry.def(water).liquid && registry.is_translucent(water));
        assert!(!registry.is_solid(water));
        let torch = registry.id("core:torch").unwrap();
        assert_eq!(registry.light_emission(torch), 14);
        assert_eq!(registry.light_opacity(stone), 15);
        assert_eq!(
            registry.light_opacity(registry.id("core:glass").unwrap()),
            0
        );
    }

    #[test]
    fn loads_core_then_mods() {
        let registry =
            BlockRegistry::load(&repo().join("assets"), Some(&repo().join("mods"))).unwrap();
        let ruby = registry
            .id("example:ruby")
            .expect("le bloc du mod est chargé");
        // Core d'abord : le rubis vient après les blocs de base (et l'air).
        let core_blocks = BlockRegistry::builtin().len();
        assert_eq!(ruby, BlockId(core_blocks as u16));
        assert!(registry.def(ruby).placeable);
        assert!(registry.placeable().any(|id| id == ruby));
    }

    #[test]
    fn same_files_give_the_same_ids() {
        let load =
            || BlockRegistry::load(&repo().join("assets"), Some(&repo().join("mods"))).unwrap();
        let (a, b) = (load(), load());
        let keys = |r: &BlockRegistry| {
            r.iter()
                .map(|(id, d)| (id, d.key.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&a), keys(&b));
    }

    #[test]
    fn mods_load_in_alphabetical_order() {
        let root = temp_dir("order");
        write(root.join("assets/core/blocks.ron"), "[]");
        write(
            root.join("mods/zeta/blocks.ron"),
            r#"[(id: "zeta:z", name: "Z", color: (1.0, 1.0, 1.0))]"#,
        );
        write(
            root.join("mods/alpha/blocks.ron"),
            r#"[(id: "alpha:a", name: "A", color: (1.0, 1.0, 1.0))]"#,
        );
        let registry = BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap();
        assert_eq!(registry.id("alpha:a"), Some(BlockId(1)));
        assert_eq!(registry.id("zeta:z"), Some(BlockId(2)));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut registry = BlockRegistry::builtin().as_ref().clone();
        let err = registry
            .add_source(
                CORE_NAMESPACE,
                r#"[(id: "core:stone", name: "Autre", color: (0.0, 0.0, 0.0))]"#,
                "doublon.ron",
            )
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains("core:stone") && message.contains("doublon.ron"),
            "{message}"
        );
    }

    #[test]
    fn wrong_namespace_and_bad_names_are_refused() {
        let mut registry = BlockRegistry::empty();
        let entry = |key: &str| format!(r#"[(id: "{key}", name: "X", color: (0.0, 0.0, 0.0))]"#);
        assert!(
            registry
                .add_source("mymod", &entry("core:hack"), "m")
                .is_err()
        );
        assert!(registry.add_source("mymod", &entry("ruby"), "m").is_err());
        assert!(
            registry
                .add_source("mymod", &entry("mymod:Ruby"), "m")
                .is_err()
        );
        assert!(
            registry
                .add_source("mymod", &entry("mymod:ruby_2"), "m")
                .is_ok()
        );
    }

    #[test]
    fn textures_per_face_and_for_all_faces() {
        let mut registry = BlockRegistry::empty();
        let source = r#"[
            (id: "test:grass", name: "G", color: (0.0, 1.0, 0.0),
             textures: (top: "grass_top", side: "grass_side", bottom: "dirt"),
             sounds: (step: "step_soft", break: "core:break_soft")),
            (id: "test:dirt", name: "D", color: (0.5, 0.3, 0.1), texture: "dirt"),
            (id: "test:plain", name: "P", color: (1.0, 1.0, 1.0)),
        ]"#;
        registry.add_source("test", source, "t").unwrap();
        let grass = registry.def(registry.id("test:grass").unwrap());
        let dirt = registry.def(registry.id("test:dirt").unwrap());
        let plain = registry.def(registry.id("test:plain").unwrap());
        assert_eq!(
            registry.textures(),
            ["test:grass_top", "test:grass_side", "test:dirt"]
        );
        assert_eq!(grass.face_textures, [1, 2, 3]);
        // La même texture garde la même couche.
        assert_eq!(dirt.face_textures, [3, 3, 3]);
        assert_eq!(plain.face_textures, [0, 0, 0]);
        assert_eq!(grass.sounds.step.as_deref(), Some("test:step_soft"));
        assert_eq!(grass.sounds.break_.as_deref(), Some("core:break_soft"));
        assert_eq!(grass.sounds.place, None);
    }

    #[test]
    fn texture_and_textures_together_are_refused() {
        let mut registry = BlockRegistry::empty();
        let source = r#"[(id: "test:x", name: "X", color: (0.0, 0.0, 0.0), texture: "a",
            textures: (top: "a", side: "a", bottom: "a"))]"#;
        assert!(registry.add_source("test", source, "t").is_err());
    }

    #[test]
    fn asset_paths_follow_the_namespace() {
        let registry =
            BlockRegistry::load(&repo().join("assets"), Some(&repo().join("mods"))).unwrap();
        let path = registry
            .asset_path("example:ruby", "textures", "png")
            .unwrap();
        assert!(path.ends_with("mods/example/textures/ruby.png"));
        assert!(path.exists(), "{}", path.display());
        let sound = registry
            .asset_path("core:step_soft", "sounds", "wav")
            .unwrap();
        assert!(sound.exists(), "{}", sound.display());
        // Chaque texture déclarée par core et le mod existe sur le disque.
        for key in registry.textures() {
            let path = registry.asset_path(key, "textures", "png").unwrap();
            assert!(path.exists(), "texture manquante : {}", path.display());
        }
    }

    #[test]
    fn mod_named_core_is_refused() {
        let root = temp_dir("core-mod");
        write(root.join("assets/core/blocks.ron"), "[]");
        write(root.join("mods/core/blocks.ron"), "[]");
        assert!(BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).is_err());
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut registry = BlockRegistry::empty();
        let source = r#"[(id: "core:x", name: "X", color: (0.0, 0.0, 0.0), solide: false)]"#;
        assert!(
            registry
                .add_source(CORE_NAMESPACE, source, "faute.ron")
                .is_err()
        );
    }
}
