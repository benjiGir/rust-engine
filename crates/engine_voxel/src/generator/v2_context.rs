//! La construction du contexte d'une colonne de chunks (génération v2) : le relief macro, les
//! cours et les lacs réservés, les berges fermées, les biomes provisoires, les règles de surface,
//! les surplombs, la nappe et les cavernes.
//!
//! # Du relief macro aux blocs
//!
//! 1. **Géographie** évaluée tous les 4 blocs (absolus) sur la zone large, interpolée au bloc.
//! 2. **Cours et lacs** : les arêtes de drainage et les lacs de la zone sont lus une fois
//!    ([`Hydrology::river_candidates`], [`Hydrology::lakes_near`]) puis évalués colonne par colonne,
//!    exactement comme [`MacroWorld::sample`](super::MacroWorld::sample) (même `river_near`, mêmes
//!    formules de vallée) avec deux différences : le remblai des berges monte jusqu'à [`FILL`] blocs
//!    au lieu de 2, et les lits sont creusés **en blocs** (voir ci-dessous).
//! 3. **Quantification** : le relief devient `top`, le dernier bloc plein : `ceil(h - 0,5)`.
//! 4. **Lits** : l'eau d'un cours est à `floor(niveau)` (l'arrondi vers le bas conserve la
//!    décroissance du profil : l'eau ne monte jamais vers l'aval) ; le lit est creusé d'au moins un
//!    bloc sous elle au centre (de deux à trois pour un grand cours), jamais au-dessus à la rive.
//!    L'eau d'un lac est à `floor(niveau du lac)` partout.
//! 5. **Berges fermées** : une colonne de terre dont une voisine (4 directions) porte de l'eau plus
//!    haut qu'elle est relevée à ce niveau. L'eau d'un lac ou d'un cours ne déborde donc jamais d'un
//!    bloc : le seul écoulement est celui du cours vers l'aval (et le déversoir d'un lac).
//! 6. **Réservation** : un poids de 0 à 1, plein dans le lit, sur les berges (3 blocs au-delà de la
//!    demi-largeur), dans les lacs et sur leur rive, décroissant sur 6 blocs. Il annule les escarpements,
//!    pilote la profondeur des cavernes, la distance de dégagement des arbres et des structures.
//! 7. **Escarpements** : dans les massifs et leurs contreforts (chaîne forte, altitude), le relief saute
//!    de 10 à 44 blocs sur des parois posées sur la structure macro (flancs de chaîne, crêtes
//!    secondaires, failles : `scarp.rs`), en quelques blocs de large : de vraies parois. Le pied d'une
//!    partie d'entre elles est évidé (`density.rs`) : surplombs, alcôves, et des arches là où une crête
//!    fine relie deux masses. Jamais dans une vallée, un lit ou une zone réservée.
//!
//! La colonne de chunks voisine calcule les mêmes valeurs dans sa marge : tout est une fonction pure
//! de la position, jamais de l'ordre ou de l'état d'un cache.
//!
//! # Climat, biomes et végétation (lot 5)
//!
//! Le climat est lu **après** le relief et l'hydrologie, tous les 32 blocs sur la grille absolue (une colonne de chunks : quatre points) (voir
//! `climate.rs` : température régionale refroidie par l'altitude, humidité de la mer, de la
//! continentalité, des versants au vent et de l'ombre pluviométrique), puis interpolé. Les effets
//! locaux s'y ajoutent colonne par colonne :
//!
//! - **marais** (`wetland`) : une terre basse, plate, humide, en taches, loin du rivage. Les biomes de
//!   zone humide (climat d'humidité de 0,7 ou plus) ne sont candidats que là où `wetland` dépasse 0,4 ;
//! - **eau proche** : +0,45 d'humidité près d'un cours ou d'un lac. Elle ne compte que pour la
//!   végétation (voir plus bas), pas pour le choix du biome : un cours n'ouvre pas une bande de forêt
//!   d'une autre variété ;
//! - **rivage** : plage jusqu'à 70 blocs du rivage sur un sol bas et doux ; côte rocheuse (variété de
//!   `biomes.ron`) là où la pente, lue sur 8 blocs, dépasse 0,22 à 0,45 : le sable ne s'alterne pas
//!   avec la roche d'une dune à l'autre.
//!
//! La famille du biome vient du relief macro : lit d'un cours ou d'un lac -> rivière (le biome de
//! rivière suit donc le réseau réel) ; sous la mer -> océan (profond si la géographie dit « mer
//! profonde ») ; rivage bas -> plage ; massif en altitude -> montagnes (pics au-dessus de `SEA + 85`) ;
//! sinon terre. Dans chaque famille, le biome est choisi par son climat comme en v1. Océans et plages
//! ne dépendent que de la température ; les rivières n'ont qu'un biome.
//!
//! **Les `relief` des biomes de `biomes.ron` ne servent pas en v2.** Le relief se décide avant le
//! biome (géographie macro, drainage, escarpements) : un biome qui le modulerait fermerait un cycle
//! (le biome dépend de l'altitude, qui dépendrait du biome) et redessinerait des vallées protégées
//! après coup. Ils restent lus par la v1. Une adaptation possible plus tard : un biome compatible
//! module l'amplitude du détail fin dans une famille de relief donnée, jamais dans une zone
//! réservée, avec la température et l'humidité lues avant (donc sans retour sur elles).
//!
//! **Végétation** (`canopy_factor`) : un coefficient par colonne qui multiplie la densité d'arbres du
//! biome (humidité relative au climat du biome, pente, ligne des arbres selon la température, berges).
//! Les règles d'apparition et de ressources (essences par biome, sols, groupes) restent celles de la
//! v1 ; les lisières viennent du gradient d'humidité et des bosquets (`groves`) du biome.

