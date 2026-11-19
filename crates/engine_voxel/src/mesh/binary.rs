//! Le maillage par masques binaires (lot 7) : le même résultat que le greedy, bien plus vite.
//!
//! Le greedy de [`super::greedy`] interroge chacune des 6 × 32 768 faces possibles d'un chunk
//! (voisin, définition du bloc, forme...) avant de savoir si elle existe, puis relit les cellules
//! d'alentour à travers les 27 chunks pour l'occlusion ambiante et la lumière douce de chaque coin.
//! Presque tout cela est inutile : dans un chunk de terrain, une face sur vingt est visible.
//!
//! # Le principe
//!
//! 1. **Une copie à plat** : le chunk et deux blocs de chaque côté (36³ cellules : la lumière douce
//!    lit jusqu'à deux blocs au-delà du bord, diagonales comprises), copiés rangée par rangée depuis
//!    les 27 chunks. Les voisins ne sont plus consultés cellule par cellule.
//! 2. **Des masques binaires** : pour chaque axe, un entier de 32 bits par rangée dit quelles cellules
//!    sont des **cubes opaques** (la voie rapide), lesquelles cachent la face de leur voisine, et
//!    lesquelles sont d'**autres cubes** (feuilles, eau, verre, glace : la voie complète). Une
//!    rangée entière de faces visibles se calcule en une opération : `opaques & !cache(voisine)`.
//! 3. **Le greedy sur ce qui existe** : on ne calcule l'apparence (occlusion, lumière, teinte) que
//!    des faces visibles, depuis la copie à plat, puis on fusionne exactement comme le greedy.
//!
//! # Équivalence
//!
//! La voie rapide ne traite que les cubes pleins ([`Kind::fast`]) ; leurs faces ne fusionnent jamais
//! avec celles d'un autre type de bloc, donc les rectangles obtenus sont ceux du greedy pour ces
//! blocs. Les autres cubes passent par les mêmes règles de visibilité (réécrites sur la copie) et par
//! `Neighbourhood::face_look` lui-même ; les plantes, rails, dalles et autres formes restent
//! à `plants()`. Le résultat est le **même ensemble de quads** que `greedy` (l'ordre d'émission
//! diffère) : les tests le comparent sur des chunks de terrain, d'eau, de forêt, d'ouvrages, avec
//! leur lumière.
//!
//! # Tampons par thread
//!
//! La copie, les masques et la table des types de blocs vivent dans un tampon par thread de fond
//! (`thread_local`), réutilisé d'un chunk au suivant : aucune allocation par chunk.

use std::cell::RefCell;

use super::*;

const N: usize = CHUNK_SIZE;
/// Blocs copiés de chaque côté du chunk.
const MARGIN: i32 = 2;
const PAD: usize = N + 2 * MARGIN as usize;
const VOLUME: usize = PAD * PAD * PAD;
/// Rangées d'un masque : les 32 tranches du chunk plus une de chaque côté.
const ROWS: usize = (N + 2) * N;

/// Pas d'index dans la copie à plat, par axe (x, y, z) : l'ordre est celui d'un chunk.
const STRIDE: [usize; 3] = [1, PAD * PAD, PAD];

/// L'index d'une cellule de coordonnées locales de -2 à 33.
#[inline(always)]
fn at(p: IVec3) -> usize {
    ((p.y + MARGIN) as usize * PAD + (p.z + MARGIN) as usize) * PAD + (p.x + MARGIN) as usize
}

/// Une cube plein à dessiner par la voie rapide.
const FAST: u8 = 1;
/// Un cube à dessiner par la voie complète (feuilles, liquides, verre...).
const REST: u8 = 2;
/// Cache la face de son voisin : pas de l'air, et opaque ou feuille enfouie.
const COVER: u8 = 4;
/// Une feuille au cœur du feuillage (voir `Neighbourhood::enclosed_leaf`).
const ENCLOSED: u8 = 8;
/// Fait de l'ombre aux coins voisins (l'occlusion ambiante) : tout ce qui n'est pas transparent.
const OCCLUDES: u8 = 16;

