//! La version de génération d'un monde et sa configuration effective.
//!
//! # Deux versions distinctes
//!
//! - la **version du format** de sauvegarde ([`FORMAT_VERSION`](crate::FORMAT_VERSION)) dit
//!   comment lire les fichiers ;
//! - la **version de génération** ([`GenerationVersion`]) dit comment le terrain est calculé
//!   depuis la graine. Un monde garde la sienne : changer les formules du relief ferait changer
//!   le paysage sous les constructions du joueur.
//!
//! Un `world.ron` sans champ `generation` (un monde d'avant ce champ) sélectionne **explicitement**
//! la version historique ([`GenerationVersion::LEGACY`]) : aucune migration silencieuse.
//!
//! # La configuration effective
//!
//! Une version ne suffit pas : les mods ajoutent des biomes, des minerais, des structures et des
//! dimensions. [`GenerationMeta`] garde donc, par dimension, son genre de monde et son décalage de
//! graine (en clair, avec des identifiants texte), et, pour tout le monde, l'**empreinte** des
//! fichiers de données de génération de chaque mod (`biomes.ron`, `ores.ron`, `structures.ron`,
//! `dimensions.ron`) et du jeu de base.
//!
//! Limite assumée : une empreinte permet de **détecter** un mod retiré ou modifié
//! ([`GenerationMeta::compare`]), pas de reconstruire sa configuration. Les données elles-mêmes ne
//! sont pas copiées dans la sauvegarde ; la référence est « espace de noms + fichier + empreinte »,
//! résoluble tant que le mod est installé tel quel.

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::biome::BiomeRegistry;
use crate::dimension::{DimensionRegistry, WorldKind};
use crate::registry::BlockRegistry;

/// Les données de génération du jeu de base, embarquées dans le binaire (comme les lit
/// `BiomeRegistry` et `StructureSet`) : une mise à jour du jeu qui les change se détecte aussi.
const CORE_DATA: [(&str, &str); 3] = [
    (
        "biomes.ron",
        include_str!("../../../assets/core/biomes.ron"),
    ),
    ("ores.ron", include_str!("../../../assets/core/ores.ron")),
    (
        "structures.ron",
        include_str!("../../../assets/core/structures.ron"),
    ),
];
/// Les fichiers de données de génération qu'un mod peut fournir.
const MOD_FILES: [&str; 4] = ["biomes.ron", "ores.ron", "structures.ron", "dimensions.ron"];

/// Comment le terrain est calculé depuis la graine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum GenerationVersion {
    /// La génération historique : relief par bruit, rivières au passage à zéro, grottes 3D.
    /// Ses blocs sont figés par `tests/generation_reference.rs`.
    #[default]
    V1,
    /// La génération v2 (lot 4 du plan) : relief macro (géographie et drainage), densité 3D,
    /// surface selon la pente, strates régionales. **Pas encore activée pour les nouveaux mondes**
    /// (voir [`GenerationVersion::FOR_NEW_WORLDS`]) : seul un monde de test la choisit, par
    /// [`GenerationVersion::for_new_world`] et la variable d'environnement
    /// [`GenerationVersion::ENV_OVERRIDE`].
    V2,
    /// La génération v3 (`docs/plan-generation-v3.md`) : une densité 3D sur le modèle de Minecraft
    /// moderne (paramètres climatiques, splines, bruit 3D), un monde de y = -128 à 255. Monde de test
    /// seulement (`VOXEL_GENERATION=3`), comme v2.
    V3,
}

impl GenerationVersion {
    /// Ce que sélectionne un monde sans métadonnée de génération.
    pub const LEGACY: Self = Self::V1;
    /// Ce que choisit la création d'un nouveau monde. Reste v1 tant que la recette du lot 9 n'a
    /// pas eu lieu.
    pub const FOR_NEW_WORLDS: Self = Self::V1;
    /// La variable d'environnement qui crée un monde de **test** en v2 : `VOXEL_GENERATION=2`
    /// (ou `v2`). Elle n'est lue qu'à la **création** d'un monde ; un monde existant garde la
    /// version de son `world.ron`, quelle que soit la variable.
    pub const ENV_OVERRIDE: &'static str = "VOXEL_GENERATION";