use super::climate::WorldClimate;
use super::density;
use super::hydrology::RiverCandidate;
use super::macro_noise::{fbm, smooth as smooth64};
use super::scarp::{Scarp, Terrain, limit_reach, scarp};
use super::surface::{CLIFF_SLOPE, SurfaceInputs};
use super::v2::{ColumnContext, CoreCell, FILL, MARGIN, NO_WATER, V2State, WIDE, WideCell, flag};
use super::{
    Column, LAVA_LEVEL, Lake, MAX_HEIGHT, SEA_LEVEL, TerrainGenerator, VALLEY_REACH, fine_cell,
    value_noise,
};
use crate::biome::BiomeKind;
use crate::chunk::CHUNK_SIZE;

/// Géographie interpolée en un point.
#[derive(Debug, Clone, Copy)]
struct Geo {
    elevation: f32,
    macro_elevation: f32,
    chain: f32,
    coast: f32,
    /// Le détail de massif ajouté au relief (blocs, signé) : voir [`massif_detail`].
    detail: f32,
    /// L'escarpement : voir [`Scarp`].
    scarp: Scarp,
}

/// Points de la grille de géographie par côté de la zone large (un point tous les 4 blocs).
const GEO_POINTS: usize = WIDE / 4 + 1;
/// Distance (blocs) de la rive d'un lac jusqu'où son remblai agit.
const LAKE_REACH: f32 = 56.0;

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    smooth64(edge0 as f64, edge1 as f64, x as f64) as f32
}

/// Le détail de massif : des bosses douces (d'une vingtaine de blocs de haut, de 90 blocs de long) et un
/// relief rugueux plus fin, d'une intensité qui varie lentement (des versants doux, d'autres chaotiques).
/// Ni gradins réguliers (leurs paliers donnaient aux grands sommets l'allure de courbes de niveau) ni
/// arêtes tirées d'un bruit (une crête où un bruit s'annule serpente en boucles et en vers : c'est ce
/// que faisait le premier jet, et ces arêtes couvraient les cartes de matériau de bandes de roche) :
/// les vraies ruptures de pente sont les escarpements ([`scarp`], posés sur la structure macro).
/// Ajouté au relief là où la chaîne est forte et au-dessus de la mer, atténué près des cours et des
/// lacs (voir `wide_cell`).
fn massif_detail(seed: u32, x: f64, z: f64, chain: f32, elevation: f32) -> f32 {
    let strength = smooth(18.0, 55.0, chain)
        * smooth(SEA_LEVEL as f32 + 4.0, SEA_LEVEL as f32 + 14.0, elevation);
    if strength <= 0.0 {
        return 0.0;
    }
    let bulk = fbm(seed, 0x5a, x / 90.0, z / 90.0, 3, 0.5) as f32;
    let rough = fbm(seed, 0x5b, x / 23.0, z / 23.0, 2, 0.5) as f32;
    let gain = 0.55
        + 0.9
            * smooth(
                -0.35,
                0.45,
                fbm(seed, 0x5d, x / 310.0, z / 310.0, 2, 0.5) as f32,
            );
    strength * gain * (20.0 * bulk + 4.0 * rough)
}

/// Le plus fort coefficient de [`canopy_factor`] : la borne que le tirage des arbres utilise pour
/// écarter vite les candidats (voir `features.rs`).
pub(super) const MAX_CANOPY: f32 = 2.6;

