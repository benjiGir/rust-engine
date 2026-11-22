//! Les arbres et les structures du terrain lointain : des volumes simples posés sur les tuiles fines.
//!
//! Le sol lointain ne montre que la teinte des forêts. Ici, les tuiles de niveau 0 et 1 reçoivent en plus
//! une **pyramide** par arbre (la couronne ; le tronc n'est pas dessiné) et, jusqu'au niveau 2, une
//! **maison** (murs et toit) par bâtiment d'un village et un **mur** de pierre par ruine. Les arbres
//! sont les vrais emplacements que plante le générateur ([`TerrainGenerator::tree_sites`]) ; les structures
//! sont les vrais sites ([`TerrainGenerator::structure_sites_in`]), mais leurs bâtiments sont une
//! approximation tirée du site : on voit *qu'il y a un village ici*, pas son plan exact.
//!
//! Chaque volume appartient à la tuile qui contient son centre. Les colonnes modifiées par le joueur
//! n'ont pas d'arbre ni de maison estimés (on ne sait plus ce qu'il y a).

use glam::Vec3;

use crate::edited_surface::EditedSurfaces;
use crate::mesh::MeshVertex;
use crate::worldgen::far::{FarDebug, FarTile};
use crate::worldgen::generator::{SEA_LEVEL, StructureKind, TerrainGenerator};
use crate::worldgen::trees::{Rng64, TreeKind};

/// Au-delà de ce niveau (inclus), plus d'arbres.
pub(crate) const TREE_MAX_LEVEL: u8 = 1;
/// Au-delà de ce niveau (inclus), plus de bâtiments.
pub(crate) const BUILDING_MAX_LEVEL: u8 = 2;
/// Les pyramides des couronnes sont un peu plus petites que les vraies : celles qui recouvrent encore
/// des chunks détaillés restent cachées dans leur feuillage.
const CROWN_SCALE: f32 = 0.85;

/// Un volume à poser : en blocs, dans le monde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Prop {
    /// Une pyramide à base carrée.
    Pyramid {
        x: f32,
        z: f32,
        ground: f32,
        radius: f32,
        base: f32,
        top: f32,
        color: [f32; 3],
    },
    /// Une boîte (murs sans dessous).
    Box {
        x: f32,
        z: f32,
        ground: f32,
        half_x: f32,
        half_z: f32,
        height: f32,
        color: [f32; 3],
    },
}

impl Prop {
    /// Le centre, en blocs.
    fn center(&self) -> (f32, f32) {
        match *self {
            Self::Pyramid { x, z, .. } | Self::Box { x, z, .. } => (x, z),
        }
    }

    /// Le point le plus haut, en blocs.
    pub(crate) fn top(&self) -> f32 {
        match *self {
            Self::Pyramid { ground, top, .. } => ground + top,
            Self::Box { ground, height, .. } => ground + height,
        }
    }
}

/// Le rayon, la hauteur totale et la couleur de la couronne d'un arbre ; `None` : pas dessiné
/// (cactus, buissons).
fn crown(kind: TreeKind) -> Option<(f32, f32, [f32; 3])> {
    Some(match kind {
        TreeKind::Oak => (2.5, 6.0, [0.17, 0.34, 0.11]),
        TreeKind::BigOak => (4.0, 10.0, [0.15, 0.32, 0.10]),
        TreeKind::Birch => (2.0, 7.0, [0.33, 0.48, 0.19]),
        TreeKind::Spruce => (2.0, 9.0, [0.07, 0.21, 0.12]),
        TreeKind::TallSpruce => (2.0, 14.0, [0.06, 0.19, 0.11]),
        TreeKind::Acacia => (3.0, 6.0, [0.38, 0.44, 0.14]),
        TreeKind::Jungle => (4.0, 14.0, [0.08, 0.33, 0.09]),
        TreeKind::Swamp => (3.5, 7.0, [0.18, 0.28, 0.14]),
        TreeKind::Redwood => (3.5, 26.0, [0.10, 0.24, 0.12]),
        TreeKind::Maple => (3.0, 9.0, [0.62, 0.22, 0.07]),
        TreeKind::Fruit => (2.2, 5.5, [0.86, 0.68, 0.74]),
        TreeKind::Cypress => (2.4, 14.0, [0.16, 0.30, 0.12]),
        TreeKind::Fir => (1.8, 15.0, [0.07, 0.18, 0.16]),
        TreeKind::Olive => (3.0, 5.0, [0.30, 0.36, 0.26]),
        TreeKind::TallBirch => (1.8, 11.0, [0.33, 0.48, 0.19]),
        TreeKind::DarkOak => (4.5, 8.0, [0.11, 0.24, 0.08]),
        TreeKind::GiantSpruce => (3.5, 22.0, [0.06, 0.19, 0.11]),
        TreeKind::Cherry => (4.2, 7.5, [0.90, 0.58, 0.70]),
        TreeKind::Cactus
        | TreeKind::Bush
        | TreeKind::JungleBush
        | TreeKind::Boulder
        | TreeKind::YoungOak => {
            return None;
        }
    })
}

