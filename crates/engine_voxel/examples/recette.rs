//! La recette des générations (lot 4 et lot 6 du plan v3) : mêmes mesures de paysage sur
//! plusieurs graines, pour la génération historique (v1) et la v3 par défaut (`--versions 1,3`),
//! imprimées en tableau Markdown.
//!
//! ```text
//! cargo run --release -p engine_voxel --example recette
//! cargo run --release -p engine_voxel --example recette -- --seeds 7,42 --extent 4000 --step 40
//! ```
//!
//! Chaque graine est échantillonnée sur une grille de `extent` × `extent` blocs centrée sur l'origine,
//! tous les `step` blocs (4 000 et 40 par défaut : 10 000 colonnes ; 20 000 et 200 pour les proportions
//! d'un continent entier). Mesures (sur les colonnes) :
//!
//! - **terre** : part des colonnes au-dessus de la mer ; **océan profond** : plus de 25 blocs sous la mer ;
//! - **montagne** : part de la terre à plus de 60 blocs au-dessus de la mer ;
//! - **falaise** : part de la terre dont un voisin à 2 blocs est plus haut ou plus bas d'au moins 6 blocs ;
//! - **marches 1 bloc** : part de la terre dont un voisin direct est exactement 1 bloc plus haut ou plus bas (les
//!   arêtes de terrasse que l'on voit sur les pentes douces) ;
//! - **constructible** : part de la terre dont les 25 points d'un carré de 16 × 16 blocs ne varient pas
//!   de plus de 3 blocs en hauteur, hors eau ;
//! - **forêt** : part de la terre dont la colonne de chunks contient au moins 8 arbres (les vrais tirages du
//!   générateur, `tree_sites`) ; **arbres/chunk** : leur nombre moyen par colonne de chunks de terre ;
//! - **eau douce** : part des colonnes de biome rivière ;
//!   part des colonnes de biome rivière ;
//! - **biomes** : nombre de biomes différents vus sur la terre, et entropie de leur répartition (bits).

use std::collections::HashMap;
use std::sync::Arc;

use engine_voxel::{BiomeKind, GenerationVersion, SEA_LEVEL, TerrainGenerator};

struct Row {
    seed: u32,
    version: &'static str,
    land: f64,
    deep: f64,
    mountain: f64,
    cliff: f64,
    ledges: f64,
    buildable: f64,
    forest: f64,
    trees: f64,
    fresh_water: f64,
    biomes: usize,
    entropy: f64,
    mean_height: f64,
    seconds: f64,
}