/// Le coefficient de végétation d'une colonne, de 0 à [`MAX_CANOPY`] : il multiplie la densité
/// d'arbres du biome (voir `features.rs`). Chaque terme est continu :
///
/// - **humidité relative** : plus humide que le climat typique du biome, la forêt se densifie ;
///   plus sèche, elle s'éclaircit (de 0,65 à 1,35) : une lisière naturelle d'une zone à l'autre ;
/// - **pente** : plus aucun arbre au-delà de 1,1 de pente, plus rares dès 0,5 ;
/// - **ligne des arbres** : `SEA + 100 + 28 × température au niveau de la mer` ; au-dessus, la
///   roche et l'herbe rase, plus d'arbres ;
/// - **berges** : jusqu'à 80 % d'arbres en plus entre 5 et 9 blocs d'une zone réservée (végétation
///   riveraine) ; les arbres eux-mêmes restent hors des zones réservées (`clearance_of`).
pub(super) fn canopy_factor(
    biome_humidity: f64,
    humidity: f64,
    slope: f32,
    top: i32,
    sea_temperature: f64,
    clear: u8,
) -> f32 {
    let moisture = 0.65 + 0.7 * smooth64(-0.4, 0.4, humidity - biome_humidity);
    let steep = 1.0 - smooth64(0.5, 1.1, slope as f64);
    let line = SEA_LEVEL as f64 + 100.0 + 28.0 * sea_temperature;
    let height = 1.0 - smooth64(line - 18.0, line + 8.0, top as f64);
    let riparian = 1.0 + 0.8 * smooth64(9.5, 5.0, clear as f64);
    ((moisture * steep * height * riparian) as f32).clamp(0.0, MAX_CANOPY)
}

