//! Le maillage : transformer les blocs d'un chunk en triangles.
//!
//! Deux algorithmes :
//! - [`mesh_culled`] (« face culling ») : une face par côté de bloc touchant l'air.
//!   Les faces cachées entre deux blocs pleins disparaissent.
//! - [`mesh_greedy`] (« greedy meshing ») : on part des mêmes faces, puis on fusionne
//!   les faces voisines, coplanaires et du même bloc en grands rectangles.
//!
//! Les deux couvrent exactement la même surface ; le greedy le fait avec bien
//! moins de triangles.
//!
//! Les positions sont en coordonnées monde : le renderer n'a pas besoin de savoir
//! où se trouve le chunk. Avec des coordonnées en f32, c'est précis jusqu'à
//! plusieurs centaines de milliers de blocs de l'origine, largement assez ici.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;
use crate::world::World;

const SIZE: i32 = CHUNK_SIZE as i32;

/// Un sommet du maillage : position monde, normale de la face, couleur du bloc.
///
/// `#[repr(C)]` fixe l'ordre des champs en mémoire, le même que dans le shader.
/// `Pod` (« plain old data ») dit que ce n'est qu'une suite d'octets : le jeu peut
/// ainsi passer ces sommets au renderer sans les recopier (`bytemuck::cast_slice`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
}

/// Le maillage d'un chunk : 4 sommets et 6 indices (2 triangles) par quad.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChunkMesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

impl ChunkMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn quad_count(&self) -> usize {
        self.indices.len() / 6
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// La boîte englobante des sommets (min, max), ou `None` si le maillage est vide.
    /// Plus serrée que le chunk entier : idéal pour le frustum culling.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        // `fold` réduit l'itérateur à une seule valeur, ici la paire (min, max).
        self.vertices.iter().map(|v| Vec3::from(v.position)).fold(
            None,
            |acc: Option<(Vec3, Vec3)>, p| match acc {
                None => Some((p, p)),
                Some((min, max)) => Some((min.min(p), max.max(p))),
            },
        )
    }

    /// Ajoute un rectangle sur la face `dir` d'une boîte de blocs.
    ///
    /// `axis` est l'axe de la normale (0 = x, 1 = y, 2 = z) ; les deux autres axes,
    /// `u` et `v`, sont pris dans l'ordre circulaire x → y → z → x. Ce choix garantit
    /// `e_u × e_v = e_axis` : les coins p0, p1, p2, p3 tournent alors dans le sens
    /// anti-horaire vu depuis +axis. Pour une face négative, on inverse l'ordre.
    fn push_quad(&mut self, face: Face, corner: IVec3, width: i32, height: i32, color: [f32; 3]) {
        let (u, v) = face.tangents();
        let p0 = corner.as_vec3();
        let du = (u * width).as_vec3();
        let dv = (v * height).as_vec3();
        let corners = if face.positive {
            [p0, p0 + du, p0 + du + dv, p0 + dv]
        } else {
            [p0, p0 + dv, p0 + du + dv, p0 + du]
        };

        let base = self.vertices.len() as u32;
        let normal = face.normal().as_vec3().to_array();
        self.vertices.extend(corners.map(|position| MeshVertex {
            position: position.to_array(),
            normal,
            color,
        }));
        self.indices.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
    }
}

/// Une des 6 directions de face : un axe et un sens.
#[derive(Debug, Clone, Copy)]
struct Face {
    axis: usize,
    positive: bool,
}

impl Face {
    /// Les 6 faces, dans l'ordre +x, -x, +y, -y, +z, -z.
    const ALL: [Face; 6] = [
        Face::new(0, true),
        Face::new(0, false),
        Face::new(1, true),
        Face::new(1, false),
        Face::new(2, true),
        Face::new(2, false),
    ];

    const fn new(axis: usize, positive: bool) -> Self {
        Self { axis, positive }
    }

