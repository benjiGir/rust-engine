//! Les règles physiques des blocs : le **support** et la **gravité**.
//!
//! - Une plante, une pousse ou une torche a besoin d'un support (`support` dans
//!   `blocks.ron`) : sans lui, elle tombe et lâche son objet.
//! - Le sable et le gravier (`gravity: true`) tombent tant qu'il n'y a que de l'air (ou
//!   un liquide, une plante) dessous.
//!
//! Une seule modification peut en entraîner d'autres : casser le bloc sous une colonne de
//! sable fait tomber toute la colonne, et les fleurs posées dessus. On traite donc une
//! **file de positions à vérifier** : chaque changement ajoute ses voisins à la file,
//! jusqu'à ce que plus rien ne bouge.

use std::collections::VecDeque;

use glam::IVec3;

use crate::block::BlockId;
use crate::registry::{BlockRegistry, Shape, Support};
use crate::streaming::ChunkStreamer;

/// Ce qui s'est passé après une modification.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Settled {
    /// Les blocs détachés (plus de support, ou écrasés par un bloc qui tombe) : à lâcher
    /// en objets à leur position.
    pub broken: Vec<(IVec3, BlockId)>,
    /// Les blocs tombés : (départ, arrivée, bloc).
    pub fallen: Vec<(IVec3, IVec3, BlockId)>,
}

const NEIGHBOURS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Y,
    IVec3::NEG_Y,
    IVec3::Z,
    IVec3::NEG_Z,
];

/// Au-delà, on arrête (une avalanche gigantesque continuera à la modification suivante).
const MAX_CHECKS: usize = 4096;

/// Le bloc en `pos` tient-il, selon son besoin de support ?
pub fn is_supported(
    streamer: &ChunkStreamer,
    registry: &BlockRegistry,
    pos: IVec3,
    support: Support,
) -> bool {
    let world = streamer.world();
    let solid = |p: IVec3| {
        let block = world.block(p);
        !block.is_air() && registry.is_solid(block) && registry.def(block).shape == Shape::Cube
    };
    match support {
        Support::None => true,
        Support::Floor => solid(pos - IVec3::Y),
        Support::Any => NEIGHBOURS.iter().any(|&o| solid(pos + o)),
        Support::Water => registry.def(world.block(pos - IVec3::Y)).liquid,
        Support::Farmland => {
            crate::blocks::farming::is_farmland(registry, world.block(pos - IVec3::Y))
        }
    }
}

/// Un bloc qui tombe peut-il traverser celui-ci ? L'air, les liquides et les plantes.
fn can_fall_through(registry: &BlockRegistry, block: BlockId) -> bool {
    let def = registry.def(block);
    block.is_air() || def.liquid || (!def.solid && def.shape != Shape::Cube)
}

