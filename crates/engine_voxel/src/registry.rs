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

use anyhow::{Context as _, Result, bail, ensure};
use glam::{IVec3, Vec3};
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
    // Façade au nord et dos au sud dans le repère du modèle.
    end_textures: [Option<u32>; 2],
    pub lid: Option<ChestLid>,
    /// Couche du masque alpha de teinte des côtés d’un cube herbeux (0 = aucun masque).
    pub side_tint: u16,
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
    /// Rotation visuelle des boîtes du modèle, autour du centre du bloc.
    pub rotation: Option<BlockRotation>,
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
    /// Les boîtes de la forme `Shape::Model` (en blocs, de 0 à 1) : la forme dessinée, la
    /// collision et la sélection. Vide pour un cube plein. Avec `connections`, ce sont
    /// les boîtes du segment d'inventaire ; `World::block_boxes` donne la forme en place.
    pub boxes: Vec<BlockBox>,
    /// Indice de la boîte vitrée, incluse dans la collision du bloc.
    pub glazing: Option<usize>,
    /// Géométries calculées depuis les voisins ; aucune variante enregistrée.
    pub connections: Option<Arc<crate::connections::ConnectedShape>>,
    /// Les états du bloc (orientation, moitié...), dans l'ordre déclaré. Chaque combinaison
    /// est un bloc du registre : voir [`BlockRegistry::variant`]. Vide si le bloc n'en a pas.
    pub states: Vec<StateKind>,
    /// Ce bloc est un conduit d'un réseau (câble, tuyau, arbre) : il relie ses voisins de la
    /// même sorte, et le jeu répartit énergie, fluide ou rotation entre les machines branchées.
    pub conduit: Option<crate::networks::NetworkKind>,
    /// Le canal d'un conduit (`mon_mod:vapeur`) : deux conduits de même sorte ne se relient que
    /// s'ils ont le même canal. Sans canal, tous les conduits de la sorte se relient.
    pub channel: Option<String>,
    /// Un portail : le joueur qui s'y tient quelques secondes passe dans cette dimension
    /// (`mon_mod:ma_dimension`, ou `core:overworld` pour revenir).
    pub portal: Option<String>,
    /// Ce bloc est un rail (voir `rails.rs`) : sa forme est calculée depuis les rails voisins.
    /// Son dessin est un panneau posé au sol (`Shape::Rail`), orienté selon cette forme.
    pub rail: Option<crate::rails::RailKind>,
    /// Le débit d'un conduit : au plus ce nombre d'unités par passage (le plus faible du réseau
    /// l'emporte). Sans valeur : le débit par défaut de la sorte.
    pub throughput: Option<u32>,
    /// Les cases d'inventaire que les réseaux d'objets remplissent (`inputs`) et vident
    /// (`outputs`) : sans cela, toutes les cases font les deux.
    pub ports: Option<InventoryPorts>,
    /// Ce que ce bloc apporte à une table d'enchantement proche (étagère) ou à une bibliothèque
    /// d'enchantements (voir [`EnchantingStats`]).
    pub enchanting: Option<EnchantingStats>,
    /// Un conduit **à sens unique** (`flow: true`, avec `conduit: Items`, `Energy` ou `Fluid` et un
    /// état `Facing4` ou `Facing6`) : le contenu ne va que vers l'« avant » de la variante (le nord de la forme
    /// écrite). Posé, il pointe là où regarde le joueur. Il accepte par l'arrière et les côtés, et
    /// ne rend que par l'avant.
    pub directed: bool,
    /// Pour un conduit à sens unique : la direction où vont les objets, selon la variante.
    pub flow: Option<IVec3>,
    /// Pour une variante : le bloc de base (celui qu'on pose depuis la barre, et qui lâche
    /// son objet). `None` pour un bloc ordinaire ou la base elle-même.
    pub base: Option<BlockId>,
    /// Pour la base : toutes ses variantes dans l'ordre des combinaisons (elle-même en tête).
    pub variants: Vec<BlockId>,
    /// La valeur de chaque état de cette variante (un indice dans `StateKind::values`).
    pub state_values: Vec<u8>,
    /// D'où vient la définition, pour les messages d'erreur.
    pub source: String,
}

/// Ce qu'un bloc apporte aux tables d'enchantement (champ `enchanting` de `blocks.ron`). Le jeu lit
/// les 32 positions autour d'une table (deux anneaux, de l'air entre la table et le bloc) et
/// additionne ces valeurs : un mod ajoute ses étagères sans toucher au jeu.
///
/// Les nombres sont tous facultatifs (0 par défaut). `eterna`, `quanta`, `arcana`, `clues` et
/// `stability` s'**ajoutent** d'un bloc à l'autre ; `max_eterna` est un **plafond** : l'Eterna de la
/// table ne dépasse jamais le plus haut plafond des blocs présents (les étagères de bois plafonnent
/// bas, les plus rares permettent d'aller jusqu'à 100).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EnchantingStats {
    /// La puissance : le niveau maximal que la table propose.
    pub eterna: f32,
    /// Le plafond d'Eterna que ce bloc permet d'atteindre.
    pub max_eterna: f32,
    /// La part de hasard dans les niveaux proposés, en pourcents (0 à 100).
    pub quanta: f32,
    /// Le nombre et la rareté des enchantements, en pourcents (0 à 100).
    pub arcana: f32,
    /// Les enchantements montrés à l'avance (on compte la partie entière du total, plus un).
    pub clues: f32,
    /// La part du hasard qui ne fait que du bien, en pourcents (0 à 100).
    pub stability: f32,
    /// Ouvre les enchantements rares (« trésor ») à la table.
    pub treasure: bool,
    /// Une bibliothèque d'enchantements : le plus haut niveau qu'elle sait ranger (0 : ce n'est pas
    /// une bibliothèque). Elle stocke `2^(niveau - 1)` points par niveau.
    pub library_level: u8,
}

/// Quelles cases de l'inventaire d'un bloc un réseau d'objets (`conduit: Items`) peut remplir
/// et vider : une machine reçoit dans ses entrées et rend par ses sorties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryPorts {
    /// Les cases qui reçoivent des objets du réseau.
    #[serde(default)]
    pub inputs: Vec<usize>,
    /// Les cases d'où le réseau retire des objets.
    #[serde(default)]
    pub outputs: Vec<usize>,
    /// Un type de recette (`mon_mod:presse`) : seuls les objets qu'une de ses recettes consomme
    /// entrent par les cases d'entrée. Sans cela, tout entre.
    #[serde(default)]
    pub accepts: Option<String>,
    /// Une case-modèle de l'inventaire : l'objet qu'elle contient est le seul à entrer par les
    /// cases d'entrée (un trieur). Vide : tout entre. Elle n'est jamais vidée par le réseau.
    #[serde(default)]
    pub filter: Option<usize>,
}

