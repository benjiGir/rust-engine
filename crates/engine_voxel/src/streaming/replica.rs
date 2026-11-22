//! Le streamer en trois rôles : seul, serveur, ou réplique d'un serveur.
//!
//! Dans le jeu, le monde est tenu en deux endroits (voir `docs/plan-multijoueur.md`, lot 3a) :
//!
//! - le **serveur** charge, génère, éclaire et sauvegarde les chunks, et applique les actions des
//!   joueurs. Il ne maille rien : il note seulement ce qui a changé ([`ChunkChange`]), et un
//!   `ChunkSender` en fait des messages ([`ChunkUpdate`]) pour chaque joueur ;
//! - la **réplique** du client ne génère rien, n'éclaire rien et ne sauvegarde rien : elle reçoit des
//!   chunks tout faits (lumière comprise) et les maille. C'est le même `ChunkStreamer`, donc le même
//!   maillage (priorités, budget d'envoi, couches sombres, remaillage des voisins).
//!
//! Les chunks passent en `Arc<Chunk>` sans copie : le serveur et la réplique partagent les mêmes
//! blocs. Quand le serveur modifie un chunk que la réplique tient encore, `Arc::make_mut` en fait une
//! copie (environ 96 Kio) ; la réplique garde l'ancien jusqu'à ce que le nouveau lui arrive.
//!
//! # Une édition se voit dans la même image
//!
//! Le serveur note, pour un bloc posé ou cassé, sa position ([`ChunkChange::edits`]). La réplique,
//! une fois tous les chunks de l'image remplacés, refait la géométrie de chaque position (le chunk et
//! ses voisins, comme le faisait `set_block`) et remaille tout de suite, hors budget. Un changement
//! sans position (la lumière d'un chunk voisin, une machine qui travaille) remaille en tâche de fond ;
//! un changement de données seules (le contenu d'un coffre) ne remaille rien.

use std::sync::Arc;

use glam::{IVec2, IVec3};
use rustc_hash::FxHashSet as HashSet;

use super::{ChunkStreamer, LightState, MeshState, Slot, edit_geometry};
use crate::chunk::Chunk;
use crate::world::World;

/// Ce que fait un streamer de son monde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Tout : charger, éclairer, mailler (les tests, les outils).
    Standalone,
    /// Charger, éclairer et sauvegarder, sans jamais mailler : le monde du serveur.
    Server,
    /// Recevoir des chunks tout faits et les mailler : le monde que voit le client.
    Replica,
}

/// Un chunk du serveur changé depuis le dernier [`ChunkStreamer::take_changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkChange {
    pub pos: IVec3,
    /// Les blocs posés ou cassés dans ce chunk par une action immédiate : le client les remaille dans
    /// l'image. Vide pour un changement qui ne touche pas la géométrie, ou qui peut attendre.
    pub edits: Vec<IVec3>,
    /// Ce chunk (lumière, fluide, machine, forme) doit être remaillé, sans urgence.
    pub remesh: bool,
}

/// Ce que le serveur envoie à un client pour son monde, et que sa réplique applique.
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkUpdate {
    /// Un chunk que le client n'avait pas.
    Chunk { pos: IVec3, chunk: Arc<Chunk> },
    /// Un chunk qui a changé : le nouveau, avec les positions des blocs édités (voir
    /// [`ChunkChange`]). Si le client ne l'avait pas, il l'ajoute.
    Changed {
        pos: IVec3,
        chunk: Arc<Chunk>,
        edits: Vec<IVec3>,
        remesh: bool,
    },
    /// Un chunk que le serveur a généré sans rien y mettre (de l'air) : il n'existe pas en mémoire,
    /// mais le client doit le savoir prêt (sinon un joueur à côté attendrait son chargement pour
    /// toujours).
    Empty(IVec3),
    /// Un chunk que le client doit oublier.
    Unload(IVec3),
}

impl ChunkStreamer {
    /// Ce streamer devient un serveur : il charge, génère, éclaire et sauvegarde, mais ne maille
    /// plus rien. Les changements s'obtiennent avec [`ChunkStreamer::take_changes`].
    pub fn as_server(mut self) -> Self {
        self.role = Role::Server;
        self.meshing = false;
        self
    }

