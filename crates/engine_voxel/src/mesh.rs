//! Le maillage : transformer les blocs d'un chunk en triangles.
//!
//! Deux algorithmes :
//! - [`mesh_culled`] (« face culling ») : une face par côté de bloc visible.
//!   Les faces cachées entre deux blocs pleins disparaissent.
//! - [`mesh_greedy`] (« greedy meshing ») : on part des mêmes faces, puis on fusionne
//!   les faces voisines, coplanaires et **identiques** (même bloc, même lumière, même
//!   occlusion à chaque coin) en grands rectangles.
//!
//! Chaque sommet porte sa lumière, lissée :
//! - **lumière douce** : la moyenne des 4 cellules d'air qui touchent ce coin de la
//!   face, au lieu de la lumière d'une seule cellule (qui ferait des marches d'escalier) ;
//! - **occlusion ambiante** (AO) : un coin coincé contre des blocs reçoit moins de
//!   lumière diffuse. On compte les 3 blocs autour du coin (deux côtés et la diagonale),
//!   méthode décrite par Mikola Lysenko (« 0fps »).
//!
//! Les blocs translucides (eau, verre) vont dans une liste d'indices à part, dessinée
//! dans une seconde passe avec mélange alpha.
//!
//! Les positions sont en coordonnées monde : le renderer n'a pas besoin de savoir
//! où se trouve le chunk. Avec des coordonnées en f32, c'est précis jusqu'à
//! plusieurs centaines de milliers de blocs de l'origine, largement assez ici.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Vec3};

use crate::biome::{BiomeId, BiomeRegistry};
use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::{BlockDef, BlockRegistry, Shape, Tint};
use crate::world::World;

const SIZE: i32 = CHUNK_SIZE as i32;
/// La lumière moyenne d'un coin est gardée en entier, de 0 à 60 (4 × 15) :
/// assez fin pour être lisse, et comparable exactement pour le greedy.
const LIGHT_SCALE: f32 = 60.0;
/// La surface d'un fluide plein est un peu sous le haut du bloc (14/16).
const FLUID_SURFACE: f32 = 0.875;

/// Un sommet du maillage.
///
/// `#[repr(C)]` fixe l'ordre des champs en mémoire, le même que dans le shader.
/// `Pod` (« plain old data ») dit que ce n'est qu'une suite d'octets : le jeu peut
/// ainsi passer ces sommets au renderer sans les recopier (`bytemuck::cast_slice`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Couleur du bloc et opacité (alpha).
    pub color: [f32; 4],
    /// Lumière du ciel, lumière des blocs et occlusion ambiante, chacune de 0 à 1.
    pub light: [f32; 3],
    /// Couche de la texture (0 = pas de texture) dans les 16 bits du bas, et des
    /// drapeaux au-dessus ([`FLAG_CUTOUT`], [`FLAG_SWAY`]). Les coordonnées de texture,
    /// elles, sont calculées dans le shader à partir de la position : la texture se
    /// répète toute seule sur un quad fusionné par le greedy.
    pub texture: u32,
}

/// Les pixels transparents de la texture sont jetés (`discard`) : feuilles, plantes.
pub const FLAG_CUTOUT: u32 = 1 << 16;
/// Le sommet se balance au vent.
pub const FLAG_SWAY: u32 = 1 << 17;
/// Les bits de la couche de texture dans [`MeshVertex::texture`].
pub const TEXTURE_MASK: u32 = 0xFFFF;

/// Le maillage d'un chunk : 4 sommets et 6 indices (2 triangles) par quad.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChunkMesh {
    pub vertices: Vec<MeshVertex>,
    /// Les triangles opaques.
    pub indices: Vec<u32>,
    /// Les triangles translucides (eau, verre), dans les mêmes sommets.
    pub translucent_indices: Vec<u32>,
}

