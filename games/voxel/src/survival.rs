//! La survie (phase O4) : ce qui blesse le joueur, et l'armure qui le protège.
//!
//! Tout passe par [`Vitals`], sans ECS ni fenêtre, donc testable seul :
//! - [`Vitals::step`] reçoit à chaque pas ce que voit la physique ([`Senses`]) et renvoie
//!   les dégâts de l'environnement : chute, noyade, lave, feu ;
//! - [`Vitals::hurt`] applique un coup, d'où qu'il vienne (créature, environnement,
//!   commande) : invulnérabilité juste après un coup, puis armure (sauf pour la chute et
//!   la noyade, que l'armure n'arrête pas).
//!
//! Les valeurs sont dans `assets/core/player.ron` (section `survival`).

use std::path::Path;

use engine_ecs::Events;
use engine_voxel::FLUID_FULL;
use glam::Vec3;
use serde::Deserialize;

use crate::components::{Collider, Health};
use crate::events::{self, PlayerAttacked, PlayerDamaged};
use crate::player::{self, MoveMode};

/// La hauteur d'une cellule de fluide pleine, en bloc (la même qu'au maillage).
const FLUID_SURFACE: f32 = 0.875;
/// À la surface de l'eau, la bouche est un peu sous les yeux : l'œil doit être au moins
/// à cette profondeur pour ne plus respirer.
const BREATH_MARGIN: f32 = 0.15;

/// Les réglages de la survie, lus dans `player.ron`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct SurvivalRules {
    /// Secondes d'invulnérabilité après un coup (sinon une lave touchée à 60 Hz tuerait
    /// en un instant, et un groupe de créatures aussi).
    pub invulnerable: f32,
    /// Réduction des dégâts par point de protection d'armure, et réduction maximale.
    pub armor_per_point: f32,
    pub armor_max: f32,
    /// Hauteur de chute sans dégâts, puis dégâts par bloc de plus.
    pub safe_fall: f32,
    pub fall_damage_per_block: f32,
    /// Secondes d'air sous l'eau, et vitesse de reprise (en secondes d'air par seconde).
    pub air: f32,
    pub air_refill: f32,
    /// Sans air : dégâts toutes les `drown_interval` secondes.
    pub drown_damage: f32,
    pub drown_interval: f32,
    /// Toutes les `lava_interval` secondes dans la lave, dégâts = dégâts du fluide × intervalle.
    pub lava_interval: f32,
    /// Après la lave, le joueur brûle `burn_time` secondes (l'eau l'éteint), à
    /// `burn_damage` toutes les `burn_interval` secondes.
    pub burn_time: f32,
    pub burn_damage: f32,
    pub burn_interval: f32,
}

impl Default for SurvivalRules {
    fn default() -> Self {
        Self {
            invulnerable: 0.5,
            armor_per_point: 0.04,
            armor_max: 0.8,
            safe_fall: 3.0,
            fall_damage_per_block: 1.0,
            air: 15.0,
            air_refill: 5.0,
            drown_damage: 2.0,
            drown_interval: 1.0,
            lava_interval: 0.5,
            burn_time: 4.0,
            burn_damage: 1.0,
            burn_interval: 1.0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlayerFile {
    #[serde(default)]
    survival: SurvivalRules,
}

impl SurvivalRules {
    pub fn load(assets: &Path) -> Self {
        let path = assets.join("core").join("player.ron");
        std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(ron::from_str::<PlayerFile>(&text)?.survival))
            .unwrap_or_else(|err| {
                log::warn!("{} : {err:#} (survie par défaut)", path.display());
                Self::default()
            })
    }
}

/// D'où vient un coup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageCause {
    Mob,
    Fall,
    Drowning,
    Lava,
    Burning,
    /// Une commande (`ctl damage`), pour les tests.
    Command,
}

impl DamageCause {
    /// L'armure n'arrête ni une chute ni la noyade.
    pub fn ignores_armor(self) -> bool {
        matches!(self, Self::Fall | Self::Drowning)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mob => "créature",
            Self::Fall => "chute",
            Self::Drowning => "noyade",
            Self::Lava => "lave",
            Self::Burning => "feu",
            Self::Command => "commande",
        }
    }
}

/// Ce que la physique dit du joueur à ce pas.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Senses {
    /// Hauteur des pieds.
    pub feet_y: f32,
    pub on_ground: bool,
    /// Le corps est dans un liquide (il nage) : une chute s'y amortit.
    pub in_liquid: bool,
    /// Le corps touche de l'eau (un fluide sans dégâts) : elle éteint le feu.
    pub in_water: bool,
    /// La tête est dans un liquide qu'on ne respire pas.
    pub head_under: bool,
    /// Les dégâts par seconde du fluide où il se trouve (la lave), 0 sinon.
    pub fluid_damage: f32,
    /// En vol : ni chute ni noyade.
    pub flying: bool,
}

