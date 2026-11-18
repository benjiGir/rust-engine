//! Les tests de la génération v2 (lot 4 du plan) : déterminisme, rivières et lacs intacts en
//! **blocs** après toutes les passes, arbres et structures posés sur un sol réel, surface cohérente
//! avec la pente, et empreintes figées (`support/reference_v2.rs`).
//!
//! La v1 est protégée par `generation_reference.rs` (et ne change pas ici). Les empreintes v2 se
//! régénèrent seulement quand la génération v2 est **volontairement** changée (elle n'est pas
//! encore livrée : aucun monde existant ne la porte) :
//! `cargo test -p engine_voxel --test generation_v2 print_reference_v2 -- --ignored --nocapture`.
//! Ne jamais les mettre à jour pour faire passer un test.

mod support {
    pub mod fingerprint;
    pub mod reference_v2;
}

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use engine_voxel::{
    BlockId, Chunk, GenerationVersion, MacroWorld, SEA_LEVEL, StructureKind, TerrainGenerator,
    WORLD_HEIGHT_CHUNKS,
};
use glam::IVec3;
use support::fingerprint::*;
use support::reference_v2::{COLUMN_MAPS, COLUMNS};

fn v2(seed: u32) -> TerrainGenerator {
    TerrainGenerator::new(seed)
        .with_version(GenerationVersion::V2)
        .expect("la génération v2 s'initialise")
}

// ----- Une zone de blocs générés -----

/// Les six chunks (de bas en haut) d'une colonne de chunks.
type Verticale = Vec<Option<Chunk>>;

/// Les colonnes de chunks `cx0..=cx1` × `cz0..=cz1`, générées sur plusieurs threads.
struct Area<'a> {
    generator: &'a TerrainGenerator,
    columns: HashMap<(i32, i32), Verticale>,
    min: (i32, i32),
    max: (i32, i32),
}

impl<'a> Area<'a> {
    fn new(generator: &'a TerrainGenerator, min: (i32, i32), max: (i32, i32)) -> Self {
        let wanted: Vec<(i32, i32)> = (min.1..=max.1)
            .flat_map(|cz| (min.0..=max.0).map(move |cx| (cx, cz)))
            .collect();
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
        let results: Vec<((i32, i32), Verticale)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    let wanted = &wanted;
                    scope.spawn(move || {
                        wanted
                            .iter()
                            .skip(t)
                            .step_by(threads)
                            .map(|&(cx, cz)| {
                                let chunks = (0..WORLD_HEIGHT_CHUNKS)
                                    .map(|cy| generator.generate(IVec3::new(cx, cy, cz)))
                                    .collect();
                                ((cx, cz), chunks)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("thread de génération"))
                .collect()
        });
        Self {
            generator,
            columns: results.into_iter().collect(),
            min: (min.0 * 32, min.1 * 32),
            max: (max.0 * 32 + 31, max.1 * 32 + 31),
        }
    }

    fn contains(&self, x: i32, z: i32) -> bool {
        (self.min.0..=self.max.0).contains(&x) && (self.min.1..=self.max.1).contains(&z)
    }

    /// Le bloc (x, y, z) : de l'air hors de la zone ou du monde.
    fn block(&self, x: i32, y: i32, z: i32) -> BlockId {
        if !(0..WORLD_HEIGHT_CHUNKS * 32).contains(&y) || !self.contains(x, z) {
            return BlockId::AIR;
        }
        self.columns[&(x.div_euclid(32), z.div_euclid(32))][(y / 32) as usize]
            .as_ref()
            .map_or(BlockId::AIR, |c| {
                c.get(
                    x.rem_euclid(32) as usize,
                    (y % 32) as usize,
                    z.rem_euclid(32) as usize,
                )
            })
    }

    fn key(&self, block: BlockId) -> &str {
        &self.generator.registry().def(block).key
    }

    fn is_water(&self, block: BlockId) -> bool {
        self.key(block) == "core:water"
    }

    fn is_solid(&self, block: BlockId) -> bool {
        let def = self.generator.registry().def(block);
        def.solid && !def.liquid
    }

    /// Le dernier bloc d'eau vu en descendant : le premier bloc plein ou liquide rencontré (les
    /// plantes, nénuphars et l'air sont sautés) doit être de l'eau.
    fn water_surface(&self, x: i32, z: i32) -> Option<i32> {
        for y in (0..WORLD_HEIGHT_CHUNKS * 32).rev() {
            let b = self.block(x, y, z);
            if b.is_air() {
                continue;
            }
            let def = self.generator.registry().def(b);
            if def.liquid {
                return self.is_water(b).then_some(y);
            }
            if def.solid {
                return None;
            }
        }
        None
    }

    /// Le bloc plein le plus haut (hors bois et feuillage), et son y.
    fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockId)> {
        for y in (0..WORLD_HEIGHT_CHUNKS * 32).rev() {
            let b = self.block(x, y, z);
            if b.is_air() || !self.is_solid(b) {
                continue;
            }
            let k = self.key(b);
            if k.ends_with("_leaves") || k.ends_with("_log") || k == "core:cactus" {
                continue;
            }
            return Some((y, b));
        }
        None
    }
}

// ----- Où trouver rivières et lacs -----

