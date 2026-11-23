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
use crate::events::{self, EntityDied, PlayerAttacked, PlayerDamaged};
use crate::player::attributes::{Pierce, damage_after_armor};
use crate::player::{self, MoveMode};

/// Le recul d'un coup : vitesse horizontale et verticale (blocs par seconde).
const KNOCKBACK: (f32, f32) = (5.0, 4.5);

/// Une créature morte plus loin ne rapporte pas d'expérience.
const XP_RADIUS: f32 = 16.0;

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

/// Les réglages de la faim, lus dans `player.ron` (section `hunger`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct HungerRules {
    /// La jauge de faim pleine, et la saturation d'un nouveau joueur.
    pub max_food: f32,
    pub start_saturation: f32,
    /// La fatigue accumulée retire 1 point (de saturation d'abord, puis de faim) à
    /// chaque fois qu'elle atteint ce seuil.
    pub exhaustion_per_point: f32,
    /// La fatigue de chaque effort.
    pub sprint_per_block: f32,
    pub jump: f32,
    pub mine: f32,
    pub attack: f32,
    pub hurt: f32,
    /// Bien nourri (au moins `heal_food`), le joueur regagne 1 point de vie toutes les
    /// `heal_interval` secondes, au prix de `heal_exhaustion` de fatigue.
    pub heal_food: f32,
    pub heal_interval: f32,
    pub heal_exhaustion: f32,
    /// Affamé (0), il perd `starve_damage` toutes les `starve_interval` secondes.
    pub starve_damage: f32,
    pub starve_interval: f32,
    /// En dessous, plus de course.
    pub sprint_min_food: f32,
    /// Secondes pour manger, bouton tenu.
    pub eat_time: f32,
}

impl Default for HungerRules {
    fn default() -> Self {
        Self {
            max_food: 20.0,
            start_saturation: 5.0,
            exhaustion_per_point: 4.0,
            sprint_per_block: 0.1,
            jump: 0.05,
            mine: 0.005,
            attack: 0.1,
            hurt: 0.1,
            heal_food: 18.0,
            heal_interval: 4.0,
            heal_exhaustion: 6.0,
            starve_damage: 1.0,
            starve_interval: 4.0,
            sprint_min_food: 6.0,
            eat_time: 1.6,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlayerFile {
    #[serde(default)]
    survival: SurvivalRules,
    #[serde(default)]
    hunger: HungerRules,
}

/// Lit `player.ron` (les réglages par défaut s'il manque ou est invalide).
pub fn load_rules(assets: &Path) -> (SurvivalRules, HungerRules) {
    let path = assets.join("core").join("player.ron");
    std::fs::read_to_string(&path)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(ron::from_str::<PlayerFile>(&text)?))
        .map(|file| (file.survival, file.hunger))
        .unwrap_or_else(|err| {
            log::warn!("{} : {err:#} (survie par défaut)", path.display());
            (SurvivalRules::default(), HungerRules::default())
        })
}

/// L'expérience du joueur : des points, qui font des niveaux de plus en plus chers
/// (7 points pour le niveau 1, puis 2 de plus à chaque niveau). Elle servira à
/// l'enchantement (phase O10) ; elle est perdue à la mort.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Experience {
    /// Le total des points gagnés.
    pub points: u32,
}

impl Experience {
    /// Les points qu'il faut pour passer du niveau `level` au suivant.
    pub fn cost(level: u32) -> u32 {
        7 + 2 * level
    }

    /// Le niveau atteint, et les points déjà gagnés vers le suivant.
    pub fn level(self) -> (u32, u32) {
        let mut level = 0;
        let mut left = self.points;
        while left >= Self::cost(level) {
            left -= Self::cost(level);
            level += 1;
        }
        (level, left)
    }

    /// L'avancement vers le niveau suivant, de 0 à 1 (la barre du HUD).
    pub fn progress(self) -> f32 {
        let (level, into) = self.level();
        into as f32 / Self::cost(level) as f32
    }