/// L'état de survie du joueur.
#[derive(Debug, Clone, PartialEq)]
pub struct Vitals {
    /// Secondes d'air restantes.
    pub air: f32,
    /// Secondes de feu restantes (0 : ne brûle pas).
    pub burning: f32,
    /// Secondes d'invulnérabilité restantes.
    invulnerable: f32,
    /// Le point le plus haut atteint depuis que les pieds ont quitté le sol.
    fall_peak: Option<f32>,
    /// Le temps écoulé depuis le dernier coup de chaque source continue.
    drown_timer: f32,
    lava_timer: f32,
    burn_timer: f32,
    /// Secondes depuis le dernier coup reçu (pour le HUD).
    pub since_hit: f32,
}

impl Vitals {
    pub fn new(rules: &SurvivalRules) -> Self {
        Self {
            air: rules.air,
            burning: 0.0,
            invulnerable: 0.0,
            fall_peak: None,
            drown_timer: 0.0,
            // Plein : un coup dès qu'on met le pied dans la lave.
            lava_timer: rules.lava_interval,
            burn_timer: 0.0,
            since_hit: f32::INFINITY,
        }
    }

    /// Un pas de simulation : renvoie les coups de l'environnement (dégâts bruts et
    /// cause), à passer ensuite à [`Vitals::hurt`].
    pub fn step(
        &mut self,
        rules: &SurvivalRules,
        senses: Senses,
        dt: f32,
    ) -> Vec<(f32, DamageCause)> {
        let mut hits = Vec::new();
        self.invulnerable = (self.invulnerable - dt).max(0.0);
        self.since_hit += dt;

        // La chute : la hauteur entre le point le plus haut et l'atterrissage. Dans un
        // liquide ou en vol, elle est oubliée.
        if senses.flying || senses.in_liquid {
            self.fall_peak = None;
        } else if senses.on_ground {
            if let Some(peak) = self.fall_peak.take() {
                // La hauteur est relevée à chaque pas, un peu après le vrai départ : une
                // tolérance de 0,1 bloc évite de perdre un point à chaque chute.
                let height = peak - senses.feet_y;
                let damage =
                    ((height - rules.safe_fall - 0.1) * rules.fall_damage_per_block).ceil();
                if damage > 0.0 {
                    hits.push((damage, DamageCause::Fall));
                }
            }
        } else {
            let peak = self.fall_peak.get_or_insert(senses.feet_y);
            *peak = peak.max(senses.feet_y);
        }

        // L'air : il baisse la tête sous l'eau, remonte vite dehors.
        if senses.head_under && !senses.flying {
            self.air = (self.air - dt).max(0.0);
            if self.air <= 0.0 {
                self.drown_timer += dt;
                if self.drown_timer >= rules.drown_interval {
                    self.drown_timer -= rules.drown_interval;
                    hits.push((rules.drown_damage, DamageCause::Drowning));
                }
            }
        } else {
            self.air = (self.air + rules.air_refill * dt).min(rules.air);
            self.drown_timer = 0.0;
        }

        // La lave brûle tant qu'on y est, puis on continue de brûler un moment ; l'eau
        // éteint le feu.
        if senses.fluid_damage > 0.0 {
            self.burning = rules.burn_time;
            self.lava_timer += dt;
            if self.lava_timer >= rules.lava_interval {
                self.lava_timer -= rules.lava_interval;
                hits.push((senses.fluid_damage * rules.lava_interval, DamageCause::Lava));
            }
        } else {
            self.lava_timer = rules.lava_interval; // Un coup dès qu'on y remet le pied.
            if senses.in_water {
                self.burning = 0.0;
            }
            if self.burning > 0.0 {
                self.burning = (self.burning - dt).max(0.0);
                self.burn_timer += dt;
                if self.burn_timer >= rules.burn_interval {
                    self.burn_timer -= rules.burn_interval;
                    hits.push((rules.burn_damage, DamageCause::Burning));
                }
            } else {
                self.burn_timer = 0.0;
            }
        }
        hits
    }

    /// Un coup : renvoie les dégâts vraiment subis (après l'armure), ou `None` s'il
    /// tombe pendant l'invulnérabilité. `protection` : le total de l'armure portée.
    pub fn hurt(
        &mut self,
        rules: &SurvivalRules,
        amount: f32,
        cause: DamageCause,
        protection: u32,
    ) -> Option<f32> {
        if amount <= 0.0 || self.invulnerable > 0.0 {
            return None;
        }
        let reduction = if cause.ignores_armor() {
            0.0
        } else {
            (protection as f32 * rules.armor_per_point).min(rules.armor_max)
        };
        self.invulnerable = rules.invulnerable;
        self.since_hit = 0.0;
        Some(amount * (1.0 - reduction))
    }

