//! Le ciel que voit la fenêtre : l'heure et le temps qu'il fait viennent du serveur.
//!
//! Le serveur tient l'horloge ([`DayCycle`]) et décide du temps ([`Weather::update_sky`]) : c'est ce
//! qui compte pour le jeu (les créatures de la nuit, les cultures sous la pluie, le lit). La fenêtre
//! garde sa copie des deux : son horloge avance d'elle-même entre deux messages, sa météo suit le ciel
//! du serveur et fait seule ce qui se voit et s'entend (les gouttes autour du joueur, les éclairs, le
//! tonnerre).
//!
//! [`ServerMsg::Sky`] part quand quelque chose change que la copie ne peut pas deviner : la vitesse
//! du temps, le ciel de la dimension, le temps qu'il fait, ou l'heure qui saute (`ctl time`, le lit).
//! Pour le savoir, le serveur fait tourner à côté la même horloge que la fenêtre (`ServerSends::sky`).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::daytime::{DayCycle, TimeSpeed};
use crate::protocol::ServerMsg;
use crate::weather::{Sky, Weather};

/// L'écart d'heure (en fraction de journée) au-delà duquel la copie est recalée : une seconde à
/// vingt minutes par jour.
const DRIFT: f32 = 1.0 / 1200.0;

/// Le ciel tel que le serveur l'envoie.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SkyView {
    pub time: f32,
    pub day_length: f32,
    pub speed: TimeSpeed,
    /// Le ciel fixe de la dimension : (couleur, lumière).
    pub fixed_sky: Option<(Vec3, f32)>,
    pub weather: Sky,
}

impl SkyView {
    pub fn of(day: &DayCycle, weather: &Weather) -> Self {
        Self {
            time: day.time,
            day_length: day.day_length,
            speed: day.speed,
            fixed_sky: day.fixed_sky(),
            weather: weather.sky,
        }
    }

    /// L'horloge que la fenêtre en tire.
    pub fn day(&self) -> DayCycle {
        DayCycle::copy_of(self.time, self.day_length, self.speed, self.fixed_sky)
    }
}

/// La copie que tient le serveur de ce qu'a la fenêtre : le dernier ciel envoyé, et une horloge qui
/// avance comme la sienne.
#[derive(Debug, Clone, Default)]
pub struct SkyEcho {
    sent: Option<(SkyView, DayCycle)>,
}

impl SkyEcho {
    /// Avance l'horloge de la copie, et dit s'il faut envoyer `now`.
    pub fn changed(&mut self, now: SkyView, dt: f32) -> bool {
        let Some((sent, day)) = &mut self.sent else {
            return self.remember(now);
        };
        day.update(dt);
        let drift = (day.time - now.time).abs();
        let drift = drift.min(1.0 - drift);
        let same = sent.speed == now.speed
            && sent.day_length == now.day_length
            && sent.fixed_sky == now.fixed_sky
            && sent.weather == now.weather;
        if same && drift <= DRIFT {
            return false;
        }
        self.remember(now)
    }

    fn remember(&mut self, now: SkyView) -> bool {
        self.sent = Some((now, now.day()));
        true
    }
}

impl crate::server_side::VoxelServer {
    /// Envoie le ciel à chaque joueur quand sa copie ne peut pas le deviner.
    pub(crate) fn send_sky(&mut self, dt: f32) {
        let now = SkyView::of(&self.day, &self.weather);
        if !self.sends.sky.changed(now, dt) {
            return;
        }
        let ids: Vec<_> = self.players.iter().map(|p| p.id).collect();
        for id in ids {
            self.replies.push((id, ServerMsg::Sky(now)));
        }
    }
}

impl crate::VoxelGame {
    /// La fenêtre recale sa copie du ciel.
    pub(crate) fn receive_sky(&mut self, view: SkyView) {
        self.client.day = view.day();
        self.client.weather.follow(view.weather);
    }

    /// Une image du ciel de la fenêtre : l'horloge avance, la météo fait tomber ses gouttes autour du
    /// joueur et gronde.
    pub(crate) fn update_sky(&mut self, dt: f32) {
        self.client.day.update(dt);
        self.client.weather.update_sky(dt);
        let feet = self.client.body.feet();
        let thunder = self
            .client
            .weather
            .update_visuals(dt, self.client.view.world(), feet);
        if let Some(audio) = &mut self.client.audio {
            if thunder {
                audio.play(crate::audio::THUNDER);
            }
            audio.update_rain(self.client.weather.amount());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(time: f32) -> SkyView {
        SkyView {
            time,
            day_length: 1200.0,
            speed: TimeSpeed::Normal,
            fixed_sky: None,
            weather: Sky::Clear,
        }
    }

    #[test]
    fn the_sky_is_sent_only_when_the_copy_cannot_guess_it() {
        let mut echo = SkyEcho::default();
        assert!(echo.changed(view(0.25), 0.0), "le premier ciel part");
        // L'heure avance comme chez la fenêtre : rien à dire.
        assert!(!echo.changed(view(0.25 + 60.0 / 1200.0), 60.0));
        // Un saut d'heure, un autre temps, une autre vitesse : ça part.
        assert!(echo.changed(view(0.75), 0.0));
        assert!(echo.changed(
            SkyView {
                weather: Sky::Rain,
                ..view(0.75)
            },
            0.0
        ));
        assert!(echo.changed(
            SkyView {
                speed: TimeSpeed::Frozen,
                weather: Sky::Rain,
                ..view(0.75)
            },
            0.0
        ));
    }

    #[test]
    fn the_copy_of_the_weather_never_changes_the_sky_by_itself() {
        let mut weather = Weather::follower(7, Sky::Clear, 0.0);
        for _ in 0..2000 {
            weather.update_sky(1.0);
        }
        assert_eq!(weather.sky, Sky::Clear);
        weather.follow(Sky::Storm);
        weather.update_sky(100.0);
        assert_eq!((weather.sky, weather.overcast), (Sky::Storm, 1.0));
    }
}
