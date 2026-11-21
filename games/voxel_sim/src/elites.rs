//! Les élites et les envahisseurs : des hostiles renforcées, au butin garanti.
//!
//! # Les élites
//!
//! À la naissance d'une hostile (voir `spawning.rs`), le palier du monde tire au sort si elle est
//! une **élite** (`elite_chance` de `tiers.ron`). Une élite a une à trois **augmentations**
//! (`elites.ron` : vie, dégâts, vitesse, armure, régénération), tirées au poids parmi celles que le
//! palier permet. Les effets se multiplient ; le nom se compose (« Rôdeur colosse et éclair »), la
//! créature prend la teinte des augmentations, et sa mort laisse un objet à affixes garanti.
//! Un boss (`boss: true`) et une créature dirigée par un script ne deviennent jamais élites.
//!
//! # Les envahisseurs
//!
//! À partir du palier 2, un envahisseur surgit à intervalles réguliers (`invader` de `tiers.ron`)
//! non loin du joueur. Il est annoncé par un **son** et un **faisceau lumineux** (une colonne de
//! lumière qui monte au ciel et se voit de loin), a deux fois plus d'augmentations qu'une élite, plus
//! de vie, et lâche un objet à affixes d'une rareté plus haute, une gemme et de la poussière.
//!
//! # Ce que lit la simulation
//!
//! Le composant [`Elite`] porte les multiplicateurs ; la vie est déjà multipliée à la promotion
//! ([`promote`]). `mobs.rs` lit [`damage_of`], [`speed_of`] et [`armor_of`] ; [`regen`] est un
//! système de la simulation. Le jeu lit [`Elite`] à la mort (`EntityDied::elite`) pour le butin.

use std::sync::Arc;

use engine_ecs::{Entity, World};
use engine_voxel::Rng64;
use glam::Vec3;

use crate::adventure_data::{AdventureData, Augmentation, Effect};
use crate::components::Health;
use crate::mobs::{FixedDt, GameRng, MobRegistry};
use crate::tiers::TierEffects;

/// Une hostile renforcée.
#[derive(Debug, Clone, PartialEq)]
pub struct Elite {
    /// Son nom complet (« Rôdeur envahisseur colosse »).
    pub name: String,
    /// Les identifiants de ses augmentations.
    pub augmentations: Vec<String>,
    /// La teinte qui se mêle à sa couleur.
    pub tint: Vec3,
    pub damage: f32,
    pub speed: f32,
    /// Des points d'armure en plus.
    pub armor: f32,
    /// Des points de vie rendus par seconde.
    pub regen: f32,
    pub invader: bool,
}

/// Tire `count` augmentations différentes, au poids, parmi celles que ce palier permet.
pub fn pick_augmentations<'a>(
    data: &'a AdventureData,
    tier: u8,
    count: usize,
    rng: &mut Rng64,
) -> Vec<&'a Augmentation> {
    let mut pool: Vec<&Augmentation> = data
        .augmentations
        .iter()
        .filter(|a| a.min_tier <= tier && a.weight > 0)
        .collect();
    let mut out = Vec::new();
    for _ in 0..count.min(pool.len()) {
        let total: u32 = pool.iter().map(|a| a.weight).sum();
        let mut pick = rng.below(total.max(1));
        let index = pool
            .iter()
            .position(|a| {
                if pick < a.weight {
                    true
                } else {
                    pick -= a.weight;
                    false
                }
            })
            .unwrap_or(0);
        out.push(pool.remove(index));
    }
    out
}

/// Combien d'augmentations tire une élite à ce palier : une, puis une de plus tous les deux
/// paliers, dans la limite des données. Un envahisseur en a le double.
pub fn picks_for(data: &AdventureData, tier: u8, invader: bool) -> usize {
    let base = 1 + usize::from(tier) / 2;
    let count = if invader { base * 2 } else { base };
    count.min(data.max_picks.max(1) as usize * if invader { 2 } else { 1 })
}

/// « colosse », « colosse et éclair », « colosse, éclair et bastion ».
fn join_titles(titles: &[&str]) -> String {
    match titles {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} et {last}", rest.join(", ")),
    }
}

