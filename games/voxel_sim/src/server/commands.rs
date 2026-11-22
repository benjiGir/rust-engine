//! Les commandes de console exécutées par le serveur lui-même, pour le joueur qui les tape.
//!
//! La fenêtre de l'hôte passe encore par son pilotage (`ctl`, voir `console.rs`). Un joueur venu du
//! réseau, lui, n'a que le serveur : ses commandes passent ici. `help` et `mods` sont à tous ; le
//! reste (objets, heure, temps, téléportation, expérience, créatures, blocs, commandes des mods)
//! demande d'être opérateur (`operators`, réglé par `server.ron`). Changer de dimension recharge le
//! monde de tous : c'est l'affaire de l'hôte.

use anyhow::{Context as _, Result, bail};
use glam::{IVec3, Vec3};

use crate::server::VoxelServer;
use crate::world::weather::Sky;

/// Une clé sans espace de noms est un objet, un bloc ou une créature du jeu de base.
fn qualified(key: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("core:{key}")
    }
}

fn number(args: &[&str], i: usize, what: &str) -> Result<f32> {
    args.get(i)
        .with_context(|| format!("{what} manquant"))?
        .parse()
        .with_context(|| format!("{what} : un nombre"))
}

fn position(args: &[&str], from: usize) -> Result<Vec3> {
    Ok(Vec3::new(
        number(args, from, "x")?,
        number(args, from + 1, "y")?,
        number(args, from + 2, "z")?,
    ))
}

fn block_pos(args: &[&str], from: usize) -> Result<IVec3> {
    Ok(position(args, from)?.floor().as_ivec3())
}

impl VoxelServer {
    /// Le joueur courant peut-il taper des commandes de jeu ?
    pub fn may_command(&self) -> bool {
        let me = self.players.me();
        self.players.current_is_local() || self.guests_command || self.operators.contains(&me.name)
    }

    /// Exécute une ligne pour le joueur courant ; la réponse est le texte à lui montrer.
    pub fn run_server_command(&mut self, line: &str) -> String {
        let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let name = name.to_lowercase();
        let args: Vec<&str> = rest.split_whitespace().collect();
        match name.as_str() {
            "help" => return self.console_help(),
            "mods" => return self.console_mods(),
            _ => {}
        }
        if !self.may_command() {
            return "Commande réservée aux opérateurs".to_string();
        }
        match self.game_command(&name, &args, rest.trim()) {
            Ok(text) => text,
            Err(err) => format!("Erreur : {err:#}"),
        }
    }

