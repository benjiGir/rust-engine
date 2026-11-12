//! Le terrain lointain : un décor simplifié qui prolonge le monde au-delà des chunks détaillés.
//!
//! # Pourquoi
//!
//! Un chunk détaillé coûte cher : génération en 3D (grottes, minerais, arbres), lumière,
//! maillage de tous les blocs. À 32 chunks de rayon on en charge déjà plus de 3 000 ; à
//! 128 chunks, plus de 50 000, ce qui est hors de portée. Or, très loin, on ne distingue que
//! la forme du terrain et ses couleurs.
//!
//! Le terrain lointain ne regarde donc que la **surface** : pour chaque point d'une grille
//! régulière, la hauteur et le biome que le générateur donne déjà sans rien creuser (pas de
//! grotte, pas d'arbre, pas de minerai). Il en fait un maillage lisse, coloré selon le biome,
//! l'altitude et la pente, avec de l'eau à la hauteur de la mer.
//!
//! # Les niveaux de détail
//!
//! Le sol est découpé en **tuiles** carrées de 32 × 32 cellules, de plus en plus grandes avec la
//! distance : au niveau `L`, une tuile fait `2 << L` chunks de côté (2, 4, 8, 16, 32), donc une
//! cellule fait 2, 4, 8, 16, 32 blocs. Toutes les tuiles ont le même coût (1 089 sommets) : la
//! distance rend 16 fois plus grand sans rien coûter de plus. Un arbre quaternaire
//! ([`select_tiles`]) choisit les tuiles pour une position de caméra :
//! - rien à l'intérieur du rayon des chunks détaillés ;
//! - autour de ce rayon, des tuiles fines (niveau 0), au grain des chunks, pour que le décor
//!   y rejoigne le monde sans trou ;
//! - plus loin, une tuile est utilisée quand sa distance dépasse `3 ×` sa taille, sinon elle
//!   se subdivise.
//!
//! Les sommets sont les mêmes que ceux des chunks ([`MeshVertex`], 24 octets) : leurs positions
//! sont en unités de cellule, et le shader les multiplie par l'échelle de la tuile (voir
//! `Renderer::add_mesh_scaled`). Entre deux niveaux, des **jupes** (des bandes verticales sous
//! les bords) cachent les fentes.
//!
//! # En tâche de fond
//!
//! [`FarStreamer`] construit les tuiles dans quelques threads, sans jamais toucher au monde.
//! Une région de 32 chunks ne change de niveau que d'un coup, quand toutes ses nouvelles
//! tuiles sont prêtes : jamais deux niveaux superposés à l'écran.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};
use glam::{Vec2, Vec3};

use crate::biome::BiomeKind;
use crate::generator::{FREEZING, SEA_LEVEL, STEP, TerrainGenerator};
use crate::mesh::MeshVertex;
use crate::registry::Tint;

/// Cellules par côté de tuile.
pub const CELLS: i32 = 32;
/// Niveaux de détail : de 0 (le plus fin) à `LEVELS - 1`.
pub const LEVELS: u8 = 5;
/// Une tuile n'est utilisée que si elle est à plus de ce multiple de sa taille.
const LOD_FACTOR: f32 = 3.0;
/// La surface de l'eau : le haut d'un bloc de fluide plein est à 14/16.
const WATER_TOP: f32 = SEA_LEVEL as f32 + 0.875;
/// Les tuiles fines se posent un peu sous le décor détaillé qu'elles recouvrent.
const SINK: f32 = 1.0;
/// Opacité de l'eau lointaine.
const WATER_ALPHA: f32 = 0.72;

/// Une tuile : son niveau et sa position (en tuiles de ce niveau).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FarTile {
    pub level: u8,
    pub x: i32,
    pub z: i32,
}

impl FarTile {
    /// Le côté en chunks.
    pub fn size_chunks(self) -> i32 {
        2 << self.level
    }

    /// Le côté en blocs.
    pub fn size_blocks(self) -> i32 {
        self.size_chunks() * 32
    }

    /// La taille d'une cellule, en blocs.
    pub fn step(self) -> i32 {
        self.size_blocks() / CELLS
    }

    /// Le coin minimal, en blocs.
    pub fn origin_blocks(self) -> (i32, i32) {
        (self.x * self.size_blocks(), self.z * self.size_blocks())
    }