    /// Dépense `levels` niveaux (l'avancement vers le suivant est gardé). `false`, sans rien
    /// dépenser, s'il n'y en a pas assez.
    pub fn spend_levels(&mut self, levels: u32) -> bool {
        let (level, into) = self.level();
        if level < levels {
            return false;
        }
        let target: u32 = (0..level - levels).map(Self::cost).sum();
        self.points = target + into.min(Self::cost(level - levels).saturating_sub(1));
        true
    }

    /// Ajoute des points ; renvoie le nouveau niveau s'il a changé.
    pub fn add(&mut self, points: u32) -> Option<u32> {
        let before = self.level().0;
        self.points = self.points.saturating_add(points);
        let after = self.level().0;
        (after > before).then_some(after)
    }
}

/// La faim du joueur.
#[derive(Debug, Clone, PartialEq)]
pub struct Hunger {
    /// De 0 à `max_food`.
    pub food: f32,
    /// Une réserve, jamais plus haute que `food`, qui s'épuise d'abord.
    pub saturation: f32,
    /// La fatigue accumulée, de 0 à `exhaustion_per_point`.
    pub exhaustion: f32,
    heal_timer: f32,
    starve_timer: f32,
    /// Depuis combien de temps il mange (bouton tenu), s'il mange.
    pub chewing: Option<f32>,
}

/// Ce que la faim fait à la vie pendant un pas.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HungerEffect {
    /// Points de vie regagnés.
    pub heal: f32,
    /// Points de vie perdus (affamé).
    pub starve: f32,
}

impl Hunger {
    pub fn new(rules: &HungerRules) -> Self {
        Self {
            food: rules.max_food,
            saturation: rules.start_saturation.min(rules.max_food),
            exhaustion: 0.0,
            heal_timer: 0.0,
            starve_timer: 0.0,
            chewing: None,
        }
    }

    /// Le bouton tenu (`holding`) avec un aliment en main : renvoie `true` quand
    /// l'aliment est mangé. Lâcher le bouton, ou être rassasié, interrompt le repas.
    pub fn chew(&mut self, rules: &HungerRules, holding: bool, dt: f32) -> bool {
        if !holding || !self.hungry(rules) {
            self.chewing = None;
            return false;
        }
        let time = self.chewing.get_or_insert(0.0);
        *time += dt;
        if *time >= rules.eat_time {
            self.chewing = None;
            return true;
        }
        false
    }

    /// Un effort : la fatigue monte, et chaque seuil atteint retire un point.
    pub fn exert(&mut self, rules: &HungerRules, amount: f32) {
        self.exhaustion += amount.max(0.0);
        while self.exhaustion >= rules.exhaustion_per_point {
            self.exhaustion -= rules.exhaustion_per_point;
            if self.saturation > 0.0 {
                self.saturation = (self.saturation - 1.0).max(0.0);
            } else {
                self.food = (self.food - 1.0).max(0.0);
            }
        }
    }

    /// Un pas : régénération si bien nourri et blessé, dégâts si affamé.
    pub fn step(
        &mut self,
        rules: &HungerRules,
        health: f32,
        max_health: f32,
        dt: f32,
    ) -> HungerEffect {
        let mut effect = HungerEffect::default();
        if self.food >= rules.heal_food && health < max_health {
            self.heal_timer += dt;
            if self.heal_timer >= rules.heal_interval {
                self.heal_timer -= rules.heal_interval;
                effect.heal = 1.0;
                self.exert(rules, rules.heal_exhaustion);
            }
        } else {
            self.heal_timer = 0.0;
        }
        if self.food <= 0.0 {
            self.starve_timer += dt;
            if self.starve_timer >= rules.starve_interval {
                self.starve_timer -= rules.starve_interval;
                effect.starve = rules.starve_damage;
            }
        } else {
            self.starve_timer = 0.0;
        }
        effect
    }

