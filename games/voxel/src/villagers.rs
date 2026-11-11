//! Les habitants des villages : le plan d'un village (voir `engine_voxel`) marque des
//! endroits (`Spot`) où vit un fermier, un forgeron ou un marchand. Quand le joueur s'approche
//! d'un village, ceux qui manquent apparaissent à leur place et restent dans les environs
//! (`Home`) ; un habitant tué revient après un moment, tant que le joueur est là.
//!
//! Les créatures ne sont pas sauvegardées : on les recrée à chaque visite, avec leurs
//! échanges neufs.

use std::collections::HashMap;
use std::sync::Arc;

use engine_ecs::World;
use engine_voxel::{Spot, StructureKind, TerrainGenerator};
use glam::{IVec3, Vec3};

use crate::components::{Home, Mob, Trader, Transform};
use crate::mobs::{self, MobRegistry};
use crate::shared::StreamerExt;
use crate::trading::TradeBook;

/// Les habitants n'apparaissent que pour un village à cette distance du joueur.
const NEAR: f32 = 48.0;
/// Un habitant tué ou perdu revient après tant de secondes.
const RESPAWN_SECONDS: f32 = 90.0;
/// À moins de cette distance de sa place, un habitant « est là » (il a un peu erré).
const PRESENT: f32 = 16.0;
/// On regarde les villages tous les tant de secondes.
const CHECK_EVERY: f32 = 1.0;

struct Place {
    spot: Spot,
    /// Avant ce délai, on ne refait pas apparaître un habitant ici.
    cooldown: f32,
}

#[derive(Default)]
pub struct Villages {
    /// Les places de chaque village connu, par position de son centre.
    known: HashMap<(i32, i32), Vec<Place>>,
    timer: f32,
}

impl Villages {
    pub fn update(
        &mut self,
        dt: f32,
        ecs: &mut World,
        generator: &TerrainGenerator,
        player: Vec3,
        trades: &TradeBook,
    ) {
        for places in self.known.values_mut() {
            for place in places {
                place.cooldown = (place.cooldown - dt).max(0.0);
            }
        }
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = CHECK_EVERY;

        let sites = generator.structure_sites_near(
            player.x.floor() as i32,
            player.z.floor() as i32,
            NEAR as i32 + 40,
        );
        for site in sites.iter().filter(|s| s.kind == StructureKind::Village) {
            let key = (site.x, site.z);
            self.known.entry(key).or_insert_with(|| {
                generator
                    .blueprint(site)
                    .map(|b| b.spots)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|spot| Place {
                        spot,
                        cooldown: 0.0,
                    })
                    .collect()
            });
        }

        // Les habitants déjà là : (type, position).
        let mut present: Vec<(u32, Vec3)> = Vec::new();
        ecs.query::<(&Mob, &Transform)>()
            .for_each(|_, (mob, transform)| present.push((mob.kind, transform.position)));
        let registry = ecs.resource::<Arc<MobRegistry>>().clone();
        let mut to_spawn: Vec<(u32, Vec3, String)> = Vec::new();
        for places in self.known.values_mut() {
            for place in places {
                let at = place.spot.pos.as_vec3() + Vec3::new(0.5, 0.0, 0.5);
                if (at.x - player.x).hypot(at.z - player.z) > NEAR || place.cooldown > 0.0 {
                    continue;
                }
                let Some(kind) = registry.id(place.spot.mob) else {
                    continue;
                };
                if present
                    .iter()
                    .any(|&(k, p)| k == kind && (p.x - at.x).hypot(p.z - at.z) < PRESENT)
                {
                    continue;
                }
                to_spawn.push((kind, at, place.spot.mob.to_string()));
                place.cooldown = RESPAWN_SECONDS;
            }
        }
        for (kind, at, key) in to_spawn {
            // Le terrain doit être chargé : de l'air, sur un sol plein.
            let ready = {
                let shared = ecs.streamer();
                let streamer = shared.borrow();
                let cell = at.floor().as_ivec3();
                let world = streamer.world();
                world.block(cell).is_air() && world.is_solid(cell - IVec3::Y)
            };
            if !ready {
                continue;
            }
            if let Some(entity) = mobs::spawn_mob(ecs, kind, at) {
                ecs.insert(entity, Home(at));
                ecs.insert(
                    entity,
                    Trader {
                        uses: trades.for_mob(&key).iter().map(|t| t.uses).collect(),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::tests::sim_at;
    use engine_voxel::ItemRegistry;

    #[test]
    fn villagers_appear_at_their_spots_once_and_come_back_after_a_death() {
        // Un village de la graine de test, et un monde chargé autour de lui.
        let probe = TerrainGenerator::with_registry(5, std::sync::Arc::new(crate::test_registry()))
            .unwrap();
        let site = probe
            .structure_sites_in((-4000, -4000), (4000, 4000))
            .into_iter()
            .find(|s| s.kind == StructureKind::Village)
            .expect("un village");
        let y = probe.column(site.x, site.z).height as f32 + 2.0;
        let center = Vec3::new(site.x as f32, y, site.z as f32);
        let mut world = sim_at(center, 5);
        let player = world.resource::<mobs::PlayerEntity>().0;
        world.get_mut::<Transform>(player).unwrap().position = center;
        let items = ItemRegistry::builtin(&crate::test_registry()).unwrap();
        let trades = TradeBook::builtin(&items).unwrap();
        let spots = probe.blueprint(&site).unwrap().spots;
        assert!(!spots.is_empty());

        let mut villages = Villages::default();
        let generator = world.streamer().borrow().generator().clone();
        let count = |world: &World| {
            let mut n = 0;
            world.query::<(&Home, &Trader)>().for_each(|_, _| n += 1);
            n
        };
        villages.update(1.0, &mut world, &generator, center, &trades);
        // Seuls les habitants dont la place est à portée (et chargée) apparaissent.
        let first = count(&world);
        assert!(first >= 1, "aucun habitant");
        assert!(first <= spots.len());
        // Une seconde passe ne double rien.
        villages.update(1.0, &mut world, &generator, center, &trades);
        assert_eq!(count(&world), first);

        // Chaque habitant a ses échanges neufs.
        world.query::<(&Trader,)>().for_each(|_, (t,)| {
            assert!(!t.uses.is_empty() && t.uses.iter().all(|&u| u > 0));
        });

        // Un habitant tué : il revient, mais pas tout de suite.
        let victim = {
            let mut found = None;
            world
                .query::<(&Home,)>()
                .for_each(|e, _| found = found.or(Some(e)));
            found.unwrap()
        };
        world.despawn(victim);
        villages.update(1.0, &mut world, &generator, center, &trades);
        assert_eq!(count(&world), first - 1, "trop tôt pour revenir");
        for _ in 0..100 {
            villages.update(1.0, &mut world, &generator, center, &trades);
        }
        assert_eq!(count(&world), first, "il est revenu");
    }
}