/// Ce que le maillage retient d'un type de bloc.
#[derive(Debug, Clone, Copy, Default)]
struct Kind {
    air: bool,
    fast: bool,
    rest: bool,
    transparent: bool,
    liquid: bool,
    translucent: bool,
    cutout: bool,
    leaf: bool,
    furnace: bool,
}

impl Kind {
    fn of(block: BlockId, def: &BlockDef) -> Self {
        let air = block.is_air();
        let cube = def.shape == Shape::Cube && !air;
        let fast = cube && !def.transparent && !def.liquid && def.alpha >= 1.0 && !def.cutout;
        Self {
            air,
            fast,
            rest: cube && !fast,
            transparent: def.transparent,
            liquid: def.liquid,
            translucent: def.alpha < 1.0,
            cutout: def.cutout,
            leaf: is_leaf(def),
            furnace: def.behavior.as_deref() == Some("core:furnace"),
        }
    }
}

/// Les tampons d'un thread : voir la documentation du module.
struct Scratch {
    registry: Option<Arc<BlockRegistry>>,
    kinds: Vec<Kind>,
    blocks: Vec<BlockId>,
    /// Lumière du ciel (4 bits de poids fort) et des blocs (4 bits de poids faible).
    light: Vec<u8>,
    class: Vec<u8>,
    /// Par axe `a`, rangée `((k + 1) * 32 + j)` : un bit par `i`, avec `i` le long de `(a + 1) % 3`, `j` le
    /// long de `(a + 2) % 3` et `k` le long de `a` (de -1 à 32).
    fast_rows: [Vec<u32>; 3],
    rest_rows: [Vec<u32>; 3],
    cover_rows: [Vec<u32>; 3],
    fast_mask: Vec<Option<FaceLook>>,
    rest_mask: Vec<Option<FaceLook>>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            registry: None,
            kinds: Vec::new(),
            blocks: vec![BlockId::AIR; VOLUME],
            light: vec![0; VOLUME],
            class: vec![0; VOLUME],
            fast_rows: std::array::from_fn(|_| vec![0; ROWS]),
            rest_rows: std::array::from_fn(|_| vec![0; ROWS]),
            cover_rows: std::array::from_fn(|_| vec![0; ROWS]),
            fast_mask: vec![None; N * N],
            rest_mask: vec![None; N * N],
        }
    }

    /// La table des types de blocs, refaite seulement si le registre a changé.
    fn use_registry(&mut self, registry: &Arc<BlockRegistry>) {
        if self
            .registry
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, registry))
        {
            return;
        }
        self.kinds = registry.iter().map(|(id, def)| Kind::of(id, def)).collect();
        self.registry = Some(registry.clone());
    }

    #[inline(always)]
    fn kind(&self, block: BlockId) -> Kind {
        // Un identifiant hors registre se comporte comme l'air (voir `BlockRegistry::def`).
        self.kinds
            .get(block.0 as usize)
            .copied()
            .unwrap_or(self.kinds[0])
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

/// Le maillage d'un chunk par masques binaires (voir la documentation du module).
pub(super) fn binary(hood: &Neighbourhood) -> ChunkMesh {
    SCRATCH.with(|scratch| mesh_with(&mut scratch.borrow_mut(), hood))
}

fn mesh_with(s: &mut Scratch, hood: &Neighbourhood) -> ChunkMesh {
    let mut mesh = new_mesh(hood);
    let origin = World::chunk_origin(hood.position);
    s.use_registry(&hood.registry);
    fill(s, hood);
    classify(s, hood);
    build_rows(s);

    for face in Face::ALL {
        let axis = face.axis;
        let step = if face.positive { 1 } else { -1 };
        let n_axis = face.normal().abs();
        let (u, v) = face.tangents();
        for k in 0..SIZE {
            let here = (k + 1) as usize * N;
            let next = (k + 1 + step) as usize * N;
            let mut any_fast = false;
            let mut any_rest = false;
            for j in 0..N {
                // Les faces visibles d'une rangée entière : les cubes opaques moins ceux dont la
                // voisine, de ce côté, cache la face.
                let mut fast = s.fast_rows[axis][here + j] & !s.cover_rows[axis][next + j];
                let mut rest = s.rest_rows[axis][here + j];
                while fast != 0 {
                    let i = fast.trailing_zeros() as i32;
                    fast &= fast - 1;
                    let p = n_axis * k + u * i + v * j as i32;
                    let block = s.blocks[at(p)];
                    s.fast_mask[i as usize + j * N] = Some(cube_look(s, hood, p, face, block));
                    any_fast = true;
                }
                while rest != 0 {
                    let i = rest.trailing_zeros() as i32;
                    rest &= rest - 1;
                    let p = n_axis * k + u * i + v * j as i32;
                    let block = s.blocks[at(p)];
                    if rest_visible(s, p, face, block) {
                        // Les fluides (hauteur de surface par coin) gardent la voie complète.
                        let look = if s.kind(block).liquid {
                            hood.face_look(p, face, block)
                        } else {
                            cube_look(s, hood, p, face, block)
                        };
                        s.rest_mask[i as usize + j * N] = Some(look);
                        any_rest = true;
                    }
                }
            }
            if any_fast {
                merge(&mut s.fast_mask, |i, j, width, height, look| {
                    let p = n_axis * k + u * i as i32 + v * j as i32;
                    let corner = face_corner(origin, p, face);
                    mesh.push_quad(face, corner, (width as i32, height as i32), look);
                });
            }
            if any_rest {
                merge(&mut s.rest_mask, |i, j, width, height, look| {
                    let p = n_axis * k + u * i as i32 + v * j as i32;
                    let corner = face_corner(origin, p, face);
                    mesh.push_quad(face, corner, (width as i32, height as i32), look);
                });
            }
        }
    }
    plants(hood, &mut mesh);
    mesh
}

/// Copie le chunk et deux blocs de chaque côté à plat, rangée par rangée.
fn fill(s: &mut Scratch, hood: &Neighbourhood) {
    let margin = MARGIN as usize;
    for y in -MARGIN..SIZE + MARGIN {
        for z in -MARGIN..SIZE + MARGIN {
            let (oy, ly) = (y.div_euclid(SIZE), y.rem_euclid(SIZE));
            let (oz, lz) = (z.div_euclid(SIZE), z.rem_euclid(SIZE));
            // Trois morceaux le long de x : (décalage du chunk, premier x local copié, nombre) pour la fin
            // du voisin -x, le chunk lui-même, le début du voisin +x.
            for (ox, from, count) in [(-1, SIZE - MARGIN, margin), (0, 0, N), (1, 0, margin)] {
                let dst = at(IVec3::new(ox * SIZE + from, y, z));
                match &hood.chunks[index_of(IVec3::new(ox, oy, oz))] {
                    Some(chunk) => {
                        let src = Chunk::index(from as usize, ly as usize, lz as usize);
                        s.blocks[dst..dst + count]
                            .copy_from_slice(&chunk.blocks()[src..src + count]);
                        s.light[dst..dst + count]
                            .copy_from_slice(&chunk.light_data()[src..src + count]);
                    }
                    None => {
                        // Un chunk absent est de l'air en plein ciel ; sous le monde, il fait noir.
                        let light = if hood.position.y + oy < hood.bottom {
                            0
                        } else {
                            0xF0
                        };
                        s.blocks[dst..dst + count].fill(BlockId::AIR);
                        s.light[dst..dst + count].fill(light);
                    }
                }
            }
        }
    }
}

/// Le type de chaque cellule du chunk et de sa première couche voisine (de -1 à 32).
fn classify(s: &mut Scratch, hood: &Neighbourhood) {
    let size = SIZE;
    // 1. Ce que chaque bloc est, d'après sa table : toute la copie, car l'occlusion lit jusqu'à deux
    // blocs au-delà du bord ; la voie de dessin et ce qui cache une face ne servent qu'au chunk et à sa
    // première couche voisine.
    for i in 0..VOLUME {
        let kind = s.kind(s.blocks[i]);
        let mut class = 0;
        if kind.fast {
            class |= FAST;
        }
        if kind.rest {
            class |= REST;
        }
        if !kind.transparent {
            class |= COVER | OCCLUDES;
        }
        s.class[i] = class;
    }
    // 2. Les feuilles enfouies (leurs voisines sont dans la copie, jusqu'à la couche -1 et 32).
    if hood.leaf_culling {
        for y in -1..=size {
            for z in -1..=size {
                for x in -1..=size {
                    let i = at(IVec3::new(x, y, z));
                    if !s.kind(s.blocks[i]).leaf {
                        continue;
                    }
                    let enclosed = STRIDE.iter().all(|&stride| {
                        [i - stride, i + stride].iter().all(|&j| {
                            let neighbour = s.kind(s.blocks[j]);
                            !neighbour.air && (!neighbour.transparent || neighbour.leaf)
                        })
                    });
                    if enclosed {
                        s.class[i] |= ENCLOSED | COVER;
                    }
                }
            }
        }
    }
}

/// Les rangées de bits de chaque axe (voir [`Scratch::fast_rows`]).
fn build_rows(s: &mut Scratch) {
    let size = SIZE;
    // Trois axes : l'index de l'axe sert aussi à choisir les deux autres, donc pas d'itérateur.
    #[allow(clippy::needless_range_loop)]
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let origin = at(IVec3::ZERO);
        for k in -1..=size {
            for j in 0..size {
                let base = origin as isize
                    + k as isize * STRIDE[axis] as isize
                    + (j as usize * STRIDE[v]) as isize;
                let mut fast = 0u32;
                let mut rest = 0u32;
                let mut cover = 0u32;
                for i in 0..N {
                    let class = s.class[base as usize + i * STRIDE[u]];
                    fast |= ((class & FAST) as u32) << i;
                    rest |= (((class & REST) >> 1) as u32) << i;
                    cover |= (((class & COVER) >> 2) as u32) << i;
                }
                let row = (k + 1) as usize * N + j as usize;
                s.fast_rows[axis][row] = fast;
                // Les rangées de la voie complète ne servent qu'aux tranches du chunk.
                s.rest_rows[axis][row] = if (0..size).contains(&k) { rest } else { 0 };
                s.cover_rows[axis][row] = cover;
            }
        }
    }
}