    /// Mange un aliment : la faim et la saturation montent (la saturation jamais
    /// au-dessus de la faim).
    pub fn eat(&mut self, rules: &HungerRules, nutrition: f32, saturation: f32) {
        self.food = (self.food + nutrition).min(rules.max_food);
        self.saturation = (self.saturation + saturation).min(self.food);
    }

    /// Peut-il manger ? (pas quand il est rassasié)
    pub fn hungry(&self, rules: &HungerRules) -> bool {
        self.food < rules.max_food
    }

    pub fn can_sprint(&self, rules: &HungerRules) -> bool {
        self.food > rules.sprint_min_food
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
    /// La faim.
    Starvation,
}

impl DamageCause {
    /// L'armure n'arrête ni une chute ni la noyade.
    pub fn ignores_armor(self) -> bool {
        matches!(self, Self::Fall | Self::Drowning | Self::Starvation)
    }

    /// La phrase de l'écran de mort.
    pub fn death_message(self) -> &'static str {
        match self {
            Self::Mob => "Tué par une créature.",
            Self::Fall => "Tombé de trop haut.",
            Self::Drowning => "Noyé.",
            Self::Lava => "Brûlé dans la lave.",
            Self::Burning => "Mort dans les flammes.",
            Self::Command => "Tué par une commande.",
            Self::Starvation => "Mort de faim.",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mob => "créature",
            Self::Fall => "chute",
            Self::Drowning => "noyade",
            Self::Lava => "lave",
            Self::Burning => "feu",
            Self::Command => "commande",
            Self::Starvation => "faim",
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
    /// Le nombre de coups reçus : le client fait rougir l'écran quand il change.
    pub hits: u32,
    /// La cause du dernier coup reçu : le message de l'écran de mort.
    pub last_cause: Option<DamageCause>,
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
            hits: 0,
            last_cause: None,
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
    /// tombe pendant l'invulnérabilité. `armor` : les points d'armure portés ; `pierce` : ce que
    /// l'attaquant en contourne (voir `attributes::damage_after_armor`). L'armure est la seule
    /// réduction, et elle ne compte ni pour une chute ni pour la noyade.
    pub fn hurt(
        &mut self,
        rules: &SurvivalRules,
        amount: f32,
        cause: DamageCause,
        armor: f32,
        pierce: Pierce,
    ) -> Option<f32> {
        if amount <= 0.0 || self.invulnerable > 0.0 {
            return None;
        }
        let dealt = if cause.ignores_armor() {
            amount
        } else {
            damage_after_armor(
                amount,
                armor,
                pierce,
                rules.armor_per_point,
                rules.armor_max,
            )
        };
        self.invulnerable = rules.invulnerable;
        self.since_hit = 0.0;
        self.hits = self.hits.wrapping_add(1);
        self.last_cause = Some(cause);
        Some(dealt)
    }
}

/// Le branchement dans le jeu : ce que voit la physique, et les coups appliqués à la vie.
impl crate::server::VoxelServer {
    /// Un pas de survie : les coups de l'environnement et ceux des créatures.
    pub fn update_vitals(&mut self, dt: f32) {
        let senses = self.senses();
        let hits = self
            .players
            .me_mut()
            .vitals
            .step(&self.survival, senses, dt);
        // Seuls les coups reçus par le joueur local se jouent ici : les autres joueurs n'ont pas
        // encore de simulation de survie (les événements portent déjà leur numéro).
        let local = self.players.local_id();
        let attacks: Vec<(f32, Vec3, f32)> = self
            .readers
            .attacked
            .read(&self.ecs.resource::<Events<PlayerAttacked>>())
            .filter(|e| e.player.is_none_or(|id| id == local))
            .map(|e| (e.amount, e.from, e.pierce))
            .collect();
        for (amount, cause) in hits {
            self.hurt_player(amount, cause);
        }
        for (amount, from, pierce) in attacks {
            if self
                .hurt_player_pierced(amount, DamageCause::Mob, Pierce::flat(pierce))
                .is_some()
            {
                self.knock_back(from);
                self.reflect_thorns(from);
            }
        }
    }

