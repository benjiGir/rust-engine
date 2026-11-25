//! Cartes découpées des packs détaillés. Les plis donnent du volume et déplacent les
//! sommets au vent, tandis que les UV restent attachés à la feuille.

use glam::{IVec3, Vec3};

use super::{ChunkMesh, FLAG_CUTOUT, FLAG_PLANT, FLAG_PLANT_UV, FLAG_SWAY};
use crate::registry::Shape;

#[derive(Clone, Copy)]
pub(super) struct Plant {
    width: f32,
    height: f32,
    bend: f32,
    panels: u8,
    rounded_crown: bool,
}

impl Plant {
    const fn new(width: f32, height: f32, bend: f32, panels: u8) -> Self {
        Self {
            width,
            height,
            bend,
            panels,
            rounded_crown: false,
        }
    }

    const fn berry_bush(stage: u8) -> Self {
        let (width, height) = if stage == 0 {
            (0.88, 0.68)
        } else {
            (1.48, 1.13)
        };
        Self {
            width,
            height,
            bend: 0.17,
            panels: 4,
            rounded_crown: true,
        }
    }
}

pub(super) fn profile(key: &str, shape: Shape) -> Option<Plant> {
    let name = key.strip_prefix("core:")?;
    if shape == Shape::Flat {
        return match name {
            "lily_pad" => Some(Plant::new(0.82, 0.045, 0.0, 0)),
            "pink_petals" => Some(Plant::new(0.73, 0.023, 0.0, 0)),
            _ => None,
        };
    }
    if shape != Shape::Cross {
        return None;
    }
    let plant = match name {
        "tall_grass" => Plant::new(0.87, 0.86, 0.17, 2),
        "fern" => Plant::new(0.87, 0.64, 0.25, 3),
        "tall_grass_double" | "tall_grass_double_top" => Plant::new(0.84, 1.0, 0.13, 2),
        "sunflower" | "sunflower_top" => Plant::new(0.83, 1.0, 0.10, 2),
        "poppy" | "tulip" => Plant::new(0.72, 0.86, 0.10, 2),
        "cornflower" | "daisy" | "dandelion" | "marigold" | "allium" => {
            Plant::new(0.77, 0.72, 0.13, 2)
        }
        "lavender" => Plant::new(1.06, 1.13, 0.16, 3),
        "heather" => Plant::new(0.86, 0.67, 0.14, 3),
        "brown_mushroom" | "red_mushroom" | "glowcap" => Plant::new(0.73, 0.59, 0.05, 2),
        "dead_bush" => Plant::new(0.79, 0.65, 0.12, 3),
        "vine" => Plant::new(0.90, 1.0, 0.09, 2),
        "seaweed" => Plant::new(0.77, 0.90, 0.19, 2),
        "oak_sapling" | "birch_sapling" | "spruce_sapling" | "acacia_sapling"
        | "jungle_sapling" | "redwood_sapling" | "maple_sapling" | "fir_sapling"
        | "olive_sapling" | "cherry_sapling" => Plant::new(0.82, 0.91, 0.08, 3),
        _ => {
            let (species, stage) = name.rsplit_once('_')?;
            let stage: u8 = stage.parse().ok()?;
            match species {
                "carrot" | "potato" if stage < 4 => Plant::new(0.82, 0.91, 0.19, 2),
                "cabbage" | "melon" if stage < 4 => Plant::new(0.86, 0.84, 0.24, 3),
                "sweet_berry_bush" | "blueberry_bush" if stage < 3 => Plant::berry_bush(stage),
                _ => return None,
            }
        }
    };
    Some(plant)
}

#[derive(Clone, Copy)]
pub(super) struct Cereal {
    stage: usize,
    height_scale: f32,
    wild: bool,
}

pub(super) fn cereal_profile(key: &str) -> Option<Cereal> {
    let name = key.strip_prefix("core:")?;
    let (species, stage, wild) = if let Some(species) = name.strip_prefix("wild_") {
        (species, 3, true)
    } else {
        let (species, stage) = name.rsplit_once('_')?;
        (species, stage.parse::<usize>().ok()?, false)
    };
    let height_scale = match species {
        "wheat" => 1.0,
        "barley" => 1.02,
        "oats" => 1.11,
        _ => return None,
    };
    (stage < 4).then_some(Cereal {
        stage,
        height_scale,
        wild,
    })
}

