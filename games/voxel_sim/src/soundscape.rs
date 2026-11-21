//! La musique et les sons d'ambiance.
//!
//! - **La musique** : un morceau de temps en temps (toutes les une à quatre minutes, dans le
//!   silence entre deux), choisi selon l'endroit : sous terre, la nuit, ou le jour.
//! - **L'ambiance** : de petits sons au hasard selon le lieu : gouttes et grondements dans
//!   les grottes, oiseaux le jour en surface (sauf sous la pluie), chouette la nuit, vagues
//!   près de l'eau.
//!
//! Tout passe par la piste « Musique et ambiance » des options.

/// Ce que le jeu sait de l'endroit où se trouve le joueur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub underground: bool,
    pub night: bool,
    pub raining: bool,
    pub near_water: bool,
    /// Un biome où l'on entend des oiseaux (pas le désert, la toundra, ni l'océan).
    pub green: bool,
}

/// Un morceau, ou un son d'ambiance, à jouer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Music(&'static str),
    Ambient(&'static str),
}

pub struct Soundscape {
    /// Secondes avant le prochain morceau, et avant le prochain son d'ambiance.
    music_in: f32,
    ambient_in: f32,
    rng: u64,
    last_music: Option<&'static str>,
}

impl Soundscape {
    pub fn new(seed: u32) -> Self {
        let mut s = Self {
            music_in: 0.0,
            ambient_in: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ u64::from(seed),
            last_music: None,
        };
        // Un premier morceau assez vite, puis le silence s'installe.
        s.music_in = 8.0 + s.random() * 20.0;
        s.ambient_in = 4.0 + s.random() * 6.0;
        s
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Le morceau qui convient à l'endroit (jamais le même deux fois de suite si possible).
    fn music_for(&mut self, place: Place) -> &'static str {
        let options: &[&'static str] = if place.underground {
            &["core:music_cave"]
        } else if place.night {
            &["core:music_night", "core:music_cave"]
        } else {
            &["core:music_day1", "core:music_day2"]
        };
        let fresh: Vec<_> = options
            .iter()
            .copied()
            .filter(|t| Some(*t) != self.last_music)
            .collect();
        let pool = if fresh.is_empty() { options } else { &fresh };
        let pick = pool[(self.random() * pool.len() as f32) as usize % pool.len()];
        self.last_music = Some(pick);
        pick
    }

    /// Le son d'ambiance de l'endroit, ou rien.
    fn ambient_for(&mut self, place: Place) -> Option<&'static str> {
        let roll = self.random();
        if place.underground {
            return Some(if roll < 0.7 {
                "core:drip"
            } else {
                "core:cave_rumble"
            });
        }
        if place.near_water && roll < 0.5 {
            return Some("core:waves");
        }
        if place.night {
            return Some("core:owl");
        }
        (place.green && !place.raining).then_some("core:birds")
    }

    /// Avance de `dt` secondes ; renvoie ce qu'il faut jouer maintenant. `music_playing` :
    /// un morceau est en cours (on n'en lance pas un second).
    pub fn update(&mut self, dt: f32, place: Place, music_playing: bool) -> Vec<Cue> {
        let mut cues = Vec::new();
        self.music_in -= dt;
        if !music_playing && self.music_in <= 0.0 {
            cues.push(Cue::Music(self.music_for(place)));
            // Le morceau fixe la suite : on réarme selon sa durée (voir `started`).
            self.music_in = f32::INFINITY;
        }
        self.ambient_in -= dt;
        if self.ambient_in <= 0.0 {
            self.ambient_in = 7.0 + self.random() * 14.0;
            if let Some(key) = self.ambient_for(place) {
                cues.push(Cue::Ambient(key));
            }
        }
        cues
    }

    /// Un morceau de `seconds` secondes vient de démarrer : le prochain, dans 1 à 4 minutes
    /// après sa fin.
    pub fn started(&mut self, seconds: f32) {
        self.music_in = seconds + 60.0 + self.random() * 180.0;
    }

    /// Le rechargement arrête la lecture : ne pas attendre la fin de l'ancien morceau.
    pub fn reloaded(&mut self) {
        self.music_in = 8.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: Place = Place {
        underground: false,
        night: false,
        raining: false,
        near_water: false,
        green: true,
    };

    #[test]
    fn music_depends_on_the_place_and_does_not_repeat() {
        let mut s = Soundscape::new(1);
        let cave = Place {
            underground: true,
            ..SURFACE
        };
        assert_eq!(s.music_for(cave), "core:music_cave");
        let night = Place {
            night: true,
            ..SURFACE
        };
        assert!(["core:music_night", "core:music_cave"].contains(&s.music_for(night)));
        let first = s.music_for(SURFACE);
        let second = s.music_for(SURFACE);
        assert_ne!(first, second, "pas deux fois le même");
    }

    #[test]
    fn ambience_fits_the_place() {
        let mut s = Soundscape::new(2);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.extend(s.ambient_for(Place {
                underground: true,
                ..SURFACE
            }));
        }
        assert_eq!(
            seen,
            ["core:drip", "core:cave_rumble"].into_iter().collect()
        );
        // Sous la pluie, pas d'oiseaux ; dans un désert non plus.
        for _ in 0..50 {
            assert_eq!(
                s.ambient_for(Place {
                    raining: true,
                    ..SURFACE
                }),
                None
            );
            assert_eq!(
                s.ambient_for(Place {
                    green: false,
                    ..SURFACE
                }),
                None
            );
        }
        assert_eq!(s.ambient_for(SURFACE), Some("core:birds"));
        assert!((0..50).any(|_| {
            s.ambient_for(Place {
                near_water: true,
                ..SURFACE
            }) == Some("core:waves")
        }));
    }

    #[test]
    fn music_starts_after_a_while_then_waits_for_silence() {
        let mut s = Soundscape::new(3);
        let mut first = None;
        let mut elapsed = 0.0;
        while first.is_none() && elapsed < 120.0 {
            for cue in s.update(1.0, SURFACE, false) {
                if let Cue::Music(track) = cue {
                    first = Some(track);
                }
            }
            elapsed += 1.0;
        }
        assert!(first.is_some(), "un premier morceau");
        // Pendant le morceau et les minutes qui suivent : rien.
        s.started(45.0);
        for _ in 0..100 {
            let music = s
                .update(1.0, SURFACE, false)
                .iter()
                .any(|c| matches!(c, Cue::Music(_)));
            assert!(!music);
        }
        // Un morceau déjà en cours bloque le suivant.
        let mut s = Soundscape::new(4);
        for _ in 0..200 {
            assert!(
                !s.update(1.0, SURFACE, true)
                    .iter()
                    .any(|c| matches!(c, Cue::Music(_)))
            );
        }
    }

    #[test]
    fn the_gap_counts_from_the_end_of_the_track_and_reload_rearms_it() {
        let mut s = Soundscape::new(5);
        s.started(180.0);
        let deadline = s.music_in;
        assert!(
            !s.update(deadline - 1.0, SURFACE, true)
                .iter()
                .any(|c| matches!(c, Cue::Music(_)))
        );
        assert!(
            s.update(1.1, SURFACE, false)
                .iter()
                .any(|c| matches!(c, Cue::Music(_)))
        );
        s.started(480.0);
        s.reloaded();
        assert!(
            s.update(8.1, SURFACE, false)
                .iter()
                .any(|c| matches!(c, Cue::Music(_)))
        );
    }
}
