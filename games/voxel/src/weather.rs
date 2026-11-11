//! La météo : ciel clair, pluie, orage. Elle change au hasard, assombrit le ciel, fait
//! tomber de la pluie ou de la neige selon le biome, et couvre le ciel de nuages.
//!
//! Tout est dessiné avec les mêmes boîtes que les créatures (`CubeInstance`) : une goutte
//! est une fine boîte, un flocon un petit cube, un nuage une grande dalle. Une goutte
//! tombe jusqu'au premier bloc plein sous elle : sous un toit, on est au sec.

use engine_render::CubeInstance;
use engine_voxel::{Precipitation, World, precipitation_at};
use glam::{IVec3, Vec3};

/// Les gouttes et les flocons gardés en vie, au plus.
const MAX_DROPS: usize = 700;
/// Les précipitations tombent dans ce rayon autour du joueur.
const RADIUS: f32 = 14.0;
/// Hauteur d'où tombent les gouttes, au-dessus de la tête du joueur.
const SKY_OFFSET: f32 = 14.0;
/// L'altitude des nuages, et la taille d'une cellule de nuage (en blocs).
const CLOUD_HEIGHT: f32 = 150.0;
const CLOUD_CELL: f32 = 12.0;
/// Rayon d'affichage des nuages, en cellules.
const CLOUD_RADIUS: i32 = 14;
/// Les nuages dérivent vers l'est à cette vitesse (blocs par seconde).
const CLOUD_DRIFT: f32 = 2.0;
/// Le ciel se couvre (ou se dégage) en une dizaine de secondes.
const FADE_SPEED: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sky {
    Clear,
    Rain,
    Storm,
}