    /// La région de 32 chunks (le niveau le plus grand) qui contient cette tuile.
    pub fn region(self) -> (i32, i32) {
        let shift = LEVELS - 1 - self.level;
        (self.x >> shift, self.z >> shift)
    }

    /// Le rectangle en chunks : (x0, z0, côté).
    fn rect_chunks(self) -> (f32, f32, f32) {
        let s = self.size_chunks();
        ((self.x * s) as f32, (self.z * s) as f32, s as f32)
    }
}

/// Jusqu'où on dessine, en chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FarConfig {
    /// Le rayon des chunks détaillés : le terrain lointain commence là.
    pub near_radius: i32,
    /// Le rayon du terrain lointain.
    pub far_radius: i32,
}

/// Les tuiles à dessiner pour une caméra à `camera` (en blocs).
pub fn select_tiles(camera: Vec3, config: FarConfig) -> Vec<FarTile> {
    let mut out = Vec::new();
    if config.far_radius <= config.near_radius {
        return out;
    }
    let center = Vec2::new(camera.x / 32.0, camera.z / 32.0);
    let root_size = (2 << (LEVELS - 1)) as f32;
    let reach = config.far_radius as f32;
    let x0 = ((center.x - reach) / root_size).floor() as i32;
    let x1 = ((center.x + reach) / root_size).floor() as i32;
    let z0 = ((center.y - reach) / root_size).floor() as i32;
    let z1 = ((center.y + reach) / root_size).floor() as i32;
    for z in z0..=z1 {
        for x in x0..=x1 {
            visit(
                FarTile {
                    level: LEVELS - 1,
                    x,
                    z,
                },
                center,
                config,
                &mut out,
            );
        }
    }
    out
}

fn visit(tile: FarTile, center: Vec2, config: FarConfig, out: &mut Vec<FarTile>) {
    let (rx, rz, size) = tile.rect_chunks();
    let nearest = Vec2::new(
        (rx - center.x).max(0.0).max(center.x - (rx + size)),
        (rz - center.y).max(0.0).max(center.y - (rz + size)),
    )
    .length();
    if nearest > config.far_radius as f32 {
        return;
    }
    let near = config.near_radius as f32;
    if tile.level == 0 {
        // Une tuile fine sert si un de ses quatre chunks est hors du rayon détaillé.
        let outside = (0..4).any(|i| {
            let c = Vec2::new(rx + (i % 2) as f32 + 0.5, rz + (i / 2) as f32 + 0.5);
            c.distance(center) > near
        });
        if outside {
            out.push(tile);
        }
        return;
    }
    let farthest = [(0.0, 0.0), (size, 0.0), (0.0, size), (size, size)]
        .iter()
        .map(|(dx, dz)| Vec2::new(rx + dx, rz + dz).distance(center))
        .fold(0.0f32, f32::max);
    if farthest <= near - 1.0 {
        return; // Tout entière dans le monde détaillé.
    }
    let touches_near = nearest < near + size;
    if !touches_near && nearest >= LOD_FACTOR * size {
        out.push(tile);
        return;
    }
    for dz in 0..2 {
        for dx in 0..2 {
            visit(
                FarTile {
                    level: tile.level - 1,
                    x: tile.x * 2 + dx,
                    z: tile.z * 2 + dz,
                },
                center,
                config,
                out,
            );
        }
    }
}

/// Le maillage d'une tuile, prêt à être envoyé au GPU.
#[derive(Debug, Clone, PartialEq)]
pub struct FarMesh {
    pub vertices: Vec<MeshVertex>,
    /// Le sol et les jupes.
    pub indices: Vec<u32>,
    /// L'eau.
    pub translucent_indices: Vec<u32>,
    /// Où est l'origine locale dans le monde (en blocs).
    pub origin: Vec3,
    /// Les positions locales sont en cellules : à multiplier par cette échelle (en blocs).
    pub scale: f32,
    /// La boîte englobante dans le monde.
    pub bounds: (Vec3, Vec3),
}

impl FarMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() && self.translucent_indices.is_empty()
    }
}

/// Les couleurs, la hauteur et la pente d'un point de la grille.
#[derive(Clone, Copy)]
struct Sample {
    height: f32,
    color: [f32; 3],
    water: [f32; 3],
}

