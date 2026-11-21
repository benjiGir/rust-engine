//! `voxel --bots N adresse [idle|walk|explore]` : N bots rejoignent une partie et y vivent.
//!
//! Pour mesurer un serveur (dédié ou une fenêtre ouverte au réseau local) : les bots marchent,
//! reçoivent le monde, et toutes les 5 s le terminal montre combien sont connectés, ce qu'ils ont
//! reçu (octets, chunks) et le temps moyen d'arrivée. Le serveur dédié, lui, écrit ses ticks.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::bot::{Behaviour, Bot};
use crate::launch::Launch;
use crate::server_side::VoxelServer;

const STEP: f32 = 1.0 / 60.0;

/// Lit `--bots N adresse [comportement]` dans les arguments.
pub fn parse_args(args: &[String]) -> Result<(usize, String, Behaviour)> {
    let at = args
        .iter()
        .position(|a| a == "--bots")
        .context("--bots manquant")?;
    let count: usize = args
        .get(at + 1)
        .context("--bots N adresse [idle|walk|explore]")?
        .parse()
        .context("--bots : un nombre de bots")?;
    let address = args
        .get(at + 2)
        .context("--bots N adresse : l'adresse manque")?
        .clone();
    let address = if address.contains(':') {
        address
    } else {
        format!("{address}:{}", crate::net_host::DEFAULT_PORT)
    };
    let behaviour = match args.get(at + 3) {
        Some(name) => {
            Behaviour::parse(name).with_context(|| format!("comportement inconnu « {name} »"))?
        }
        None => Behaviour::Walk,
    };
    if count == 0 || count > 256 {
        bail!("--bots : de 1 à 256 bots");
    }
    Ok((count, address, behaviour))
}

pub fn run(args: &[String]) -> Result<()> {
    let (count, address, behaviour) = parse_args(args)?;
    // Le contenu (blocs, objets) pour l'empreinte et pour relire le monde : une coquille de serveur.
    let settings = crate::settings::Settings::default();
    let shell = Launch {
        world: None,
        seed: Some(1),
        title: false,
        difficulty: None,
        dimension: None,
        join: None,
    };
    let shell = VoxelServer::boot(&shell, &settings, STEP, false, false)?.server;
    let content = crate::net_host::content_fingerprint(&shell.registry, &shell.content.items);
    let registry = shell.registry.clone();
    drop(shell);
    let mut bots = Vec::new();
    let mut join_times = Vec::new();
    for i in 0..count {
        let start = Instant::now();
        // Un identifiant par numéro : un bot qui revient retrouve son inventaire.
        let uuid = 0xb07_0000 + i as u128;
        match Bot::join(
            &address,
            &format!("Bot{i}"),
            uuid,
            content,
            registry.clone(),
        ) {
            Ok(bot) => {
                join_times.push(start.elapsed());
                bots.push(bot);
            }
            Err(err) => println!("Bot{i} : {err:#}"),
        }
    }
    let mean_join = join_times.iter().sum::<Duration>() / join_times.len().max(1) as u32;
    println!(
        "{} bot(s) dans la partie ({mean_join:.0?} en moyenne pour être accueilli)",
        bots.len()
    );
    let mut report = Instant::now();
    let mut last = Instant::now();
    let mut lag = 0.0f32;
    let mut wire_before = 0;
    while !bots.is_empty() {
        let now = Instant::now();
        lag += (now - last).as_secs_f32().min(0.25);
        last = now;
        bots.retain_mut(|bot| match bot.poll() {
            Ok(()) => true,
            Err(err) => {
                println!("{err:#}");
                false
            }
        });
        while lag >= STEP {
            for bot in &mut bots {
                bot.act(behaviour, STEP);
            }
            lag -= STEP;
        }
        if report.elapsed() >= Duration::from_secs(5) {
            let seconds = report.elapsed().as_secs_f32();
            report = Instant::now();
            let bytes: usize = bots
                .iter_mut()
                .map(|b| std::mem::take(&mut b.received_bytes))
                .sum();
            let chunks: usize = bots
                .iter_mut()
                .map(|b| std::mem::take(&mut b.chunks_received))
                .sum();
            let wire: usize = bots.iter().map(Bot::wire_bytes).sum();
            let per_bot = bots.len().max(1) as f32;
            println!(
                "{} bot(s) : {:.1} Kio/s reçus par bot ({:.1} Kio/s décompressés), {:.1} chunks/s par bot, \
                 {} octets par chunk décompressé ; {:.1} Mio reçus en tout",
                bots.len(),
                wire.saturating_sub(wire_before) as f32 / 1024.0 / seconds / per_bot,
                bytes as f32 / 1024.0 / seconds / per_bot,
                chunks as f32 / seconds / per_bot,
                bytes / chunks.max(1),
                wire as f32 / (1024.0 * 1024.0),
            );
            wire_before = wire;
        }
        if let Some(rest) = Duration::from_secs_f32(STEP).checked_sub(now.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn bot_arguments_parse() {
        let (n, address, behaviour) = parse_args(&args("voxel --bots 4 127.0.0.1")).unwrap();
        assert_eq!(
            (n, address.as_str(), behaviour),
            (4, "127.0.0.1:25600", Behaviour::Walk)
        );
        let (_, address, behaviour) =
            parse_args(&args("voxel --bots 2 hote:25610 explore")).unwrap();
        assert_eq!(
            (address.as_str(), behaviour),
            ("hote:25610", Behaviour::Explore)
        );
        assert!(parse_args(&args("voxel --bots 0 x")).is_err());
        assert!(parse_args(&args("voxel --bots 2 x danse")).is_err());
    }
}