/// La face de ce cube « complet » est-elle visible ? Les règles de `Neighbourhood::visible_face`,
/// lues dans la copie à plat.
#[inline]
fn rest_visible(s: &Scratch, p: IVec3, face: Face, block: BlockId) -> bool {
    let kind = s.kind(block);
    let neighbour_index = at(p + face.normal());
    let neighbour_block = s.blocks[neighbour_index];
    let neighbour = s.kind(neighbour_block);
    if neighbour.air {
        return true;
    }
    if kind.liquid && neighbour.liquid {
        return false;
    }
    // Un bloc translucide n'a pas de face contre un liquide : sa surface est déjà dessinée.
    if neighbour.liquid && kind.translucent {
        return false;
    }
    if s.class[neighbour_index] & ENCLOSED != 0 {
        return false;
    }
    neighbour.transparent && (neighbour_block != block || kind.cutout)
}

/// L'apparence de la face `face` du cube `block` en `p` : `Neighbourhood::face_look` pour un bloc qui
/// n'est pas un fluide (cubes pleins, feuilles, verre, glace), avec la lumière et l'occlusion lues dans
/// la copie à plat.
fn cube_look(s: &Scratch, hood: &Neighbourhood, p: IVec3, face: Face, block: BlockId) -> FaceLook {
    let (u, v) = face.tangents();
    let front = p + face.normal();
    let def = hood.registry.def(block);
    let corners = [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|(su, sv)| {
        // La cellule devant la face, ses deux voisines de côté et la diagonale.
        let cells = [
            front,
            front + u * su,
            front + v * sv,
            front + u * su + v * sv,
        ]
        .map(|c| {
            let i = at(c);
            (
                s.class[i] & OCCLUDES != 0,
                s.light[i] >> 4,
                s.light[i] & 0x0F,
            )
        });
        let side1 = cells[1].0;
        let side2 = cells[2].0;
        let diagonal = cells[3].0;
        // Coincé entre deux côtés pleins : le coin est dans l'ombre, même si la diagonale est vide.
        let ao = if side1 && side2 {
            0
        } else {
            3 - (side1 as u8 + side2 as u8 + diagonal as u8)
        };
        let mut sky = 0u32;
        let mut blk = 0u32;
        let mut count = 0u32;
        for (k, &(occludes, sky_light, block_light)) in cells.iter().enumerate() {
            let hidden = k == 3 && side1 && side2;
            if k == 0 || (!occludes && !hidden) {
                sky += sky_light as u32;
                blk += block_light as u32;
                count += 1;
            }
        }
        // Moyenne arrondie, sur l'échelle 0..60 (4 × la lumière).
        let average = |sum: u32| ((sum * 4 + count / 2) / count) as u8;
        CornerLight {
            sky: average(sky),
            block: average(blk),
            ao,
        }
    });
    let texture = match (face.axis, face.positive) {
        (1, true) => def.face_textures[0],
        (1, false) => def.face_textures[2],
        _ => def.face_textures[1],
    };
    let rgb = if texture == 0 { def.color } else { [1.0; 3] };
    let tinted = match def.tint {
        Tint::Grass => face.axis == 1 && face.positive,
        Tint::Foliage | Tint::Water => true,
        Tint::None => false,
    };
    let tint = if tinted {
        hood.tint(def.tint, p.x, p.z)
    } else {
        [1.0; 3]
    };
    let kind = s.kind(block);
    let fire = if kind.furnace {
        hood.furnace_fire(p, def, face)
    } else {
        0
    };
    // Une feuille au cœur du feuillage est dessinée opaque (ses trous remplis).
    let flags = if s.class[at(p)] & ENCLOSED != 0 {
        (block_flags(def) & !FLAG_CUTOUT) | FLAG_FILLED
    } else {
        block_flags(def)
    };
    FaceLook {
        block,
        texture,
        drops: [0; 4],
        flags: fire
            | if def.tint == Tint::Grass && face.axis == 1 && face.positive {
                FLAG_GROUND
            } else {
                0
            }
            | flags,
        color: [
            rgb[0] * tint[0],
            rgb[1] * tint[1],
            rgb[2] * tint[2],
            def.alpha,
        ],
        translucent: kind.translucent,
        corners,
    }
}