impl ChunkMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() && self.translucent_indices.is_empty()
    }

    pub fn quad_count(&self) -> usize {
        self.triangle_count() / 2
    }

    pub fn triangle_count(&self) -> usize {
        (self.indices.len() + self.translucent_indices.len()) / 3
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

    /// Ajoute un rectangle sur la face `face` d'une boîte de blocs.
    ///
    /// `axis` est l'axe de la normale (0 = x, 1 = y, 2 = z) ; les deux autres axes,
    /// `u` et `v`, sont pris dans l'ordre circulaire x → y → z → x. Ce choix garantit
    /// `e_u × e_v = e_axis` : les coins p0, p1, p2, p3 tournent alors dans le sens
    /// anti-horaire vu depuis +axis. Pour une face négative, on inverse l'ordre.
    ///
    /// `look.corners` est indexé par coin de la face : 0 = (-u, -v), 1 = (+u, -v),
    /// 2 = (+u, +v), 3 = (-u, +v).
    fn push_quad(&mut self, face: Face, corner: IVec3, size: (i32, i32), look: FaceLook) {
        let (u, v) = face.tangents();
        let p0 = corner.as_vec3();
        let du = (u * size.0).as_vec3();
        let dv = (v * size.1).as_vec3();
        let (positions, order) = if face.positive {
            ([p0, p0 + du, p0 + du + dv, p0 + dv], [0, 1, 2, 3])
        } else {
            ([p0, p0 + dv, p0 + du + dv, p0 + du], [0, 3, 2, 1])
        };
        let corners = order.map(|c| look.corners[c]);
        let drops = order.map(|c| look.drops[c]);

        let base = self.vertices.len() as u32;
        let normal = face.normal().as_vec3().to_array();
        for ((position, corner), drop) in positions.into_iter().zip(corners).zip(drops) {
            // La surface d'un fluide : le coin descend selon la quantité.
            let position = position - Vec3::Y * (drop as f32 / 255.0);
            self.vertices.push(MeshVertex {
                position: position.to_array(),
                normal,
                color: look.color,
                light: [
                    corner.sky as f32 / LIGHT_SCALE,
                    corner.block as f32 / LIGHT_SCALE,
                    corner.ao as f32 / 3.0,
                ],
                texture: look.texture | look.flags,
            });
        }
        // Un quad se coupe en deux triangles par une diagonale. Si l'éclairage varie
        // d'un coin à l'autre, le choix de la diagonale se voit : on coupe selon la
        // diagonale dont les coins sont les plus clairs, sinon l'AO dessine des
        // traînées en biais (« anisotropie »).
        let brightness = corners.map(|c| c.brightness());
        let triangles = if brightness[0] + brightness[2] >= brightness[1] + brightness[3] {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 1, 3, 1, 2, 3]
        };
        let list = if look.translucent {
            &mut self.translucent_indices
        } else {
            &mut self.indices
        };
        list.extend(triangles.map(|i| base + i));
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

/// L'éclairage d'un coin de face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CornerLight {
    /// Lumière moyenne, de 0 à 60 (voir [`LIGHT_SCALE`]).
    sky: u8,
    block: u8,
    /// Occlusion ambiante : 3 = coin dégagé, 0 = coin enfermé.
    ao: u8,
}

impl CornerLight {
    fn brightness(self) -> u32 {
        self.ao as u32 * 128 + self.sky.max(self.block) as u32
    }
}

/// Tout ce qui décide de l'apparence d'une face. Deux faces voisines ne sont
/// fusionnées par le greedy que si elles sont identiques sur tous ces points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FaceLook {
    block: BlockId,
    color: [f32; 4],
    texture: u32,
    flags: u32,
    translucent: bool,
    corners: [CornerLight; 4],
    /// Pour un fluide : de combien chaque coin du haut descend sous le haut du bloc, en
    /// 255e de bloc (0 : rien). La surface suit la quantité de fluide.
    drops: [u8; 4],
}

/// Un chunk et ses 26 voisins : tout ce qu'il faut pour le mailler.
///
/// Pour savoir si une face au bord du chunk est cachée, il faut regarder le bloc
/// dans le chunk d'à côté ; pour la lumière douce et l'AO d'un coin, il faut même les
/// chunks en diagonale. On récupère les voisins une fois pour toutes plutôt que
/// d'interroger la `HashMap` du monde pour chaque bloc du bord.
///
/// La struct possède des `Arc<Chunk>` au lieu d'emprunter le monde (`&World`) :
/// elle ne dépend donc d'aucune durée de vie et on peut la déplacer (« move »)
/// vers un thread de fond. Le monde, lui, reste sur le thread principal.
#[derive(Debug, Clone)]
pub struct Neighbourhood {
    position: IVec3,
    /// 3 × 3 × 3 chunks, le chunk maillé au centre (indice 13). `None` : chunk d'air.
    chunks: [Option<Arc<Chunk>>; 27],
    /// Pour savoir quels blocs sont transparents, et leur couleur.
    registry: Arc<BlockRegistry>,
    /// Les teintes de l'herbe et du feuillage de chaque colonne du chunk central
    /// (x + 32 z), déjà lissées. Vides si les biomes sont inconnus (teinte neutre).
    grass_tint: Vec<[f32; 3]>,
    foliage_tint: Vec<[f32; 3]>,
    water_tint: Vec<[f32; 3]>,
}

