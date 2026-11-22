//! La lumière, façon Minecraft : deux niveaux de 0 à 15 par bloc.
//!
//! - **Lumière du ciel** : elle tombe tout droit depuis le haut du monde sans rien
//!   perdre tant qu'elle ne traverse que de l'air (ou du verre). Puis elle s'étale sur
//!   les côtés en perdant 1 par bloc : sous un surplomb ou à l'entrée d'une grotte,
//!   elle décroît doucement ; au fond d'une grotte fermée, elle vaut 0.
//! - **Lumière des blocs** : chaque bloc émissif (torche 14, pierre lumineuse 15)
//!   éclaire ses voisins en perdant 1 par bloc.
//!
//! Les deux se calculent par un **parcours en largeur** (BFS) : on part des sources
//! dans une file (`VecDeque`), et chaque bloc sorti de la file propage `niveau - 1`
//! à ses 6 voisins s'ils étaient moins éclairés ; ceux-là entrent à leur tour dans la
//! file. Comme on traite les blocs par distance croissante, chacun reçoit son niveau
//! final dès sa première visite : le coût est proportionnel au nombre de blocs éclairés.
//!
//! Deux usages :
//! - [`LightRegion::compute`] : la lumière complète d'une colonne de chunks, sur un
//!   thread de fond, à partir des blocs seuls ;
//! - [`update_light`] : la mise à jour incrémentale sur le thread principal quand le
//!   joueur casse ou pose un bloc (BFS de retrait puis BFS d'ajout).

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use glam::{IVec2, IVec3};

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk, DEFAULT_LIGHT, VOLUME};
use crate::registry::BlockRegistry;
use crate::world::World;

pub const MAX_LIGHT: u8 = 15;
const SIZE: i32 = CHUNK_SIZE as i32;
/// La lumière ne porte pas à plus de 15 blocs : pour calculer exactement une colonne,
/// il suffit de regarder 15 blocs autour d'elle.
const MARGIN: i32 = MAX_LIGHT as i32;
/// Largeur de la zone de calcul : la colonne et sa marge de chaque côté.
const WIDTH: i32 = SIZE + 2 * MARGIN;

/// Les 6 directions, avec un drapeau « vers le bas » pour la règle du ciel.
const DIRECTIONS: [(IVec3, bool); 6] = [
    (IVec3::X, false),
    (IVec3::NEG_X, false),
    (IVec3::Y, false),
    (IVec3::NEG_Y, true),
    (IVec3::Z, false),
    (IVec3::NEG_Z, false),
];

/// Le niveau qu'apporte une lumière `level` en entrant dans un bloc d'opacité `opacity`.
///
/// Règle du ciel : vers le bas, un niveau 15 traverse l'air sans rien perdre.
/// Sinon on perd au moins 1, et davantage dans un bloc qui absorbe (l'eau).
#[inline]
fn propagated(level: u8, opacity: u8, downward_sky: bool) -> u8 {
    if downward_sky && level == MAX_LIGHT && opacity == 0 {
        MAX_LIGHT
    } else {
        level.saturating_sub(opacity.max(1))
    }
}

/// La lumière calculée d'une colonne : une entrée par chunk, de bas en haut.
/// `None` : rien à stocker, la lumière par défaut (plein ciel, aucune torche) suffit.
pub type ColumnLight = Vec<Option<Box<[u8]>>>;

/// Tout ce qu'il faut pour calculer la lumière d'une colonne de chunks : les blocs
/// de la colonne et des 8 colonnes autour (des `Arc`, donc en lecture seule et sans copie).
pub struct LightRegion {
    column: IVec2,
    /// Les couches de chunks du monde (voir `World::vertical`), de bas en haut.
    vertical: std::ops::Range<i32>,
    /// 3 × 3 colonnes de chunks sur ces couches. `None` : chunk d'air.
    chunks: Vec<Option<Arc<Chunk>>>,
    registry: Arc<BlockRegistry>,
}

