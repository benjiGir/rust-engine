//! L'audio, avec `kira` : sons des blocs, pas, éclaboussures et ambiance jour/nuit.
//!
//! # Les contraintes du temps réel
//!
//! La carte son réclame un petit paquet d'échantillons toutes les quelques
//! millisecondes, sur un thread à elle. Si ce thread attend (un verrou, une allocation
//! mémoire, la lecture d'un fichier), le paquet arrive en retard : on entend un
//! craquement. Le thread audio ne doit donc **jamais** bloquer.
//!
//! `kira` s'en charge : le thread du jeu ne touche jamais au son directement. Il
//! envoie des commandes (« joue ce son », « baisse ce volume ») par des files sans
//! verrou, que le thread audio lit à son rythme. De notre côté, on fait tout le travail
//! lent à l'avance : les fichiers WAV sont lus et décodés au démarrage, et jouer un son
//! ne fait que cloner un `Arc` vers ses échantillons déjà prêts.
//!
//! # Deux bus
//!
//! Les sons passent par deux pistes (« bus ») dont on règle le volume séparément,
//! comme sur une table de mixage : les effets (pas, blocs) et l'ambiance (vent, grillons).

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use engine_voxel::{BlockId, BlockRegistry};
use kira::effect::compressor::CompressorBuilder;
use kira::sound::FromFileError;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{MainTrackBuilder, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};

const AMBIENCE_DAY: &str = "core:ambience_day";
const AMBIENCE_NIGHT: &str = "core:ambience_night";
pub const SPLASH: &str = "core:splash";
const RAIN: &str = "core:rain";
pub const THUNDER: &str = "core:thunder";
/// Les sons d'ambiance joués de temps en temps, et les morceaux de musique.
pub const AMBIENT_SOUNDS: &[&str] = &[
    "core:birds",
    "core:drip",
    "core:cave_rumble",
    "core:waves",
    "core:owl",
];
pub const MUSIC_TRACKS: &[&str] = &[
    "core:music_day1",
    "core:music_day2",
    "core:music_night",
    "core:music_cave",
];
/// Les sons de l'enchantement, de l'alambic et des potions.
pub const CHEST_OPEN: &str = "core:chest_open";
pub const CHEST_CLOSE: &str = "core:chest_close";
pub const UI_SOUNDS: &[&str] = &[
    "core:enchant",
    "core:bubbles",
    "core:gulp",
    CHEST_OPEN,
    CHEST_CLOSE,
];

/// Les réglages, lus dans l'environnement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioSettings {
    pub muted: bool,
    /// Volumes de 0 à 1.
    pub effects: f32,
    pub ambience: f32,
}

impl AudioSettings {
    /// `VOXEL_MUTE=1`, `VOXEL_VOLUME_EFFECTS=0.8`, `VOXEL_VOLUME_AMBIENCE=0.3`.
    pub fn from_env() -> Self {
        let volume = |name: &str, default: f32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(default)
                .clamp(0.0, 1.0)
        };
        Self {
            muted: std::env::var("VOXEL_MUTE").is_ok_and(|v| v != "0"),
            effects: volume("VOXEL_VOLUME_EFFECTS", 0.8),
            ambience: volume("VOXEL_VOLUME_AMBIENCE", 0.35),
        }
    }
}

/// Un volume de 0 à 1 vers des décibels : l'oreille entend en échelle logarithmique.
/// 1 → 0 dB (inchangé), 0,5 → -6 dB, 0 → silence.
pub fn to_decibels(volume: f32) -> Decibels {
    if volume <= 0.0001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * volume.log10())
    }
}

/// Le type de son d'un bloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlockSound {
    Step,
    Break,
    Place,
}

struct SoundBank {
    takes: Vec<StaticSoundData>,
    last: Option<usize>,
}

/// Tirage uniforme parmi les autres prises : aucune répétition immédiate.
fn take_index(seed: &mut u32, count: usize, last: Option<usize>) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    if count <= 1 {
        return 0;
    }
    let excluded = last.filter(|&i| i < count);
    let pick = *seed as usize % (count - usize::from(excluded.is_some()));
    pick + usize::from(excluded.is_some_and(|i| pick >= i))
}

