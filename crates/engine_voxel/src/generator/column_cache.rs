//! Le cache des colonnes : ce que les six chunks d'une même verticale ont en commun.
//!
//! La hauteur, le biome et les températures d'une colonne (x, z) ne dépendent pas de l'altitude
//! du chunk. Avant ce cache, chacun des six chunks d'une verticale recalculait la grille de bruits
//! de 9 × 9 points et les 1 024 colonnes interpolées : le plus gros poste de la génération d'un
//! chunk d'altitude (environ 250 µs sur 650).
//!
//! # Garanties
//!
//! - **Une valeur, jamais une décision** : le cache ne décide de rien, il garde le résultat exact
//!   de [`TerrainGenerator::compute_column_grid`]. Les empreintes de
//!   `tests/generation_reference.rs` prouvent que les blocs ne changent pas, cache vide ou plein.
//! - **Clés explicites** : graine, genre de monde (dimension), version de génération et
//!   coordonnées de la colonne de chunks. Deux générateurs qui se partageraient un cache sans
//!   avoir la même configuration ne pourraient pas se voir leurs colonnes.
//! - **Instantanés immuables** : un [`ColumnGrid`] est rendu dans un `Arc` et ne change jamais ;
//!   un chunk en cours de génération le garde même si le cache l'évince entre-temps.
//! - **Pas de verrou pendant le calcul** : le `Mutex` ne protège que la table (recherche,
//!   insertion, éviction), quelques dizaines de nanosecondes. Le calcul se fait dans une
//!   `OnceLock` propre à la clé : deux workers qui demandent la même colonne en même temps ne la
//!   calculent qu'une fois (le second attend le premier, pas les autres colonnes), et le thread
//!   de rendu n'y touche jamais pendant le chargement.
//! - **Mémoire bornée** : [`COLUMN_CACHE_LIMIT`] colonnes, les plus anciennes évincées d'abord.

use std::collections::VecDeque;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, OnceLock};

use rustc_hash::FxHashMap;

use super::{Column, SEA_LEVEL};
use crate::chunk::CHUNK_SIZE;
use crate::generation::GenerationVersion;

/// Nombre maximal de colonnes gardées : 17 Kio chacune, soit environ 17 Mio. Une verticale est
/// consommée par ses six chunks à peu près en même temps (la file de chargement est triée par
/// distance) : une trentaine de colonnes sont « en cours » à la fois, bien en deçà de cette borne.
pub(super) const COLUMN_CACHE_LIMIT: usize = 1024;

/// Ce qui identifie une colonne de chunks dans le cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ColumnKey {
    pub seed: u32,
    /// L'empreinte du genre de monde et des biomes de terre permis : la dimension, vue du
    /// générateur (voir `TerrainGenerator::dimension_tag`).
    pub dimension: u64,
    pub version: GenerationVersion,
    pub cx: i32,
    pub cz: i32,
}

/// Les données des 32 × 32 colonnes d'une colonne de chunks, communes à ses six chunks.
#[derive(Debug)]
pub(super) struct ColumnGrid {
    /// `[x][z]`, comme `TerrainGenerator::columns`.
    pub columns: [[Column; CHUNK_SIZE]; CHUNK_SIZE],
    /// Les biomes, `x + 32 z`, comme `Chunk::set_biomes` les attend.
    pub biomes: Vec<u8>,
    /// La hauteur du sol la plus haute de la colonne de chunks.
    pub max_height: i32,
    /// L'altitude la plus haute où une grotte peut être creusée dans la colonne de chunks : un
    /// chunk entièrement au-dessus n'a pas besoin des bruits de grottes.
    pub max_cave_ceiling: i32,
}

impl ColumnGrid {
    pub fn new(columns: [[Column; CHUNK_SIZE]; CHUNK_SIZE]) -> Self {
        let mut biomes = vec![0; CHUNK_SIZE * CHUNK_SIZE];
        let (mut max_height, mut max_cave_ceiling) = (i32::MIN, i32::MIN);
        for (x, row) in columns.iter().enumerate() {
            for (z, column) in row.iter().enumerate() {
                biomes[x + z * CHUNK_SIZE] = column.biome.0;
                max_height = max_height.max(column.height);
                max_cave_ceiling = max_cave_ceiling.max(cave_ceiling(column));
            }
        }
        Self {
            columns,
            biomes,
            max_height,
            max_cave_ceiling,
        }
    }
}

/// L'altitude au-dessus de laquelle une colonne n'a pas de grotte. Pas de grotte juste sous
/// l'eau (elle viderait la mer, qui ne coule pas encore), ni juste sous la surface (pour ne pas
/// trouer le sol partout).
pub(super) fn cave_ceiling(column: &Column) -> i32 {
    if column.height < SEA_LEVEL + 2 {
        column.height - 9
    } else {
        column.height - 4
    }
}