/// Le dernier bloc plein d'un relief de hauteur flottante `h`.
pub(super) fn quantize(h: f32) -> i32 {
    (h - 0.5).ceil() as i32
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_geo(a: Geo, b: Geo, t: f32) -> Geo {
    Geo {
        elevation: lerp(a.elevation, b.elevation, t),
        macro_elevation: lerp(a.macro_elevation, b.macro_elevation, t),
        chain: lerp(a.chain, b.chain, t),
        coast: lerp(a.coast, b.coast, t),
        detail: lerp(a.detail, b.detail, t),
        scarp: Scarp::lerp(a.scarp, b.scarp, t),
    }
}

impl TerrainGenerator {
    /// Le contexte de la colonne de chunks (cx, cz) : voir la documentation du module.
    pub(super) fn build_context(&self, v2: &V2State, cx: i32, cz: i32) -> ColumnContext {
        let size = CHUNK_SIZE as i32;
        let (x0, z0) = (cx * size, cz * size);
        let (wx0, wz0) = (x0 - MARGIN, z0 - MARGIN);
        let world = &v2.world;
        let hydro = world.hydrology();

        // 1. La géographie sur une grille de 4 blocs.
        let mut grid = Vec::with_capacity(GEO_POINTS * GEO_POINTS);
        for j in 0..GEO_POINTS {
            for i in 0..GEO_POINTS {
                let g = world
                    .geography()
                    .sample((wx0 + 4 * i as i32) as f64, (wz0 + 4 * j as i32) as f64);
                let (px, pz) = ((wx0 + 4 * i as i32) as f64, (wz0 + 4 * j as i32) as f64);
                let terrain = Terrain {
                    chain: g.chain_height,
                    elevation: g.elevation,
                    axis: g.axis_distance,
                    ridge: g.ridge_distance,
                };
                grid.push(Geo {
                    elevation: g.elevation,
                    macro_elevation: g.macro_elevation,
                    chain: g.chain_height,
                    coast: g.coast_distance,
                    detail: massif_detail(self.seed, px, pz, g.chain_height, g.elevation),
                    scarp: scarp(self.seed, px, pz, &terrain),
                });
            }
        }
        let mut scarps: Vec<Scarp> = grid.iter().map(|g| g.scarp).collect();
        limit_reach(&mut scarps, GEO_POINTS);
        for (g, s) in grid.iter_mut().zip(scarps) {
            g.scarp = s;
        }
        let geo_at = |wi: usize, wj: usize| {
            let (gi, gj) = (wi / 4, wj / 4);
            let (fx, fz) = ((wi % 4) as f32 / 4.0, (wj % 4) as f32 / 4.0);
            let row =
                |j: usize| lerp_geo(grid[j * GEO_POINTS + gi], grid[j * GEO_POINTS + gi + 1], fx);
            lerp_geo(row(gj), row(gj + 1), fz)
        };

        // 2. Les cours et les lacs de la zone, lus une fois.
        let c_min = fine_cell(wx0 as f64, wz0 as f64);
        let c_max = fine_cell(
            (wx0 + WIDE as i32 - 1) as f64,
            (wz0 + WIDE as i32 - 1) as f64,
        );
        let candidates: Vec<RiverCandidate> =
            hydro.river_candidates((c_min.0 - 2, c_min.1 - 2), (c_max.0 + 2, c_max.1 + 2));
        let mut lakes: Vec<Lake> = Vec::new();
        // Un lac peut agir à 56 blocs de son rivage : on lit ceux des cellules à 3 cellules de la
        // zone, assez pour que chaque colonne voie tous les lacs qui l'influencent (voir la doc de
        // `lakes_near`), quel que soit son contexte.
        for fz in c_min.1 - 3..=c_max.1 + 3 {
            for fx in c_min.0 - 3..=c_max.0 + 3 {
                for lake in hydro.lakes_near((fx, fz)) {
                    if !lakes.contains(&lake) {
                        lakes.push(lake);
                    }
                }
            }
        }

        // 3. Chaque colonne de la zone large.
        let mut wide = vec![
            WideCell {
                top: 0,
                water: NO_WATER,
                flags: 0,
                protect: 0,
                valley: 0,
                base: 0,
                jump: 0,
                river_width: 0,
                table: 0.0,
            };
            WIDE * WIDE
        ];
        let mut geos = Vec::with_capacity(WIDE * WIDE);
        for wj in 0..WIDE {
            for wi in 0..WIDE {
                let (x, z) = (wx0 + wi as i32, wz0 + wj as i32);
                let g = geo_at(wi, wj);
                geos.push(g);
                wide[wj * WIDE + wi] =
                    self.wide_cell(hydro, &candidates, &lakes, x as f64, z as f64, g);
            }
        }

        // 4. Les berges fermées : une colonne de terre plus basse que l'eau voisine est relevée.
        let raised: Vec<(usize, i16)> = (1..WIDE - 1)
            .flat_map(|wj| (1..WIDE - 1).map(move |wi| (wj, wi)))
            .filter_map(|(wj, wi)| {
                let i = wj * WIDE + wi;
                let cell = &wide[i];
                if cell.water != NO_WATER || (cell.top as i32) < SEA_LEVEL {
                    return None;
                }
                let need = [i - 1, i + 1, i - WIDE, i + WIDE]
                    .into_iter()
                    .map(|n| wide[n].water)
                    .max()
                    .unwrap_or(NO_WATER);
                (need != NO_WATER && need > cell.top).then_some((i, need))
            })
            .collect();
        for (i, level) in raised {
            wide[i].top = level;
            wide[i].base = wide[i].base.max(level);
        }

        // 5. La distance à la zone réservée (Chebyshev, jusqu'à 9) par deux passes.
        let clearance = clearance_field(&wide);

        // 6. Le climat (voir `climate.rs`), aux quatre coins de la colonne de chunks, sur le cœur : température, humidité,
        // variété et température au niveau de la mer. Les points tombent sur la grille absolue de
        // 32 blocs : la colonne de chunks voisine lit les mêmes valeurs à sa frontière.
        let climate_model = WorldClimate::new(self.seed);
        let climate_points: Vec<(f64, f64, f64, f64)> = (0..2)
            .flat_map(|j| (0..2).map(move |i| (i, j)))
            .map(|(i, j)| {
                let c = climate_model.sample(
                    world.geography(),
                    (x0 + 32 * i) as f64,
                    (z0 + 32 * j) as f64,
                );
                (c.temperature, c.humidity, c.variety, c.sea_temperature)
            })
            .collect();

        // 7. Le cœur : pente, biome, surface, surplombs, nappe, cavernes.
        let top_at = |lx: i32, lz: i32| wide[ColumnContext::wide_index(lx, lz)].top as i32;
        let mut core = Vec::with_capacity(CHUNK_SIZE * CHUNK_SIZE);
        let mut columns = Box::new([[Column::default(); CHUNK_SIZE]; CHUNK_SIZE]);
        let mut biomes = vec![0u8; CHUNK_SIZE * CHUNK_SIZE];
        let mut pending: Vec<(usize, usize)> = Vec::new();
        for lz in 0..size {
            for lx in 0..size {
                let w = wide[ColumnContext::wide_index(lx, lz)];
                let g = geos[(lz + MARGIN) as usize * WIDE + (lx + MARGIN) as usize];
                let top = w.top as i32;
                // La pente et la marche, sur les hauteurs en blocs.
                let (gx, gz) = (
                    (top_at(lx + 2, lz) - top_at(lx - 2, lz)) as f32 / 4.0,
                    (top_at(lx, lz + 2) - top_at(lx, lz - 2)) as f32 / 4.0,
                );
                let slope = gx.hypot(gz);
                let step = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .map(|&(dx, dz)| (top_at(lx + dx, lz + dz) - top).abs())
                    .max()
                    .unwrap_or(0);
                let protect = w.protect as f32 / 255.0;
                let valley = w.valley as f32 / 255.0;
                // Le climat interpolé, l'humidité relevée près de l'eau.
                let (fx, fz) = (lx as f64 / 32.0, lz as f64 / 32.0);
                let at = |i: i32, j: i32| climate_points[(j * 2 + i) as usize];
                let (i0, j0) = (0, 0);
                let mix = |f: fn(&(f64, f64, f64, f64)) -> f64| {
                    let top_row = f(&at(i0, j0)) * (1.0 - fx) + f(&at(i0 + 1, j0)) * fx;
                    let bottom_row = f(&at(i0, j0 + 1)) * (1.0 - fx) + f(&at(i0 + 1, j0 + 1)) * fx;
                    top_row * (1.0 - fz) + bottom_row * fz
                };
                let (t, h_regional, vty, sea_t) =
                    (mix(|c| c.0), mix(|c| c.1), mix(|c| c.2), mix(|c| c.3));
                // L'humidité locale : l'eau toute proche (cours, lacs) et les zones basses humides.
                // Un marais est une terre basse, plate, humide, en taches (voir `climate.rs`) :
                // jamais en hauteur, jamais au rivage (la plage passe avant), jamais dans le froid.
                // Le climat seul ne fait pas un marais : les biomes de zone humide ne sont candidats
                // que si `wetland` dépasse 0,4. La platitude se lit sur 8 blocs de part et d'autre
                // (la pente locale, quantifiée en blocs, ferait des taches).
                let flat = {
                    // Sur l'altitude de la géographie (flottante, sans le creusement des cours ni les
                    // escarpements), pas sur les blocs : un relief quantifié trace des taches.
                    let e = |dx: i32, dz: i32| {
                        geos[(lz + MARGIN + dz) as usize * WIDE + (lx + MARGIN + dx) as usize]
                            .elevation as f64
                    };
                    let here = g.elevation as f64;
                    ((e(8, 0) - here).abs()
                        + (e(-8, 0) - here).abs()
                        + (e(0, 8) - here).abs()
                        + (e(0, -8) - here).abs())
                        / 32.0
                };
                let wetland = smooth64(
                    SEA_LEVEL as f64 + 14.0,
                    SEA_LEVEL as f64 + 5.0,
                    g.elevation as f64,
                ) * smooth64(0.30, 0.10, flat)
                    * smooth64(-0.30, 0.10, h_regional)
                    * smooth64(
                        -0.15,
                        0.25,
                        climate_model.wetland_noise((x0 + lx) as f64, (z0 + lz) as f64),
                    )
                    * smooth64(80.0, 220.0, g.coast as f64);
                // Le biome se choisit sur l'humidité régionale et les zones humides ; l'eau proche
                // (+0,45) ne compte que pour la végétation (couvert riverain) : un cours n'ouvre pas
                // une bande de forêt d'une autre variété.
                let h_biome = (h_regional + 0.75 * wetland).clamp(-1.0, 1.0);
                let h = (h_regional + 0.45 * protect as f64 + 0.75 * wetland).clamp(-1.0, 1.0);
                // La côte est rocheuse là où le terrain monte vite derrière le rivage : la pente est
                // lue sur 8 blocs pour que le sable ne s'alterne pas avec la roche d'une dune à l'autre.
                let shore_slope = flat;
                // La famille du biome.
                let bed = w.flags & (flag::BED_RIVER | flag::BED_LAKE);
                let kind = if bed != 0 {
                    BiomeKind::River
                } else if top < SEA_LEVEL {
                    if g.macro_elevation < SEA_LEVEL as f32 - 28.0 {
                        BiomeKind::DeepOcean
                    } else {
                        BiomeKind::Ocean
                    }
                } else if g.chain >= 24.0 && top > SEA_LEVEL + 10 {
                    if top > SEA_LEVEL + 85 {
                        BiomeKind::Peaks
                    } else {
                        BiomeKind::Mountains
                    }
                } else if g.coast < 70.0 && top <= SEA_LEVEL + 4 && slope < 0.9 {
                    BiomeKind::Beach
                } else {
                    BiomeKind::Land
                };
                // Les océans, les plages et les rivières ne dépendent pas de l'humidité ; une côte
                // pentue est rocheuse (la « variété » 0,8 désigne la côte rocheuse de `biomes.ron`).
                let (h_pick, v_pick) = match kind {
                    BiomeKind::Beach => (0.0, 0.8 * smooth64(0.22, 0.45, shore_slope)),
                    BiomeKind::Ocean | BiomeKind::DeepOcean | BiomeKind::River => (0.0, 0.0),
                    _ => (h_biome, vty),
                };
                let biome = self.v2_biome(kind, t, h_pick, v_pick, wetland >= 0.4);
                let def = self.biomes.get(biome);
                // L'ombre des pentes tournées vers le pôle (-z) : un peu plus froid.
                let facing = (gz / slope.max(1e-3)).clamp(-1.0, 1.0);
                let shade = smooth(0.1, 0.5, slope) * facing.max(0.0);
                let altitude = (top - (SEA_LEVEL + 40)).max(0) as f32;
                let temperature = def.temperature - altitude * 0.012 - 0.10 * shade;
                // La rive : de l'eau d'un cours ou d'un lac à deux blocs au plus.
                let mut bank: Option<i32> = None;
                let mut wide_bank = false;
                if w.water == NO_WATER && bed == 0 {
                    for dz in -2..=2 {
                        for dx in -2..=2 {
                            let n = wide[ColumnContext::wide_index(lx + dx, lz + dz)];
                            if n.water != NO_WATER {
                                bank = Some(bank.map_or(n.water as i32, |b| b.max(n.water as i32)));
                                wide_bank |= n.river_width >= 6 || n.flags & flag::WATER_LAKE != 0;
                            }
                        }
                    }
                }
                let (x, z) = (x0 + lx, z0 + lz);
                let rule = self.surface_rule(&SurfaceInputs {
                    x,
                    z,
                    biome,
                    top,
                    slope,
                    step,
                    temperature,
                    bed,
                    river_width: w.river_width as f32,
                    bank,
                    bank_wide: wide_bank,
                });
                // L'escarpement : la hauteur et la distance à la ligne de rupture de la paroi qui
                // commande l'entaille, et l'entaille de son pied (par plages ; jamais sous une zone
                // réservée, un lit, l'eau, ni près de la mer).
                let jump = w.jump as f32;
                let dist = if jump > 0.0 { g.scarp.main().dist } else { 0.0 };
                let notchable = jump >= 12.0
                    && protect == 0.0
                    && valley < 0.2
                    && bed == 0
                    && w.water == NO_WATER
                    && w.base as i32 > SEA_LEVEL + 6;
                let reach = if notchable {
                    // Une rugosité de 15 % le long de la paroi : des alcôves aux bords irréguliers.
                    // Jamais plus profonde que la moitié de la hauteur de la paroi : la voûte garde
                    // une pente douce, ce qui laisse le toit relié à la masse.
                    (g.scarp.reach * (0.85 + 0.3 * value_noise(self.seed, x0 + lx, z0 + lz, 7, 71)))
                        .min(0.5 * jump)
                } else {
                    0.0
                };
                // Seules les colonnes du côté haut, assez près de la ligne pour être touchées.
                let reach = if reach >= 1.0 && dist > -1.0 && dist < reach + 1.0 {
                    reach
                } else {
                    0.0
                };
                if reach > 0.0 {
                    pending.push((lx as usize, lz as usize));
                }
                // La nappe, bornée à 8 blocs sous le relief le plus bas des 5 × 5 voisines.
                let lowest = (-2..=2)
                    .flat_map(|dz| (-2..=2).map(move |dx| (dx, dz)))
                    .map(|(dx, dz)| top_at(lx + dx, lz + dz))
                    .min()
                    .unwrap_or(top);
                let table = (w.table.floor() as i32).min(lowest - 8);
                let table = if table >= LAVA_LEVEL + super::v2::LAVA_GAP {
                    table as i16
                } else {
                    NO_WATER
                };
                // La profondeur des cavernes : celle de la v1, plus profonde sous l'eau ; sous une
                // zone réservée, à plus de 10 blocs sous le lit. Une entrée contrôlée peut
                // affleurer sur une falaise nue (jamais dans une zone réservée, jamais sous un
                // sol qui tombe).
                let wet = w.water != NO_WATER || top < SEA_LEVEL + 2;
                let mut limit = if wet { top - 9 } else { top - 4 };
                if w.protect > 0 {
                    limit = limit.min(top - 10);
                }
                let entrance = w.protect == 0
                    && slope >= CLIFF_SLOPE
                    && top > SEA_LEVEL + 6
                    && value_noise(self.seed, x, z, 23, 70) > 0.88
                    && rule.top == super::surface::Mat::Rock;
                if entrance {
                    limit = top - 2;
                }
                // À moins de 16 blocs derrière une paroi (de 4 blocs ou plus) et de 3 blocs devant,
                // aucune caverne ne monte au-dessus du pied : elle pourrait trancher la face ou le
                // toit d'une entaille et en détacher des blocs.
                if jump >= 4.0 && dist > -3.0 && dist < 24.0 {
                    limit = limit.min(w.base as i32 - 4);
                }
                let clear = clearance[(lz + MARGIN) as usize * WIDE + (lx + MARGIN) as usize];
                let canopy = canopy_factor(def.climate.1, h, slope, top, sea_t, clear);
                let cell = CoreCell {
                    top: top as i16,
                    slope,
                    base: w.base,
                    jump,
                    dist,
                    reach,
                    shape: g.scarp.shape,
                    lid: g.scarp.lid,
                    biome,
                    rule,
                    table,
                    cave_limit: limit as i16,
                    clear,
                    humidity: h as f32,
                    climate: t as f32,
                    canopy,
                };
                core.push(cell);
                columns[lx as usize][lz as usize] = Column {
                    height: top,
                    biome,
                    temperature,
                    climate_temperature: t as f32,
                };
                biomes[(lx + lz * size) as usize] = biome.0;
            }
        }

        // 8. Le vrai sommet des colonnes à entaille (une paroi à demi franchie peut avoir toute sa
        // partie haute évidée).
        for (lx, lz) in pending {
            let i = lx + lz * CHUNK_SIZE;
            let strata = self.strata_column(x0 + lx as i32, z0 + lz as i32);
            let top = density::real_top(&strata, &core[i]);
            core[i].top = top as i16;
            columns[lx][lz].height = top;
        }
        let mut max_y = SEA_LEVEL.min(0);
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                let wt = {
                    let cell = &wide[ColumnContext::wide_index(lx as i32, lz as i32)];
                    if cell.water != NO_WATER {
                        cell.water as i32
                    } else if (cell.top as i32) < SEA_LEVEL {
                        SEA_LEVEL
                    } else {
                        i32::MIN
                    }
                };
                max_y = max_y.max(core[lx + lz * CHUNK_SIZE].top as i32).max(wt);
            }
        }
        let cave_ceiling = core.iter().map(|c| c.cave_limit as i32).max().unwrap_or(0);
        ColumnContext {
            wide,
            core,
            columns,
            biomes,
            max_y,
            cave_ceiling,
        }
    }

    /// Une colonne de la zone large : relief après vallée, lits et lacs, eau, réservation.
    fn wide_cell(
        &self,
        hydro: &super::Hydrology,
        candidates: &[RiverCandidate],
        lakes: &[Lake],
        x: f64,
        z: f64,
        g: Geo,
    ) -> WideCell {
        let sea = SEA_LEVEL as f32;
        let river = (g.macro_elevation >= sea - 1.0)
            .then(|| hydro.river_in(candidates, x, z))
            .flatten();
        let mut elevation = g.elevation;
        let mut flags = 0u8;
        let mut protect = 0.0f32;
        let mut valley = 0.0f32;
        let mut river_width = 0.0f32;
        let mut table = sea - 6.0 + 0.28 * (g.macro_elevation - sea);
        // (niveau en blocs, au centre) d'un lit de cours.
        let mut river_bed: Option<(i32, i32)> = None;
        // Le plancher des berges : le détail ne creuse pas la vallée sous le niveau de l'eau.
        let mut bank_floor = f32::NEG_INFINITY;
        if let Some(r) = river {
            let half = r.width * 0.5;
            let over = (r.distance - half).max(0.0);
            let relief = (g.elevation - r.level).max(0.0);
            let slope = 0.12 + 0.68 * smooth(8.0, 60.0, relief);
            let floor = (r.level + 1.0).min(g.elevation + FILL);
            let target = g.elevation.clamp(floor, floor + slope * over);
            let k = (1.0 - smooth(0.0, VALLEY_REACH, over)) * r.taper;
            bank_floor = bank_floor.max(floor);
            elevation += (target - elevation) * k;
            valley = valley.max(k);
            protect = protect.max(1.0 - smooth(half + 3.0, half + 9.0, r.distance));
            river_width = r.width;
            let dd = over;
            let drain = r.level - 2.0 + 0.1 * dd;
            table += (drain - table) * (1.0 - smooth(20.0, 140.0, dd));
            if r.distance <= half {
                flags |= flag::BED_RIVER;
                let depth = 1.0 + 0.1 * r.width;
                elevation = elevation.min(r.level - depth * (1.0 - r.distance / half.max(0.5)));
                let wl = r.level.floor() as i32;
                // Le lit est creusé en blocs : au moins un bloc d'eau au centre.
                let dep = 1 + i32::from(r.width >= 12.0) + i32::from(r.width >= 30.0);
                river_bed = Some((wl, if r.distance <= 0.7 * half { dep } else { 0 }));
            }
        }
        // Le lac le plus proche (le plus « dedans »).
        let mut lake: Option<(Lake, f64, f64)> = None;
        for l in lakes {
            let (dist, radius) = hydro.lake_geometry(l, x, z);
            let sd = dist - radius;
            if sd < LAKE_REACH as f64 && lake.as_ref().is_none_or(|(_, best, _)| sd < *best) {
                lake = Some((*l, sd, radius));
            }
        }
        let mut lake_bed: Option<(i32, f32)> = None;
        if let Some((l, sd, radius)) = lake {
            let sd = sd as f32;
            let drain = l.level - 2.0 + 0.1 * sd.max(0.0);
            table += (drain - table) * (1.0 - smooth(0.0, LAKE_REACH, sd.max(0.0)));
            protect = protect.max(1.0 - smooth(3.0, 9.0, sd));
            if sd < 0.0 {
                let fraction = (-sd as f64 / radius) as f32;
                let bottom = l.level - 1.0 - 5.0 * fraction.sqrt();
                elevation += (bottom - elevation) * smooth(0.0, 0.25, fraction);
                flags |= flag::BED_LAKE;
                valley = 1.0;
                lake_bed = Some((l.level.floor() as i32, fraction));
            } else {
                let floor = (l.level + 1.0).min(g.elevation + FILL);
                bank_floor = bank_floor.max(floor);
                let k = 1.0 - smooth(0.0, LAKE_REACH, sd);
                elevation += (elevation.max(floor) - elevation) * k;
                valley = valley.max(k);
            }
        }
        // Le détail de massif, atténué dans la vallée et nul dans la zone réservée ; il ne creuse
        // pas sous le niveau des berges.
        if g.detail != 0.0 {
            let before = elevation;
            let attenuation = (1.0 - protect) * (1.0 - valley);
            elevation = (before + g.detail * attenuation).max(bank_floor.min(before));
        }
        // L'escarpement : jamais dans une vallée (ni près d'un cours ou d'un lac), un lit ou une zone
        // réservée. Le pied de la paroi est le relief abaissé de la rampe (voir `scarp.rs`) ; le
        // sommet est ce pied relevé de la hauteur de la marche.
        let mut foot = elevation;
        let mut jump = 0.0;
        let gate = (1.0 - smooth(0.02, 0.2, valley)) * (1.0 - smooth(0.0, 0.3, protect));
        let main = g.scarp.main();
        if gate > 0.0 && g.scarp.walls.iter().any(|w| w.height > 0.0) {
            jump = main.height * gate;
            foot = elevation - g.scarp.foot_drop(gate);
            elevation = foot + g.scarp.lift(gate);
        }
        let mut top = quantize(elevation).clamp(1, MAX_HEIGHT);
        let mut base = quantize(foot).clamp(1, MAX_HEIGHT).min(top);
        let mut water = NO_WATER as i32;
        if let Some((wl, center_depth)) = river_bed {
            top = top.min(if center_depth > 0 {
                wl - center_depth
            } else {
                wl
            });
            if top < wl {
                water = water.max(wl);
                flags |= flag::WATER_RIVER;
            }
            base = base.min(top);
        }
        if let Some((wl, fraction)) = lake_bed {
            top = top.min(if fraction > 0.15 { wl - 1 } else { wl });
            if top < wl {
                water = water.max(wl);
                flags |= flag::WATER_LAKE;
            }
            base = base.min(top);
        }
        WideCell {
            top: top as i16,
            base: base as i16,
            jump: jump.round().clamp(0.0, 255.0) as u8,
            water: if water == NO_WATER as i32 {
                NO_WATER
            } else {
                water as i16
            },
            flags,
            protect: (protect * 255.0).round() as u8,
            valley: (valley * 255.0).round() as u8,
            river_width: river_width.round().clamp(0.0, 255.0) as u8,
            table,
        }
    }
}

