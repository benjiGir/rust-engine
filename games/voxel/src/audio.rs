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

use anyhow::Result;
use engine_voxel::{BlockId, BlockRegistry};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween};

const AMBIENCE_DAY: &str = "core:ambience_day";
const AMBIENCE_NIGHT: &str = "core:ambience_night";
pub const SPLASH: &str = "core:splash";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSound {
    Step,
    Break,
    Place,
}

pub struct Audio {
    /// Le gestionnaire doit rester en vie : le détruire coupe le son.
    _manager: AudioManager,
    effects: TrackHandle,
    ambience: TrackHandle,
    /// Les sons décodés, par identifiant (`core:step_soft`...).
    sounds: HashMap<String, StaticSoundData>,
    day: Option<StaticSoundHandle>,
    night: Option<StaticSoundHandle>,
    /// Le dernier mélange jour/nuit appliqué, pour ne pas renvoyer la même commande.
    last_mix: f32,
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
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())?;
        let effects =
            manager.add_sub_track(TrackBuilder::new().volume(to_decibels(settings.effects)))?;
        let ambience =
            manager.add_sub_track(TrackBuilder::new().volume(to_decibels(settings.ambience)))?;
        let mut audio = Self {
            _manager: manager,
            effects,
            ambience,
            sounds: load_sounds(registry),
            day: None,
            night: None,
            last_mix: -1.0,
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

    /// Relit les fichiers (rechargement à chaud) et relance l'ambiance.
    pub fn reload(&mut self, registry: &BlockRegistry) {
        self.sounds = load_sounds(registry);
        for handle in [self.day.take(), self.night.take()].into_iter().flatten() {
            let mut handle = handle;
            handle.stop(Tween::default());
        }
        self.last_mix = -1.0;
        self.start_ambience();
        log::info!("{} sons rechargés", self.sounds.len());
    }

    /// Les deux boucles d'ambiance démarrent ensemble ; leur volume relatif suit l'heure.
    fn start_ambience(&mut self) {
        let start = |key: &str, audio: &mut Self| {
            let data = audio.sounds.get(key)?.clone();
            // `loop_region(..)` : tout le son, en boucle. Volume nul au départ.
            let data = data.loop_region(..).volume(Decibels::SILENCE);
            audio.ambience.play(data).ok()
        };
        self.day = start(AMBIENCE_DAY, self);
        self.night = start(AMBIENCE_NIGHT, self);
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
            day.set_volume(to_decibels(mix), tween);
        }
        if let Some(night) = &mut self.night {
            night.set_volume(to_decibels(1.0 - mix), tween);
        }
    }

    /// Joue un son sur le bus des effets, avec une légère variation de hauteur.
    pub fn play(&mut self, key: &str) {
        let Some(data) = self.sounds.get(key) else {
            return;
        };
        // Xorshift 32 bits : de quoi tirer une hauteur entre 0,92 et 1,08.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let pitch = 0.92 + (self.seed % 1000) as f64 / 1000.0 * 0.16;
        // Cloner un `StaticSoundData` ne copie pas les échantillons : ils sont dans un `Arc`.
        let data = data.clone().playback_rate(pitch);
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
fn load_sounds(registry: &BlockRegistry) -> HashMap<String, StaticSoundData> {
    let mut keys: Vec<String> = vec![AMBIENCE_DAY.into(), AMBIENCE_NIGHT.into(), SPLASH.into()];
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
        let Some(path) = registry.asset_path(&key, "sounds", "wav") else {
            continue;
        };
        match StaticSoundData::from_file(&path) {
            Ok(data) => {
                sounds.insert(key, data);
            }
            Err(err) => log::warn!("son « {key} » ({}) : {err}", path.display()),
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

    /// Le décodage marche sans carte son : on vérifie que chaque son cité existe et se lit.
    #[test]
    fn every_referenced_sound_decodes() {
        let registry = crate::load_registry().unwrap();
        let sounds = load_sounds(&registry);
        for key in [
            AMBIENCE_DAY,
            AMBIENCE_NIGHT,
            SPLASH,
            "core:step_soft",
            "core:break_glass",
        ] {
            let sound = sounds
                .get(key)
                .unwrap_or_else(|| panic!("son « {key} » absent"));
            assert!(sound.duration().as_secs_f64() > 0.05, "{key}");
        }
    }
}
