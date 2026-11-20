//! Plusieurs centres de chargement : un par joueur.
//!
//! Un monde où jouent plusieurs personnes doit rester chargé autour de **chacune**. Chaque centre
//! est une [`View`]. Un seul est « le maillé » : celui de la fenêtre, pour qui on construit les
//! maillages et le terrain lointain. Les autres chargent, génèrent et éclairent seulement : leurs
//! joueurs n'ont rien à dessiner ici, mais le monde autour d'eux doit être là (physique, machines,
//! créatures).
//!
//! Les chunks chargés sont l'union des rayons de chaque vue, triés par la meilleure priorité parmi
//! elles, et un chunk n'est libéré que s'il est hors du rayon de toutes.

use glam::{IVec3, Vec2, Vec3};

use super::{ChunkStreamer, Role};
use crate::world::World;

/// Un centre de chargement : un joueur, et où il regarde.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Où il est (n'importe quel point de son corps : seul compte le chunk qui le contient).
    pub position: Vec3,
    /// Où il regarde, pour charger devant lui avant derrière (voir `StreamingConfig::view_bias`).
    /// `Vec3::ZERO` n'en donne aucune ; seule la vue qui maille s'en sert.
    pub forward: Vec3,
    /// Cette vue est celle dont on construit les maillages (la fenêtre locale). S'il y en a
    /// plusieurs, la première compte ; s'il n'y en a aucune (un serveur sans fenêtre), rien n'est
    /// maillé et la première vue sert de référence pour le reste.
    pub mesh: bool,
}

impl ChunkStreamer {
    /// Une image : suivre toutes les vues, récupérer les résultats, lancer de nouvelles tâches.
    /// Avec une seule vue qui maille, c'est exactement [`ChunkStreamer::update_view`].
    pub fn update_views(&mut self, views: &[View]) {
        let _span = engine_trace::span("chunks : file et réception");
        let Some(main) = views.iter().find(|v| v.mesh).or(views.first()) else {
            // Aucune vue : on garde ce qui est chargé, et on range seulement les résultats.
            self.receive();
            return;
        };
        let centre = World::chunk_containing(main.position);
        let flat = Vec2::new(main.forward.x, main.forward.z);
        if flat.length_squared() > 0.04 {
            self.view = Some(flat.normalize());
        }
        let mut others: Vec<IVec3> = views
            .iter()
            .filter(|v| !std::ptr::eq(*v, main))
            .map(|v| World::chunk_containing(v.position))
            .filter(|c| *c != centre)
            .collect();
        others.sort_by_key(|c| (c.x, c.y, c.z));
        others.dedup();
        // Un serveur ne maille jamais : les maillages sont ceux de la réplique de chaque client.
        let meshing = self.role != Role::Server && views.iter().any(|v| v.mesh);
        if self.centre != Some(centre) || self.others != others || self.meshing != meshing {
            self.centre = Some(centre);
            self.others = others;
            self.meshing = meshing;
            if self.role == Role::Replica {
                // Rien à charger ni à libérer : les chunks viennent des messages du serveur.
                self.refresh_meshing(centre);
            } else {
                self.refresh(centre);
            }
        } else if self.view_changed_enough() {
            self.resort_load_queue(centre);
        }
        // Les blocs modifiés reçus se remaillent dans cette image, avant tout résultat de fond.
        self.flush_edits();
        self.receive();
        // Les chunks modifiés en silence (machines) : un seul remaillage par chunk et par image.
        for pos in std::mem::take(&mut self.remesh_pending) {
            self.remesh_soon(pos);
        }
        self.schedule(centre);
    }
}

#[cfg(test)]
mod tests;