/// Fusionne les cases voisines et identiques du masque en rectangles, comme le greedy, et les
/// remet à zéro.
fn merge(
    mask: &mut [Option<FaceLook>],
    mut emit: impl FnMut(usize, usize, usize, usize, FaceLook),
) {
    for j in 0..N {
        let mut i = 0;
        while i < N {
            let Some(look) = mask[i + j * N] else {
                i += 1;
                continue;
            };
            let mut width = 1;
            while i + width < N && mask[i + width + j * N] == Some(look) {
                width += 1;
            }
            let mut height = 1;
            while j + height < N && (0..width).all(|k| mask[i + k + (j + height) * N] == Some(look))
            {
                height += 1;
            }
            emit(i, j, width, height, look);
            for dj in 0..height {
                mask[i + (j + dj) * N..i + width + (j + dj) * N].fill(None);
            }
            i += width;
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::IVec2;

    use super::*;
    use crate::light::{LightRegion, apply_column_light};
    use crate::{GenerationVersion, TerrainGenerator};

    /// Les triangles d'un maillage comme des suites d'octets de sommets, triés : deux maillages qui
    /// dessinent les mêmes quads, dans un autre ordre, sont égaux.
    fn canonical(mesh: &ChunkMesh) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let triangles = |indices: &[u32]| {
            let mut all: Vec<Vec<u8>> = indices
                .chunks(3)
                .map(|t| {
                    t.iter()
                        .flat_map(|&i| bytemuck::bytes_of(&mesh.vertices[i as usize]).to_vec())
                        .collect()
                })
                .collect();
            all.sort();
            all
        };
        (
            triangles(&mesh.indices),
            triangles(&mesh.translucent_indices),
        )
    }

    /// Le binaire et le greedy dessinent la même chose pour ce chunk (avec et sans culling des feuilles).
    fn assert_same(world: &World, pos: IVec3, what: &str) -> Option<ChunkMesh> {
        let hood = Neighbourhood::from_world(world, pos)?;
        let mut shown = None;
        for leaf_culling in [true, false] {
            let hood = hood.clone().with_leaf_culling(leaf_culling);
            let (old, new) = (greedy(&hood), binary(&hood));
            assert_eq!(old.origin, new.origin, "{what} {pos}");
            assert_eq!(old.visibility, new.visibility, "{what} {pos}");
            assert_eq!(
                old.vertices.len(),
                new.vertices.len(),
                "{what} {pos} : sommets"
            );
            assert_eq!(
                canonical(&old),
                canonical(&new),
                "{what} {pos} (feuilles {leaf_culling})"
            );
            shown = Some(new);
        }
        shown
    }

    /// Éclaire les colonnes d'un monde comme le streamer.
    fn lit(mut world: World, columns: impl IntoIterator<Item = IVec2>) -> World {
        for column in columns {
            let light = LightRegion::from_world(&world, column).compute();
            apply_column_light(&mut world, column, light);
        }
        world
    }

    fn columns(from: IVec2, count: i32) -> Vec<IVec2> {
        (0..count)
            .flat_map(|x| (0..count).map(move |z| from + IVec2::new(x, z)))
            .collect()
    }

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }

    /// Compare tous les chunks d'une région générée ; renvoie (chunks, avec eau, avec feuilles).
    fn compare_region(
        generator: &TerrainGenerator,
        from: IVec2,
        what: &str,
    ) -> (usize, usize, usize) {
        let region = generator.generate_area(IVec3::new(from.x, 0, from.y), 3, 3);
        let world = lit(region, columns(from, 3));
        let positions: Vec<IVec3> = world.chunk_positions().collect();
        let (mut water, mut leaves) = (0, 0);
        for &pos in &positions {
            if let Some(mesh) = assert_same(&world, pos, what) {
                water += !mesh.translucent_indices.is_empty() as usize;
                leaves += mesh.vertices.iter().any(|v| v.texture & FLAG_CUTOUT != 0) as usize;
            }
        }
        (positions.len(), water, leaves)
    }

    #[test]
    fn terrain_chunks_match_the_greedy_with_their_light() {
        let (mut chunks, mut water, mut leaves) = (0, 0, 0);
        for seed in [7, 42, 1337] {
            let generator = TerrainGenerator::new(seed);
            for from in [IVec2::ZERO, IVec2::new(37, -12), IVec2::new(-60, 80)] {
                let (c, w, l) = compare_region(&generator, from, &format!("graine {seed}"));
                chunks += c;
                water += w;
                leaves += l;
            }
        }
        assert!(chunks > 150, "{chunks} chunks");
        assert!(water > 5, "{water} chunks avec de l'eau");
        assert!(leaves > 5, "{leaves} chunks avec des feuilles");
    }

    #[test]
    fn v2_chunks_match_the_greedy_too() {
        let generator = TerrainGenerator::new(42)
            .with_version(GenerationVersion::V2)
            .expect("v2");
        let (chunks, _, _) = compare_region(&generator, IVec2::new(-20, 15), "v2");
        assert!(chunks > 15);
    }

    /// Un petit générateur déterministe de 0 à 99.
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state % 100
    }

    #[test]
    fn mixed_blocks_across_chunk_borders_match_the_greedy() {
        // Du désordre à travers les frontières : pierre, verre, glace, eau, feuilles, braises,
        // lumière de torche, une dalle... sur un chunk et ses voisins (lumière comprise).
        let kinds = [
            id("core:stone"),
            id("core:glass"),
            id("core:ice"),
            id("core:water"),
            id("core:oak_leaves"),
            id("core:birch_leaves"),
            id("core:glowstone"),
            id("core:grass"),
            id("core:dirt"),
            id("core:tall_grass"),
            id("core:oak_planks"),
            id("core:furnace"),
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for round in 0..6 {
            let mut world = World::new();
            for cx in -1..=1 {
                for cy in 0..=1 {
                    for cz in -1..=1 {
                        let chunk = world.chunk_mut(IVec3::new(cx, cy, cz));
                        let density = 25 + 10 * round as u64;
                        for y in 0..CHUNK_SIZE {
                            for z in 0..CHUNK_SIZE {
                                for x in 0..CHUNK_SIZE {
                                    if rng(&mut state) < density {
                                        let k = rng(&mut state) as usize % kinds.len();
                                        // Surtout de la pierre : de grandes faces à fusionner.
                                        let block = if rng(&mut state) < 60 {
                                            kinds[0]
                                        } else {
                                            kinds[k]
                                        };
                                        chunk.set(x, y, z, block);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let world = lit(world, columns(IVec2::new(-1, -1), 3));
            for pos in [
                IVec3::new(0, 0, 0),
                IVec3::new(0, 1, 0),
                IVec3::new(-1, 0, 1),
            ] {
                assert_same(&world, pos, &format!("désordre {round}"));
            }
        }
    }

    #[test]
    fn full_layers_and_enclosed_leaves_match_the_greedy() {
        let mut world = World::new();
        let (stone, leaves, water) = (id("core:stone"), id("core:oak_leaves"), id("core:water"));
        for y in 0..10 {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let block = if y < 6 {
                        stone
                    } else if y < 9 {
                        leaves
                    } else {
                        water
                    };
                    world.chunk_mut(IVec3::ZERO).set(x, y, z, block);
                }
            }
        }
        // Des feuilles enfouies dans la pierre et des trous dans le feuillage.
        for p in [(3, 2, 3), (4, 2, 3), (4, 2, 4), (10, 7, 10), (11, 8, 10)] {
            world
                .chunk_mut(IVec3::ZERO)
                .set(p.0, p.1, p.2, BlockId::AIR);
        }
        world.chunk_mut(IVec3::ZERO).set(20, 3, 20, leaves);
        let world = lit(world, columns(IVec2::new(-1, -1), 3));
        assert_same(&world, IVec3::ZERO, "couches");
    }

    #[test]
    fn a_second_registry_and_a_missing_neighbour_are_handled() {
        // Un registre refait (le jeu en charge un autre au rechargement des mods) : les types de blocs
        // du tampon du thread sont reconstruits, jamais ceux de l'ancien registre.
        for _ in 0..2 {
            let registry = BlockRegistry::builtin();
            let mut world = World::with_registry(registry.clone());
            let stone = registry.id("core:stone").unwrap();
            // Un chunk seul, sans voisins, tout en haut du monde et tout en bas.
            for cy in [0, 5] {
                let chunk = world.chunk_mut(IVec3::new(0, cy, 0));
                for x in 0..CHUNK_SIZE {
                    for z in 0..CHUNK_SIZE {
                        chunk.set(x, 3, z, stone);
                        chunk.set(x, CHUNK_SIZE - 1, z, stone);
                    }
                }
                assert_same(&world, IVec3::new(0, cy, 0), "sans voisins");
            }
        }
    }

    #[test]
    fn the_mesher_enum_selects_the_binary_path_and_cycles_through_the_three() {
        assert_eq!(Mesher::Culling.other(), Mesher::Greedy);
        assert_eq!(Mesher::Greedy.other(), Mesher::Binary);
        assert_eq!(Mesher::Binary.other(), Mesher::Culling);
        assert_eq!(Mesher::Binary.name(), "binaire");
        let generator = TerrainGenerator::new(42);
        let world = lit(generator.generate_region(3, 3), columns(IVec2::ZERO, 3));
        let hood = Neighbourhood::from_world(&world, IVec3::new(1, 1, 1)).unwrap();
        assert_eq!(
            canonical(&Mesher::Binary.mesh(&hood)),
            canonical(&Mesher::Greedy.mesh(&hood))
        );
    }

    /// `cargo test -p engine_voxel --release --lib print_phase_costs -- --ignored --nocapture` : ce que coûte
    /// chaque étape sur un chunk de surface, de forêt et de mer (pour savoir où gagner encore).
    #[test]
    #[ignore = "mesure, n'affirme rien"]
    fn print_phase_costs() {
        use std::time::Instant;
        let wide = lit(
            TerrainGenerator::new(7).generate_region(5, 5),
            columns(IVec2::ZERO, 5),
        );
        let pick = |wanted: &dyn Fn(&ChunkMesh) -> usize| {
            wide.chunk_positions()
                .max_by_key(|&p| wanted(&greedy(&Neighbourhood::from_world(&wide, p).unwrap())))
                .unwrap()
        };
        let scenes = [
            ("surface", IVec3::new(2, 1, 2)),
            (
                "forêt",
                pick(&|m| {
                    m.vertices
                        .iter()
                        .filter(|v| v.texture & FLAG_CUTOUT != 0)
                        .count()
                }),
            ),
            ("mer", pick(&|m| m.translucent_indices.len())),
        ];
        for (name, pos) in scenes {
            let hood = Neighbourhood::from_world(&wide, pos).unwrap();
            let mut s = Scratch::new();
            s.use_registry(&hood.registry);
            let rounds = 400;
            let time = |f: &mut dyn FnMut()| {
                let started = Instant::now();
                for _ in 0..rounds {
                    f();
                }
                started.elapsed().as_secs_f64() * 1e6 / rounds as f64
            };
            let fill_us = time(&mut || fill(&mut s, &hood));
            let classify_us = time(&mut || classify(&mut s, &hood));
            let rows_us = time(&mut || build_rows(&mut s));
            let all_us = time(&mut || {
                binary(&hood);
            });
            let visibility_us = time(&mut || {
                std::hint::black_box(new_mesh(&hood));
            });
            let plants_us = time(&mut || {
                let mut m = ChunkMesh::default();
                plants(&hood, &mut m);
                std::hint::black_box(m);
            });
            println!("  visibilité {visibility_us:.0} µs, plantes {plants_us:.0} µs");
            // Les faces visibles de chaque voie, et les quads émis.
            fill(&mut s, &hood);
            classify(&mut s, &hood);
            build_rows(&mut s);
            let mut faces = 0usize;
            for face in Face::ALL {
                let step = if face.positive { 1 } else { -1 };
                for k in 0..SIZE {
                    for j in 0..N {
                        let here = (k + 1) as usize * N + j;
                        let next = (k + 1 + step) as usize * N + j;
                        faces += (s.fast_rows[face.axis][here] & !s.cover_rows[face.axis][next])
                            .count_ones() as usize;
                    }
                }
            }
            let mesh = binary(&hood);
            println!(
                "{name:8} copie {fill_us:5.0} µs | classes {classify_us:5.0} | rangées {rows_us:5.0} | total {all_us:6.0} µs | {faces} faces rapides, {} quads ({:.0} ns par face)",
                mesh.quad_count(),
                (all_us - fill_us - classify_us - rows_us) * 1000.0 / faces.max(1) as f64
            );
        }
    }
}
