//! Le résumé des chunks modifiés, pour que le terrain lointain montre ce que le joueur a construit
//! ou creusé et pas seulement ce que le générateur aurait fait.
//!
//! Un chunk modifié puis déchargé (ou resté sur disque d'une session précédente) laisse un
//! [`ChunkSurface`] : pour chacune de ses 32 × 32 colonnes, le bloc de **sol** le plus haut. Les
//! tuiles lointaines ([`crate::far`]) le préfèrent au générateur ([`EditedSurfaces::surface_at`]).
//!
//! # Qu'est-ce qu'un sol ?
//!
//! Un cube plein, opaque, ni liquide, ni feuillage, ni bûche : les arbres naturels, l'herbe haute et
//! l'eau ne comptent pas (le lointain ne les dessine pas non plus : un tronc isolé ferait une aiguille
//! de dix blocs à chaque cellule qui le touche). Les planches, la pierre, la terre posées par le joueur
//! comptent.
//!
//! # D'où viennent les résumés
//!
//! - au déchargement d'un chunk modifié, le streamer en note le résumé ([`EditedSurfaces::record`]) ;
//! - les chunks déjà sauvegardés sont lus **paresseusement** dans le dossier du monde, par colonne de
//!   chunks, la première fois qu'une tuile lointaine les demande (sur le thread qui la construit).
//!
//! Chaque résumé noté signale sa colonne ([`EditedSurfaces::take_changed`]) : le terrain lointain
//! reconstruit alors les tuiles montrées qui la couvrent.
//!
//! # Limites
//!
//! Une colonne de chunk entièrement creusée jusqu'en dessous de la surface générée, dans un chunk
//! non modifié plus bas, retombe sur le générateur (rare). Seul le sol compte : ni structures
//! mobiles, ni eau ajoutée par le joueur.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use glam::IVec3;

use crate::block::BlockId;
use crate::chunk::{CHUNK_SIZE, Chunk};
use crate::generator::{MAX_CHUNK_Y, MIN_CHUNK_Y};
use crate::registry::{BlockDef, BlockRegistry, Shape, Tint};
use crate::save::SaveStore;

/// Au-delà, on oublie quelles colonnes ont déjà été lues sur disque (elles seront relues, sans
/// doublon) : la mémoire reste bornée pendant une longue exploration.
const SCANNED_LIMIT: usize = 100_000;

/// Un bloc que le lointain traite comme le sol.
fn is_ground(def: &BlockDef) -> bool {
    def.solid
        && !def.liquid
        && !def.cutout
        && def.alpha >= 1.0
        && def.shape == Shape::Cube
        && def.tint != Tint::Foliage
        && !def.key.ends_with("_log")
        && !def.key.ends_with("_leaves")
}

/// Le bloc de sol le plus haut de chacune des colonnes d'un chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSurface {
    /// y local du sol (0..32) ; −1 : aucun sol dans la colonne.
    tops: Box<[i8]>,
    blocks: Box<[BlockId]>,
}

impl ChunkSurface {
    pub fn of(chunk: &Chunk, registry: &BlockRegistry) -> Self {
        let size = CHUNK_SIZE;
        let mut tops = vec![-1i8; size * size].into_boxed_slice();
        let mut blocks = vec![BlockId::AIR; size * size].into_boxed_slice();
        // Les classes de blocs se répètent beaucoup : on retient la réponse par identifiant.
        let mut ground: HashMap<BlockId, bool> = HashMap::new();
        for z in 0..size {
            for x in 0..size {
                for y in (0..size).rev() {
                    let id = chunk.get(x, y, z);
                    if id.is_air() {
                        continue;
                    }
                    let solid = *ground
                        .entry(id)
                        .or_insert_with(|| is_ground(registry.def(id)));
                    if solid {
                        tops[x + z * size] = y as i8;
                        blocks[x + z * size] = id;
                        break;
                    }
                }
            }
        }
        Self { tops, blocks }
    }

    /// Le sol de la colonne locale (x, z) : son y local et son bloc.
    pub fn at(&self, x: usize, z: usize) -> Option<(i32, BlockId)> {
        let i = x + z * CHUNK_SIZE;
        (self.tops[i] >= 0).then(|| (i32::from(self.tops[i]), self.blocks[i]))
    }
}