pub struct Audio {
    /// Le gestionnaire doit rester en vie : le détruire coupe le son.
    _manager: AudioManager,
    effects: TrackHandle,
    ambience: TrackHandle,
    /// Les sons décodés, par identifiant (`core:step_soft`...).
    sounds: HashMap<String, SoundBank>,
    /// Les morceaux longs se décodent au fil de la lecture, pas tous en mémoire.
    music_paths: HashMap<String, PathBuf>,
    music: Option<StreamingSoundHandle<FromFileError>>,
    day: Option<StaticSoundHandle>,
    night: Option<StaticSoundHandle>,
    /// La boucle de pluie, et le dernier volume appliqué.
    rain: Option<StaticSoundHandle>,
    last_rain: f32,
    /// Le dernier mélange jour/nuit appliqué, pour ne pas renvoyer la même commande.
    last_mix: f32,
    environment_gain: f32,
    /// Un petit générateur pseudo-aléatoire : chaque son est joué un peu plus aigu ou
    /// plus grave, pour que des pas répétés ne sonnent pas comme un disque rayé.
    seed: u32,
}

impl Audio {
    /// `None` si le son est coupé ou s'il n'y a pas de sortie audio : le jeu continue.
    pub fn new(registry: &BlockRegistry, settings: AudioSettings) -> Option<Self> {
        if settings.muted {
            log::info!("son coupé (VOXEL_MUTE)");
            return None;
        }
        match Self::try_new(registry, settings) {
            Ok(audio) => Some(audio),
            Err(err) => {
                log::warn!("pas d'audio : {err:#}");
                None
            }
        }
    }

    fn try_new(registry: &BlockRegistry, settings: AudioSettings) -> Result<Self> {
        let settings_manager = AudioManagerSettings {
            main_track_builder: MainTrackBuilder::new().volume(Decibels(-3.0)).with_effect(
                CompressorBuilder::new()
                    .threshold(-4.0)
                    .ratio(8.0)
                    .attack_duration(std::time::Duration::from_millis(2))
                    .release_duration(std::time::Duration::from_millis(120)),
            ),
            ..Default::default()
        };
        let mut manager = AudioManager::<DefaultBackend>::new(settings_manager)?;
        let effects =
            manager.add_sub_track(TrackBuilder::new().volume(to_decibels(settings.effects)))?;
        let ambience =
            manager.add_sub_track(TrackBuilder::new().volume(to_decibels(settings.ambience)))?;
        let mut audio = Self {
            _manager: manager,
            effects,
            ambience,
            sounds: load_sounds(registry),
            music_paths: music_paths(registry),
            music: None,
            day: None,
            night: None,
            rain: None,
            last_rain: -1.0,
            last_mix: -1.0,
            environment_gain: 1.0,
            seed: 0x9E37_79B9,
        };
        audio.start_ambience();
        log::info!(
            "audio : {} sons chargés (effets {:.0} %, ambiance {:.0} %)",
            audio.sounds.len(),
            settings.effects * 100.0,
            settings.ambience * 100.0
        );
        Ok(audio)
    }

    /// Change les volumes (depuis les options), avec un court fondu.
    pub fn set_volumes(&mut self, effects: f32, ambience: f32) {
        let tween = Tween {
            duration: std::time::Duration::from_millis(100),
            ..Tween::default()
        };
        self.effects.set_volume(to_decibels(effects), tween);
        self.ambience.set_volume(to_decibels(ambience), tween);
    }

    /// Relit les fichiers (rechargement à chaud) et relance l'ambiance.
    pub fn reload(&mut self, registry: &BlockRegistry) {
        self.sounds = load_sounds(registry);
        self.music_paths = music_paths(registry);
        if let Some(mut handle) = self.music.take() {
            handle.stop(Tween::default());
        }
        for handle in [self.day.take(), self.night.take(), self.rain.take()]
            .into_iter()
            .flatten()
        {
            let mut handle = handle;
            handle.stop(Tween::default());
        }
        self.last_mix = -1.0;
        self.last_rain = -1.0;
        self.start_ambience();
        log::info!("{} sons rechargés", self.sounds.len());
    }

