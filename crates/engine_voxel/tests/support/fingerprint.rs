//! Les empreintes de la génération : un hash stable des blocs d'un chunk, et le corpus fixe de
//! colonnes sur lequel elles sont figées (voir `generation_reference.rs`).
//!
//! Le hash passe par les **identifiants texte** des blocs et des biomes (`core:stone`...), pas par
//! leurs numéros : l'ajout d'un bloc à la fin de `blocks.ron` ne doit pas casser la référence.

use engine_voxel::{BiomeId, BlockId, TerrainGenerator};
use glam::IVec3;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64 bits : simple, stable d'une version de Rust à l'autre.
pub fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes
        .into_iter()
        .fold(FNV_OFFSET, |h, b| (h ^ b as u64).wrapping_mul(FNV_PRIME))
}

fn mix(h: u64, value: u64) -> u64 {
    value
        .to_le_bytes()
        .into_iter()
        .fold(h, |h, b| (h ^ b as u64).wrapping_mul(FNV_PRIME))
}

/// Les hashs des identifiants de blocs et de biomes d'un générateur.
pub struct Keys {
    blocks: Vec<u64>,
    biomes: Vec<u64>,
}

impl Keys {
    pub fn of(generator: &TerrainGenerator) -> Self {
        let registry = generator.registry();
        let mut blocks = vec![0; registry.iter().count()];
        for (id, def) in registry.iter() {
            blocks[id.0 as usize] = fnv(def.key.bytes());
        }
        let mut biomes = vec![0; 256];
        for (id, def) in generator.biomes().iter() {
            biomes[id.0 as usize] = fnv(def.key.bytes());
        }
        Self { blocks, biomes }
    }

    fn block(&self, id: BlockId) -> u64 {
        self.blocks[id.0 as usize]
    }

    fn biome(&self, id: BiomeId) -> u64 {
        self.biomes[id.0 as usize]
    }
}

/// L'empreinte d'un chunk : ses blocs, puis les biomes de ses colonnes. 0 : pas de chunk (de l'air).
pub fn chunk_hash(generator: &TerrainGenerator, keys: &Keys, pos: IVec3) -> u64 {
    let Some(chunk) = generator.generate(pos) else {
        return 0;
    };
    let mut h = FNV_OFFSET;
    for &block in chunk.blocks() {
        h = mix(h, keys.block(block));
    }
    for biome in generator.biome_columns(pos) {
        h = mix(h, keys.biome(BiomeId(biome)));
    }
    h
}

/// L'empreinte de la carte des colonnes (hauteur, biome, températures) sur une grille de
/// 33 × 33 points tous les 37 blocs, de -600 à +584 : les coordonnées négatives y comptent.
pub fn columns_hash(generator: &TerrainGenerator, keys: &Keys) -> u64 {
    let mut h = FNV_OFFSET;
    for i in 0..33 {
        for j in 0..33 {
            let column = generator.column(-600 + i * 37, -600 + j * 37);
            h = mix(h, column.height as u64);
            h = mix(h, keys.biome(column.biome));
            h = mix(h, column.temperature.to_bits() as u64);
            h = mix(h, column.climate_temperature.to_bits() as u64);
        }
    }
    h
}

/// Les graines suivies : 7 et 42 en détail, puis un petit corpus fixe.
pub const MAIN_SEEDS: [u32; 2] = [7, 42];
pub const CORPUS_SEEDS: [u32; 5] = [1, 1337, 20260, 987_654_321, 4_000_000_000];

/// Les colonnes de chunks (cx, cz) vérifiées : l'origine, les coordonnées négatives, la
/// frontière des régions de sauvegarde (16 colonnes), et des colonnes éloignées.
pub const MAIN_COLUMNS: [(i32, i32); 8] = [
    (0, 0),
    (-1, -1),
    (3, 5),
    (-17, 9),
    (15, -16),
    (16, 16),
    (31, -33),
    (-64, 40),
];
pub const CORPUS_COLUMNS: [(i32, i32); 4] = [(0, 0), (-1, -1), (7, -12), (-23, 18)];

/// Les colonnes suivies pour une graine.
pub fn columns_of(seed: u32) -> &'static [(i32, i32)] {
    if MAIN_SEEDS.contains(&seed) {
        &MAIN_COLUMNS
    } else {
        &CORPUS_COLUMNS
    }
}

/// Toutes les graines suivies, dans l'ordre de la table.
pub fn seeds() -> impl Iterator<Item = u32> {
    MAIN_SEEDS.into_iter().chain(CORPUS_SEEDS)
}

/// Les six chunks (de bas en haut) d'une colonne.
pub fn column_hashes(generator: &TerrainGenerator, keys: &Keys, (cx, cz): (i32, i32)) -> [u64; 6] {
    std::array::from_fn(|y| chunk_hash(generator, keys, IVec3::new(cx, y as i32, cz)))
}