    /// La version d'un monde que l'on crée maintenant : [`Self::FOR_NEW_WORLDS`], ou la version
    /// demandée par [`Self::ENV_OVERRIDE`] (monde de test).
    pub fn for_new_world() -> Self {
        Self::for_new_world_with(std::env::var(Self::ENV_OVERRIDE).ok().as_deref())
    }

    /// Comme [`Self::for_new_world`], avec la valeur de la variable donnée (testable sans toucher à
    /// l'environnement). Une valeur inconnue est ignorée : la version par défaut des nouveaux
    /// mondes.
    pub fn for_new_world_with(requested: Option<&str>) -> Self {
        match requested.map(|v| v.trim().to_lowercase()).as_deref() {
            Some("1" | "v1") => Self::V1,
            Some("2" | "v2") => Self::V2,
            Some("3" | "v3") => Self::V3,
            _ => Self::FOR_NEW_WORLDS,
        }
    }

    /// Le numéro écrit dans `world.ron`.
    pub fn number(self) -> u32 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
            Self::V3 => 3,
        }
    }

    /// Relit un numéro. Un numéro inconnu vient d'un jeu plus récent : erreur, jamais un repli
    /// silencieux sur une autre génération.
    pub fn from_number(number: u32) -> Result<Self> {
        match number {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            other => bail!(
                "version de génération {other} inconnue : ce monde vient d'une version plus récente du jeu"
            ),
        }
    }
}

impl std::fmt::Display for GenerationVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::V1 => f.write_str("v1 (historique)"),
            Self::V2 => f.write_str("v2 (relief macro, monde de test)"),
            Self::V3 => f.write_str("v3 (densité 3D, monde de test)"),
        }
    }
}

/// Le genre de monde d'une dimension, en identifiants texte (jamais de numéro de bloc ou de biome).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KindRecord {
    Terrain {
        biomes: Vec<String>,
    },
    Flat {
        layers: Vec<(String, u32)>,
        biome: String,
    },
    Void {
        platform: Option<String>,
    },
}

/// La configuration effective d'une dimension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionRecord {
    pub key: String,
    /// Ajouté à la graine du monde pour cette dimension.
    pub seed_offset: u32,
    pub kind: KindRecord,
}

/// L'empreinte d'un fichier de données de génération.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataRecord {
    /// L'espace de noms du mod (`core` pour le jeu de base).
    pub namespace: String,
    pub file: String,
    /// FNV-1a 64 bits du contenu du fichier.
    pub hash: u64,
}

/// Ce que `world.ron` retient de la génération.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationMeta {
    /// Voir [`GenerationVersion::number`].
    pub version: u32,
    pub dimensions: Vec<DimensionRecord>,
    pub data: Vec<DataRecord>,
}

/// Un écart entre la configuration d'un monde sauvegardé et celle d'aujourd'hui.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationIssue {
    /// Un fichier de données sauvegardé n'existe plus (mod retiré).
    MissingData { namespace: String, file: String },
    /// Un fichier de données a changé (mod mis à jour, jeu mis à jour).
    ChangedData { namespace: String, file: String },
    /// Un fichier de données est apparu (mod ajouté) : les chunks pas encore générés changeront.
    AddedData { namespace: String, file: String },
    /// Une dimension sauvegardée n'existe plus.
    MissingDimension { key: String },
    /// Une dimension a un autre genre de monde ou un autre décalage de graine.
    ChangedDimension { key: String },
}

impl std::fmt::Display for GenerationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingData { namespace, file } => write!(
                f,
                "les données de génération « {namespace}/{file} » du monde sont absentes (mod retiré ?)"
            ),
            Self::ChangedData { namespace, file } => write!(
                f,
                "les données de génération « {namespace}/{file} » ont changé depuis la création du monde : les chunks pas encore générés peuvent différer"
            ),
            Self::AddedData { namespace, file } => write!(
                f,
                "nouvelles données de génération « {namespace}/{file} » depuis la création du monde : les chunks pas encore générés peuvent différer"
            ),
            Self::MissingDimension { key } => {
                write!(f, "la dimension « {key} » du monde n'existe plus")
            }
            Self::ChangedDimension { key } => write!(
                f,
                "la dimension « {key} » n'est plus générée comme à la création du monde"
            ),
        }
    }
}