/// Une boîte alignée sur les axes, en coordonnées de bloc (0 à 1 sur chaque axe).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl BlockDef {
    pub fn facing_turns(&self) -> u8 {
        self.states
            .iter()
            .zip(&self.state_values)
            .find_map(|(state, &value)| (*state == StateKind::Facing4).then_some(value))
            .unwrap_or(0)
    }

    pub fn has_end_textures(&self) -> bool {
        self.end_textures.iter().any(Option::is_some)
    }

    pub fn face_texture(&self, axis: usize, positive: bool) -> u32 {
        if axis == 1 {
            return self.face_textures[if positive { 0 } else { 2 }];
        }
        let side = self.face_textures[1];
        if !self.has_end_textures() {
            return side;
        }
        let direction = match (axis, positive) {
            (2, false) => 0,
            (0, true) => 1,
            (2, true) => 2,
            _ => 3,
        };
        match (direction + 4 - self.facing_turns()) % 4 {
            0 => self.end_textures[0].unwrap_or(side),
            2 => self.end_textures[1].unwrap_or(side),
            _ => side,
        }
    }
}

/// Axe dans le repère du bloc, avant ses états d'orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum RotationAxis {
    X,
    Y,
    Z,
}

impl RotationAxis {
    pub fn vector(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }
}

/// Une animation décorative, calculée par le GPU sans remailler à chaque image.
/// Les collisions et la sélection conservent les boîtes déclarées.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockRotation {
    pub axis: RotationAxis,
    /// Radians par seconde, arrondis au huitième, de -15,875 à 15,875.
    pub speed: f32,
    /// Indices des boîtes animées ; vide : toutes les boîtes.
    #[serde(default)]
    pub boxes: Vec<usize>,
}

impl BlockBox {
    pub const FULL: BlockBox = BlockBox {
        min: Vec3::ZERO,
        max: Vec3::ONE,
    };

    /// La même boîte après avoir déplacé ses deux coins avec `f` (en repère centré sur le bloc).
    fn mapped(self, f: impl Fn(Vec3) -> Vec3) -> Self {
        let center = Vec3::splat(0.5);
        let a = f(self.min - center) + center;
        let b = f(self.max - center) + center;
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }
}

/// Un état de bloc : une façon dont un même bloc peut varier. Chaque état multiplie le
/// nombre de variantes (le registre en fabrique une par combinaison). La forme écrite dans
/// le fichier est celle de la **première valeur** (face avant au nord, moitié basse, axe Y) ;
/// les autres valeurs en sont déduites par rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum StateKind {
    /// Face avant vers le nord (-Z), l'est (+X), le sud (+Z) ou l'ouest (-X). Posé, l'avant
    /// regarde le joueur.
    Facing4,
    /// Comme `Facing4`, plus le haut et le bas.
    Facing6,
    /// Moitié basse ou haute du bloc : dalles, escaliers. Posé sur le dessous d'un bloc, ou
    /// sur la moitié haute d'une face, le bloc est en haut.
    Half,
    /// Le grand axe : Y (debout), X ou Z (couché). Posé, il suit la face visée.
    Axis,
    /// Pivote autour du montant nord-ouest. Les boîtes fermées ont une profondeur de 3/16.
    DoorOpen,
    /// Un coffre seul, ou une moitié de grand coffre (voir `double_chest.rs`) : la moitié
    /// gauche (vue de face, côté +X du modèle) ou droite. Les boîtes d'une moitié s'étirent
    /// jusqu'à la jointure ; la serrure se partage entre les deux. Toujours après `Facing4`.
    ChestPart,
}

/// Le sens d'un conduit à sens unique : la direction de son état d'orientation (`north` : -Z,
/// `east` : +X, `south` : +Z, `west` : -X, `up`, `down`).
fn flow_of(states: &[StateKind], values: &[u8], directed: bool) -> Option<IVec3> {
    if !directed {
        return None;
    }
    let (_, &value) = states
        .iter()
        .zip(values)
        .find(|(s, _)| matches!(s, StateKind::Facing4 | StateKind::Facing6))?;
    Some(match value {
        0 => IVec3::NEG_Z,
        1 => IVec3::X,
        2 => IVec3::Z,
        3 => IVec3::NEG_X,
        4 => IVec3::Y,
        _ => IVec3::NEG_Y,
    })
}

impl StateKind {
    /// Le nom de l'état dans l'identifiant (`facing=east`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Facing4 | Self::Facing6 => "facing",
            Self::Half => "half",
            Self::Axis => "axis",
            Self::DoorOpen => "open",
            Self::ChestPart => "part",
        }
    }

    pub fn values(self) -> &'static [&'static str] {
        match self {
            Self::Facing4 => &["north", "east", "south", "west"],
            Self::Facing6 => &["north", "east", "south", "west", "up", "down"],
            Self::Half => &["bottom", "top"],
            Self::Axis => &["y", "x", "z"],
            Self::DoorOpen => &["false", "true"],
            Self::ChestPart => &["single", "left", "right"],
        }
    }

    /// Où va le point `c` (repère centré sur le bloc) quand l'état vaut `value`.
    fn apply(self, value: u8, c: Vec3) -> Vec3 {
        match (self, value) {
            (Self::Facing4 | Self::Facing6, 1) => Vec3::new(-c.z, c.y, c.x),
            (Self::Facing4 | Self::Facing6, 2) => Vec3::new(-c.x, c.y, -c.z),
            (Self::Facing4 | Self::Facing6, 3) => Vec3::new(c.z, c.y, -c.x),
            (Self::Facing6, 4) => Vec3::new(c.x, -c.z, c.y),
            (Self::Facing6, 5) => Vec3::new(c.x, c.z, -c.y),
            (Self::Half, 1) => Vec3::new(c.x, -c.y, c.z),
            (Self::Axis, 1) => Vec3::new(c.y, c.x, c.z),
            (Self::Axis, 2) => Vec3::new(c.x, c.z, c.y),
            (Self::DoorOpen, 1) => Vec3::new(-c.z - 13.0 / 16.0, c.y, c.x),
            _ => c,
        }
    }
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
    /// Une ou plusieurs boîtes (`boxes` dans le fichier) : dalles, escaliers, barrières,
    /// engrenages, tout ce qui n'est pas un cube plein. Sert aussi à la collision et à la
    /// sélection.
    Model,
    /// Un rail : un panneau texturé au ras du sol, tourné et incliné selon sa forme (rangée dans
    /// l'état du bloc, voir `rails.rs`). `textures` : dessus = rail droit, côtés = virage. La
    /// sélection se fait sur une dalle très fine ; le joueur le traverse.
    Rail,
}

/// La teinte qui dépend du biome (multipliée à la couleur de la texture).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Tint {
    #[default]
    None,
    /// La couleur de l'herbe du biome (le dessus, les côtés masqués, les herbes hautes).
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
    /// De la terre labourée dessous (les cultures).
    Farmland,
}

