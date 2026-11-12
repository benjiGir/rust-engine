mod support {
    pub mod transit;
}
use engine_voxel::*;
use glam::{IVec3, Vec3};
use support::transit::*;

#[test]
fn traces_describe_real_item_liquid_and_gas_transfers_through_a_bend() {
    let mut r = Rig::new();
    for _ in 0..4 {
        r.tick();
    }
    let transfers = r.networks.take_visual_transfers();
    for (index, content) in [
        TransitContent::Fluid("core:water".into()),
        TransitContent::Fluid("review:vapor".into()),
        TransitContent::Item("core:iron_ingot".into()),
    ]
    .into_iter()
    .enumerate()
    {
        let events: Vec<_> = transfers
            .iter()
            .filter(|e| e.grid == r.ids[index])
            .collect();
        assert!(!events.is_empty());
        assert!(events.iter().all(|e| e.content == content && e.amount > 0));
        for event in &events {
            assert_eq!(
                event.path.as_ref(),
                &[SOURCE, ROUTE[0], ROUTE[1], ROUTE[2], ROUTE[3], SINK]
            );
            assert!(
                event
                    .path
                    .windows(2)
                    .all(|p| (p[1] - p[0]).abs().element_sum() == 1)
            );
        }
        let grid = r.grids.get(r.ids[index]).unwrap().borrow();
        let caps = grid.block_entity(SINK).unwrap().caps.as_ref().unwrap();
        let received = if index == 2 {
            u64::from(
                caps.inventory.as_ref().unwrap().slots[0]
                    .as_ref()
                    .unwrap()
                    .count,
            )
        } else {
            u64::from(caps.tanks[0].amount)
        };
        assert_eq!(events.iter().map(|e| e.amount).sum::<u64>(), received);
    }
    assert!(r.networks.take_visual_transfers().is_empty());
}

#[test]
fn no_new_traces_after_a_cut_or_when_receivers_are_full() {
    let mut r = Rig::new();
    for _ in 0..4 {
        r.tick();
    }
    assert!(!r.networks.take_visual_transfers().is_empty());
    for id in r.ids.clone() {
        r.cut(id, ROUTE[1]);
    }
    for _ in 0..4 {
        r.tick();
    }
    assert!(r.networks.take_visual_transfers().is_empty());
    let mut r = Rig::new();
    for id in &r.ids[..2] {
        let mut grid = r.grids.get(*id).unwrap().borrow_mut();
        let caps = grid.block_entity_mut(SINK).unwrap().caps_mut();
        caps.tanks[0].fluid = Some(
            if *id == r.ids[0] {
                "core:water"
            } else {
                "review:vapor"
            }
            .into(),
        );
        caps.tanks[0].amount = caps.tanks[0].capacity;
    }
    r.tick();
    assert!(r.networks.take_visual_transfers().is_empty());
}

#[test]
fn direction_changes_invalidate_cached_visual_paths() {
    let mut r = Rig::new();
    let id = r.ids[2];
    {
        let mut grid = r.grids.get(id).unwrap().borrow_mut();
        grid.set_block(
            ROUTE[0],
            r.blocks.id("review:one_way[facing=east]").unwrap(),
        );
    }
    r.networks.rebuild(
        Grids::with_subgrids(&r.world, &r.grids),
        &[GridPos::new(id, ROUTE[0])],
    );
    for _ in 0..4 {
        r.tick();
    }
    assert!(
        r.networks
            .take_visual_transfers()
            .iter()
            .any(|e| e.grid == id)
    );
    r.grids.get(id).unwrap().borrow_mut().set_block(
        ROUTE[0],
        r.blocks.id("review:one_way[facing=west]").unwrap(),
    );
    r.networks.rebuild(
        Grids::with_subgrids(&r.world, &r.grids),
        &[GridPos::new(id, ROUTE[0])],
    );
    for _ in 0..4 {
        r.tick();
    }
    assert!(
        !r.networks
            .take_visual_transfers()
            .iter()
            .any(|e| e.grid == id)
    );
}

#[test]
fn hollow_rendering_preserves_physical_boxes_and_has_transparent_windows() {
    let r = Rig::new();
    let mut world = World::with_registry(r.blocks.clone());
    world.set_block(IVec3::ZERO, r.blocks.id("core:pipe").unwrap());
    let boxes = world.block_boxes(IVec3::ZERO);
    let shell = conduit_shell(r.blocks.def(world.block(IVec3::ZERO)), &boxes).unwrap();
    assert_eq!(boxes.len(), 1);
    assert_eq!(shell.windows.len(), 1);
    assert_eq!(shell.rails.len(), 12);
    assert!(
        shell
            .rails
            .iter()
            .all(|b| b.min.cmpge(boxes[0].min).all() && b.max.cmple(boxes[0].max).all())
    );
    assert!(
        !shell
            .rails
            .iter()
            .any(|b| Vec3::splat(0.5).cmpgt(b.min).all() && Vec3::splat(0.5).cmplt(b.max).all())
    );
    let mesh = mesh_greedy(&world, IVec3::ZERO);
    assert!(!mesh.translucent_indices.is_empty());
    assert!(!mesh.indices.is_empty());
    assert_eq!(world.block_boxes(IVec3::ZERO).as_ref(), boxes.as_ref());
}

#[test]
fn observation_queue_is_bounded_and_never_limits_the_real_transfer() {
    let mut r = Rig::new();
    for _ in 0..220 {
        r.tick();
    }
    assert!(r.networks.take_visual_transfers().len() <= 256);
    for id in &r.ids[..2] {
        let grid = r.grids.get(*id).unwrap().borrow();
        assert_eq!(
            grid.block_entity(SINK)
                .unwrap()
                .caps
                .as_ref()
                .unwrap()
                .tanks[0]
                .amount,
            100_000
        );
    }
}

#[test]
fn liquid_trace_leaves_by_the_face_of_the_tank_actually_used() {
    let mut r = Rig::new();
    let id = r.ids[0];
    {
        let mut grid = r.grids.get(id).unwrap().borrow_mut();
        for p in [SOURCE + IVec3::Z, IVec3::Z] {
            grid.set_block(p, r.blocks.id("core:pipe").unwrap());
        }
        let caps = grid.block_entity_mut(SOURCE).unwrap().caps_mut();
        caps.tanks[0].sides = Sides(1 << Face::PosZ as u8).with_io(Io::Out);
        let mut other = FluidTank::new(100_000);
        other.fluid = Some("core:lava".into());
        other.amount = 5000;
        other.sides = Sides(1 << Face::PosX as u8).with_io(Io::Out);
        caps.tanks.push(other);
    }
    r.networks.rebuild(
        Grids::with_subgrids(&r.world, &r.grids),
        &[GridPos::new(id, SOURCE)],
    );
    r.tick();
    let events: Vec<_> = r
        .networks
        .take_visual_transfers()
        .into_iter()
        .filter(|e| e.grid == id)
        .collect();
    assert!(!events.is_empty());
    for e in events {
        assert_eq!(e.content, TransitContent::Fluid("core:water".into()));
        assert_eq!(
            e.path[1],
            SOURCE + IVec3::Z,
            "ne pas dessiner l'eau sur la sortie réservée à la lave"
        );
    }
}
