//! Le cycle jour/nuit.
//!
//! L'heure est un nombre de 0 à 1 : 0 = minuit, 0,25 = lever du soleil, 0,5 = midi,
//! 0,75 = coucher. Rien n'est recalculé dans le monde quand l'heure change : les blocs
//! gardent leur « lumière du ciel » (0 à 15), et le shader la multiplie par l'intensité
//! du jour. C'est pour ça que Minecraft sépare lumière du ciel et lumière des torches.

use glam::Vec3;

/// En mode accéléré (touche T), le temps passe 40 fois plus vite.
const FAST_FACTOR: f32 = 40.0;
/// La nuit n'est jamais complètement noire : la lune éclaire un peu.
const MOONLIGHT: f32 = 0.1;

const NIGHT_SKY: Vec3 = Vec3::new(0.008, 0.012, 0.035);
const DAY_SKY: Vec3 = Vec3::new(0.40, 0.62, 0.78);
const SUNSET_SKY: Vec3 = Vec3::new(0.64, 0.36, 0.29);
const SUNSET_LIGHT: Vec3 = Vec3::new(1.0, 0.73, 0.49);
const MOON_LIGHT: Vec3 = Vec3::new(0.60, 0.73, 0.91);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TimeSpeed {
    Normal,
    Fast,
    Frozen,
}

impl TimeSpeed {
    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Fast => "accéléré",
            Self::Frozen => "figé",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DayCycle {
    /// L'heure, de 0 à 1.
    pub time: f32,
    /// Durée d'une journée complète, en secondes réelles.
    pub day_length: f32,
    pub speed: TimeSpeed,
    /// Le ciel d'une dimension à lumière fixe : (couleur du ciel, lumière de 0 à 1). `None` :
    /// le cycle du jour.
    fixed_sky: Option<(Vec3, f32)>,
}

impl DayCycle {
    pub fn new(time: f32, day_length: f32) -> Self {
        Self {
            time: time.rem_euclid(1.0),
            day_length: day_length.max(1.0),
            speed: TimeSpeed::Normal,
            fixed_sky: None,
        }
    }

    pub fn update(&mut self, dt: f32) {
        let factor = match self.speed {
            TimeSpeed::Normal => 1.0,
            TimeSpeed::Fast => FAST_FACTOR,
            TimeSpeed::Frozen => 0.0,
        };
        // `rem_euclid` : reste toujours entre 0 et 1, même après minuit.
        self.time = (self.time + dt * factor / self.day_length).rem_euclid(1.0);
    }

    /// Le ciel fixe de la dimension (voir [`DayCycle::set_dimension_sky`]), pour le copier chez la fenêtre.
    pub fn fixed_sky(&self) -> Option<(Vec3, f32)> {
        self.fixed_sky
    }

    /// Une horloge qui reprend celle du serveur (voir `sky_view.rs`).
    pub fn copy_of(
        time: f32,
        day_length: f32,
        speed: TimeSpeed,
        fixed_sky: Option<(Vec3, f32)>,
    ) -> Self {
        Self {
            time,
            day_length,
            speed,
            fixed_sky,
        }
    }

    /// Le ciel d'une dimension : une couleur et une lumière fixes (sans cycle), ou un seul des deux.
    /// `None, None` : le cycle du jour habituel.
    pub fn set_dimension_sky(&mut self, color: Option<Vec3>, daylight: Option<f32>) {
        self.fixed_sky = match (color, daylight) {
            (None, None) => None,
            (color, light) => Some((color.unwrap_or(DAY_SKY), light.unwrap_or(1.0))),
        };
    }

    pub fn has_daylight_cycle(&self) -> bool {
        self.fixed_sky.is_none()
    }

    /// Normal → accéléré → figé → normal.
    pub fn cycle_speed(&mut self) {
        self.speed = match self.speed {
            TimeSpeed::Normal => TimeSpeed::Fast,
            TimeSpeed::Fast => TimeSpeed::Frozen,
            TimeSpeed::Frozen => TimeSpeed::Normal,
        };
    }