/// Plusieurs pieds distincts par case, en deux rangs dans les champs et
/// irrégulièrement espacés dans les graminées sauvages.
pub(super) fn cereal(
    mesh: &mut ChunkMesh,
    crop: Cereal,
    cell: IVec3,
    tint: [f32; 4],
    light: [f32; 3],
    texture: u32,
) {
    let base = cell.as_vec3();
    let height = [0.27, 0.51, 0.87, 1.09][crop.stage] * crop.height_scale;
    let spacing = if crop.wild { 0.15 } else { 0.049 };
    let sways = texture & FLAG_SWAY != 0;
    let offsets = [
        (0.26, 0.17),
        (0.74, 0.5),
        (0.26, 0.83),
        (0.74, 0.17),
        (0.26, 0.50),
        (0.74, 0.83),
    ];
    for (i, &(x, z)) in offsets[..[3, 4, 5, 6][crop.stage]].iter().enumerate() {
        let hash = hash_cell(cell + IVec3::new(13 * i as i32 + 1, 251, -37 * i as i32));
        let sample = |shift| ((hash >> shift) & 255u32) as f32 / 255.0;
        let root = Vec3::new(
            x + (2.0 * sample(0) - 1.0) * spacing,
            0.0,
            z + (2.0 * sample(8) - 1.0) * spacing,
        );
        let height = height * (0.85 + 0.28 * sample(16));
        let yaw = sample(24) * std::f32::consts::TAU;
        let lean = Vec3::new(yaw.cos(), 0.0, yaw.sin()) * height * (0.06 + 0.09 * sample(0));
        let width = [0.255, 0.26, 0.29, 0.33][crop.stage] * (0.93 + 0.12 * sample(8));
        let u = ((hash >> 12) as u16 & 3) * 32;
        for direction in [yaw, yaw + std::f32::consts::FRAC_PI_2] {
            let half = Vec3::new(direction.cos(), 0.0, direction.sin()) * (width * 0.5);
            let row = |fraction: f32| {
                let bend = fraction * fraction;
                let center = root + Vec3::Y * (height * fraction) + lean * bend;
                let v = ((1.0 - fraction) * 128.0).round() as u16;
                let sway = if sways {
                    bend * height.min(1.0) * 0.82
                } else {
                    0.0
                };
                [
                    (center - half, [u, v], sway),
                    (center + half, [u + 32, v], sway),
                ]
            };
            let [a, b] = row(0.0);
            let [d, c] = row(0.5);
            let [f, e] = row(1.0);
            write_face(mesh, base, [a, b, c, d], tint, light, texture);
            write_face(mesh, base, [d, c, e, f], tint, light, texture);
        }
    }
}

