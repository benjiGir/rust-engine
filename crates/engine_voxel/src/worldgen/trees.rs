//! Les arbres : des formes procédurales, une par essence.
//!
//! Un arbre se décrit par une fonction qui, à partir de sa base (le bloc au-dessus du
//! sol) et d'un générateur pseudo-aléatoire, énumère ses blocs : tronc, feuilles, lianes.
//! La même fonction sert à la génération du monde et aux pousses qui grandissent.
//!
//! La fonction n'écrit rien elle-même : elle appelle `put` pour chaque bloc. C'est
//! l'appelant qui décide quoi faire (écrire dans un chunk, vérifier la place libre...).

use anyhow::{Context as _, Result};
use glam::IVec3;
use serde::Deserialize;

use crate::block::BlockId;
use crate::registry::BlockRegistry;

/// Les essences et formes d'arbres (et le cactus, qui pousse comme un arbre du désert).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum TreeKind {
    Oak,
    /// Un grand chêne, avec des branches.
    BigOak,
    Birch,
    Spruce,
    Acacia,
    /// Un arbre géant de jungle, tronc de 2 × 2, couvert de lianes.
    Jungle,
    /// Un buisson de jungle : une bûche et une boule de feuilles.
    JungleBush,
    /// Un chêne de marais, large et couvert de lianes.
    Swamp,
    Cactus,
    /// Un buisson bas : une bûche et une petite boule de feuilles de chêne.
    Bush,
    /// Un grand sapin des vieilles forêts : haut et étroit.
    TallSpruce,
    /// Un séquoia géant : tronc de 2 × 2, très haut, couronne en cône dans le dernier tiers.
    Redwood,
    /// Un érable : tronc de chêne, large couronne rouge et orangée.
    Maple,
    /// Les autres teintes de l'érable d'automne (v4) : même arbre, feuillage jaune, or, rouge, rouge
    /// sombre. Le `Maple` d'origine est orangé ; une érablière mêle les cinq, un arbre une teinte.
    MapleYellow,
    MapleGold,
    MapleRed,
    MapleCrimson,
    /// Un arbre fruitier en fleurs : petit tronc, boule de fleurs roses et blanches.
    Fruit,
    /// Un cyprès de bayou : tronc haut et fin, petite couronne, lianes.
    Cypress,
    /// Un sapin des forêts froides : tronc de sapin, cône étroit et serré d'aiguilles bleutées.
    Fir,
    /// Un bloc erratique : un rocher de moellons, moussus par endroits, à demi enfoncé.
    Boulder,
    /// Un olivier : tronc court et tordu, deux ou trois branches, couronnes plates argentées.
    Olive,
    /// Un grand bouleau des vieilles forêts : tronc haut et fin, couronne étroite.
    TallBirch,
    /// Un chêne noir : tronc de 2 × 2, couronne large et plate qui ferme le ciel des forêts sombres.
    DarkOak,
    /// Un épicéa géant des vieilles taïgas : tronc de 2 × 2, très haut.
    GiantSpruce,
    /// Un jeune chêne du sous-bois : un tronc court et une petite boule de feuilles.
    YoungOak,
    /// Un cerisier : un tronc court qui se divise en deux ou trois branches en biais, chacune sous une
    /// large couronne rose dont le bord retombe.
    Cherry,
}

/// Ce qu'est un bloc d'arbre : il décide de ce qu'il a le droit de remplacer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Le tronc remplace l'air, les feuilles et les plantes.
    Log,
    /// Les feuilles et les lianes ne remplacent que l'air.
    Leaves,
}

/// Une essence : sa bûche et ses feuilles.
#[derive(Debug, Clone, Copy)]
pub struct Wood {
    pub log: BlockId,
    pub leaves: BlockId,
}

/// Les blocs de tous les arbres, retrouvés dans le registre.
#[derive(Debug, Clone, Copy)]
pub struct TreeBlocks {
    pub oak: Wood,
    pub birch: Wood,
    pub spruce: Wood,
    pub acacia: Wood,
    pub jungle: Wood,
    /// Les essences de la génération v3 ; `None` dans un registre qui ne les a pas (un registre de
    /// test réduit, ou un jeu d'avant la vague 1) : leurs arbres prennent alors le chêne.
    pub redwood: Option<Wood>,
    pub maple_leaves: Option<BlockId>,
    /// Les feuilles d'érable jaune, or, rouge et rouge sombre (voir [`TreeKind::MapleYellow`]).
    pub maple_hues: [Option<BlockId>; 4],
    pub blossom_leaves: Option<BlockId>,
    /// La vague 2 : les aiguilles de sapin et les rochers (moellons, moellons moussus).
    pub fir_leaves: Option<BlockId>,
    pub boulder: Option<(BlockId, BlockId)>,
    /// La vague 3 : les feuilles d'olivier.
    pub olive_leaves: Option<BlockId>,
    /// Le cerisier (lot N4 de `docs/plan-nature.md`), s'il est dans le registre.
    pub cherry: Option<Wood>,
    pub vine: BlockId,
    /// Les grands arbres de la génération v3 : les essences courantes (chêne, bouleau, épicéa, sapin,
    /// érable...) poussent plus haut et plus large, à l'échelle de son relief. Faux en v1.
    pub tall: bool,
    pub cactus: BlockId,
}

