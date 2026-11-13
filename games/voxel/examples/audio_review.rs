//! Écoute de contrôle sans carte son : même décodeur Kira que le jeu, niveau fixe, sans normalisation.
//! `cargo run -p voxel --example audio_review -- <dossier sons> <sfx|ambience|music> <sortie.wav>`
use anyhow::{Context, Result};
use kira::{Frame, sound::static_sound::StaticSoundData};
use std::{fs, path::Path};
const RATE: u32 = 44_100;
fn load(root: &Path, key: &str) -> Result<StaticSoundData> {
    let fallback = match key {
        "step_wood" => "step_hard",
        "place_wood" => "place_hard",
        _ => key,
    };
    for name in [key, fallback] {
        for ext in ["wav", "ogg"] {
            let p = root.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Ok(StaticSoundData::from_file(p)?);
            }
        }
    }
    anyhow::bail!("son absent : {key}")
}
fn mix(out: &mut [Frame], sound: &StaticSoundData, start: f32, seconds: f32, from: f32) {
    let offset = (start * RATE as f32) as usize;
    for i in 0..(seconds * RATE as f32) as usize {
        let at = (from + i as f32 / RATE as f32) * sound.sample_rate as f32;
        let n = at as usize;
        let Some(a) = sound.frames.get(n) else {
            break;
        };
        let b = sound.frames.get(n + 1).unwrap_or(a);
        let Some(dest) = out.get_mut(offset + i) else {
            break;
        };
        let fade = (i as f32 / (RATE as f32 * 0.01)).min(1.0)
            * ((seconds * RATE as f32 - i as f32) / (RATE as f32 * 0.06)).min(1.0);
        let gain = 0.65 * fade;
        dest.left += (a.left + (b.left - a.left) * at.fract()) * gain;
        dest.right += (a.right + (b.right - a.right) * at.fract()) * gain;
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "audio_review <sons> <sfx|ambience|music> <sortie.wav>"
    );
    let root = Path::new(&args[0]);
    let mut out = vec![Frame::ZERO; RATE as usize * 32];
    let duration = match args[1].as_str() {
        "sfx" => {
            for (group, key) in ["step_soft", "step_hard", "step_wood", "step_snow"]
                .into_iter()
                .enumerate()
            {
                for i in 0..4 {
                    let variant = format!("{key}_{i}");
                    let s = load(
                        root,
                        if i > 0 && root.join(format!("{variant}.wav")).is_file() {
                            &variant
                        } else {
                            key
                        },
                    )?;
                    mix(&mut out, &s, group as f32 * 2.0 + i as f32 * 0.43, 0.7, 0.0);
                }
            }
            for (at, key, length) in [
                (8.0, "place_wood", 1.0),
                (9.0, "place_hard", 1.0),
                (10.0, "break_hard", 1.5),
                (12.0, "break_glass", 1.5),
                (14.0, "splash", 1.8),
                (16.0, "gulp", 1.0),
                (18.0, "enchant", 2.5),
            ] {
                mix(&mut out, &load(root, key)?, at, length, 0.0);
            }
            21.0
        }
        "ambience" => {
            for (i, key) in ["ambience_day", "ambience_night", "rain", "waves", "owl"]
                .into_iter()
                .enumerate()
            {
                mix(&mut out, &load(root, key)?, i as f32 * 6.0, 5.5, 0.0);
            }
            30.0
        }
        "music" => {
            for (i, key) in ["music_day1", "music_day2", "music_night", "music_cave"]
                .into_iter()
                .enumerate()
            {
                // Le décodage complet contrôle également les derniers paquets Vorbis.
                mix(&mut out, &load(root, key)?, i as f32 * 8.0, 7.5, 20.0);
            }
            32.0
        }
        _ => anyhow::bail!("mode inconnu"),
    };
    out.truncate((duration * RATE as f32) as usize);
    let peak = out
        .iter()
        .fold(0.0f32, |m, f| m.max(f.left.abs()).max(f.right.abs()));
    anyhow::ensure!(peak < 1.0, "écrêtage dans le contrôle : {peak}");
    let size = (out.len() * 4) as u32;
    let mut bytes = Vec::with_capacity(size as usize + 44);
    bytes.extend(b"RIFF");
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 4).to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for f in out {
        for s in [f.left, f.right] {
            bytes.extend(((s * 32767.0).round() as i16).to_le_bytes());
        }
    }
    let path = Path::new(&args[2]);
    fs::create_dir_all(path.parent().context("dossier de sortie")?)?;
    fs::write(path, bytes)?;
    println!(
        "{} : {duration}s, pic {:.1} dBFS",
        path.display(),
        20.0 * peak.log10()
    );
    Ok(())
}