pub(super) fn mesh(
    mesh: &mut ChunkMesh,
    key: &str,
    plant: Plant,
    cell: IVec3,
    tint: [f32; 4],
    light: [f32; 3],
    texture: u32,
) {
    let both = matches!(
        key,
        "core:tall_grass_double"
            | "core:tall_grass_double_top"
            | "core:sunflower"
            | "core:sunflower_top"
    );
    let upper = both && key.ends_with("_top");
    let vine = key == "core:vine";
    let root = if vine {
        IVec3::new(cell.x, 0, cell.z)
    } else {
        cell - if upper { IVec3::Y } else { IVec3::ZERO }
    };
    let hash = hash_cell(root);
    let angle = std::f32::consts::TAU * (hash & 1023) as f32 / 1024.0;
    let base = cell.as_vec3();
    let center = Vec3::new(
        0.5 + ((((hash >> 10) & 31) as f32) / 31.0 - 0.5) * 0.07,
        0.0,
        0.5 + ((((hash >> 15) & 31) as f32) / 31.0 - 0.5) * 0.07,
    );

    if plant.panels == 0 {
        let (sin, cos) = angle.sin_cos();
        let x = Vec3::new(cos, 0.0, sin) * (plant.width * 0.5);
        let z = Vec3::new(-sin, 0.0, cos) * (plant.width * 0.5);
        // Les bords du nénuphar suivent la surface, sa nervure centrale est relevée.
        let points = [
            center - x - z,
            center + x - z,
            center + x + z,
            center - x + z,
        ];
        let crown = center + Vec3::Y * plant.height;
        let edge = Vec3::Y * 0.02;
        let uv = [[0, 0], [128, 0], [128, 128], [0, 128]];
        for i in 0..4 {
            let j = (i + 1) % 4;
            write_face(
                mesh,
                base,
                [
                    (points[i] + edge, uv[i], 0.0),
                    (crown, [64, 64], 0.0),
                    (points[j] + edge, uv[j], 0.0),
                ],
                tint,
                light,
                texture,
            );
        }
        return;
    }

    let start_height = f32::from(upper);
    let scale = if both || vine {
        1.0
    } else {
        0.87 + ((hash >> 20) & 15) as f32 * (0.17 / 15.0)
    };
    let sway = if texture & FLAG_SWAY != 0 { 1.0 } else { 0.0 };
    for panel in 0..plant.panels {
        let yaw = angle + std::f32::consts::PI * f32::from(panel) / f32::from(plant.panels);
        let (sin, cos) = yaw.sin_cos();
        let across = Vec3::new(cos, 0.0, sin);
        let forward = Vec3::new(-sin, 0.0, cos);
        let width = plant.width * if panel == 0 { 0.5 } else { 0.45 };
        let row = |fraction: f32| {
            let height = start_height + fraction;
            let arch = (std::f32::consts::PI * fraction).sin().max(0.0);
            let bend = if vine {
                arch
            } else {
                height * height / (1.0 + height)
            };
            let lean = if vine { 0.0 } else { 0.38 * height };
            let spine = center
                + Vec3::Y * (fraction * plant.height * scale)
                + forward * (plant.bend * bend)
                + across * (plant.bend * lean);
            let flexibility = if vine {
                arch * 0.42
            } else if both {
                height * height * 0.22
            } else if plant.rounded_crown {
                fraction * fraction * 0.48
            } else {
                fraction * fraction * 0.73
            };
            // La souche est resserrée et le feuillage déborde de tous les côtés.
            let spread = if plant.rounded_crown {
                0.59 + 0.66 * (std::f32::consts::PI * fraction * 0.86).sin()
            } else {
                1.0
            };
            let half = across * width * spread;
            let v = ((1.0 - fraction) * 128.0) as u16;
            [
                (spine - half, [0, v], flexibility * sway),
                (spine + half, [128, v], flexibility * sway),
            ]
        };
        let top = if key == "core:sunflower_top" && panel == 1 {
            // La face de la fleur reste unique. La seconde carte soutient la tige.
            0.1875
        } else {
            1.0
        };
        let rows: &[f32] = if plant.rounded_crown {
            &[0.0, 0.33, 0.68, 1.0]
        } else {
            &[0.0, 0.5, 1.0]
        };
        for section in rows.windows(2) {
            let [a, b] = row(top * section[0]);
            let [d, c] = row(top * section[1]);
            write_face(mesh, base, [a, b, c, d], tint, light, texture);
        }
    }
}