    /// Après la mort : de l'air, pas de feu, pas de chute en cours.
    pub fn reset(&mut self, rules: &SurvivalRules) {
        *self = Self::new(rules);
    }
}

/// Le branchement dans le jeu : ce que voit la physique, et les coups appliqués à la vie.
impl crate::VoxelGame {
    /// Un pas de survie : les coups de l'environnement et ceux des créatures.
    pub(crate) fn update_vitals(&mut self, dt: f32) {
        let senses = self.senses();
        let hits = self.vitals.step(&self.survival, senses, dt);
        let attacks: Vec<f32> = self
            .readers
            .attacked
            .read(&self.ecs.resource::<Events<PlayerAttacked>>())
            .map(|e| e.amount)
            .collect();
        for (amount, cause) in hits {
            self.hurt_player(amount, cause);
        }
        for amount in attacks {
            self.hurt_player(amount, DamageCause::Mob);
        }
    }

    /// Ce que la physique dit du joueur.
    fn senses(&self) -> Senses {
        let body = player::body(&self.ecs, self.player);
        let collider = self
            .ecs
            .get::<Collider>(self.player)
            .map(|c| *c)
            .unwrap_or_default();
        let eye = player::eye(&self.ecs, self.player, 1.0);
        let streamer = self.streamer();
        let world = streamer.world();
        let fluid_at = |p: Vec3| self.fluids.kind_of(world.block(p.floor().as_ivec3()));
        // Les fluides autour du corps (les pieds, puis le torse) : la lave blesse, l'eau
        // éteint le feu. Lus dans les blocs : l'état de la physique ne suit pas en vol.
        let body_fluids: Vec<f32> = [0.2, 1.2]
            .into_iter()
            .filter_map(|dy| fluid_at(body.position + Vec3::Y * dy))
            .map(|kind| kind.damage)
            .collect();
        let fluid_damage = body_fluids.iter().copied().fold(0.0, f32::max);
        let in_water = body_fluids.iter().any(|&damage| damage <= 0.0);
        // La tête est sous l'eau si l'œil est sous la surface de sa cellule : comme au
        // maillage, une cellule pleine monte aux 7/8 du bloc ; tout le bloc si un fluide ou
        // un bloc plein est au-dessus (pas de poche d'air sous un plafond). Avec une marge :
        // à la surface libre, on nage la bouche hors de l'eau.
        let cell = eye.floor().as_ivec3();
        let above = cell + glam::IVec3::Y;
        let surface = if fluid_at(eye).is_none() {
            None
        } else if fluid_at(eye + Vec3::Y).is_some() || world.is_solid(above) {
            Some(cell.y as f32 + 1.0)
        } else {
            let level = streamer
                .fluid_amount(cell)
                .map_or(1.0, |a| (a as f32 / FLUID_FULL as f32).min(1.0));
            Some(cell.y as f32 + level * FLUID_SURFACE)
        };
        Senses {
            feet_y: body.position.y,
            on_ground: collider.on_ground,
            in_liquid: collider.in_liquid,
            in_water,
            head_under: surface.is_some_and(|s| eye.y + BREATH_MARGIN < s),
            fluid_damage,
            flying: player::mode(&self.ecs, self.player) == MoveMode::Fly,
        }
    }

