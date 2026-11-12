//! Contenu visible des conduits : traces réelles, interpolation locale et modèles texturés.
use engine_render::CubeInstance;
use engine_voxel::{
    BlockAccess, BlockDef, FluidRegistry, GridId, ItemId, ItemRegistry, NetworkKind,
    TransitContent, VisualTransfer,
};
use glam::{IVec3, Vec3};
use std::collections::HashMap;

const MAX_ACTIVE: usize = 256;
const DISTANCE: f32 = 64.0;

struct Flight {
    transfer: VisualTransfer,
    age: f32,
    duration: f32,
}
#[derive(Default)]
pub struct ConduitFx {
    flights: Vec<Flight>,
    models: HashMap<ItemId, Vec<CubeInstance>>,
    time: f32,
}

fn kind(def: &BlockDef) -> Option<NetworkKind> {
    def.conduit
        .or_else(|| def.behavior.as_deref().and_then(NetworkKind::of_behavior))
}

fn valid(transfer: &VisualTransfer, grid: &(impl BlockAccess + ?Sized)) -> bool {
    if transfer.path.len() < 3 {
        return false;
    }
    let expected = match transfer.content {
        TransitContent::Item(_) => NetworkKind::Items,
        TransitContent::Fluid(_) => NetworkKind::Fluid,
    };
    for (end, neighbour) in [
        (transfer.path[0], transfer.path[1]),
        (
            transfer.path[transfer.path.len() - 1],
            transfer.path[transfer.path.len() - 2],
        ),
    ] {
        let Some(face) = engine_voxel::FACINGS
            .iter()
            .position(|&d| end + d == neighbour)
            .and_then(|i| engine_voxel::Face::from_index(i as i32))
        else {
            return false;
        };
        let Some(caps) = grid.block_entity(end).and_then(|e| e.caps.as_deref()) else {
            return false;
        };
        let open = match expected {
            NetworkKind::Items => caps
                .inventory
                .as_ref()
                .is_some_and(|i| i.sides.contains(face)),
            NetworkKind::Fluid => caps.tanks.iter().any(|t| t.sides.contains(face)),
            _ => false,
        };
        if !open {
            return false;
        }
    }
    let mut channel = None;
    for (i, &p) in transfer
        .path
        .iter()
        .enumerate()
        .skip(1)
        .take(transfer.path.len() - 2)
    {
        let def = grid.registry().def(grid.block(p));
        if kind(def) != Some(expected) {
            return false;
        }
        if let Some(previous) = channel
            && previous != def.channel.as_deref()
        {
            return false;
        }
        channel = Some(def.channel.as_deref());
        if def
            .flow
            .is_some_and(|d| p + d != transfer.path[i + 1] || p + d == transfer.path[i - 1])
        {
            return false;
        }
    }
    true
}

fn point(p: IVec3, grid: &(impl BlockAccess + ?Sized), item: bool) -> Vec3 {
    let def = grid.registry().def(grid.block(p));
    let mut center = p.as_vec3() + Vec3::splat(0.5);
    // Une courroie est un support bas : la pile se pose au-dessus, pas sous son tapis.
    if item && kind(def) == Some(NetworkKind::Items) && def.connections.is_none() {
        let top = def.boxes.iter().map(|b| b.max.y).fold(0.0f32, f32::max);
        if top > 0.0 && top < 0.5 {
            center.y = p.y as f32 + top + 0.13;
        }
    }
    center
}

impl ConduitFx {
    pub fn clear_models(&mut self) {
        self.models.clear();
    }
    pub fn advance(&mut self, dt: f32) {
        self.time += dt;
        for f in &mut self.flights {
            f.age += dt;
        }
        self.flights.retain(|f| f.age < f.duration);
    }
    pub fn ingest(&mut self, transfers: Vec<VisualTransfer>) {
        for transfer in transfers {
            if transfer.path.len() < 3 || transfer.amount == 0 {
                continue;
            }
            let fluid = matches!(transfer.content, TransitContent::Fluid(_));
            if fluid
                && let Some(f) = self.flights.iter_mut().find(|f| {
                    f.transfer.grid == transfer.grid
                        && f.transfer.content == transfer.content
                        && f.transfer.path == transfer.path
                })
            {
                f.age = 0.0;
                f.transfer.amount = transfer.amount;
                continue;
            }
            if self.flights.len() >= MAX_ACTIVE {
                self.flights.remove(0);
            }
            let duration = if fluid {
                0.4
            } else {
                ((transfer.path.len() - 1) as f32 / 4.0).clamp(0.4, 8.0)
            };
            self.flights.push(Flight {
                transfer,
                age: 0.0,
                duration,
            });
        }
    }