    /// La réplique d'un serveur : même registre, mêmes réglages de maillage et mêmes threads de fond,
    /// mais aucun chunk au départ. Elle ne génère, n'éclaire ni ne sauvegarde jamais ; ses chunks
    /// viennent de [`ChunkStreamer::apply_updates`]. (Elle garde le générateur du serveur pour le
    /// registre et la hauteur du monde, jamais pour générer.)
    pub fn replica_of(source: &ChunkStreamer) -> Self {
        let mut replica = Self::with_generator(
            source.generator.clone(),
            source.config.clone(),
            Role::Replica,
            Some(source.pool.clone()),
        );
        replica.world.set_registry(source.world.registry().clone());
        if let Some(fluids) = source.world.fluids() {
            replica.world.set_fluids(fluids.clone());
        }
        replica
    }

    /// Les chunks du serveur changés depuis le dernier appel, une fois chacun.
    pub fn take_changes(&mut self) -> Vec<ChunkChange> {
        self.changes.drain().map(|(_, change)| change).collect()
    }

    /// Note qu'un chunk du serveur a changé (rien d'autre ailleurs : seul le serveur suit les changements).
    pub(super) fn note_change(&mut self, chunk: IVec3, edit: Option<IVec3>, remesh: bool) {
        if self.role != Role::Server {
            return;
        }
        let change = self.changes.entry(chunk).or_insert_with(|| ChunkChange {
            pos: chunk,
            edits: Vec::new(),
            remesh: false,
        });
        change.edits.extend(edit);
        change.remesh |= remesh;
    }

    /// Un chunk doit être remaillé en tâche de fond : sur place, ou chez les clients pour un serveur.
    pub(super) fn request_remesh(&mut self, chunk: IVec3) {
        if self.role == Role::Server {
            self.note_change(chunk, None, true);
        } else {
            self.remesh_pending.insert(chunk);
        }
    }

    /// Le serveur change un bloc : un changement immédiat porte sa position (la réplique remaille
    /// dans l'image), un changement silencieux remaille les chunks voisins en tâche de fond.
    pub(super) fn note_cell_change(
        &mut self,
        pos: IVec3,
        immediate: bool,
        light_changed: impl IntoIterator<Item = IVec3>,
    ) {
        let chunk = World::split(pos).0;
        if immediate {
            self.note_change(chunk, Some(pos), false);
        } else {
            for c in edit_geometry(pos) {
                self.note_change(c, None, true);
            }
        }
        // Seule leur lumière a changé : ils sont renvoyés, et remaillés si la géométrie ne l'a pas fait.
        for c in light_changed {
            self.note_change(c, None, true);
        }
    }

    /// Réplique : applique les messages du serveur de cette image. Les chunks sont remplacés d'abord, les
    /// éditions se remaillent à la prochaine mise à jour des vues (voir `update_views`), une fois tous
    /// les chunks de l'image en place.
    pub fn apply_updates(&mut self, updates: impl IntoIterator<Item = ChunkUpdate>) {
        debug_assert!(self.role == Role::Replica, "réservé à la réplique");
        let mut gone: Vec<IVec3> = Vec::new();
        for update in updates {
            match update {
                ChunkUpdate::Chunk { pos, chunk } => {
                    self.put_chunk(pos, chunk, true);
                }
                ChunkUpdate::Changed {
                    pos,
                    chunk,
                    edits,
                    remesh,
                } => {
                    let fresh = self.put_chunk(pos, chunk, remesh);
                    self.pending_edits.extend(edits);
                    if remesh && !fresh {
                        self.remesh_pending.insert(pos);
                    }
                }
                ChunkUpdate::Empty(pos) => self.put_empty(pos),
                ChunkUpdate::Unload(pos) => gone.push(pos),
            }
        }
        if !gone.is_empty() {
            self.drop_chunks(&gone);
        }
    }