/// Des points de rivière (largeur d'au moins 4 blocs, loin de la mer) ou de lac, de bassins et de
/// lieux distincts, trouvés sur le relief macro (peu coûteux).
fn find_spots(seed: u32, lakes: bool, wanted: usize) -> Vec<(i32, i32)> {
    let world = MacroWorld::new(seed);
    let mut spots: Vec<(i32, i32)> = Vec::new();
    let mut ring = 0i32;
    while spots.len() < wanted && ring < 90 {
        let r = ring * 96;
        for d in (-r..=r).step_by(96) {
            for (x, z) in [(d, -r), (d, r), (-r, d), (r, d)] {
                let s = world.sample(x as f64, z as f64);
                let hit = if lakes {
                    s.lake.is_some() && s.geo.macro_elevation > SEA_LEVEL as f32 + 8.0
                } else {
                    s.in_water
                        && s.river.is_some_and(|r| r.width >= 4.0)
                        && s.geo.macro_elevation > SEA_LEVEL as f32 + 6.0
                };
                if hit
                    && spots
                        .iter()
                        .all(|&(sx, sz)| (sx - x).abs().max((sz - z).abs()) > 500)
                {
                    spots.push((x, z));
                }
            }
        }
        ring += 1;
    }
    spots
}

/// La zone de 5 × 5 colonnes de chunks centrée sur un point.
fn around(generator: &TerrainGenerator, spot: (i32, i32)) -> Area<'_> {
    around_by(generator, spot, 2)
}

/// La zone de `(2 reach + 1)²` colonnes de chunks centrée sur un point.
fn around_by(generator: &TerrainGenerator, (x, z): (i32, i32), reach: i32) -> Area<'_> {
    let (cx, cz) = (x.div_euclid(32), z.div_euclid(32));
    Area::new(
        generator,
        (cx - reach, cz - reach),
        (cx + reach, cz + reach),
    )
}

const SEEDS: [u32; 4] = [7, 42, 1337, 20260];

// ----- Rivières -----

const NEIGHBOURS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

/// Ce que les blocs montrent des cours d'eau d'une zone : pour chaque colonne qui porte de l'eau de
/// cours (selon le contexte) le niveau de la surface d'eau lue **dans les blocs**.
fn river_surfaces(area: &Area) -> HashMap<(i32, i32), i32> {
    let mut out = HashMap::new();
    for z in area.min.1..=area.max.1 {
        for x in area.min.0..=area.max.0 {
            let info = area.generator.surface_info(x, z).unwrap();
            if info.river_bed && info.water.is_some() {
                if let Some(level) = area.water_surface(x, z) {
                    out.insert((x, z), level);
                } else {
                    panic!("({x}, {z}) : un lit de cours sans eau en surface dans les blocs");
                }
            }
        }
    }
    out
}

/// Les colonnes d'eau (blocs) d'une zone, tous types confondus (mers, lacs, cours).
fn all_water(area: &Area) -> HashMap<(i32, i32), i32> {
    let mut out = HashMap::new();
    for z in area.min.1..=area.max.1 {
        for x in area.min.0..=area.max.0 {
            if let Some(level) = area.water_surface(x, z) {
                out.insert((x, z), level);
            }
        }
    }
    out
}

/// Vrai si l'eau de la colonne `(x, z)` (niveau `level`) peut s'écouler vers un niveau inférieur ou
/// égal en restant dans l'eau : on suit les voisines d'eau de niveau au plus égal, par plateaux,
/// jusqu'à une marche vers le bas, la mer, un lac ou le bord de la zone.
fn drains(water: &HashMap<(i32, i32), i32>, area: &Area, start: (i32, i32)) -> bool {
    let mut seen = HashSet::new();
    let level = water[&start];
    let mut queue = VecDeque::from([start]);
    seen.insert(start);
    while let Some((x, z)) = queue.pop_front() {
        // Une colonne au bord de la zone : le cours continue au-delà.
        if x <= area.min.0 + 1 || x >= area.max.0 - 1 || z <= area.min.1 + 1 || z >= area.max.1 - 1
        {
            return true;
        }
        for (dx, dz) in NEIGHBOURS {
            let n = (x + dx, z + dz);
            match water.get(&n) {
                Some(&l) if l < level => return true,
                Some(&l) if l == level && seen.insert(n) => queue.push_back(n),
                _ => {}
            }
        }
        // Le niveau de la mer est une sortie.
        if level <= SEA_LEVEL {
            return true;
        }
    }
    false
}

#[test]
fn rivers_stay_open_and_never_climb_downstream_in_blocks() {
    let mut checked_columns = 0;
    let mut spots_checked = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in find_spots(seed, false, 3) {
            let area = around(&generator, spot);
            let rivers = river_surfaces(&area);
            if rivers.len() < 20 {
                continue;
            }
            spots_checked += 1;
            let water = all_water(&area);
            for (&(x, z), &level) in &rivers {
                checked_columns += 1;
                let info = generator.surface_info(x, z).unwrap();
                // Le niveau des blocs est celui du contexte.
                assert_eq!(Some(level), info.water, "graine {seed}, ({x}, {z})");
                // Le lit est ouvert : de l'eau du fond à la surface, de l'air (ou une plante) au-dessus.
                for y in info.top + 1..=level {
                    assert!(
                        !area.is_solid(area.block(x, y, z)),
                        "graine {seed}, ({x}, {y}, {z}) : le lit est obstrué ({})",
                        area.key(area.block(x, y, z))
                    );
                }
                for y in level + 1..level + 7 {
                    assert!(
                        !area.is_solid(area.block(x, y, z)),
                        "graine {seed}, ({x}, {y}, {z}) : un bloc plein au-dessus de l'eau d'un cours ({})",
                        area.key(area.block(x, y, z))
                    );
                }
                // La surface d'eau ne remonte jamais vers l'aval : toute colonne d'eau rejoint un
                // niveau inférieur ou égal sans cuvette fermée.
                assert!(
                    drains(&water, &area, (x, z)),
                    "graine {seed}, ({x}, {z}) : une cuvette d'eau à {level} sans sortie"
                );
            }
        }
    }
    assert!(
        spots_checked >= 4,
        "assez de rivières vues : {spots_checked}"
    );
    assert!(
        checked_columns > 500,
        "assez de colonnes : {checked_columns}"
    );
}