/// Construit l'élite : ses multiplicateurs, son nom et sa teinte. Renvoie aussi le facteur de vie
/// (déjà compris les envahisseurs), à appliquer à la vie de la créature.
pub fn build(
    data: &AdventureData,
    base_name: &str,
    picks: &[&Augmentation],
    invader: bool,
) -> (Elite, f32) {
    let mut elite = Elite {
        name: String::new(),
        augmentations: picks.iter().map(|a| a.id.clone()).collect(),
        tint: Vec3::ZERO,
        damage: 1.0,
        speed: 1.0,
        armor: 0.0,
        regen: 0.0,
        invader,
    };
    let mut health = if invader { data.invader_health } else { 1.0 };
    for aug in picks {
        match aug.effect {
            Effect::Health(x) => health *= x,
            Effect::Damage(x) => elite.damage *= x,
            Effect::Speed(x) => elite.speed *= x,
            Effect::Armor(x) => elite.armor += x,
            Effect::Regen(x) => elite.regen += x,
        }
    }
    let colors: Vec<Vec3> = picks
        .iter()
        .map(|a| Vec3::new(a.color.0, a.color.1, a.color.2))
        .collect();
    elite.tint = if invader {
        let c = data.invader_color;
        Vec3::new(c.0, c.1, c.2)
    } else if colors.is_empty() {
        Vec3::ONE
    } else {
        colors.iter().sum::<Vec3>() / colors.len() as f32
    };
    let titles: Vec<&str> = picks.iter().map(|a| a.title.as_str()).collect();
    let mut name = base_name.to_string();
    if invader {
        name.push_str(" envahisseur");
    }
    if !titles.is_empty() {
        name = format!("{name} {}", join_titles(&titles));
    }
    elite.name = name;
    (elite, health)
}

/// La créature peut-elle devenir élite ?
fn eligible(world: &World, entity: Entity) -> bool {
    let registry = world.resource::<Arc<MobRegistry>>();
    world
        .get::<crate::components::Mob>(entity)
        .and_then(|m| {
            registry
                .get(m.kind)
                .map(|d| d.hostile && !d.boss && !d.brain)
        })
        .unwrap_or(false)
        && !world.has::<Elite>(entity)
}

/// Fait de cette créature une élite (ou un envahisseur) : sa vie est multipliée, elle reçoit le
/// composant [`Elite`]. `false` si elle n'est pas éligible.
pub fn promote(
    world: &mut World,
    entity: Entity,
    data: &AdventureData,
    tier: u8,
    invader: bool,
    rng: &mut Rng64,
) -> bool {
    if !eligible(world, entity) {
        return false;
    }
    let base_name = {
        let registry = world.resource::<Arc<MobRegistry>>();
        let Some(mob) = world.get::<crate::components::Mob>(entity) else {
            return false;
        };
        match registry.get(mob.kind) {
            Some(def) => def.name.clone(),
            None => return false,
        }
    };
    let picks = pick_augmentations(data, tier, picks_for(data, tier, invader), rng);
    let (elite, health) = build(data, &base_name, &picks, invader);
    if let Some(mut h) = world.get_mut::<Health>(entity) {
        h.max *= health;
        h.current = h.max;
    }
    world.insert(entity, elite);
    true
}

/// À la naissance d'une hostile : le palier tire si elle est une élite. Sans palier dans le monde
/// (tests de la simulation), rien ne change.
pub fn maybe_promote(world: &mut World, entity: Entity) {
    let Some(effects) = world.get_resource::<TierEffects>().map(|e| *e) else {
        return;
    };
    if effects.elite_chance <= 0.0 || !eligible(world, entity) {
        return;
    }
    let seed = {
        let mut rng = world.resource_mut::<GameRng>();
        let roll = rng.next_f32();
        if roll >= effects.elite_chance {
            return;
        }
        (rng.next_f32() * 16_777_216.0) as u64 ^ u64::from(entity.index) << 20
    };
    let data = crate::adventure_data::current();
    promote(
        world,
        entity,
        &data,
        effects.tier,
        false,
        &mut Rng64::new(seed),
    );
}

