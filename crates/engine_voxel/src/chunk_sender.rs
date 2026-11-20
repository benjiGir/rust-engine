//! L'expéditeur de chunks : ce qu'un joueur voit du monde du serveur.
//!
//! Chaque joueur a le sien. À chaque pas du serveur, il lit le [`ChunkStreamer`] du serveur et
//! produit des [`ChunkUpdate`] : les chunks que le joueur n'a pas encore (du plus proche au plus
//! lointain, sous un budget), ceux qu'il doit oublier, et ceux qui ont changé.
//!
//! - **Colonnes entières.** Un chunk n'est maillé que si ses 26 voisins sont là : l'expéditeur envoie
//!   donc une colonne (tous ses chunks non vides) d'un seul coup, et seulement quand sa lumière est
//!   calculée (elle est dans le chunk). La réplique du client sait ainsi, dès qu'un chunk d'une colonne
//!   arrive, que toute la colonne est là.
//! - **Le rayon.** Les colonnes envoyées sont celles que le serveur éclaire pour la vue du joueur (le
//!   rayon de vue, plus une colonne autour : voir `ChunkStreamer`) ; le client maille jusqu'au rayon
//!   de vue, avec ses voisins éclairés. Une colonne n'est reprise qu'au-delà de ce rayon, plus une
//!   marge (hystérésis : faire un pas d'avant en arrière ne décharge rien).
//! - **Sans copie.** Les chunks sont des `Arc` : la réplique du même processus tient les mêmes blocs
//!   que le serveur. Au réseau (lot 5), le transport les encodera.
//! - **Les changements.** Le serveur note les chunks qui changent ([`ChunkChange`]) ; l'expéditeur
//!   renvoie ceux que le joueur a, avec le chunk tel qu'il est maintenant. Un chunk apparu dans une
//!   colonne déjà envoyée (un bloc posé en plein ciel) part aussi.

use glam::{IVec2, IVec3};
use rustc_hash::FxHashMap as HashMap;

use crate::streaming::{ChunkChange, ChunkStreamer, ChunkUpdate};

/// Chunks envoyés au plus par pas (une colonne entamée est finie) : le même budget pour tous les
/// joueurs, pour que le serveur ne passe pas son pas à envoyer.
pub const DEFAULT_SEND_BUDGET: usize = 256;

/// Marge de déchargement, en chunks, au-delà du rayon envoyé.
const UNLOAD_MARGIN: i32 = 2;

/// Ce qu'un joueur a reçu du monde, et ce qu'il lui reste à recevoir.
#[derive(Debug)]
pub struct ChunkSender {
    /// Le rayon de vue du client, en chunks.
    radius: i32,
    budget: usize,
    /// La colonne du joueur au dernier pas.
    centre: Option<IVec2>,
    /// Les chunks envoyés, par colonne : les hauteurs (en chunks) qui ont été envoyées.
    sent: HashMap<IVec2, Vec<i32>>,
    /// Les colonnes à envoyer, de la plus proche à la plus lointaine.
    waiting: Vec<IVec2>,
}

impl ChunkSender {
    /// `view_radius` : le rayon de vue du client (celui de sa réplique).
    pub fn new(view_radius: i32) -> Self {
        Self {
            radius: view_radius,
            budget: DEFAULT_SEND_BUDGET,
            centre: None,
            sent: HashMap::default(),
            waiting: Vec::new(),
        }
    }

    /// Change le nombre de chunks envoyés au plus par pas.
    pub fn with_budget(mut self, chunks: usize) -> Self {
        self.budget = chunks.max(1);
        self
    }

    /// Des colonnes attendent encore d'être envoyées (pas encore éclairées, ou sous le budget).
    pub fn pending(&self) -> usize {
        self.waiting.len()
    }

    /// Le nombre de chunks que le joueur a reçus.
    pub fn sent_chunks(&self) -> usize {
        self.sent.values().map(Vec::len).sum()
    }

