//! Les deux cellules d'un lit partagent une orientation. L'ancien bloc reste lisible.

use glam::{IVec3, Vec3};

use crate::{BlockId, BlockRegistry, World};

pub const ITEM: &str = "core:bed";
pub const FOOT: &str = "core:bed_foot";
pub const HEAD: &str = "core:bed_head";
const DIRECTIONS: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];

fn part(registry: &BlockRegistry, block: BlockId) -> Option<bool> {
    match registry.def(registry.base(block)).key.as_str() {
        FOOT => Some(false),
        HEAD => Some(true),
        _ => None,
    }
}

pub fn is_part(registry: &BlockRegistry, block: BlockId) -> bool {
    part(registry, block).is_some()
}

/// Les modèles de l'objet lit sont centrés sur la jonction du matelas.
pub fn display_parts(registry: &BlockRegistry, block: BlockId) -> Option<[(BlockId, Vec3); 2]> {
    if registry.def(block).key != ITEM {
        return None;
    }
    Some([
        (registry.id(FOOT)?, Vec3::Z * 0.5),
        (registry.id(HEAD)?, Vec3::NEG_Z * 0.5),
    ])
}

/// Pied près du joueur, tête dans la direction horizontale du regard.
pub fn placement(
    registry: &BlockRegistry,
    foot: IVec3,
    look: Vec3,
) -> Option<[(IVec3, BlockId); 2]> {
    let facing = if look.x.abs() > look.z.abs() {
        if look.x > 0.0 { 1 } else { 3 }
    } else if look.z > 0.0 {
        2
    } else {
        0
    };
    let direction = DIRECTIONS[facing];
    let facing = [facing as u8];
    Some([
        (foot, registry.variant(registry.id(FOOT)?, &facing)),
        (
            foot + direction,
            registry.variant(registry.id(HEAD)?, &facing),
        ),
    ])
}

/// Position et identifiant exact attendus de l'autre moitié.
pub fn partner(registry: &BlockRegistry, pos: IVec3, block: BlockId) -> Option<(IVec3, BlockId)> {
    let head = part(registry, block)?;
    let facing = *registry.def(block).state_values.first()?;
    let direction = *DIRECTIONS.get(facing as usize)?;
    let other = registry.id(if head { FOOT } else { HEAD })?;
    Some((
        pos + if head { -direction } else { direction },
        registry.variant(other, &[facing]),
    ))
}

/// Le point de réapparition est commun aux deux moitiés du même lit.
pub fn head_at(world: &World, pos: IVec3) -> Option<IVec3> {
    let registry = world.registry();
    let block = world.block(pos);
    if registry.def(block).key == ITEM {
        return Some(pos);
    }
    let (other, expected) = partner(registry, pos, block)?;
    (world.block(other) == expected).then_some(if part(registry, block)? { pos } else { other })
}

/// Les grilles mobiles tournent de `k` quarts de tour autour de Y avant d'atterrir.
pub(crate) fn turned(registry: &BlockRegistry, block: BlockId, k: u8) -> BlockId {
    if is_part(registry, block) {
        let facing = registry.def(block).state_values[0];
        registry.variant(block, &[(facing + k) % 4])
    } else {
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_agree_in_all_directions_and_follow_ship_rotation() {
        let registry = BlockRegistry::builtin();
        let foot = IVec3::new(-4, 64, 32);
        for direction in DIRECTIONS {
            let [a, b] = placement(&registry, foot, direction.as_vec3()).unwrap();
            assert_eq!((a.0, b.0), (foot, foot + direction));
            assert_eq!(partner(&registry, a.0, a.1), Some(b));
            assert_eq!(partner(&registry, b.0, b.1), Some(a));
            for k in 0..4 {
                let top = foot
                    + crate::rotate_y(
                        direction.as_vec3(),
                        f32::from(k) * std::f32::consts::FRAC_PI_2,
                    )
                    .round()
                    .as_ivec3();
                assert_eq!(
                    partner(&registry, foot, turned(&registry, a.1, k)),
                    Some((top, turned(&registry, b.1, k)))
                );
            }
        }
    }
}
