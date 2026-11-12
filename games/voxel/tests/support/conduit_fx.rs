use super::*;
#[path = "../../../../crates/engine_voxel/tests/support/transit.rs"]
mod fixture;
use fixture::{ROUTE, Rig};

fn effects() -> (Rig, ConduitFx, ItemRegistry) {
    let mut rig = Rig::new();
    for _ in 0..4 {
        rig.tick();
    }
    let items = ItemRegistry::builtin(&rig.blocks).unwrap();
    let mut fx = ConduitFx::default();
    fx.ingest(rig.networks.take_visual_transfers());
    (rig, fx, items)
}
fn draw(fx: &mut ConduitFx, rig: &Rig, items: &ItemRegistry, index: usize) -> Vec<CubeInstance> {
    let grid = rig.grids.get(rig.ids[index]).unwrap().borrow();
    let mut out = Vec::new();
    fx.instances(
        rig.ids[index],
        &*grid,
        items,
        &rig.fluids,
        Vec3::ZERO,
        |p| p,
        0.0,
        &mut out,
        8192,
    );
    out
}

#[test]
fn actual_item_model_moves_and_disappears_when_the_route_breaks() {
    let (mut rig, mut fx, items) = effects();
    let first = draw(&mut fx, &rig, &items, 2);
    assert!(
        first.len() > 1,
        "un objet utilise ses pixels extrudés, pas un cube de couleur"
    );
    fx.advance(0.35);
    let next = draw(&mut fx, &rig, &items, 2);
    assert!(!next.is_empty());
    assert_ne!(first[0].center, next[0].center);
    rig.cut(rig.ids[2], ROUTE[1]);
    assert!(draw(&mut fx, &rig, &items, 2).is_empty());
    fx.clear_models();
    assert!(fx.models.is_empty());
    let (rig, mut fx, items) = effects();
    rig.grids
        .get(rig.ids[2])
        .unwrap()
        .borrow_mut()
        .set_block(fixture::SINK, engine_voxel::BlockId::AIR);
    assert!(
        draw(&mut fx, &rig, &items, 2).is_empty(),
        "une machine retirée ne reçoit plus de paquet visible"
    );
}

#[test]
fn fluid_and_gas_use_the_registered_color_and_fade_when_transfers_stop() {
    let (rig, mut fx, items) = effects();
    let water = draw(&mut fx, &rig, &items, 0);
    let gas = draw(&mut fx, &rig, &items, 1);
    assert!(!water.is_empty() && !gas.is_empty());
    assert_eq!(water[0].color, [0.2, 0.4, 0.9]);
    assert_eq!(gas[0].color, [0.7, 0.65, 0.85]);
    assert!(gas[0].opacity < water[0].opacity);
    fx.advance(0.41);
    assert!(draw(&mut fx, &rig, &items, 0).is_empty());
    assert!(draw(&mut fx, &rig, &items, 1).is_empty());
}

#[test]
fn instances_follow_mobile_grid_transforms_and_respect_distance_and_budget() {
    let (rig, mut fx, items) = effects();
    let original = draw(&mut fx, &rig, &items, 2);
    let grid = rig.grids.get(rig.ids[2]).unwrap().borrow();
    let shift = Vec3::new(7.0, 2.0, 4.0);
    let mut moved = Vec::new();
    fx.instances(
        rig.ids[2],
        &*grid,
        &items,
        &rig.fluids,
        Vec3::ZERO,
        |p| shift + engine_voxel::rotate_y(p, 0.8),
        0.8,
        &mut moved,
        8192,
    );
    assert_eq!(original.len(), moved.len());
    for (a, b) in original.iter().zip(&moved) {
        let expected = shift + engine_voxel::rotate_y(Vec3::from_array(a.center), 0.8);
        assert!((Vec3::from_array(b.center) - expected).length() < 0.0001);
        assert!((b.yaw - a.yaw - 0.8).abs() < 0.0001);
    }
    let mut hidden = Vec::new();
    fx.instances(
        rig.ids[2],
        &*grid,
        &items,
        &rig.fluids,
        Vec3::splat(1000.0),
        |p| p,
        0.0,
        &mut hidden,
        8192,
    );
    assert!(hidden.is_empty());
    fx.instances(
        rig.ids[2],
        &*grid,
        &items,
        &rig.fluids,
        Vec3::ZERO,
        |p| p,
        0.0,
        &mut hidden,
        1,
    );
    assert_eq!(hidden.len(), 1);
}