impl Neighbourhood {
    /// Prend des copies des `Arc` du chunk et de ses voisins (aucun bloc n'est copié).
    /// `None` si le chunk lui-même n'existe pas.
    pub fn from_world(world: &World, chunk_pos: IVec3) -> Option<Self> {
        world.chunk(chunk_pos)?;
        // `from_fn` construit le tableau en appelant la closure pour chaque indice.
        let chunks: [Option<Arc<Chunk>>; 27] =
            std::array::from_fn(|i| world.chunk_arc(chunk_pos + offset_of(i)));
        let [grass_tint, foliage_tint, water_tint] = match (world.biomes(), &chunks[13]) {
            (Some(biomes), Some(center)) if center.biome(0, 0).is_some() => tints(biomes, center),
            _ => [Vec::new(), Vec::new(), Vec::new()],
        };
        Some(Self {
            position: chunk_pos,
            chunks,
            registry: world.registry().clone(),
            grass_tint,
            foliage_tint,
            water_tint,
        })
    }

    /// La hauteur de la surface d'un fluide dans la cellule `p` (de 0 à 1), `None` si ce
    /// n'est pas ce fluide. Une cellule recouverte du même fluide est pleine jusqu'en haut.
    fn fluid_height(&self, fluid: BlockId, p: IVec3) -> Option<f32> {
        if self.get(p) != fluid {
            return None;
        }
        if self.get(p + IVec3::Y) == fluid {
            return Some(1.0);
        }
        let offset = p.div_euclid(IVec3::splat(SIZE));
        let amount = self.chunks[index_of(offset)].as_ref().and_then(|c| {
            let q = p.rem_euclid(IVec3::splat(SIZE));
            c.fluid_amount(q.x as usize, q.y as usize, q.z as usize)
        });
        // Un réservoir (pas de quantité) est plein ; la surface est un peu sous le haut
        // du bloc, comme une eau calme.
        let level = amount.map_or(1.0, |a| (a as f32 / crate::fluid_sim::FULL as f32).min(1.0));
        Some(level * FLUID_SURFACE)
    }

