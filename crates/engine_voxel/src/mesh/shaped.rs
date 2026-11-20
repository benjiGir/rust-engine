//! Le maillage d'une case de formes (`core:shaped`) : ses pièces, une à une, avec **les textures de
//! leur matériau**.
//!
//! - Une **couche** ou un **microbloc** est une ou plusieurs boîtes, dessinées comme celles d'un
//!   modèle. Une face n'est pas dessinée si elle est cachée : contre une autre pièce opaque de la
//!   case (l'occupation au seizième), ou contre un voisin opaque au bord de la case.
//! - Une **pente** est un vrai plan incliné : la face inclinée (un quad dont la normale penche), le
//!   mur du bord haut, le dessous, et deux **triangles** de côté. Le shader tire déjà ses UV de la
//!   position selon l'axe dominant de la normale : une pente à 45° (ou moins) se texture comme un
//!   dessus, sans rien changer. La collision, elle, suit l'escalier de seizièmes de la pente.
//! - La lumière est celle de la cellule d'à côté (comme les modèles) et il n'y a pas d'occlusion
//!   ambiante fine : une case de formes est petite, souvent contre un mur.
//! - La teinte de biome (herbe, feuillage) et la découpe (cutout) de chaque matériau sont gardées ;
//!   un matériau translucide (verre) va dans la passe translucide. Un matériau que le registre ne
//!   connaît plus (mod retiré) est dessiné en `core:unknown`.

use std::ops::Range;

use glam::{IVec3, Vec3};

use super::{ChunkMesh, FLAG_GROUND, Face, Neighbourhood, block_flags, face_tint};
use crate::block::BlockId;
use crate::chunk::BlockEntity;
use crate::registry::{BlockDef, Tint};
use crate::shaped::{self, Occupancy, Piece, Shaped};

/// Les matériaux de la case, résolus une fois : le bloc et son opacité.
struct Material<'a> {
    id: BlockId,
    def: &'a BlockDef,
    opaque: bool,
}

struct Cell<'a> {
    hood: &'a Neighbourhood,
    /// La cellule, relative au chunk maillé.
    p: IVec3,
    /// Le coin bas de la cellule, en coordonnées du monde.
    base: Vec3,
    own_sky: u8,
    own_light: u8,
    materials: Vec<Material<'a>>,
    /// Ce que remplissent les pièces opaques de la case.
    opaque: Occupancy,
    /// Ce que remplissent les pièces opaques des cases de formes voisines (une par face).
    neighbours: [Option<Occupancy>; 6],
}

/// Dessine la case de formes en `p` (relatif au chunk maillé). `origin` : l'origine du chunk.
pub(super) fn mesh_cell(
    hood: &Neighbourhood,
    mesh: &mut ChunkMesh,
    p: IVec3,
    origin: IVec3,
    entity: Option<&BlockEntity>,
) {
    let Some(shaped) = entity.and_then(|e| Shaped::decode(&e.data)) else {
        return;
    };
    let materials: Vec<Material> = shaped
        .materials
        .iter()
        .map(|name| material(hood, name))
        .collect();
    let (_, own_sky, own_light) = hood.cell(p);
    let neighbours = Face::ALL.map(|face| {
        let q = p + face.normal();
        let block = hood.get(q);
        let def = hood.registry.def(block);
        shaped::is_shaped_block(def).then(|| {
            let other = hood.entity(q).and_then(|e| Shaped::decode(&e.data))?;
            Some(other.occupancy_where(|name| material(hood, name).opaque))
        })?
    });
    let cell = Cell {
        hood,
        p,
        base: (origin + p).as_vec3(),
        own_sky,
        own_light,
        opaque: shaped.occupancy_where(|name| material(hood, name).opaque),
        materials,
        neighbours,
    };
    for part in &shaped.parts {
        let Some(material) = cell.materials.get(usize::from(part.material)) else {
            continue;
        };
        match part.piece {
            Piece::Slope {
                facing,
                ceiling,
                level,
            } => cell.slope(mesh, material, facing, ceiling, level),
            piece => cell.boxes(mesh, material, piece),
        }
    }
}

fn material<'a>(hood: &'a Neighbourhood, name: &str) -> Material<'a> {
    let registry = &hood.registry;
    let id = registry
        .id(name)
        .filter(|&id| shaped::is_shapeable(registry.def(id)))
        .or_else(|| registry.id(shaped::UNKNOWN_MATERIAL))
        .unwrap_or(BlockId::AIR);
    let def = registry.def(id);
    Material {
        id,
        def,
        opaque: !def.transparent && def.alpha >= 1.0 && !def.cutout,
    }
}