    /// Les deux boucles d'ambiance démarrent ensemble ; leur volume relatif suit l'heure.
    fn start_ambience(&mut self) {
        let start = |key: &str, audio: &mut Self| {
            let data = audio.sounds.get(key)?.takes.first()?.clone();
            // `loop_region(..)` : tout le son, en boucle. Volume nul au départ.
            let data = data.loop_region(..).volume(Decibels::SILENCE);
            audio.ambience.play(data).ok()
        };
        self.day = start(AMBIENCE_DAY, self);
        self.night = start(AMBIENCE_NIGHT, self);
        self.rain = start(RAIN, self);
    }

    /// Joue un son d'ambiance ou un morceau de musique sur la piste de l'ambiance (réglée
    /// par « Musique et ambiance »). Renvoie sa durée en secondes.
    pub fn play_ambient(&mut self, key: &str) -> Option<f32> {
        if let Some(path) = self.music_paths.get(key) {
            let data = StreamingSoundData::from_file(path)
                .map_err(|err| log::warn!("musique « {key} » : {err}"))
                .ok()?;
            let seconds = data.duration().as_secs_f32();
            if let Some(mut old) = self.music.take() {
                old.stop(Tween::default());
            }
            self.music = Some(self.ambience.play(data).ok()?);
            return Some(seconds);
        }
        let bank = self.sounds.get_mut(key)?;
        let index = take_index(&mut self.seed, bank.takes.len(), bank.last);
        bank.last = Some(index);
        let data = bank.takes[index].clone();
        let seconds = data.duration().as_secs_f32();
        self.ambience.play(data).ok()?;
        Some(seconds)
    }

    /// Le volume de la pluie, de 0 (rien) à 1 (l'averse).
    pub fn update_rain(&mut self, amount: f32) {
        if (amount - self.last_rain).abs() < 0.02 {
            return;
        }
        self.last_rain = amount;
        let tween = Tween {
            duration: std::time::Duration::from_secs(1),
            ..Tween::default()
        };
        if let Some(rain) = &mut self.rain {
            rain.set_volume(to_decibels(amount * 0.8), tween);
        }
    }

    /// À chaque image : fond le vent du jour et les grillons de la nuit selon la lumière.
    pub fn update_ambience(&mut self, daylight: f32) {
        let mix = ((daylight - 0.1) / 0.9).clamp(0.0, 1.0);
        if (mix - self.last_mix).abs() < 0.02 {
            return;
        }
        self.last_mix = mix;
        // Un fondu d'une seconde : pas de saut de volume audible.
        let tween = Tween {
            duration: std::time::Duration::from_secs(1),
            ..Tween::default()
        };
        if let Some(day) = &mut self.day {
            day.set_volume(to_decibels(mix * self.environment_gain), tween);
        }
        if let Some(night) = &mut self.night {
            night.set_volume(to_decibels((1.0 - mix) * self.environment_gain), tween);
        }
    }

    /// Les boucles de nature ne doivent pas introduire des oiseaux dans une grotte ou un désert.
    pub fn set_environment(&mut self, place: crate::soundscape::Place) {
        let gain = if !place.underground && place.green {
            1.0
        } else {
            0.0
        };
        if self.environment_gain != gain {
            self.environment_gain = gain;
            self.last_mix = -1.0;
        }
    }

    /// Joue un son sur le bus des effets, avec une légère variation de hauteur.
    pub fn play(&mut self, key: &str) {
        let Some(bank) = self.sounds.get_mut(key) else {
            return;
        };
        let index = take_index(&mut self.seed, bank.takes.len(), bank.last);
        bank.last = Some(index);
        // Deux pour cent seulement : garder le timbre des prises enregistrées.
        let pitch = 0.98 + (self.seed.rotate_left(11) % 1000) as f64 / 1000.0 * 0.04;
        // Cloner un `StaticSoundData` ne copie pas les échantillons : ils sont dans un `Arc`.
        let data = bank.takes[index].clone().playback_rate(pitch);
        if let Err(err) = self.effects.play(data) {
            log::debug!("son « {key} » non joué : {err}");
        }
    }

    /// Le son d'un bloc, tel que décrit dans `blocks.ron`.
    pub fn play_block(&mut self, registry: &BlockRegistry, block: BlockId, kind: BlockSound) {
        let sounds = &registry.def(block).sounds;
        let key = match kind {
            BlockSound::Step => &sounds.step,
            BlockSound::Break => &sounds.break_,
            BlockSound::Place => &sounds.place,
        };
        if let Some(key) = key.clone() {
            self.play(&key);
        }
    }
}

