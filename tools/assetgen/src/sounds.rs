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
/// La musique est écrite à cette fréquence plus basse (elle n'a pas d'aigus à garder).
const MUSIC_RATE: u32 = 11_025;

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

/// La fréquence d'un demi-ton `n` au-dessus de `root`.
fn note(root: f32, n: f32) -> f32 {
    root * 2.0f32.powf(n / 12.0)
}

/// Une note pincée : un son doux, quelques harmoniques, qui s'éteint.
fn pluck(out: &mut [f32], rate: u32, start: usize, hz: f32, length: f32, volume: f32, bright: f32) {
    let n = (length * rate as f32) as usize;
    for k in 0..n {
        let Some(s) = out.get_mut(start + k) else {
            break;
        };
        let t = k as f32 / rate as f32;
        let attack = (t / 0.008).min(1.0);
        let env = attack * (-t / (length * 0.45)).exp();
        let phase = t * hz * std::f32::consts::TAU;
        *s += (phase.sin()
            + 0.35 * bright * (2.0 * phase).sin()
            + 0.12 * bright * (3.0 * phase).sin())
            * env
            * volume;
    }
}

/// Un accord tenu : des sinus lents, avec une montée et une descente douces.
fn pad(out: &mut [f32], rate: u32, start: usize, hz: f32, length: f32, volume: f32) {
    let n = (length * rate as f32) as usize;
    for k in 0..n {
        let Some(s) = out.get_mut(start + k) else {
            break;
        };
        let t = k as f32 / rate as f32;
        let env = (t / 1.2).min(1.0) * ((length - t) / 1.5).clamp(0.0, 1.0);
        let phase = t * hz * std::f32::consts::TAU;
        *s += (phase.sin() + 0.2 * (2.0 * phase + 0.4).sin()) * env * volume;
    }
}

/// Un morceau : une suite d'accords tenus et une mélodie qui se promène sur une gamme,
/// avec un peu d'écho. Pas de répétition exacte : le hasard (la graine) choisit les notes.
fn music(
    seconds_total: f32,
    seed: u64,
    root: f32,
    scale: &[f32],
    bpm: f32,
    rest: f32,
    bright: f32,
) -> Vec<f32> {
    let rate = MUSIC_RATE;
    let n = (seconds_total * rate as f32) as usize;
    let mut out = vec![0.0f32; n];
    let beat = 60.0 / bpm;
    let mut rng = Rng::new(seed);
    // Les accords : quatre degrés de la gamme, chacun tenu huit temps.
    let degrees = [0usize, 3 % scale.len(), 4 % scale.len(), 2 % scale.len()];
    let mut t = 0.0;
    let mut chord = 0;
    while t < seconds_total {
        let d = degrees[chord % degrees.len()];
        let base = scale[d];
        for (i, interval) in [0.0, 7.0, 12.0].iter().enumerate() {
            let hz = note(root / 2.0, base + interval);
            pad(
                &mut out,
                rate,
                (t * rate as f32) as usize,
                hz,
                8.0 * beat + 1.0,
                0.09 - 0.015 * i as f32,
            );
        }
        t += 8.0 * beat;
        chord += 1;
    }
    // La mélodie : une marche aléatoire sur la gamme, deux octaves.
    let mut degree = (scale.len() * 2) as i32 / 2;
    let steps = scale.len() as i32 * 2;
    let mut t = beat * 2.0;
    while t < seconds_total - 3.0 {
        let length = if rng.next_f32() < 0.5 {
            beat
        } else {
            beat * 2.0
        };
        if rng.next_f32() >= rest {
            let octave = degree.div_euclid(scale.len() as i32);
            let step = degree.rem_euclid(scale.len() as i32) as usize;
            let semitones = scale[step] + 12.0 * octave as f32;
            pluck(
                &mut out,
                rate,
                (t * rate as f32) as usize,
                note(root, semitones),
                length * 2.2,
                0.2,
                bright,
            );
        }
        degree = (degree + (rng.next_u64() % 5) as i32 - 2).clamp(0, steps - 1);
        t += length;
    }
    // L'écho : deux copies décalées et affaiblies.
    for (delay, gain) in [(beat * 0.75, 0.32f32), (beat * 1.5, 0.18)] {
        let shift = (delay * rate as f32) as usize;
        for i in (shift..n).rev() {
            out[i] += out[i - shift] * gain;
        }
    }
    // Un fondu au début et à la fin.
    let fade = (1.5 * rate as f32) as usize;
    for i in 0..fade.min(n / 2) {
        let g = i as f32 / fade as f32;
        out[i] *= g;
        out[n - 1 - i] *= g;
    }
    normalize(out, 0.55)
}