    /// Un chunk vide reçu : prêt, sans rien à stocker ni à mailler. Un chunk qui avait des blocs (on
    /// les a tous cassés) les perd.
    fn put_empty(&mut self, pos: IVec3) {
        let had = self.world.remove(pos).is_some();
        if self.slots.contains_key(&pos) {
            if had {
                self.remesh_pending.insert(pos);
            }
            return;
        }
        self.slots.insert(pos, Slot::Generated(MeshState::Done));
        let info = self.columns.entry(IVec2::new(pos.x, pos.z)).or_default();
        info.generated += 1;
        info.light = LightState::Done;
    }

    /// Place un chunk reçu. Renvoie `true` si la réplique ne l'avait pas. `relit` : sa lumière a pu
    /// changer (le sombre est à recalculer).
    fn put_chunk(&mut self, pos: IVec3, chunk: Arc<Chunk>, relit: bool) -> bool {
        let fresh = !self.slots.contains_key(&pos);
        // Aucune lumière du ciel : le maillage peut attendre le joueur (voir `StreamingConfig::mesh_depth`).
        if relit || fresh {
            if chunk.light_data().iter().all(|v| v >> 4 == 0) {
                self.dark.insert(pos);
            } else {
                self.dark.remove(&pos);
            }
        }
        self.world.insert(pos, chunk);
        if !fresh {
            return false;
        }
        if let Some(timings) = &mut self.timings {
            // Le chunk entre dans la file de maillage : la génération et la lumière sont déjà faites.
            timings.requested(pos);
            timings.generated(pos);
        }
        self.slots.insert(pos, Slot::Generated(MeshState::Missing));
        let column = IVec2::new(pos.x, pos.z);
        let info = self.columns.entry(column).or_default();
        let first = info.generated == 0;
        info.generated += 1;
        // Un chunk n'arrive qu'avec sa lumière : sa colonne est éclairée pour le maillage.
        info.light = LightState::Done;
        if first {
            // Les voisins de cette colonne attendaient peut-être qu'elle soit là.
            for dx in -1..=1 {
                for dz in -1..=1 {
                    for y in self.world.vertical() {
                        self.mesh_candidates
                            .insert(IVec3::new(column.x + dx, y, column.y + dz));
                    }
                }
            }
        } else {
            self.mesh_candidates.insert(pos);
        }
        true
    }

    /// Oublie des chunks : leurs maillages sont libérés côté GPU, étalés sur plusieurs images.
    fn drop_chunks(&mut self, gone: &[IVec3]) {
        for &pos in gone {
            self.world.remove(pos);
            self.slots.remove(&pos);
            self.dark.remove(&pos);
            self.mesh_candidates.remove(&pos);
            let column = IVec2::new(pos.x, pos.z);
            if let Some(info) = self.columns.get_mut(&column) {
                info.generated = info.generated.saturating_sub(1);
                if info.generated == 0 {
                    self.columns.remove(&column);
                }
            }
            if self.unloads_pending.insert(pos) {
                self.unloads.push_back(pos);
            }
        }
        // Un seul passage sur chaque file (voir `refresh`).
        let gone: HashSet<IVec3> = gone.iter().copied().collect();
        self.urgent
            .retain(|e| !matches!(e, super::StreamEvent::Mesh(p, _) if gone.contains(p)));
        self.waiting.retain(|p, _| !gone.contains(p));
        self.meshing_dirty.retain(|p| !gone.contains(p));
        self.remesh_pending.retain(|p| !gone.contains(p));
    }

    /// Réplique : remaille dans l'image les chunks des blocs édités reçus, le chunk de la première
    /// position en tête (comme `change_cell` pour une action immédiate du joueur).
    pub(super) fn flush_edits(&mut self) {
        if self.pending_edits.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut self.pending_edits);
        let mut seen: HashSet<IVec3> = HashSet::default();
        let mut geometry: Vec<IVec3> = Vec::new();
        for pos in edits {
            for chunk in edit_geometry(pos) {
                if seen.insert(chunk) {
                    geometry.push(chunk);
                }
            }
        }
        // `urgent.push_front` : dans l'ordre inverse, pour que le premier arrive en tête.
        for &chunk in geometry.iter().rev() {
            self.remesh_now(chunk);
        }
        // Déjà remaillés : pas de seconde tâche pour eux.
        self.remesh_pending.retain(|p| !seen.contains(p));
    }
}

#[cfg(test)]
mod tests;