impl Sky {
    pub fn name(self) -> &'static str {
        match self {
            Self::Clear => "clair",
            Self::Rain => "pluie",
            Self::Storm => "orage",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "clear" | "clair" => Some(Self::Clear),
            "rain" | "pluie" => Some(Self::Rain),
            "storm" | "orage" => Some(Self::Storm),
            _ => None,
        }
    }

    /// À quel point le ciel est couvert quand ce temps est installé.
    fn overcast(self) -> f32 {
        match self {
            Self::Clear => 0.0,
            Self::Rain => 0.7,
            Self::Storm => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Drop {
    position: Vec3,
    /// La hauteur où elle s'arrête (le dessus du premier bloc plein), si elle en trouve un.
    ground: f32,
}

pub struct Weather {
    pub sky: Sky,
    /// De 0 (ciel dégagé) à 1 (orage) : suit `sky` en douceur.
    pub overcast: f32,
    /// Secondes avant le prochain changement de temps.
    timer: f32,
    /// Secondes avant le prochain éclair, et la lueur de celui en cours (1 → 0).
    lightning_in: f32,
    flash: f32,
    /// Le tonnerre arrive après l'éclair (le son voyage moins vite que la lumière).
    thunder_in: Option<f32>,
    /// Ce qui tombe ici (selon le biome du joueur), relu régulièrement.
    precipitation: Precipitation,
    check_in: f32,
    drops: Vec<Drop>,
    /// Temps écoulé, pour la dérive des nuages et le vent.
    time: f32,
    rng: u64,
    /// Seconde de nuages différente par monde.
    seed: u32,
}

impl Weather {
    pub fn new(seed: u32) -> Self {
        let mut weather = Self {
            sky: Sky::Clear,
            overcast: 0.0,
            timer: 0.0,
            lightning_in: 10.0,
            flash: 0.0,
            thunder_in: None,
            precipitation: Precipitation::Rain,
            check_in: 0.0,
            drops: Vec::new(),
            time: 0.0,
            rng: 0x9E37_79B9_7F4A_7C15 ^ seed as u64,
            seed,
        };
        weather.timer = weather.clear_duration();
        weather
    }

    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }

    fn clear_duration(&mut self) -> f32 {
        240.0 + self.random() * 420.0
    }

    /// Impose un temps (la commande `weather`) ; il dure un moment.
    pub fn set(&mut self, sky: Sky) {
        self.sky = sky;
        self.timer = match sky {
            Sky::Clear => self.clear_duration(),
            _ => 120.0 + self.random() * 180.0,
        };
    }

    /// De la pluie ou de la neige tombe-t-elle à l'écran (ciel assez couvert, et pas un désert) ?
    pub fn precipitation(&self) -> Precipitation {
        if self.overcast < 0.5 {
            Precipitation::None
        } else {
            self.precipitation
        }
    }

    /// La force des précipitations, de 0 à 1.
    pub fn amount(&self) -> f32 {
        if self.precipitation() == Precipitation::None {
            0.0
        } else {
            ((self.overcast - 0.5) * 2.0).clamp(0.0, 1.0)
        }
    }

    /// Un facteur sur la lumière du ciel : 1 par beau temps, plus sombre sous les nuages,
    /// et un éclair l'éclaire d'un coup.
    pub fn daylight_factor(&self) -> f32 {
        (1.0 - 0.5 * self.overcast).max(self.flash)
    }

    /// Le ciel grisonne avec les nuages, et blanchit pendant un éclair.
    pub fn tint_sky(&self, sky: Vec3) -> Vec3 {
        let gray = Vec3::splat(sky.dot(Vec3::new(0.3, 0.59, 0.11)) * 0.55);
        let overcast = sky.lerp(gray, self.overcast * 0.75);
        overcast.lerp(Vec3::splat(0.9), self.flash * 0.6)
    }

    /// Le brouillard se resserre sous la pluie.
    pub fn fog_scale(&self) -> f32 {
        1.0 - 0.4 * self.overcast
    }

    /// Avance d'un pas. Renvoie `true` quand le tonnerre doit se faire entendre.
    pub fn update(&mut self, dt: f32, world: &World, player: Vec3) -> bool {
        self.time += dt;
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.next_weather();
        }
        let target = self.sky.overcast();
        self.overcast += (target - self.overcast).clamp(-FADE_SPEED * dt, FADE_SPEED * dt);

        self.check_in -= dt;
        if self.check_in <= 0.0 {
            self.check_in = 1.0;
            self.precipitation = precipitation_at(world, player.floor().as_ivec3());
        }

        // Les éclairs, seulement dans l'orage.
        self.flash = (self.flash - dt * 3.0).max(0.0);
        let mut thunder = false;
        if self.sky == Sky::Storm && self.overcast > 0.9 {
            self.lightning_in -= dt;
            if self.lightning_in <= 0.0 {
                self.lightning_in = 6.0 + self.random() * 18.0;
                self.flash = 1.0;
                self.thunder_in = Some(0.4 + self.random() * 1.8);
            }
        }
        if let Some(delay) = &mut self.thunder_in {
            *delay -= dt;
            if *delay <= 0.0 {
                self.thunder_in = None;
                thunder = true;
            }
        }

        self.move_drops(dt, world, player);
        thunder
    }

    /// Le temps change : du beau temps vers la pluie (parfois l'orage), puis le retour.
    fn next_weather(&mut self) {
        let roll = self.random();
        self.sky = match self.sky {
            Sky::Clear if roll < 0.75 => Sky::Rain,
            Sky::Clear => Sky::Storm,
            Sky::Rain if roll < 0.3 => Sky::Storm,
            Sky::Rain | Sky::Storm => Sky::Clear,
        };
        self.timer = match self.sky {
            Sky::Clear => self.clear_duration(),
            _ => 90.0 + self.random() * 150.0,
        };
    }

    fn move_drops(&mut self, dt: f32, world: &World, player: Vec3) {
        let wanted = (MAX_DROPS as f32 * self.amount()) as usize;
        let snow = self.precipitation == Precipitation::Snow;
        let speed = if snow { 2.5 } else { 22.0 };
        // Trop de gouttes (la pluie faiblit) : on en retire quelques-unes à chaque pas.
        self.drops
            .truncate(wanted.max(self.drops.len().saturating_sub(40)));
        while self.drops.len() < wanted {
            let drop = self.new_drop(world, player, true);
            self.drops.push(drop);
        }
        let wind = Vec3::new(1.5, 0.0, 0.5);
        for i in 0..self.drops.len() {
            let mut drop = self.drops[i];
            drop.position.y -= speed * dt;
            if snow {
                drop.position.x += (self.time * 1.3 + i as f32).sin() * 0.6 * dt;
                drop.position.z += (self.time * 0.9 + i as f32 * 0.7).cos() * 0.6 * dt;
            } else {
                drop.position += wind * dt;
            }
            let far = (drop.position - player).length() > RADIUS * 2.0;
            if drop.position.y <= drop.ground || far || drop.position.y < player.y - 20.0 {
                drop = self.new_drop(world, player, false);
            }
            self.drops[i] = drop;
        }
        // Moins de précipitations qu'avant : on retire les surplus au fil du temps.
        if self.drops.len() > wanted {
            self.drops.truncate(wanted);
        }
    }

    /// Une nouvelle goutte au-dessus du joueur ; `spread` : à n'importe quelle hauteur
    /// (pour remplir le ciel d'un coup).
    fn new_drop(&mut self, world: &World, player: Vec3, spread: bool) -> Drop {
        let angle = self.random() * std::f32::consts::TAU;
        let distance = self.random().sqrt() * RADIUS;
        let x = player.x + angle.cos() * distance;
        let z = player.z + angle.sin() * distance;
        let top = player.y + SKY_OFFSET;
        let ground = ground_below(world, x, z, top);
        let y = if spread {
            ground.max(player.y - 6.0) + self.random() * (top - ground.max(player.y - 6.0)).max(0.0)
        } else {
            top + self.random() * 4.0
        };
        Drop {
            position: Vec3::new(x, y, z),
            ground,
        }
    }

    /// Les boîtes à dessiner : précipitations et nuages.
    pub fn instances(&self, camera: Vec3, out: &mut Vec<CubeInstance>) {
        let snow = self.precipitation == Precipitation::Snow;
        for drop in &self.drops {
            if snow {
                out.push(CubeInstance::new(
                    drop.position,
                    Vec3::splat(0.09),
                    Vec3::splat(0.97),
                ));
            } else {
                out.push(CubeInstance::new(
                    drop.position,
                    Vec3::new(0.025, 0.55, 0.025),
                    Vec3::new(0.55, 0.65, 0.9),
                ));
            }
        }
        self.clouds(camera, out);
    }

    fn clouds(&self, camera: Vec3, out: &mut Vec<CubeInstance>) {
        // Quelques nuages par beau temps, un ciel couvert sous la pluie.
        let coverage = 0.3 + 0.65 * self.overcast;
        let threshold = 1.0 - coverage;
        let drift = self.time * CLOUD_DRIFT;
        let center_x = ((camera.x - drift) / CLOUD_CELL).floor() as i32;
        let center_z = (camera.z / CLOUD_CELL).floor() as i32;
        let color = Vec3::splat(0.96).lerp(Vec3::new(0.42, 0.44, 0.5), self.overcast);
        for dz in -CLOUD_RADIUS..=CLOUD_RADIUS {
            for dx in -CLOUD_RADIUS..=CLOUD_RADIUS {
                let (cx, cz) = (center_x + dx, center_z + dz);
                if cloud_density(cx, cz, self.seed) < threshold {
                    continue;
                }
                let position = Vec3::new(
                    (cx as f32 + 0.5) * CLOUD_CELL + drift,
                    CLOUD_HEIGHT,
                    (cz as f32 + 0.5) * CLOUD_CELL,
                );
                // Les nuages s'effacent au loin plutôt que d'apparaître d'un coup.
                if (position.x - camera.x).hypot(position.z - camera.z)
                    > (CLOUD_RADIUS as f32 - 1.0) * CLOUD_CELL
                {
                    continue;
                }
                out.push(CubeInstance::new(
                    position,
                    Vec3::new(CLOUD_CELL, 3.5, CLOUD_CELL),
                    color,
                ));
            }
        }
    }
}

