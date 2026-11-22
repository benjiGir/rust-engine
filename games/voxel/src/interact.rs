//! Les interactions du joueur avec le monde : levier, bouton, houe, engrais, animaux et
//! habitants, fioles, et ce qui s'y rattache (potions en cours, musique, flèche d'orientation).

use super::*;
#[allow(unused_imports)]
use voxel_sim::server::interact::*;
use voxel_sim::world::soundscape;

impl VoxelGame {
    /// Pose le bloc de la case choisie contre la face visée. Renvoie sa position si c'est fait.
    /// Une flèche sur le bloc visé s'il est orienté (piston, inverseur, répéteur) : une barre
    /// claire dans sa direction, avec un bout plus vif devant.
    pub(crate) fn direction_hint(&self, out: &mut Vec<engine_render::CubeInstance>) {
        let Some(hit) = self.client.target else {
            return;
        };
        let streamer = self.server.streamer();
        let block = streamer.world().block(hit.block_pos);
        let directional = matches!(
            self.client.registry.def(block).behavior.as_deref(),
            Some(
                "core:piston"
                    | "core:sticky_piston"
                    | "core:inverter"
                    | "core:repeater"
                    | "core:propeller"
            )
        );
        if !directional {
            return;
        }
        let facing = engine_voxel::facing_of(streamer.block_entity(hit.block_pos));
        let center = hit.block_pos.as_vec3() + Vec3::splat(0.5) + hit.normal.as_vec3() * 0.52;
        let along = facing.as_vec3();
        let bar = Vec3::ONE * 0.12 + along.abs() * 0.5;
        out.push(engine_render::CubeInstance::new(
            center,
            bar,
            Vec3::new(1.0, 0.9, 0.3),
        ));
        out.push(engine_render::CubeInstance::new(
            center + along * 0.3,
            Vec3::splat(0.2),
            Vec3::new(1.0, 0.3, 0.1),
        ));
    }

    /// La musique et les sons d'ambiance, selon l'endroit où se trouve le joueur.
    pub(crate) fn update_soundscape(&mut self, dt: f32) {
        if self.client.audio.is_none() {
            return;
        }
        let feet = self.client.body.body.position;
        let place = {
            let streamer = self.server.streamer();
            let generator = streamer.generator();
            let (x, z) = (feet.x.floor() as i32, feet.z.floor() as i32);
            let column = generator.column(x, z);
            let key = &generator.biome(column.biome).key;
            let (sky, _) = streamer.world().light(feet.floor().as_ivec3() + IVec3::Y);
            let water = ["ocean", "beach", "river", "swamp"]
                .iter()
                .any(|w| key.contains(w));
            soundscape::Place {
                underground: sky <= 2 && feet.y < column.height as f32 - 8.0,
                night: self.client.day.is_night(),
                raining: self.client.weather.amount() > 0.2,
                near_water: water,
                green: !["desert", "tundra", "ocean", "beach", "mountain"]
                    .iter()
                    .any(|w| key.contains(w)),
            }
        };
        let playing = self.client.music_until.is_some_and(|t| Instant::now() < t);
        let cues = self.client.soundscape.update(dt, place, playing);
        let Some(audio) = &mut self.client.audio else {
            return;
        };
        audio.set_environment(place);
        for cue in cues {
            match cue {
                soundscape::Cue::Music(key) => {
                    // Un morceau introuvable : on réessaiera plus tard.
                    let seconds = audio.play_ambient(key).unwrap_or(0.0);
                    self.client.soundscape.started(seconds);
                    self.client.music_until =
                        Some(Instant::now() + std::time::Duration::from_secs_f32(seconds));
                }
                soundscape::Cue::Ambient(key) => {
                    audio.play_ambient(key);
                }
            }
        }
    }

    /// Ferme l'écran ouvert : la grille d'artisanat et la main retournent dans
    /// l'inventaire (ce qui n'y tient pas tombe par terre).
    pub(crate) fn close_screen(&mut self) {
        // Le serveur prévient le mod (`CloseMenu`, voir `close_menu`).
        self.client.ui.mod_menu = None;
        self.client.ui.browser.close();
        self.client.ui.split = None;
        // Le simulateur range la main et la grille d'artisanat (voir `intents.rs`).
        self.send_now(voxel_sim::net::protocol::ClientMsg::CloseMenu);
        self.client.ui.inventory_open = false;
        self.client.ui.screen = ui::Screen::Inventory;
    }
}
