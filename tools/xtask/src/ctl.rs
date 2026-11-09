//! `cargo xtask ctl <commande>...` : envoie des commandes au jeu lancé avec
//! `VOXEL_CONTROL` (voir `games/voxel/src/control.rs`) et affiche ses réponses.
//!
//! Chaque argument est une commande : `cargo xtask ctl "goto 0 80 0" "wait 2" state`.
//! Sans argument, les commandes sont lues sur l'entrée standard, une par ligne.
//! Le port : `VOXEL_CONTROL` s'il est défini, sinon 7878.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

const DEFAULT_PORT: u16 = 7878;

pub fn run(commands: Vec<String>) -> Result<()> {
    let port = match std::env::var("VOXEL_CONTROL")
        .ok()
        .map(|v| v.trim().parse::<u16>())
    {
        Some(Ok(1)) | None => DEFAULT_PORT,
        Some(Ok(port)) => port,
        Some(Err(_)) => bail!("VOXEL_CONTROL doit être un numéro de port"),
    };
    let stream = TcpStream::connect(("127.0.0.1", port)).with_context(|| {
        format!("pas de jeu sur le port {port} : le lancer avec VOXEL_CONTROL={port}")
    })?;
    // Un `wait` dure au plus 120 s : au-delà, le jeu ne répond plus.
    stream.set_read_timeout(Some(Duration::from_secs(150)))?;
    let mut writer = stream.try_clone()?;
    let mut replies = BufReader::new(stream).lines();
    let mut send = |command: &str| -> Result<bool> {
        writeln!(writer, "{command}")?;
        let reply = replies
            .next()
            .context("le jeu a fermé la connexion")?
            .context("pas de réponse du jeu")?;
        println!("{reply}");
        Ok(reply.contains("\"ok\":true"))
    };
    let mut failed = 0;
    if commands.is_empty() {
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            if !line.trim().is_empty() && !send(line.trim())? {
                failed += 1;
            }
        }
    } else {
        for command in &commands {
            if !send(command)? {
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} commande(s) refusée(s)");
    }
    Ok(())
}