    /// Dessin dans une grille, puis application de sa transformation dans le monde.
    #[allow(clippy::too_many_arguments)]
    pub fn instances(
        &mut self,
        id: GridId,
        grid: &(impl BlockAccess + ?Sized),
        items: &ItemRegistry,
        fluids: &FluidRegistry,
        eye: Vec3,
        to_world: impl Fn(Vec3) -> Vec3,
        yaw: f32,
        out: &mut Vec<CubeInstance>,
        limit: usize,
    ) {
        for flight in &self.flights {
            if out.len() >= limit {
                return;
            }
            let transfer = &flight.transfer;
            if transfer.grid != id || !valid(transfer, grid) {
                continue;
            }
            let blocks = grid.registry();
            let mut push = |mut part: CubeInstance| {
                if out.len() >= limit {
                    return;
                }
                let local = Vec3::from_array(part.center);
                let center = to_world(local);
                if center.distance_squared(eye) > DISTANCE * DISTANCE {
                    return;
                }
                part.center = center.to_array();
                part.yaw += yaw;
                out.push(part);
            };
            match &transfer.content {
                TransitContent::Item(key) => {
                    let Some(item) = items.id(key) else {
                        continue;
                    };
                    let t = (flight.age / flight.duration).clamp(0.0, 0.9999)
                        * (transfer.path.len() - 1) as f32;
                    let index = t.floor() as usize;
                    let a = point(transfer.path[index], grid, true);
                    let b = point(transfer.path[index + 1], grid, true);
                    let center = a.lerp(b, t.fract());
                    let size = 0.24;
                    let parts = self
                        .models
                        .entry(item)
                        .or_insert_with(|| crate::item_models::build(item, items, blocks));
                    for part in parts {
                        let mut part = *part;
                        part.center = (center + Vec3::from_array(part.center) * size).to_array();
                        part.size = (Vec3::from_array(part.size) * size).to_array();
                        push(part);
                    }
                }
                TransitContent::Fluid(key) => {
                    let Some(fluid) = fluids.get(key) else {
                        continue;
                    };
                    let gas = fluid.density < 0;
                    let color = Vec3::new(fluid.color.0, fluid.color.1, fluid.color.2);
                    let fade = (1.0 - (flight.age - 0.2).max(0.0) / 0.2).clamp(0.0, 1.0);
                    let height = if gas {
                        0.22
                    } else {
                        0.09 + 0.13 * (transfer.amount as f32 / 500.0).min(1.0)
                    };
                    for (index, pair) in transfer.path.windows(2).enumerate() {
                        let a = point(pair[0], grid, false);
                        let b = point(pair[1], grid, false);
                        let mut center = (a + b) * 0.5;
                        let mut size = (b - a).abs() + Vec3::splat(0.22);
                        if pair[0].y == pair[1].y {
                            size.y = height;
                            center.y -= (0.22 - height) * 0.5;
                        }
                        let mut volume = CubeInstance::new(center, size, color);
                        volume.light[1] = fluid.light_emission as f32 / 15.0;
                        volume.opacity = if gas { 0.22 * fade } else { 0.8 * fade };
                        push(volume);
                        // Un reflet plus clair avance dans le sens observé du transfert.
                        let progress = (self.time * 2.0 - index as f32 * 0.31).rem_euclid(1.0);
                        let mut pulse = CubeInstance::new(
                            a.lerp(b, progress),
                            Vec3::splat(if gas { 0.07 } else { 0.12 }),
                            color.lerp(Vec3::ONE, 0.35),
                        );
                        pulse.light[1] = fluid.light_emission as f32 / 15.0;
                        pulse.opacity = 0.7 * fade;
                        push(pulse);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/conduit_fx.rs"]
mod tests;
