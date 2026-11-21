//! L'aventure côté jeu : le palier du monde, la progression, le butin à affixes, les envahisseurs.
//!
//! Les règles pures sont ailleurs (`tiers.rs`, `affixes.rs`, `elites.rs`, `gems.rs`,
//! `forge_tables.rs`) ; ce module les branche sur le monde vivant.
//!
//! # À chaque pas
//!
//! [`VoxelGame::update_adventure`] tient à jour la ressource [`TierEffects`] que la simulation lit,
//! puis, une fois par seconde : la profondeur du joueur, les objets qu'il porte (la plus haute
//! rareté obtenue ; un objet « à tirer » ramassé dans un coffre est tiré ici s'il a échappé à
//! l'ouverture), et les paliers qui se débloquent (un message l'annonce). Si le palier le permet, un
//! minuteur fait surgir un **envahisseur** : un son grave, un message, et un faisceau de lumière
//! (voir `elites.rs`).
//!
//! # Le butin
//!
//! - une **hostile** lâche, avec la chance `drop_chance` du palier, un objet à affixes ;
//! - une **élite** en lâche toujours un (rareté minimale `elite_min_rarity`), parfois une gemme et de
//!   la poussière ; un **envahisseur** un objet d'une rareté plus haute, une gemme et de la
//!   poussière ;
//! - les objets viennent de la liste `loot` de `elites.ron` (filtrée par palier) ; leur rareté suit
//!   le palier choisi ([`TierDef::rarity_boost`](crate::adventure_data::TierDef)).
//!
//! # Les coffres
//!
//! Les coffres des structures contiennent des objets « à tirer » (voir `affixes::resolve_pending`) ;
//! [`VoxelGame::resolve_chest_loot`] les tire à l'ouverture du coffre.

use engine_ecs::Entity;
use engine_voxel::{ItemRegistry, Rng64};
use glam::{IVec3, Vec3};

use crate::adventure_data::{self, AdventureData, GemDrop};
use crate::affixes;
use crate::elites::{self, Elite};
use crate::inventory::ItemStack;
use crate::item_data;
use crate::mobs::MobRegistry;
use crate::player;
use crate::tiers::{Progress, TierEffects};

/// L'état d'aventure d'un monde ouvert.
#[derive(Debug, Clone, Default)]
pub struct Adventure {
    pub progress: Progress,
    /// Le temps avant le prochain envahisseur (0 : à tirer).
    invader_timer: f32,
    /// Le temps avant la prochaine revue (profondeur, objets portés, déblocages).
    scan_timer: f32,
    /// La rareté que la table de réforge est en train de proposer.
    pub forge_rarity: u8,
    /// La graine des tirages de la table de réforge (elle avance après chaque réforge).
    pub forge_seed: u64,
}

impl Adventure {
    /// Relit la progression de `world.ron`.
    pub fn load(save: Option<&engine_voxel::AdventureSave>, data: &AdventureData) -> Self {
        Self {
            progress: save.map_or_else(Progress::default, |s| Progress::from_save(s, &data.tiers)),
            ..Self::default()
        }
    }

    /// Ce que la simulation lit du palier choisi.
    pub fn effects(&self, data: &AdventureData) -> TierEffects {
        data.tier(self.progress.tier)
            .map_or_else(TierEffects::default, |t| {
                TierEffects::of(self.progress.tier, t)
            })
    }
}

/// Un nombre au hasard entre min et max (inclus).
fn between(rng: &mut Rng64, (min, max): (u32, u32)) -> u32 {
    if max <= min {
        min
    } else {
        min + rng.below(max - min + 1)
    }
}