    /// Ce que la physique dit du joueur.
    fn senses(&self) -> Senses {
        let body = player::body(&self.ecs, self.players.entity());
        let collider = self
            .ecs
            .get::<Collider>(self.players.entity())
            .map(|c| *c)
            .unwrap_or_default();
        let eye = player::eye(&self.ecs, self.players.entity(), 1.0);
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
            flying: player::mode(&self.ecs, self.players.entity()) == MoveMode::Fly,
        }
    }

    /// La faim d'un pas : régénération si bien nourri, dégâts si affamé.
    pub fn update_hunger(&mut self, dt: f32) {
        let health = player::health(&self.ecs, self.players.entity());
        let effect =
            self.players
                .me_mut()
                .hunger
                .step(&self.hunger_rules, health.current, health.max, dt);
        if effect.heal > 0.0
            && let Some(mut health) = self.ecs.get_mut::<Health>(self.players.entity())
        {
            health.current = (health.current + effect.heal).min(health.max);
        }
        if effect.starve > 0.0 {
            self.hurt_player(effect.starve, DamageCause::Starvation);
        }
    }

    /// Un effort (courir, sauter, miner, frapper) : la faim s'en ressent.
    pub fn exert(&mut self, amount: f32) {
        self.players
            .me_mut()
            .hunger
            .exert(&self.hunger_rules, amount);
    }

    /// La fatigue des déplacements d'un pas : la course au sol, et chaque saut.
    pub fn exert_movement(&mut self, walk: Vec3, jump: bool, sprinting: bool, dt: f32) {
        if player::mode(&self.ecs, self.players.entity()) == MoveMode::Fly {
            return;
        }
        let on_ground = self
            .ecs
            .get::<Collider>(self.players.entity())
            .is_some_and(|c| c.on_ground);
        if !on_ground {
            return;
        }
        let rules = &self.hunger_rules;
        let mut effort = 0.0;
        if sprinting {
            effort += rules.sprint_per_block * walk.length() * dt;
        }
        if jump {
            effort += rules.jump;
        }
        self.exert(effort);
    }

    /// Peut-il courir ? (pas affamé)
    pub fn can_sprint(&self) -> bool {
        self.players.me().hunger.can_sprint(&self.hunger_rules)
    }

    /// Manger : le bouton droit tenu avec un aliment en main. Renvoie l'avancement
    /// (0 à 1) pour la barre, ou `None`.
    pub fn update_eating(&mut self, holding: bool, dt: f32) -> Option<f32> {
        let food = self
            .players
            .me_mut()
            .inventory
            .selected_item()
            .and_then(|item| self.content.items.get(item).food);
        let Some(food) = food else {
            self.players.me_mut().hunger.chewing = None;
            return None;
        };
        if self
            .players
            .me_mut()
            .hunger
            .chew(&self.hunger_rules, holding, dt)
        {
            let eaten = self.players.me_mut().inventory.consume_selected();
            self.players
                .me_mut()
                .hunger
                .eat(&self.hunger_rules, food.nutrition, food.saturation);
            if let Some(potion) = eaten.and_then(|item| self.potions.get(item)) {
                // Une potion : l'effet démarre, la fiole vide revient.
                let healed = self.players.me_mut().effects.drink(potion);
                if healed > 0.0 {
                    self.heal(healed);
                }
                self.toast(potion.effect.name());
                self.give_back_flask();
                self.sound("core:gulp");
            } else {
                self.sound("core:place_soft");
            }
            log::debug!(
                "miam : faim {:.1}, saturation {:.1}",
                self.players.me().hunger.food,
                self.players.me().hunger.saturation
            );
        }
        self.players
            .me_mut()
            .hunger
            .chewing
            .map(|t| (t / self.hunger_rules.eat_time).clamp(0.0, 1.0))
    }

