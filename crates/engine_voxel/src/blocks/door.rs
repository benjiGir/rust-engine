//! Portes à deux cellules. Le registre porte l'orientation et l'ouverture de chaque battant.

use glam::{IVec3, Vec3};

use crate::{BlockDef, BlockId, BlockRegistry, StateKind, World};

const TYPES: [(&str, &str); 4] = [
    ("core:wooden_door", "core:wooden_door_top"),
    ("core:wooden_glass_door", "core:wooden_glass_door_top"),
    ("core:iron_door", "core:iron_door_top"),
    ("core:iron_glass_door", "core:iron_glass_door_top"),
];
/// Verre légèrement teinté en espace linéaire.
pub const GLAZING_COLOR: [f32; 4] = [0.44, 0.70, 0.76, 0.23];

fn kind(registry: &BlockRegistry, block: BlockId) -> Option<(&'static str, &'static str, bool)> {
    let def = registry.def(registry.base(block));
    TYPES.iter().find_map(|&(bottom, top)| {
        let upper = def.key == top;
        (upper || def.key == bottom).then_some((bottom, top, upper))
    })
}

pub fn is_door(def: &BlockDef) -> bool {
    def.states.first() == Some(&StateKind::DoorOpen)
}

pub fn is_open(def: &BlockDef) -> bool {
    is_door(def) && def.state_values.first() == Some(&1)
}

pub fn item_key(registry: &BlockRegistry, block: BlockId) -> Option<&'static str> {
    kind(registry, block).map(|(bottom, _, _)| bottom)
}

pub fn placement(
    registry: &BlockRegistry,
    block: BlockId,
    pos: IVec3,
    look: Vec3,
) -> Option<[(IVec3, BlockId); 2]> {
    let (bottom, top, upper) = kind(registry, block)?;
    if upper {
        return None;
    }
    let bottom = registry.placement_variant(registry.id(bottom)?, look, IVec3::Y, 0.0);
    let top = registry.variant(registry.id(top)?, &registry.def(bottom).state_values);
    Some([(pos, bottom), (pos + IVec3::Y, top)])
}

pub(crate) fn partner(
    registry: &BlockRegistry,
    pos: IVec3,
    block: BlockId,
) -> Option<(IVec3, BlockId)> {
    if !is_door(registry.def(block)) {
        return None;
    }
    let (bottom, top, upper) = kind(registry, block)?;
    let other = registry.id(if upper { bottom } else { top })?;
    let position = pos + if upper { IVec3::NEG_Y } else { IVec3::Y };
    Some((
        position,
        registry.variant(other, &registry.def(block).state_values),
    ))
}

pub fn toggled(world: &World, pos: IVec3) -> Option<[(IVec3, BlockId); 2]> {
    let registry = world.registry();
    let block = world.block(pos);
    let (other, expected) = partner(registry, pos, block)?;
    if world.block(other) != expected {
        return None;
    }
    let values = [
        u8::from(!is_open(registry.def(block))),
        registry.def(block).state_values[1],
    ];
    Some([
        (pos, registry.variant(block, &values)),
        (other, registry.variant(expected, &values)),
    ])
}

pub(crate) fn display_parts(
    registry: &BlockRegistry,
    block: BlockId,
) -> Option<[(BlockId, Vec3); 2]> {
    let (bottom, top, _) = kind(registry, block)?;
    Some([
        (registry.id(bottom)?, Vec3::NEG_Y * 0.5),
        (registry.id(top)?, Vec3::Y * 0.5),
    ])
}

pub(crate) fn turned(registry: &BlockRegistry, block: BlockId, k: u8) -> BlockId {
    let def = registry.def(block);
    if is_door(def) {
        registry.variant(block, &[def.state_values[0], (def.state_values[1] + k) % 4])
    } else {
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_door_preserves_its_facing_when_toggled_and_its_hinge_when_turned() {
        let registry = BlockRegistry::builtin();
        let mut world = World::with_registry(registry.clone());
        for (key, _) in TYPES {
            let id = registry.id(key).unwrap();
            for (facing, look) in [Vec3::Z, Vec3::NEG_X, Vec3::NEG_Z, Vec3::X]
                .into_iter()
                .enumerate()
            {
                let parts = placement(&registry, id, IVec3::Y, look).unwrap();
                for (pos, block) in parts {
                    world.set_block(pos, block);
                    assert_eq!(registry.def(block).state_values, [0, facing as u8]);
                }
                for (_, open) in toggled(&world, 2 * IVec3::Y).unwrap() {
                    assert_eq!(registry.def(open).state_values, [1, facing as u8]);
                    let rotated = turned(&registry, open, 1);
                    assert_eq!(
                        registry.def(rotated).state_values,
                        [1, ((facing + 1) % 4) as u8]
                    );
                    assert_eq!(item_key(&registry, rotated), Some(key));
                }
            }
        }
    }
}
