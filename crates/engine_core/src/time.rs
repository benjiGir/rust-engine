//! Boucle à pas de temps fixe (« Fix Your Timestep! », Glenn Fiedler).
//!
//! La logique avance par ticks de durée fixe, quel que soit le nombre d'images
//! par seconde. C'est l'équivalent de `FixedUpdate` dans Unity.

use std::time::Duration;

/// Au-delà de cette durée, une frame est tronquée : si le jeu a gelé une seconde
/// (débogueur, fenêtre déplacée), on ne rattrape pas 60 ticks d'un coup.
const MAX_FRAME_TIME: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct FixedTimestep {
    step: Duration,
    accumulator: Duration,
}

impl FixedTimestep {
    /// `ticks_per_second` : 60 pour un jeu d'action, 20 pour Minecraft.
    pub fn new(ticks_per_second: u32) -> Self {
        assert!(ticks_per_second > 0, "il faut au moins un tick par seconde");
        Self {
            step: Duration::from_secs(1) / ticks_per_second,
            accumulator: Duration::ZERO,
        }
    }

    /// Durée d'un tick.
    pub fn step(&self) -> Duration {
        self.step
    }

    /// Ajoute le temps réel écoulé depuis la dernière frame et renvoie
    /// le nombre de ticks de logique à exécuter maintenant (souvent 0, 1 ou 2).
    pub fn advance(&mut self, frame_time: Duration) -> u32 {
        self.accumulator += frame_time.min(MAX_FRAME_TIME);
        let mut ticks = 0;
        while self.accumulator >= self.step {
            self.accumulator -= self.step;
            ticks += 1;
        }
        ticks
    }

    /// Fraction du tick suivant déjà écoulée, entre 0 et 1.
    /// Sert à interpoler l'affichage entre deux états de la logique.
    pub fn alpha(&self) -> f32 {
        self.accumulator.as_secs_f32() / self.step.as_secs_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_tick_per_step() {
        let mut ts = FixedTimestep::new(50); // 20 ms par tick
        assert_eq!(ts.advance(Duration::from_millis(20)), 1);
        assert_eq!(ts.advance(Duration::from_millis(10)), 0);
        assert!((ts.alpha() - 0.5).abs() < 1e-4);
        assert_eq!(ts.advance(Duration::from_millis(30)), 2);
    }

    #[test]
    fn long_frames_are_clamped() {
        let mut ts = FixedTimestep::new(60);
        // Une frame de 5 s ne doit pas déclencher 300 ticks.
        let ticks = ts.advance(Duration::from_secs(5));
        assert!(ticks <= 15, "trop de ticks : {ticks}");
    }
}