#[test]
fn river_banks_are_closed_so_the_water_never_spills_sideways() {
    let mut leaks = Vec::new();
    let mut checked = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in find_spots(seed, false, 3) {
            let area = around(&generator, spot);
            let water = all_water(&area);
            for (&(x, z), &level) in &river_surfaces(&area) {
                checked += 1;
                for (dx, dz) in NEIGHBOURS {
                    let (nx, nz) = (x + dx, z + dz);
                    if !area.contains(nx, nz) || water.get(&(nx, nz)).is_some_and(|&l| l >= level) {
                        continue;
                    }
                    // Une voisine d'eau plus basse est une marche descendante (écoulement vers
                    // l'aval) ; sinon le bloc voisin au niveau de l'eau doit être plein.
                    if water.contains_key(&(nx, nz)) {
                        continue;
                    }
                    if !area.is_solid(area.block(nx, level, nz)) {
                        leaks.push((seed, x, z, nx, nz, level));
                    }
                }
            }
        }
    }
    assert!(checked > 500);
    assert!(
        leaks.is_empty(),
        "{} fuites latérales, par exemple {:?}",
        leaks.len(),
        &leaks[..leaks.len().min(5)]
    );
}

// ----- Lacs -----

#[test]
fn lakes_have_one_level_and_closed_banks_except_at_the_spillway() {
    let mut lakes_seen = 0;
    let mut problems: Vec<String> = Vec::new();
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in find_spots(seed, true, 4) {
            let area = around_by(&generator, spot, 5);
            let water = all_water(&area);
            // Les colonnes de lac, groupées par composantes connexes (4 voisines).
            let mut lake_cols: HashSet<(i32, i32)> = HashSet::new();
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    // Un cours qui traverse le lac a son propre niveau (une marche d'un bloc à l'entrée).
                    if info.lake_bed && !info.river_bed && info.water.is_some() {
                        lake_cols.insert((x, z));
                    }
                }
            }
            let mut seen = HashSet::new();
            for &start in &lake_cols {
                if !seen.insert(start) {
                    continue;
                }
                let mut component = vec![start];
                let mut queue = VecDeque::from([start]);
                while let Some((x, z)) = queue.pop_front() {
                    for (dx, dz) in NEIGHBOURS {
                        let n = (x + dx, z + dz);
                        if lake_cols.contains(&n) && seen.insert(n) {
                            component.push(n);
                            queue.push_back(n);
                        }
                    }
                }
                // Les lacs qui touchent le bord de la zone sont jugés ailleurs.
                if component.iter().any(|&(x, z)| {
                    x <= area.min.0 + 1
                        || x >= area.max.0 - 1
                        || z <= area.min.1 + 1
                        || z >= area.max.1 - 1
                }) || component.len() < 30
                {
                    continue;
                }
                lakes_seen += 1;
                let levels: HashSet<i32> = component
                    .iter()
                    .map(|c| generator.surface_info(c.0, c.1).unwrap().water.unwrap())
                    .collect();
                if levels.len() != 1 {
                    problems.push(format!("graine {seed} : niveaux de lac {levels:?}"));
                    continue;
                }
                let level = *levels.iter().next().unwrap();
                for &(x, z) in &component {
                    if water.get(&(x, z)) != Some(&level) {
                        problems.push(format!(
                            "graine {seed} ({x}, {z}) : surface {:?} au lieu de {level}",
                            water.get(&(x, z))
                        ));
                    }
                    for (dx, dz) in NEIGHBOURS {
                        let n = (x + dx, z + dz);
                        if lake_cols.contains(&n) {
                            continue;
                        }
                        // Une voisine d'eau plus basse est le déversoir (cours qui part du lac).
                        if water.get(&n).is_some_and(|&l| l <= level)
                            || generator.surface_info(n.0, n.1).unwrap().river_bed
                        {
                            continue;
                        }
                        if !area.is_solid(area.block(n.0, level, n.1)) {
                            problems.push(format!(
                                "graine {seed} : rive ouverte en {n:?} (lac à {level})"
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(lakes_seen >= 3, "assez de lacs entiers vus : {lakes_seen}");
    assert!(
        problems.is_empty(),
        "{} problèmes, par exemple {:?}",
        problems.len(),
        &problems[..problems.len().min(5)]
    );
}

// ----- Arbres et structures -----

/// Des zones variées pour juger arbres et structures : rivières, lacs, et un point de terre du
/// corpus pour chaque graine (négatifs compris).
fn mixed_spots(seed: u32) -> Vec<(i32, i32)> {
    let mut spots = find_spots(seed, false, 2);
    spots.extend(find_spots(seed, true, 1));
    spots.push((-400, -300));
    spots
}

/// Vrai si l'ensemble des bûches reliées à `start` (voisines en 26 directions) a une bûche dont le
/// bloc dessous est plein et n'est pas du bois ou du feuillage.
fn supported(area: &Area, start: (i32, i32, i32)) -> bool {
    let is_log = |p: (i32, i32, i32)| area.key(area.block(p.0, p.1, p.2)).ends_with("_log");
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some((x, y, z)) = queue.pop_front() {
        let below = area.block(x, y - 1, z);
        let key = area.key(below);
        if area.is_solid(below) && !key.ends_with("_log") && !key.ends_with("_leaves") {
            return true;
        }
        if seen.len() > 4000 {
            return true;
        }
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let n = (x + dx, y + dy, z + dz);
                    if is_log(n) && seen.insert(n) {
                        queue.push_back(n);
                    }
                }
            }
        }
    }
    false
}

#[test]
fn trees_stand_on_real_soil_and_never_in_the_water_of_a_river_or_lake() {
    let mut trees = 0;
    let mut trunks = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mixed_spots(seed) {
            let area = around(&generator, spot);
            // Les emplacements : sol réel sous le tronc, hors de toute eau et zone réservée.
            for cz in area.min.1.div_euclid(32)..=area.max.1.div_euclid(32) {
                for cx in area.min.0.div_euclid(32)..=area.max.0.div_euclid(32) {
                    for site in generator.tree_sites(cx, cz).iter() {
                        trees += 1;
                        let (x, z) = (site.base.x, site.base.z);
                        let info = generator.surface_info(x, z).unwrap();
                        assert!(
                            info.water.is_none() && !info.river_bed && !info.lake_bed,
                            "graine {seed}, arbre en ({x}, {z}) dans l'eau ou un lit"
                        );
                        assert_eq!(
                            site.base.y,
                            info.top + 1,
                            "graine {seed}, arbre en ({x}, {z}) : pas posé sur le sol réel"
                        );
                    }
                }
            }
            // Les blocs : un tronc repose sur un bloc plein, aucune bûche ni feuille dans l'eau.
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    for y in (0..WORLD_HEIGHT_CHUNKS * 32).rev() {
                        let key = area.key(area.block(x, y, z));
                        let log = key.ends_with("_log");
                        let leaf = key.ends_with("_leaves");
                        if !(log || leaf) {
                            continue;
                        }
                        if let Some(w) = info.water {
                            assert!(
                                y > w,
                                "graine {seed}, ({x}, {y}, {z}) : {key} dans l'eau ({w})"
                            );
                        }
                        if log {
                            trunks += 1;
                            // Un tronc, avec ses branches (des bûches voisines, même en biais),
                            // repose par au moins une bûche sur un bloc plein qui n'est pas du bois.
                            if !area.contains(x - 8, z - 8) || !area.contains(x + 8, z + 8) {
                                continue;
                            }
                            assert!(
                                supported(&area, (x, y, z)),
                                "graine {seed}, ({x}, {y}, {z}) : un arbre flotte"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(trees > 100, "assez d'arbres : {trees}");
    assert!(trunks > 300, "assez de troncs : {trunks}");
}

#[test]
fn structures_keep_away_from_water_beds_and_stand_on_ground() {
    let mut sites_seen = 0;
    for seed in [7, 42, 1337, 20260] {
        let generator = v2(seed);
        let sites = generator.structure_sites_in((-1500, -1500), (1500, 1500));
        for site in sites {
            sites_seen += 1;
            let reach = generator.structures().unwrap().defs()[site.def].radius;
            // Une grille de 5 blocs sur l'emprise : ni eau, ni lit, ni zone réservée.
            let mut dz = -reach;
            while dz <= reach {
                let mut dx = -reach;
                while dx <= reach {
                    let info = generator.surface_info(site.x + dx, site.z + dz).unwrap();
                    // L'eau d'un cours ou d'un lac (la mer, elle, peut border l'emprise d'un
                    // grand rayon, comme en v1).
                    let inland = info.river_bed
                        || info.lake_bed
                        || info.water.is_some_and(|_| info.top >= SEA_LEVEL);
                    assert!(
                        !inland,
                        "graine {seed}, {:?} en ({}, {}) : de l'eau à ({dx}, {dz}) de son centre",
                        site.kind, site.x, site.z
                    );
                    dx += 5;
                }
                dz += 5;
            }
            // Sol réel autour du centre (le centre porte parfois un puits) : un bloc plein sous la
            // surface de quatre colonnes voisines, dans les blocs finals.
            let area = Area::new(
                &generator,
                (site.x.div_euclid(32) - 1, site.z.div_euclid(32) - 1),
                (site.x.div_euclid(32) + 1, site.z.div_euclid(32) + 1),
            );
            if site.kind != StructureKind::Dungeon && site.kind != StructureKind::Mine {
                for (dx, dz) in [(7, 7), (-7, 7), (7, -7), (-7, -7)] {
                    let (x, z) = (site.x + dx, site.z + dz);
                    let info = generator.surface_info(x, z).unwrap();
                    assert!(
                        area.is_solid(area.block(x, info.top - 2, z)),
                        "graine {seed}, structure en ({}, {}) sans sol en ({x}, {z})",
                        site.x,
                        site.z
                    );
                }
            }
        }
    }
    assert!(sites_seen >= 10, "assez de structures vues : {sites_seen}");
}

// ----- v1 inchangée -----

#[test]
fn selecting_v1_explicitly_gives_the_v1_blocks_and_default_stays_v1() {
    // Une v1 « réglée » sur V1 est bit à bit la v1 par défaut : pas de contexte v2, mêmes blocs.
    for seed in [7, 42] {
        let plain = TerrainGenerator::new(seed);
        let switched = TerrainGenerator::new(seed)
            .with_version(GenerationVersion::V1)
            .unwrap();
        assert!(plain.surface_info(0, 0).is_none());
        assert!(switched.surface_info(0, 0).is_none());
        assert_eq!(plain.cached_contexts(), 0);
        let keys = Keys::of(&plain);
        for column in [(0, 0), (-1, -1), (3, 5)] {
            assert_eq!(
                column_hashes(&plain, &keys, column),
                column_hashes(&switched, &keys, column),
                "graine {seed}, {column:?}"
            );
        }
    }
    assert_eq!(GenerationVersion::FOR_NEW_WORLDS, GenerationVersion::V1);
    assert_eq!(GenerationVersion::LEGACY, GenerationVersion::V1);
}

#[test]
fn v2_differs_from_v1_and_the_two_never_share_a_cache() {
    let plain = TerrainGenerator::new(42);
    let new = v2(42);
    let keys = Keys::of(&plain);
    let pos = IVec3::new(0, 2, 0);
    // Les mêmes chunks, deux mondes : on ne confond jamais leurs valeurs.
    assert_ne!(chunk_hash(&plain, &keys, pos), chunk_hash(&new, &keys, pos));
    assert!(new.cached_contexts() >= 1);
    assert_eq!(plain.cached_contexts(), 0);
    // Un clone de la v1 réglé en v2 repart de caches neufs : la v1 d'origine garde ses colonnes.
    let before = plain.biome_columns(IVec3::new(5, 0, 5));
    let _ = plain.clone().with_version(GenerationVersion::V2).unwrap();
    assert_eq!(plain.biome_columns(IVec3::new(5, 0, 5)), before);
}

// ----- Déterminisme -----

/// Les colonnes (de la table v2) en parcours croissant ou décroissant.
fn hashes_in_order(
    generator: &TerrainGenerator,
    keys: &Keys,
    seed: u32,
    order: &[usize],
    descending: bool,
) -> Vec<[u64; 6]> {
    let columns = columns_of(seed);
    let mut out = vec![[0; 6]; columns.len()];
    for &i in order {
        let (cx, cz) = columns[i];
        let ys: Vec<usize> = if descending {
            (0..6).rev().collect()
        } else {
            (0..6).collect()
        };
        for y in ys {
            out[i][y] = chunk_hash(generator, keys, IVec3::new(cx, y as i32, cz));
        }
    }
    out
}

#[test]
fn v2_gives_the_same_blocks_in_any_order_with_any_cache_state() {
    let seed = 42;
    let reference = {
        let generator = v2(seed);
        let keys = Keys::of(&generator);
        let order: Vec<usize> = (0..columns_of(seed).len()).collect();
        hashes_in_order(&generator, &keys, seed, &order, false)
    };
    let generator = v2(seed);
    let keys = Keys::of(&generator);
    let count = columns_of(seed).len();
    let forward: Vec<usize> = (0..count).collect();
    let reverse: Vec<usize> = (0..count).rev().collect();
    // Ordre inverse, du haut vers le bas, cache vide.
    assert_eq!(generator.cached_contexts(), 0);
    assert_eq!(
        hashes_in_order(&generator, &keys, seed, &reverse, true),
        reference,
        "ordre inverse"
    );
    assert!(generator.cached_contexts() > 0);
    // Cache plein.
    assert_eq!(
        hashes_in_order(&generator, &keys, seed, &forward, false),
        reference,
        "cache plein"
    );
    // Cache vidé.
    generator.clear_caches();
    assert_eq!(generator.cached_contexts(), 0);
    assert_eq!(
        hashes_in_order(&generator, &keys, seed, &forward, true),
        reference,
        "cache vidé"
    );
    // Cache évincé : bien au-delà de la borne, avec d'autres colonnes.
    for i in 0..TerrainGenerator::CONTEXT_CACHE_LIMIT as i32 + 100 {
        generator.biome_columns(IVec3::new(2000 + i, 0, 2000));
    }
    assert!(generator.cached_contexts() <= TerrainGenerator::CONTEXT_CACHE_LIMIT);
    assert_eq!(
        hashes_in_order(&generator, &keys, seed, &forward, false),
        reference,
        "cache évincé"
    );
}

#[test]
fn v2_parallel_generation_gives_the_same_blocks() {
    let seed = 7;
    let reference = {
        let generator = v2(seed);
        let keys = Keys::of(&generator);
        let order: Vec<usize> = (0..columns_of(seed).len()).collect();
        hashes_in_order(&generator, &keys, seed, &order, false)
    };
    let generator = Arc::new(v2(seed));
    let keys = Keys::of(&generator);
    let count = columns_of(seed).len();
    // Quatre threads se disputent les mêmes colonnes, chacun dans son ordre.
    let results: Vec<Vec<[u64; 6]>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|thread| {
                let (generator, keys) = (&generator, &keys);
                scope.spawn(move || {
                    let order: Vec<usize> =
                        (0..count).map(|i| (i * 3 + thread * 5) % count).collect();
                    hashes_in_order(generator, keys, seed, &order, thread % 2 == 1)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (thread, result) in results.iter().enumerate() {
        assert_eq!(result, &reference, "thread {thread}");
    }
}

#[test]
fn v2_chunk_columns_do_not_depend_on_their_neighbours_at_edges_and_corners() {
    // Une zone générée d'un seul tenant (voisins en cache) et chaque colonne générée seule, par un
    // générateur neuf (aucun voisin en cache) : mêmes blocs, bords et coins de quatre colonnes
    // compris, en coordonnées négatives et positives.
    for seed in [42, 1337] {
        let together = v2(seed);
        let keys = Keys::of(&together);
        for (cx, cz) in [(-1, -1), (0, 0), (-2, 1), (-17, -16), (15, -16), (16, 16)] {
            for y in 0..6 {
                let pos = IVec3::new(cx, y, cz);
                // Les voisines d'abord, pour que le cache soit plein de leurs contextes.
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)] {
                    let _ = together.generate(IVec3::new(cx + dx, y, cz + dz));
                }
                let alone = v2(seed);
                assert_eq!(
                    chunk_hash(&together, &keys, pos),
                    chunk_hash(&alone, &keys, pos),
                    "graine {seed}, {pos}"
                );
            }
        }
    }
}

#[test]
fn v2_columns_match_the_blocks_across_chunk_seams() {
    // `column(x, z)` est la hauteur du bloc solide le plus haut, aux bords et aux coins comme au
    // milieu ; et le relief n'a pas de couture : les marches aux joints ne dépassent pas celles de
    // l'intérieur des chunks.
    let generator = v2(42);
    let area = Area::new(&generator, (-3, -3), (2, 2));
    let (mut seam, mut seam_n, mut inner, mut inner_n) = (0i64, 0i64, 0i64, 0i64);
    for z in area.min.1..=area.max.1 {
        for x in area.min.0..=area.max.0 {
            let column = generator.column(x, z);
            let top = generator.surface_info(x, z).unwrap().top;
            assert_eq!(column.height, top, "({x}, {z}) : hauteur de colonne");
            if let Some((y, _)) = area.ground(x, z) {
                // Le sol vu depuis le haut est la surface (un arbre ou une structure peut en poser
                // d'autres blocs : on les a exclus).
                assert!(y >= top, "({x}, {z}) : le sol {y} sous la surface {top}");
            }
            if x < area.max.0 {
                let step = (generator.surface_info(x + 1, z).unwrap().top - top).abs() as i64;
                if (x + 1).rem_euclid(32) == 0 {
                    seam += step;
                    seam_n += 1;
                } else {
                    inner += step;
                    inner_n += 1;
                }
            }
        }
    }
    let (seam, inner) = (seam as f64 / seam_n as f64, inner as f64 / inner_n as f64);
    assert!(
        seam <= inner * 1.5 + 0.1,
        "couture : marche moyenne {seam:.2} aux joints contre {inner:.2} à l'intérieur"
    );
}

// ----- Surface selon la pente -----

fn mountain_spots(seed: u32, wanted: usize) -> Vec<(i32, i32)> {
    let world = MacroWorld::new(seed);
    let mut spots: Vec<(i32, i32)> = Vec::new();
    for ring in 0..60 {
        let r = ring * 128;
        for d in (-r..=r).step_by(128) {
            for (x, z) in [(d, -r), (d, r), (-r, d), (r, d)] {
                let g = world.geography().sample(x as f64, z as f64);
                if g.chain_height > 40.0
                    && g.elevation > SEA_LEVEL as f32 + 45.0
                    && spots
                        .iter()
                        .all(|&(sx, sz)| (sx - x).abs().max((sz - z).abs()) > 400)
                {
                    spots.push((x, z));
                    if spots.len() >= wanted {
                        return spots;
                    }
                }
            }
        }
    }
    spots
}

#[test]
fn surface_materials_follow_the_slope() {
    let mut steep = 0;
    let mut steep_rock = 0;
    let mut gentle = 0;
    let mut gentle_rock = 0;
    let mut overhangs = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mountain_spots(seed, 2) {
            let area = around(&generator, spot);
            for z in area.min.1 + 8..=area.max.1 - 8 {
                for x in area.min.0 + 8..=area.max.0 - 8 {
                    let info = generator.surface_info(x, z).unwrap();
                    if info.reserved || info.water.is_some() {
                        continue;
                    }
                    let Some((y, block)) = area.ground(x, z) else {
                        continue;
                    };
                    let key = area.key(block);
                    // Les arbres et structures peuvent poser un bloc : on ne juge que la roche et le sol
                    // de la surface calculée.
                    if y != info.top {
                        continue;
                    }
                    let rocky = matches!(
                        key,
                        "core:stone"
                            | "core:sandstone"
                            | "core:volcanic_rock"
                            | "core:gravel"
                            | "core:coarse_dirt"
                            | "core:deepstone"
                            | "core:cobblestone"
                    );
                    if info.overhang > 0.0 {
                        overhangs += 1;
                    }
                    // Pas d'herbe, de neige ni de sable sur une pente raide.
                    if info.slope >= 1.0 {
                        steep += 1;
                        steep_rock += usize::from(rocky);
                        assert!(
                            !matches!(key, "core:grass" | "core:snow" | "core:sand"),
                            "graine {seed}, ({x}, {z}) : {key} sur une pente de {:.2}",
                            info.slope
                        );
                    } else if info.slope < 0.4 && info.top < SEA_LEVEL + 60 {
                        gentle += 1;
                        gentle_rock += usize::from(rocky);
                    }
                }
            }
        }
    }
    assert!(steep > 300, "assez de pentes raides : {steep}");
    assert!(gentle > 1000, "assez de pentes douces : {gentle}");
    // Les filons de terre ou de gravier (ores.ron) affleurent parfois : au plus 3 % des pentes raides.
    assert!(
        steep_rock as f64 >= steep as f64 * 0.97,
        "pentes raides sans roche : {steep_rock}/{steep}"
    );
    assert!(
        (gentle_rock as f64) < gentle as f64 * 0.35,
        "trop de roche sur les pentes douces : {gentle_rock}/{gentle}"
    );
    assert!(overhangs > 0, "des surplombs existent dans les massifs");
}

#[test]
fn overhangs_exist_only_on_steep_massif_columns_and_never_over_water_beds() {
    let mut overhang_columns = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mountain_spots(seed, 2) {
            let area = around(&generator, spot);
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    if info.overhang > 0.0 {
                        overhang_columns += 1;
                        assert!(
                            !info.reserved && info.water.is_none() && info.slope >= 0.6,
                            "graine {seed}, ({x}, {z}) : surplomb interdit ({info:?})"
                        );
                    }
                    // Aucun bloc au-dessus de la surface calculée en dehors des arbres/structures :
                    // le sol réel est le relief (rien ne flotte).
                    assert!(info.top <= info.relief_top, "({x}, {z}) : {info:?}");
                }
            }
        }
    }
    assert!(overhang_columns > 100, "{overhang_columns}");
}

// ----- Sous-sol -----

#[test]
fn caves_never_open_under_beds_and_water_never_touches_lava() {
    let mut bed_columns = 0;
    let mut lava_blocks = 0;
    let mut cave_water = 0;
    for seed in SEEDS {
        let generator = v2(seed);
        let mut spots = mixed_spots(seed);
        spots.extend(find_spots(seed, true, 1));
        for spot in spots {
            let area = around(&generator, spot);
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    // Sous un lit ou un lac : dix blocs de roche pleine (sauf le fond du monde).
                    if info.river_bed || info.lake_bed {
                        bed_columns += 1;
                        for y in (info.top - 9).max(5)..=info.top {
                            assert!(
                                area.is_solid(area.block(x, y, z)),
                                "graine {seed}, ({x}, {y}, {z}) : un vide sous un lit ({})",
                                area.key(area.block(x, y, z))
                            );
                        }
                    }
                    // L'eau n'est jamais voisine de la lave.
                    for y in 1..64 {
                        let key = area.key(area.block(x, y, z));
                        if key == "core:lava" {
                            lava_blocks += 1;
                            for (dx, dy, dz) in [
                                (1, 0, 0),
                                (-1, 0, 0),
                                (0, 1, 0),
                                (0, -1, 0),
                                (0, 0, 1),
                                (0, 0, -1),
                            ] {
                                assert_ne!(
                                    area.key(area.block(x + dx, y + dy, z + dz)),
                                    "core:water",
                                    "graine {seed}, ({x}, {y}, {z}) : de l'eau contre la lave"
                                );
                            }
                        } else if key == "core:water" && y < info.top - 6 {
                            cave_water += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(bed_columns > 3000, "{bed_columns}");
    assert!(lava_blocks > 100, "assez de lave : {lava_blocks}");
    assert!(cave_water > 100, "des nappes existent : {cave_water}");
}

// ----- Fluides -----

#[test]
fn river_and_lake_water_settles_with_the_fluid_simulation() {
    use engine_voxel::{ChunkStreamer, FluidRegistry, FluidSim, StreamingConfig};
    use glam::Vec3;
    use std::time::Duration;

    for (seed, lakes) in [(7u32, false), (42, true)] {
        let generator = v2(seed);
        let spot = find_spots(seed, lakes, 1)[0];
        let water_id = generator.registry().id("core:water").unwrap();
        let config = StreamingConfig {
            view_radius: 3,
            ..StreamingConfig::default()
        };
        let mut streamer = ChunkStreamer::new(generator.clone(), config);
        let eye = Vec3::new(spot.0 as f32, 150.0, spot.1 as f32);
        let started = std::time::Instant::now();
        loop {
            streamer.update(eye);
            while streamer.next_event().is_some() {}
            if streamer.is_idle() {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "chargement trop long"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut sim = FluidSim::new(streamer.world().registry(), &FluidRegistry::builtin());
        // Toute eau du terrain qui touche de l'air (sur le côté ou dessous) se réveille : c'est ce
        // que ferait le premier bloc cassé ou posé à côté.
        let (cx, cz) = (spot.0.div_euclid(32), spot.1.div_euclid(32));
        let mut woken = 0;
        for x in (cx - 2) * 32..(cx + 3) * 32 {
            for z in (cz - 2) * 32..(cz + 3) * 32 {
                for y in SEA_LEVEL - 8..160 {
                    let p = IVec3::new(x, y, z);
                    if streamer.world().block(p) != water_id {
                        continue;
                    }
                    let open = [
                        IVec3::new(1, 0, 0),
                        IVec3::new(-1, 0, 0),
                        IVec3::new(0, 0, 1),
                        IVec3::new(0, 0, -1),
                        IVec3::new(0, -1, 0),
                    ]
                    .iter()
                    .any(|&o| streamer.world().block(p + o).is_air());
                    if open {
                        sim.wake(p);
                        woken += 1;
                    }
                }
            }
        }
        assert!(
            woken > 20,
            "graine {seed} : peu d'eau à réveiller ({woken})"
        );
        let mut steps = 0;
        while sim.active() > 0 && steps < 4000 {
            sim.step(&mut streamer, Duration::from_secs(1));
            steps += 1;
        }
        println!("graine {seed} : {woken} cellules réveillées, endormi après {steps} étapes");
        assert!(
            sim.active() == 0,
            "graine {seed} : la simulation ne s'endort pas ({} cellules actives après {steps} étapes)",
            sim.active()
        );
    }
}

// ----- Empreintes figées -----

/// Les empreintes de toutes les colonnes suivies d'une graine.
fn current_columns(generator: &TerrainGenerator, keys: &Keys, seed: u32) -> Vec<[u64; 6]> {
    columns_of(seed)
        .iter()
        .map(|&c| column_hashes(generator, keys, c))
        .collect()
}

#[test]
#[ignore = "affiche la table à figer"]
fn print_reference_v2() {
    println!("pub const COLUMNS: &[(u32, i32, i32, [u64; 6])] = &[");
    for seed in seeds() {
        let generator = v2(seed);
        let keys = Keys::of(&generator);
        for (&(cx, cz), hashes) in columns_of(seed)
            .iter()
            .zip(current_columns(&generator, &keys, seed))
        {
            println!("    ({seed}, {cx}, {cz}, {hashes:?}),");
        }
    }
    println!("];");
    println!("pub const COLUMN_MAPS: &[(u32, u64)] = &[");
    for seed in seeds() {
        let generator = v2(seed);
        println!(
            "    ({seed}, {}),",
            columns_hash(&generator, &Keys::of(&generator))
        );
    }
    println!("];");
}

#[test]
fn v2_blocks_match_the_frozen_fingerprints() {
    let mut checked = 0;
    for seed in seeds() {
        let generator = v2(seed);
        let keys = Keys::of(&generator);
        for &(s, cx, cz, expected) in COLUMNS.iter().filter(|c| c.0 == seed) {
            let got = column_hashes(&generator, &keys, (cx, cz));
            assert_eq!(
                got, expected,
                "graine {s}, colonne ({cx}, {cz}) : la génération v2 a changé"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, COLUMNS.len(), "toutes les entrées ont servi");
    assert_eq!(
        COLUMNS.len(),
        2 * 8 + 5 * 4,
        "la table ne doit pas se réduire"
    );
}

#[test]
fn v2_column_maps_match_the_frozen_fingerprints() {
    assert_eq!(COLUMN_MAPS.len(), 7);
    for &(seed, expected) in COLUMN_MAPS {
        let generator = v2(seed);
        let got = columns_hash(&generator, &Keys::of(&generator));
        assert_eq!(
            got, expected,
            "graine {seed} : hauteurs ou biomes v2 changés"
        );
    }
}

/// Les blocs d'air sous un bloc plein de leur colonne et ouverts sur le côté vers l'extérieur : un
/// vrai surplomb (ou une arche), pas une poche fermée.
fn overhang_blocks(area: &Area, x: i32, z: i32, top: i32) -> usize {
    let mut n = 0;
    for y in (top - 20).max(SEA_LEVEL + 2)..top {
        if !area.block(x, y, z).is_air() || !area.is_solid(area.block(x, y + 1, z)) {
            continue;
        }
        // Ouvert sur le côté : une voisine dont le sol est plus bas que ce bloc d'air.
        let open = NEIGHBOURS.iter().any(|&(dx, dz)| {
            area.contains(x + dx, z + dz) && area.ground(x + dx, z + dz).is_some_and(|(g, _)| g < y)
        });
        n += usize::from(open);
    }
    n
}

#[test]
fn massif_cliffs_have_real_overhangs_but_they_stay_rare() {
    let (mut blocks, mut total) = (0, 0);
    for seed in SEEDS {
        let generator = v2(seed);
        for spot in mountain_spots(seed, 3) {
            let area = around(&generator, spot);
            for z in area.min.1..=area.max.1 {
                for x in area.min.0..=area.max.0 {
                    let info = generator.surface_info(x, z).unwrap();
                    total += 1;
                    let n = overhang_blocks(&area, x, z, info.top);
                    if n > 0 {
                        assert!(
                            !info.reserved,
                            "graine {seed}, ({x}, {z}) : un vide ouvert dans une zone réservée"
                        );
                    }
                    blocks += n;
                }
            }
        }
    }
    assert!(blocks >= 40, "des surplombs existent : {blocks} blocs");
    assert!(
        blocks * 50 < total,
        "les surplombs restent rares : {blocks} blocs pour {total} colonnes"
    );
}