impl TreeBlocks {
    pub fn resolve(registry: &BlockRegistry) -> Result<Self> {
        let id = |key: &str| {
            registry
                .id(key)
                .with_context(|| format!("les arbres ont besoin du bloc « {key} »"))
        };
        let wood = |name: &str| -> Result<Wood> {
            Ok(Wood {
                log: id(&format!("core:{name}_log"))?,
                leaves: id(&format!("core:{name}_leaves"))?,
            })
        };
        Ok(Self {
            oak: wood("oak")?,
            birch: wood("birch")?,
            spruce: wood("spruce")?,
            acacia: wood("acacia")?,
            jungle: wood("jungle")?,
            redwood: wood("redwood").ok(),
            maple_leaves: registry.id("core:maple_leaves"),
            maple_hues: [
                registry.id("core:maple_yellow_leaves"),
                registry.id("core:maple_gold_leaves"),
                registry.id("core:maple_red_leaves"),
                registry.id("core:maple_crimson_leaves"),
            ],
            blossom_leaves: registry.id("core:blossom_leaves"),
            fir_leaves: registry.id("core:fir_leaves"),
            boulder: registry
                .id("core:cobblestone")
                .zip(registry.id("core:mossy_cobblestone")),
            olive_leaves: registry.id("core:olive_leaves"),
            cherry: wood("cherry").ok(),
            tall: false,
            vine: id("core:vine")?,
            cactus: id("core:cactus")?,
        })
    }

    /// L'essence d'une forme d'arbre.
    pub fn wood(&self, kind: TreeKind) -> Wood {
        match kind {
            TreeKind::Oak
            | TreeKind::BigOak
            | TreeKind::Swamp
            | TreeKind::Bush
            | TreeKind::DarkOak
            | TreeKind::YoungOak => self.oak,
            TreeKind::Birch | TreeKind::TallBirch => self.birch,
            TreeKind::Spruce | TreeKind::TallSpruce | TreeKind::GiantSpruce => self.spruce,
            TreeKind::Acacia => self.acacia,
            TreeKind::Jungle | TreeKind::JungleBush => self.jungle,
            TreeKind::Redwood => self.redwood.unwrap_or(self.spruce),
            TreeKind::Cherry => self.cherry.unwrap_or(Wood {
                log: self.oak.log,
                leaves: self.blossom_leaves.unwrap_or(self.oak.leaves),
            }),
            TreeKind::Maple => Wood {
                log: self.oak.log,
                leaves: self.maple_leaves.unwrap_or(self.oak.leaves),
            },
            TreeKind::MapleYellow
            | TreeKind::MapleGold
            | TreeKind::MapleRed
            | TreeKind::MapleCrimson => {
                let hue = match kind {
                    TreeKind::MapleYellow => 0,
                    TreeKind::MapleGold => 1,
                    TreeKind::MapleRed => 2,
                    _ => 3,
                };
                Wood {
                    log: self.oak.log,
                    leaves: self.maple_hues[hue]
                        .or(self.maple_leaves)
                        .unwrap_or(self.oak.leaves),
                }
            }
            TreeKind::Fruit => Wood {
                log: self.oak.log,
                leaves: self.blossom_leaves.unwrap_or(self.oak.leaves),
            },
            TreeKind::Cypress => Wood {
                log: self.spruce.log,
                leaves: self.oak.leaves,
            },
            TreeKind::Olive => Wood {
                log: self.oak.log,
                leaves: self.olive_leaves.unwrap_or(self.oak.leaves),
            },
            TreeKind::Fir => Wood {
                log: self.spruce.log,
                leaves: self.fir_leaves.unwrap_or(self.spruce.leaves),
            },
            // La « bûche » du rocher : des moellons (le moussu est choisi bloc par bloc).
            TreeKind::Boulder => {
                let (stone, mossy) = self.boulder.unwrap_or((self.cactus, self.cactus));
                Wood {
                    log: stone,
                    leaves: mossy,
                }
            }
            TreeKind::Cactus => Wood {
                log: self.cactus,
                leaves: self.cactus,
            },
        }
    }

    /// La forme d'arbre qui pousse d'une pousse (`core:oak_sapling` → chêne).
    pub fn kind_of_sapling(key: &str) -> Option<TreeKind> {
        Some(match key {
            "core:oak_sapling" => TreeKind::Oak,
            "core:birch_sapling" => TreeKind::Birch,
            "core:spruce_sapling" => TreeKind::Spruce,
            "core:acacia_sapling" => TreeKind::Acacia,
            "core:jungle_sapling" => TreeKind::JungleBush,
            "core:redwood_sapling" => TreeKind::Redwood,
            "core:maple_sapling" => TreeKind::Maple,
            "core:fir_sapling" => TreeKind::Fir,
            "core:olive_sapling" => TreeKind::Olive,
            "core:cherry_sapling" => TreeKind::Cherry,
            _ => return None,
        })
    }
}

/// Un générateur pseudo-aléatoire (splitmix64) : rapide, et la même suite pour la même graine.
#[derive(Debug, Clone)]
pub struct Rng64(u64);

impl Rng64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Un entier dans `0..n`.
    pub fn below(&mut self, n: u32) -> u32 {
        (((self.next_u64() >> 32) * n as u64) >> 32) as u32
    }

    /// Un entier dans `low..=high`.
    pub fn range(&mut self, low: i32, high: i32) -> i32 {
        low + self.below((high - low + 1) as u32) as i32
    }
}

