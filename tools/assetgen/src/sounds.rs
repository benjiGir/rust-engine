//! Des sons synthétisés : du bruit filtré et des enveloppes, écrits en WAV.
//!
//! Un son, c'est une suite d'échantillons (ici 44 100 par seconde) entre -1 et 1.
//! - Le **bruit blanc** (des valeurs au hasard) sonne comme un « chhh ».
//! - Un **filtre passe-bas** garde les graves : bruit sourd, pas, souffle du vent.
//! - Une **enveloppe** règle le volume dans le temps : attaque rapide puis
//!   décroissance exponentielle pour un choc, montée et descente lentes pour une ambiance.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Rng;

const RATE: u32 = 44_100;

/// Du bruit filtré par un passe-bas à un pôle. `cutoff` entre 0 (très sourd) et 1 (brut).
fn filtered_noise(samples: usize, cutoff: f32, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut state = 0.0;
    (0..samples)
        .map(|_| {
            state += cutoff * (rng.signed() - state);
            state
        })
        .collect()
}

/// Un choc : attaque de `attack` secondes, puis décroissance de constante `decay`.
fn hit_envelope(i: usize, attack: f32, decay: f32) -> f32 {
    let t = i as f32 / RATE as f32;
    if t < attack {
        t / attack
    } else {
        (-(t - attack) / decay).exp()
    }
}

/// Ramène le maximum à `peak` (évite la saturation, uniformise les volumes).
fn normalize(mut samples: Vec<f32>, peak: f32) -> Vec<f32> {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 0.0 {
        for s in &mut samples {
            *s *= peak / max;
        }
    }
    samples
}

fn seconds(s: f32) -> usize {
    (s * RATE as f32) as usize
}

/// Un pas ou un bloc qui casse : du bruit filtré dans une enveloppe de choc.
fn noise_hit(duration: f32, cutoff: f32, decay: f32, peak: f32, seed: u64) -> Vec<f32> {
    let noise = filtered_noise(seconds(duration), cutoff, seed);
    let shaped = noise
        .iter()
        .enumerate()
        .map(|(i, n)| n * hit_envelope(i, 0.004, decay))
        .collect();
    normalize(shaped, peak)
}

/// Un « toc » sourd pour poser un bloc : une sinusoïde dont la fréquence chute, plus un peu de bruit.
fn thump(start_hz: f32, end_hz: f32, duration: f32, seed: u64) -> Vec<f32> {
    let n = seconds(duration);
    let noise = filtered_noise(n, 0.3, seed);
    let mut phase = 0.0f32;
    let samples = (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let hz = start_hz + (end_hz - start_hz) * t;
            phase += hz / RATE as f32 * std::f32::consts::TAU;
            (phase.sin() + 0.3 * noise[i]) * hit_envelope(i, 0.002, duration / 5.0)
        })
        .collect();
    normalize(samples, 0.8)
}

/// Le verre qui casse : des tintements aigus aléatoires sur un bruit clair.
fn glass_break(seed: u64) -> Vec<f32> {
    let n = seconds(0.45);
    let mut samples = noise_hit(0.45, 0.9, 0.06, 0.5, seed);
    let mut rng = Rng::new(seed + 1);
    for _ in 0..6 {
        let start = (rng.next_f32() * n as f32 * 0.5) as usize;
        let hz = 2500.0 + rng.next_f32() * 3000.0;
        for (k, s) in samples[start..].iter_mut().enumerate() {
            let t = k as f32 / RATE as f32;
            *s += (t * hz * std::f32::consts::TAU).sin() * (-t / 0.05).exp() * 0.3;
        }
    }
    normalize(samples, 0.8)
}

/// L'éclaboussure : un bruit clair qui s'éteint lentement, avec quelques gouttes.
fn splash(seed: u64) -> Vec<f32> {
    let n = seconds(0.7);
    let mut samples = noise_hit(0.7, 0.5, 0.15, 0.6, seed);
    let mut rng = Rng::new(seed + 1);
    for _ in 0..8 {
        let start = (rng.next_f32() * n as f32 * 0.8) as usize;
        let hz = 600.0 + rng.next_f32() * 900.0;
        for (k, s) in samples[start..].iter_mut().enumerate() {
            let t = k as f32 / RATE as f32;
            // Une goutte : une fréquence qui monte vite, comme un « bloup ».
            let f = hz * (1.0 + t * 8.0);
            *s += (t * f * std::f32::consts::TAU).sin() * (-t / 0.03).exp() * 0.25;
        }
    }
    normalize(samples, 0.8)
}

/// La pluie : un bruit clair et continu (huit secondes qui bouclent sans à-coup).
fn rain(duration: f32, seed: u64) -> Vec<f32> {
    let n = seconds(duration);
    let hiss = filtered_noise(n, 0.5, seed);
    let body = filtered_noise(n, 0.12, seed + 1);
    let samples = hiss
        .iter()
        .zip(&body)
        .map(|(h, b)| h * 0.7 + b * 0.5)
        .collect();
    normalize(samples, 0.6)
}

