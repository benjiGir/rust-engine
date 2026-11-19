//! Le moteur de densité de la génération v3 (lot 1 de `docs/plan-generation-v3.md`) : des bruits et
//! des fonctions composées décrits en RON, compilés en graphe et évalués par position ou par chunk.
//!
//! Le modèle est celui de la génération de Minecraft moderne (repris d'Horizon, réécrit ici) : le
//! terrain est une **densité 3D**, positive dans la matière, négative dans le vide. Les fonctions se
//! composent (sommes, produits, splines de paramètres climatiques, bruits déformés) ; la part chère
//! est calculée aux coins de cellules de 4 × 8 × 4 blocs et interpolée.
//!
//! - [`noise`] : Perlin amélioré, octaves, « normal noise » ;
//! - [`def`] : la description RON ([`DensityFile`], [`DensityDef`]) ;
//! - [`graph`] : la compilation et l'évaluation ([`DensityGraph`]).

pub mod def;
pub mod graph;
pub mod noise;

pub use def::{DensityDef, DensityFile, SplineDef, SplinePoint, SplineValue};
pub use graph::{CELL_HEIGHT, CELL_WIDTH, DensityError, DensityGraph, NodeId};
pub use noise::{ImprovedPerlin, NoiseDef, NormalNoise, OctaveNoise};