/// Une salve de cris d'oiseau : des glissandos courts et aigus.
fn birds(seed: u64) -> Vec<f32> {
    let n = seconds(3.2);
    let mut samples = vec![0.0f32; n];
    let mut rng = Rng::new(seed);
    let mut start = seconds(0.1);
    while start + seconds(0.3) < n {
        let base = 2400.0 + rng.next_f32() * 1800.0;
        let slope = (rng.next_f32() - 0.4) * 3000.0;
        for chirp in 0..(2 + (rng.next_u64() % 3) as usize) {
            let begin = start + chirp * seconds(0.11);
            for k in 0..seconds(0.07) {
                let Some(s) = samples.get_mut(begin + k) else {
                    break;
                };
                let t = k as f32 / RATE as f32;
                let env = (std::f32::consts::PI * k as f32 / seconds(0.07) as f32).sin();
                let hz = base + slope * t * 10.0;
                *s += (t * hz * std::f32::consts::TAU).sin() * env * 0.25;
            }
        }
        start += seconds(0.5 + rng.next_f32() * 0.7);
    }
    normalize(samples, 0.5)
}

/// Une goutte dans une grotte : un « ploc » clair suivi d'un petit écho.
fn drip(seed: u64) -> Vec<f32> {
    let n = seconds(1.2);
    let mut samples = vec![0.0f32; n];
    let mut rng = Rng::new(seed);
    let hz = 900.0 + rng.next_f32() * 500.0;
    for (delay, gain) in [(0.0, 1.0), (0.22, 0.35), (0.45, 0.15)] {
        let start = seconds(delay);
        for k in 0..seconds(0.25) {
            let Some(s) = samples.get_mut(start + k) else {
                break;
            };
            let t = k as f32 / RATE as f32;
            let f = hz * (1.0 + t * 6.0);
            *s += (t * f * std::f32::consts::TAU).sin() * (-t / 0.035).exp() * gain;
        }
    }
    normalize(samples, 0.6)
}

/// Un grondement lointain dans la roche.
fn cave_rumble(seed: u64) -> Vec<f32> {
    let n = seconds(4.5);
    let noise = filtered_noise(n, 0.006, seed);
    let samples = (0..n)
        .map(|i| noise[i] * hit_envelope(i, 1.2, 1.6))
        .collect();
    normalize(samples, 0.7)
}

/// Des vagues : un bruit qui gonfle et retombe, deux fois.
fn waves(seed: u64) -> Vec<f32> {
    let n = seconds(7.0);
    let noise = filtered_noise(n, 0.07, seed);
    let samples = (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let swell =
                (t * std::f32::consts::TAU * 2.0 - std::f32::consts::FRAC_PI_2).sin() * 0.5 + 0.5;
            let fade = (t * std::f32::consts::PI).sin();
            noise[i] * swell * fade
        })
        .collect();
    normalize(samples, 0.6)
}

/// Une chouette : deux « hou » graves.
fn owl(seed: u64) -> Vec<f32> {
    let n = seconds(2.2);
    let mut samples = vec![0.0f32; n];
    let mut rng = Rng::new(seed);
    let hz = 330.0 + rng.next_f32() * 40.0;
    for (delay, drop) in [(0.1, 0.0), (0.75, 40.0)] {
        let start = seconds(delay);
        for k in 0..seconds(0.45) {
            let Some(s) = samples.get_mut(start + k) else {
                break;
            };
            let t = k as f32 / RATE as f32;
            let env = (std::f32::consts::PI * k as f32 / seconds(0.45) as f32)
                .sin()
                .powi(2);
            let phase = t * (hz - drop - t * 60.0) * std::f32::consts::TAU;
            *s += (phase.sin() + 0.3 * (2.0 * phase).sin()) * env * 0.5;
        }
    }
    normalize(samples, 0.5)
}