fn sample(generator: &TerrainGenerator, x: i32, z: i32, step: i32) -> Sample {
    // Aligné sur la grille des cartes de bruit (tous les 4 blocs) : une évaluation seulement.
    let column = if step % STEP == 0 {
        generator.column_at_grid_point(x / STEP, z / STEP)
    } else {
        generator.column(x, z)
    };
    let biome = generator.biome(column.biome);
    let def = generator.registry().def(biome.surface);
    let (grass_ref, _, water_ref) = generator.biomes().tint_reference();
    let mut color = if def.tint == Tint::Grass {
        // Même palette linéaire et mêmes multiplicateurs que les chunks texturés.
        std::array::from_fn(|c| def.color[c] * biome.grass_color[c] / grass_ref[c].max(0.01))
    } else {
        def.color
    };
    let top = column.height as f32 + 1.0;
    // Les forêts sont plus sombres vues de loin (la canopée).
    if biome.tree_density > 3.0 && column.height >= SEA_LEVEL {
        color = color.map(|c| c * 0.78);
    }
    // La roche des montagnes, la neige du froid et des sommets.
    if matches!(biome.kind, BiomeKind::Mountains | BiomeKind::Peaks) {
        let rock = ((top - 92.0) / 30.0).clamp(0.0, 1.0);
        color = lerp(
            color,
            palette(generator, "core:stone", [0.23, 0.26, 0.26]),
            rock,
        );
    }
    if (column.height >= SEA_LEVEL && column.temperature < FREEZING) || top > 150.0 {
        color = palette(generator, "core:snow", [0.80, 0.87, 0.87]);
    }
    // Sous la mer, le fond s'assombrit avec la profondeur.
    if column.height < SEA_LEVEL {
        let depth = ((SEA_LEVEL - column.height) as f32 / 30.0).clamp(0.0, 1.0);
        color = lerp(
            generator.registry().def(biome.underwater).color,
            [0.035, 0.055, 0.075],
            depth,
        );
    }
    Sample {
        height: top,
        color,
        water: {
            let base = palette(generator, "core:water", [0.05, 0.20, 0.34]);
            std::array::from_fn(|c| base[c] * biome.water_color[c] / water_ref[c].max(0.01))
        },
    }
}

