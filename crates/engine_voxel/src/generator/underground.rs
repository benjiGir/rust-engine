//! Le sous-sol du générateur v2 : cavernes, nappes et lave.
//!
//! Les **formes** sont celles de la v1 : tunnels « spaghetti » (deux champs 3D proches de zéro) et
//! grandes cavernes « fromage » (champ haut, plus rare près de la surface), évalués sur la même
//! grille de 4 blocs, avec les mêmes tampons par worker (`CaveGrid`). Ce qui change, ce sont les
//! **règles** : où l'on a le droit de creuser, et ce qui remplit le vide.
//!
//! - **Plafond** : fixé par colonne dans le contexte (`CoreCell::cave_limit`) : 4 blocs sous le relief,
//!   9 sous l'eau, plus de 10 sous une zone réservée (lit, berge, lac) ; jamais de caverne sous un
//!   cours ni sous un lac. Une **entrée contrôlée** peut affleurer (à deux blocs du relief) sur une
//!   falaise nue, rare et jamais dans une zone réservée : l'ouverture se lit sur la paroi, la peau du
//!   haut reste.
//! - **Nappes** : une caverne se remplit d'eau sous le niveau de la nappe de sa colonne. La nappe est
//!   la hauteur d'un relief atténué (0,28 de l'altitude macro au-dessus de la mer) ramenée vers le
//!   niveau du cours ou du lac voisin : au bord d'un cours, la nappe est 2 blocs sous son niveau et
//!   monte de 0,1 bloc par bloc en s'éloignant ; la relation s'efface à 140 blocs d'un cours (56 d'un
//!   lac). Elle est bornée à 8 blocs sous le relief le plus bas du voisinage (5 × 5 colonnes).
//! - **Eau et lave** : la lave remplit les cavernes jusqu'à `LAVA_LEVEL` ; l'eau ne descend jamais
//!   sous `LAVA_LEVEL + 4` : entre les deux, il reste du vide ou de la roche, jamais un contact.

use glam::IVec3;

use super::v2::{ColumnContext, LAVA_GAP, NO_WATER};
use super::{CaveGrid, LAVA_LEVEL, TerrainGenerator};
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};

impl TerrainGenerator {
    /// Creuse les cavernes d'un chunk dont le terrain est posé, et les remplit d'eau ou de lave.
    pub(super) fn carve_underground(&self, chunk: &mut Chunk, origin: IVec3, ctx: &ColumnContext) {
        // Un chunk entièrement au-dessus des plafonds n'a pas besoin des bruits.
        if origin.y > ctx.cave_ceiling {
            return;
        }
        let caves = CaveGrid::new(self, origin);
        let buffers = caves.buffers();
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let core = ctx.core_at(lx, lz);
                let limit = core.cave_limit as i32;
                let top = core.top as i32;
                for ly in 0..CHUNK_SIZE {
                    let y = origin.y + ly as i32;
                    if y <= 0 || y > limit {
                        continue;
                    }
                    let current = chunk.get(lx, ly, lz);
                    if current.is_air() || current == self.blocks.water {
                        continue;
                    }
                    let a = CaveGrid::sample(&buffers.tunnel_a, lx, ly, lz);
                    let b = CaveGrid::sample(&buffers.tunnel_b, lx, ly, lz);
                    let tunnel = a * a + b * b < 0.006;
                    // Plus rares près de la surface, pour que le sol ne s'effondre pas.
                    let near_surface = (y - (top - 24)).max(0) as f32;
                    let cheese = !tunnel
                        && CaveGrid::sample(&buffers.cheese, lx, ly, lz)
                            > 0.42 + near_surface * 0.02;
                    if !tunnel && !cheese {
                        continue;
                    }
                    let fill: BlockId = if y <= LAVA_LEVEL {
                        self.blocks.lava
                    } else if core.table != NO_WATER
                        && y <= core.table as i32
                        && y >= LAVA_LEVEL + LAVA_GAP
                    {
                        self.blocks.water
                    } else {
                        BlockId::AIR
                    };
                    chunk.set(lx, ly, lz, fill);
                }
            }
        }
    }
}