impl LightRegion {
    pub fn from_world(world: &World, column: IVec2) -> Self {
        let vertical = world.vertical();
        let mut chunks = Vec::with_capacity(9 * vertical.len());
        for dz in -1..=1 {
            for dx in -1..=1 {
                for y in vertical.clone() {
                    let pos = IVec3::new(column.x + dx, y, column.y + dz);
                    chunks.push(world.chunk_arc(pos));
                }
            }
        }
        Self {
            column,
            vertical,
            chunks,
            registry: world.registry().clone(),
        }
    }

    pub fn column(&self) -> IVec2 {
        self.column
    }

    /// Calcule la lumière de la colonne centrale. Pensé pour un thread de fond.
    pub fn compute(&self) -> ColumnLight {
        let height = self.height();
        let cells = (WIDTH * WIDTH * height) as usize;
        // Une table « numéro de bloc → opacité / émission » : plus rapide que d'interroger
        // le registre pour chacune des ~600 000 cellules de la zone.
        let opacity_of: Vec<u8> = self.registry.iter().map(|(_, d)| d.light_opacity).collect();
        let emission_of: Vec<u8> = self
            .registry
            .iter()
            .map(|(_, d)| d.light_emission)
            .collect();

        let mut opacity = vec![0u8; cells];
        let mut block = vec![0u8; cells];
        let mut block_queue = VecDeque::new();
        self.fill(|i, id| {
            let id = id.0 as usize;
            opacity[i] = opacity_of.get(id).copied().unwrap_or(15);
            let emission = emission_of.get(id).copied().unwrap_or(0);
            if emission > 0 {
                block[i] = emission;
                block_queue.push_back(i as u32);
            }
        });

        let sky = sky_light(&opacity, height);
        let mut sky_values = sky.values;
        bfs(&mut sky_values, &opacity, sky.seeds, true, height);
        bfs(&mut block, &opacity, block_queue, false, height);

        // On ne garde que la colonne centrale, découpée en chunks (indices locaux : 0 est la
        // couche la plus basse du monde).
        (0..self.vertical.len() as i32)
            .map(|cy| {
                let mut light = vec![DEFAULT_LIGHT; VOLUME].into_boxed_slice();
                let mut uniform = true;
                for ly in 0..SIZE {
                    for lz in 0..SIZE {
                        for lx in 0..SIZE {
                            let i = cell(lx + MARGIN, cy * SIZE + ly, lz + MARGIN);
                            let value = (sky_values[i] << 4) | block[i];
                            uniform &= value == DEFAULT_LIGHT;
                            light[Chunk::index(lx as usize, ly as usize, lz as usize)] = value;
                        }
                    }
                }
                (!uniform).then_some(light)
            })
            .collect()
    }

    /// La hauteur de la zone de calcul, en blocs.
    fn height(&self) -> i32 {
        self.vertical.len() as i32 * SIZE
    }