/// Le rayon horizontal maximal d'un arbre autour de sa base : un arbre ne déborde jamais
/// au-delà du chunk voisin (32 blocs), condition pour la génération par chunk.
pub const MAX_RADIUS: i32 = 8;

/// Le gain de hauteur au plus d'un arbre serré (voir [`grow_in_grove`]).
pub const MAX_BOOST: i32 = 3;

/// Énumère les blocs d'un arbre de forme `kind`, dont le tronc part de `base`.
pub fn grow(
    kind: TreeKind,
    base: IVec3,
    rng: &mut Rng64,
    blocks: &TreeBlocks,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    grow_in_grove(kind, base, 0, rng, blocks, put);
}

/// Comme [`grow`], pour un arbre serré dans un bosquet : il monte de `boost` blocs de plus (au plus
/// [`MAX_BOOST`]), comme les arbres d'une forêt qui cherchent la lumière.
pub fn grow_in_grove(
    kind: TreeKind,
    base: IVec3,
    boost: i32,
    rng: &mut Rng64,
    blocks: &TreeBlocks,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    let wood = blocks.wood(kind);
    let tall = blocks.tall;
    let boost = boost.clamp(0, MAX_BOOST);
    // La hauteur d'une essence : `(bas, haut)` en v1, `(bas2, haut2)` pour les grands arbres v3,
    // plus le gain d'un arbre serré.
    let pick = |rng: &mut Rng64, small: (i32, i32), large: (i32, i32)| {
        let (low, high) = if tall { large } else { small };
        rng.range(low, high) + boost
    };
    match kind {
        // En v1, le chêne et le bouleau gardent exactement leur forme d'origine (ses mondes ne
        // changent jamais) ; en v3, des couronnes irrégulières.
        TreeKind::Oak if !tall => {
            let height = pick(rng, (4, 6), (5, 8));
            round_tree(base, height, if height >= 7 { 3 } else { 2 }, wood, put)
        }
        TreeKind::Oak => {
            let height = pick(rng, (4, 6), (5, 9));
            oak(base, height, rng, wood, put)
        }
        TreeKind::Birch if !tall => round_tree(base, pick(rng, (5, 7), (6, 9)), 2, wood, put),
        TreeKind::Birch => {
            let height = pick(rng, (5, 7), (6, 9));
            rough_tree(base, height, 2, rng, wood, put)
        }
        TreeKind::TallBirch => {
            let height = pick(rng, (8, 11), (10, 14));
            tall_birch(base, height, rng, wood, put)
        }
        TreeKind::DarkOak => {
            let height = pick(rng, (5, 7), (6, 9));
            dark_oak(base, height, rng, wood, put)
        }
        TreeKind::GiantSpruce => {
            let height = pick(rng, (17, 22), (21, 26));
            giant_spruce(base, height, rng, wood, put)
        }
        TreeKind::YoungOak => {
            let height = rng.range(2, 3);
            for y in 0..height {
                put(base + IVec3::Y * y, wood.log, Part::Log);
            }
            blob(
                base + IVec3::Y * height,
                1.5 + rng.below(3) as f32 * 0.2,
                wood.leaves,
                put,
            );
        }
        TreeKind::BigOak => big_oak(base, rng, tall, wood, put),
        TreeKind::Cherry => {
            let trunk = pick(rng, (2, 3), (3, 4));
            cherry(base, trunk, rng, tall, wood, put)
        }
        TreeKind::Spruce => {
            let height = pick(rng, (7, 12), (9, 15));
            spruce(base, height, rng, wood, put)
        }
        TreeKind::TallSpruce => {
            let height = pick(rng, (15, 21), (18, 24));
            spruce(base, height, rng, wood, put)
        }
        TreeKind::Bush => {
            put(base, wood.log, Part::Log);
            let r = 1.4 + rng.below(3) as f32 * 0.3;
            blob(base + IVec3::Y, r, wood.leaves, put);
        }
        TreeKind::Acacia => acacia(base, rng, tall, wood, put),
        TreeKind::Jungle => jungle(base, rng, wood, blocks.vine, put),
        TreeKind::JungleBush => {
            put(base, wood.log, Part::Log);
            blob(base + IVec3::Y, 2.2, wood.leaves, put);
        }
        TreeKind::Swamp => {
            let height = pick(rng, (5, 6), (6, 8));
            round_tree(base, height, 3, wood, put);
            // Des lianes pendent du bord des feuilles.
            for _ in 0..8 {
                let (dx, dz) = (rng.range(-3, 3), rng.range(-3, 3));
                if dx.abs() == 3 || dz.abs() == 3 {
                    let top = base + IVec3::new(dx, height - 3, dz);
                    for k in 0..rng.range(1, 4) {
                        put(top - IVec3::Y * k, blocks.vine, Part::Leaves);
                    }
                }
            }
        }
        TreeKind::Redwood => redwood(base, rng, wood, put),
        TreeKind::Maple
        | TreeKind::MapleYellow
        | TreeKind::MapleGold
        | TreeKind::MapleRed
        | TreeKind::MapleCrimson => {
            let height = pick(rng, (6, 8), (8, 10));
            let crown = if tall { 3.9 } else { 3.3 };
            for y in 0..height {
                put(base + IVec3::Y * y, wood.log, Part::Log);
            }
            blob(base + IVec3::Y * (height - 1), crown, wood.leaves, put);
            blob(
                base + IVec3::new(0, height + 1, 0),
                crown - 1.1,
                wood.leaves,
                put,
            );
        }
        TreeKind::Fruit => {
            let height = pick(rng, (3, 4), (4, 5));
            for y in 0..height {
                put(base + IVec3::Y * y, wood.log, Part::Log);
            }
            blob(
                base + IVec3::Y * height,
                if tall { 2.9 } else { 2.4 },
                wood.leaves,
                put,
            );
        }
        TreeKind::Cypress => {
            let height = pick(rng, (10, 14), (12, 16));
            for y in 0..height {
                put(base + IVec3::Y * y, wood.log, Part::Log);
            }
            // Des « genoux » : de petites racines qui sortent du sol autour du tronc.
            for (dx, dz) in [(2, 0), (-2, 1), (1, -2)] {
                if rng.below(2) == 0 {
                    put(base + IVec3::new(dx, 0, dz), wood.log, Part::Log);
                }
            }
            blob(base + IVec3::Y * height, 2.6, wood.leaves, put);
            blob(base + IVec3::Y * (height - 3), 2.0, wood.leaves, put);
            for _ in 0..6 {
                let (dx, dz) = (rng.range(-2, 2), rng.range(-2, 2));
                if dx.abs() == 2 || dz.abs() == 2 {
                    let top = base + IVec3::new(dx, height - 2, dz);
                    for k in 0..rng.range(2, 5) {
                        put(top - IVec3::Y * k, blocks.vine, Part::Leaves);
                    }
                }
            }
        }
        TreeKind::Fir => fir(base, rng, tall, wood, put),
        TreeKind::Olive => olive(base, rng, wood, put),
        TreeKind::Boulder => {
            if blocks.boulder.is_some() {
                boulder(base, rng, wood, put);
            }
        }
        TreeKind::Cactus => {
            for k in 0..rng.range(1, 3) {
                put(base + IVec3::Y * k, blocks.cactus, Part::Log);
            }
        }
    }
}