    /// Direction vers le soleil. Il se lève à l'est (+x), passe au sud et se couche à l'ouest.
    pub fn sun_direction(&self) -> Vec3 {
        let angle = (self.time - 0.25) * std::f32::consts::TAU;
        Vec3::new(angle.cos(), angle.sin(), 0.35).normalize()
    }

    /// 0 quand le soleil est bien sous l'horizon, 1 quand il est haut.
    fn day_factor(&self) -> f32 {
        smoothstep(-0.12, 0.3, self.sun_direction().y)
    }

    /// La nuit : le soleil est sous l'horizon (on peut dormir).
    pub fn is_night(&self) -> bool {
        if let Some((_, light)) = self.fixed_sky {
            return light < 0.3;
        }
        self.sun_direction().y < 0.0
    }

    /// L'intensité de la lumière du ciel, de [`MOONLIGHT`] (la nuit) à 1.
    pub fn daylight(&self) -> f32 {
        if let Some((_, light)) = self.fixed_sky {
            return light;
        }
        MOONLIGHT + (1.0 - MOONLIGHT) * self.day_factor()
    }

    /// La couleur de la brume à l'horizon. Le shader ajoute la diffusion
    /// directionnelle au lever et au coucher du soleil.
    pub fn sky_color(&self) -> Vec3 {
        if let Some((color, _)) = self.fixed_sky {
            return color;
        }
        let base = NIGHT_SKY.lerp(DAY_SKY, self.day_factor());
        base.lerp(SUNSET_SKY, self.sunset_factor() * 0.75)
    }

    /// La teinte de la lumière du ciel : orangée près de l'horizon, bleutée la nuit.
    pub fn sun_color(&self) -> Vec3 {
        if self.fixed_sky.is_some() {
            return Vec3::ONE;
        }
        let day = Vec3::ONE.lerp(SUNSET_LIGHT, self.sunset_factor() * 0.85);
        MOON_LIGHT.lerp(day, self.day_factor())
    }

    /// 1 quand le soleil touche l'horizon, 0 quand il en est loin (courbe en cloche).
    fn sunset_factor(&self) -> f32 {
        (-(self.sun_direction().y / 0.15).powi(2)).exp()
    }

    /// L'heure sous forme « 14:30 ».
    pub fn clock(&self) -> String {
        let minutes = (self.time * 24.0 * 60.0) as u32;
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn night_is_between_sunset_and_sunrise() {
        let at = |time| DayCycle::new(time, 600.0).is_night();
        assert!(at(0.0) && at(0.2) && at(0.8) && at(0.95));
        assert!(!at(0.3) && !at(0.5) && !at(0.7));
    }

    #[test]
    fn noon_is_bright_and_midnight_is_dark() {
        let noon = DayCycle::new(0.5, 600.0);
        let midnight = DayCycle::new(0.0, 600.0);
        assert!(noon.daylight() > 0.95);
        assert!((midnight.daylight() - MOONLIGHT).abs() < 1e-6);
        assert!(noon.sun_direction().y > 0.9);
        assert!(midnight.sun_direction().y < -0.9);
    }

    #[test]
    fn sunset_sky_is_orange() {
        let sunset = DayCycle::new(0.75, 600.0).sky_color();
        assert!(sunset.x > sunset.z, "plus de rouge que de bleu : {sunset}");
        let noon = DayCycle::new(0.5, 600.0).sky_color();
        assert!(noon.z > noon.x);
    }

    #[test]
    fn time_wraps_around_and_can_be_frozen() {
        let mut day = DayCycle::new(0.9, 10.0);
        day.update(2.0);
        assert!((day.time - 0.1).abs() < 1e-5, "{}", day.time);
        day.cycle_speed();
        day.cycle_speed();
        assert_eq!(day.speed, TimeSpeed::Frozen);
        day.update(5.0);
        assert!((day.time - 0.1).abs() < 1e-5);
        assert_eq!(DayCycle::new(0.5, 600.0).clock(), "12:00");
    }
}