/// Tous les sons cités par le registre, plus l'ambiance et l'éclaboussure.
/// Le décodage des WAV se fait ici, une fois, loin du thread audio.
fn sound_path(registry: &BlockRegistry, key: &str) -> Option<PathBuf> {
    ["wav", "ogg"]
        .into_iter()
        .filter_map(|ext| registry.asset_path(key, "sounds", ext))
        .find(|p| p.is_file())
}

fn music_paths(registry: &BlockRegistry) -> HashMap<String, PathBuf> {
    MUSIC_TRACKS
        .iter()
        .filter_map(|key| Some(((*key).to_string(), sound_path(registry, key)?)))
        .collect()
}

fn load_sounds(registry: &BlockRegistry) -> HashMap<String, SoundBank> {
    let mut keys: Vec<String> = [AMBIENCE_DAY, AMBIENCE_NIGHT, SPLASH, RAIN, THUNDER]
        .into_iter()
        .chain(AMBIENT_SOUNDS.iter().copied())
        .chain(UI_SOUNDS.iter().copied())
        .map(str::to_owned)
        .collect();
    for (_, def) in registry.iter() {
        for key in [&def.sounds.step, &def.sounds.break_, &def.sounds.place]
            .into_iter()
            .flatten()
        {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    let mut sounds = HashMap::new();
    for key in keys {
        let Some(path) = sound_path(registry, &key) else {
            continue;
        };
        let mut takes = Vec::new();
        let mut paths = vec![path.clone()];
        for take in 1..8 {
            let name = format!(
                "{}_{}.{}",
                path.file_stem().unwrap().to_string_lossy(),
                take,
                path.extension().unwrap().to_string_lossy()
            );
            let variant = path.with_file_name(name);
            if variant.is_file() {
                paths.push(variant);
            }
        }
        for path in paths {
            match StaticSoundData::from_file(&path) {
                Ok(data) => takes.push(data),
                Err(err) => log::warn!("son « {key} » ({}) : {err}", path.display()),
            }
        }
        if !takes.is_empty() {
            sounds.insert(key, SoundBank { takes, last: None });
        }
    }
    sounds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_to_decibels() {
        assert_eq!(to_decibels(1.0), Decibels(0.0));
        assert!((to_decibels(0.5).0 + 6.02).abs() < 0.01);
        assert_eq!(to_decibels(0.0), Decibels::SILENCE);
    }

    #[test]
    fn take_selection_varies_without_immediate_repetition_and_supports_single_takes() {
        let mut seed = 42;
        let mut last = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let next = take_index(&mut seed, 4, last);
            assert_ne!(Some(next), last);
            seen.insert(next);
            last = Some(next);
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(take_index(&mut seed, 1, Some(0)), 0);
    }

    #[test]
    fn music_streams_and_material_banks_have_four_real_takes() {
        let registry = crate::test_registry();
        let paths = music_paths(&registry);
        for key in MUSIC_TRACKS {
            let data = StreamingSoundData::from_file(paths.get(*key).unwrap()).unwrap();
            assert!(data.duration().as_secs_f32() > 60.0, "{key}");
        }
        let banks = load_sounds(&registry);
        for material in ["soft", "hard", "sand", "snow", "wood", "metal"] {
            let key = format!("core:step_{material}");
            assert_eq!(banks.get(&key).unwrap().takes.len(), 4, "{key}");
        }
    }

    /// Le décodage marche sans carte son : on vérifie que chaque son cité existe et se lit.
    #[test]
    fn every_referenced_sound_decodes() {
        let registry = crate::test_registry();
        let sounds = load_sounds(&registry);
        for key in [
            AMBIENCE_DAY,
            AMBIENCE_NIGHT,
            SPLASH,
            "core:step_soft",
            "core:break_glass",
        ]
        .into_iter()
        .chain(AMBIENT_SOUNDS.iter().copied())
        .chain(UI_SOUNDS.iter().copied())
        {
            let sound = sounds
                .get(key)
                .unwrap_or_else(|| panic!("son « {key} » absent"));
            assert!(sound.takes[0].duration().as_secs_f64() > 0.05, "{key}");
        }
    }
}