/// L'arbre « classique » : un tronc, quatre couches de feuilles (deux larges, deux
/// étroites) dont les coins sont parfois absents.
fn round_tree(
    base: IVec3,
    height: i32,
    radius: i32,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    for layer in 0..4 {
        let y = height - 3 + layer;
        let r = if layer < 2 { radius } else { radius - 1 };
        for dx in -r..=r {
            for dz in -r..=r {
                // Les coins des couches : arrondis (un coin sur deux, toujours le même
                // pour un arbre donné grâce au motif de la position).
                let corner = dx.abs() == r && dz.abs() == r;
                if corner && (layer == 3 || (dx + dz + y).rem_euclid(2) == 0) {
                    continue;
                }
                put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
            }
        }
    }
    put(base + IVec3::Y * height, wood.leaves, Part::Leaves);
}

/// Le chêne : la moitié du temps une couronne en couches aux bords irréguliers, l'autre moitié une
/// couronne en grappes (une grosse boule au sommet, deux ou trois plus petites au bout de courtes
/// branches) : deux chênes voisins ne se ressemblent plus.
fn oak(
    base: IVec3,
    height: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    if height < 6 || rng.below(2) == 0 {
        return rough_tree(
            base,
            height,
            if height >= 7 { 3 } else { 2 },
            rng,
            wood,
            put,
        );
    }
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    let top = base + IVec3::Y * height;
    blob(top, 2.4 + rng.below(4) as f32 * 0.2, wood.leaves, put);
    let directions = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ];
    let first = rng.below(8) as usize;
    for k in 0..rng.range(2, 3) as usize {
        let (dx, dz) = directions[(first + k * 3) % 8];
        let from = rng.range(height / 2 + 1, height - 1);
        let length = rng.range(1, 2);
        let mut p = base + IVec3::Y * from;
        for step in 1..=length {
            p = base + IVec3::new(dx * step, from + step, dz * step);
            put(p, wood.log, Part::Log);
        }
        blob(p, 1.7 + rng.below(3) as f32 * 0.25, wood.leaves, put);
    }
}