/// Une colonne en cours de calcul ou calculée.
type Slot = Arc<OnceLock<Arc<ColumnGrid>>>;

#[derive(Debug, Default)]
struct Table {
    slots: FxHashMap<ColumnKey, Slot>,
    /// L'ordre d'insertion, pour évincer la plus ancienne.
    order: VecDeque<ColumnKey>,
}

/// Le cache. Partagé entre les clones d'un générateur et entre les workers.
#[derive(Debug, Default)]
pub(super) struct ColumnCache {
    table: Mutex<Table>,
}

impl ColumnCache {
    /// La colonne `key`, calculée par `compute` si elle manque. Le verrou de la table n'est tenu
    /// ni pendant `compute` ni pendant l'attente d'un autre worker.
    pub fn get_or_compute(
        &self,
        key: ColumnKey,
        compute: impl FnOnce() -> ColumnGrid,
    ) -> Arc<ColumnGrid> {
        let slot = {
            let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
            let Table { slots, order } = &mut *table;
            match slots.entry(key) {
                Entry::Occupied(entry) => entry.get().clone(),
                Entry::Vacant(entry) => {
                    let slot = entry.insert(Slot::default()).clone();
                    order.push_back(key);
                    if order.len() > COLUMN_CACHE_LIMIT
                        && let Some(oldest) = order.pop_front()
                    {
                        slots.remove(&oldest);
                    }
                    slot
                }
            }
        };
        slot.get_or_init(|| Arc::new(compute())).clone()
    }

    /// Vide le cache. Les instantanés déjà rendus restent valides.
    pub fn clear(&self) {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        table.slots.clear();
        table.order.clear();
    }

    /// Le nombre de colonnes gardées (calculées ou en cours de calcul).
    pub fn len(&self) -> usize {
        self.table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .slots
            .len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::biome::BiomeId;

    fn key(cx: i32) -> ColumnKey {
        ColumnKey {
            seed: 1,
            dimension: 0,
            version: GenerationVersion::V1,
            cx,
            cz: 0,
        }
    }

    fn grid(height: i32) -> ColumnGrid {
        ColumnGrid::new(
            [[Column {
                height,
                biome: BiomeId(3),
                temperature: 0.5,
                climate_temperature: 0.5,
            }; CHUNK_SIZE]; CHUNK_SIZE],
        )
    }

    #[test]
    fn a_column_is_computed_once_and_shared() {
        let cache = ColumnCache::default();
        let calls = AtomicUsize::new(0);
        let compute = || {
            calls.fetch_add(1, Ordering::SeqCst);
            grid(70)
        };
        let a = cache.get_or_compute(key(0), compute);
        let b = cache.get_or_compute(key(0), || panic!("déjà calculée"));
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(a.max_height, 70);
        assert_eq!(a.biomes.len(), CHUNK_SIZE * CHUNK_SIZE);
        assert!(a.biomes.iter().all(|&b| b == 3));
    }

    #[test]
    fn the_key_separates_seed_dimension_version_and_position() {
        let cache = ColumnCache::default();
        let base = key(0);
        let others = [
            ColumnKey { seed: 2, ..base },
            ColumnKey {
                dimension: 9,
                ..base
            },
            ColumnKey { cx: 1, ..base },
            ColumnKey { cz: -1, ..base },
        ];
        cache.get_or_compute(base, || grid(10));
        for (i, other) in others.into_iter().enumerate() {
            let got = cache.get_or_compute(other, || grid(20 + i as i32));
            assert_eq!(
                got.max_height,
                20 + i as i32,
                "clé distincte, valeur distincte"
            );
        }
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn memory_is_bounded_and_evicted_snapshots_stay_valid() {
        let cache = ColumnCache::default();
        let first = cache.get_or_compute(key(0), || grid(1));
        for cx in 1..=COLUMN_CACHE_LIMIT as i32 + 10 {
            cache.get_or_compute(key(cx), || grid(2));
        }
        assert_eq!(cache.len(), COLUMN_CACHE_LIMIT);
        assert_eq!(first.max_height, 1, "l'instantané évincé reste lisible");
        // La plus ancienne a été évincée : elle se recalcule.
        let again = cache.get_or_compute(key(0), || grid(1));
        assert!(!Arc::ptr_eq(&first, &again));
        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn concurrent_requests_for_one_column_compute_it_once() {
        let cache = ColumnCache::default();
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    cache.get_or_compute(key(7), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        grid(5)
                    });
                });
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn caves_stop_below_the_surface_and_lower_under_the_sea() {
        let column = |height| Column {
            height,
            ..Column::default()
        };
        assert_eq!(cave_ceiling(&column(100)), 96);
        assert_eq!(cave_ceiling(&column(SEA_LEVEL + 2)), SEA_LEVEL - 2);
        assert_eq!(cave_ceiling(&column(SEA_LEVEL + 1)), SEA_LEVEL - 8);
    }
}