    /// La fiole vide d'une potion bue.
    fn give_back_flask(&mut self) {
        let Some(flask) = self.content.items.id("core:flask") else {
            return;
        };
        let items = self.content.items.clone();
        let left = self
            .players
            .me_mut()
            .inventory
            .add_stack(crate::player::inventory::ItemStack::new(flask, 1), &items);
        if left > 0 {
            let at = player::body(&self.ecs, self.players.entity())
                .position
                .floor()
                .as_ivec3();
            self.drop_stack(at, crate::player::inventory::ItemStack::new(flask, left));
        }
    }

    /// Le butin des créatures mortes, tombé là où elles sont mortes.
    pub fn drop_mob_loot(&mut self) {
        let died: Vec<_> = self
            .readers
            .loot
            .read(&self.ecs.resource::<Events<EntityDied>>())
            .map(|e| {
                (
                    e.kind.clone(),
                    e.position,
                    e.elite.clone(),
                    e.coat,
                    e.shorn,
                    e.killer,
                )
            })
            .collect();
        let player_at = player::body(&self.ecs, self.players.entity()).position;
        let me = self.players.local_id();
        for (kind, position, elite, coat, shorn, killer) in died {
            let (drops, xp, hostile, boss) = self
                .ecs
                .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                .by_key(&kind)
                .map(|def| (def.drops.clone(), def.xp, def.hostile, def.boss))
                .unwrap_or_default();
            // L'expérience, si le joueur était là (pas pour une créature morte au loin) et que la
            // créature n'a pas été tuée par un autre joueur.
            if position.distance(player_at) <= XP_RADIUS && killer.is_none_or(|k| k == me) {
                let points = self.roll_range(xp);
                self.grant_xp(points);
                // La progression d'aventure, et le butin à affixes (voir `adventure.rs`).
                if hostile {
                    self.credit_kill(boss);
                }
                self.adventure_loot(position, hostile, elite.as_ref());
            }
            for drop in drops {
                let mut rng = engine_voxel::Rng64::new(self.next_random());
                if rng.next_f32() >= drop.chance {
                    continue;
                }
                // Un animal tondu n'a plus de laine à laisser.
                if shorn && drop.item == "core:wool" {
                    continue;
                }
                let key = crate::mobs::sheep_colors::drop_key(&drop.item, coat);
                let Some(item) = self.content.items.id(key) else {
                    log::warn!("butin de {kind} : objet « {key} » inconnu");
                    continue;
                };
                let (min, max) = drop.count;
                let span = max.saturating_sub(min) + 1;
                let count = min + (rng.next_u64() % u64::from(span)) as u32;
                // La chance de butin du joueur ajoute des objets (voir `player/attributes.rs`).
                let count = self.loot_count(count);
                // Aspiration : le butin d'une créature tuée près du joueur va dans l'inventaire.
                let near = position.distance(player_at) <= XP_RADIUS;
                self.give_loot(
                    position.floor().as_ivec3(),
                    crate::player::inventory::ItemStack::new(item, count),
                    near,
                );
            }
        }
    }

    /// Un nombre au hasard entre min et max (inclus).
    pub fn roll_range(&mut self, (min, max): (u32, u32)) -> u32 {
        if max <= min {
            return min;
        }
        let mut rng = engine_voxel::Rng64::new(self.next_random());
        min + (rng.next_u64() % u64::from(max - min + 1)) as u32
    }

    /// Gagne de l'expérience ; un message et un son à chaque niveau.
    pub fn grant_xp(&mut self, points: u32) {
        if points == 0 {
            return;
        }
        let points = self.scaled_xp(points);
        if points == 0 {
            return;
        }
        if let Some(level) = self.players.me_mut().experience.add(points) {
            self.toast(&format!("Niveau {level}"));
            self.sound("core:break_glass");
        }
    }