    fn normal(self) -> IVec3 {
        let mut n = IVec3::ZERO;
        n[self.axis] = if self.positive { 1 } else { -1 };
        n
    }

    fn tangents(self) -> (IVec3, IVec3) {
        let mut u = IVec3::ZERO;
        let mut v = IVec3::ZERO;
        u[(self.axis + 1) % 3] = 1;
        v[(self.axis + 2) % 3] = 1;
        (u, v)
    }
}

/// Un chunk et ses 6 voisins directs : tout ce qu'il faut pour le mailler.
///
/// Pour savoir si une face au bord du chunk est cachée, il faut regarder le bloc
/// dans le chunk d'à côté. On récupère les voisins une fois pour toutes plutôt que
/// d'interroger la `HashMap` du monde pour chaque bloc du bord.
///
/// La struct possède des `Arc<Chunk>` au lieu d'emprunter le monde (`&World`) :
/// elle ne dépend donc d'aucune durée de vie et on peut la déplacer (« move »)
/// vers un thread de fond. Le monde, lui, reste sur le thread principal.
#[derive(Debug, Clone)]
pub struct Neighbourhood {
    position: IVec3,
    center: Arc<Chunk>,
    /// Dans l'ordre de `Face::ALL`. `None` : chunk absent, donc de l'air.
    neighbours: [Option<Arc<Chunk>>; 6],
    /// Pour savoir quels blocs sont transparents, et leur couleur.
    registry: Arc<BlockRegistry>,
}

impl Neighbourhood {
    /// Prend des copies des `Arc` du chunk et de ses voisins (aucun bloc n'est copié).
    /// `None` si le chunk lui-même n'existe pas.
    pub fn from_world(world: &World, chunk_pos: IVec3) -> Option<Self> {
        let center = world.chunk_arc(chunk_pos)?;
        let neighbours = Face::ALL.map(|face| world.chunk_arc(chunk_pos + face.normal()));
        Some(Self {
            position: chunk_pos,
            center,
            neighbours,
            registry: world.registry().clone(),
        })
    }

    pub fn position(&self) -> IVec3 {
        self.position
    }

    /// Le bloc en coordonnées locales. Une seule coordonnée peut sortir du chunk,
    /// et d'un bloc au plus : c'est tout ce dont on a besoin pour tester un voisin.
    #[inline]
    fn get(&self, p: IVec3) -> BlockId {
        // Cas le plus fréquent d'abord : le bloc est dans le chunk central.
        // `cmpge` / `cmplt` comparent les 3 composantes d'un coup.
        if p.cmpge(IVec3::ZERO).all() && p.cmplt(IVec3::splat(SIZE)).all() {
            return self.center.get(p.x as usize, p.y as usize, p.z as usize);
        }
        let outside = Face::ALL.iter().position(|face| {
            let c = p[face.axis];
            if face.positive { c >= SIZE } else { c < 0 }
        });
        match outside {
            None => self.center.get(p.x as usize, p.y as usize, p.z as usize),
            // `as_deref` : `&Option<Arc<Chunk>>` devient `Option<&Chunk>`.
            Some(i) => match self.neighbours[i].as_deref() {
                None => BlockId::AIR,
                Some(chunk) => {
                    let q = p.rem_euclid(IVec3::splat(SIZE));
                    chunk.get(q.x as usize, q.y as usize, q.z as usize)
                }
            },
        }
    }

    /// Le bloc à `p`, si sa face `face` doit être dessinée : le bloc n'est pas de l'air
    /// et son voisin de ce côté est transparent (de l'air, ou un bloc transparent
    /// d'un autre type : entre deux blocs de verre identiques, pas de face).
    #[inline]
    fn visible_face(&self, p: IVec3, face: Face) -> Option<BlockId> {
        let block = self.get(p);
        if block.is_air() {
            return None;
        }
        let neighbour = self.get(p + face.normal());
        let visible =
            neighbour.is_air() || (neighbour != block && self.registry.is_transparent(neighbour));
        visible.then_some(block)
    }