/// Le multiplicateur de dégâts d'une créature : le palier, puis l'élite.
pub fn damage_of(world: &World, entity: Entity) -> f32 {
    let tier = world
        .get_resource::<TierEffects>()
        .map_or(1.0, |e| e.damage);
    let elite = world.get::<Elite>(entity).map_or(1.0, |e| e.damage);
    tier * elite
}

/// Le multiplicateur de vitesse d'une créature.
pub fn speed_of(world: &World, entity: Entity) -> f32 {
    world.get::<Elite>(entity).map_or(1.0, |e| e.speed)
}

/// Les points d'armure qu'une élite ajoute.
pub fn armor_of(world: &World, entity: Entity) -> f32 {
    world.get::<Elite>(entity).map_or(0.0, |e| e.armor)
}

/// Le multiplicateur de dégâts du palier seul (les tirs des archers).
pub fn tier_damage(world: &World) -> f32 {
    world
        .get_resource::<TierEffects>()
        .map_or(1.0, |e| e.damage)
}

/// Les élites régénérantes se soignent (un système de la simulation).
pub fn regen(world: &mut World) {
    let dt = world.resource::<FixedDt>().0;
    world
        .query::<(&Elite, &mut Health)>()
        .for_each(|_, (elite, health)| {
            if elite.regen > 0.0 && health.current > 0.0 {
                health.current = (health.current + elite.regen * dt).min(health.max);
            }
        });
}

/// La couleur d'une boîte d'élite : sa teinte se mêle à celle de la créature.
pub fn tinted(color: Vec3, elite: &crate::entities::EliteLook) -> Vec3 {
    color.lerp(elite.tint, if elite.invader { 0.65 } else { 0.5 })
}