    /// La hauteur d'un coin de surface : le coin (cx, cz) est partagé par quatre cellules
    /// à l'altitude `y` ; on fait la moyenne de celles qui contiennent ce fluide ou de
    /// l'air (l'air tire le bord vers le bas, comme une cascade qui déborde). Si une des
    /// quatre est recouverte de fluide, le coin est tout en haut.
    fn corner_height(&self, fluid: BlockId, y: i32, cx: i32, cz: i32) -> f32 {
        let (mut sum, mut count) = (0.0, 0.0);
        for (dx, dz) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
            let cell = IVec3::new(cx + dx, y, cz + dz);
            match self.fluid_height(fluid, cell) {
                Some(h) if h >= 1.0 => return 1.0,
                Some(h) => {
                    sum += h;
                    count += 1.0;
                }
                None => {
                    let block = self.get(cell);
                    if block.is_air() || !self.registry.is_solid(block) {
                        count += 1.0;
                    }
                }
            }
        }
        if count == 0.0 {
            FLUID_SURFACE
        } else {
            sum / count
        }
    }

    /// La teinte (un multiplicateur de couleur) de la colonne (x, z) du chunk central.
    fn tint(&self, tint: Tint, x: i32, z: i32) -> [f32; 3] {
        let table = match tint {
            Tint::None => return [1.0; 3],
            Tint::Grass => &self.grass_tint,
            Tint::Foliage => &self.foliage_tint,
            Tint::Water => &self.water_tint,
        };
        let i = (x.clamp(0, SIZE - 1) + z.clamp(0, SIZE - 1) * SIZE) as usize;
        table.get(i).copied().unwrap_or([1.0; 3])
    }

    pub fn position(&self) -> IVec3 {
        self.position
    }

    /// Le bloc et la lumière (ciel, blocs) en coordonnées locales, de -1 à 32 sur chaque axe.
    #[inline]
    fn cell(&self, p: IVec3) -> (BlockId, u8, u8) {
        let offset = p.div_euclid(IVec3::splat(SIZE));
        match &self.chunks[index_of(offset)] {
            Some(chunk) => {
                let q = p.rem_euclid(IVec3::splat(SIZE));
                let (x, y, z) = (q.x as usize, q.y as usize, q.z as usize);
                (
                    chunk.get(x, y, z),
                    chunk.sky_light(x, y, z),
                    chunk.block_light(x, y, z),
                )
            }
            // Un chunk absent est de l'air en plein ciel ; sous le monde, il fait noir.
            None if self.position.y + offset.y < 0 => (BlockId::AIR, 0, 0),
            None => (BlockId::AIR, 15, 0),
        }
    }

    #[inline]
    fn get(&self, p: IVec3) -> BlockId {
        // Cas le plus fréquent d'abord : le bloc est dans le chunk central.
        // `cmpge` / `cmplt` comparent les 3 composantes d'un coup.
        if p.cmpge(IVec3::ZERO).all()
            && p.cmplt(IVec3::splat(SIZE)).all()
            && let Some(center) = &self.chunks[13]
        {
            return center.get(p.x as usize, p.y as usize, p.z as usize);
        }
        self.cell(p).0
    }

    /// Un bloc qui fait de l'ombre aux coins voisins (pour l'AO) et cache la lumière.
    #[inline]
    fn occludes(&self, block: BlockId) -> bool {
        !self.registry.is_transparent(block)
    }

    /// Le bloc à `p`, si sa face `face` doit être dessinée : le bloc n'est pas de l'air
    /// et son voisin de ce côté est transparent (de l'air, ou un bloc transparent
    /// d'un autre type : entre deux blocs d'eau ou de verre identiques, pas de face).
    ///
    /// Exceptions : les plantes (formes autres que le cube) n'ont pas de faces de cube ;
    /// deux feuilles côte à côte gardent leurs faces (on voit à travers leurs trous) ;
    /// deux liquides côte à côte (l'eau et des algues) n'en ont pas entre eux.
    #[inline]
    fn visible_face(&self, p: IVec3, face: Face) -> Option<BlockId> {
        let block = self.get(p);
        if block.is_air() {
            return None;
        }
        let def = self.registry.def(block);
        if def.shape != Shape::Cube {
            return None;
        }
        let neighbour = self.get(p + face.normal());
        if neighbour.is_air() {
            return Some(block);
        }
        let other = self.registry.def(neighbour);
        if def.liquid && other.liquid {
            return None;
        }
        let visible = other.transparent && (neighbour != block || def.cutout);
        visible.then_some(block)
    }

    /// L'apparence de la face `face` du bloc `block` en `p`.
    fn face_look(&self, p: IVec3, face: Face, block: BlockId) -> FaceLook {
        let (u, v) = face.tangents();
        let front = p + face.normal();
        let def = self.registry.def(block);
        let corners = [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|(su, sv)| {
            // La cellule devant la face, ses deux voisines de côté et la diagonale.
            let cells = [
                front,
                front + u * su,
                front + v * sv,
                front + u * su + v * sv,
            ]
            .map(|c| self.cell(c));
            let side1 = self.occludes(cells[1].0);
            let side2 = self.occludes(cells[2].0);
            let diagonal = self.occludes(cells[3].0);
            // Coincé entre deux côtés pleins : le coin est complètement dans l'ombre,
            // même si la diagonale est vide (la lumière ne peut pas y passer).
            let ao = if side1 && side2 {
                0
            } else {
                3 - (side1 as u8 + side2 as u8 + diagonal as u8)
            };
            // Lumière douce : la moyenne des cellules qui laissent passer la lumière.
            let mut sky = 0u32;
            let mut blk = 0u32;
            let mut count = 0u32;
            for (k, &(id, s, b)) in cells.iter().enumerate() {
                let hidden = k == 3 && side1 && side2;
                if k == 0 || (!self.occludes(id) && !hidden) {
                    sky += s as u32;
                    blk += b as u32;
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
        // Dessus, côtés ou dessous.
        let texture = match (face.axis, face.positive) {
            (1, true) => def.face_textures[0],
            (1, false) => def.face_textures[2],
            _ => def.face_textures[1],
        };
        // Une face texturée prend les couleurs de sa texture : la couleur du registre ne
        // sert plus que de repli pour les blocs sans texture.
        let rgb = if texture == 0 { def.color } else { [1.0; 3] };
        // La teinte du biome : pour l'herbe, seulement le dessus (les côtés montrent la terre).
        let tinted = match def.tint {
            Tint::Grass => face.axis == 1 && face.positive,
            Tint::Foliage | Tint::Water => true,
            Tint::None => false,
        };
        let tint = if tinted {
            self.tint(def.tint, p.x, p.z)
        } else {
            [1.0; 3]
        };
        // Un fluide : la hauteur de chaque coin du haut, moyenne des cellules qui le
        // partagent (pas de marche ni de trou entre deux cellules voisines).
        let drops = if def.liquid {
            [(-1, -1), (1, -1), (1, 1), (-1, 1)].map(|(su, sv)| {
                let mut vertex = if face.positive {
                    face.normal()
                } else {
                    IVec3::ZERO
                };
                if su > 0 {
                    vertex += u;
                }
                if sv > 0 {
                    vertex += v;
                }
                if vertex.y == 1 {
                    let height = self.corner_height(block, p.y, p.x + vertex.x, p.z + vertex.z);
                    ((1.0 - height) * 255.0).round() as u8
                } else {
                    0
                }
            })
        } else {
            [0; 4]
        };
        FaceLook {
            block,
            texture,
            drops,
            flags: block_flags(def),
            color: [
                rgb[0] * tint[0],
                rgb[1] * tint[1],
                rgb[2] * tint[2],
                def.alpha,
            ],
            translucent: self.registry.is_translucent(block),
            corners,
        }
    }
}

/// Les drapeaux de sommet d'un bloc.
fn block_flags(def: &BlockDef) -> u32 {
    let mut flags = 0;
    if def.cutout {
        flags |= FLAG_CUTOUT;
    }
    if def.sway {
        flags |= FLAG_SWAY;
    }
    flags
}

/// Les teintes lissées de l'herbe et du feuillage, colonne par colonne : la moyenne des
/// 3 × 3 colonnes autour, divisée par la couleur de référence (celle des textures).
/// Lisser évite une couleur qui change d'un coup à la frontière de deux biomes.
fn tints(biomes: &BiomeRegistry, chunk: &Chunk) -> [Vec<[f32; 3]>; 3] {
    let (grass_ref, foliage_ref, water_ref) = biomes.tint_reference();
    let n = CHUNK_SIZE;
    let mut grass = vec![[1.0; 3]; n * n];
    let mut foliage = vec![[1.0; 3]; n * n];
    let mut water = vec![[1.0; 3]; n * n];
    let count = biomes.len() as u8;
    for z in 0..n {
        for x in 0..n {
            let (mut g, mut f, mut w, mut k) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 3], 0.0);
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let (sx, sz) = (x as i32 + dx, z as i32 + dz);
                    if !(0..n as i32).contains(&sx) || !(0..n as i32).contains(&sz) {
                        continue;
                    }
                    let Some(id) = chunk
                        .biome(sx as usize, sz as usize)
                        .filter(|&id| id < count)
                    else {
                        continue;
                    };
                    let b = biomes.get(BiomeId(id));
                    for c in 0..3 {
                        g[c] += b.grass_color[c];
                        f[c] += b.foliage_color[c];
                        w[c] += b.water_color[c];
                    }
                    k += 1.0;
                }
            }
            if k > 0.0 {
                grass[x + z * n] = std::array::from_fn(|c| g[c] / k / grass_ref[c].max(0.01));
                foliage[x + z * n] = std::array::from_fn(|c| f[c] / k / foliage_ref[c].max(0.01));
                water[x + z * n] = std::array::from_fn(|c| w[c] / k / water_ref[c].max(0.01));
            }
        }
    }
    [grass, foliage, water]
}