/// Tige côtelée. Les tronçons empilés se rejoignent sans face interne.
pub(super) fn cactus(
    mesh: &mut ChunkMesh,
    base: Vec3,
    tint: [f32; 4],
    light: [f32; 3],
    layers: [u32; 3],
    joined: [bool; 2],
) {
    let [above, below] = joined;
    const FACETS: usize = 16;
    let radial: [Vec3; FACETS] = std::array::from_fn(|i| {
        let angle = std::f32::consts::TAU * i as f32 / FACETS as f32;
        let radius = if i.is_multiple_of(2) { 0.43 } else { 0.34 };
        Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
    });
    let center = Vec3::new(0.5, 0.0, 0.5);
    let lower = radial.map(|p| center + p);
    let shoulder = radial.map(|p| center + p + Vec3::Y * if above { 1.0 } else { 0.87 });
    let crown = radial.map(|p| center + p * 0.77 + Vec3::Y * 0.98);
    let cap_uv = |p: Vec3| [(p.x * 128.0).round() as u16, (p.z * 128.0).round() as u16];
    for i in 0..FACETS {
        let j = (i + 1) % FACETS;
        let u = i as u16 * 8;
        let v = if above { 0 } else { 16 };
        write_face(
            mesh,
            base,
            [
                (lower[i], [u, 128], 0.0),
                (shoulder[i], [u, v], 0.0),
                (shoulder[j], [u + 8, v], 0.0),
                (lower[j], [u + 8, 128], 0.0),
            ],
            tint,
            light,
            layers[1],
        );
        if !above {
            write_face(
                mesh,
                base,
                [
                    (shoulder[i], [u, 16], 0.0),
                    (crown[i], [u, 0], 0.0),
                    (crown[j], [u + 8, 0], 0.0),
                    (shoulder[j], [u + 8, 16], 0.0),
                ],
                tint,
                light,
                layers[1],
            );
            write_face(
                mesh,
                base,
                [
                    (center + Vec3::Y * 0.98, [64; 2], 0.0),
                    (crown[j], cap_uv(crown[j]), 0.0),
                    (crown[i], cap_uv(crown[i]), 0.0),
                ],
                tint,
                light,
                layers[0],
            );
        }
        if !below {
            write_face(
                mesh,
                base,
                [
                    (center, [64; 2], 0.0),
                    (lower[i], cap_uv(lower[i]), 0.0),
                    (lower[j], cap_uv(lower[j]), 0.0),
                ],
                tint,
                light,
                layers[2],
            );
        }
    }
}

type Point = (Vec3, [u16; 2], f32);

fn write_face<const N: usize>(
    mesh: &mut ChunkMesh,
    base: Vec3,
    points: [Point; N],
    tint: [f32; 4],
    light: [f32; 3],
    texture: u32,
) {
    let start = mesh.vertices.len() as u32;
    let normal = (points[1].0 - points[0].0)
        .cross(points[2].0 - points[0].0)
        .normalize();
    for (position, [u, v], sway) in points {
        let flags = FLAG_PLANT_UV | if sway > 0.0 { FLAG_PLANT } else { 0 };
        mesh.push_vertex(
            base + position,
            normal.to_array(),
            tint,
            light,
            texture | flags,
        );
        let vertex = mesh.vertices.last_mut().unwrap();
        vertex.position[3] = u | (v << 8);
        vertex.light[3] = (sway * 255.0).round() as u8;
    }
    for i in 1..N as u32 - 1 {
        mesh.indices.extend([start, start + i, start + i + 1]);
        if texture & FLAG_CUTOUT != 0 {
            mesh.indices.extend([start, start + i + 1, start + i]);
        }
    }
}