/// La couronne en couches de [`round_tree`], aux bords irréguliers : des feuilles manquent au bord
/// des couches basses, d'autres dépassent ; deux arbres de même forme n'ont plus la même silhouette.
fn rough_tree(
    base: IVec3,
    height: i32,
    radius: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    for layer in 0..4 {
        let y = height - 3 + layer;
        let r = if layer < 2 { radius } else { radius - 1 };
        for dx in -r - 1..=r + 1 {
            for dz in -r - 1..=r + 1 {
                let edge = dx.abs().max(dz.abs());
                let corner = dx.abs() == r && dz.abs() == r;
                let keep = if edge > r {
                    // Une feuille qui dépasse, au milieu des côtés des couches basses.
                    layer < 2 && dx.abs().min(dz.abs()) <= r / 2 && rng.below(5) == 0
                } else if edge == r {
                    // Le bord : les coins manquent souvent, les côtés parfois.
                    if corner {
                        layer < 3 && rng.below(3) == 0
                    } else {
                        rng.below(6) != 0
                    }
                } else {
                    true
                };
                if keep {
                    put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
    put(base + IVec3::Y * height, wood.leaves, Part::Leaves);
}

/// Le cerisier : un tronc court, puis deux ou trois branches qui montent en biais (deux pas en diagonale,
/// puis un ou deux qui s'écartent), chacune sous une couronne en dôme large et aplatie ; une couronne
/// plus petite coiffe le tronc. Au bord du bas des couronnes, des feuilles retombent d'un ou deux
/// blocs : la silhouette en parasol des cerisiers.
fn cherry(
    base: IVec3,
    trunk: i32,
    rng: &mut Rng64,
    tall: bool,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..trunk {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    let fork = base + IVec3::Y * (trunk - 1);
    let directions = [
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
    ];
    let first = rng.below(8) as usize;
    let branches = rng.range(2, 3) as usize;
    let mut crowns = Vec::new();
    for k in 0..branches {
        // Des branches bien écartées : réparties autour du tronc.
        let (dx, dz) = directions[(first + k * 8 / branches + rng.below(2) as usize) % 8];
        let mut p = fork;
        let rise = rng.range(3, 4) + i32::from(tall);
        for step in 1..=rise {
            let out = if step <= 2 || rng.below(2) == 0 { 1 } else { 0 };
            p += IVec3::new(dx * out, 1, dz * out);
            put(p, wood.log, Part::Log);
        }
        crowns.push(p + IVec3::Y);
    }
    crowns.push(fork + IVec3::Y * 3);
    let last = crowns.len() - 1;
    for (i, &center) in crowns.iter().enumerate() {
        let r = if i == last {
            2.2
        } else {
            3.0 + rng.below(3) as f32 * 0.25
        };
        let n = r.ceil() as i32;
        for dy in -1..=2 {
            // Un dôme aplati : large en bas, qui se resserre vers le haut.
            let layer = r - (dy.max(0) as f32) * 0.9 + if dy < 0 { -0.4 } else { 0.0 };
            for dx in -n..=n {
                for dz in -n..=n {
                    let d = ((dx * dx + dz * dz) as f32).sqrt();
                    if d > layer {
                        continue;
                    }
                    let at = center + IVec3::new(dx, dy, dz);
                    put(at, wood.leaves, Part::Leaves);
                    // Au bord du bas, des feuilles retombent.
                    if dy == -1 && d > layer - 1.0 && rng.below(3) == 0 {
                        for k in 1..=rng.range(1, 2) {
                            put(at - IVec3::Y * k, wood.leaves, Part::Leaves);
                        }
                    }
                }
            }
        }
    }
}

/// Le grand bouleau : un tronc haut et fin, une couronne étroite et longue sur son dernier tiers.
fn tall_birch(
    base: IVec3,
    height: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    let bottom = height - 5;
    for y in bottom..=height + 1 {
        // Étroite en bas et en haut, plus large au milieu.
        let r: i32 = if y <= bottom || y >= height { 1 } else { 2 };
        for dx in -r..=r {
            for dz in -r..=r {
                let corner = dx.abs() == r && dz.abs() == r;
                if (corner && (r == 1 || rng.below(3) != 0)) || (dx, dz) == (0, 0) && y < height {
                    continue;
                }
                put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
            }
        }
    }
}

/// Le chêne noir : un tronc de 2 × 2, quelques racines ou branches basses, et une couronne large et
/// plate (trois couches) qui touche celle des voisins.
fn dark_oak(
    base: IVec3,
    height: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            put(base + IVec3::new(dx, y, dz), wood.log, Part::Log);
        }
    }
    // Des racines au pied, et une ou deux branches qui partent sous la couronne.
    for (dx, dz) in [(-1, 0), (2, 1), (1, -1), (0, 2)] {
        if rng.below(3) == 0 {
            put(base + IVec3::new(dx, 0, dz), wood.log, Part::Log);
        }
    }
    let crown = base + IVec3::Y * height;
    for _ in 0..rng.range(1, 2) {
        let (dx, dz) = [(2, 0), (-1, 1), (1, 2), (0, -1)][rng.below(4) as usize];
        put(crown + IVec3::new(dx, -2, dz), wood.log, Part::Log);
        put(crown + IVec3::new(dx, -1, dz), wood.log, Part::Log);
    }
    for (layer, r) in [(-1, 4.2f32), (0, 4.6), (1, 3.4), (2, 1.8)] {
        let n = r.ceil() as i32;
        for dx in -n..=n + 1 {
            for dz in -n..=n + 1 {
                let (cx, cz) = (dx as f32 - 0.5, dz as f32 - 0.5);
                let d = cx * cx + cz * cz;
                // Un bord irrégulier : la dernière couronne de feuilles manque par endroits.
                if d <= r * r && (d < (r - 1.0) * (r - 1.0) || rng.below(4) != 0) {
                    put(crown + IVec3::new(dx, layer, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
}

/// L'épicéa géant : un tronc de 2 × 2, nu sur son premier tiers, puis un cône de couches alternées
/// qui s'élargit vers le bas (rayon 5 au plus), et une flèche.
fn giant_spruce(
    base: IVec3,
    height: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            put(base + IVec3::new(dx, y, dz), wood.log, Part::Log);
        }
    }
    for k in 0..2 {
        put(base + IVec3::Y * (height + k), wood.leaves, Part::Leaves);
    }
    let lowest = height / 3 + rng.range(0, 2);
    for y in lowest..height {
        let from_top = (height - y) as f32;
        let r = (1.0 + from_top * 0.3).min(5.0) - if y % 2 == 0 { 0.0 } else { 1.1 };
        let r = r.max(1.0);
        let n = r.ceil() as i32;
        for dx in -n..=n + 1 {
            for dz in -n..=n + 1 {
                let (cx, cz) = (dx as f32 - 0.5, dz as f32 - 0.5);
                if cx * cx + cz * cz <= r * r {
                    put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
}

/// Une boule de feuilles de rayon `r` centrée sur `center`.
fn blob(center: IVec3, r: f32, leaves: BlockId, put: &mut impl FnMut(IVec3, BlockId, Part)) {
    let n = r.ceil() as i32;
    for dy in -n..=n {
        for dx in -n..=n {
            for dz in -n..=n {
                let d = (dx * dx + dz * dz) as f32 + (dy * dy) as f32 * 1.6;
                if d <= r * r {
                    put(center + IVec3::new(dx, dy, dz), leaves, Part::Leaves);
                }
            }
        }
    }
}

fn big_oak(
    base: IVec3,
    rng: &mut Rng64,
    tall: bool,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    let height = if tall {
        rng.range(10, 14)
    } else {
        rng.range(8, 12)
    };
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    blob(
        base + IVec3::Y * height,
        if tall { 3.7 } else { 3.2 },
        wood.leaves,
        put,
    );
    // Trois ou quatre branches qui finissent chacune par une boule de feuilles.
    for _ in 0..rng.range(3, 4) {
        let start = rng.range(height / 2, height - 2);
        let (dx, dz) = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
        ][rng.below(8) as usize];
        let length = rng.range(2, 4);
        let mut p = base + IVec3::Y * start;
        for k in 1..=length {
            p = base + IVec3::new(dx * k, start + k / 2, dz * k);
            put(p, wood.log, Part::Log);
        }
        blob(p + IVec3::Y, 2.3, wood.leaves, put);
    }
}

/// Le sapin : un tronc droit et des couches de feuilles en cône, larges puis étroites.
fn spruce(
    base: IVec3,
    height: i32,
    rng: &mut Rng64,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    put(base + IVec3::Y * height, wood.leaves, Part::Leaves);
    put(base + IVec3::Y * (height + 1), wood.leaves, Part::Leaves);
    let lowest = rng.range(2, 3);
    for (i, y) in (lowest..height).rev().enumerate() {
        // Le rayon grandit en descendant, avec une couche étroite sur deux.
        let r = ((i as i32 + 2) / 2).min(3) - (i as i32 % 2);
        let r = r.max(1);
        for dx in -r..=r {
            for dz in -r..=r {
                if dx * dx + dz * dz <= r * r + 1 && (dx, dz) != (0, 0) {
                    put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
}

/// Le sapin des forêts froides : plus haut et plus serré que l'épicéa, des étages courts dont le rayon
/// grandit lentement vers le bas (au plus 2), une flèche au sommet.
fn fir(
    base: IVec3,
    rng: &mut Rng64,
    tall: bool,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    let height = if tall {
        rng.range(13, 19)
    } else {
        rng.range(11, 17)
    };
    for y in 0..height {
        put(base + IVec3::Y * y, wood.log, Part::Log);
    }
    for k in 0..3 {
        put(base + IVec3::Y * (height + k), wood.leaves, Part::Leaves);
    }
    let lowest = rng.range(2, 4);
    for (i, y) in (lowest..height).rev().enumerate() {
        // Des étages de trois couches : large, moyenne, étroite ; le large s'élargit vers le bas.
        let step = i as i32 % 3;
        let tier = (i as i32 / 3 + 1).min(2);
        let r = (tier - step / 2).max(1);
        for dx in -r..=r {
            for dz in -r..=r {
                let corner = dx.abs() == r && dz.abs() == r;
                if (dx, dz) != (0, 0) && !(corner && (r == 1 || step != 0)) {
                    put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
}

/// L'olivier : un tronc court qui penche, puis deux ou trois branches courtes en biais, chacune
/// sous une couronne aplatie ; une couronne au-dessus du tronc.
fn olive(base: IVec3, rng: &mut Rng64, wood: Wood, put: &mut impl FnMut(IVec3, BlockId, Part)) {
    let height = rng.range(2, 4);
    let (lx, lz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.below(4) as usize];
    let mut top = base;
    for y in 0..height {
        top = base
            + IVec3::new(
                if y == height - 1 { lx } else { 0 },
                y,
                if y == height - 1 { lz } else { 0 },
            );
        put(top, wood.log, Part::Log);
    }
    let flat = |center: IVec3, r: f32, put: &mut dyn FnMut(IVec3, BlockId, Part)| {
        let n = r.ceil() as i32;
        for dy in 0..=1 {
            let r = if dy == 0 { r } else { r - 1.0 };
            for dx in -n..=n {
                for dz in -n..=n {
                    if (dx * dx + dz * dz) as f32 <= r * r {
                        put(center + IVec3::new(dx, dy, dz), wood.leaves, Part::Leaves);
                    }
                }
            }
        }
    };
    let directions = [(1, 1), (-1, 1), (1, -1), (-1, -1)];
    let first = rng.below(4) as usize;
    for k in 0..rng.range(2, 3) as usize {
        let (dx, dz) = directions[(first + k) % 4];
        let mut p = top;
        for step in 1..=2 {
            p = top + IVec3::new(dx * step, step, dz * step);
            put(p, wood.log, Part::Log);
        }
        flat(p + IVec3::Y, 2.3, put);
    }
    flat(top + IVec3::Y * 2, 2.0, put);
}

/// Un bloc erratique : une boule de moellons écrasée, centrée un peu sous le sol (seule la partie hors
/// du sol est posée, le tronc ne remplaçant que l'air et les plantes), moussue par endroits.
fn boulder(base: IVec3, rng: &mut Rng64, wood: Wood, put: &mut impl FnMut(IVec3, BlockId, Part)) {
    let r = 1.8 + rng.below(4) as f32 * 0.4;
    let center = base - IVec3::Y;
    let n = r.ceil() as i32;
    for dy in 0..=n {
        for dx in -n..=n {
            for dz in -n..=n {
                let d = (dx * dx + dz * dz) as f32 + (dy * dy) as f32 * 1.4;
                if d <= r * r {
                    let mossy = dy > 0 && rng.below(3) == 0;
                    let block = if mossy { wood.leaves } else { wood.log };
                    put(center + IVec3::new(dx, dy, dz), block, Part::Log);
                }
            }
        }
    }
}

/// L'acacia : un tronc qui se tord, et une couronne plate.
fn acacia(
    base: IVec3,
    rng: &mut Rng64,
    tall: bool,
    wood: Wood,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    let height = if tall {
        rng.range(6, 9)
    } else {
        rng.range(5, 7)
    };
    let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.below(4) as usize];
    let bend = height - 2;
    let mut top = base;
    for y in 0..height {
        let shift = (y - bend + 1).max(0);
        top = base + IVec3::new(dx * shift, y, dz * shift);
        put(top, wood.log, Part::Log);
    }
    for ddx in -3..=3i32 {
        for ddz in -3..=3i32 {
            if ddx.abs() + ddz.abs() <= 4 {
                put(top + IVec3::new(ddx, 1, ddz), wood.leaves, Part::Leaves);
            }
            if ddx.abs() + ddz.abs() <= 2 {
                put(top + IVec3::new(ddx, 2, ddz), wood.leaves, Part::Leaves);
            }
        }
    }
}

/// L'arbre géant de la jungle : un tronc de 2 × 2, une couronne large, des lianes.
/// Le séquoia : un tronc de 2 × 2 très haut, nu sur ses deux premiers tiers, puis des couches de
/// feuilles en cône serré.
fn redwood(base: IVec3, rng: &mut Rng64, wood: Wood, put: &mut impl FnMut(IVec3, BlockId, Part)) {
    let height = rng.range(20, 27);
    for y in 0..height {
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            put(base + IVec3::new(dx, y, dz), wood.log, Part::Log);
        }
    }
    let lowest = height * 2 / 3 - 2;
    for y in lowest..height + 3 {
        // Le rayon diminue vers le haut, en couches alternées pour un cône dentelé.
        let from_top = (height + 3 - y) as f32;
        let r = (from_top * 0.38).min(4.2) - if y % 2 == 0 { 0.0 } else { 0.8 };
        if r < 0.6 {
            put(base + IVec3::new(0, y, 0), wood.leaves, Part::Leaves);
            continue;
        }
        let n = r.ceil() as i32;
        for dx in -n..=n + 1 {
            for dz in -n..=n + 1 {
                let (cx, cz) = (dx as f32 - 0.5, dz as f32 - 0.5);
                if cx * cx + cz * cz <= r * r {
                    put(base + IVec3::new(dx, y, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
}

fn jungle(
    base: IVec3,
    rng: &mut Rng64,
    wood: Wood,
    vine: BlockId,
    put: &mut impl FnMut(IVec3, BlockId, Part),
) {
    let height = rng.range(14, 24);
    for y in 0..height {
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            put(base + IVec3::new(dx, y, dz), wood.log, Part::Log);
        }
    }
    let crown = base + IVec3::new(0, height, 0);
    for (layer, r) in [(-2, 4.5f32), (-1, 4.8), (0, 4.0), (1, 2.8)] {
        let n = r.ceil() as i32;
        for dx in -n..=n + 1 {
            for dz in -n..=n + 1 {
                let (cx, cz) = (dx as f32 - 0.5, dz as f32 - 0.5);
                if cx * cx + cz * cz <= r * r {
                    put(crown + IVec3::new(dx, layer, dz), wood.leaves, Part::Leaves);
                }
            }
        }
    }
    // Des lianes le long du tronc et sous la couronne.
    for _ in 0..14 {
        let side = rng.below(4);
        let y = rng.range(2, height - 3);
        let (vx, vz) = match side {
            0 => (-1, rng.range(0, 1)),
            1 => (2, rng.range(0, 1)),
            2 => (rng.range(0, 1), -1),
            _ => (rng.range(0, 1), 2),
        };
        for k in 0..rng.range(2, 6) {
            put(base + IVec3::new(vx, y - k, vz), vine, Part::Leaves);
        }
    }
    for _ in 0..10 {
        let (dx, dz) = (rng.range(-4, 5), rng.range(-4, 5));
        let edge = dx.abs().max(dz.abs()) >= 3;
        if edge {
            for k in 0..rng.range(2, 7) {
                put(crown + IVec3::new(dx, -3 - k, dz), vine, Part::Leaves);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les formes de la vague 1 restent dans les bornes que la génération par chunk suppose : au plus
    /// `MAX_RADIUS` blocs de côté et 30 de haut (voir `features.rs`).
    #[test]
    fn new_tree_shapes_stay_within_the_chunk_bounds() {
        let registry = BlockRegistry::builtin();
        let blocks = TreeBlocks::resolve(&registry).unwrap();
        assert!(blocks.redwood.is_some() && blocks.maple_leaves.is_some());
        for kind in [
            TreeKind::Redwood,
            TreeKind::Maple,
            TreeKind::Fruit,
            TreeKind::Cypress,
            TreeKind::Fir,
            TreeKind::Olive,
            TreeKind::Oak,
            TreeKind::Birch,
            TreeKind::TallBirch,
            TreeKind::DarkOak,
            TreeKind::GiantSpruce,
            TreeKind::YoungOak,
            TreeKind::Cherry,
        ] {
            for seed in 0..40 {
                let mut rng = Rng64::new(seed);
                let mut count = 0;
                grow(kind, IVec3::ZERO, &mut rng, &blocks, &mut |p, _, _| {
                    count += 1;
                    assert!(
                        p.x.abs() <= MAX_RADIUS && p.z.abs() <= MAX_RADIUS,
                        "{kind:?} : {p}"
                    );
                    assert!((0..=30).contains(&p.y), "{kind:?} : {p}");
                });
                assert!(count > 10, "{kind:?} : trop petit");
            }
        }
        assert_eq!(
            TreeBlocks::kind_of_sapling("core:redwood_sapling"),
            Some(TreeKind::Redwood)
        );
        assert_eq!(
            TreeBlocks::kind_of_sapling("core:maple_sapling"),
            Some(TreeKind::Maple)
        );
        assert_eq!(
            TreeBlocks::kind_of_sapling("core:fir_sapling"),
            Some(TreeKind::Fir)
        );
        assert_eq!(
            TreeBlocks::kind_of_sapling("core:olive_sapling"),
            Some(TreeKind::Olive)
        );
    }

    /// Les grands arbres de la v3 : plus hauts que ceux de la v1 (en moyenne), et toujours dans les
    /// bornes de la génération par chunk.
    #[test]
    fn tall_trees_are_taller_and_stay_within_the_chunk_bounds() {
        let registry = BlockRegistry::builtin();
        let small = TreeBlocks::resolve(&registry).unwrap();
        let tall = TreeBlocks {
            tall: true,
            ..small
        };
        let top = |blocks: &TreeBlocks, kind: TreeKind, seed: u64| {
            let mut rng = Rng64::new(seed);
            let mut highest = 0;
            grow(kind, IVec3::ZERO, &mut rng, blocks, &mut |p, _, _| {
                assert!(
                    p.x.abs() <= MAX_RADIUS && p.z.abs() <= MAX_RADIUS,
                    "{kind:?} : {p}"
                );
                // Les lianes du chêne des marais pendent sous sa base (comme en v1).
                assert!((-3..=30).contains(&p.y), "{kind:?} : {p}");
                highest = highest.max(p.y);
            });
            highest
        };
        for kind in [
            TreeKind::Oak,
            TreeKind::Birch,
            TreeKind::BigOak,
            TreeKind::Spruce,
            TreeKind::TallSpruce,
            TreeKind::Acacia,
            TreeKind::Swamp,
            TreeKind::Maple,
            TreeKind::Fruit,
            TreeKind::Cypress,
            TreeKind::Fir,
        ] {
            let (mut a, mut b) = (0, 0);
            for seed in 0..60 {
                a += top(&small, kind, seed);
                b += top(&tall, kind, seed);
            }
            assert!(b > a, "{kind:?} : {b} contre {a}");
        }
    }

    /// Le rocher : petit, à demi enfoncé (une couche sous la base au plus), de moellons et de moellons
    /// moussus, jamais de mousse dans la partie enterrée.
    #[test]
    fn boulders_are_small_half_buried_cobblestone_mounds() {
        let registry = BlockRegistry::builtin();
        let blocks = TreeBlocks::resolve(&registry).unwrap();
        let (stone, mossy) = blocks.boulder.expect("les moellons sont dans le registre");
        let mut mossy_seen = false;
        for seed in 0..40 {
            let mut rng = Rng64::new(seed);
            let mut above = 0;
            grow(
                TreeKind::Boulder,
                IVec3::ZERO,
                &mut rng,
                &blocks,
                &mut |p, block, part| {
                    assert_eq!(part, Part::Log);
                    assert!(
                        p.x.abs() <= 3 && p.z.abs() <= 3 && (-1..=2).contains(&p.y),
                        "{p}"
                    );
                    assert!(block == stone || (block == mossy && p.y >= 0), "{p}");
                    mossy_seen |= block == mossy;
                    above += usize::from(p.y >= 0);
                },
            );
            assert!(above >= 3, "rocher trop petit : {above}");
        }
        assert!(mossy_seen);
    }
}