/// Les volumes de la tuile (centre dans son rectangle).
pub(crate) fn props_for_tile(
    generator: &TerrainGenerator,
    tile: FarTile,
    edits: Option<&EditedSurfaces>,
) -> Vec<Prop> {
    let mut props = Vec::new();
    let (ox, oz) = tile.origin_blocks();
    let size = tile.size_blocks();
    let inside = |x: f32, z: f32| {
        x >= ox as f32 && x < (ox + size) as f32 && z >= oz as f32 && z < (oz + size) as f32
    };
    let edited = |x: i32, z: i32, ground: i32| {
        edits.is_some_and(|e| e.surface_at(x, z, ground, generator.registry()).is_some())
    };

    if tile.level <= TREE_MAX_LEVEL {
        let chunk = 32;
        for cz in oz.div_euclid(chunk)..=(oz + size - 1).div_euclid(chunk) {
            for cx in ox.div_euclid(chunk)..=(ox + size - 1).div_euclid(chunk) {
                for site in generator.tree_sites(cx, cz).iter() {
                    let Some((radius, height, color)) = crown(site.kind) else {
                        continue;
                    };
                    // Les grands arbres de la v3 (sauf les géants, déjà à leur taille).
                    let (radius, height) = if generator.version() == crate::GenerationVersion::V3
                        && !matches!(
                            site.kind,
                            TreeKind::Jungle
                                | TreeKind::Redwood
                                | TreeKind::Olive
                                | TreeKind::GiantSpruce
                        ) {
                        (radius * 1.15, height * 1.25)
                    } else {
                        (radius, height)
                    };
                    let (x, z) = (site.base.x as f32 + 0.5, site.base.z as f32 + 0.5);
                    if !inside(x, z) || edited(site.base.x, site.base.z, site.base.y - 1) {
                        continue;
                    }
                    let spruce = matches!(
                        site.kind,
                        TreeKind::Spruce | TreeKind::TallSpruce | TreeKind::GiantSpruce
                    );
                    props.push(Prop::Pyramid {
                        x,
                        z,
                        ground: site.base.y as f32,
                        radius: radius * CROWN_SCALE,
                        base: height * if spruce { 0.12 } else { 0.38 },
                        top: height,
                        color,
                    });
                }
            }
        }
    }

    if tile.level <= BUILDING_MAX_LEVEL {
        for site in generator.structure_sites_in((ox, oz), (ox + size - 1, oz + size - 1)) {
            let mut rng = Rng64::new(site.seed ^ 0x0057_A71C);
            match site.kind {
                StructureKind::Village => {
                    let houses = 5 + rng.below(4);
                    for _ in 0..houses {
                        let angle = rng.next_f32() * std::f32::consts::TAU;
                        let distance = 8.0 + rng.next_f32() * 26.0;
                        let (hx, hz) = (
                            site.x as f32 + angle.cos() * distance,
                            site.z as f32 + angle.sin() * distance,
                        );
                        let half_x = 3.0 + rng.below(2) as f32;
                        let half_z = 3.0 + rng.below(2) as f32;
                        let wall = 4.0 + rng.below(2) as f32;
                        if !inside(hx, hz) {
                            continue;
                        }
                        let ground = generator.column(hx as i32, hz as i32).height;
                        if ground < SEA_LEVEL || edited(hx as i32, hz as i32, ground) {
                            continue;
                        }
                        let ground = ground as f32 + 1.0;
                        props.push(Prop::Box {
                            x: hx,
                            z: hz,
                            ground,
                            half_x,
                            half_z,
                            height: wall,
                            color: [0.52, 0.38, 0.21],
                        });
                        props.push(Prop::Pyramid {
                            x: hx,
                            z: hz,
                            ground: ground + wall,
                            radius: half_x.max(half_z) + 0.5,
                            base: 0.0,
                            top: 3.0,
                            color: [0.46, 0.19, 0.12],
                        });
                    }
                }
                StructureKind::Ruin => {
                    let (x, z) = (site.x as f32, site.z as f32);
                    if !inside(x, z) {
                        continue;
                    }
                    let ground = generator.column(site.x, site.z).height;
                    if ground < SEA_LEVEL || edited(site.x, site.z, ground) {
                        continue;
                    }
                    props.push(Prop::Box {
                        x,
                        z,
                        ground: ground as f32 + 1.0,
                        half_x: 4.0,
                        half_z: 4.0,
                        height: 5.0,
                        color: [0.42, 0.42, 0.40],
                    });
                }
                _ => {}
            }
        }
    }
    props
}