/// Les résumés de tous les chunks modifiés connus, partagés entre le streamer et les threads du
/// terrain lointain.
#[derive(Default)]
pub struct EditedSurfaces {
    surfaces: RwLock<HashMap<IVec3, Arc<ChunkSurface>>>,
    /// Colonnes de chunks dont le disque a déjà été lu.
    scanned: Mutex<HashSet<(i32, i32)>>,
    changed: Mutex<HashSet<(i32, i32)>>,
    store: Mutex<Option<Arc<SaveStore>>>,
}

impl EditedSurfaces {
    pub fn new() -> Self {
        Self::default()
    }

    /// Le dossier du monde où lire les chunks déjà sauvegardés.
    pub fn attach_store(&self, store: Arc<SaveStore>) {
        *self.store.lock().unwrap_or_else(|e| e.into_inner()) = Some(store);
    }

    /// Note le résumé d'un chunk modifié (à son déchargement) et signale sa colonne.
    pub fn record(&self, pos: IVec3, chunk: &Chunk, registry: &BlockRegistry) {
        let surface = Arc::new(ChunkSurface::of(chunk, registry));
        self.surfaces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pos, surface);
        self.changed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((pos.x, pos.z));
    }

    /// Les colonnes de chunks dont un résumé a changé depuis le dernier appel.
    pub fn take_changed(&self) -> Vec<(i32, i32)> {
        let mut changed = self.changed.lock().unwrap_or_else(|e| e.into_inner());
        changed.drain().collect()
    }

    /// Le nombre de chunks résumés.
    pub fn len(&self) -> usize {
        self.surfaces
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lit sur disque les chunks sauvegardés de la colonne (une fois), sans écraser un résumé plus
    /// récent noté par [`Self::record`].
    fn scan_column(&self, cx: i32, cz: i32, registry: &BlockRegistry) {
        {
            let mut scanned = self.scanned.lock().unwrap_or_else(|e| e.into_inner());
            if scanned.len() > SCANNED_LIMIT {
                scanned.clear();
            }
            if !scanned.insert((cx, cz)) {
                return;
            }
        }
        let Some(store) = self.store.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return;
        };
        for cy in store.saved_heights(cx, cz) {
            let pos = IVec3::new(cx, cy, cz);
            if self
                .surfaces
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&pos)
            {
                continue;
            }
            if let Ok(Some(loaded)) = store.load_chunk(pos, registry) {
                let surface = Arc::new(ChunkSurface::of(&loaded.chunk, registry));
                self.surfaces
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(pos)
                    .or_insert(surface);
            }
        }
    }

    /// Le sol modifié de la colonne de blocs (x, z), s'il y en a un : son y (monde) et son bloc.
    /// `generated_top` est le sol que donnerait le générateur : un chunk non modifié à ce niveau ou
    /// en dessous est intact, donc le générateur a raison.
    pub fn surface_at(
        &self,
        x: i32,
        z: i32,
        generated_top: i32,
        registry: &BlockRegistry,
    ) -> Option<(i32, BlockId)> {
        let size = CHUNK_SIZE as i32;
        let (cx, cz) = (x.div_euclid(size), z.div_euclid(size));
        self.scan_column(cx, cz, registry);
        let (lx, lz) = (x.rem_euclid(size) as usize, z.rem_euclid(size) as usize);
        let generated_chunk = generated_top.div_euclid(size);
        let surfaces = self.surfaces.read().unwrap_or_else(|e| e.into_inner());
        for cy in (MIN_CHUNK_Y..MAX_CHUNK_Y).rev() {
            match surfaces.get(&IVec3::new(cx, cy, cz)) {
                // Non modifié : au niveau du sol généré ou dessous, c'est le générateur ; au-dessus,
                // du vide (ou de l'arbre, que le lointain ignore).
                None if cy <= generated_chunk => return None,
                None => {}
                Some(surface) => {
                    if let Some((y, block)) = surface.at(lx, lz) {
                        return Some((cy * size + y, block));
                    }
                }
            }
        }
        None
    }
}

impl std::fmt::Debug for EditedSurfaces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditedSurfaces")
            .field("chunks", &self.len())
            .finish()
    }
}
