//! Le réseau : le protocole, l'hôte et l'invité, ce que la fenêtre reçoit du serveur, les bots.

pub mod bot;
pub mod bot_swarm;
pub mod dedicated;
pub mod entities;
pub mod ghosts;
pub mod host;
pub mod join;
pub mod local_link;
pub mod player_view;
pub mod predicted;
pub mod protocol;
pub mod sky_view;
#[cfg(test)]
mod tests;
pub mod wire;