fn palette(generator: &TerrainGenerator, key: &str, fallback: [f32; 3]) -> [f32; 3] {
    generator
        .registry()
        .id(key)
        .map_or(fallback, |id| generator.registry().def(id).color)
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Construit le maillage d'une tuile à partir du générateur.
pub fn build_tile(generator: &TerrainGenerator, tile: FarTile) -> FarMesh {
    let stone = palette(generator, "core:stone", [0.23, 0.26, 0.26]);
    let step = tile.step();
    let (ox, oz) = tile.origin_blocks();
    // Les échantillons de -1 à CELLS + 1 : la bordure sert aux normales.
    let n = (CELLS + 3) as usize;
    let mut samples = Vec::with_capacity(n * n);
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            samples.push(sample(
                generator,
                ox + (i - 1) * step,
                oz + (j - 1) * step,
                step,
            ));
        }
    }
    let at = |i: i32, j: i32| samples[((j + 1) as usize) * n + (i + 1) as usize];
    let sink = if tile.level <= 1 { SINK } else { 0.0 };
    let height = |i: i32, j: i32| at(i, j).height - sink;

    // La hauteur minimale et maximale de la tuile, pour placer l'origine verticale : les
    // positions locales tiennent de -8 à 56 cellules.
    let mut min_h = f32::MAX;
    let mut max_h = f32::MIN;
    for j in 0..=CELLS {
        for i in 0..=CELLS {
            let h = height(i, j);
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }
    let scale = step as f32;
    // Les jupes descendent de 4 cellules sous le point le plus bas.
    let skirt = 4.0 * scale;
    let origin_y = (min_h - skirt).floor();
    let local_y = |y: f32| ((y - origin_y) / scale).min(55.0);

    let mut vertices: Vec<MeshVertex> =
        Vec::with_capacity(((CELLS + 1) * (CELLS + 1)) as usize + 4 * (CELLS as usize + 1) * 2);
    let mut indices: Vec<u32> = Vec::new();
    // Le sol : une grille de (CELLS + 1)² sommets, normale par différences centrées.
    for j in 0..=CELLS {
        for i in 0..=CELLS {
            let dx = (height(i + 1, j) - height(i - 1, j)) / (2.0 * scale);
            let dz = (height(i, j + 1) - height(i, j - 1)) / (2.0 * scale);
            let normal = Vec3::new(-dx, 1.0, -dz).normalize();
            // Une pente forte devient de la roche.
            let steep = ((0.75 - normal.y) / 0.25).clamp(0.0, 1.0);
            let color = lerp(at(i, j).color, stone, steep * 0.7);
            vertices.push(MeshVertex::new(
                Vec3::new(i as f32, local_y(height(i, j)), j as f32),
                normal.to_array(),
                [color[0], color[1], color[2], 1.0],
                [1.0, 0.0, 1.0],
                0,
            ));
        }
    }
    let grid = |i: i32, j: i32| (j * (CELLS + 1) + i) as u32;
    for j in 0..CELLS {
        for i in 0..CELLS {
            // Deux triangles par cellule, dans le sens anti-horaire vu du dessus.
            indices.extend([grid(i, j), grid(i, j + 1), grid(i + 1, j)]);
            indices.extend([grid(i + 1, j), grid(i, j + 1), grid(i + 1, j + 1)]);
        }
    }
    // Les jupes : une bande verticale sous chaque bord, tournée vers l'extérieur.
    let mut skirt_strip = |edge: &dyn Fn(i32) -> (i32, i32), outward: Vec3| {
        let base = vertices.len() as u32;
        for k in 0..=CELLS {
            let (i, j) = edge(k);
            let top = vertices[grid(i, j) as usize];
            let mut bottom = top;
            bottom.position[1] =
                (((local_y(height(i, j)) - 4.0).max(-7.0) + 8.0) * 1024.0).round() as u16;
            vertices.push(top);
            vertices.push(bottom);
        }
        for k in 0..CELLS as u32 {
            let (t0, b0, t1, b1) = (
                base + 2 * k,
                base + 2 * k + 1,
                base + 2 * k + 2,
                base + 2 * k + 3,
            );
            // Le triangle (haut, haut suivant, bas) doit regarder vers l'extérieur.
            let p = |v: u32| vertices[v as usize].local_position();
            let normal = (p(t1) - p(t0)).cross(p(b0) - p(t0));
            if normal.dot(outward) >= 0.0 {
                indices.extend([t0, t1, b0, t1, b1, b0]);
            } else {
                indices.extend([t0, b0, t1, t1, b0, b1]);
            }
        }
    };
    skirt_strip(&|k| (k, 0), Vec3::NEG_Z);
    skirt_strip(&|k| (k, CELLS), Vec3::Z);
    skirt_strip(&|k| (0, k), Vec3::NEG_X);
    skirt_strip(&|k| (CELLS, k), Vec3::X);

    // L'eau : un quad plat à la hauteur de la mer sur chaque cellule dont un coin est sous l'eau.
    let mut translucent: Vec<u32> = Vec::new();
    let water_y = local_y(WATER_TOP);
    for j in 0..CELLS {
        for i in 0..CELLS {
            let wet = [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)]
                .iter()
                .any(|&(a, b)| at(a, b).height <= SEA_LEVEL as f32);
            if !wet {
                continue;
            }
            let [r, g, b] = at(i, j).water;
            let base = vertices.len() as u32;
            for (a, c) in [(i, j), (i, j + 1), (i + 1, j), (i + 1, j + 1)] {
                vertices.push(MeshVertex::new(
                    Vec3::new(a as f32, water_y, c as f32),
                    [0.0, 1.0, 0.0],
                    [r, g, b, WATER_ALPHA],
                    [1.0, 0.0, 1.0],
                    0,
                ));
            }
            translucent.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        }
    }

    let size = tile.size_blocks() as f32;
    FarMesh {
        vertices,
        indices,
        translucent_indices: translucent,
        origin: Vec3::new(ox as f32, origin_y, oz as f32),
        scale,
        bounds: (
            Vec3::new(ox as f32, origin_y, oz as f32),
            Vec3::new(ox as f32 + size, max_h.max(WATER_TOP), oz as f32 + size),
        ),
    }
}

// ----- Le chargement en tâche de fond -----

/// Ce que le jeu doit faire côté GPU.
#[derive(Debug)]
pub enum FarEvent {
    Add(FarTile, Box<FarMesh>),
    Remove(FarTile),
}

/// Des compteurs pour les logs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FarStats {
    pub shown: usize,
    pub pending: usize,
    pub staged: usize,
}

