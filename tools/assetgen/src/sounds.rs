//! Banque sonore : prises enregistrées et niveaux reproductibles, sans téléchargement.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Deserialize)]
enum Kind {
    Effect,
    Ambient,
    Loop,
    Music,
    Chime,
}
#[derive(Debug, Deserialize)]
struct Recipe {
    name: String,
    source: String,
    kind: Kind,
    #[serde(default)]
    lowpass: Option<f32>,
}
#[derive(Debug, Clone)]
struct Clip {
    rate: u32,
    channels: usize,
    samples: Vec<f32>,
}
fn bank(root: &Path) -> Result<Vec<Recipe>> {
    Ok(ron::from_str(&fs::read_to_string(
        root.join("tools/assetgen/audio/bank.ron"),
    )?)?)
}

/// Les sources sont importées en PCM une fois ; aucun codec externe à la génération.
fn read_wav(path: &Path) -> Result<Clip> {
    let bytes = fs::read(path).with_context(|| format!("source audio {}", path.display()))?;
    ensure!(
        bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
        "WAV invalide : {}",
        path.display()
    );
    let (mut format, mut data, mut cursor) = (None, None, 12usize);
    while cursor + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into()?) as usize;
        let start = cursor + 8;
        let end = start.checked_add(size).context("taille WAV excessive")?;
        ensure!(end <= bytes.len(), "chunk WAV tronqué");
        match &bytes[cursor..cursor + 4] {
            b"fmt " => {
                ensure!(size >= 16, "fmt tronqué");
                let u16_at = |i| u16::from_le_bytes([bytes[start + i], bytes[start + i + 1]]);
                let rate = u32::from_le_bytes(bytes[start + 4..start + 8].try_into()?);
                ensure!(
                    u16_at(0) == 1 && u16_at(14) == 16,
                    "source attendue : PCM 16 bits"
                );
                let channels = u16_at(2) as usize;
                ensure!(
                    (1..=2).contains(&channels) && rate == 44_100,
                    "source attendue : mono/stéréo 44,1 kHz"
                );
                format = Some((rate, channels));
            }
            b"data" => data = Some(&bytes[start..end]),
            _ => {}
        }
        cursor = end + size % 2;
    }
    let (rate, channels) = format.context("fmt absent")?;
    let data = data.context("données absentes")?;
    ensure!(
        !data.is_empty() && data.len() % (channels * 2) == 0,
        "PCM incomplet"
    );
    Ok(Clip {
        rate,
        channels,
        samples: data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect(),
    })
}
fn db(value: f32) -> f32 {
    10.0f32.powf(value / 20.0)
}
fn filter(clip: &mut Clip, lowpass: Option<f32>) {
    let alpha = (-std::f32::consts::TAU * 35.0 / clip.rate as f32).exp();
    let low = lowpass.map(|hz| 1.0 - (-std::f32::consts::TAU * hz / clip.rate as f32).exp());
    for channel in 0..clip.channels {
        let (mut x, mut y, mut smooth) = (clip.samples[channel], 0.0, 0.0);
        for sample in clip.samples.iter_mut().skip(channel).step_by(clip.channels) {
            let input = *sample;
            y = alpha * (y + input - x);
            x = input;
            *sample = if let Some(a) = low {
                smooth += a * (y - smooth);
                smooth
            } else {
                y
            };
        }
    }
}
fn trim(clip: &mut Clip) {
    let peak = clip
        .samples
        .iter()
        .copied()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    let frames = clip.samples.len() / clip.channels;
    let active = |i: usize| {
        clip.samples[i * clip.channels..(i + 1) * clip.channels]
            .iter()
            .any(|s| s.abs() > peak * 0.003)
    };
    let start = (0..frames)
        .find(|&i| active(i))
        .unwrap_or(0)
        .saturating_sub((clip.rate / 200) as usize);
    let end = (0..frames)
        .rfind(|&i| active(i))
        .map_or(frames, |i| (i + (clip.rate / 25) as usize).min(frames));
    clip.samples = clip.samples[start * clip.channels..end * clip.channels].to_vec();
}
/// Puissance constante et traitement identique des canaux : pas de trou à la jointure.
fn loop_crossfade(clip: &mut Clip) {
    let frames = clip.samples.len() / clip.channels;
    let fade = (clip.rate as usize * 3 / 4).min(frames / 4);
    if fade < 2 {
        return;
    }
    let body = frames - fade;
    for frame in 0..fade {
        let angle = frame as f32 / (fade - 1) as f32 * std::f32::consts::FRAC_PI_2;
        for channel in 0..clip.channels {
            let head = frame * clip.channels + channel;
            let tail = (body + frame) * clip.channels + channel;
            clip.samples[head] =
                clip.samples[tail] * angle.cos() + clip.samples[head] * angle.sin();
        }
    }
    clip.samples.truncate(body * clip.channels);
}
fn fades(clip: &mut Clip) {
    let frames = clip.samples.len() / clip.channels;
    let attack = (clip.rate as usize / 500).min(frames / 4).max(1);
    let release = (clip.rate as usize / 30).min(frames / 4).max(1);
    for frame in 0..frames {
        let gain = (frame as f32 / attack as f32).min(1.0)
            * ((frames - 1 - frame) as f32 / release as f32).min(1.0);
        for sample in &mut clip.samples[frame * clip.channels..(frame + 1) * clip.channels] {
            *sample *= gain;
        }
    }
}
/// Motif original monté à partir d'une cloche enregistrée, sans oscillateur continu.
fn chime(source: &Clip) -> Clip {
    let mut out = Clip {
        rate: source.rate,
        channels: source.channels,
        samples: vec![0.0; source.rate as usize * source.channels * 3],
    };
    for (voice, semitones) in [0.0f32, 7.0, 4.0, 12.0].into_iter().enumerate() {
        let ratio = 2.0f32.powf(semitones / 12.0);
        let offset = (voice as f32 * 0.19 * source.rate as f32) as usize;
        for frame in offset..out.samples.len() / out.channels {
            let at = (frame - offset) as f32 * ratio;
            let index = at as usize;
            if (index + 1) * source.channels >= source.samples.len() {
                break;
            }
            for channel in 0..out.channels {
                let a = source.samples[index * source.channels + channel];
                let b = source.samples[(index + 1) * source.channels + channel];
                out.samples[frame * out.channels + channel] += (a + (b - a) * at.fract()) * 0.45;
            }
        }
    }
    out
}
fn prepare(root: &Path, recipe: &Recipe) -> Result<Clip> {
    let mut clip = read_wav(&root.join("tools/assetgen/audio").join(&recipe.source))?;
    filter(&mut clip, recipe.lowpass);
    if matches!(recipe.kind, Kind::Chime) {
        clip = chime(&clip);
    }
    if matches!(recipe.kind, Kind::Loop) {
        loop_crossfade(&mut clip);
    } else {
        trim(&mut clip);
        fades(&mut clip);
    }
    let (rms_db, peak_db) = match recipe.kind {
        Kind::Effect => (-23.0, -6.0),
        Kind::Ambient => (-28.0, -10.0),
        Kind::Loop => (-31.0, -12.0),
        Kind::Chime => (-27.0, -9.0),
        Kind::Music => unreachable!("les morceaux sont des masters OGG versionnés"),
    };
    let rms = (clip
        .samples
        .iter()
        .map(|s| (*s as f64).powi(2))
        .sum::<f64>()
        / clip.samples.len() as f64)
        .sqrt() as f32;
    let peak = clip
        .samples
        .iter()
        .copied()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    ensure!(
        rms > 0.000001 && peak.is_finite(),
        "source silencieuse : {}",
        recipe.name
    );
    // Garder de la marge au mélange, sans amplifier un souffle de plus de 12 dB.
    let gain = (db(rms_db) / rms).min(db(peak_db) / peak).min(4.0);
    for sample in &mut clip.samples {
        *sample *= gain;
    }
    Ok(clip)
}
fn wav_bytes(clip: &Clip) -> Vec<u8> {
    let size = (clip.samples.len() * 2) as u32;
    let channels = clip.channels as u16;
    let mut b = Vec::with_capacity(size as usize + 44);
    b.extend(b"RIFF");
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(clip.rate.to_le_bytes());
    b.extend((clip.rate * channels as u32 * 2).to_le_bytes());
    b.extend((channels * 2).to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for sample in &clip.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        b.extend(value.to_le_bytes());
    }
    b
}
pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join("assets/core/sounds");
    fs::create_dir_all(&dir)?;
    let mut paths = Vec::new();
    for recipe in bank(root)? {
        ensure!(!recipe.name.contains(['/', '\\']), "nom de son invalide");
        let music = matches!(recipe.kind, Kind::Music);
        let path = dir.join(format!(
            "{}.{}",
            recipe.name,
            if music { "ogg" } else { "wav" }
        ));
        if music {
            fs::copy(
                root.join("tools/assetgen/audio").join(&recipe.source),
                &path,
            )?;
        } else {
            fs::write(&path, wav_bytes(&prepare(root, &recipe)?))?;
        }
        paths.push(path);
    }
    let credits = dir.join("CREDITS.md");
    fs::copy(root.join("tools/assetgen/audio/CREDITS.md"), &credits)?;
    paths.push(credits);
    Ok(paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }
    #[test]
    fn masters_are_finite_quiet_enough_and_have_no_abrupt_effect_edges() {
        let root = root();
        for recipe in bank(&root).unwrap() {
            if matches!(recipe.kind, Kind::Music) {
                continue;
            }
            let clip = prepare(&root, &recipe).unwrap();
            assert!(
                clip.samples.len() > clip.rate as usize / 25,
                "{}",
                recipe.name
            );
            let peak = clip
                .samples
                .iter()
                .copied()
                .fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(
                peak > 0.001 && peak <= db(-6.0) + 0.00001,
                "{} : pic {peak}",
                recipe.name
            );
            if !matches!(recipe.kind, Kind::Loop) {
                assert_eq!(clip.samples[0], 0.0, "{}", recipe.name);
                assert_eq!(*clip.samples.last().unwrap(), 0.0, "{}", recipe.name);
            }
        }
    }
    #[test]
    fn loop_joint_is_natural_and_stereo_stays_in_phase() {
        let samples: Vec<f32> = (0..44_100 * 2)
            .flat_map(|i| {
                let s = (i as f32 * std::f32::consts::TAU * 100.0 / 44_100.0).sin() * 0.3;
                [s, s]
            })
            .collect();
        let mut clip = Clip {
            rate: 44_100,
            channels: 2,
            samples,
        };
        loop_crossfade(&mut clip);
        assert!((clip.samples[0] - clip.samples[clip.samples.len() - 2]).abs() < 0.01);
        assert!(clip.samples.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
    }
    #[test]
    fn wav_keeps_rate_channels_and_exact_pcm_length() {
        let clip = Clip {
            rate: 44_100,
            channels: 2,
            samples: vec![0.0, 0.5, -0.5, 0.0],
        };
        let b = wav_bytes(&clip);
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(b.len(), 52);
        assert_eq!(&b[22..24], &2u16.to_le_bytes());
        assert_eq!(&b[24..28], &44_100u32.to_le_bytes());
    }
}