fn measure(seed: u32, version: GenerationVersion, extent: i32, step: i32) -> Row {
    let started = std::time::Instant::now();
    let generator = TerrainGenerator::new(seed)
        .with_version(version)
        .expect("version de génération");
    let generator = Arc::new(generator);
    let sea = SEA_LEVEL;
    let (mut total, mut land, mut deep, mut mountain, mut cliff, mut buildable) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut forest, mut fresh, mut height_sum, mut tree_sum) = (0u64, 0u64, 0f64, 0u64);
    let mut ledges = 0u64;
    let mut biomes: HashMap<u8, u64> = HashMap::new();
    let n = extent / step;
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (-extent / 2 + i * step, -extent / 2 + j * step);
            let column = generator.column(x, z);
            total += 1;
            height_sum += f64::from(column.height);
            let biome = generator.biome(column.biome);
            if biome.kind == BiomeKind::River {
                fresh += 1;
            }
            if column.height < sea {
                if column.height < sea - 25 {
                    deep += 1;
                }
                continue;
            }
            land += 1;
            *biomes.entry(column.biome.0).or_default() += 1;
            if column.height > sea + 60 {
                mountain += 1;
            }
            let trees = generator
                .tree_sites(x.div_euclid(32), z.div_euclid(32))
                .len() as u64;
            tree_sum += trees;
            if trees >= 8 {
                forest += 1;
            }
            let h = |dx: i32, dz: i32| generator.column(x + dx, z + dz).height;
            // Une marche d'un bloc vers un voisin direct : l'arête d'une terrasse visible.
            if (h(1, 0) - column.height).abs() == 1 || (h(0, 1) - column.height).abs() == 1 {
                ledges += 1;
            }
            if [(2, 0), (-2, 0), (0, 2), (0, -2)]
                .iter()
                .any(|&(dx, dz)| (h(dx, dz) - column.height).abs() >= 6)
            {
                cliff += 1;
            }
            // Constructible : un carré de 16 × 16, vu en 25 points, sans eau.
            let mut lo = i32::MAX;
            let mut hi = i32::MIN;
            let mut wet = false;
            for dz in (-8..=8).step_by(4) {
                for dx in (-8..=8).step_by(4) {
                    let height = h(dx, dz);
                    lo = lo.min(height);
                    hi = hi.max(height);
                    wet |= height < sea;
                }
            }
            if !wet && hi - lo <= 3 {
                buildable += 1;
            }
        }
    }
    let fraction = |count: u64, of: u64| count as f64 / of.max(1) as f64;
    let entropy = biomes
        .values()
        .map(|&c| {
            let p = fraction(c, land);
            -p * p.log2()
        })
        .sum();
    Row {
        seed,
        version: match version {
            GenerationVersion::V1 => "v1",
            GenerationVersion::V3 => "v3",
            GenerationVersion::V4 => "v4",
        },
        land: fraction(land, total),
        deep: fraction(deep, total),
        mountain: fraction(mountain, land),
        cliff: fraction(cliff, land),
        ledges: fraction(ledges, land),
        buildable: fraction(buildable, land),
        forest: fraction(forest, land),
        trees: tree_sum as f64 / land.max(1) as f64,
        fresh_water: fraction(fresh, total),
        biomes: biomes.len(),
        entropy,
        mean_height: height_sum / total as f64,
        seconds: started.elapsed().as_secs_f64(),
    }
}

fn main() {
    let mut seeds: Vec<u32> = vec![7, 42, 1337, 20260, 987_654_321, 0, 1, 123_456, u32::MAX];
    let (mut extent, mut step) = (4000, 40);
    let mut versions = vec![GenerationVersion::V1, GenerationVersion::V3];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--seeds" => {
                seeds = it
                    .next()
                    .expect("--seeds a,b")
                    .split(',')
                    .map(|s| s.parse().expect("graine"))
                    .collect();
            }
            "--extent" => extent = it.next().expect("--extent N").parse().expect("entier"),
            "--step" => step = it.next().expect("--step N").parse().expect("entier"),
            "--versions" => {
                versions = it
                    .next()
                    .expect("--versions 1,3")
                    .split(',')
                    .map(|v| {
                        GenerationVersion::from_number(v.trim().parse().expect("numéro"))
                            .expect("version")
                    })
                    .collect();
            }
            other => panic!("option inconnue : {other}"),
        }
    }
    let rows: Vec<Row> = std::thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .iter()
            .flat_map(|&seed| versions.iter().map(move |&v| (seed, v)))
            .map(|(seed, v)| scope.spawn(move || measure(seed, v, extent, step)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("mesure"))
            .collect()
    });
    println!(
        "| graine | gén. | terre | océan prof. | montagne | falaise | marches 1 bloc | constructible | forêt | arbres/chunk | eau douce | biomes | entropie | h moy. | s |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    let pct = |x: f64| format!("{:.1} %", x * 100.0);
    for r in &rows {
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.1} | {} | {} | {:.2} | {:.0} | {:.0} |",
            r.seed,
            r.version,
            pct(r.land),
            pct(r.deep),
            pct(r.mountain),
            pct(r.cliff),
            pct(r.ledges),
            pct(r.buildable),
            pct(r.forest),
            r.trees,
            pct(r.fresh_water),
            r.biomes,
            r.entropy,
            r.mean_height,
            r.seconds
        );
    }
}