    /// Ce joueur a ce chunk.
    pub fn has(&self, chunk: IVec3) -> bool {
        self.sent
            .get(&IVec2::new(chunk.x, chunk.z))
            .is_some_and(|layers| layers.contains(&chunk.y))
    }

    /// Un pas : ce qu'il faut envoyer au joueur dont le corps est dans le chunk `player`, d'après le
    /// monde du serveur et les changements de ce pas (`changes`, communs à tous les joueurs). Les
    /// messages sont ajoutés à `out`, dans l'ordre : libérations, changements, chunks neufs.
    pub fn step(
        &mut self,
        server: &ChunkStreamer,
        player: IVec3,
        changes: &[ChunkChange],
        out: &mut Vec<ChunkUpdate>,
    ) {
        let centre = IVec2::new(player.x, player.z);
        if self.centre != Some(centre) {
            self.centre = Some(centre);
            self.recentre(centre, out);
        }

        // Les chunks changés que le joueur a, ou qui apparaissent dans une colonne qu'il a. Ceux d'une
        // colonne pas encore envoyée partiront entiers, avec leur état d'aujourd'hui.
        for change in changes {
            let column = IVec2::new(change.pos.x, change.pos.z);
            let Some(layers) = self.sent.get_mut(&column) else {
                continue;
            };
            let Some(chunk) = server.world().chunk_arc(change.pos) else {
                continue;
            };
            if !layers.contains(&change.pos.y) {
                layers.push(change.pos.y);
            }
            out.push(ChunkUpdate::Changed {
                pos: change.pos,
                chunk,
                edits: change.edits.clone(),
                remesh: change.remesh,
            });
        }

        // Les colonnes neuves, du plus proche au plus lointain, jusqu'au budget.
        if self.waiting.is_empty() {
            return;
        }
        let vertical = server.world().vertical();
        let mut chunks = 0;
        let Self {
            sent,
            waiting,
            budget,
            ..
        } = self;
        waiting.retain(|&column| {
            if chunks >= *budget || !server.is_lit(IVec3::new(column.x, 0, column.y)) {
                return true;
            }
            let mut layers = Vec::new();
            for y in vertical.clone() {
                let pos = IVec3::new(column.x, y, column.y);
                if let Some(chunk) = server.world().chunk_arc(pos) {
                    out.push(ChunkUpdate::Chunk { pos, chunk });
                    layers.push(y);
                }
            }
            chunks += layers.len().max(1);
            sent.insert(column, layers);
            false
        });
    }

    /// Le joueur a changé de colonne : on reprend ce qui est trop loin, et on refait la liste de ce
    /// qui reste à envoyer.
    fn recentre(&mut self, centre: IVec2, out: &mut Vec<ChunkUpdate>) {
        let keep = (self.radius + 1 + UNLOAD_MARGIN).pow(2);
        let far: Vec<IVec2> = self
            .sent
            .keys()
            .copied()
            .filter(|c| (*c - centre).length_squared() > keep)
            .collect();
        for column in far {
            for y in self.sent.remove(&column).unwrap_or_default() {
                out.push(ChunkUpdate::Unload(IVec3::new(column.x, y, column.y)));
            }
        }
        let reach = self.radius + 1;
        self.waiting.clear();
        for dx in -reach..=reach {
            for dz in -reach..=reach {
                let column = centre + IVec2::new(dx, dz);
                if in_range(column, centre, self.radius) && !self.sent.contains_key(&column) {
                    self.waiting.push(column);
                }
            }
        }
        self.waiting
            .sort_by_cached_key(|c| (*c - centre).length_squared());
    }
}

/// Le serveur éclaire une colonne si elle touche (diagonales comprises) une colonne du rayon de vue :
/// c'est ce que la réplique demande pour mailler jusqu'au bord de son rayon.
fn in_range(column: IVec2, centre: IVec2, radius: i32) -> bool {
    let view_sq = radius * radius;
    (-1..=1).any(|dx| {
        (-1..=1).any(|dz| (column + IVec2::new(dx, dz) - centre).length_squared() <= view_sq)
    })
}