    fn color(&self, block: BlockId) -> [f32; 3] {
        self.registry.def(block).color
    }
}

/// Le coin du quad de taille 1 pour la face `face` du bloc `p` (local),
/// converti en coordonnées monde.
fn face_corner(origin: IVec3, p: IVec3, face: Face) -> IVec3 {
    let mut corner = origin + p;
    if face.positive {
        // La face positive est sur le plan « de l'autre côté » du bloc.
        corner[face.axis] += 1;
    }
    corner
}

/// L'algorithme de maillage. Un `enum` sans données, comme en C#.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mesher {
    Culling,
    Greedy,
}

impl Mesher {
    pub fn other(self) -> Self {
        match self {
            Self::Culling => Self::Greedy,
            Self::Greedy => Self::Culling,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Culling => "culling",
            Self::Greedy => "greedy",
        }
    }

    pub fn mesh(self, hood: &Neighbourhood) -> ChunkMesh {
        match self {
            Self::Culling => culled(hood),
            Self::Greedy => greedy(hood),
        }
    }
}

/// Maillage avec suppression des faces cachées : un quad 1×1 par face visible.
///
/// Renvoie un maillage vide si le chunk n'existe pas.
pub fn mesh_culled(world: &World, chunk_pos: IVec3) -> ChunkMesh {
    Neighbourhood::from_world(world, chunk_pos).map_or_else(ChunkMesh::default, |h| culled(&h))
}

/// Greedy meshing (voir [`greedy`]). Renvoie un maillage vide si le chunk n'existe pas.
pub fn mesh_greedy(world: &World, chunk_pos: IVec3) -> ChunkMesh {
    Neighbourhood::from_world(world, chunk_pos).map_or_else(ChunkMesh::default, |h| greedy(&h))
}

fn culled(hood: &Neighbourhood) -> ChunkMesh {
    let mut mesh = ChunkMesh::default();
    let origin = World::chunk_origin(hood.position);

    for y in 0..SIZE {
        for z in 0..SIZE {
            for x in 0..SIZE {
                let p = IVec3::new(x, y, z);
                for face in Face::ALL {
                    if let Some(block) = hood.visible_face(p, face) {
                        let corner = face_corner(origin, p, face);
                        mesh.push_quad(face, corner, 1, 1, hood.color(block));
                    }
                }
            }
        }
    }
    mesh
}