    /// Appelle `f(cellule, bloc)` pour chaque bloc non vide de la zone.
    fn fill(&self, mut f: impl FnMut(usize, BlockId)) {
        let origin = IVec3::new(
            self.column.x * SIZE - MARGIN,
            self.vertical.start * SIZE,
            self.column.y * SIZE - MARGIN,
        );
        let mut k = 0;
        for dz in -1..=1 {
            for dx in -1..=1 {
                for cy in self.vertical.clone() {
                    let chunk = &self.chunks[k];
                    k += 1;
                    let Some(chunk) = chunk else { continue };
                    let chunk_origin = IVec3::new(
                        (self.column.x + dx) * SIZE,
                        cy * SIZE,
                        (self.column.y + dz) * SIZE,
                    );
                    // La partie du chunk qui tombe dans la zone (elle ne couvre que 15 blocs
                    // des colonnes voisines).
                    let rel = chunk_origin - origin;
                    let x_range = rel.x.max(0)..(rel.x + SIZE).min(WIDTH);
                    let z_range = rel.z.max(0)..(rel.z + SIZE).min(WIDTH);
                    for ly in 0..SIZE {
                        for z in z_range.clone() {
                            for x in x_range.clone() {
                                let id = chunk.get(
                                    (x - rel.x) as usize,
                                    ly as usize,
                                    (z - rel.z) as usize,
                                );
                                if !id.is_air() {
                                    f(cell(x, rel.y + ly, z), id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[inline]
fn cell(x: i32, y: i32, z: i32) -> usize {
    (x + z * WIDTH + y * WIDTH * WIDTH) as usize
}

#[inline]
fn coords(i: usize) -> IVec3 {
    let i = i as i32;
    IVec3::new(i % WIDTH, i / (WIDTH * WIDTH), (i / WIDTH) % WIDTH)
}

struct SkyStart {
    values: Vec<u8>,
    seeds: VecDeque<u32>,
}

/// Le ciel « direct » : on descend chaque colonne depuis le haut du monde tant que
/// l'air laisse passer le 15. Seules les cellules au bord de cette zone éclairée
/// servent de départ au BFS : les autres sont entourées de 15 et n'ont rien à propager.
fn sky_light(opacity: &[u8], height: i32) -> SkyStart {
    let mut values = vec![0u8; opacity.len()];
    let mut seeds = VecDeque::new();
    // Pour chaque colonne (x, z) : le plus bas y encore en plein ciel.
    let mut bottom = vec![height; (WIDTH * WIDTH) as usize];
    for z in 0..WIDTH {
        for x in 0..WIDTH {
            let mut y = height - 1;
            while y >= 0 {
                let i = cell(x, y, z);
                let level = propagated(MAX_LIGHT, opacity[i], true);
                if level < MAX_LIGHT {
                    // Premier bloc qui arrête (ou atténue) le ciel.
                    if opacity[i] < MAX_LIGHT && level > 0 {
                        values[i] = level;
                        seeds.push_back(i as u32);
                    }
                    break;
                }
                values[i] = MAX_LIGHT;
                y -= 1;
            }
            bottom[(x + z * WIDTH) as usize] = y + 1;
        }
    }
    // Une cellule de plein ciel est un départ utile si un voisin horizontal, à la
    // même hauteur, n'est pas en plein ciel, ou si elle est la plus basse de sa colonne.
    for z in 0..WIDTH {
        for x in 0..WIDTH {
            let own = bottom[(x + z * WIDTH) as usize];
            let mut highest_neighbour = own + 1;
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if (0..WIDTH).contains(&nx) && (0..WIDTH).contains(&nz) {
                    highest_neighbour = highest_neighbour.max(bottom[(nx + nz * WIDTH) as usize]);
                }
            }
            for y in own..highest_neighbour.min(height) {
                seeds.push_back(cell(x, y, z) as u32);
            }
        }
    }
    SkyStart { values, seeds }
}

/// Le parcours en largeur sur la zone de calcul.
fn bfs(values: &mut [u8], opacity: &[u8], mut queue: VecDeque<u32>, sky: bool, height: i32) {
    while let Some(i) = queue.pop_front() {
        let i = i as usize;
        let level = values[i];
        if level <= 1 {
            continue; // Plus rien à donner aux voisins.
        }
        let p = coords(i);
        for (offset, down) in DIRECTIONS {
            let n = p + offset;
            if n.x < 0 || n.z < 0 || n.y < 0 || n.x >= WIDTH || n.z >= WIDTH || n.y >= height {
                continue;
            }
            let j = cell(n.x, n.y, n.z);
            if opacity[j] >= MAX_LIGHT {
                continue;
            }
            let new = propagated(level, opacity[j], sky && down);
            if new > values[j] {
                values[j] = new;
                queue.push_back(j as u32);
            }
        }
    }
}

/// Un des deux canaux de lumière.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Sky,
    Block,
}

/// Met à jour la lumière autour de `pos`, dont le bloc vient de changer.
///
/// `is_lit(chunk)` dit si la lumière de ce chunk est déjà calculée : on ne touche
/// qu'à ceux-là (les autres seront calculés en entier plus tard, avec les bons blocs).
/// Renvoie les chunks dont le maillage doit être refait.
pub fn update_light(
    world: &mut World,
    pos: IVec3,
    is_lit: impl Fn(IVec3) -> bool,
) -> HashSet<IVec3> {
    let mut editor = LightEditor {
        registry: world.registry().clone(),
        world,
        is_lit,
        touched: HashSet::new(),
    };
    editor.relight(pos, Channel::Sky);
    editor.relight(pos, Channel::Block);
    editor.touched
}

struct LightEditor<'a, F> {
    world: &'a mut World,
    registry: Arc<BlockRegistry>,
    is_lit: F,
    touched: HashSet<IVec3>,
}

impl<F: Fn(IVec3) -> bool> LightEditor<'_, F> {
    /// On peut modifier la lumière à cette position.
    fn editable(&self, pos: IVec3) -> bool {
        self.world.block_y_range().contains(&pos.y) && (self.is_lit)(World::split(pos).0)
    }

    /// Le niveau à `pos`, ou `None` si la position est hors de la zone connue.
    /// Au-dessus du monde, le ciel vaut 15 : c'est une source qu'on lit sans l'écrire.
    fn get(&self, pos: IVec3, channel: Channel) -> Option<u8> {
        if pos.y >= self.world.block_y_range().end {
            return Some(if channel == Channel::Sky {
                MAX_LIGHT
            } else {
                0
            });
        }
        if !self.editable(pos) {
            return None;
        }
        let (sky, block) = self.world.light(pos);
        Some(if channel == Channel::Sky { sky } else { block })
    }

    fn set(&mut self, pos: IVec3, channel: Channel, level: u8) {
        let (chunk_pos, [x, y, z]) = World::split(pos);
        // `chunk_mut` crée au besoin un chunk d'air (lumière par défaut), puis le modifie.
        let chunk = self.world.chunk_mut(chunk_pos);
        match channel {
            Channel::Sky => chunk.set_sky_light(x, y, z, level),
            Channel::Block => chunk.set_block_light(x, y, z, level),
        }
        // Les faces des chunks voisins lisent la lumière jusqu'à un bloc au-delà
        // de leur bord (diagonales comprises) : il faut les remailler aussi.
        let near = |c: usize| match c {
            0 => [-1, 0],
            c if c == CHUNK_SIZE - 1 => [0, 1],
            _ => [0, 0],
        };
        for ox in near(x) {
            for oy in near(y) {
                for oz in near(z) {
                    self.touched.insert(chunk_pos + IVec3::new(ox, oy, oz));
                }
            }
        }
    }

    fn opacity(&self, pos: IVec3) -> u8 {
        self.registry.light_opacity(self.world.block(pos))
    }

    fn emission(&self, pos: IVec3) -> u8 {
        self.registry.light_emission(self.world.block(pos))
    }

    fn relight(&mut self, pos: IVec3, channel: Channel) {
        if !self.editable(pos) {
            return;
        }
        let sky = channel == Channel::Sky;
        let mut removal: VecDeque<(IVec3, u8)> = VecDeque::new();
        let mut add: VecDeque<IVec3> = VecDeque::new();

        // 1. BFS de retrait : on éteint tout ce qui tenait sa lumière de `pos`.
        //    Un voisin moins éclairé que nous la tenait de nous (ou peut-être d'ailleurs :
        //    on l'éteint quand même, il sera rallumé à l'étape 2). Un voisin au moins
        //    aussi éclairé a sa propre source : il servira à rallumer le trou.
        let old = self.get(pos, channel).unwrap_or(0);
        self.set(pos, channel, 0);
        removal.push_back((pos, old));
        while let Some((p, level)) = removal.pop_front() {
            for (offset, down) in DIRECTIONS {
                let n = p + offset;
                let Some(l) = self.get(n, channel) else {
                    continue;
                };
                let fed_by_p =
                    (l != 0 && l < level) || (sky && down && level == MAX_LIGHT && l == MAX_LIGHT);
                if fed_by_p && self.editable(n) {
                    self.set(n, channel, 0);
                    removal.push_back((n, l));
                    // Une source garde sa propre lumière.
                    let emission = if sky { 0 } else { self.emission(n) };
                    if emission > 0 {
                        self.set(n, channel, emission);
                        add.push_back(n);
                    }
                } else if l > 0 {
                    add.push_back(n);
                }
            }
        }

        // Le nouveau bloc est peut-être lui-même une source.
        if !sky {
            let emission = self.emission(pos);
            if emission > 0 {
                self.set(pos, channel, emission);
                add.push_back(pos);
            }
        }

        // 2. BFS d'ajout : on repropage depuis les bords du trou et les sources.
        while let Some(p) = add.pop_front() {
            let Some(level) = self.get(p, channel) else {
                continue;
            };
            if level <= 1 {
                continue;
            }
            for (offset, down) in DIRECTIONS {
                let n = p + offset;
                if !self.editable(n) {
                    continue;
                }
                let opacity = self.opacity(n);
                if opacity >= MAX_LIGHT {
                    continue;
                }
                let new = propagated(level, opacity, sky && down);
                if new > self.get(n, channel).unwrap_or(MAX_LIGHT) {
                    self.set(n, channel, new);
                    add.push_back(n);
                }
            }
        }
    }
}

/// Applique le résultat de [`LightRegion::compute`] aux chunks d'une colonne.
pub fn apply_column_light(world: &mut World, column: IVec2, light: ColumnLight) {
    let bottom = world.vertical().start;
    for (cy, data) in light.into_iter().enumerate() {
        let pos = IVec3::new(column.x, bottom + cy as i32, column.y);
        match data {
            // Une lumière non uniforme : on la range, quitte à créer un chunk d'air.
            Some(data) => world.chunk_mut(pos).set_light_data(data),
            None => {
                if world.chunk(pos).is_some() {
                    world.chunk_mut(pos).reset_light();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::stone;

    fn id(key: &str) -> BlockId {
        BlockRegistry::builtin().id(key).unwrap()
    }

    /// Calcule la lumière de toutes les colonnes de `columns`.
    fn light_all(world: &mut World, columns: &[IVec2]) {
        let results: Vec<_> = columns
            .iter()
            .map(|&c| (c, LightRegion::from_world(world, c).compute()))
            .collect();
        for (c, light) in results {
            apply_column_light(world, c, light);
        }
    }

    /// Un sol de pierre de 0 à `top` (inclus) sur 3 × 3 colonnes autour de (0, 0).
    fn ground(top: i32) -> World {
        let mut world = World::new();
        for x in -32..64 {
            for z in -32..64 {
                for y in 0..=top {
                    world.set_block(IVec3::new(x, y, z), stone());
                }
            }
        }
        world
    }

    #[test]
    fn sky_lights_the_open_air_and_not_the_ground() {
        let mut world = ground(10);
        light_all(&mut world, &[IVec2::ZERO]);
        assert_eq!(
            world.light(IVec3::new(5, 11, 5)),
            (15, 0),
            "juste au-dessus du sol"
        );
        assert_eq!(world.light(IVec3::new(5, 150, 5)), (15, 0));
        assert_eq!(world.light(IVec3::new(5, 10, 5)).0, 0, "dans la pierre");
    }

    #[test]
    fn closed_cave_stays_dark_and_open_shaft_is_lit() {
        let mut world = ground(40);
        // Une grotte fermée de 5 × 3 × 5.
        for x in 10..15 {
            for z in 10..15 {
                for y in 20..23 {
                    world.set_block(IVec3::new(x, y, z), BlockId::AIR);
                }
            }
        }
        // Un puits ouvert vers le ciel.
        for y in 30..=40 {
            world.set_block(IVec3::new(25, y, 25), BlockId::AIR);
        }
        light_all(&mut world, &[IVec2::ZERO]);
        assert_eq!(world.light(IVec3::new(12, 21, 12)), (0, 0), "grotte fermée");
        assert_eq!(
            world.light(IVec3::new(25, 30, 25)).0,
            15,
            "fond du puits : le ciel descend sans perte"
        );
    }

    #[test]
    fn torch_light_decreases_by_one_per_block() {
        let mut world = ground(10);
        world.set_block(IVec3::new(5, 11, 5), id("core:torch"));
        light_all(&mut world, &[IVec2::ZERO]);
        for d in 0..14 {
            assert_eq!(
                world.light(IVec3::new(5 + d, 11, 5)).1,
                14 - d as u8,
                "à {d} blocs"
            );
        }
    }

    #[test]
    fn light_crosses_chunk_borders() {
        let mut world = ground(10);
        // Torche au bord du chunk x = 0 ; on calcule les deux colonnes.
        world.set_block(IVec3::new(31, 11, 5), id("core:torch"));
        light_all(&mut world, &[IVec2::ZERO, IVec2::new(1, 0)]);
        assert_eq!(world.light(IVec3::new(32, 11, 5)).1, 13);
        assert_eq!(world.light(IVec3::new(40, 11, 5)).1, 5);
    }

    /// Le test clé : après des modifications, la mise à jour incrémentale doit donner
    /// exactement la même lumière qu'un calcul complet.
    #[test]
    fn incremental_updates_match_a_full_recompute() {
        let generator = crate::worldgen::generator::TerrainGenerator::new(3);
        let mut world = generator.generate_region(5, 5);
        let lit: Vec<IVec2> = (1..4)
            .flat_map(|x| (1..4).map(move |z| IVec2::new(x, z)))
            .collect();
        light_all(&mut world, &lit);
        let is_lit = |c: IVec3| (1..4).contains(&c.x) && (1..4).contains(&c.z);

        let torch = id("core:torch");
        let glow = id("core:glowstone");
        let water = id("core:water");
        let top = generator.column(80, 80).height;
        // Des modifications variées au centre de la colonne (2, 2) : x et z de 64 à 95.
        let edits = [
            (IVec3::new(80, top + 1, 80), torch),
            (IVec3::new(80, top + 6, 80), stone()), // un toit au-dessus de la torche
            (IVec3::new(82, top, 80), BlockId::AIR), // un trou dans le sol
            (IVec3::new(70, top - 3, 75), glow),    // une source enterrée
            (IVec3::new(81, top + 1, 80), water),
            (IVec3::new(80, top + 1, 80), BlockId::AIR), // on retire la torche
            (IVec3::new(80, top + 6, 80), BlockId::AIR), // et le toit
            (IVec3::new(90, top + 2, 66), torch),
        ];
        for (pos, block) in edits {
            world.set_block(pos, block);
            update_light(&mut world, pos, is_lit);
        }

        let mut fresh = world.clone();
        light_all(&mut fresh, &[IVec2::new(2, 2)]);
        for y in 0..crate::worldgen::generator::WORLD_HEIGHT_CHUNKS * SIZE {
            for z in 64..96 {
                for x in 64..96 {
                    let p = IVec3::new(x, y, z);
                    assert_eq!(world.light(p), fresh.light(p), "en {p}");
                }
            }
        }
    }

    #[test]
    fn removing_a_torch_turns_its_light_off() {
        let mut world = ground(40);
        // Une pièce fermée et une torche au milieu.
        for x in 8..20 {
            for z in 8..20 {
                for y in 20..24 {
                    world.set_block(IVec3::new(x, y, z), BlockId::AIR);
                }
            }
        }
        light_all(&mut world, &[IVec2::ZERO]);
        let torch = IVec3::new(14, 21, 14);
        let is_lit = |c: IVec3| c.x == 0 && c.z == 0;
        world.set_block(torch, id("core:torch"));
        update_light(&mut world, torch, is_lit);
        assert_eq!(world.light(IVec3::new(10, 21, 14)).1, 10);

        world.set_block(torch, BlockId::AIR);
        let touched = update_light(&mut world, torch, is_lit);
        for x in 8..20 {
            assert_eq!(world.light(IVec3::new(x, 21, 14)), (0, 0), "x = {x}");
        }
        assert!(touched.contains(&IVec3::new(0, 0, 0)));
    }

    #[test]
    fn placing_a_roof_casts_a_shadow_down() {
        let mut world = ground(10);
        light_all(&mut world, &[IVec2::ZERO]);
        let is_lit = |c: IVec3| c.x == 0 && c.z == 0;
        let roof = IVec3::new(16, 100, 16);
        world.set_block(roof, stone());
        let touched = update_light(&mut world, roof, is_lit);
        // Juste dessous, le ciel arrive par les côtés : 15 - 1.
        assert_eq!(world.light(IVec3::new(16, 99, 16)).0, 14);
        assert_eq!(world.light(IVec3::new(16, 11, 16)).0, 14);
        // L'ombre descend jusqu'au sol : les chunks y = 0 à 3 sont à remailler.
        for y in 0..4 {
            assert!(touched.contains(&IVec3::new(0, y, 0)), "chunk y = {y}");
        }
    }
}
