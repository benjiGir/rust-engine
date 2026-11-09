//! Le culling des grottes : ne dessiner que les chunks qu'on peut voir depuis la caméra.
//!
//! Le frustum culling garde tout ce qui est devant la caméra, même enfoui : debout sur
//! l'herbe, on dessine toutes les grottes en dessous. Or on ne voit un chunk que si un
//! chemin d'air (ou de blocs transparents) y mène depuis la caméra.
//!
//! 1. **Au maillage** (sur un thread de fond), [`FaceConnections::of_chunk`] remplit
//!    chaque zone d'air du chunk et note quelles faces du chunk elle touche : « par
//!    l'air, on peut passer de la face ouest à la face du dessus ». 15 paires possibles.
//! 2. **À chaque image**, [`VisibilityGraph::visible`] part du chunk de la caméra et
//!    avance de chunk en chunk, en largeur : on n'entre dans un voisin que par une face
//!    reliée à celle par où on est entré, si le voisin est dans le champ de vision, et
//!    sans jamais revenir vers la caméra (un chemin ne change pas de sens sur un axe).
//!
//! C'est l'« advanced cave culling » de Tommaso Checchi (Minecraft 1.8), repris aussi par
//! Horizon. Le résultat est prudent : il peut garder un chunk caché (le chemin d'air
//! existe mais tourne trop), jamais en retirer un visible.

use glam::{IVec3, Vec3};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::registry::BlockRegistry;

/// Les six faces d'un chunk, dans l'ordre de [`FACE_DIRECTIONS`].
pub const FACES: usize = 6;

/// La direction de chaque face : -X, +X, -Y, +Y, -Z, +Z. La face opposée de `f` est
/// `f ^ 1`.
pub const FACE_DIRECTIONS: [IVec3; FACES] = [
    IVec3::NEG_X,
    IVec3::X,
    IVec3::NEG_Y,
    IVec3::Y,
    IVec3::NEG_Z,
    IVec3::Z,
];

/// Quelles faces d'un chunk communiquent par l'intérieur : une matrice 6 × 6 symétrique,
/// rangée dans les bits d'un entier (bit `a * 6 + b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceConnections(u64);

impl FaceConnections {
    /// Tout communique : un chunk d'air, ou un chunk inconnu (prudent). Seules les paires
    /// de faces distinctes comptent (on ne ressort jamais par où on est entré).
    pub const ALL: Self = {
        let mut bits = 0u64;
        let mut a = 0;
        while a < FACES {
            let mut b = 0;
            while b < FACES {
                if a != b {
                    bits |= 1 << (a * FACES + b);
                }
                b += 1;
            }
            a += 1;
        }
        Self(bits)
    };
    /// Rien ne communique : un chunk plein.
    pub const NONE: Self = Self(0);

    /// Vrai si on peut passer de la face `a` à la face `b` par l'intérieur.
    pub fn connects(self, a: usize, b: usize) -> bool {
        self.0 & (1 << (a * FACES + b)) != 0
    }

    fn connect(&mut self, a: usize, b: usize) {
        self.0 |= (1 << (a * FACES + b)) | (1 << (b * FACES + a));
    }

    /// Le nombre de paires de faces distinctes reliées (de 0 à 15).
    pub fn pairs(self) -> u32 {
        let mut pairs = 0;
        for a in 0..FACES {
            for b in a + 1..FACES {
                pairs += u32::from(self.connects(a, b));
            }
        }
        pairs
    }