    fn game_command(&mut self, name: &str, args: &[&str], rest: &str) -> Result<String> {
        let entity = self.players.entity();
        let feet = crate::player::body(&self.ecs, entity).position;
        match name {
            "give" => {
                let key = qualified(args.first().context("give <objet> [nombre]")?);
                let count = args.get(1).map_or(Ok(1.0), |_| number(args, 1, "nombre"))?;
                let item = self
                    .content
                    .items
                    .id(&key)
                    .with_context(|| format!("objet inconnu « {key} »"))?;
                let items = self.content.items.clone();
                let left = self
                    .players
                    .me_mut()
                    .inventory
                    .add(item, count.max(1.0) as u32, &items);
                Ok(if left == 0 {
                    format!("{key} donné")
                } else {
                    format!("{key} donné ({left} ne tenaient pas)")
                })
            }
            "time" => {
                let time = number(args, 0, "l'heure (0 à 1)")?;
                self.day.time = time.rem_euclid(1.0);
                Ok("heure changée".into())
            }
            "weather" => {
                let sky = match args.first().copied() {
                    Some("clear") => Sky::Clear,
                    Some("rain") => Sky::Rain,
                    Some("storm") => Sky::Storm,
                    _ => bail!("weather <clear|rain|storm>"),
                };
                self.weather.set(sky);
                Ok("temps changé".into())
            }
            "tp" => {
                let to = position(args, 0)?;
                crate::player::set_eye(
                    &self.ecs,
                    entity,
                    to + Vec3::Y * engine_voxel::PLAYER_EYE_HEIGHT,
                );
                // Sa fenêtre est replacée (voir `predicted.rs`).
                let id = self.players.current_id();
                self.send_correct(id, false);
                self.forget_last_move();
                Ok(format!("téléporté en {:.1} {:.1} {:.1}", to.x, to.y, to.z))
            }
            "xp" => {
                let points = number(args, 0, "points")?.max(0.0) as u32;
                self.grant_xp(points);
                Ok(format!("{points} points d'expérience"))
            }
            "summon" => {
                let key = qualified(args.first().context("summon <créature>")?);
                let kind = self
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .id(&key)
                    .with_context(|| format!("créature inconnue « {key} »"))?;
                let look = self.players.me().look;
                let at = feet + Vec3::new(look.x, 0.0, look.z).normalize_or_zero() * 3.0;
                crate::mobs::spawn_mob(&mut self.ecs, kind, at).context("impossible ici")?;
                Ok(format!("{key} est apparu"))
            }
            "break" => {
                let pos = block_pos(args, 0)?;
                if self.edit(pos, engine_voxel::BlockId::AIR) {
                    Ok("bloc cassé".into())
                } else {
                    bail!("rien à casser en {pos}")
                }
            }
            "place" => {
                let pos = block_pos(args, 0)?;
                let key = qualified(args.get(3).context("place <x> <y> <z> <bloc>")?);
                let block = self
                    .registry
                    .id(&key)
                    .with_context(|| format!("bloc inconnu « {key} »"))?;
                if self.edit(pos, block) {
                    Ok(format!("{key} posé"))
                } else {
                    bail!("impossible de poser en {pos}")
                }
            }
            "travel" => bail!("changer de dimension est réservé à l'hôte"),
            _ => self.mod_command(name, rest),
        }
    }

    /// Une commande de mod : `nom`, ou `mod:nom` pour lever une ambiguïté.
    fn mod_command(&mut self, name: &str, args: &str) -> Result<String> {
        let (wanted_mod, wanted_name) = match name.split_once(':') {
            Some((m, n)) => (Some(m), n),
            None => (None, name),
        };
        let found = self
            .mods
            .commands()
            .into_iter()
            .find(|(m, n, _)| n == wanted_name && wanted_mod.is_none_or(|w| w == m));
        let Some((mod_id, command, _)) = found else {
            bail!("commande inconnue : /{name} (voir /help)");
        };
        let (reply, actions) = self.mods.command(&mod_id, &command, args);
        self.apply_machine_actions(actions);
        Ok(reply.unwrap_or_else(|| "fait".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::server::boot::tests::headless;

    #[test]
    fn a_guest_commands_only_as_an_operator() {
        let mut server = headless();
        let guest = server.add_remote_player("Bob", 77);
        server.players.set_acting(Some(guest));
        assert!(server.run_server_command("help").contains("/give"));
        assert_eq!(
            server.run_server_command("time 0.5"),
            "Commande réservée aux opérateurs"
        );
        server.guests_command = true;
        assert_eq!(server.run_server_command("time 0.3"), "heure changée");
        server.guests_command = false;
        server.operators.push("Bob".into());
        assert_eq!(server.run_server_command("time 0.25"), "heure changée");
        assert_eq!(server.day.time, 0.25);
        assert_eq!(server.run_server_command("give dirt 3"), "core:dirt donné");
        let dirt = server.content.items.id("core:dirt").unwrap();
        assert!(
            server
                .players
                .me()
                .inventory
                .slots
                .iter()
                .flatten()
                .any(|s| s.item == dirt)
        );
        assert!(
            server
                .run_server_command("travel nether")
                .starts_with("Erreur")
        );
        assert!(server.run_server_command("inconnue").starts_with("Erreur"));
    }
}