/// Le tonnerre : un claquement sec, puis un grondement grave qui roule et s'éteint.
fn thunder(seed: u64) -> Vec<f32> {
    let n = seconds(4.0);
    let crack = filtered_noise(n, 0.4, seed);
    let rumble = filtered_noise(n, 0.012, seed + 1);
    let samples = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let roll = 0.6 + 0.4 * (t * 7.0).sin().abs();
            crack[i] * hit_envelope(i, 0.002, 0.06) * 0.6
                + rumble[i] * 6.0 * hit_envelope(i, 0.15, 1.3) * roll
        })
        .collect();
    normalize(samples, 0.9)
}

/// Le vent : du bruit très filtré dont le volume ondule. Le nombre de vagues est entier
/// sur la durée : le son peut boucler sans à-coup de volume.
fn wind(duration: f32, seed: u64) -> Vec<f32> {
    let n = seconds(duration);
    let noise = filtered_noise(n, 0.02, seed);
    let samples = noise
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            s * (0.6 + 0.25 * (t * 2.0).sin() + 0.15 * (t * 5.0).sin())
        })
        .collect();
    crossfade_loop(normalize(samples, 0.5))
}

/// La nuit : un vent plus faible et des grillons (des salves de bips aigus).
fn night(duration: f32, seed: u64) -> Vec<f32> {
    let n = seconds(duration);
    let mut samples: Vec<f32> = wind(duration, seed).iter().map(|s| s * 0.4).collect();
    let mut rng = Rng::new(seed + 1);
    let mut start = 0;
    while start < n {
        let hz = 4200.0 + rng.next_f32() * 600.0;
        // Une salve de 3 ou 4 « cri » de 30 ms.
        for chirp in 0..(3 + rng.next_u64() % 2) as usize {
            let begin = start + chirp * seconds(0.06);
            for k in 0..seconds(0.03) {
                if let Some(s) = samples.get_mut(begin + k) {
                    let t = k as f32 / RATE as f32;
                    let env = (std::f32::consts::PI * k as f32 / seconds(0.03) as f32).sin();
                    *s += (t * hz * std::f32::consts::TAU).sin() * env * 0.12;
                }
            }
        }
        start += seconds(0.7 + rng.next_f32() * 1.2);
    }
    crossfade_loop(normalize(samples, 0.5))
}

/// Fond la fin du son dans son début : en boucle, on n'entend pas la jointure.
fn crossfade_loop(samples: Vec<f32>) -> Vec<f32> {
    let fade = seconds(0.5).min(samples.len() / 2);
    let body = samples.len() - fade;
    let mut out = samples[..body].to_vec();
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        out[i] = out[i] * t + samples[body + i] * (1.0 - t);
    }
    out
}

/// Écrit un fichier WAV mono 16 bits. Le format est simple : un en-tête de 44 octets
/// (« RIFF », « WAVE », le format, la taille), puis les échantillons.
fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let mut file = BufWriter::new(File::create(path)?);
    let data_len = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // taille du bloc « fmt »
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&RATE.to_le_bytes())?;
    file.write_all(&(RATE * 2).to_le_bytes())?; // octets par seconde
    file.write_all(&2u16.to_le_bytes())?; // octets par échantillon
    file.write_all(&16u16.to_le_bytes())?; // bits par échantillon
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        let value = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        file.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

/// Écrit tous les sons ; renvoie les chemins.
pub fn generate(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join("assets/core/sounds");
    let sounds: Vec<(&str, Vec<f32>)> = vec![
        ("step_soft", noise_hit(0.12, 0.08, 0.03, 0.5, 1)),
        ("step_hard", noise_hit(0.09, 0.35, 0.02, 0.5, 2)),
        ("step_sand", noise_hit(0.16, 0.5, 0.04, 0.4, 3)),
        ("step_snow", noise_hit(0.18, 0.25, 0.05, 0.4, 4)),
        ("break_soft", noise_hit(0.3, 0.12, 0.08, 0.8, 5)),
        ("break_hard", noise_hit(0.35, 0.45, 0.07, 0.8, 6)),
        ("break_glass", glass_break(7)),
        ("place_soft", thump(160.0, 80.0, 0.15, 8)),
        ("place_hard", thump(320.0, 140.0, 0.12, 9)),
        ("splash", splash(10)),
        ("rain", rain(8.0, 13)),
        ("thunder", thunder(15)),
        ("ambience_day", wind(8.0, 11)),
        ("ambience_night", night(8.0, 12)),
    ];
    let mut paths = Vec::new();
    for (name, samples) in sounds {
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, &samples)?;
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_normalized_and_not_silent() {
        for samples in [
            noise_hit(0.1, 0.2, 0.03, 0.5, 1),
            thump(200.0, 80.0, 0.1, 2),
            splash(3),
        ] {
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.3 && peak <= 1.0, "pic {peak}");
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let path = std::env::temp_dir().join(format!("assetgen-{}.wav", std::process::id()));
        write_wav(&path, &[0.0, 0.5, -0.5]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 3 * 2);
    }
}