/// Applique le support et la gravité autour de `changed` (qui vient de changer).
pub fn settle(streamer: &mut ChunkStreamer, changed: IVec3) -> Settled {
    let registry = streamer.world().registry().clone();
    let mut result = Settled::default();
    let mut queue: VecDeque<IVec3> = VecDeque::from([changed]);
    queue.extend(NEIGHBOURS.iter().map(|&o| changed + o));
    let mut checks = 0;
    while let Some(pos) = queue.pop_front() {
        checks += 1;
        if checks > MAX_CHECKS {
            break;
        }
        let block = streamer.world().block(pos);
        if block.is_air() {
            continue;
        }
        let def = registry.def(block);
        if def.support != Support::None && !is_supported(streamer, &registry, pos, def.support) {
            let partner = crate::blocks::paired_block::partner(&registry, pos, block)
                .filter(|&(p, b)| streamer.world().block(p) == b);
            if streamer.set_block(pos, BlockId::AIR) {
                result.broken.push((pos, block));
                queue.extend(NEIGHBOURS.iter().map(|&o| pos + o));
                if let Some((other, _)) = partner {
                    queue.extend(NEIGHBOURS.iter().map(|&o| other + o));
                }
            }
            continue;
        }
        if def.gravity
            && can_fall_through(&registry, streamer.world().block(pos - IVec3::Y))
            && pos.y > 0
        {
            // Il tombe jusqu'au premier bloc qui l'arrête. Les plantes traversées sont écrasées.
            let mut landing = pos;
            while landing.y > 0
                && can_fall_through(&registry, streamer.world().block(landing - IVec3::Y))
            {
                landing -= IVec3::Y;
                let below = streamer.world().block(landing);
                if !below.is_air() && !registry.def(below).liquid {
                    result.broken.push((landing, below));
                }
            }
            if streamer.set_block(pos, BlockId::AIR) && streamer.set_block(landing, block) {
                result.fallen.push((pos, landing, block));
                queue.extend(NEIGHBOURS.iter().map(|&o| pos + o));
                queue.extend(NEIGHBOURS.iter().map(|&o| landing + o));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::streaming::{StreamingConfig, tests::pump_until_idle};
    use crate::worldgen::generator::TerrainGenerator;

    fn world() -> ChunkStreamer {
        let config = StreamingConfig {
            view_radius: 1,
            ..StreamingConfig::default()
        };
        let mut s = ChunkStreamer::new(TerrainGenerator::new(7), config);
        pump_until_idle(&mut s, Vec3::new(5.0, 100.0, 5.0));
        s
    }

    fn id(s: &ChunkStreamer, key: &str) -> BlockId {
        s.world().registry().id(key).unwrap()
    }

    #[test]
    fn a_flower_falls_when_its_ground_is_broken() {
        let mut s = world();
        let (stone, flower) = (id(&s, "core:stone"), id(&s, "core:poppy"));
        let ground = IVec3::new(8, 140, 8);
        s.set_block(ground, stone);
        s.set_block(ground + IVec3::Y, flower);
        assert!(
            settle(&mut s, ground + IVec3::Y).broken.is_empty(),
            "la fleur tient"
        );
        s.set_block(ground, BlockId::AIR);
        let settled = settle(&mut s, ground);
        assert_eq!(settled.broken, vec![(ground + IVec3::Y, flower)]);
        assert!(s.world().block(ground + IVec3::Y).is_air());
    }

    #[test]
    fn a_torch_needs_a_neighbour() {
        let mut s = world();
        let (stone, torch) = (id(&s, "core:stone"), id(&s, "core:torch"));
        let wall = IVec3::new(8, 140, 8);
        s.set_block(wall, stone);
        s.set_block(wall + IVec3::X, torch);
        assert!(
            settle(&mut s, wall + IVec3::X).broken.is_empty(),
            "accrochée au mur"
        );
        s.set_block(wall, BlockId::AIR);
        assert_eq!(settle(&mut s, wall).broken, vec![(wall + IVec3::X, torch)]);
    }

    #[test]
    fn a_sand_column_falls_and_crushes_a_flower() {
        let mut s = world();
        let (stone, sand, flower) = (
            id(&s, "core:stone"),
            id(&s, "core:sand"),
            id(&s, "core:poppy"),
        );
        let floor = IVec3::new(8, 130, 8);
        s.set_block(floor, stone);
        s.set_block(floor + IVec3::Y, flower);
        // Trois blocs de sable en l'air, quatre blocs au-dessus de la fleur.
        for k in 5..8 {
            s.set_block(floor + IVec3::Y * k, sand);
        }
        let settled = settle(&mut s, floor + IVec3::Y * 5);
        for k in 1..4 {
            assert_eq!(s.world().block(floor + IVec3::Y * k), sand, "y + {k}");
        }
        assert!(s.world().block(floor + IVec3::Y * 4).is_air());
        assert_eq!(settled.fallen.len(), 3);
        assert!(settled.broken.contains(&(floor + IVec3::Y, flower)));
    }
}