/// Les textures d'un bloc dans un fichier : une par face (dessus, côtés, dessous).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaceTextures {
    top: String,
    side: String,
    bottom: String,
    #[serde(default)]
    front: Option<String>,
    #[serde(default)]
    back: Option<String>,
    #[serde(default)]
    side_tint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChestLid {
    pub body: usize,
    pub boxes: Vec<usize>,
    /// Hauteur et profondeur de la charnière au nord, en seizièmes.
    pub hinge: (u8, u8),
    pub inside: u32,
    paired_textures: Option<[PairedChestLayers; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PairedChestLayers {
    top: u32,
    front: u32,
    back: u32,
    inside: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LidEntry {
    body: usize,
    boxes: Vec<usize>,
    hinge: (u8, u8),
    inside: String,
    #[serde(default)]
    paired_textures: Option<PairedChestEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairedChestEntry {
    left: ChestHalfEntry,
    right: ChestHalfEntry,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChestHalfEntry {
    top: String,
    front: String,
    back: String,
    inside: String,
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
    lid: Option<LidEntry>,
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
    #[serde(default)]
    rotation: Option<BlockRotation>,
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
    /// Les boîtes de la forme, en seizièmes de bloc : `(from: (0, 0, 0), to: (16, 8, 16))`.
    #[serde(default)]
    boxes: Vec<BoxEntry>,
    #[serde(default)]
    glazing: Option<usize>,
    #[serde(default)]
    connections: Option<crate::connections::ConnectionSize>,
    /// Les états : une variante du bloc par combinaison (voir [`StateKind`]).
    #[serde(default)]
    states: Vec<StateKind>,
    /// `conduit: Kinetic` (ou `Energy`, `Fluid`) : le bloc est un conduit de réseau.
    #[serde(default)]
    conduit: Option<crate::networks::NetworkKind>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    portal: Option<String>,
    /// `rail: Normal` (ou `Booster`, `Detector`, `Activator`, `Stop`) : le bloc est un rail.
    #[serde(default)]
    rail: Option<crate::rails::RailKind>,
    #[serde(default)]
    throughput: Option<u32>,
    #[serde(default)]
    ports: Option<InventoryPorts>,
    /// Les stats d'enchantement du bloc (étagère, bibliothèque).
    #[serde(default)]
    enchanting: Option<EnchantingStats>,
    /// Un conduit d'objets à sens unique (voir [`BlockDef::flow`]).
    #[serde(default)]
    flow: bool,
}

/// Une boîte dans un fichier, en seizièmes de bloc (0 à 16).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoxEntry {
    from: (f32, f32, f32),
    to: (f32, f32, f32),
}

/// Au plus ce nombre de variantes par bloc (6 orientations × 2 moitiés = 12 d'ordinaire).
const MAX_VARIANTS: usize = 64;

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
    /// Remplacements graphiques uniquement ; les données du registre ne changent pas.
    visual_pack: Option<PathBuf>,
    texture_size: u32,
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
            end_textures: [None; 2],
            lid: None,
            side_tint: 0,
            sounds: BlockSounds::default(),
            behavior: None,
            fluid: None,
            unbreakable: false,
            shape: Shape::Cube,
            cutout: false,
            tint: Tint::None,
            sway: false,
            rotation: None,
            hardness: 0.0,
            tool: None,
            tier: 0,
            support: Support::None,
            gravity: false,
            xp: (0, 0),
            boxes: Vec::new(),
            glazing: None,
            connections: None,
            states: Vec::new(),
            conduit: None,
            channel: None,
            portal: None,
            rail: None,
            throughput: None,
            ports: None,
            enchanting: None,
            directed: false,
            flow: None,
            base: None,
            variants: Vec::new(),
            state_values: Vec::new(),
            source: "intégré".into(),
        };
        Self {
            by_key: HashMap::from([(air.key.clone(), BlockId::AIR)]),
            defs: vec![air],
            textures: Vec::new(),
            asset_dirs: HashMap::new(),
            visual_pack: None,
            texture_size: 16,
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
        let mut directional = Vec::new();
        registry.add_file(
            CORE_NAMESPACE,
            &assets_dir.join("core").join("blocks.ron"),
            &mut directional,
        )?;
        for (namespace, dir) in mods {
            if namespace == CORE_NAMESPACE {
                bail!(
                    "le mod {} ne peut pas s'appeler « core », ce nom est réservé",
                    dir.display()
                );
            }
            let file = dir.join("blocks.ron");
            if file.exists() {
                registry.add_file(namespace, &file, &mut directional)?;
            }
            // Sinon : un mod sans nouveaux blocs.
        }
        for id in directional {
            registry.add_variants(id)?;
        }
        Ok(registry)
    }

    fn add_file(
        &mut self,
        namespace: &str,
        path: &Path,
        directional: &mut Vec<BlockId>,
    ) -> Result<()> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("lecture de {}", path.display()))?;
        // Le dossier qui contient `blocks.ron` contient aussi `textures/` et `sounds/`.
        if let Some(dir) = path.parent() {
            self.asset_dirs
                .insert(namespace.to_string(), dir.to_path_buf());
        }
        self.read_source(namespace, &source, &path.display().to_string(), directional)
    }

    /// Ajoute les blocs décrits par `source` (texte RON). `origin` sert aux messages d'erreur.
    pub fn add_source(&mut self, namespace: &str, source: &str, origin: &str) -> Result<()> {
        let mut directional = Vec::new();
        self.read_source(namespace, source, origin, &mut directional)?;
        for id in directional {
            self.add_variants(id)?;
        }
        Ok(())
    }

    fn read_source(
        &mut self,
        namespace: &str,
        source: &str,
        origin: &str,
        directional: &mut Vec<BlockId>,
    ) -> Result<()> {
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
            let ends = entry.textures.as_ref().map_or([None; 2], |faces| {
                [faces.front.as_deref(), faces.back.as_deref()]
            });
            let oriented = ends.iter().any(Option::is_some);
            let end_textures =
                ends.map(|name| name.map(|name| self.texture_layer(&qualify(name, namespace))));
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
            if entry.conduit.is_some() && entry.behavior.is_some() {
                bail!(
                    "{origin} : « {} » : un conduit n'a pas de comportement (une machine branchée au réseau est une extrémité, pas un conduit)",
                    entry.id
                );
            }
            if (entry.channel.is_some() || entry.throughput.is_some()) && entry.conduit.is_none() {
                bail!(
                    "{origin} : « {} » : `channel` et `throughput` ne vont qu'avec `conduit`",
                    entry.id
                );
            }
            if entry.flow {
                if !matches!(
                    entry.conduit,
                    Some(
                        crate::networks::NetworkKind::Items
                            | crate::networks::NetworkKind::Energy
                            | crate::networks::NetworkKind::Fluid
                    )
                ) {
                    bail!(
                        "{origin} : « {} » : `flow` (sens unique) va avec `conduit: Items`, `Energy` ou `Fluid` (pas `Kinetic`)",
                        entry.id
                    );
                }
                if !entry
                    .states
                    .iter()
                    .any(|s| matches!(s, StateKind::Facing4 | StateKind::Facing6))
                {
                    bail!(
                        "{origin} : « {} » : `flow` exige un état `Facing4` ou `Facing6` (le sens)",
                        entry.id
                    );
                }
            }
            if entry.throughput == Some(0) {
                bail!(
                    "{origin} : « {} » : `throughput` doit être d'au moins 1",
                    entry.id
                );
            }
            let connections = entry.connections.as_ref().map(|size| {
                anyhow::ensure!(entry.boxes.is_empty() && entry.states.is_empty() && entry.rotation.is_none(),
                    "{origin} : connections remplace boxes et ne se combine pas avec states ou rotation");
                anyhow::ensure!(entry.conduit.or_else(||entry.behavior.as_deref().and_then(crate::NetworkKind::of_behavior)).is_some(),
                    "{origin} : connections exige un conduit de réseau");
                crate::connections::ConnectedShape::new(size).map(Arc::new)
            }).transpose().with_context(||format!("{origin} : {}",entry.id))?;
            // L'objet et l'icône montrent un segment droit ; dans le monde, les voisins décident.
            let boxes = if entry.rail.is_some() {
                anyhow::ensure!(
                    entry.boxes.is_empty()
                        && entry.states.is_empty()
                        && entry.connections.is_none(),
                    "{origin} : « {} » : un rail n'a ni boîtes, ni états, ni connexions",
                    entry.id
                );
                // La sélection : une dalle très fine au ras du sol.
                vec![BlockBox {
                    min: Vec3::ZERO,
                    max: Vec3::new(1.0, 0.125, 1.0),
                }]
            } else if let Some(shape) = &connections {
                shape.boxes(3).to_vec()
            } else {
                parse_boxes(&entry, origin)?
            };
            ensure!(
                entry.glazing.is_none_or(|i| i < boxes.len()
                    && connections.is_none()
                    && entry.rail.is_none()),
                "{origin} : « {} » : glazing doit désigner une boîte du modèle",
                entry.id,
            );
            if entry.states.contains(&StateKind::DoorOpen) {
                ensure!(
                    entry.states == [StateKind::DoorOpen, StateKind::Facing4]
                        && entry.rotation.is_none()
                        && boxes.iter().all(|b| b.max.z <= 3.0 / 16.0),
                    "{origin} : « {} » : DoorOpen exige [DoorOpen, Facing4] et des boîtes sur les 3 premiers seizièmes de Z",
                    entry.id,
                );
            }
            ensure!(
                !entry.states.contains(&StateKind::ChestPart) || entry.lid.is_some(),
                "{origin} : « {} » : ChestPart exige un couvercle (lid) et les états [Facing4, ChestPart]",
                entry.id,
            );
            if let Some(rotation) = &entry.rotation
                && (boxes.is_empty()
                    || !rotation.speed.is_finite()
                    || rotation.speed.abs() > 15.875
                    || rotation.boxes.iter().any(|&i| i >= boxes.len())
                    || entry.sway)
            {
                bail!(
                    "{origin} : « {} » : rotation exige un modèle, une vitesse finie entre -15.875 et 15.875, des indices de boîtes valides et aucun sway",
                    entry.id
                );
            }
            let model = !boxes.is_empty();
            let lid = if let Some(lid) = entry.lid {
                ensure!(
                    lid.body < boxes.len()
                        && !lid.boxes.is_empty()
                        && lid.boxes.iter().all(|&i| i < boxes.len() && i != lid.body)
                        && lid.hinge.0 <= 16
                        && lid.hinge.1 <= 16
                        && entry.rotation.is_none()
                        && entry.glazing.is_none()
                        && (entry.states.is_empty()
                            || entry.states == [StateKind::Facing4]
                            || entry.states == [StateKind::Facing4, StateKind::ChestPart]),
                    "{origin} : « {} » : corps, pièces ou charnière du couvercle invalides",
                    entry.id
                );
                ensure!(
                    lid.paired_textures.is_none() || entry.states.contains(&StateKind::ChestPart),
                    "{origin} : « {} » : `paired_textures` exige un état `ChestPart`",
                    entry.id
                );
                let inside = self.texture_layer(&qualify(&lid.inside, namespace));
                let paired_textures = lid.paired_textures.map(|pair| {
                    [pair.left, pair.right].map(|half| PairedChestLayers {
                        top: self.texture_layer(&qualify(&half.top, namespace)),
                        front: self.texture_layer(&qualify(&half.front, namespace)),
                        back: self.texture_layer(&qualify(&half.back, namespace)),
                        inside: self.texture_layer(&qualify(&half.inside, namespace)),
                    })
                });
                Some(ChestLid {
                    body: lid.body,
                    boxes: lid.boxes,
                    hinge: lid.hinge,
                    inside,
                    paired_textures,
                })
            } else {
                None
            };
            let side_tint =
                if let Some(mask) = entry.textures.as_ref().and_then(|f| f.side_tint.as_deref()) {
                    ensure!(
                        entry.tint == Tint::Grass
                            && entry.shape == Shape::Cube
                            && !model
                            && entry.rail.is_none()
                            && !entry.liquid,
                        "{origin} : « {} » : `side_tint` exige un cube avec `tint: Grass`",
                        entry.id
                    );
                    u16::try_from(self.texture_layer(&qualify(mask, namespace)))
                        .context("trop de couches de textures")?
                } else {
                    0
                };
            // Par défaut, un bloc transparent laisse passer la lumière, un opaque l'arrête.
            // Une forme à boîtes se juge sur `solid` : une dalle pleine arrête la lumière.
            let light_opacity = entry.light_opacity.unwrap_or(if model {
                if entry.solid { 15 } else { 0 }
            } else if entry.transparent {
                0
            } else {
                15
            });
            let def = BlockDef {
                key: entry.id,
                name: entry.name,
                color: [r, g, b],
                // Un rail se traverse.
                solid: entry.solid && entry.rail.is_none(),
                // Une forme à boîtes ne remplit pas son cube : les voisins gardent leurs faces.
                transparent: entry.transparent || model,
                selectable: entry.selectable,
                placeable: entry.placeable,
                alpha: entry.alpha,
                liquid: entry.liquid,
                light_emission: entry.light_emission,
                light_opacity,
                face_textures,
                end_textures,
                lid,
                side_tint,
                sounds,
                behavior: entry.behavior.as_ref().map(|b| qualify(b, namespace)),
                fluid: entry.fluid.as_ref().map(|f| qualify(f, namespace)),
                unbreakable: entry.unbreakable,
                shape: if entry.rail.is_some() {
                    Shape::Rail
                } else if model {
                    Shape::Model
                } else {
                    entry.shape
                },
                cutout: entry.cutout || entry.rail.is_some(),
                tint: entry.tint,
                sway: entry.sway,
                rotation: entry.rotation.clone(),
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
                boxes: boxes.clone(),
                glazing: entry.glazing,
                connections,
                states: entry.states.clone(),
                conduit: entry.conduit,
                channel: entry.channel.as_ref().map(|c| qualify(c, namespace)),
                portal: entry.portal.as_ref().map(|p| qualify(p, namespace)),
                rail: entry.rail,
                throughput: entry.throughput,
                ports: entry.ports.clone().map(|mut p| {
                    p.accepts = p.accepts.map(|a| qualify(&a, namespace));
                    p
                }),
                enchanting: entry.enchanting.clone(),
                directed: entry.flow,
                flow: None,
                base: None,
                variants: Vec::new(),
                state_values: vec![0; entry.states.len()],
                source: origin.to_string(),
            };
            self.by_key.insert(def.key.clone(), id);
            self.defs.push(def);
            // Ces orientations sont ajoutées après les anciens blocs et variantes,
            // afin que leur arrivée ne décale aucun identifiant existant.
            if oriented
                && matches!(
                    entry.states.as_slice(),
                    [StateKind::Facing4] | [StateKind::Facing4, StateKind::ChestPart]
                )
            {
                directional.push(id);
            } else {
                self.add_variants(id)?;
            }
        }
        Ok(())
    }

    /// Crée les variantes du bloc `base` qui vient d'être ajouté : une par combinaison de ses
    /// états, à la suite. La première combinaison est la base elle-même.
    fn add_variants(&mut self, base: BlockId) -> Result<()> {
        let states = self.def(base).states.clone();
        if states.is_empty() {
            return Ok(());
        }
        let origin = self.def(base).source.clone();
        let key = self.def(base).key.clone();
        if self.def(base).boxes.is_empty() {
            bail!("{origin} : « {key} » a des états mais pas de boîtes à faire tourner");
        }
        let count: usize = states.iter().map(|s| s.values().len()).product();
        if count > MAX_VARIANTS {
            bail!("{origin} : « {key} » aurait {count} variantes ({MAX_VARIANTS} au maximum)");
        }
        let template = self.def(base).clone();
        let chest_part = states.iter().position(|s| *s == StateKind::ChestPart);
        let mut ids = vec![base];
        for index in 0..count {
            // Le premier état varie le plus vite.
            let mut rest = index;
            let values: Vec<u8> = states
                .iter()
                .map(|s| {
                    let n = s.values().len();
                    let v = rest % n;
                    rest /= n;
                    v as u8
                })
                .collect();
            let suffix = format!(
                "[{}]",
                states
                    .iter()
                    .zip(&values)
                    .map(|(s, &v)| format!("{}={}", s.name(), s.values()[v as usize]))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let full = format!("{key}{suffix}");
            // Un état ajouté en dernier garde lisibles les sauvegardes d'avant : sa première
            // valeur répond aussi au nom sans lui (`core:chest[facing=east]`).
            let id = if index == 0 {
                base
            } else {
                BlockId(self.defs.len() as u16)
            };
            for kept in (1..states.len()).rev() {
                if values[kept] != 0 {
                    break;
                }
                let short = format!(
                    "{key}[{}]",
                    states[..kept]
                        .iter()
                        .zip(&values)
                        .map(|(s, &v)| format!("{}={}", s.name(), s.values()[v as usize]))
                        .collect::<Vec<_>>()
                        .join(",")
                );
                self.by_key.entry(short).or_insert(id);
            }
            if index == 0 {
                // La base est sa propre première variante, joignable aussi par son nom complet.
                self.defs[base.0 as usize].flow = flow_of(&states, &values, template.directed);
                self.defs[base.0 as usize].state_values = values;
                self.by_key.insert(full, base);
                continue;
            }
            let id = u16::try_from(self.defs.len())
                .map(BlockId)
                .context("trop de blocs (65 535 au maximum)")?;
            let mut def = template.clone();
            def.key = full.clone();
            def.placeable = false;
            def.base = Some(base);
            def.variants = Vec::new();
            let part = chest_part.map_or(0, |i| values[i]);
            if let Some(lid) = &mut def.lid
                && let Some(halves) = lid.paired_textures.take()
                && let Some(index) = part.checked_sub(1)
            {
                let layers = halves[usize::from(index)];
                def.face_textures[0] = layers.top;
                def.end_textures = [Some(layers.front), Some(layers.back)];
                if def.face_textures[2] == lid.inside {
                    def.face_textures[2] = layers.inside;
                }
                lid.inside = layers.inside;
            }
            def.boxes = template
                .boxes
                .iter()
                // Les deux moitiés sont dessinées dans le repère du modèle avant rotation.
                .map(|&b| crate::double_chest::half_box(b, part))
                .map(|b| {
                    b.mapped(|c| {
                        states
                            .iter()
                            .zip(&values)
                            .fold(c, |c, (s, &v)| s.apply(v, c))
                    })
                })
                .collect();
            def.flow = flow_of(&states, &values, template.directed);
            def.state_values = values;
            if crate::door::is_open(&def) {
                def.light_opacity = 0;
            }
            if let Some(rotation) = &mut def.rotation {
                let axis = states.iter().zip(&def.state_values).fold(
                    rotation.axis.vector(),
                    |axis, (state, &value)| {
                        let mapped = state.apply(value, axis);
                        // Half et Axis utilisent une réflexion : une vitesse angulaire
                        // est un pseudovecteur (son sens s'inverse avec le repère).
                        if matches!(
                            (state, value),
                            (StateKind::Half, 1) | (StateKind::Axis, 1 | 2)
                        ) {
                            -mapped
                        } else {
                            mapped
                        }
                    },
                );
                let (axis, sign) = if axis.x.abs() > 0.5 {
                    (RotationAxis::X, axis.x)
                } else if axis.y.abs() > 0.5 {
                    (RotationAxis::Y, axis.y)
                } else {
                    (RotationAxis::Z, axis.z)
                };
                rotation.axis = axis;
                rotation.speed *= sign;
            }
            self.by_key.insert(full, id);
            self.defs.push(def);
            ids.push(id);
        }
        self.defs[base.0 as usize].variants = ids;
        Ok(())
    }

    /// Le bloc de base d'une variante (le bloc lui-même s'il n'en est pas une).
    #[inline]
    pub fn base(&self, id: BlockId) -> BlockId {
        self.def(id).base.unwrap_or(id)
    }

    /// La variante de `block` pour ces valeurs d'états (une par état, dans l'ordre déclaré).
    /// Un bloc sans états, ou des valeurs invalides, donnent `block` tel quel.
    pub fn variant(&self, block: BlockId, values: &[u8]) -> BlockId {
        let base = self.def(self.base(block));
        let mut index = 0;
        let mut stride = 1;
        for (state, &v) in base.states.iter().zip(values) {
            let n = state.values().len();
            if v as usize >= n {
                return block;
            }
            index += v as usize * stride;
            stride *= n;
        }
        base.variants.get(index).copied().unwrap_or(block)
    }

    /// La variante à poser quand le joueur regarde dans la direction `look` et vise la face de
    /// normale `normal`, au point de hauteur `hit_y` (0 à 1) du bloc visé.
    pub fn placement_variant(
        &self,
        block: BlockId,
        look: Vec3,
        normal: IVec3,
        hit_y: f32,
    ) -> BlockId {
        let base = self.def(self.base(block));
        if base.states.is_empty() {
            return block;
        }
        // L'avant du bloc regarde le joueur : il va dans le sens opposé au regard. Un conduit à
        // sens unique, lui, pointe là où le joueur regarde (le courant s'éloigne de lui).
        let toward = if base.directed { look } else { -look };
        let values: Vec<u8> = base
            .states
            .iter()
            .map(|state| match state {
                StateKind::Facing4 | StateKind::Facing6 => {
                    let vertical = toward.y.abs() > toward.x.abs().max(toward.z.abs());
                    if *state == StateKind::Facing6 && vertical {
                        if toward.y > 0.0 { 4 } else { 5 }
                    } else if toward.x.abs() > toward.z.abs() {
                        if toward.x > 0.0 { 1 } else { 3 }
                    } else if toward.z > 0.0 {
                        2
                    } else {
                        0
                    }
                }
                StateKind::Half => {
                    let top = match normal.y {
                        -1 => true,
                        1 => false,
                        _ => hit_y >= 0.5,
                    };
                    top as u8
                }
                StateKind::Axis => {
                    if normal.x != 0 {
                        1
                    } else if normal.z != 0 {
                        2
                    } else {
                        0
                    }
                }
                StateKind::DoorOpen | StateKind::ChestPart => 0,
            })
            .collect();
        self.variant(block, &values)
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

    /// Ajoute une surface d'entité au tableau partagé, sans créer de bloc ni modifier ses identifiants.
    pub fn register_texture(&mut self, key: &str) -> u32 {
        self.texture_layer(key)
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
        if extension == "png"
            && matches!(kind, "textures" | "items")
            && let Some(root) = &self.visual_pack
        {
            let path = root.join(namespace).join(kind).join(format!("{name}.png"));
            if path.is_file() {
                return Some(path);
            }
        }
        Some(dir.join(kind).join(format!("{name}.{extension}")))
    }

    /// Active un dossier `assets/<espace de noms>/{textures,items}` de remplacement.
    /// Les fichiers absents reviennent aux ressources du jeu ou du mod.
    pub fn set_visual_pack(&mut self, assets: Option<PathBuf>, size: u32) -> Result<()> {
        anyhow::ensure!(
            matches!(size, 16 | 32),
            "résolution de pack non prise en charge : {size}"
        );
        if let Some(root) = &assets {
            anyhow::ensure!(root.is_dir(), "pack absent : {}", root.display());
        }
        self.texture_size = if assets.is_some() { size } else { 16 };
        self.visual_pack = assets;
        Ok(())
    }

    pub fn texture_size(&self) -> u32 {
        self.texture_size
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

/// Les boîtes d'un fichier, converties de seizièmes en blocs et vérifiées.
fn parse_boxes(entry: &BlockFileEntry, origin: &str) -> Result<Vec<BlockBox>> {
    entry
        .boxes
        .iter()
        .map(|b| {
            let (from, to) = (Vec3::new(b.from.0, b.from.1, b.from.2), Vec3::new(b.to.0, b.to.1, b.to.2));
            let inside = |v: Vec3| v.min_element() >= 0.0 && v.max_element() <= 16.0;
            if !inside(from) || !inside(to) || from.cmpge(to).any() {
                bail!(
                    "{origin} : « {} » : une boîte va de 0 à 16 (seizièmes de bloc), avec `from` plus petit que `to`",
                    entry.id
                );
            }
            Ok(BlockBox {
                min: from / 16.0,
                max: to / 16.0,
            })
        })
        .collect()
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
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn oriented_rotation_matches_the_transformed_model() {
        for state in [
            StateKind::Facing4,
            StateKind::Facing6,
            StateKind::Axis,
            StateKind::Half,
        ] {
            let mut registry = BlockRegistry::empty();
            registry
                .add_source(
                    "test",
                    &format!(
                        r#"[(
                id: "test:rotor", name: "Rotor", color: (1.0, 1.0, 1.0),
                boxes: [(from: (6, 0, 6), to: (10, 16, 10))], states: [{state:?}],
                rotation: (axis: Y, speed: 1.5),
            )]"#
                    ),
                    "test",
                )
                .unwrap();
            let rotor = registry.id("test:rotor").unwrap();
            for value in 0..state.values().len() {
                let variant = registry.variant(rotor, &[value as u8]);
                let rotation = registry.def(variant).rotation.as_ref().unwrap();
                let point = Vec3::new(0.2, 0.3, 0.4);
                let expected = state.apply(value as u8, glam::Quat::from_rotation_y(0.75) * point);
                let actual =
                    glam::Quat::from_axis_angle(rotation.axis.vector(), rotation.speed * 0.5)
                        * state.apply(value as u8, point);
                assert!(actual.distance(expected) < 1e-5, "{state:?} / {value}");
            }
        }
    }

    #[test]
    fn invalid_rotations_are_rejected() {
        for fields in [
            "rotation: (axis: Y, speed: 1.0)",
            "boxes: [(from: (0, 0, 0), to: (16, 16, 16))], rotation: (axis: Y, speed: 16.0)",
            "boxes: [(from: (0, 0, 0), to: (16, 16, 16))], rotation: (axis: X, speed: 1.0, boxes: [1])",
            "boxes: [(from: (0, 0, 0), to: (16, 16, 16))], sway: true, rotation: (axis: Z, speed: 1.0)",
        ] {
            let mut registry = BlockRegistry::empty();
            assert!(
                registry
                    .add_source(
                        "test",
                        &format!(
                            r#"[(id: "test:bad", name: "Bad", color: (1.0, 1.0, 1.0), {fields})]"#
                        ),
                        "test"
                    )
                    .is_err()
            );
        }
    }

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
            155,
            "quatre portes constructibles, les moitiés hautes et états ouverts restent internes ; huit minerais v3"
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
            .id("example:ruby_slab")
            .expect("le bloc du mod est chargé");
        // Les variantes directionnelles ajoutées aux machines passent après les mods.
        // Les blocs et variantes déjà enregistrés conservent ainsi leur numéro.
        let builtin = BlockRegistry::builtin();
        let first_new = builtin.id("core:workbench[facing=east]").unwrap();
        assert!(ruby.0 >= first_new.0);
        for (id, def) in builtin.iter().take(first_new.0 as usize) {
            assert_eq!(registry.id(&def.key), Some(id));
        }
        assert!(registry.id("core:workbench[facing=east]").unwrap().0 > ruby.0);
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
    fn new_machine_facings_leave_existing_core_and_mod_ids_in_place() {
        let root = temp_dir("directional-machines");
        write(
            root.join("mods/check/blocks.ron"),
            r#"[(id: "check:marker", name: "M", color: (0.0, 0.0, 0.0))]"#,
        );
        let core = root.join("assets/core/blocks.ron");
        let front = r#"(id: "core:machine", name: "M", color: (0.0, 0.0, 0.0),
            boxes: [(from: (1, 1, 1), to: (15, 12, 15))],
            textures: (top: "top", side: "side", bottom: "base", front: "front", back: "back")"#;
        let rotator = r#"(id: "core:slab", name: "S", color: (0.0, 0.0, 0.0),
            boxes: [(from: (0, 0, 0), to: (16, 8, 16))], states: [Half])"#;
        let load = |states| {
            write(core.clone(), &format!("[{front}{states}), {rotator}]"));
            BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap()
        };
        let old = load("");
        let new = load(", states: [Facing4]");
        for (id, def) in old.iter() {
            assert_eq!(new.id(&def.key), Some(id));
        }
        let base = new.id("core:machine").unwrap();
        let east = new.placement_variant(base, Vec3::NEG_X, IVec3::Y, 0.5);
        assert_eq!(new.def(east).facing_turns(), 1);
        assert_eq!(
            new.def(east).face_texture(0, true),
            new.def(base).face_texture(2, false)
        );
        assert_eq!(
            new.def(east).face_texture(0, false),
            new.def(base).face_texture(2, true)
        );
        assert_ne!(
            new.def(east).face_texture(0, false),
            new.def(east).face_texture(2, true)
        );
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

    /// Un registre de base avec des dalles, des escaliers et des poteaux.
    pub(crate) fn with_shapes() -> BlockRegistry {
        let mut registry = (*BlockRegistry::builtin()).clone();
        registry.add_source("test", SHAPES, "test").unwrap();
        registry
    }

    pub(crate) const SHAPES: &str = r#"[
    (id: "test:slab", name: "Dalle", color: (0.5, 0.5, 0.5), texture: "stone",
     boxes: [(from: (0, 0, 0), to: (16, 8, 16))], states: [Half]),
    (id: "test:stairs", name: "Escalier", color: (0.5, 0.5, 0.5),
     boxes: [(from: (0, 0, 0), to: (16, 8, 16)), (from: (0, 8, 0), to: (16, 16, 8))],
     states: [Facing4, Half]),
    (id: "test:post", name: "Poteau", color: (0.5, 0.5, 0.5),
     boxes: [(from: (6, 0, 6), to: (10, 16, 10))], states: [Axis]),
]"#;

    #[test]
    fn a_block_with_boxes_is_a_model_that_does_not_fill_its_cube() {
        let r = with_shapes();
        let slab = r.def(r.id("test:slab").unwrap());
        assert_eq!(slab.shape, Shape::Model);
        assert_eq!(slab.boxes.len(), 1);
        assert_eq!(slab.boxes[0].max, Vec3::new(1.0, 0.5, 1.0));
        assert!(slab.transparent, "les voisins gardent leurs faces");
        assert!(slab.solid);
        assert_eq!(slab.light_opacity, 15, "une dalle pleine arrête la lumière");
    }

    #[test]
    fn states_make_one_variant_per_combination() {
        let r = with_shapes();
        let stairs = r.id("test:stairs").unwrap();
        assert_eq!(
            r.def(stairs).variants.len(),
            8,
            "4 orientations x 2 moitiés"
        );
        // La base est sa première variante, et se retrouve par son nom complet.
        assert_eq!(r.id("test:stairs[facing=north,half=bottom]"), Some(stairs));
        let east_top = r.id("test:stairs[facing=east,half=top]").unwrap();
        assert_eq!(r.base(east_top), stairs);
        assert!(!r.def(east_top).placeable && r.def(stairs).placeable);
        // `variant` suit l'ordre déclaré : facing (indice 1 = east), half (1 = top).
        assert_eq!(r.variant(stairs, &[1, 1]), east_top);
        assert_eq!(
            r.variant(stairs, &[9, 0]),
            stairs,
            "valeur invalide : inchangé"
        );
    }

    #[test]
    fn variants_rotate_and_flip_their_boxes() {
        let r = with_shapes();
        let boxes = |key: &str| r.def(r.id(key).unwrap()).boxes.clone();
        // Dalle en haut : la moitié haute.
        let top = boxes("test:slab[half=top]");
        assert_eq!((top[0].min.y, top[0].max.y), (0.5, 1.0));
        // Escalier : la marche haute est au nord (z de 0 à 0.5) ; à l'est elle est à l'est.
        let east = boxes("test:stairs[facing=east,half=bottom]");
        assert_eq!((east[1].min.x, east[1].max.x), (0.5, 1.0));
        let south = boxes("test:stairs[facing=south,half=bottom]");
        assert_eq!((south[1].min.z, south[1].max.z), (0.5, 1.0));
        // Poteau couché sur X : long sur X, mince sur Y.
        let x = boxes("test:post[axis=x]");
        assert_eq!((x[0].min.x, x[0].max.x), (0.0, 1.0));
        assert!((x[0].max.y - x[0].min.y - 0.25).abs() < 1e-6);
    }

    #[test]
    fn placement_picks_the_variant_from_the_look_and_the_face() {
        let r = with_shapes();
        let stairs = r.id("test:stairs").unwrap();
        let name = |id: BlockId| r.def(id).key.clone();
        // Le joueur regarde vers -Z (le nord) : l'avant lui fait face, donc vers le sud.
        let v = r.placement_variant(stairs, Vec3::NEG_Z, IVec3::Y, 0.0);
        assert_eq!(name(v), "test:stairs[facing=south,half=bottom]");
        // Visé sous un bloc : moitié haute. Sur le côté, selon la hauteur visée.
        assert!(
            name(r.placement_variant(stairs, Vec3::X, IVec3::NEG_Y, 0.0)).ends_with("half=top]")
        );
        assert!(name(r.placement_variant(stairs, Vec3::X, IVec3::X, 0.8)).ends_with("half=top]"));
        assert!(
            name(r.placement_variant(stairs, Vec3::X, IVec3::X, 0.2)).ends_with("half=bottom]")
        );
        // Le poteau suit la face visée.
        let post = r.id("test:post").unwrap();
        assert_eq!(
            name(r.placement_variant(post, Vec3::X, IVec3::Z, 0.0)),
            "test:post[axis=z]"
        );
    }

    #[test]
    fn a_one_way_conduit_points_where_the_player_looks() {
        let mut r = BlockRegistry::empty();
        r.add_source(
            "test",
            r#"[(id: "test:belt", name: "B", color: (1,1,1), boxes: [(from: (0,0,0), to: (16,4,16))],
                  states: [Facing6], conduit: Items, flow: true)]"#,
            "test",
        )
        .unwrap();
        let belt = r.id("test:belt").unwrap();
        let flow = |key: &str| r.def(r.id(key).unwrap()).flow;
        assert_eq!(flow("test:belt"), Some(IVec3::NEG_Z), "la base : le nord");
        assert_eq!(flow("test:belt[facing=east]"), Some(IVec3::X));
        assert_eq!(flow("test:belt[facing=south]"), Some(IVec3::Z));
        assert_eq!(flow("test:belt[facing=up]"), Some(IVec3::Y));
        // Posée en regardant vers +X, elle mène vers +X (un bloc ordinaire ferait face au joueur).
        let placed = r.placement_variant(belt, Vec3::X, IVec3::Y, 0.0);
        assert_eq!(r.def(placed).flow, Some(IVec3::X));
        let up = r.placement_variant(belt, Vec3::Y, IVec3::Y, 0.0);
        assert_eq!(r.def(up).flow, Some(IVec3::Y));
    }

    #[test]
    fn a_one_way_conduit_needs_a_network_it_can_direct_and_a_facing() {
        let bad = |src: &str| {
            BlockRegistry::empty()
                .add_source("test", src, "test")
                .unwrap_err()
                .to_string()
        };
        assert!(bad(r#"[(id: "test:a", name: "A", color: (1,1,1), boxes: [(from: (0,0,0), to: (16,4,16))], states: [Facing4], conduit: Kinetic, flow: true)]"#).contains("pas `Kinetic`"));
        assert!(bad(r#"[(id: "test:a", name: "A", color: (1,1,1), boxes: [(from: (0,0,0), to: (16,4,16))], states: [Half], conduit: Items, flow: true)]"#).contains("Facing"));
        assert!(
            bad(r#"[(id: "test:a", name: "A", color: (1,1,1), conduit: Items, flow: true)]"#)
                .contains("Facing")
        );
    }

    #[test]
    fn bad_boxes_and_states_are_refused() {
        let mut r = BlockRegistry::empty();
        let bad = |src: &str| {
            BlockRegistry::empty()
                .add_source("test", src, "test")
                .unwrap_err()
                .to_string()
        };
        assert!(bad(r#"[(id: "test:a", name: "A", color: (1,1,1), boxes: [(from: (0,0,0), to: (17,8,16))])]"#).contains("0 à 16"));
        assert!(bad(r#"[(id: "test:a", name: "A", color: (1,1,1), boxes: [(from: (8,0,0), to: (8,8,16))])]"#).contains("0 à 16"));
        assert!(
            bad(r#"[(id: "test:a", name: "A", color: (1,1,1), states: [Half])]"#)
                .contains("pas de boîtes")
        );
        assert!(bad(r#"[(id: "test:a", name: "A", color: (1,1,1), boxes: [(from: (0,0,0), to: (8,8,8))], states: [Facing6, Facing6, Half])]"#).contains("variantes"));
        let _ = &mut r;
    }

    #[test]
    fn a_block_can_declare_itself_a_conduit_but_not_with_a_behavior() {
        let mut r = BlockRegistry::empty();
        r.add_source(
            "test",
            r#"[(id: "test:rod", name: "Tige", color: (1,1,1), boxes: [(from: (6,0,6), to: (10,16,10))],
                states: [Axis], conduit: Kinetic)]"#,
            "test",
        )
        .unwrap();
        let rod = r.id("test:rod").unwrap();
        let along_x = r.id("test:rod[axis=x]").unwrap();
        // Les variantes sont aussi des conduits.
        for id in [rod, along_x] {
            assert_eq!(
                r.def(id).conduit,
                Some(crate::networks::NetworkKind::Kinetic)
            );
        }
        let machines = crate::machines::Machines::with_natives(&r);
        assert!(machines.has_behavior(rod) && machines.has_behavior(along_x));
        let err = BlockRegistry::empty()
            .add_source(
                "test",
                r#"[(id: "test:bad", name: "B", color: (1,1,1), conduit: Energy, behavior: "x")]"#,
                "test",
            )
            .unwrap_err();
        assert!(err.to_string().contains("conduit"), "{err}");
    }

    #[test]
    fn enchanting_stats_are_optional_and_checked() {
        let mut r = BlockRegistry::empty();
        r.add_source(
            "test",
            r#"[
                (id: "test:shelf", name: "S", color: (1,1,1), enchanting: (eterna: 2.0, max_eterna: 40.0, arcana: 3.0, treasure: true)),
                (id: "test:plain", name: "P", color: (1,1,1)),
                (id: "test:library", name: "L", color: (1,1,1), enchanting: (library_level: 12)),
            ]"#,
            "test",
        )
        .unwrap();
        let shelf = r
            .def(r.id("test:shelf").unwrap())
            .enchanting
            .clone()
            .unwrap();
        assert_eq!(
            (shelf.eterna, shelf.max_eterna, shelf.arcana),
            (2.0, 40.0, 3.0)
        );
        assert!(shelf.treasure);
        // Ce qui n'est pas dit vaut zéro.
        assert_eq!(
            (shelf.quanta, shelf.stability, shelf.clues),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(shelf.library_level, 0);
        assert!(r.def(r.id("test:plain").unwrap()).enchanting.is_none());
        assert_eq!(
            r.def(r.id("test:library").unwrap())
                .enchanting
                .as_ref()
                .unwrap()
                .library_level,
            12
        );
        // Un champ mal orthographié est refusé, comme dans le reste du fichier.
        let bad = BlockRegistry::empty().add_source(
            "test",
            r#"[(id: "test:a", name: "A", color: (1,1,1), enchanting: (eterne: 1.0))]"#,
            "test",
        );
        assert!(bad.is_err());
        // Les étagères du jeu de base montent en échelle : plafond et Eterna croissants.
        let core = BlockRegistry::builtin();
        let caps: Vec<f32> = ["oak", "sapphire", "amethyst", "jade", "volcanic"]
            .iter()
            .map(|m| {
                core.def(core.id(&format!("core:shelf_{m}")).unwrap())
                    .enchanting
                    .clone()
                    .unwrap()
                    .max_eterna
            })
            .collect();
        assert_eq!(caps, vec![15.0, 40.0, 50.0, 65.0, 100.0]);
        let table = core.def(core.id("core:enchanting_table").unwrap());
        assert_eq!(table.enchanting.as_ref().unwrap().quanta, 15.0);
    }

    #[test]
    fn channels_throughput_and_ports_are_read_and_checked() {
        let mut r = BlockRegistry::empty();
        r.add_source(
            "test",
            r#"[
                (id: "test:belt", name: "B", color: (1,1,1), conduit: Items, channel: "belts", throughput: 4),
                (id: "test:press", name: "P", color: (1,1,1), ports: (inputs: [0], outputs: [1], accepts: "press")),
            ]"#,
            "test",
        )
        .unwrap();
        let belt = r.def(r.id("test:belt").unwrap());
        assert_eq!(belt.conduit, Some(crate::networks::NetworkKind::Items));
        assert_eq!(
            (belt.channel.as_deref(), belt.throughput),
            (Some("test:belts"), Some(4))
        );
        let ports = r.def(r.id("test:press").unwrap()).ports.clone().unwrap();
        assert_eq!((ports.inputs, ports.outputs), (vec![0], vec![1]));
        assert_eq!(ports.accepts.as_deref(), Some("test:press"));
        let bad = |src: &str| {
            BlockRegistry::empty()
                .add_source("test", src, "test")
                .unwrap_err()
                .to_string()
        };
        assert!(
            bad(r#"[(id: "test:a", name: "A", color: (1,1,1), channel: "x")]"#).contains("conduit")
        );
        assert!(
            bad(r#"[(id: "test:a", name: "A", color: (1,1,1), conduit: Items, throughput: 0)]"#)
                .contains("au moins 1")
        );
    }
}