/// Le dessus du premier bloc plein sous `(x, top, z)`, ou très bas s'il n'y en a pas.
fn ground_below(world: &World, x: f32, z: f32, top: f32) -> f32 {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let start = top.floor() as i32;
    for y in (start - 48..=start).rev() {
        if world.is_solid(IVec3::new(bx, y, bz)) {
            return y as f32 + 1.0;
        }
    }
    f32::MIN
}

/// Un nombre entre 0 et 1 pour la cellule `(x, z)` : deux échelles de bruit lissé, pour
/// des nuages en amas plutôt qu'en poussière.
fn cloud_density(x: i32, z: i32, seed: u32) -> f32 {
    let big = value_noise(x as f32 / 4.0, z as f32 / 4.0, seed);
    let small = value_noise(x as f32 / 1.7 + 50.0, z as f32 / 1.7, seed.wrapping_add(7));
    big * 0.75 + small * 0.25
}

fn hash(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x85EB_CA6B)
        ^ (z as u32).wrapping_mul(0xC2B2_AE35)
        ^ seed.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (ix, iz) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - ix as f32, z - iz as f32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (u, v) = (smooth(fx), smooth(fz));
    let a = hash(ix, iz, seed);
    let b = hash(ix + 1, iz, seed);
    let c = hash(ix, iz + 1, seed);
    let d = hash(ix + 1, iz + 1, seed);
    (a + (b - a) * u) + ((c + (d - c) * u) - (a + (b - a) * u)) * v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_world() -> World {
        World::with_registry(std::sync::Arc::new(crate::test_registry()))
    }

    #[test]
    fn cloud_noise_is_stable_and_forms_clumps() {
        assert_eq!(cloud_density(3, 4, 9), cloud_density(3, 4, 9));
        // Des cellules voisines se ressemblent (amas), des lointaines non.
        let mut near = 0.0;
        let mut far = 0.0;
        for i in 0..200 {
            near += (cloud_density(i, 0, 1) - cloud_density(i + 1, 0, 1)).abs();
            far += (cloud_density(i, 0, 1) - cloud_density(i + 37, 5, 1)).abs();
        }
        assert!(near < far * 0.6, "près {near}, loin {far}");
        for i in 0..100 {
            let d = cloud_density(i, -i, 3);
            assert!((0.0..=1.0).contains(&d));
        }
    }

    #[test]
    fn the_sky_clouds_over_then_clears_and_storms_flash() {
        let world = empty_world();
        let mut weather = Weather::new(1);
        let player = Vec3::new(0.0, 80.0, 0.0);
        weather.set(Sky::Storm);
        let mut thunder = 0;
        let mut flashed = false;
        for _ in 0..(120 * 20) {
            thunder += weather.update(0.05, &world, player) as u32;
            flashed |= weather.flash > 0.5;
        }
        assert!(weather.overcast > 0.99);
        assert!(
            flashed && thunder > 0,
            "éclairs {flashed}, tonnerre {thunder}"
        );
        assert!(weather.daylight_factor() < 0.6 || weather.flash > 0.0);
        weather.set(Sky::Clear);
        for _ in 0..(30 * 20) {
            weather.update(0.05, &world, player);
        }
        assert!(weather.overcast < 0.01);
        assert_eq!(weather.amount(), 0.0);
    }

    #[test]
    fn rain_falls_only_when_overcast_and_stops_at_a_roof() {
        let mut world = empty_world();
        // Un toit de pierre à y = 90, large.
        let stone = world.registry().clone().id("core:stone").unwrap();
        for x in -20..20 {
            for z in -20..20 {
                world.set_block(IVec3::new(x, 90, z), stone);
            }
        }
        let mut weather = Weather::new(2);
        let player = Vec3::new(0.0, 80.0, 0.0);
        for _ in 0..40 {
            weather.update(0.05, &world, player);
        }
        let mut quiet = Vec::new();
        weather.instances(player, &mut quiet);
        let clouds_only = quiet.len();
        weather.set(Sky::Rain);
        for _ in 0..(40 * 20) {
            weather.update(0.05, &world, player);
        }
        assert!(weather.amount() > 0.3);
        assert!(!weather.drops.is_empty());
        // Sous le toit : aucune goutte ne passe sous y = 90.
        assert!(weather.drops.iter().all(|d| d.position.y > 89.0));
        let mut out = Vec::new();
        weather.instances(player, &mut out);
        assert!(out.len() > clouds_only);
    }

    #[test]
    fn weather_names_parse() {
        assert_eq!(Sky::parse("orage"), Some(Sky::Storm));
        assert_eq!(Sky::parse("rain"), Some(Sky::Rain));
        assert_eq!(Sky::parse("grêle"), None);
    }
}