    /// Un coup au joueur : invulnérabilité, armure, puis la vie et l'événement
    /// [`PlayerDamaged`] (le son, les mods). Renvoie les dégâts vraiment subis.
    pub(crate) fn hurt_player(&mut self, amount: f32, cause: DamageCause) -> Option<f32> {
        let protection = self.inventory.protection(&self.content.items);
        let damage = self
            .vitals
            .hurt(&self.survival, amount, cause, protection)?;
        let left = {
            let mut health = self.ecs.get_mut::<Health>(self.player)?;
            health.current = (health.current - damage).max(0.0);
            health.current
        };
        log::debug!("{} : -{damage:.1} ({left:.1} restants)", cause.name());
        events::send(
            &self.ecs,
            PlayerDamaged {
                amount: damage,
                health_left: left,
            },
        );
        Some(damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn run(vitals: &mut Vitals, senses: Senses, seconds: f32) -> Vec<(f32, DamageCause)> {
        let rules = SurvivalRules::default();
        let steps = (seconds / DT).round() as usize;
        (0..steps)
            .flat_map(|_| vitals.step(&rules, senses, DT))
            .collect()
    }

    fn standing(y: f32) -> Senses {
        Senses {
            feet_y: y,
            on_ground: true,
            ..Senses::default()
        }
    }

    fn falling(y: f32) -> Senses {
        Senses {
            feet_y: y,
            ..Senses::default()
        }
    }

    #[test]
    fn falls_hurt_beyond_three_blocks() {
        let rules = SurvivalRules::default();
        // 9,99 : la hauteur relevée un pas après le départ d'une chute de 10 blocs.
        for (height, expected) in [
            (3.0, None),
            (3.05, None),
            (3.5, Some(1.0)),
            (4.0, Some(1.0)),
            (9.99, Some(7.0)),
            (10.0, Some(7.0)),
        ] {
            let mut v = Vitals::new(&rules);
            v.step(&rules, falling(64.0 + height), DT);
            v.step(&rules, falling(64.5), DT);
            let hits = v.step(&rules, standing(64.0), DT);
            assert_eq!(
                hits.first().map(|h| h.0),
                expected,
                "chute de {height} blocs"
            );
        }
    }

    #[test]
    fn landing_in_water_or_flying_cancels_the_fall() {
        let rules = SurvivalRules::default();
        let mut v = Vitals::new(&rules);
        v.step(&rules, falling(90.0), DT);
        let water = Senses {
            in_liquid: true,
            ..falling(70.0)
        };
        v.step(&rules, water, DT);
        assert!(v.step(&rules, standing(64.0), DT).is_empty());

        v.step(&rules, falling(90.0), DT);
        let flying = Senses {
            flying: true,
            ..falling(90.0)
        };
        v.step(&rules, flying, DT);
        assert!(v.step(&rules, standing(64.0), DT).is_empty());
    }

    #[test]
    fn drowning_starts_when_air_runs_out() {
        let rules = SurvivalRules::default();
        let mut v = Vitals::new(&rules);
        let under = Senses {
            head_under: true,
            in_liquid: true,
            ..standing(10.0)
        };
        assert!(run(&mut v, under, 14.9).is_empty(), "15 s d'air");
        let hits = run(&mut v, under, 3.2);
        assert_eq!(hits.len(), 3, "puis un coup par seconde : {hits:?}");
        assert!(hits.iter().all(|h| h == &(2.0, DamageCause::Drowning)));
        // Dehors, l'air revient en 3 secondes.
        run(&mut v, standing(10.0), 3.0);
        assert!((v.air - rules.air).abs() < 0.01);
    }

    #[test]
    fn lava_burns_then_fire_keeps_burning_until_water() {
        let rules = SurvivalRules::default();
        let mut v = Vitals::new(&rules);
        let lava = Senses {
            fluid_damage: 4.0,
            in_liquid: true,
            ..standing(10.0)
        };
        let hits = run(&mut v, lava, 0.99);
        assert_eq!(
            hits.len(),
            2,
            "un coup tout de suite, puis toutes les 0,5 s : {hits:?}"
        );
        assert!(hits.iter().all(|h| h == &(2.0, DamageCause::Lava)));
        // Sorti : le feu continue 4 s (un coup par seconde).
        let burning = run(&mut v, standing(10.0), 5.0);
        assert_eq!(burning.len(), 4, "{burning:?}");
        // De nouveau en feu, puis dans l'eau : éteint.
        run(&mut v, lava, 0.1);
        let water = Senses {
            in_liquid: true,
            in_water: true,
            ..standing(10.0)
        };
        assert!(run(&mut v, water, 2.0).is_empty());
        assert_eq!(v.burning, 0.0);
    }

    #[test]
    fn armor_reduces_hits_but_not_falls() {
        let rules = SurvivalRules::default();
        let mut v = Vitals::new(&rules);
        // 10 points de protection : 40 % de moins.
        assert_eq!(v.hurt(&rules, 5.0, DamageCause::Mob, 10), Some(3.0));
        // Juste après : invulnérable.
        assert_eq!(v.hurt(&rules, 5.0, DamageCause::Mob, 10), None);
        v.step(&rules, standing(0.0), 0.6);
        assert_eq!(v.hurt(&rules, 5.0, DamageCause::Fall, 10), Some(5.0));
        v.step(&rules, standing(0.0), 0.6);
        // Au plus 80 %.
        let hit = v.hurt(&rules, 10.0, DamageCause::Lava, 100).unwrap();
        assert!((hit - 2.0).abs() < 1e-5, "{hit}");
    }

    #[test]
    fn rules_are_read_from_player_ron() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core/player.ron"),
        )
        .unwrap();
        let file: PlayerFile = ron::from_str(&text).unwrap();
        assert!(file.survival.air > 0.0);
    }
}