/// Un objet à affixes tiré dans la liste de butin (filtrée par palier). Pure : la graine décide.
pub fn roll_affixed(
    data: &AdventureData,
    items: &ItemRegistry,
    tier: u8,
    boost: f32,
    min_rarity: u8,
    seed: u64,
) -> Option<ItemStack> {
    let candidates: Vec<_> = data
        .loot
        .iter()
        .filter(|(_, min)| *min <= tier)
        .filter_map(|(key, _)| items.id(key))
        .collect();
    let mut rng = Rng64::new(seed);
    if candidates.is_empty() {
        return None;
    }
    let item = candidates[rng.below(candidates.len() as u32) as usize];
    let sheet = affixes::generate(data, items.get(item), boost, min_rarity, rng.next_u64())?;
    let mut damage = 0;
    affixes::apply(&mut damage, sheet).then_some(ItemStack {
        item,
        count: 1,
        damage,
    })
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Fait de cette créature un envahisseur.
    pub fn promote_invader(&mut self, entity: Entity, data: &AdventureData) -> bool {
        let tier = self.players.me_mut().adventure.progress.tier;
        let mut rng = Rng64::new(self.next_random());
        elites::promote(&mut self.ecs, entity, data, tier, true, &mut rng)
    }

    /// Une hostile ordinaire (ni boss ni scriptée) au hasard.
    fn pick_invader_kind(&mut self) -> Option<u32> {
        let registry = self.ecs.resource::<std::sync::Arc<MobRegistry>>().clone();
        let kinds: Vec<u32> = (0..registry.len() as u32)
            .filter(|k| {
                registry
                    .get(*k)
                    .is_some_and(|d| d.hostile && !d.boss && !d.brain && d.spawn.weight > 0)
            })
            .collect();
        if kinds.is_empty() {
            return None;
        }
        let pick = Rng64::new(self.next_random()).below(kinds.len() as u32) as usize;
        Some(kinds[pick])
    }

    /// Un sol où se tenir à cette distance et dans cette direction du joueur.
    fn invader_site(&self, center: Vec3, angle: f32, distance: f32) -> Option<Vec3> {
        let x = (center.x + angle.cos() * distance).floor() as i32;
        let z = (center.z + angle.sin() * distance).floor() as i32;
        let registry = self.ecs.resource::<std::sync::Arc<MobRegistry>>().clone();
        let shared = crate::shared::StreamerExt::streamer(&self.ecs);
        let streamer = shared.borrow();
        let height = streamer.generator().column(x, z).height;
        let floor = IVec3::new(x, height, z);
        (streamer.is_lit(engine_voxel::World::split(floor).0)
            && crate::spawning::standable(streamer.world(), &registry, floor))
        .then(|| Vec3::new(x as f32 + 0.5, height as f32 + 1.0, z as f32 + 0.5))
    }

    fn announce_invader(&mut self, center: Vec3, site: Vec3) {
        let distance = center.distance(site).round();
        self.toast(&format!("Un envahisseur approche ! ({distance} blocs)"));
        self.sound("core:cave_rumble");
        self.sound("core:enchant");
    }

    /// Fait apparaître un envahisseur non loin du joueur : un son, un message, un faisceau.
    pub fn spawn_invader(&mut self, data: &AdventureData) -> Option<Vec3> {
        let center = player::body(&self.ecs, self.players.entity()).position;
        let kind = self.pick_invader_kind()?;
        let site = (0..12).find_map(|_| {
            let [angle, dist] = [(); 2].map(|()| Rng64::new(self.next_random()).next_f32());
            self.invader_site(center, angle * std::f32::consts::TAU, 24.0 + dist * 12.0)
        })?;
        let entity = crate::mobs::spawn_mob(&mut self.ecs, kind, site)?;
        self.promote_invader(entity, data);
        self.announce_invader(center, site);
        Some(site)
    }

    /// La revue d'une seconde : profondeur, objets portés, déblocages.
    fn scan_adventure(&mut self, data: &AdventureData) {
        let feet = player::body(&self.ecs, self.players.entity()).position;
        self.players
            .me_mut()
            .adventure
            .progress
            .note_depth(feet.y, engine_voxel::SEA_LEVEL);
        let items = self.content.items.clone();
        let boost = self.rarity_boost(data);
        let mut best = None;
        let mut visit = |stack: &mut Option<ItemStack>| {
            let Some(s) = stack.as_mut() else { return };
            if item_data::is_pending(s.damage) {
                s.damage = affixes::resolve_pending(data, items.get(s.item), s.damage, boost);
            }
            if let Some(r) = affixes::rarity_of(s.damage) {
                best = best.max(Some(r));
            }
        };
        for slot in self.players.me_mut().inventory.slots.iter_mut() {
            visit(slot);
        }
        for slot in self.players.me_mut().inventory.armor.iter_mut() {
            visit(slot);
        }
        visit(&mut self.players.me_mut().inventory.cursor);
        if let Some(rarity) = best {
            self.players.me_mut().adventure.progress.note_rarity(rarity);
        }
        self.unlock_tiers(data);
    }

    /// À chaque pas de la simulation (voir la doc du module).
    pub fn update_adventure(&mut self, dt: f32) {
        let data = adventure_data::current();
        self.ecs
            .insert_resource(self.players.me_mut().adventure.effects(&data));
        self.players.me_mut().adventure.scan_timer -= dt;
        if self.players.me_mut().adventure.scan_timer <= 0.0 {
            self.players.me_mut().adventure.scan_timer = 1.0;
            self.scan_adventure(&data);
        }
        let (min, max) = self.players.me_mut().adventure.effects(&data).invader;
        if max <= 0.0 {
            self.players.me_mut().adventure.invader_timer = 0.0;
            return;
        }
        if self.players.me_mut().adventure.invader_timer <= 0.0 {
            let roll = Rng64::new(self.next_random()).next_f32();
            self.players.me_mut().adventure.invader_timer = min + (max - min) * roll;
        }
        self.players.me_mut().adventure.invader_timer -= dt;
        if self.players.me_mut().adventure.invader_timer <= 0.0 {
            self.spawn_invader(&data);
        }
    }

    /// Tire les objets « à tirer » d'un coffre qu'on ouvre (voir la doc du module).
    pub fn resolve_chest_loot(&mut self, pos: IVec3) {
        let data = adventure_data::current();
        let items = self.content.items.clone();
        let boost = self.rarity_boost(&data);
        let mut streamer = self.streamer_mut();
        let Some(mut entity) = streamer.block_entity(pos).cloned() else {
            return;
        };
        let mut changed = false;
        if let Some(inventory) = entity.caps.as_mut().and_then(|c| c.inventory.as_mut()) {
            for stack in inventory.slots.iter_mut().flatten() {
                if item_data::is_pending(stack.damage)
                    && let Some(id) = items.id(&stack.item)
                {
                    stack.damage =
                        affixes::resolve_pending(&data, items.get(id), stack.damage, boost);
                    changed = true;
                }
            }
        }
        if changed {
            streamer.set_block_entity(pos, Some(entity));
        }
    }

    /// La hausse de rareté du palier choisi.
    fn rarity_boost(&self, data: &AdventureData) -> f32 {
        data.tier(self.players.me().adventure.progress.tier)
            .map_or(0.0, |t| t.rarity_boost)
    }

    /// Débloque ce que la progression permet, avec un message.
    pub fn unlock_tiers(&mut self, data: &AdventureData) {
        if let Some(tier) = self
            .players
            .me_mut()
            .adventure
            .progress
            .refresh_unlocked(&data.tiers)
        {
            let name = data.tier(tier).map_or("?", |t| t.name.as_str()).to_string();
            // La fenêtre ajoute la touche des paliers, qui est l'un de ses réglages.
            let id = self.players.current_id();
            self.replies.push((
                id,
                crate::protocol::ServerMsg::Feedback(crate::player_view::Feedback::TierUnlocked(
                    name,
                )),
            ));
            self.sound("core:enchant");
        }
    }

    /// Une gemme tirée selon `drop`, et peut-être de la poussière.
    fn roll_gem_stacks(&mut self, drop: GemDrop) -> Vec<ItemStack> {
        let data = adventure_data::current();
        let items = self.content.items.clone();
        let mut rng = Rng64::new(self.next_random());
        let mut out = Vec::new();
        if rng.next_f32() < drop.chance && !data.gems.is_empty() {
            let kind = &data.gems[rng.below(data.gems.len() as u32) as usize];
            let top = data.qualities.len().saturating_sub(1) as u32;
            let quality = between(&mut rng, (drop.quality.0.min(top), drop.quality.1.min(top)));
            if let Some(id) = items.id(&kind.item_key(quality as usize)) {
                out.push(ItemStack::new(id, 1));
            }
        }
        if rng.next_f32() < drop.dust
            && let Some(id) = crate::forge_tables::dust_item(&items)
        {
            out.push(ItemStack::new(id, 1));
        }
        out
    }

    /// Un objet à affixes tiré au hasard dans la liste de butin du palier choisi.
    pub fn roll_affixed_stack(&mut self, min_rarity: u8) -> Option<ItemStack> {
        let data = adventure_data::current();
        let items = self.content.items.clone();
        let tier = self.players.me_mut().adventure.progress.tier;
        let boost = self.rarity_boost(&data);
        let seed = self.next_random();
        roll_affixed(&data, &items, tier, boost, min_rarity, seed)
    }

    /// Une hostile vient de mourir près du joueur : la progression la compte.
    pub fn credit_kill(&mut self, boss: bool) {
        self.players.me_mut().adventure.progress.note_kill(boss);
        let data = adventure_data::current();
        self.unlock_tiers(&data);
    }

    /// Le butin d'adventure d'une créature morte : un objet à affixes (une chance pour une hostile,
    /// toujours pour une élite), des gemmes. Les objets tombent là où elle est morte.
    pub fn adventure_loot(&mut self, position: Vec3, hostile: bool, elite: Option<&Elite>) {
        let data = adventure_data::current();
        let at = position.floor().as_ivec3();
        let mut stacks = Vec::new();
        match elite {
            Some(elite) => {
                let (rarity, gems) = if elite.invader {
                    (data.invader_min_rarity, data.invader_gem)
                } else {
                    (data.elite_min_rarity, data.elite_gem)
                };
                stacks.extend(self.roll_affixed_stack(rarity));
                stacks.extend(self.roll_gem_stacks(gems));
            }
            None if hostile => {
                let chance = data
                    .tier(self.players.me_mut().adventure.progress.tier)
                    .map_or(0.0, |t| t.drop_chance);
                if Rng64::new(self.next_random()).next_f32() < chance {
                    stacks.extend(self.roll_affixed_stack(0));
                }
            }
            None => {}
        }
        for stack in stacks {
            self.drop_stack(at, stack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn an_affixed_drop_comes_from_the_loot_list_of_the_tier() {
        let data = AdventureData::builtin();
        let items = items();
        let tier0: std::collections::HashSet<_> = data
            .loot
            .iter()
            .filter(|(_, t)| *t == 0)
            .map(|(k, _)| k.clone())
            .collect();
        for seed in 0..100 {
            let stack = roll_affixed(&data, &items, 0, 0.0, 1, seed).unwrap();
            assert!(tier0.contains(&items.get(stack.item).key));
            assert!(item_data::is_extended(stack.damage));
            let sheet = item_data::sheet(stack.damage).unwrap();
            assert!(sheet.rarity >= 1, "la rareté minimale");
            assert!(!sheet.affixes.is_empty());
        }
        // Aux paliers hauts, on trouve aussi du saphir.
        let found: std::collections::HashSet<_> = (0..300)
            .filter_map(|s| roll_affixed(&data, &items, 4, 2.0, 0, s))
            .map(|s| items.get(s.item).key.clone())
            .collect();
        assert!(found.iter().any(|k| k.contains("sapphire")));
        // Même graine, même objet.
        assert_eq!(
            roll_affixed(&data, &items, 2, 0.5, 0, 77).map(|s| items.get(s.item).key.clone()),
            roll_affixed(&data, &items, 2, 0.5, 0, 77).map(|s| items.get(s.item).key.clone())
        );
    }

    #[test]
    fn a_world_without_adventure_data_loads_at_tier_zero() {
        let data = AdventureData::builtin();
        let adventure = Adventure::load(None, &data);
        assert_eq!(adventure.progress, Progress::default());
        assert_eq!(adventure.effects(&data), TierEffects::of(0, &data.tiers[0]));
        let saved = engine_voxel::AdventureSave {
            tier: 3,
            unlocked: 3,
            ..Default::default()
        };
        let adventure = Adventure::load(Some(&saved), &data);
        assert_eq!(adventure.effects(&data).tier, 3);
        assert!(adventure.effects(&data).health > 2.0);
    }
}