fn hash_cell(p: IVec3) -> u32 {
    let mut n = (p.x as u32).wrapping_mul(0x9E37_79B9)
        ^ (p.y as u32).wrapping_mul(0x85EB_CA6B)
        ^ (p.z as u32).wrapping_mul(0xC2B2_AE35);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7FEB_352D);
    n ^ (n >> 15)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockRegistry, World, mesh_binary, mesh_culled, mesh_greedy};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn world(size: u32) -> World {
        let mut registry = BlockRegistry::builtin();
        Arc::make_mut(&mut registry)
            .set_visual_pack(Some(std::env::temp_dir()), size)
            .unwrap();
        World::with_registry(registry)
    }

    fn place(world: &mut World, key: &str, p: IVec3) {
        world.set_block(p, world.registry().id(key).unwrap());
    }

    #[test]
    fn the_64_pack_uses_folded_fern_cards_with_rooted_wind_in_each_mesher() {
        let p = IVec3::new(10, 4, 4);
        for size in [16, 32, 64] {
            let mut world = world(size);
            place(&mut world, "core:fern", p);
            for mesher in [mesh_greedy, mesh_binary, mesh_culled] {
                let mesh = mesher(&world, IVec3::ZERO);
                if size != 64 {
                    assert_eq!(mesh.vertices.len(), 8);
                    assert!(mesh.vertices.iter().all(|v| v.texture & FLAG_PLANT_UV == 0));
                    continue;
                }
                assert_eq!(mesh.vertices.len(), 24);
                for vertex in &mesh.vertices {
                    assert_ne!(vertex.texture & FLAG_PLANT_UV, 0);
                    let height = mesh.world_position(vertex).y - p.y as f32;
                    let v = vertex.position[3] >> 8;
                    assert_eq!(height == 0.0, v == 128);
                    assert_eq!(height == 0.0, vertex.light[3] == 0);
                }
            }
        }
    }

    #[test]
    fn stacked_vegetation_shares_geometry_and_wind_across_chunks() {
        let p = IVec3::new(-4, crate::CHUNK_SIZE as i32 - 1, -3);
        let lower_chunk = World::chunk_containing(p.as_vec3());
        let border = (p.y + 1) as f32;
        let seam = |mesh: ChunkMesh| -> BTreeSet<_> {
            mesh.vertices
                .iter()
                .filter_map(|v| {
                    let xyz = mesh.world_position(v);
                    (xyz.y == border).then_some((
                        (xyz.x * 1024.0) as i32,
                        (xyz.z * 1024.0) as i32,
                        v.position[3] & 255,
                        v.light[3],
                    ))
                })
                .collect()
        };
        for (lower, upper, vertices) in [
            ("core:sunflower", "core:sunflower_top", 4),
            ("core:tall_grass_double", "core:tall_grass_double_top", 4),
            ("core:vine", "core:vine", 4),
            ("core:cactus", "core:cactus", 17),
        ] {
            let mut world = world(64);
            place(&mut world, lower, p);
            place(&mut world, upper, p + IVec3::Y);
            let lower = seam(mesh_binary(&world, lower_chunk));
            let upper = seam(mesh_binary(&world, lower_chunk + IVec3::Y));
            assert_eq!(lower.len(), vertices);
            assert_eq!(lower, upper);
        }
    }

    #[test]
    fn berry_bushes_keep_their_full_crown_after_harvest() {
        let p = IVec3::new(31, 4, 31);
        for species in ["blueberry_bush", "sweet_berry_bush"] {
            let bounds = [0, 1, 2].map(|stage| {
                let mut world = world(64);
                place(&mut world, &format!("core:{species}_{stage}"), p);
                let mesh = mesh_binary(&world, IVec3::ZERO);
                assert_eq!(mesh.vertices.len(), 48);
                mesh.bounds().unwrap()
            });
            assert_eq!(bounds[1], bounds[2]);
            let (min, max) = bounds[2];
            let span = max - min;
            assert!(span.x > 1.4 && span.z > 1.4 && span.y > 0.95, "{span}");
            let young = bounds[0].1 - bounds[0].0;
            assert!(span.cmpgt(young * 1.5).all());
        }
    }

    #[test]
    fn both_detailed_packs_grow_cereals_as_separate_rooted_stalks() {
        let p = IVec3::new(31, 4, 31);
        for size in [32, 64] {
            for species in ["wheat", "barley", "oats"] {
                let mut last_height = 0.0;
                for (stage, stalks) in [3, 4, 5, 6].into_iter().enumerate() {
                    let mut world = world(size);
                    place(&mut world, &format!("core:{species}_{stage}"), p);
                    let mesh = mesh_binary(&world, IVec3::ZERO);
                    assert_eq!(mesh.vertices.len(), stalks * 16);
                    for stalk in mesh.vertices.as_chunks::<16>().0 {
                        let left = stalk[0].position[3] & 255;
                        assert!(stalk.iter().all(|v| {
                            let u = v.position[3] & 255;
                            v.texture & FLAG_PLANT_UV != 0 && (u == left || u == left + 32)
                        }));
                        assert_eq!(stalk[0].local_position().y, p.y as f32);
                        assert_eq!(stalk[0].light[3], 0);
                        assert!(stalk[6].light[3] > 0);
                    }
                    let height = mesh.bounds().unwrap().1.y - p.y as f32;
                    assert!(height > last_height);
                    last_height = height;
                }
            }
        }
    }
}