/// Greedy meshing : fusionne les faces visibles en rectangles aussi grands que possible.
///
/// Pour chaque direction de face, on découpe le chunk en 32 tranches. Dans chaque
/// tranche, un « masque » 32×32 note quel bloc montre une face à chaque case.
/// On balaie ce masque : depuis chaque case non vide, on étend le rectangle
/// le long de `u` tant que le bloc est le même, puis le long de `v` tant que
/// toute la ligne suivante correspond. On émet ce quad et on efface ses cases.
fn greedy(hood: &Neighbourhood) -> ChunkMesh {
    let mut mesh = ChunkMesh::default();
    let origin = World::chunk_origin(hood.position);
    let n = CHUNK_SIZE;
    // Un seul masque, réutilisé pour chaque tranche : pas d'allocation dans la boucle.
    let mut mask = vec![BlockId::AIR; n * n];

    for face in Face::ALL {
        // Les vecteurs unitaires de la tranche : `p = slice·n + i·u + j·v`.
        // Plus rapide que d'écrire `p[axe] = …`, qui passe par un `match` sur l'axe.
        let n_axis = face.normal().abs();
        let (u, v) = face.tangents();
        for slice in 0..SIZE {
            // 1. Remplir le masque de cette tranche.
            for j in 0..n {
                for i in 0..n {
                    let p = n_axis * slice + u * i as i32 + v * j as i32;
                    mask[i + j * n] = hood.visible_face(p, face).unwrap_or(BlockId::AIR);
                }
            }

            // 2. Le découper en rectangles.
            for j in 0..n {
                let mut i = 0;
                while i < n {
                    let block = mask[i + j * n];
                    if block.is_air() {
                        i += 1;
                        continue;
                    }
                    let mut width = 1;
                    while i + width < n && mask[i + width + j * n] == block {
                        width += 1;
                    }
                    let mut height = 1;
                    // `all` s'arrête au premier `false` : on teste la ligne suivante entière.
                    while j + height < n
                        && (0..width).all(|k| mask[i + k + (j + height) * n] == block)
                    {
                        height += 1;
                    }

                    let p = n_axis * slice + u * i as i32 + v * j as i32;
                    let corner = face_corner(origin, p, face);
                    mesh.push_quad(face, corner, width as i32, height as i32, hood.color(block));

                    for dj in 0..height {
                        // `fill` remplit une tranche du tableau d'un coup.
                        mask[i + (j + dj) * n..i + width + (j + dj) * n].fill(BlockId::AIR);
                    }
                    i += width;
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::TerrainGenerator;
    use crate::registry::test_blocks::{grass, stone};

    fn terrain(chunks: i32) -> World {
        TerrainGenerator::new(42).generate_region(chunks, chunks)
    }

    fn world_with(blocks: &[IVec3]) -> World {
        let mut world = World::new();
        for &p in blocks {
            world.set_block(p, stone());
        }
        world
    }

    /// Aire totale du maillage : la somme des aires des triangles.
    fn surface(mesh: &ChunkMesh) -> f32 {
        mesh.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|k| Vec3::from(mesh.vertices[k as usize].position));
                (b - a).cross(c - a).length() / 2.0
            })
            .sum()
    }

    /// Nombre de quads dont la normale vaut `normal`.
    fn quads_facing(mesh: &ChunkMesh, normal: Vec3) -> usize {
        mesh.vertices
            .chunks(4)
            .filter(|q| Vec3::from(q[0].normal) == normal)
            .count()
    }

    #[test]
    fn single_block_has_six_faces() {
        let world = world_with(&[IVec3::new(3, 4, 5)]);
        assert_eq!(mesh_culled(&world, IVec3::ZERO).quad_count(), 6);
        assert_eq!(mesh_greedy(&world, IVec3::ZERO).quad_count(), 6);
    }

    #[test]
    fn two_touching_blocks_hide_two_faces() {
        let world = world_with(&[IVec3::new(3, 4, 5), IVec3::new(4, 4, 5)]);
        assert_eq!(mesh_culled(&world, IVec3::ZERO).quad_count(), 10);
        // Le greedy fusionne les paires de faces côte à côte : une boîte 2×1×1 = 6 quads.
        assert_eq!(mesh_greedy(&world, IVec3::ZERO).quad_count(), 6);
    }

    #[test]
    fn flat_layer_is_one_quad_on_top_with_greedy() {
        let mut world = World::new();
        for x in 0..SIZE {
            for z in 0..SIZE {
                world.set_block(IVec3::new(x, 0, z), grass());
            }
        }
        let culled = mesh_culled(&world, IVec3::ZERO);
        let greedy = mesh_greedy(&world, IVec3::ZERO);
        assert_eq!(quads_facing(&culled, Vec3::Y), 32 * 32);
        assert_eq!(quads_facing(&greedy, Vec3::Y), 1);
        // Dessus, dessous et 4 bandes sur les côtés.
        assert_eq!(greedy.quad_count(), 6);
    }

    #[test]
    fn faces_between_chunks_are_hidden() {
        // x = 31 dans le chunk 0 et x = 32 (local 0) dans le chunk 1.
        let world = world_with(&[IVec3::new(31, 0, 0), IVec3::new(32, 0, 0)]);
        assert_eq!(mesh_culled(&world, IVec3::ZERO).quad_count(), 5);
        assert_eq!(mesh_culled(&world, IVec3::X).quad_count(), 5);
        assert_eq!(mesh_greedy(&world, IVec3::X).quad_count(), 5);
    }

    #[test]
    fn transparent_neighbours_keep_faces_visible() {
        let mut registry = BlockRegistry::builtin().as_ref().clone();
        registry
            .add_source(
                "test",
                r#"[(id: "test:glass", name: "Verre", color: (0.8, 0.9, 1.0), transparent: true)]"#,
                "test",
            )
            .unwrap();
        let glass = registry.id("test:glass").unwrap();
        let mut world = World::with_registry(Arc::new(registry));
        world.set_block(IVec3::new(0, 0, 0), stone());
        world.set_block(IVec3::new(1, 0, 0), glass);
        world.set_block(IVec3::new(2, 0, 0), glass);
        let mesh = mesh_culled(&world, IVec3::ZERO);
        // Pierre : 6 faces (celle contre le verre reste visible à travers lui).
        // Premier verre : 4 faces (cachées contre la pierre, opaque, et contre l'autre verre).
        // Second verre : 5 faces (pas de face entre deux blocs de verre identiques).
        assert_eq!(mesh.quad_count(), 6 + 4 + 5);
    }

    #[test]
    fn missing_chunk_gives_empty_mesh() {
        let world = World::new();
        assert!(mesh_culled(&world, IVec3::ZERO).is_empty());
        assert!(mesh_greedy(&world, IVec3::ZERO).is_empty());
    }

    #[test]
    fn different_blocks_are_not_merged() {
        let mut world = world_with(&[IVec3::new(0, 0, 0)]);
        world.set_block(IVec3::new(1, 0, 0), grass());
        let greedy = mesh_greedy(&world, IVec3::ZERO);
        // Le dessus reste en deux quads : pierre et herbe n'ont pas la même couleur.
        assert_eq!(quads_facing(&greedy, Vec3::Y), 2);
    }

    #[test]
    fn greedy_and_culled_cover_the_same_surface() {
        let world = terrain(2);
        for chunk in [IVec3::ZERO, IVec3::new(1, 1, 1)] {
            let culled = mesh_culled(&world, chunk);
            let greedy = mesh_greedy(&world, chunk);
            // Chaque face 1×1 a une aire de 1 : l'aire vaut le nombre de faces.
            assert_eq!(surface(&culled), culled.quad_count() as f32);
            assert_eq!(surface(&greedy), surface(&culled));
            assert!(greedy.quad_count() < culled.quad_count() / 2);
        }
    }

    /// Même règle que pour le cube instancié : sens anti-horaire vu de l'extérieur,
    /// sinon le backface culling du GPU cache la face.
    #[test]
    fn triangles_face_outwards() {
        let world = terrain(1);
        for mesh in [
            mesh_culled(&world, IVec3::ZERO),
            mesh_greedy(&world, IVec3::ZERO),
        ] {
            for tri in mesh.indices.chunks(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]]
                    .map(|k| Vec3::from(mesh.vertices[k as usize].position));
                let geometric = (b - a).cross(c - a).normalize();
                let declared = Vec3::from(mesh.vertices[tri[0] as usize].normal);
                assert!(
                    geometric.dot(declared) > 0.99,
                    "triangle {tri:?} mal orienté"
                );
            }
        }
    }

    #[test]
    fn bounds_fit_the_mesh() {
        let world = world_with(&[IVec3::new(32, 0, 0)]);
        let mesh = mesh_greedy(&world, IVec3::X);
        assert_eq!(
            mesh.bounds(),
            Some((Vec3::new(32.0, 0.0, 0.0), Vec3::new(33.0, 1.0, 1.0)))
        );
        assert_eq!(ChunkMesh::default().bounds(), None);
    }
}