/// Les plantes du chunk : deux panneaux en X (`Cross`) ou un panneau au sol (`Flat`).
/// Chaque panneau est émis deux fois, une par face : on le voit des deux côtés.
fn plants(hood: &Neighbourhood, mesh: &mut ChunkMesh) {
    let origin = World::chunk_origin(hood.position);
    let center = match &hood.chunks[13] {
        Some(chunk) => chunk.clone(),
        None => return,
    };
    for y in 0..SIZE {
        for z in 0..SIZE {
            for x in 0..SIZE {
                let block = center.get(x as usize, y as usize, z as usize);
                if block.is_air() {
                    continue;
                }
                let def = hood.registry.def(block);
                if def.shape == Shape::Cube {
                    continue;
                }
                let p = IVec3::new(x, y, z);
                let (_, sky, light) = hood.cell(p);
                let tint = hood.tint(def.tint, x, z);
                let rgb = if def.face_textures[1] == 0 {
                    def.color
                } else {
                    [1.0; 3]
                };
                let color = [rgb[0] * tint[0], rgb[1] * tint[1], rgb[2] * tint[2], 1.0];
                let vertex_light = [sky as f32 / 15.0, light as f32 / 15.0, 1.0];
                let base = (origin + p).as_vec3();
                let flags = block_flags(def) | FLAG_CUTOUT;
                let texture = def.face_textures[1] | (flags & !FLAG_SWAY);
                // Les sommets du haut se balancent, ceux du bas restent plantés.
                let sway_top = flags & FLAG_SWAY;
                let quads: Vec<([Vec3; 4], [f32; 3])> = match def.shape {
                    Shape::Cross => {
                        let n = std::f32::consts::FRAC_1_SQRT_2;
                        vec![
                            (
                                [
                                    Vec3::new(0.0, 0.0, 0.0),
                                    Vec3::new(1.0, 0.0, 1.0),
                                    Vec3::new(1.0, 1.0, 1.0),
                                    Vec3::new(0.0, 1.0, 0.0),
                                ],
                                [n, 0.0, -n],
                            ),
                            (
                                [
                                    Vec3::new(1.0, 0.0, 0.0),
                                    Vec3::new(0.0, 0.0, 1.0),
                                    Vec3::new(0.0, 1.0, 1.0),
                                    Vec3::new(1.0, 1.0, 0.0),
                                ],
                                [n, 0.0, n],
                            ),
                        ]
                    }
                    Shape::Flat => vec![(
                        [
                            Vec3::new(0.0, 0.02, 0.0),
                            Vec3::new(1.0, 0.02, 0.0),
                            Vec3::new(1.0, 0.02, 1.0),
                            Vec3::new(0.0, 0.02, 1.0),
                        ],
                        [0.0, 1.0, 0.0],
                    )],
                    Shape::Cube => Vec::new(),
                };
                for (corners, normal) in quads {
                    let start = mesh.vertices.len() as u32;
                    for corner in corners {
                        let top = corner.y > 0.5;
                        mesh.vertices.push(MeshVertex {
                            position: (base + corner).to_array(),
                            normal,
                            color,
                            light: vertex_light,
                            texture: texture | if top { sway_top } else { 0 },
                        });
                    }
                    // Les deux faces : sens direct et sens inverse.
                    mesh.indices
                        .extend([0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2].map(|i| start + i));
                }
            }
        }
    }
}