/// Écrit un volume dans le maillage de la tuile. `local` convertit une position monde (x, y, z) en
/// position locale de la tuile (en cellules).
pub(crate) fn write_prop(
    prop: &Prop,
    debug: FarDebug,
    level: u8,
    local: &dyn Fn(f32, f32, f32) -> Vec3,
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let tint = |c: [f32; 3]| debug.tint(level, false, c);
    let (cx, cz) = prop.center();
    // Les sommets d'une face, du dehors : le sens est corrigé d'après le centre du volume.
    let mut face = |points: &[Vec3], inside: Vec3, color: [f32; 3]| {
        let n = points
            .windows(3)
            .next()
            .map(|w| (w[1] - w[0]).cross(w[2] - w[0]))
            .unwrap_or(Vec3::Y);
        let centroid = points.iter().copied().sum::<Vec3>() / points.len() as f32;
        let flip = n.dot(centroid - inside) < 0.0;
        let normal = if flip { -n } else { n }.normalize_or_zero();
        let base = vertices.len() as u32;
        let ordered: Vec<Vec3> = if flip {
            points.iter().rev().copied().collect()
        } else {
            points.to_vec()
        };
        for p in &ordered {
            vertices.push(MeshVertex::new(
                *p,
                normal.to_array(),
                [color[0], color[1], color[2], 1.0],
                [1.0, 0.0, 1.0],
                0,
            ));
        }
        for k in 1..ordered.len() as u32 - 1 {
            indices.extend([base, base + k, base + k + 1]);
        }
    };
    match *prop {
        Prop::Pyramid {
            ground,
            radius,
            base,
            top,
            color,
            ..
        } => {
            let color = tint(color);
            let y0 = ground + base;
            let corners = [
                local(cx - radius, y0, cz - radius),
                local(cx + radius, y0, cz - radius),
                local(cx + radius, y0, cz + radius),
                local(cx - radius, y0, cz + radius),
            ];
            let apex = local(cx, ground + top, cz);
            let inside = local(cx, (y0 + ground + top) / 2.0, cz);
            for k in 0..4 {
                face(&[corners[k], corners[(k + 1) % 4], apex], inside, color);
            }
        }
        Prop::Box {
            ground,
            half_x,
            half_z,
            height,
            color,
            ..
        } => {
            let color = tint(color);
            let (x0, x1, z0, z1) = (cx - half_x, cx + half_x, cz - half_z, cz + half_z);
            let (y0, y1) = (ground - 1.0, ground + height);
            let inside = local(cx, (y0 + y1) / 2.0, cz);
            let p = |x, y, z| local(x, y, z);
            face(
                &[p(x0, y0, z0), p(x1, y0, z0), p(x1, y1, z0), p(x0, y1, z0)],
                inside,
                color,
            );
            face(
                &[p(x1, y0, z0), p(x1, y0, z1), p(x1, y1, z1), p(x1, y1, z0)],
                inside,
                color,
            );
            face(
                &[p(x1, y0, z1), p(x0, y0, z1), p(x0, y1, z1), p(x1, y1, z1)],
                inside,
                color,
            );
            face(
                &[p(x0, y0, z1), p(x0, y0, z0), p(x0, y1, z0), p(x0, y1, z1)],
                inside,
                color,
            );
            // Le dessus (au-dessus d'une maison : le toit le cache ; sur une ruine : visible).
            face(
                &[p(x0, y1, z0), p(x1, y1, z0), p(x1, y1, z1), p(x0, y1, z1)],
                inside,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator() -> TerrainGenerator {
        TerrainGenerator::new(42)
    }

    /// La première tuile fine autour de l'origine qui a au moins un arbre dessiné.
    fn forest_tile(generator: &TerrainGenerator) -> FarTile {
        for z in -20..20 {
            for x in -20..20 {
                let tile = FarTile { level: 0, x, z };
                if props_for_tile(generator, tile, None)
                    .iter()
                    .any(|p| matches!(p, Prop::Pyramid { .. }))
                {
                    return tile;
                }
            }
        }
        panic!("aucune forêt trouvée");
    }

    #[test]
    fn trees_come_from_the_generator_and_belong_to_one_tile() {
        let g = generator();
        let tile = forest_tile(&g);
        let props = props_for_tile(&g, tile, None);
        let (ox, oz) = tile.origin_blocks();
        let size = tile.size_blocks() as f32;
        assert!(!props.is_empty());
        for prop in &props {
            let (x, z) = prop.center();
            assert!(x >= ox as f32 && x < ox as f32 + size, "x hors de la tuile");
            assert!(z >= oz as f32 && z < oz as f32 + size, "z hors de la tuile");
        }
        // Chaque couronne est celle d'un vrai arbre du générateur.
        let sites: Vec<_> = (ox.div_euclid(32)..=(ox + 63).div_euclid(32))
            .flat_map(|cx| {
                let g = &g;
                (oz.div_euclid(32)..=(oz + 63).div_euclid(32))
                    .flat_map(move |cz| g.tree_sites(cx, cz).to_vec())
            })
            .collect();
        for prop in &props {
            let Prop::Pyramid { x, z, .. } = prop else {
                continue;
            };
            assert!(
                sites
                    .iter()
                    .any(|s| (s.base.x as f32 + 0.5 - x).abs() < 0.01
                        && (s.base.z as f32 + 0.5 - z).abs() < 0.01),
                "couronne sans arbre"
            );
        }
    }

    #[test]
    fn coarse_levels_have_no_trees() {
        let g = generator();
        let fine = forest_tile(&g);
        // La tuile de niveau 2 qui la contient : aucun arbre (peut avoir des bâtiments).
        let coarse = FarTile {
            level: 2,
            x: fine.x.div_euclid(4),
            z: fine.z.div_euclid(4),
        };
        let (ox, oz) = coarse.origin_blocks();
        let size = coarse.size_blocks();
        let buildings = g
            .structure_sites_in((ox, oz), (ox + size - 1, oz + size - 1))
            .len();
        let props = props_for_tile(&g, coarse, None);
        if buildings == 0 {
            assert!(props.is_empty(), "des arbres au niveau 2");
        }
        // Au niveau 3 (au-delà de 2), jamais rien.
        let far = FarTile {
            level: 3,
            x: coarse.x.div_euclid(2),
            z: coarse.z.div_euclid(2),
        };
        assert!(props_for_tile(&g, far, None).is_empty());
    }

    #[test]
    fn a_village_site_gets_houses_on_fine_and_medium_tiles() {
        let g = generator();
        let mut found = None;
        'scan: for r in 0..40 {
            for (x, z) in [(r * 600, 0), (0, r * 600), (-r * 600, 300), (300, -r * 600)] {
                if let Some(site) = g
                    .structure_sites_near(x, z, 600)
                    .into_iter()
                    .find(|s| s.kind == StructureKind::Village)
                {
                    found = Some(site);
                    break 'scan;
                }
            }
        }
        let site = found.expect("un village autour de l'origine");
        let tile = FarTile {
            level: 2,
            x: site.x.div_euclid(256),
            z: site.z.div_euclid(256),
        };
        let houses = props_for_tile(&g, tile, None)
            .iter()
            .filter(|p| matches!(p, Prop::Box { .. }))
            .count();
        assert!(houses > 0, "aucune maison pour un village");
    }

    #[test]
    fn every_face_points_away_from_its_volume() {
        for prop in [
            Prop::Pyramid {
                x: 10.0,
                z: 10.0,
                ground: 20.0,
                radius: 3.0,
                base: 2.0,
                top: 9.0,
                color: [0.1, 0.4, 0.1],
            },
            Prop::Box {
                x: 10.0,
                z: 10.0,
                ground: 20.0,
                half_x: 3.0,
                half_z: 4.0,
                height: 5.0,
                color: [0.5, 0.4, 0.2],
            },
        ] {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            // Les positions des sommets tiennent de -8 à 56 : on reste dans cet intervalle.
            let local = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
            write_prop(&prop, FarDebug::Off, 0, &local, &mut vertices, &mut indices);
            let middle = Vec3::new(10.0, 20.0 + (prop.top() - 20.0) / 2.0, 10.0);
            assert!(!indices.is_empty());
            for t in indices.chunks(3) {
                let p = |i: u32| vertices[i as usize].local_position();
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let normal = (b - a).cross(c - a);
                let centroid = (a + b + c) / 3.0;
                assert!(
                    normal.dot(centroid - middle) > 0.0,
                    "face tournée vers l'intérieur"
                );
            }
        }
    }
}