    /// Remplit chaque zone de blocs transparents du chunk et relie les faces qu'elle
    /// touche. Environ 0,1 ms pour 32³ blocs : on le fait avec le maillage, en fond.
    pub fn of_chunk(chunk: &Chunk, registry: &BlockRegistry) -> Self {
        const N: usize = CHUNK_SIZE;
        let blocks = chunk.blocks();
        // `true` : un bloc à travers lequel on voit (le même critère que le maillage).
        let open: Vec<bool> = blocks.iter().map(|&b| registry.is_transparent(b)).collect();
        if open.iter().all(|&o| o) {
            return Self::ALL;
        }
        if !open.iter().any(|&o| o) {
            return Self::NONE;
        }
        let mut seen = vec![false; blocks.len()];
        let mut stack = Vec::new();
        let mut connections = Self::NONE;
        for start in 0..blocks.len() {
            if !open[start] || seen[start] {
                continue;
            }
            // Un remplissage par pile : chaque bloc ouvert n'est visité qu'une fois.
            seen[start] = true;
            stack.push(start);
            let mut touched = 0u8;
            while let Some(i) = stack.pop() {
                // L'ordre de `Chunk::index` : x, puis z, puis y.
                let (x, z, y) = (i % N, (i / N) % N, i / (N * N));
                touched |= faces_touched(x, y, z);
                let neighbours = [
                    (x > 0).then(|| i - 1),
                    (x + 1 < N).then(|| i + 1),
                    (y > 0).then(|| i - N * N),
                    (y + 1 < N).then(|| i + N * N),
                    (z > 0).then(|| i - N),
                    (z + 1 < N).then(|| i + N),
                ];
                for j in neighbours.into_iter().flatten() {
                    if open[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            for a in 0..FACES {
                for b in a + 1..FACES {
                    if touched & (1 << a) != 0 && touched & (1 << b) != 0 {
                        connections.connect(a, b);
                    }
                }
            }
            if connections == Self::ALL {
                break; // Rien de plus à apprendre.
            }
        }
        connections
    }
}

impl Default for FaceConnections {
    /// Prudent : sans information, tout communique.
    fn default() -> Self {
        Self::ALL
    }
}

/// Les faces du chunk que touche le bloc local `(x, y, z)`, en bits.
fn faces_touched(x: usize, y: usize, z: usize) -> u8 {
    let last = CHUNK_SIZE - 1;
    u8::from(x == 0)
        | u8::from(x == last) << 1
        | u8::from(y == 0) << 2
        | u8::from(y == last) << 3
        | u8::from(z == 0) << 4
        | u8::from(z == last) << 5
}

/// Les connexions de chaque chunk maillé. Un chunk absent compte comme ouvert partout :
/// un chunk d'air (jamais maillé) ou pas encore chargé.
#[derive(Debug, Default)]
pub struct VisibilityGraph {
    chunks: HashMap<IVec3, FaceConnections>,
    /// Réutilisés d'une image à l'autre : pas d'allocation par image.
    queue: std::collections::VecDeque<(IVec3, u8, u8)>,
    seen: HashSet<IVec3>,
}

impl VisibilityGraph {
    pub fn set(&mut self, chunk: IVec3, connections: FaceConnections) {
        if connections == FaceConnections::ALL {
            self.chunks.remove(&chunk);
        } else {
            self.chunks.insert(chunk, connections);
        }
    }

    pub fn remove(&mut self, chunk: IVec3) {
        self.chunks.remove(&chunk);
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
    }

    pub fn get(&self, chunk: IVec3) -> FaceConnections {
        self.chunks.get(&chunk).copied().unwrap_or_default()
    }

    /// Les chunks visibles depuis `camera` (en blocs), ajoutés à `out`. On ne va pas plus
    /// loin que `radius` chunks à l'horizontale, ni hors des couches `heights` (élargies
    /// pour contenir la caméra). `in_view(chunk)` dit si un chunk est dans le champ.
    pub fn visible(
        &mut self,
        camera: Vec3,
        radius: i32,
        heights: std::ops::Range<i32>,
        in_view: impl Fn(IVec3) -> bool,
        out: &mut Vec<IVec3>,
    ) {
        let _span = engine_trace::span("culling des grottes");
        let start = crate::World::chunk_containing(camera);
        let heights = heights.start.min(start.y)..heights.end.max(start.y + 1);
        self.queue.clear();
        self.seen.clear();
        // (chunk, face d'entrée (6 = aucune : la caméra), directions déjà prises).
        self.queue.push_back((start, FACES as u8, 0));
        self.seen.insert(start);
        out.push(start);
        while let Some((chunk, entry, taken)) = self.queue.pop_front() {
            let connections = self.get(chunk);
            for (exit, direction) in FACE_DIRECTIONS.iter().enumerate() {
                // Jamais vers la caméra : on ne prend pas la direction opposée à une
                // direction déjà prise. Ça borne aussi le parcours.
                if taken & (1 << (exit ^ 1)) != 0 {
                    continue;
                }
                let entry = usize::from(entry);
                if entry < FACES && !connections.connects(entry, exit) {
                    continue;
                }
                let next = chunk + *direction;
                if (next.x - start.x).abs() > radius
                    || (next.z - start.z).abs() > radius
                    || !heights.contains(&next.y)
                    || self.seen.contains(&next)
                    || !in_view(next)
                {
                    continue;
                }
                self.seen.insert(next);
                out.push(next);
                // On entre dans le voisin par sa face opposée.
                self.queue
                    .push_back((next, (exit ^ 1) as u8, taken | (1 << exit)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;

    fn registry() -> std::sync::Arc<BlockRegistry> {
        BlockRegistry::builtin()
    }

    fn stone(registry: &BlockRegistry) -> BlockId {
        registry.id("core:stone").unwrap()
    }

    fn filled(registry: &BlockRegistry) -> Chunk {
        let mut chunk = Chunk::new();
        let stone = stone(registry);
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    chunk.set(x, y, z, stone);
                }
            }
        }
        chunk
    }

    #[test]
    fn empty_and_full_chunks() {
        let registry = registry();
        let air = Chunk::new();
        assert_eq!(
            FaceConnections::of_chunk(&air, &registry),
            FaceConnections::ALL
        );
        let full = filled(&registry);
        assert_eq!(
            FaceConnections::of_chunk(&full, &registry),
            FaceConnections::NONE
        );
    }

    #[test]
    fn a_tunnel_connects_only_its_two_ends() {
        let registry = registry();
        let mut chunk = filled(&registry);
        // Un tunnel d'ouest (-X) en est (+X), à mi-hauteur.
        for x in 0..CHUNK_SIZE {
            chunk.set(x, 10, 10, BlockId::AIR);
        }
        // Une bulle fermée au milieu : elle ne relie rien.
        chunk.set(20, 20, 20, BlockId::AIR);
        let c = FaceConnections::of_chunk(&chunk, &registry);
        assert!(c.connects(0, 1) && c.connects(1, 0));
        assert_eq!(c.pairs(), 1);
    }

    #[test]
    fn surface_chunk_connects_sides_and_top_but_not_bottom() {
        let registry = registry();
        let mut chunk = filled(&registry);
        // De l'air dans la moitié haute : la moitié basse est pleine.
        for y in 16..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    chunk.set(x, y, z, BlockId::AIR);
                }
            }
        }
        let c = FaceConnections::of_chunk(&chunk, &registry);
        assert!(c.connects(3, 0), "dessus ↔ ouest");
        assert!(c.connects(4, 5), "nord ↔ sud");
        assert!(!c.connects(2, 3), "le dessous ne mène nulle part");
        assert_eq!(c.pairs(), 10, "5 faces reliées deux à deux");
    }

    /// Un monde de 5 × 3 × 5 chunks : le sol (y = 1) est plein, sauf un puits sous la
    /// caméra. Les chunks du dessous (y = 0) ne sont visibles que par le puits.
    #[test]
    fn caves_below_a_solid_floor_are_hidden() {
        let mut graph = VisibilityGraph::default();
        for x in -2..=2 {
            for z in -2..=2 {
                graph.set(IVec3::new(x, 1, z), FaceConnections::NONE);
                graph.set(IVec3::new(x, 0, z), FaceConnections::NONE);
            }
        }
        let everything = |_| true;
        let mut out = Vec::new();
        graph.visible(Vec3::new(16.0, 80.0, 16.0), 2, 0..3, everything, &mut out);
        assert!(out.iter().all(|c| c.y >= 1), "{out:?}");
        // Le sol lui-même est visible (on le voit d'en haut), pas ce qui est dessous.
        assert!(out.contains(&IVec3::new(1, 1, -2)));

        // Un puits : le chunk de sol sous la caméra relie le dessus au dessous.
        let mut shaft = FaceConnections::NONE;
        shaft.connect(2, 3);
        graph.set(IVec3::new(0, 1, 0), shaft);
        out.clear();
        graph.visible(Vec3::new(16.0, 80.0, 16.0), 2, 0..3, everything, &mut out);
        assert!(out.contains(&IVec3::new(0, 0, 0)));
        assert!(
            !out.contains(&IVec3::new(1, 0, 0)),
            "plein : on n'y entre pas"
        );
    }

    #[test]
    fn chunks_outside_the_view_are_skipped() {
        let mut graph = VisibilityGraph::default();
        let mut out = Vec::new();
        // Seuls les chunks devant la caméra (x >= 0) sont dans le champ.
        graph.visible(Vec3::splat(16.0), 3, 0..3, |c| c.x >= 0, &mut out);
        assert!(out.iter().all(|c| c.x >= 0));
        assert!(out.contains(&IVec3::new(3, 2, -3)));
        // Tout l'espace devant la caméra : 4 × 3 × 7 chunks.
        assert_eq!(out.len(), 4 * 3 * 7);
    }
}