/// Décalage (de -1 à 1 sur chaque axe) du voisin numéro `i` (0 à 26).
fn offset_of(i: usize) -> IVec3 {
    let i = i as i32;
    IVec3::new(i % 3, i / 9, (i / 3) % 3) - IVec3::ONE
}

/// L'inverse de [`offset_of`].
#[inline]
fn index_of(offset: IVec3) -> usize {
    ((offset.x + 1) + (offset.z + 1) * 3 + (offset.y + 1) * 9) as usize
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
                        let look = hood.face_look(p, face, block);
                        mesh.push_quad(face, face_corner(origin, p, face), (1, 1), look);
                    }
                }
            }
        }
    }
    plants(hood, &mut mesh);
    mesh
}

/// Greedy meshing : fusionne les faces visibles en rectangles aussi grands que possible.
///
/// Pour chaque direction de face, on découpe le chunk en 32 tranches. Dans chaque
/// tranche, un « masque » 32×32 note l'apparence de la face visible à chaque case.
/// On balaie ce masque : depuis chaque case non vide, on étend le rectangle
/// le long de `u` tant que l'apparence est la même, puis le long de `v` tant que
/// toute la ligne suivante correspond. On émet ce quad et on efface ses cases.
fn greedy(hood: &Neighbourhood) -> ChunkMesh {
    let mut mesh = ChunkMesh::default();
    let origin = World::chunk_origin(hood.position);
    let n = CHUNK_SIZE;
    // Un seul masque, réutilisé pour chaque tranche : pas d'allocation dans la boucle.
    // `Option<FaceLook>` : `None` là où il n'y a pas de face.
    let mut mask: Vec<Option<FaceLook>> = vec![None; n * n];

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
                    mask[i + j * n] = hood
                        .visible_face(p, face)
                        .map(|block| hood.face_look(p, face, block));
                }
            }

            // 2. Le découper en rectangles.
            for j in 0..n {
                let mut i = 0;
                while i < n {
                    let Some(look) = mask[i + j * n] else {
                        i += 1;
                        continue;
                    };
                    let mut width = 1;
                    while i + width < n && mask[i + width + j * n] == Some(look) {
                        width += 1;
                    }
                    let mut height = 1;
                    // `all` s'arrête au premier `false` : on teste la ligne suivante entière.
                    while j + height < n
                        && (0..width).all(|k| mask[i + k + (j + height) * n] == Some(look))
                    {
                        height += 1;
                    }

                    let p = n_axis * slice + u * i as i32 + v * j as i32;
                    let corner = face_corner(origin, p, face);
                    mesh.push_quad(face, corner, (width as i32, height as i32), look);

                    for dj in 0..height {
                        // `fill` remplit une tranche du tableau d'un coup.
                        mask[i + (j + dj) * n..i + width + (j + dj) * n].fill(None);
                    }
                    i += width;
                }
            }
        }
    }
    plants(hood, &mut mesh);
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

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }

    /// Aire totale du maillage : la somme des aires des triangles.
    fn surface(mesh: &ChunkMesh) -> f32 {
        let indices: Vec<u32> = mesh
            .indices
            .iter()
            .chain(&mesh.translucent_indices)
            .copied()
            .collect();
        indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|k| Vec3::from(mesh.vertices[k as usize].position));
                (b - a).cross(c - a).length() / 2.0
            })
            .sum()
    }

    /// Les quads dont la normale vaut `normal` (4 sommets chacun).
    fn quads_facing(mesh: &ChunkMesh, normal: Vec3) -> Vec<&[MeshVertex]> {
        mesh.vertices
            .chunks(4)
            .filter(|q| Vec3::from(q[0].normal) == normal)
            .collect()
    }

    #[test]
    fn neighbour_indices_round_trip() {
        for i in 0..27 {
            assert_eq!(index_of(offset_of(i)), i);
        }
        assert_eq!(offset_of(13), IVec3::ZERO);
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
        assert_eq!(quads_facing(&culled, Vec3::Y).len(), 32 * 32);
        assert_eq!(quads_facing(&greedy, Vec3::Y).len(), 1);
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
        let glass = id("core:glass");
        let mut world = World::new();
        world.set_block(IVec3::new(0, 0, 0), stone());
        world.set_block(IVec3::new(1, 0, 0), glass);
        world.set_block(IVec3::new(2, 0, 0), glass);
        let mesh = mesh_culled(&world, IVec3::ZERO);
        // Pierre : 6 faces (celle contre le verre reste visible à travers lui).
        // Premier verre : 4 faces (cachées contre la pierre, opaque, et contre l'autre verre).
        // Second verre : 5 faces (pas de face entre deux blocs de verre identiques).
        assert_eq!(mesh.quad_count(), 6 + 4 + 5);
        // Le verre est translucide : ses 9 faces vont dans la seconde liste.
        assert_eq!(mesh.translucent_indices.len(), 9 * 6);
    }

    #[test]
    fn faces_between_water_blocks_are_hidden() {
        let water = id("core:water");
        let mut world = World::new();
        // Un bassin d'eau 4 × 1 × 4 : seulement le contour extérieur.
        for x in 0..4 {
            for z in 0..4 {
                world.set_block(IVec3::new(x, 5, z), water);
            }
        }
        let mesh = mesh_culled(&world, IVec3::ZERO);
        assert_eq!(
            mesh.quad_count(),
            16 + 16 + 4 * 4,
            "dessus, dessous, 4 côtés"
        );
        assert!(mesh.indices.is_empty(), "rien d'opaque");
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
        assert_eq!(quads_facing(&greedy, Vec3::Y).len(), 2);
    }

    #[test]
    fn corner_ambient_occlusion() {
        // Un sol de 3 × 3 et un bloc posé dessus, dans le coin (2, 1, 2).
        let mut blocks: Vec<IVec3> = (0..3)
            .flat_map(|x| (0..3).map(move |z| IVec3::new(x, 0, z)))
            .collect();
        blocks.push(IVec3::new(2, 1, 2));
        let world = world_with(&blocks);
        let mesh = mesh_culled(&world, IVec3::ZERO);
        let top_of = |x: f32, z: f32| {
            quads_facing(&mesh, Vec3::Y)
                .into_iter()
                .find(|q| {
                    q.iter().all(|v| {
                        v.position[1] == 1.0
                            && (x..=x + 1.0).contains(&v.position[0])
                            && (z..=z + 1.0).contains(&v.position[2])
                    })
                })
                .unwrap()
        };
        // Le dessus du bloc central du sol (1, 0, 1) : son coin (2, 1, 2) touche le bloc
        // posé en diagonale (AO 2 sur 3), les trois autres sont dégagés.
        for v in top_of(1.0, 1.0) {
            let ao = v.light[2];
            if v.position[0] == 2.0 && v.position[2] == 2.0 {
                assert!(
                    (ao - 2.0 / 3.0).abs() < 1e-6,
                    "coin occulté par la diagonale : {ao}"
                );
            } else {
                assert_eq!(ao, 1.0, "coin dégagé en {:?}", v.position);
            }
        }
        // Le dessus du sol en (2, 0, 1), collé au bloc posé : ses deux coins côté z = 2
        // touchent un côté plein.
        let occluded = top_of(2.0, 1.0).iter().filter(|v| v.light[2] < 1.0).count();
        assert_eq!(occluded, 2);
    }

    #[test]
    fn greedy_does_not_merge_faces_with_different_light() {
        let mut world = world_with(&(0..8).map(|x| IVec3::new(x, 0, 0)).collect::<Vec<_>>());
        assert_eq!(
            quads_facing(&mesh_greedy(&world, IVec3::ZERO), Vec3::Y).len(),
            1,
            "même éclairage : un seul quad"
        );
        // De la lumière de torche au-dessus de la moitié de la rangée.
        for x in 0..4 {
            world.chunk_mut(IVec3::ZERO).set_block_light(x, 1, 0, 10);
        }
        let top = quads_facing(&mesh_greedy(&world, IVec3::ZERO), Vec3::Y).len();
        assert!(top >= 2, "deux éclairages, au moins deux quads : {top}");
    }

    #[test]
    fn greedy_and_culled_cover_the_same_surface() {
        let world = terrain(2);
        for chunk in [IVec3::ZERO, IVec3::new(1, 1, 1)] {
            let culled = mesh_culled(&world, chunk);
            let greedy = mesh_greedy(&world, chunk);
            // Chaque face 1×1 a une aire de 1, sauf les faces des fluides dont la surface
            // est plus basse que le haut du bloc : l'aire vaut au plus le nombre de faces.
            assert!(surface(&culled) <= culled.quad_count() as f32 + 0.01);
            assert!((surface(&greedy) - surface(&culled)).abs() < 0.5);
            assert!(greedy.quad_count() < culled.quad_count());
        }
    }

    /// Même règle que pour le cube instancié : sens anti-horaire vu de l'extérieur,
    /// sinon le backface culling du GPU cache la face. Vrai aussi pour les quads
    /// dont on a retourné la diagonale.
    #[test]
    fn triangles_face_outwards() {
        let world = terrain(1);
        for mesh in [
            mesh_culled(&world, IVec3::new(0, 1, 0)),
            mesh_greedy(&world, IVec3::new(0, 1, 0)),
        ] {
            for tri in mesh.indices.chunks(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]]
                    .map(|k| Vec3::from(mesh.vertices[k as usize].position));
                let geometric = (b - a).cross(c - a).normalize();
                let declared = Vec3::from(mesh.vertices[tri[0] as usize].normal);
                // Les plantes sont dessinées des deux côtés (un triangle par sens) : on
                // ne vérifie que les faces de cubes (normale sur un axe, sommets entiers).
                let plant = declared.abs().max_element() < 0.99 || a.y.fract().abs() > 0.001;
                if plant {
                    continue;
                }
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
