//! Le terrain lointain d'un monde v3 (lot 6 du plan v3) : les grands massifs gardent leur relief
//! entier dans une tuile fine, au lieu d'être coupés à plat vers 110 blocs de dénivelé.

use engine_voxel::{FAR_CELLS, FarTile, GenerationVersion, TerrainGenerator, build_tile};

#[test]
fn fine_tiles_keep_the_whole_relief_of_high_mountains() {
    let g = TerrainGenerator::new(42)
        .with_version(GenerationVersion::V3)
        .unwrap();
    let size = FarTile {
        level: 0,
        x: 0,
        z: 0,
    }
    .size_blocks();
    let step = size / FAR_CELLS;
    // Une tuile fine de grand dénivelé, cherchée autour de l'origine (les points de grille du lointain
    // sont tous les 4 blocs).
    let relief = |tile: FarTile| {
        let (ox, oz) = tile.origin_blocks();
        let heights: Vec<i32> = (0..=FAR_CELLS)
            .flat_map(|j| (0..=FAR_CELLS).map(move |i| (i, j)))
            .step_by(7)
            .map(|(i, j)| {
                g.column_at_grid_point((ox + i * step) / 4, (oz + j * step) / 4)
                    .height
            })
            .collect();
        heights.iter().max().unwrap() - heights.iter().min().unwrap()
    };
    let tile = (-40..40)
        .flat_map(|j| {
            (-40..40).map(move |i| FarTile {
                level: 0,
                x: i,
                z: j,
            })
        })
        .find(|&t| relief(t) > 130)
        .expect("une tuile de plus de 130 blocs de dénivelé");
    let mesh = build_tile(&g, tile);
    let (ox, oz) = tile.origin_blocks();
    let ground = ((FAR_CELLS + 1) * (FAR_CELLS + 1)) as usize;
    let world = |k: usize| mesh.origin + mesh.vertices[k].local_position() * mesh.scale;
    let ys: Vec<f32> = (0..ground).map(|k| world(k).y).collect();
    let (lo, hi) = ys
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
    // Les vraies hauteurs aux mêmes points de grille.
    let truth: Vec<f32> = (0..=FAR_CELLS)
        .flat_map(|j| (0..=FAR_CELLS).map(move |i| (i, j)))
        .map(|(i, j)| {
            g.column_at_grid_point((ox + i * step) / 4, (oz + j * step) / 4)
                .height as f32
        })
        .collect();
    let (tlo, thi) = truth
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
    assert!(
        ((hi - lo) - (thi - tlo)).abs() < 1.5,
        "dénivelé du maillage {} contre {} (tuile {tile:?})",
        hi - lo,
        thi - tlo
    );
    // La grille reste à sa place dans le monde.
    for (k, (i, j)) in (0..=FAR_CELLS)
        .flat_map(|j| (0..=FAR_CELLS).map(move |i| (i, j)))
        .enumerate()
    {
        let p = world(k);
        assert!(
            (p.x - (ox + i * step) as f32).abs() < 0.05,
            "x du sommet {k}"
        );
        assert!(
            (p.z - (oz + j * step) as f32).abs() < 0.05,
            "z du sommet {k}"
        );
    }
}
