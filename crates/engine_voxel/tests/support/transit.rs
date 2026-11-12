//! Réseaux de contrôle en mémoire, sans lire ni écrire de monde sauvegardé.
#![allow(dead_code)]
use engine_voxel::*;
use glam::IVec3;
use std::{cell::RefCell, path::Path, sync::Arc};

pub const ROUTE: [IVec3; 4] = [
    IVec3::new(0, 0, 0),
    IVec3::new(1, 0, 0),
    IVec3::new(1, 0, 1),
    IVec3::new(2, 0, 1),
];
pub const SOURCE: IVec3 = IVec3::new(-1, 0, 0);
pub const SINK: IVec3 = IVec3::new(3, 0, 1);
pub struct Rig {
    pub blocks: Arc<BlockRegistry>,
    pub fluids: FluidRegistry,
    pub world: RefCell<ChunkStreamer>,
    pub grids: SubGrids,
    pub networks: Networks,
    pub ids: Vec<GridId>,
}
impl Rig {
    pub fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut blocks =
            BlockRegistry::load(&root.join("assets"), Some(&root.join("mods"))).unwrap();
        blocks.add_source("review", r#"[
          (id:"review:items",name:"Transporteur vitré",color:(0.3,0.4,0.4),texture:"core:pipe",conduit:Items,connections:(core:8,arm:6)),
          (id:"review:one_way",name:"Transporteur orienté",color:(0.3,0.4,0.4),texture:"core:pipe",conduit:Items,boxes:[(from:(4,4,0),to:(12,12,16))],states:[Facing6],flow:true)
        ]"#, "test transport").unwrap();
        let blocks = Arc::new(blocks);
        let mut fluids = FluidRegistry::builtin();
        fluids.add_source("review", r#"[(id:"review:vapor",name:"Vapeur",color:(0.7,0.65,0.85,0.4),viscosity:1,density:-1,temperature:400)]"#, "test gaz").unwrap();
        let mut grids = SubGrids::new(blocks.clone());
        let mut ids = Vec::new();
        for content in ["core:water", "review:vapor", "items"] {
            let id = grids.create();
            ids.push(id);
            let mut grid = grids.get(id).unwrap().borrow_mut();
            let pipe = blocks
                .id(if content == "items" {
                    "review:items"
                } else {
                    "core:pipe"
                })
                .unwrap();
            for p in ROUTE {
                grid.set_block(p, pipe);
            }
            for (pos, producer) in [(SOURCE, true), (SINK, false)] {
                grid.set_block(
                    pos,
                    blocks
                        .id(if content == "items" {
                            "core:chest"
                        } else {
                            "core:tank"
                        })
                        .unwrap(),
                );
                let mut e = BlockEntity::default();
                if content == "items" {
                    let mut inv = Inventory::new(2);
                    inv.sides = if producer {
                        Sides::ALL.with_io(Io::Out)
                    } else {
                        Sides::ALL.with_io(Io::In)
                    };
                    if producer {
                        inv.slots[0] = Some(engine_voxel::ItemStack {
                            item: "core:iron_ingot".into(),
                            count: 64,
                            damage: 0,
                        });
                    }
                    e.caps_mut().inventory = Some(inv);
                } else {
                    let mut tank = FluidTank::new(100_000);
                    tank.sides = if producer {
                        Sides::ALL.with_io(Io::Out)
                    } else {
                        Sides::ALL.with_io(Io::In)
                    };
                    if producer {
                        tank.fluid = Some(content.into());
                        tank.amount = 100_000;
                    }
                    e.caps_mut().tanks.push(tank);
                }
                grid.set_block_entity(pos, Some(e));
            }
        }
        let world = RefCell::new(ChunkStreamer::new(
            TerrainGenerator::new(7),
            StreamingConfig::default(),
        ));
        let mut networks = Networks::new(&blocks);
        let dirty: Vec<_> = ids
            .iter()
            .flat_map(|&id| ROUTE.map(|p| GridPos::new(id, p)))
            .collect();
        networks.rebuild(Grids::with_subgrids(&world, &grids), &dirty);
        Self {
            blocks,
            fluids,
            world,
            grids,
            networks,
            ids,
        }
    }
    pub fn tick(&mut self) {
        self.networks
            .tick(Grids::with_subgrids(&self.world, &self.grids));
    }
    pub fn cut(&mut self, id: GridId, pos: IVec3) {
        self.grids
            .get(id)
            .unwrap()
            .borrow_mut()
            .set_block(pos, BlockId::AIR);
        self.networks.rebuild(
            Grids::with_subgrids(&self.world, &self.grids),
            &[GridPos::new(id, pos)],
        );
    }
}