pub struct FarStreamer {
    config: FarConfig,
    shown: HashSet<FarTile>,
    /// Construites, pas encore montrées (en attente du reste de leur région).
    staged: HashMap<FarTile, FarMesh>,
    pending: HashSet<FarTile>,
    requests: Option<Sender<FarTile>>,
    results: Receiver<(FarTile, FarMesh)>,
    workers: Vec<JoinHandle<()>>,
}

impl FarStreamer {
    pub fn new(generator: Arc<TerrainGenerator>, config: FarConfig, threads: usize) -> Self {
        let (request_tx, request_rx) = crossbeam_channel::unbounded::<FarTile>();
        let (result_tx, result_rx) = crossbeam_channel::unbounded();
        let workers = (0..threads.max(1))
            .map(|i| {
                let (rx, tx, generator) =
                    (request_rx.clone(), result_tx.clone(), generator.clone());
                std::thread::Builder::new()
                    .name(format!("terrain lointain {i}"))
                    .spawn(move || {
                        // Le canal se ferme quand le streamer est détruit : le thread s'arrête.
                        while let Ok(tile) = rx.recv() {
                            let _span = engine_trace::span("terrain lointain : tuile");
                            let mesh = build_tile(&generator, tile);
                            if tx.send((tile, mesh)).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("thread du terrain lointain")
            })
            .collect();
        Self {
            config,
            shown: HashSet::new(),
            staged: HashMap::new(),
            pending: HashSet::new(),
            requests: Some(request_tx),
            results: result_rx,
            workers,
        }
    }

    pub fn config(&self) -> FarConfig {
        self.config
    }

    pub fn set_config(&mut self, config: FarConfig) {
        self.config = config;
    }

    pub fn stats(&self) -> FarStats {
        FarStats {
            shown: self.shown.len(),
            pending: self.pending.len(),
            staged: self.staged.len(),
        }
    }

    /// Les tuiles montrées en ce moment.
    pub fn shown(&self) -> impl Iterator<Item = FarTile> + '_ {
        self.shown.iter().copied()
    }

    /// À appeler à chaque image avec la position de la caméra (en blocs).
    pub fn update(&mut self, camera: Vec3) -> Vec<FarEvent> {
        while let Ok((tile, mesh)) = self.results.try_recv() {
            self.pending.remove(&tile);
            self.staged.insert(tile, mesh);
        }
        let needed: Vec<FarTile> = select_tiles(camera, self.config);
        let needed_set: HashSet<FarTile> = needed.iter().copied().collect();

        // On demande ce qui manque, du plus proche au plus loin.
        let center = Vec2::new(camera.x / 32.0, camera.z / 32.0);
        let mut missing: Vec<(f32, FarTile)> = needed
            .iter()
            .filter(|t| {
                !self.shown.contains(t) && !self.staged.contains_key(t) && !self.pending.contains(t)
            })
            .map(|t| {
                let (x, z, s) = t.rect_chunks();
                (center.distance(Vec2::new(x + s / 2.0, z + s / 2.0)), *t)
            })
            .collect();
        missing.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(tx) = &self.requests {
            for (_, tile) in missing {
                if tx.send(tile).is_ok() {
                    self.pending.insert(tile);
                }
            }
        }
        // Ce qui n'est plus voulu et pas montré n'a plus d'intérêt.
        self.staged.retain(|t, _| needed_set.contains(t));

        // Région par région : on ne change de niveau que quand tout est prêt.
        let mut regions: HashMap<(i32, i32), Vec<FarTile>> = HashMap::new();
        for t in &needed {
            regions.entry(t.region()).or_default().push(*t);
        }
        let mut shown_by_region: HashMap<(i32, i32), Vec<FarTile>> = HashMap::new();
        for t in &self.shown {
            shown_by_region.entry(t.region()).or_default().push(*t);
        }
        let mut events = Vec::new();
        let region_keys: HashSet<(i32, i32)> = regions
            .keys()
            .chain(shown_by_region.keys())
            .copied()
            .collect();
        for key in region_keys {
            let want = regions.get(&key).map_or(&[][..], Vec::as_slice);
            let have = shown_by_region.get(&key).map_or(&[][..], Vec::as_slice);
            let want_set: HashSet<FarTile> = want.iter().copied().collect();
            if want.is_empty() {
                // Sortie du rayon : tout disparaît.
                for t in have {
                    self.shown.remove(t);
                    events.push(FarEvent::Remove(*t));
                }
                continue;
            }
            let ready = want
                .iter()
                .all(|t| self.shown.contains(t) || self.staged.contains_key(t));
            // Une région encore vide se remplit au fur et à mesure ; sinon, d'un coup.
            if !have.is_empty() && !ready {
                continue;
            }
            for t in have {
                if !want_set.contains(t) {
                    self.shown.remove(t);
                    events.push(FarEvent::Remove(*t));
                }
            }
            for t in want {
                if let Some(mesh) = self.staged.remove(t) {
                    self.shown.insert(*t);
                    events.push(FarEvent::Add(*t, Box::new(mesh)));
                }
            }
        }
        events
    }
}

impl Drop for FarStreamer {
    fn drop(&mut self) {
        // Fermer le canal des demandes arrête les threads ; on les attend (ils finissent la
        // tuile en cours, au plus quelques millisecondes).
        self.requests = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FarConfig {
        FarConfig {
            near_radius: 16,
            far_radius: 128,
        }
    }

    /// Les chunks dont le centre est dans l'anneau [near, far) sont couverts exactement une fois.
    fn coverage(camera: Vec3, config: FarConfig) -> HashMap<(i32, i32), u32> {
        let mut count: HashMap<(i32, i32), u32> = HashMap::new();
        for tile in select_tiles(camera, config) {
            let s = tile.size_chunks();
            for dz in 0..s {
                for dx in 0..s {
                    *count.entry((tile.x * s + dx, tile.z * s + dz)).or_default() += 1;
                }
            }
        }
        count
    }

    #[test]
    fn tiles_cover_the_ring_exactly_once_and_never_overlap() {
        for camera in [
            Vec3::new(0.0, 80.0, 0.0),
            Vec3::new(1234.5, 90.0, -777.0),
            Vec3::new(-50_000.0, 70.0, 32.0),
        ] {
            let config = config();
            let count = coverage(camera, config);
            assert!(count.values().all(|&n| n == 1), "des tuiles se chevauchent");
            let center = Vec2::new(camera.x / 32.0, camera.z / 32.0);
            let (cx, cz) = (center.x.floor() as i32, center.y.floor() as i32);
            for dz in -config.far_radius..=config.far_radius {
                for dx in -config.far_radius..=config.far_radius {
                    let chunk = (cx + dx, cz + dz);
                    let d = Vec2::new(chunk.0 as f32 + 0.5, chunk.1 as f32 + 0.5).distance(center);
                    let covered = count.contains_key(&chunk);
                    if d > config.near_radius as f32 && d < config.far_radius as f32 - 32.0 {
                        assert!(covered, "trou en {chunk:?} (distance {d:.1})");
                    }
                    if d <= config.near_radius as f32 - 4.0 {
                        assert!(!covered, "tuile dans le monde détaillé en {chunk:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn detail_drops_with_distance_and_the_count_stays_small() {
        let tiles = select_tiles(Vec3::new(0.0, 80.0, 0.0), config());
        let mut by_level = [0usize; LEVELS as usize];
        for t in &tiles {
            by_level[t.level as usize] += 1;
        }
        assert!(
            by_level.iter().all(|&n| n > 0),
            "tous les niveaux servent : {by_level:?}"
        );
        // 128 chunks de rayon (4 km) avec quelques centaines de tuiles seulement.
        assert!(tiles.len() < 700, "{} tuiles", tiles.len());
        // Les tuiles proches sont les plus fines.
        let nearest = tiles
            .iter()
            .min_by(|a, b| {
                let d = |t: &&FarTile| {
                    let (x, z, s) = t.rect_chunks();
                    Vec2::new(x + s / 2.0, z + s / 2.0).length()
                };
                d(a).total_cmp(&d(b))
            })
            .unwrap();
        assert_eq!(nearest.level, 0);
        // Rien quand il n'y a pas d'anneau.
        assert!(
            select_tiles(
                Vec3::ZERO,
                FarConfig {
                    near_radius: 16,
                    far_radius: 16
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn region_and_geometry_helpers_agree() {
        let tile = FarTile {
            level: 2,
            x: -3,
            z: 5,
        };
        assert_eq!(tile.size_chunks(), 8);
        assert_eq!(tile.step(), 8);
        assert_eq!(tile.origin_blocks(), (-3 * 256, 5 * 256));
        // Quatre tuiles d'un niveau sont dans la même région que leur parent.
        let parent = FarTile {
            level: 3,
            x: -2,
            z: 2,
        };
        for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let child = FarTile {
                level: 2,
                x: -4 + dx,
                z: 4 + dz,
            };
            assert_eq!(child.region(), parent.region());
        }
    }

    #[test]
    fn a_tile_mesh_is_well_formed_and_deterministic() {
        let generator = TerrainGenerator::new(7);
        for tile in [
            FarTile {
                level: 0,
                x: 3,
                z: -2,
            },
            FarTile {
                level: 2,
                x: 5,
                z: 1,
            },
            FarTile {
                level: 4,
                x: -2,
                z: 3,
            },
        ] {
            let mesh = build_tile(&generator, tile);
            assert_eq!(mesh, build_tile(&generator, tile), "déterministe");
            assert!(!mesh.is_empty());
            assert_eq!(mesh.scale, tile.step() as f32);
            // Tous les indices pointent un sommet, par triangles entiers.
            assert!(
                mesh.indices.len().is_multiple_of(3)
                    && mesh.translucent_indices.len().is_multiple_of(3)
            );
            let max = mesh.vertices.len() as u32;
            assert!(
                mesh.indices
                    .iter()
                    .chain(&mesh.translucent_indices)
                    .all(|&i| i < max)
            );
            // Les positions locales tiennent dans la plage des sommets (sans écrêtage).
            for v in &mesh.vertices {
                let p = v.local_position();
                assert!(p.x >= -0.01 && p.x <= CELLS as f32 + 0.01, "{p}");
                assert!(p.y >= -8.0 && p.y <= 56.0, "{p}");
            }
            // Le sol : (CELLS + 1)² sommets, des normales vers le haut, une boîte cohérente.
            let (lo, hi) = mesh.bounds;
            assert!(lo.y < hi.y && hi.x - lo.x == tile.size_blocks() as f32);
            for v in mesh
                .vertices
                .iter()
                .take(((CELLS + 1) * (CELLS + 1)) as usize)
            {
                assert!(v.normal().y > 0.0);
            }
        }
    }

    #[test]
    fn ocean_tiles_have_water_and_land_tiles_do_not_always() {
        let generator = TerrainGenerator::new(7);
        // Au large : de l'eau. On cherche une tuile qui en a, et une qui n'en a pas.
        let mut wet = 0;
        let mut dry = 0;
        for z in -6..6 {
            for x in -6..6 {
                let mesh = build_tile(&generator, FarTile { level: 3, x, z });
                if mesh.translucent_indices.is_empty() {
                    dry += 1;
                } else {
                    wet += 1;
                }
            }
        }
        assert!(wet > 0 && dry > 0, "mer et terre : {wet} / {dry}");
    }

    #[test]
    fn the_streamer_fills_the_ring_and_swaps_regions_without_overlap() {
        let generator = Arc::new(TerrainGenerator::new(7));
        let mut streamer = FarStreamer::new(generator, config(), 2);
        let mut camera = Vec3::new(0.0, 80.0, 0.0);
        let mut events = 0usize;
        // La première passe : tout arrive au bout de quelques secondes.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            events += streamer.update(camera).len();
            let wanted = select_tiles(camera, config()).len();
            if streamer.stats().shown == wanted && streamer.stats().pending == 0 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "trop long : {:?}",
                streamer.stats()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(events > 100);
        // Les tuiles montrées ne se chevauchent pas.
        let overlap = |streamer: &FarStreamer| {
            let mut seen: HashSet<(i32, i32)> = HashSet::new();
            streamer.shown().all(|t| {
                let s = t.size_chunks();
                (0..s).all(|dz| (0..s).all(|dx| seen.insert((t.x * s + dx, t.z * s + dz))))
            })
        };
        assert!(overlap(&streamer));
        // Le joueur avance de 2 km : les régions changent, jamais de superposition, et on finit
        // par montrer exactement ce qu'il faut.
        camera.x += 2_000.0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            streamer.update(camera);
            assert!(overlap(&streamer), "deux niveaux superposés");
            let wanted: HashSet<FarTile> = select_tiles(camera, config()).into_iter().collect();
            let shown: HashSet<FarTile> = streamer.shown().collect();
            if wanted == shown {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "n'arrive pas à jour");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
