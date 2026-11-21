//! Le multijoueur depuis `xtask` : `bots` lance des bots sur une partie, `bench-net` mesure un serveur
//! dédié avec 1, 4 puis 16 bots.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Compile en release les deux binaires (le jeu pour les bots, le serveur dédié).
fn build(root: &Path) -> Result<()> {
    let status = Command::new(cargo())
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--quiet",
            "-p",
            "voxel",
            "-p",
            "voxel_sim",
        ])
        .status()?;
    if !status.success() {
        bail!("la compilation a échoué");
    }
    Ok(())
}

/// `cargo xtask bots N adresse [idle|walk|explore]`.
pub fn bots(root: &Path, args: &[String]) -> Result<()> {
    if args.len() < 2 {
        bail!("usage : cargo xtask bots N adresse [idle|walk|explore]");
    }
    build(root)?;
    let status = Command::new(root.join("target/release/voxel"))
        .current_dir(root)
        .arg("--bots")
        .args(args)
        .status()?;
    if !status.success() {
        bail!("les bots se sont arrêtés en erreur");
    }
    Ok(())
}

/// Lance `program` et rend ses lignes de sortie au fil de l'eau.
fn spawn_lines(mut command: Command) -> Result<(Child, std::sync::mpsc::Receiver<String>)> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().context("pas de sortie")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Ok((child, rx))
}

/// `cargo xtask bench-net [secondes]` : un serveur dédié sur un monde jetable, et 1, 4, 16 bots
/// qui explorent pendant `secondes` (20 par défaut) chacun.
pub fn bench(root: &Path, args: &[String]) -> Result<()> {
    let seconds: u64 = args
        .first()
        .map_or(Ok(20), |s| s.parse())
        .context("bench-net [secondes]")?;
    build(root)?;
    let dir = std::env::temp_dir().join(format!("voxel-bench-net-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let port = 25_630;
    std::fs::write(
        dir.join("server.ron"),
        format!(
            "(name: \"bench-net\", world: \"bench\", seed: Some(3), port: {port}, max_players: 64)\n"
        ),
    )?;
    let mut server_cmd = Command::new(root.join("target/release/voxel-server"));
    server_cmd
        .current_dir(root)
        .env("VOXEL_SAVES", &dir)
        .env("VOXEL_SERVER_CONFIG", dir.join("server.ron"))
        .stdin(Stdio::piped());
    let (mut server, server_lines) = spawn_lines(server_cmd)?;
    let started = Instant::now();
    loop {
        match server_lines.recv_timeout(Duration::from_secs(60)) {
            Ok(line) if line.contains("ouvert sur le port") => break,
            Ok(_) => {}
            Err(_) => bail!("le serveur ne démarre pas"),
        }
    }
    println!("serveur prêt en {:.1?}", started.elapsed());
    for count in [1, 4, 16] {
        println!("--- {count} bot(s), {seconds} s ---");
        let mut bots_cmd = Command::new(root.join("target/release/voxel"));
        bots_cmd.current_dir(root).args([
            "--bots",
            &count.to_string(),
            &format!("127.0.0.1:{port}"),
            "explore",
        ]);
        let (mut bots, bot_lines) = spawn_lines(bots_cmd)?;
        let end = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < end {
            while let Ok(line) = bot_lines.try_recv() {
                println!("  bots    : {line}");
            }
            while let Ok(line) = server_lines.try_recv() {
                if line.contains("ticks/s") {
                    println!("  serveur : {line}");
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = bots.kill();
        let _ = bots.wait();
        // Le temps que le serveur voie partir les bots.
        std::thread::sleep(Duration::from_secs(2));
    }
    if let Some(stdin) = server.stdin.as_mut() {
        let _ = writeln!(stdin, "stop");
    }
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