/// Un scintillement : l'enchantement. Des notes aiguës qui montent et s'estompent.
fn enchant() -> Vec<f32> {
    let n = seconds(1.4);
    let mut samples = vec![0.0f32; n];
    for (i, semitone) in [0.0f32, 4.0, 7.0, 12.0, 16.0, 19.0, 24.0]
        .iter()
        .enumerate()
    {
        pluck(
            &mut samples,
            RATE,
            seconds(0.08 * i as f32),
            note(660.0, *semitone),
            0.7,
            0.25,
            0.6,
        );
    }
    normalize(samples, 0.6)
}

/// Des bulles : l'alambic.
fn bubbles(seed: u64) -> Vec<f32> {
    let n = seconds(0.9);
    let mut samples = vec![0.0f32; n];
    let mut rng = Rng::new(seed);
    for _ in 0..6 {
        let start = (rng.next_f32() * n as f32 * 0.75) as usize;
        let hz = 300.0 + rng.next_f32() * 500.0;
        for k in 0..seconds(0.12) {
            let Some(s) = samples.get_mut(start + k) else {
                break;
            };
            let t = k as f32 / RATE as f32;
            *s +=
                (t * hz * (1.0 + t * 12.0) * std::f32::consts::TAU).sin() * (-t / 0.03).exp() * 0.4;
        }
    }
    normalize(samples, 0.6)
}

/// Une gorgée : deux glouglous graves.
fn gulp() -> Vec<f32> {
    let n = seconds(0.55);
    let mut samples = vec![0.0f32; n];
    for delay in [0.02, 0.26] {
        let start = seconds(delay);
        for k in 0..seconds(0.2) {
            let Some(s) = samples.get_mut(start + k) else {
                break;
            };
            let t = k as f32 / RATE as f32;
            *s += (t * (180.0 + t * 500.0) * std::f32::consts::TAU).sin() * (-t / 0.06).exp();
        }
    }
    normalize(samples, 0.6)
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
    write_wav_at(path, samples, RATE)
}

/// Comme [`write_wav`], à une autre fréquence d'échantillonnage (la musique, plus lente,
/// se contente de 11 025 Hz : quatre fois moins lourde).
fn write_wav_at(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let mut file = BufWriter::new(File::create(path)?);
    let data_len = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // taille du bloc « fmt »
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 2).to_le_bytes())?; // octets par seconde
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
    let mut sounds = sounds;
    sounds.extend([
        ("birds", birds(21)),
        ("drip", drip(22)),
        ("cave_rumble", cave_rumble(23)),
        ("waves", waves(24)),
        ("owl", owl(25)),
        ("enchant", enchant()),
        ("bubbles", bubbles(26)),
        ("gulp", gulp()),
    ]);
    let mut paths = Vec::new();
    // La musique : quatre morceaux d'humeurs différentes, à leur propre fréquence.
    let pentatonic = [0.0, 2.0, 4.0, 7.0, 9.0];
    let lydian = [0.0, 2.0, 4.0, 6.0, 7.0, 9.0, 11.0];
    let minor = [0.0, 3.0, 5.0, 7.0, 10.0];
    let dark = [0.0, 1.0, 5.0, 7.0, 8.0];
    for (name, samples) in [
        (
            "music_day1",
            music(46.0, 31, 261.6, &pentatonic, 72.0, 0.25, 1.0),
        ),
        (
            "music_day2",
            music(46.0, 32, 293.7, &lydian, 64.0, 0.3, 0.8),
        ),
        (
            "music_night",
            music(48.0, 33, 220.0, &minor, 52.0, 0.4, 0.6),
        ),
        ("music_cave", music(48.0, 34, 110.0, &dark, 44.0, 0.55, 0.4)),
    ] {
        let path = dir.join(format!("{name}.wav"));
        write_wav_at(&path, &samples, MUSIC_RATE)?;
        paths.push(path);
    }
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
