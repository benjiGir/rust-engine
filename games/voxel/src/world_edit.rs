//! Les méthodes du serveur qui étaient restées dans `main.rs` : fluides, éditions de blocs, actions
//! des machines.

use crate::components::{Collider, Transform};
use crate::events::{BlockBroken, BlockPlaced};
use crate::mobs::MobRegistry;
use crate::{components, drops, events, inventory, mobs, player, protocol};
use engine_voxel::{BlockId, ChunkStreamer, MachineAction};
use glam::{IVec3, Vec3};
use std::cell::{Ref, RefMut};
use std::sync::Arc;
use std::time::Instant;

/// Une étape des fluides tous les 6 pas de simulation (60 Hz) : 10 par seconde.
const FLUID_STEP_EVERY: u32 = 6;

/// Le temps maximal d'une étape des fluides : au-delà, la suite attend l'étape suivante.
const FLUID_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Une étape des fluides tous les 6 pas (10 par seconde), dans un budget de temps.
    pub(crate) fn step_fluids(&mut self) {
        self.fluid_steps += 1;
        if self.fluid_steps < FLUID_STEP_EVERY {
            return;
        }
        self.fluid_steps = 0;
        let report = self.fluids.step(&mut self.world.borrow_mut(), FLUID_BUDGET);
        self.fluid_report = report;
    }

    /// Ramasse les objets au sol tout proches.
    pub(crate) fn pick_up_drops(&mut self, dt: f32) {
        let feet = player::body(&self.ecs, self.players.entity()).position;
        for (entity, stack) in drops::update(&mut self.ecs, feet, dt) {
            let left = self.collect_stack(stack);
            if left < stack.count {
                self.sound("core:place_soft");
                if left == 0 {
                    self.ecs.despawn(entity);
                } else if let Some(mut drop) = self.ecs.get_mut::<drops::DroppedItem>(entity) {
                    drop.stack.count = left;
                }
            }
        }
    }

    pub(crate) fn notify_edit(&mut self, pos: IVec3) {
        self.fluids.wake(pos);
        let actions = self.mods.block_changed(pos);
        self.apply_machine_actions(actions);
    }

    pub(crate) fn after_edit(&mut self, pos: IVec3) {
        self.notify_edit(pos);
        // Une fleur sans sol tombe, le sable s'écroule.
        let settled = engine_voxel::settle(&mut self.streamer_mut(), pos);
        for (p, broken) in settled.broken {
            let harvest = self.registry.def(broken).tier == 0;
            self.drop_loot(p, broken, None, harvest);
            self.notify_edit(p);
            if let Some((other, _)) = engine_voxel::paired_block::partner(&self.registry, p, broken)
            {
                self.notify_edit(other);
            }
        }
        for (from, to, _) in settled.fallen {
            for p in [from, to] {
                self.notify_edit(p);
            }
        }
    }

    pub(crate) fn edit(&mut self, pos: IVec3, block: BlockId) -> bool {
        let old = self.streamer().world().block(pos);
        let partner = engine_voxel::paired_block::partner(&self.registry, pos, old)
            .filter(|&(p, expected)| old != block && self.streamer().world().block(p) == expected);
        let half = (old != block).then(|| self.split_chest(pos)).flatten();
        // Un bloc à inventaire (coffre, four, machine de mod) lâche son contenu quand il change.
        let contents = if old != block {
            self.container_contents(pos)
        } else {
            Vec::new()
        };
        let start = Instant::now();
        let done = self.streamer_mut().set_block(pos, block);
        if done {
            for stack in contents {
                self.drop_stack(pos, stack);
            }
            self.edit_times.push(start.elapsed());
            // On annonce le changement : le son (et plus tard les mods) s'y abonnent.
            if block.is_air() {
                events::send(
                    &self.ecs,
                    BlockBroken {
                        pos,
                        block: old,
                        player: Some(self.players.current_id()),
                    },
                );
            } else {
                events::send(
                    &self.ecs,
                    BlockPlaced {
                        pos,
                        block,
                        player: Some(self.players.current_id()),
                    },
                );
            }
            let joined = (!block.is_air()).then(|| self.join_chest(pos)).flatten();
            self.after_edit(pos);
            for other in partner
                .map(|(p, _)| p)
                .into_iter()
                .chain(half)
                .chain(joined)
            {
                self.after_edit(other);
            }
        }
        done
    }

    pub(crate) fn any_mob_body(
        &self,
        predicate: impl Fn(&engine_voxel::PlayerBody) -> bool,
    ) -> bool {
        let mut overlap = false;
        self.ecs
            .query::<(&Transform, &Collider, &components::Mob)>()
            .for_each(|_, (transform, collider, _)| {
                let body = engine_voxel::PlayerBody::with_size(
                    transform.position,
                    collider.width,
                    collider.height,
                );
                overlap |= predicate(&body);
            });
        overlap
    }

    /// Une créature occupe-t-elle ce bloc ?
    pub(crate) fn mob_overlaps(&self, pos: IVec3) -> bool {
        self.any_mob_body(|body| body.overlaps_block(pos))
    }

    /// L'objet en main s'use d'un point (Solidité peut l'épargner). `true` s'il se brise.
    pub(crate) fn wear_held(&mut self) -> bool {
        let roll = engine_voxel::Rng64::new(self.next_random()).next_f32();
        self.players
            .me_mut()
            .inventory
            .wear_selected(&self.content.items, roll)
    }

    pub(crate) fn streamer_mut(&self) -> RefMut<'_, ChunkStreamer> {
        self.world.borrow_mut()
    }

    pub(crate) fn place_at_target(&mut self) -> Option<IVec3> {
        let item = self.players.me_mut().inventory.selected_item()?;
        if self.content.items.get(item).key == "core:flask" {
            self.fill_flask(item);
            return None;
        }
        if self.content.items.get(item).key.ends_with("bucket") {
            self.use_bucket(item);
            return None;
        }
        // Une pièce taillée : elle s'ajoute à une case de formes (voir `shaped_play.rs`).
        if let Some(done) = self.place_shaped_piece() {
            return done;
        }
        let aim = self.aim();
        let hit = aim.hit?;
        let def = self.content.items.get(item);
        // Une graine plante son premier stade ; sinon l'objet pose son bloc.
        let block = def
            .block
            .or_else(|| def.plants.as_deref().and_then(|key| self.registry.id(key)))
            .or_else(|| def.places.as_deref().and_then(|key| self.registry.id(key)))?;
        // Un bloc à états (escalier, dalle, tuyau...) se pose selon le regard et la face visée.
        let forward = aim.forward;
        let hit_y = (aim.eye + forward * hit.distance).y - hit.block_pos.y as f32;
        let pos = hit.block_pos + hit.normal;
        if hit.normal == IVec3::ZERO {
            return None;
        }
        if let Some(parts) = self.paired_placement(pos, block, forward) {
            self.place_pair(parts)?;
            self.players.me_mut().inventory.consume_selected();
            self.punch();
            return Some(pos);
        }
        let block = self
            .registry
            .placement_variant(block, forward, hit.normal, hit_y);
        // On peut poser dans l'air ou à la place d'un liquide (l'eau).
        let target = self.streamer().world().block(pos);
        let replaceable = target.is_air() || self.registry.def(target).liquid;
        if !replaceable {
            return None;
        }
        if player::body(&self.ecs, self.players.entity()).overlaps_block(pos)
            || self.mob_overlaps(pos)
        {
            log::debug!("impossible de poser un bloc dans une entité");
            return None;
        }
        // Une plante ou une torche a besoin d'un support.
        let support = self.registry.def(block).support;
        if !engine_voxel::is_supported(&self.streamer(), &self.registry, pos, support) {
            return None;
        }
        let placed = self.edit(pos, block);
        if placed {
            self.players.me_mut().inventory.consume_selected();
            self.punch();
            // Des feuilles posées par le joueur ne se décomposent pas : une entité de bloc
            // les marque comme « posées ».
            // Un piston, un inverseur, un répéteur : ils visent où le joueur regarde.
            if let Some(state) = self
                .registry
                .def(block)
                .behavior
                .as_deref()
                .and_then(|b| engine_voxel::placement_state(b, forward))
            {
                self.streamer_mut().set_block_entity(pos, Some(state));
            }
            if self.registry.def(block).behavior.as_deref() == Some("core:leaves") {
                self.streamer_mut()
                    .set_block_entity(pos, Some(engine_voxel::BlockEntity::default()));
            }
        }
        placed.then_some(pos)
    }

    /// L'outil tenu en main.
    pub(crate) fn held_tool(&self) -> Option<engine_voxel::ToolDef> {
        self.players
            .me()
            .inventory
            .selected_item()
            .and_then(|i| self.content.items.get(i).tool)
    }

    /// Ce que les machines et les scripts demandent au jeu.
    pub(crate) fn apply_machine_actions(&mut self, actions: Vec<MachineAction>) {
        for action in actions {
            match action {
                MachineAction::GiveItem { block, count } => {
                    let Some(item) = self.content.items.of_block(block) else {
                        continue;
                    };
                    let left =
                        self.players
                            .me_mut()
                            .inventory
                            .add(item, count, &self.content.items);
                    if left > 0 {
                        log::info!("inventaire plein : {left} objet(s) perdu(s)");
                    }
                }
                MachineAction::GiveStack { item, count } => {
                    let left =
                        self.players
                            .me_mut()
                            .inventory
                            .add(item, count, &self.content.items);
                    if left > 0 {
                        log::info!("inventaire plein : {left} objet(s) perdu(s)");
                    }
                }
                MachineAction::ShowMenu { mod_id, at, data } => {
                    self.show_mod_menu(mod_id, at, &data);
                }
                MachineAction::CloseMenu { mod_id } => self.close_mod_menu_of(&mod_id),
                action @ (MachineAction::PlayerHeal { .. }
                | MachineAction::PlayerTeleport { .. }
                | MachineAction::PlayerEffect { .. }
                | MachineAction::HurtArea { .. }
                | MachineAction::Say { .. }
                | MachineAction::QuestSignal { .. }
                | MachineAction::PlayerHurt { .. }
                | MachineAction::ConsumeHeld { .. }
                | MachineAction::HudBar { .. }
                | MachineAction::HudClear { .. }) => self.apply_player_action(action),
                MachineAction::Travel { dimension } => {
                    self.travel(&dimension);
                }
                MachineAction::ShowOptions { mod_id, data } => {
                    // Une page d'options (réglages du jeu) : la fenêtre la montre si elle est ouverte.
                    let id = self.players.current_id();
                    self.replies
                        .push((id, protocol::ServerMsg::ModOptions { mod_id, data }));
                }
                MachineAction::SpawnEntity { kind, position } => {
                    let registry = self.ecs.resource::<Arc<MobRegistry>>().clone();
                    match registry.id(&kind) {
                        Some(id) => {
                            mobs::spawn_mob(&mut self.ecs, id, position);
                        }
                        None => log::warn!("créature inconnue demandée par un mod : {kind}"),
                    }
                }
                MachineAction::DropStack {
                    pos,
                    item,
                    count,
                    damage,
                } => {
                    if let Some(item) = self.content.items.id(&item) {
                        self.drop_stack(
                            pos,
                            inventory::ItemStack {
                                item,
                                count,
                                damage,
                            },
                        );
                    }
                }
                MachineAction::BlockBroken { pos, block } => {
                    events::send(
                        &self.ecs,
                        BlockBroken {
                            pos,
                            block,
                            player: None,
                        },
                    );
                }
                MachineAction::Log(message) => log::info!("{message}"),
                // Les ordres aux créatures ne valent que pendant leur lot (`mob_brains.rs`).
                MachineAction::MobMove { .. }
                | MachineAction::MobMoveTo { .. }
                | MachineAction::MobJump { .. }
                | MachineAction::MobAttack { .. }
                | MachineAction::MobState { .. } => {}
            }
        }
    }

    /// Casse le bloc en `pos` : il lâche son butin (selon l'outil), l'outil s'use, et les
    /// blocs qui en dépendaient (une fleur dessus, du sable) suivent.
    pub(crate) fn break_block(&mut self, pos: IVec3) -> bool {
        // Une case de formes perd une pièce à la fois (voir `shaped_play.rs`).
        if let Some(done) = self.break_shaped_piece(pos) {
            return done;
        }
        let block = self.streamer().world().block(pos);
        let def = self.registry.def(block).clone();
        if def.hardness < 0.0 {
            return false;
        }
        let tool = self.held_tool();
        // (`edit` fait tomber ce que contenait un coffre, un four ou une machine.)
        if !self.edit(pos, BlockId::AIR) {
            return false;
        }
        let harvest = engine_voxel::can_harvest(&def, tool.as_ref());
        self.drop_loot(pos, block, tool.map(|t| t.kind), harvest);
        // Un minerai cassé avec le bon outil : de l'expérience.
        if harvest {
            let points = self.roll_range(def.xp);
            self.grant_xp(points);
        }
        if tool.is_some() && def.hardness > 0.0 && self.wear_held() {
            log::info!("l'outil s'est cassé");
            self.sound("core:break_glass");
        }
        self.exert(self.hunger_rules.mine);
        true
    }

    /// Mort : on réapparaît au point de départ, en pleine forme.
    /// Le streamer des chunks (partagé avec l'ECS et les mods).
    pub(crate) fn streamer(&self) -> Ref<'_, ChunkStreamer> {
        self.world.borrow()
    }

    /// Un nombre pseudo-aléatoire pour le jeu (butin, rebonds).
    pub(crate) fn next_random(&mut self) -> u64 {
        self.random = self
            .random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.random ^ (self.random >> 29)
    }

    pub(crate) fn drop_stack(&mut self, pos: IVec3, stack: inventory::ItemStack) {
        let seed = self.next_random();
        drops::spawn(&mut self.ecs, stack, pos.as_vec3() + Vec3::splat(0.5), seed);
    }
}