    /// Un coup venu de `from` : le joueur recule (pas en vol).
    fn knock_back(&mut self, from: Vec3) {
        if player::mode(&self.ecs, self.players.entity()) == MoveMode::Fly {
            return;
        }
        let at = player::body(&self.ecs, self.players.entity()).position;
        let away = Vec3::new(at.x - from.x, 0.0, at.z - from.z).normalize_or_zero();
        self.push_player(KNOCKBACK.1, away * KNOCKBACK.0);
    }

    /// Un coup au joueur : invulnérabilité, armure, puis la vie et l'événement
    /// [`PlayerDamaged`] (le son, les mods). Renvoie les dégâts vraiment subis.
    pub fn hurt_player(&mut self, amount: f32, cause: DamageCause) -> Option<f32> {
        self.hurt_player_pierced(amount, cause, Pierce::NONE)
    }

    /// Comme [`Self::hurt_player`], pour un coup qui contourne une partie de l'armure (la
    /// perforation d'une créature, `ctl damage n perce`).
    pub fn hurt_player_pierced(
        &mut self,
        amount: f32,
        cause: DamageCause,
        pierce: Pierce,
    ) -> Option<f32> {
        // Légèreté : une part des dégâts de chute est évitée.
        let amount = self.environment_damage(amount, cause);
        // En difficile, les créatures frappent plus fort.
        let amount = if cause == DamageCause::Mob && self.difficulty >= 2 {
            amount * 1.5
        } else {
            amount
        };
        // La résistance au feu : ni la lave ni les flammes ne blessent.
        if matches!(cause, DamageCause::Lava | DamageCause::Burning)
            && self
                .players
                .me_mut()
                .effects
                .has(crate::player::potions::Effect::FireResistance)
        {
            return None;
        }
        let armor = self.players.me_mut().gear.armor();
        let damage =
            self.players
                .me_mut()
                .vitals
                .hurt(&self.survival, amount, cause, armor, pierce)?;
        // L'armure qui a encaissé s'use ; une pièce peut se briser.
        if armor > 0.0 && !cause.ignores_armor() {
            let roll = engine_voxel::Rng64::new(self.next_random()).next_f32();
            for item in self
                .players
                .me_mut()
                .inventory
                .wear_armor(&self.content.items, roll)
            {
                let name = self.content.items.get(item).name.clone();
                self.toast(&format!("{name} : brisé"));
                self.sound("core:break_glass");
            }
        }
        let left = {
            let mut health = self.ecs.get_mut::<Health>(self.players.entity())?;
            health.current = (health.current - damage).max(0.0);
            health.current
        };
        log::debug!("{} : -{damage:.1} ({left:.1} restants)", cause.name());
        self.exert(self.hunger_rules.hurt);
        events::send(
            &self.ecs,
            PlayerDamaged {
                player: Some(self.players.local_id()),
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
        let none = Pierce::NONE;
        assert_eq!(v.hurt(&rules, 5.0, DamageCause::Mob, 10.0, none), Some(3.0));
        // Juste après : invulnérable.
        assert_eq!(v.hurt(&rules, 5.0, DamageCause::Mob, 10.0, none), None);
        v.step(&rules, standing(0.0), 0.6);
        assert_eq!(
            v.hurt(&rules, 5.0, DamageCause::Fall, 10.0, none),
            Some(5.0)
        );
        v.step(&rules, standing(0.0), 0.6);
        // Perforation : 5 des 10 points sont ignorés, donc 20 % de moins seulement.
        let pierced = v
            .hurt(&rules, 5.0, DamageCause::Mob, 10.0, Pierce::flat(5.0))
            .unwrap();
        assert!((pierced - 4.0).abs() < 1e-5, "{pierced}");
        v.step(&rules, standing(0.0), 0.6);
        // Au plus 80 %.
        let hit = v
            .hurt(&rules, 10.0, DamageCause::Lava, 100.0, none)
            .unwrap();
        assert!((hit - 2.0).abs() < 1e-5, "{hit}");
    }

    #[test]
    fn exhaustion_eats_saturation_then_food() {
        let rules = HungerRules::default();
        let mut h = Hunger::new(&rules);
        assert_eq!((h.food, h.saturation), (20.0, 5.0));
        // 40 blocs de course : 4 de fatigue, un point de saturation.
        h.exert(&rules, 40.0 * rules.sprint_per_block);
        assert_eq!((h.food, h.saturation), (20.0, 4.0));
        // 5 points de plus : la saturation (4) s'épuise, puis la faim perd 1.
        h.exert(&rules, 5.0 * rules.exhaustion_per_point);
        assert_eq!((h.food, h.saturation), (19.0, 0.0));
    }

    #[test]
    fn well_fed_heals_and_starving_hurts() {
        let rules = HungerRules::default();
        let mut h = Hunger::new(&rules);
        let mut healed = 0.0;
        for _ in 0..(8.5 / (1.0 / 60.0)) as usize {
            healed += h.step(&rules, 10.0, 20.0, 1.0 / 60.0).heal;
        }
        assert_eq!(healed, 2.0, "un point toutes les 4 s");
        assert!(h.saturation < 5.0, "la régénération fatigue");
        // Pleine vie : pas de régénération.
        assert_eq!(h.step(&rules, 20.0, 20.0, 5.0).heal, 0.0);
        // Affamé : un coup toutes les 4 s.
        h.food = 0.0;
        h.saturation = 0.0;
        let mut starve = 0.0;
        for _ in 0..(8.5 / (1.0 / 60.0)) as usize {
            starve += h.step(&rules, 10.0, 20.0, 1.0 / 60.0).starve;
        }
        assert_eq!(starve, 2.0);
        assert!(!h.can_sprint(&rules));
    }

    #[test]
    fn eating_fills_food_then_saturation_capped_by_food() {
        let rules = HungerRules::default();
        let mut h = Hunger::new(&rules);
        h.food = 5.0;
        h.saturation = 0.0;
        assert!(h.hungry(&rules));
        h.eat(&rules, 8.0, 12.8);
        assert_eq!(h.food, 13.0);
        assert_eq!(h.saturation, 12.8);
        h.eat(&rules, 8.0, 12.8);
        assert_eq!(h.food, 20.0);
        assert_eq!(h.saturation, 20.0, "jamais plus que la faim");
        assert!(!h.hungry(&rules));
    }

    #[test]
    fn eating_takes_time_and_stops_when_released() {
        let rules = HungerRules::default();
        let mut h = Hunger::new(&rules);
        h.food = 10.0;
        assert!(!h.chew(&rules, true, 1.0));
        assert!(!h.chew(&rules, false, 0.1), "lâché : on recommence");
        assert_eq!(h.chewing, None);
        assert!(!h.chew(&rules, true, 1.0));
        assert!(h.chew(&rules, true, 0.7), "1,6 s tenu");
        h.food = rules.max_food;
        assert!(!h.chew(&rules, true, 2.0), "rassasié : on ne mange pas");
    }

    #[test]
    fn levels_cost_more_and_more() {
        assert_eq!(Experience { points: 0 }.level(), (0, 0));
        assert_eq!(Experience { points: 6 }.level(), (0, 6));
        assert_eq!(Experience { points: 7 }.level(), (1, 0));
        // 7 + 9 + 11 = 27 points pour le niveau 3.
        assert_eq!(Experience { points: 27 }.level(), (3, 0));
        assert_eq!(Experience { points: 30 }.level(), (3, 3));
        assert!((Experience { points: 30 }.progress() - 3.0 / 13.0).abs() < 1e-6);
        let mut xp = Experience::default();
        assert_eq!(xp.add(5), None);
        assert_eq!(xp.add(25), Some(3), "plusieurs niveaux d'un coup");
    }

    #[test]
    fn rules_are_read_from_player_ron() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core/player.ron"),
        )
        .unwrap();
        let file: PlayerFile = ron::from_str(&text).unwrap();
        assert!(file.survival.air > 0.0);
        assert!(file.hunger.max_food > 0.0);
    }
}
