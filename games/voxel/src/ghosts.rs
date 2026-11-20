//! Les joueurs fantômes de `ctl` : des joueurs du serveur sans fenêtre, pour voir et viser les autres
//! joueurs avant d'avoir le réseau.
//!
//! - `ghost [nom] [x y z]` ajoute un joueur immobile (devant le joueur local par défaut) : il est
//!   envoyé au client comme une entité (`EntityKind::Player`), dessiné avec son nom, visé par les
//!   créatures, et il charge le monde autour de lui comme tout joueur ;
//! - `ghost remove [nom]` retire un fantôme (ou tous) ;
//! - `ghost list` les liste.
//!
//! Un fantôme n'est jamais sauvegardé (voir `PlayerState::ghost`) et garde toute sa vie.

use anyhow::{Context as _, Result, bail};
use glam::Vec3;

use crate::VoxelGame;
use crate::attributes::BaseAttributes;
use crate::control::{json_str, vec3};
use crate::gear::Gear;
use crate::inventory::Inventory;
use crate::player::{self, MoveMode};
use crate::players::{PlayerId, PlayerIdentity, PlayerState, random_uuid};

/// Une commande des fantômes, lue par `control.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Add(Option<String>, Option<Vec3>),
    Remove(Option<String>),
    List,
}

/// Lit `ghost` ; `None` si ce n'est pas une commande des fantômes.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    if name != "ghost" {
        return None;
    }
    Some((|| {
        let number = |text: &str| -> Result<f32> {
            text.parse()
                .with_context(|| format!("ghost : « {text} » n'est pas un nombre"))
        };
        match args {
            ["list"] => Ok(Command::List),
            ["remove"] => Ok(Command::Remove(None)),
            ["remove", who] => Ok(Command::Remove(Some((*who).to_string()))),
            [] => Ok(Command::Add(None, None)),
            [x, y, z] if number(x).is_ok() => Ok(Command::Add(
                None,
                Some(Vec3::new(number(x)?, number(y)?, number(z)?)),
            )),
            [who] => Ok(Command::Add(Some((*who).to_string()), None)),
            [who, x, y, z] => Ok(Command::Add(
                Some((*who).to_string()),
                Some(Vec3::new(number(x)?, number(y)?, number(z)?)),
            )),
            _ => bail!("ghost : [nom] [x y z] | remove [nom] | list"),
        }
    })())
}

impl VoxelGame {
    /// Exécute une commande des fantômes ; renvoie la réponse JSON.
    pub(crate) fn ghost_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Add(name, at) => {
                let feet = at.unwrap_or_else(|| {
                    let body = player::body(&self.ecs, self.players.local().entity).position;
                    body + self.camera.forward_flat() * 4.0
                });
                let number = self.players.iter().filter(|p| p.ghost).count() + 1;
                let name = name.unwrap_or_else(|| format!("Fantôme {number}"));
                self.add_ghost(&name, feet);
                Ok(format!(
                    "{{\"ok\":true,\"name\":{},\"pos\":{}}}",
                    json_str(&name),
                    vec3(feet)
                ))
            }
            Command::Remove(who) => {
                let gone: Vec<(PlayerId, String)> = self
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
                    .players
                    .iter()
                    .filter(|p| p.ghost)
                    .map(|p| {
                        let at = player::body(&self.ecs, p.entity).position;
                        format!("{{\"name\":{},\"pos\":{}}}", json_str(&p.name), vec3(at))
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"ghosts\":[{}]}}", rows.join(",")))
            }
        }
    }

    /// Ajoute un joueur serveur sans fenêtre, les pieds en `feet`.
    pub(crate) fn add_ghost(&mut self, name: &str, feet: Vec3) -> PlayerId {
        let eye = feet + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT;
        let entity = player::spawn(&mut self.ecs, eye, MoveMode::Walk);
        let id = self.players.next_id();
        let mut state = PlayerState::new(
            PlayerIdentity {
                id,
                name: name.to_string(),
                uuid: random_uuid(),
            },
            entity,
            Inventory::default(),
            Gear::new(BaseAttributes::default()),
            &self.survival,
            &self.hunger_rules,
        );
        state.ghost = true;
        self.players.add(state);
        self.players.sync_ecs(&mut self.ecs);
        id
    }

    /// Retire un joueur (et son entité) ; le client oublie ce qu'il lui avait envoyé.
    pub(crate) fn remove_ghost(&mut self, id: PlayerId) {
        if let Some(state) = self.players.remove(id) {
            self.ecs.despawn(state.entity);
            self.players.sync_ecs(&mut self.ecs);
            self.view.forget_player(id);
        }
    }

    /// Un fantôme garde toute sa vie : il reste une cible tant qu'on ne le retire pas.
    pub(crate) fn keep_ghosts_alive(&mut self) {
        for ghost in self.players.iter().filter(|p| p.ghost) {
            if let Some(mut health) = self.ecs.get_mut::<crate::components::Health>(ghost.entity) {
                health.current = health.max;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_commands_parse() {
        let at = Some(Vec3::new(1.0, 70.0, -3.5));
        assert_eq!(
            parse("ghost", &[]).unwrap().unwrap(),
            Command::Add(None, None)
        );
        assert_eq!(
            parse("ghost", &["Alice"]).unwrap().unwrap(),
            Command::Add(Some("Alice".into()), None)
        );
        assert_eq!(
            parse("ghost", &["1", "70", "-3.5"]).unwrap().unwrap(),
            Command::Add(None, at)
        );
        assert_eq!(
            parse("ghost", &["Bob", "1", "70", "-3.5"])
                .unwrap()
                .unwrap(),
            Command::Add(Some("Bob".into()), at)
        );
        assert_eq!(
            parse("ghost", &["remove"]).unwrap().unwrap(),
            Command::Remove(None)
        );
        assert_eq!(
            parse("ghost", &["remove", "Bob"]).unwrap().unwrap(),
            Command::Remove(Some("Bob".into()))
        );
        assert!(parse("ghost", &["a", "b"]).unwrap().is_err());
        assert!(parse("ghost", &["Bob", "x", "1", "2"]).unwrap().is_err());
        assert!(parse("autre", &[]).is_none());
    }
}