/// La distance de Chebyshev (au plus 9) de chaque colonne à la zone réservée ou à l'eau la plus
/// proche : deux passes de chanfrein à huit voisins, exactes jusqu'à 8 pour les colonnes du cœur
/// (la marge est de 8 blocs).
fn clearance_field(wide: &[WideCell]) -> Vec<u8> {
    const CAP: u8 = 9;
    let mut d: Vec<u8> = wide
        .iter()
        .map(|c| {
            if c.protect > 0 || c.water != NO_WATER {
                0
            } else {
                CAP
            }
        })
        .collect();
    for j in 0..WIDE {
        for i in 0..WIDE {
            let mut best = d[j * WIDE + i];
            for (di, dj) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0)] {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if (0..WIDE as i32).contains(&ni) && (0..WIDE as i32).contains(&nj) {
                    best = best.min(d[nj as usize * WIDE + ni as usize].saturating_add(1));
                }
            }
            d[j * WIDE + i] = best.min(CAP);
        }
    }
    for j in (0..WIDE).rev() {
        for i in (0..WIDE).rev() {
            let mut best = d[j * WIDE + i];
            for (di, dj) in [(1i32, 1i32), (0, 1), (-1, 1), (1, 0)] {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if (0..WIDE as i32).contains(&ni) && (0..WIDE as i32).contains(&nj) {
                    best = best.min(d[nj as usize * WIDE + ni as usize].saturating_add(1));
                }
            }
            d[j * WIDE + i] = best.min(CAP);
        }
    }
    d
}