impl Cell<'_> {
    /// Les cellules de 1/16 de bloc sont-elles toutes couvertes d'une pièce opaque, de l'autre côté
    /// du plan `plane` (en seizièmes) de la face `face` ? `ranges` : l'étendue de la face sur les
    /// deux autres axes, dans l'ordre circulaire.
    fn covered(&self, face: Face, plane: i32, ranges: [Range<i32>; 2]) -> bool {
        let a = face.axis;
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        // La cellule de l'autre côté : dans la case, ou la première / dernière du voisin.
        let beyond = if face.positive { plane } else { plane - 1 };
        let inside = (0..16).contains(&beyond);
        if !inside {
            let q = self.p + face.normal();
            let block = self.hood.get(q);
            if block.is_air() {
                return false;
            }
            let def = self.hood.registry.def(block);
            if shaped::is_shaped_block(def) {
                let Some(occupancy) = &self.neighbours[face_index(face)] else {
                    return false;
                };
                let slab = if face.positive { 0 } else { 15 };
                return cells_set(occupancy, a, slab, u, v, &ranges);
            }
            return !def.transparent && !def.liquid;
        }
        cells_set(&self.opaque, a, beyond as usize, u, v, &ranges)
    }

    fn light(&self, face: Option<Face>) -> [f32; 3] {
        let (sky, light) = match face {
            Some(face) => {
                let (_, sky, light) = self.hood.cell(self.p + face.normal());
                (sky.max(self.own_sky), light.max(self.own_light))
            }
            None => (self.own_sky, self.own_light),
        };
        [sky as f32 / 15.0, light as f32 / 15.0, 1.0]
    }

    /// Les boîtes d'une couche ou d'un microbloc.
    fn boxes(&self, mesh: &mut ChunkMesh, material: &Material, piece: Piece) {
        for b in piece.int_boxes() {
            let b = b.map(i32::from);
            for face in Face::ALL {
                let a = face.axis;
                let (u, v) = ((a + 1) % 3, (a + 2) % 3);
                let plane = if face.positive { b[a + 3] } else { b[a] };
                let ranges = [b[u]..b[u + 3], b[v]..b[v + 3]];
                if self.covered(face, plane, ranges.clone()) {
                    continue;
                }
                // Les quatre coins de la face, comme ceux d'un modèle.
                let mut origin = [b[0], b[1], b[2]].map(|c| c as f32 / 16.0);
                origin[a] = plane as f32 / 16.0;
                let (tu, tv) = face.tangents();
                let du = tu.as_vec3() * ((b[u + 3] - b[u]) as f32 / 16.0);
                let dv = tv.as_vec3() * ((b[v + 3] - b[v]) as f32 / 16.0);
                let origin = Vec3::from(origin);
                let corners = if face.positive {
                    vec![origin, origin + du, origin + du + dv, origin + dv]
                } else {
                    vec![origin, origin + dv, origin + du + dv, origin + du]
                };
                let on_edge = plane == if face.positive { 16 } else { 0 };
                let light = self.light(on_edge.then_some(face));
                self.emit(
                    mesh,
                    material,
                    &corners,
                    face.normal().as_vec3(),
                    face,
                    light,
                );
            }
        }
    }

    /// Une pente : le plan incliné, le mur du bord haut, le dessous et les deux triangles de côté.
    fn slope(
        &self,
        mesh: &mut ChunkMesh,
        material: &Material,
        facing: u8,
        ceiling: bool,
        level: u8,
    ) {
        let h = f32::from(level) / 16.0;
        let hi = i32::from(level);
        // La pente « canonique » : son bord haut au nord (z = 0), qui descend vers z = 1.
        let incline = Vec3::new(0.0, 1.0, h).normalize();
        let polygons: [(Vec<Vec3>, Vec3); 5] = [
            (
                vec![
                    Vec3::new(0.0, h, 0.0),
                    Vec3::new(1.0, h, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                    Vec3::new(0.0, 0.0, 1.0),
                ],
                incline,
            ),
            (
                vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.0, h, 0.0),
                    Vec3::new(1.0, h, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                ],
                -Vec3::Z,
            ),
            (
                vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                    Vec3::new(0.0, 0.0, 1.0),
                ],
                -Vec3::Y,
            ),
            (
                vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.0, 0.0, 1.0),
                    Vec3::new(0.0, h, 0.0),
                ],
                -Vec3::X,
            ),
            (
                vec![
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(1.0, h, 0.0),
                    Vec3::new(1.0, 0.0, 1.0),
                ],
                Vec3::X,
            ),
        ];
        for (index, (corners, normal)) in polygons.into_iter().enumerate() {
            // Où est cette face une fois la pente tournée et retournée ?
            let turned: Vec<Vec3> = corners
                .iter()
                .map(|&c| orient(c, facing, ceiling))
                .collect();
            let normal = orient_direction(normal, facing, ceiling);
            // Le plan incliné prend la texture du dessus (ou du dessous, au plafond).
            let face = if index == 0 {
                Face::new(1, normal.y > 0.0)
            } else {
                Face::of_normal(normal)
            };
            let (lo, top) = if ceiling { (16 - hi, 16) } else { (0, hi) };
            let plane = if face.positive { 16 } else { 0 };
            let hidden = match index {
                // Le plan incliné : toujours visible.
                0 => false,
                // Le dessous : toute la face de la case.
                2 => self.covered(face, plane, [0..16, 0..16]),
                // Le mur du bord haut, ou un triangle de côté (caché si tout son rectangle l'est).
                _ => {
                    let ranges = if face.axis == 0 {
                        [lo..top, 0..16]
                    } else {
                        [0..16, lo..top]
                    };
                    self.covered(face, plane, ranges)
                }
            };
            if hidden {
                continue;
            }
            let light = self.light(Some(face));
            self.emit(mesh, material, &turned, normal, face, light);
        }
    }

    /// Ajoute un polygone de 3 ou 4 coins à la face dessinée du matériau. L'ordre des coins est
    /// corrigé pour que le côté visible soit celui de la normale.
    fn emit(
        &self,
        mesh: &mut ChunkMesh,
        material: &Material,
        corners: &[Vec3],
        normal: Vec3,
        face: Face,
        light: [f32; 3],
    ) {
        let def = material.def;
        let mut order: Vec<usize> = (0..corners.len()).collect();
        let geometric = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        if geometric.dot(normal) < 0.0 {
            order.reverse();
        }
        let texture = def.face_texture(face.axis, face.positive);
        let rgb = if texture == 0 { def.color } else { [1.0; 3] };
        let (tint_kind, side_tint) = face_tint(def, face);
        let tint = self.hood.tint(tint_kind, self.p.x, self.p.z);
        let color = [
            rgb[0] * tint[0],
            rgb[1] * tint[1],
            rgb[2] * tint[2],
            def.alpha,
        ];
        let flags = block_flags(def)
            | if tint_kind == Tint::Grass {
                FLAG_GROUND
            } else {
                0
            };
        let start = mesh.vertices.len() as u32;
        for &i in &order {
            mesh.push_vertex(
                self.base + corners[i],
                normal.to_array(),
                color,
                light,
                texture | flags,
            );
            mesh.vertices.last_mut().expect("sommet ajouté").position[3] = side_tint;
        }
        let triangles: &[u32] = if order.len() == 4 {
            &[0, 1, 2, 0, 2, 3]
        } else {
            &[0, 1, 2]
        };
        let list = if self.hood.registry.is_translucent(material.id) {
            &mut mesh.translucent_indices
        } else {
            &mut mesh.indices
        };
        list.extend(triangles.iter().map(|i| start + i));
    }
}