/// L'élite tuée, pour le butin : lue avant que la créature ne disparaisse.
pub fn of(world: &World, entity: Entity) -> Option<Elite> {
    world.get::<Elite>(entity).map(|e| e.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Mob, Transform};

    fn data() -> AdventureData {
        AdventureData::builtin()
    }

    #[test]
    fn augmentations_are_drawn_without_repeats_and_depend_on_the_tier() {
        let data = data();
        let mut rng = Rng64::new(3);
        for _ in 0..200 {
            let picks = pick_augmentations(&data, 4, 3, &mut rng);
            assert_eq!(picks.len(), 3);
            let mut ids: Vec<_> = picks.iter().map(|a| &a.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), 3, "jamais deux fois la même");
        }
        // Au palier 0, bastion et revenant (min_tier 1) ne sortent pas.
        for _ in 0..300 {
            let picks = pick_augmentations(&data, 0, 3, &mut rng);
            assert!(
                picks
                    .iter()
                    .all(|a| a.id != "bastion" && a.id != "revenant")
            );
        }
        // Même graine, même tirage.
        let a = pick_augmentations(&data, 3, 2, &mut Rng64::new(9));
        let b = pick_augmentations(&data, 3, 2, &mut Rng64::new(9));
        assert_eq!(a, b);
        // Plus d'augmentations qu'il n'y en a : on s'arrête.
        assert_eq!(
            pick_augmentations(&data, 4, 99, &mut rng).len(),
            data.augmentations.len()
        );
    }

    #[test]
    fn augmentations_multiply_their_effects_and_compose_the_name() {
        let data = data();
        let by = |id: &str| data.augmentations.iter().find(|a| a.id == id).unwrap();
        let picks = [by("colossus"), by("butcher"), by("lightning")];
        let (elite, health) = build(&data, "Rôdeur", &picks, false);
        assert_eq!(health, 1.8);
        assert_eq!(elite.damage, 1.5);
        assert_eq!(elite.speed, 1.45);
        assert_eq!(elite.name, "Rôdeur colosse, bourreau et éclair");
        assert_eq!(elite.augmentations, ["colossus", "butcher", "lightning"]);
        let (one, _) = build(&data, "Cracheur", &[by("bastion")], false);
        assert_eq!(one.name, "Cracheur bastion");
        assert_eq!(one.armor, 6.0);
        let (two, _) = build(&data, "Rôdeur", &[by("revenant"), by("colossus")], false);
        assert_eq!(two.name, "Rôdeur revenant et colosse");
        assert_eq!(two.regen, 1.5);
        // Un envahisseur : plus de vie, son nom et sa teinte à lui.
        let (invader, health) = build(&data, "Rôdeur", &[by("colossus")], true);
        assert!((health - 1.8 * data.invader_health).abs() < 1e-5);
        assert_eq!(invader.name, "Rôdeur envahisseur colosse");
        assert_eq!(
            invader.tint,
            Vec3::new(
                data.invader_color.0,
                data.invader_color.1,
                data.invader_color.2
            )
        );
        assert!(invader.invader && !elite.invader);
    }

    #[test]
    fn the_number_of_augmentations_grows_with_the_tier() {
        let data = data();
        assert_eq!(picks_for(&data, 0, false), 1);
        assert_eq!(picks_for(&data, 2, false), 2);
        assert_eq!(picks_for(&data, 4, false), 3, "plafonné par max_picks");
        assert_eq!(
            picks_for(&data, 2, true),
            4,
            "le double pour un envahisseur"
        );
        assert!(picks_for(&data, 4, true) <= 6);
    }

    fn world_with_prowler() -> (World, Entity) {
        let mut world = crate::mobs::tests::sim();
        let kind = world
            .resource::<Arc<MobRegistry>>()
            .id("core:prowler")
            .unwrap();
        let prowler = crate::mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 70.0, 5.5)).unwrap();
        (world, prowler)
    }

    #[test]
    fn promoting_a_hostile_scales_its_life_and_marks_it() {
        let (mut world, prowler) = world_with_prowler();
        let base = world.get::<Health>(prowler).unwrap().max;
        let data = data();
        assert!(promote(
            &mut world,
            prowler,
            &data,
            3,
            false,
            &mut Rng64::new(4)
        ));
        let elite = of(&world, prowler).unwrap();
        assert!(elite.name.starts_with("Rôdeur "));
        assert!(!elite.augmentations.is_empty());
        let health = world.get::<Health>(prowler).unwrap();
        assert_eq!(health.current, health.max, "la vie est pleine");
        assert!(health.max >= base * 0.99);
        // On ne devient pas élite deux fois.
        drop(health);
        assert!(!promote(
            &mut world,
            prowler,
            &data,
            3,
            false,
            &mut Rng64::new(5)
        ));
        // Un animal n'est jamais élite.
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let pig = crate::mobs::spawn_mob(&mut world, pig, Vec3::new(8.5, 70.0, 5.5)).unwrap();
        assert!(!promote(
            &mut world,
            pig,
            &data,
            3,
            false,
            &mut Rng64::new(5)
        ));
    }

    #[test]
    fn the_crypt_warden_of_the_donjons_mod_is_a_boss() {
        let assets = crate::assets_dir();
        let mods = crate::modding::mod_dirs(&crate::modding::resolve_order(&assets));
        let registry =
            MobRegistry::load(&assets, &mods, &crate::test_registry()).expect("les créatures");
        assert!(registry.by_key("donjons:warden").unwrap().boss);
        assert!(!registry.by_key("core:prowler").unwrap().boss);
    }

    #[test]
    fn a_boss_is_never_an_elite() {
        let mut world = crate::mobs::tests::sim();
        let blocks = engine_voxel::BlockRegistry::builtin();
        let mut registry = (**world.resource::<Arc<MobRegistry>>()).clone();
        registry
            .add_source(
                "core",
                r#"[(id: "core:titan", name: "Titan", health: 50.0, speed: 1.0, size: (1.0, 2.0),
                    hostile: true, boss: true, spawn_on: ["core:grass"], parts: [])]"#,
                "t",
                &blocks,
            )
            .unwrap();
        let kind = registry.id("core:titan").unwrap();
        world.insert_resource(Arc::new(registry));
        let titan = crate::mobs::spawn_mob(&mut world, kind, Vec3::new(5.5, 70.0, 5.5)).unwrap();
        assert!(!promote(
            &mut world,
            titan,
            &data(),
            4,
            false,
            &mut Rng64::new(1)
        ));
        assert!(!world.has::<Elite>(titan));
    }

    #[test]
    fn the_tier_decides_how_many_hostiles_are_elites() {
        let data = data();
        let count_elites = |chance: f32| {
            let mut world = crate::mobs::tests::sim();
            world.insert_resource(TierEffects {
                elite_chance: chance,
                tier: 2,
                ..TierEffects::default()
            });
            let kind = world
                .resource::<Arc<MobRegistry>>()
                .id("core:prowler")
                .unwrap();
            let mut elites = 0;
            for i in 0..200 {
                let e = crate::mobs::spawn_mob(
                    &mut world,
                    kind,
                    Vec3::new(5.5 + (i % 10) as f32, 70.0, 5.5 + (i / 10) as f32),
                )
                .unwrap();
                maybe_promote(&mut world, e);
                elites += usize::from(world.has::<Elite>(e));
            }
            elites
        };
        let _ = &data;
        assert_eq!(count_elites(0.0), 0);
        let some = count_elites(0.3);
        assert!((30..=90).contains(&some), "{some} élites sur 200 à 30 %");
        assert_eq!(count_elites(1.0), 200);
    }

    #[test]
    fn elites_hit_harder_walk_faster_and_the_tier_scales_damage() {
        let (mut world, prowler) = world_with_prowler();
        assert_eq!(damage_of(&world, prowler), 1.0);
        world.insert_resource(TierEffects {
            damage: 2.0,
            ..TierEffects::default()
        });
        assert_eq!(damage_of(&world, prowler), 2.0, "le palier");
        world.insert(
            prowler,
            Elite {
                name: "Rôdeur".into(),
                augmentations: vec![],
                tint: Vec3::ONE,
                damage: 1.5,
                speed: 1.4,
                armor: 6.0,
                regen: 0.0,
                invader: false,
            },
        );
        assert_eq!(damage_of(&world, prowler), 3.0, "le palier fois l'élite");
        assert_eq!(speed_of(&world, prowler), 1.4);
        assert_eq!(armor_of(&world, prowler), 6.0);
        assert_eq!(tier_damage(&world), 2.0);
    }

    #[test]
    fn elites_and_invaders_do_not_dissolve_in_the_sun() {
        let mut world = crate::mobs::tests::sim();
        let registry = world.resource::<Arc<MobRegistry>>().clone();
        let slime = registry.id("example:slime").unwrap();
        let plain = crate::mobs::spawn_mob(&mut world, slime, Vec3::new(5.5, 150.0, 5.5)).unwrap();
        let elite = crate::mobs::spawn_mob(&mut world, slime, Vec3::new(6.5, 150.0, 5.5)).unwrap();
        assert!(promote(
            &mut world,
            elite,
            &data(),
            2,
            true,
            &mut Rng64::new(2)
        ));
        for _ in 0..60 {
            world.resource_mut::<crate::mobs::SimClock>().dissolve_timer = 0.0;
            crate::spawning::dissolve(&mut world);
            world.apply_commands();
        }
        assert!(
            world.get::<Transform>(plain).is_none(),
            "l'ordinaire se dissout"
        );
        assert!(
            world.get::<Transform>(elite).is_some(),
            "l'envahisseur reste"
        );
    }

    #[test]
    fn a_regenerating_elite_heals_up_to_its_maximum() {
        let (mut world, prowler) = world_with_prowler();
        world.insert(
            prowler,
            Elite {
                name: "Rôdeur".into(),
                augmentations: vec![],
                tint: Vec3::ONE,
                damage: 1.0,
                speed: 1.0,
                armor: 0.0,
                regen: 6.0,
                invader: false,
            },
        );
        world.get_mut::<Health>(prowler).unwrap().current = 1.0;
        for _ in 0..60 {
            regen(&mut world);
        }
        let health = world.get::<Health>(prowler).unwrap().current;
        assert!((health - 7.0).abs() < 0.05, "{health}");
        for _ in 0..6000 {
            regen(&mut world);
        }
        let h = world.get::<Health>(prowler).unwrap();
        assert_eq!(h.current, h.max);
    }

    #[test]
    fn an_invader_tints_what_it_wears() {
        let (elite, _) = build(&data(), "Rôdeur", &[], true);
        let look = crate::entities::EliteLook {
            tint: elite.tint,
            invader: elite.invader,
        };
        let tint = tinted(Vec3::new(0.2, 0.5, 0.2), &look);
        assert_ne!(tint, Vec3::new(0.2, 0.5, 0.2));
        let _ = (Mob { kind: 0 }, Transform::default());
    }
}
