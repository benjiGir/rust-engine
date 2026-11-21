//! La part fenêtre de `sky_view.rs` : ses méthodes sur `VoxelGame` (voir le lot 8 du plan
//! multijoueur ; la simulation reste dans `sky_view.rs`).

#[allow(unused_imports)]
use crate::sky_view::*;

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