fn face_index(face: Face) -> usize {
    face.axis * 2 + usize::from(!face.positive)
}

/// Toutes les cellules de la tranche `slab` (selon l'axe `a`), sur les étendues `ranges` des axes
/// `u` et `v`, sont-elles prises ?
fn cells_set(
    occupancy: &Occupancy,
    a: usize,
    slab: usize,
    u: usize,
    v: usize,
    ranges: &[Range<i32>; 2],
) -> bool {
    ranges[0].clone().all(|i| {
        ranges[1].clone().all(|j| {
            let mut c = [0usize; 3];
            c[a] = slab;
            c[u] = i as usize;
            c[v] = j as usize;
            shaped::is_set(occupancy, c[0], c[1], c[2])
        })
    })
}

/// Un point de la pente canonique (bord haut au nord) une fois tournée de `facing` quarts de tour et
/// éventuellement retournée au plafond.
fn orient(p: Vec3, facing: u8, ceiling: bool) -> Vec3 {
    let mut p = p;
    for _ in 0..facing {
        p = Vec3::new(1.0 - p.z, p.y, p.x);
    }
    if ceiling {
        p.y = 1.0 - p.y;
    }
    p
}

/// La même transformation pour une direction.
fn orient_direction(d: Vec3, facing: u8, ceiling: bool) -> Vec3 {
    let mut d = d;
    for _ in 0..facing {
        d = Vec3::new(-d.z, d.y, d.x);
    }
    if ceiling {
        d.y = -d.y;
    }
    d
}
