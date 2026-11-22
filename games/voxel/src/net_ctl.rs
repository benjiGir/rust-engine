//! Les commandes `ctl` du réseau local : `host [port]` ouvre la partie, `players` liste les joueurs.

use anyhow::{Result, bail};

use crate::VoxelGame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Host(Option<u16>),
    Players,
}

pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    Some(match (name, args) {
        ("host", []) => Ok(Command::Host(None)),
        ("host", [port]) => match port.parse() {
            Ok(port) => Ok(Command::Host(Some(port))),
            Err(_) => Err(anyhow::anyhow!("host : un numéro de port, pas « {port} »")),
        },
        ("host", _) => Err(anyhow::anyhow!("host [port]")),
        ("players", _) => Ok(Command::Players),
        _ => return None,
    })
}

impl VoxelGame {
    pub(crate) fn net_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Host(port) => {
                let port = port.unwrap_or(voxel_sim::net::host::DEFAULT_PORT);
                match self.server.open_lan(port) {
                    Ok(port) => Ok(format!("{{\"ok\":true,\"port\":{port}}}")),
                    Err(err) => bail!("impossible d'écouter : {err}"),
                }
            }
            Command::Players => {
                let list: Vec<String> = self
                    .server
                    .players
                    .iter()
                    .map(|p| {
                        let feet = voxel_sim::player::body(&self.server.ecs, p.entity).position;
                        format!(
                            "{{\"id\":{},\"name\":{},\"ghost\":{},\"pos\":[{:.2},{:.2},{:.2}]}}",
                            p.id.0,
                            voxel_sim::server::json_text::json_str(&p.name),
                            p.ghost,
                            feet.x,
                            feet.y,
                            feet.z
                        )
                    })
                    .collect();
                let port = self.server.net.as_ref().map_or(0, |n| n.port());
                Ok(format!(
                    "{{\"ok\":true,\"port\":{port},\"players\":[{}]}}",
                    list.join(",")
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_commands_parse() {
        assert_eq!(parse("host", &[]).unwrap().unwrap(), Command::Host(None));
        assert_eq!(
            parse("host", &["25601"]).unwrap().unwrap(),
            Command::Host(Some(25601))
        );
        assert!(parse("host", &["x"]).unwrap().is_err());
        assert_eq!(parse("players", &[]).unwrap().unwrap(), Command::Players);
        assert!(parse("ghost", &[]).is_none());
    }
}
