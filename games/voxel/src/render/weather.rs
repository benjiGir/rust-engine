//! Ce que la fenêtre dessine de la météo (voir `world/weather.rs`, côté simulation) : le ciel
//! nuageux et les gouttes ou les flocons autour du joueur.

use engine_render::CubeInstance;
use glam::Vec3;

use voxel_sim::world::weather::Weather;

/// Le ciel à dessiner : nuages, éclair, dérive.
pub fn sky(weather: &Weather) -> engine_render::Sky {
    engine_render::Sky {
        overcast: weather.overcast,
        flash: weather.flash(),
        time: weather.clock(),
        seed: weather.seed(),
    }
}

/// Les gouttes (ou les flocons) autour du joueur.
pub fn instances(weather: &Weather, out: &mut Vec<CubeInstance>) {
    let snow = weather.snowing();
    for position in weather.drops() {
        out.push(if snow {
            CubeInstance::new(position, Vec3::splat(0.09), Vec3::splat(0.97))
        } else {
            CubeInstance::new(
                position,
                Vec3::new(0.025, 0.55, 0.025),
                Vec3::new(0.55, 0.65, 0.9),
            )
        });
    }
}
