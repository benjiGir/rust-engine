//! La part fenêtre de `ghosts.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `ghosts.rs`).

use crate::VoxelGame;
use crate::control::{json_str, vec3};
use anyhow::{Result, bail};
#[allow(unused_imports)]
use voxel_sim::net::ghosts::*;
use voxel_sim::player::players::PlayerId;
use voxel_sim::player::{self};

impl VoxelGame {
    /// Exécute une commande des fantômes ; renvoie la réponse JSON.
    pub(crate) fn ghost_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Add(name, at) => {
                let feet = at.unwrap_or_else(|| {
                    let body =
                        player::body(&self.server.ecs, self.server.players.local().entity).position;
                    body + self.client.camera.forward_flat() * 4.0
                });
                let number = self.server.players.iter().filter(|p| p.ghost).count() + 1;
                let name = name.unwrap_or_else(|| format!("Fantôme {number}"));
                self.server.add_ghost(&name, feet);
                Ok(format!(
                    "{{\"ok\":true,\"name\":{},\"pos\":{}}}",
                    json_str(&name),
                    vec3(feet)
                ))
            }
            Command::Remove(who) => {
                let gone: Vec<(PlayerId, String)> = self
                    .server
                    .players
                    .iter()
                    .filter(|p| p.ghost && who.as_ref().is_none_or(|w| *w == p.name))
                    .map(|p| (p.id, p.name.clone()))
                    .collect();
                if gone.is_empty() {
                    bail!("ghost remove : aucun fantôme à retirer");
                }
                for (id, _) in &gone {
                    self.remove_ghost(*id);
                }
                Ok(format!("{{\"ok\":true,\"removed\":{}}}", gone.len()))
            }
            Command::List => {
                let rows: Vec<String> = self
                    .server
                    .players
                    .iter()
                    .filter(|p| p.ghost)
                    .map(|p| {
                        let at = player::body(&self.server.ecs, p.entity).position;
                        format!("{{\"name\":{},\"pos\":{}}}", json_str(&p.name), vec3(at))
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"ghosts\":[{}]}}", rows.join(",")))
            }
        }
    }

    /// Retire un joueur (et son entité) ; le client oublie ce qu'il lui avait envoyé.
    pub(crate) fn remove_ghost(&mut self, id: PlayerId) {
        if let Some(state) = self.server.players.remove(id) {
            self.server.ecs.despawn(state.entity);
            self.server.players.sync_ecs(&mut self.server.ecs);
            self.server.sends.forget(id);
        }
    }
}