/// FNV-1a 64 bits : stable d'une version de Rust à l'autre (contrairement au hasheur standard).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl GenerationMeta {
    /// Décrit la configuration effective d'aujourd'hui : toutes les dimensions connues et les
    /// fichiers de données des mods, dans l'ordre de chargement. Un fichier illisible est omis
    /// (le chargement du monde l'aura déjà signalé).
    pub fn describe(
        version: GenerationVersion,
        dimensions: &DimensionRegistry,
        blocks: &BlockRegistry,
        biomes: &BiomeRegistry,
        mods: &[(String, PathBuf)],
    ) -> Self {
        let block_key = |id| blocks.def(id).key.clone();
        let biome_key = |id| biomes.get(id).key.clone();
        let dimensions = dimensions
            .iter()
            .map(|d| DimensionRecord {
                key: d.key.clone(),
                seed_offset: d.seed_offset,
                kind: match &d.kind {
                    WorldKind::Terrain { biomes } => KindRecord::Terrain {
                        biomes: biomes.iter().map(|&b| biome_key(b)).collect(),
                    },
                    WorldKind::Flat { layers, biome } => KindRecord::Flat {
                        layers: layers.iter().map(|&(b, n)| (block_key(b), n)).collect(),
                        biome: biome_key(*biome),
                    },
                    WorldKind::Void { platform } => KindRecord::Void {
                        platform: platform.map(block_key),
                    },
                },
            })
            .collect();
        let mut data: Vec<DataRecord> = CORE_DATA
            .iter()
            .map(|(file, text)| DataRecord {
                namespace: "core".into(),
                file: (*file).into(),
                hash: fnv1a(text.as_bytes()),
            })
            .collect();
        for (namespace, dir) in mods {
            for file in MOD_FILES {
                if let Ok(bytes) = std::fs::read(dir.join(file)) {
                    data.push(DataRecord {
                        namespace: namespace.clone(),
                        file: file.into(),
                        hash: fnv1a(&bytes),
                    });
                }
            }
        }
        Self {
            version: version.number(),
            dimensions,
            data,
        }
    }

    /// La version de génération enregistrée.
    pub fn generation_version(&self) -> Result<GenerationVersion> {
        GenerationVersion::from_number(self.version)
    }

    /// Les écarts entre cette configuration (celle du monde sauvegardé) et `now`. Vide : le monde
    /// se génère comme à sa création.
    pub fn compare(&self, now: &Self) -> Vec<GenerationIssue> {
        let mut issues = Vec::new();
        let same = |a: &DataRecord, b: &DataRecord| a.namespace == b.namespace && a.file == b.file;
        for saved in &self.data {
            match now.data.iter().find(|d| same(saved, d)) {
                None => issues.push(GenerationIssue::MissingData {
                    namespace: saved.namespace.clone(),
                    file: saved.file.clone(),
                }),
                Some(current) if current.hash != saved.hash => {
                    issues.push(GenerationIssue::ChangedData {
                        namespace: saved.namespace.clone(),
                        file: saved.file.clone(),
                    })
                }
                Some(_) => {}
            }
        }
        for current in &now.data {
            if !self.data.iter().any(|d| same(d, current)) {
                issues.push(GenerationIssue::AddedData {
                    namespace: current.namespace.clone(),
                    file: current.file.clone(),
                });
            }
        }
        for saved in &self.dimensions {
            match now.dimensions.iter().find(|d| d.key == saved.key) {
                None => issues.push(GenerationIssue::MissingDimension {
                    key: saved.key.clone(),
                }),
                Some(current) if current != saved => {
                    issues.push(GenerationIssue::ChangedDimension {
                        key: saved.key.clone(),
                    })
                }
                Some(_) => {}
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_for(mods: &[(String, PathBuf)]) -> GenerationMeta {
        let blocks = BlockRegistry::builtin();
        let biomes = BiomeRegistry::builtin(&blocks).unwrap();
        GenerationMeta::describe(
            GenerationVersion::V1,
            &DimensionRegistry::builtin(),
            &blocks,
            &biomes,
            mods,
        )
    }

    fn temp_mod(name: &str, biomes: &str) -> (String, PathBuf) {
        let dir = std::env::temp_dir().join(format!("voxel-gen-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("biomes.ron"), biomes).unwrap();
        (name.to_string(), dir)
    }

    #[test]
    fn version_numbers_round_trip_and_unknown_ones_are_refused() {
        assert_eq!(
            GenerationVersion::from_number(1).unwrap(),
            GenerationVersion::V1
        );
        assert_eq!(GenerationVersion::V1.number(), 1);
        assert_eq!(GenerationVersion::LEGACY, GenerationVersion::V1);
        assert_eq!(
            GenerationVersion::from_number(2).unwrap(),
            GenerationVersion::V2
        );
        assert_eq!(GenerationVersion::V2.number(), 2);
        assert!(GenerationVersion::from_number(0).is_err());
        assert!(GenerationVersion::from_number(99).is_err());
    }

    #[test]
    fn new_worlds_stay_v1_unless_a_test_world_is_requested() {
        assert_eq!(GenerationVersion::FOR_NEW_WORLDS, GenerationVersion::V1);
        assert_eq!(
            GenerationVersion::for_new_world_with(None),
            GenerationVersion::V1
        );
        for v2 in ["2", "v2", " V2 "] {
            assert_eq!(
                GenerationVersion::for_new_world_with(Some(v2)),
                GenerationVersion::V2
            );
        }
        for v3 in ["3", "v3", " V3 "] {
            assert_eq!(
                GenerationVersion::for_new_world_with(Some(v3)),
                GenerationVersion::V3
            );
        }
        for v1 in ["1", "v1", "", "4", "n'importe quoi"] {
            assert_eq!(
                GenerationVersion::for_new_world_with(Some(v1)),
                GenerationVersion::V1
            );
        }
    }

    #[test]
    fn the_effective_configuration_lists_dimensions_and_core_data() {
        let meta = meta_for(&[]);
        assert_eq!(meta.version, 1);
        assert_eq!(meta.dimensions.len(), 1);
        assert_eq!(meta.dimensions[0].key, crate::dimension::OVERWORLD);
        assert_eq!(
            meta.dimensions[0].kind,
            KindRecord::Terrain { biomes: Vec::new() }
        );
        let files: Vec<&str> = meta.data.iter().map(|d| d.file.as_str()).collect();
        assert_eq!(files, ["biomes.ron", "ores.ron", "structures.ron"]);
        assert_eq!(meta, meta_for(&[]), "la description est déterministe");
    }

    #[test]
    fn a_missing_changed_or_added_mod_file_is_reported() {
        let saved = meta_for(&[temp_mod("gen_a", "[]")]);
        assert!(
            saved
                .compare(&meta_for(&[temp_mod("gen_a", "[]")]))
                .is_empty()
        );

        let changed = saved.compare(&meta_for(&[temp_mod("gen_a", "[ ]  ")]));
        assert_eq!(
            changed,
            [GenerationIssue::ChangedData {
                namespace: "gen_a".into(),
                file: "biomes.ron".into()
            }]
        );
        let missing = saved.compare(&meta_for(&[]));
        assert_eq!(
            missing,
            [GenerationIssue::MissingData {
                namespace: "gen_a".into(),
                file: "biomes.ron".into()
            }]
        );
        let added = meta_for(&[]).compare(&saved);
        assert_eq!(
            added,
            [GenerationIssue::AddedData {
                namespace: "gen_a".into(),
                file: "biomes.ron".into()
            }]
        );
        assert!(changed[0].to_string().contains("gen_a/biomes.ron"));
    }

    #[test]
    fn a_changed_or_removed_dimension_is_reported() {
        let saved = meta_for(&[]);
        let mut now = saved.clone();
        now.dimensions[0].seed_offset = 5;
        assert_eq!(
            saved.compare(&now),
            [GenerationIssue::ChangedDimension {
                key: crate::dimension::OVERWORLD.into()
            }]
        );
        now.dimensions.clear();
        assert_eq!(
            saved.compare(&now),
            [GenerationIssue::MissingDimension {
                key: crate::dimension::OVERWORLD.into()
            }]
        );
    }
}
