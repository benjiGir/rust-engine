//! Les structures mobiles dans le jeu : assembler, faire avancer, dessiner, y monter.
//!
//! - **L'assembleur** : un clic droit sur lui, dans le monde, détache du monde la structure qui
//!   y est reliée (tout ce qui touche, sauf le terrain) ; un clic droit sur lui, sur la
//!   structure, la repose (si la place est libre, arrondie au quart de tour).
//! - **Le mouvement** (système ECS) : la dynamique de `engine_voxel::ship` (gravité, flottaison,
//!   ballons, hélices, barre, collisions), puis les passagers sont portés avec la structure.
//! - **Les passagers** : une entité qui se tient sur le pont reçoit `OnGrid` ; sa physique se fait
//!   alors dans le repère de la structure (voir `mobs::physics`), donc elle marche, saute et tombe
//!   normalement sur un pont qui avance et tourne.
//! - **Le dessin** : chaque chunk local est maillé avec le mailleur du monde, et déplacé chaque
//!   image avec la transformation de la structure (voir `Renderer::set_mesh_transform`).
//!
//! Une structure assemblée ne se modifie pas : on la repose, on la change, on la réassemble.
//! Ses machines (câbles, moteurs, hélices, leviers) continuent de tourner.

use std::collections::HashMap;

use engine_ecs::{Entity, World as Ecs};
use engine_voxel::{GridId, GridPos, Grids, ShipInput, SubGrid, step_ship};

use super::*;
use crate::components::{Collider, OnGrid, Transform};
use crate::shared::{SharedSubGrids, StreamerExt};

/// À qui obéit la barre, et dans quel sens : `-1` à gauche, `1` à droite.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShipControl {
    pub grid: Option<GridId>,
    pub steer: f32,
    /// Un braquage imposé de l'extérieur (la commande `steer`), qui passe avant le clavier.
    pub forced: Option<(GridId, f32)>,
}

/// Ce que le système de mouvement laisse au jeu : de combien la structure a tourné sous les
/// pieds du joueur (pour tourner la caméra avec elle).
#[derive(Debug, Clone, Copy, Default)]
pub struct ShipEvents {
    pub player_turn: f32,
}

/// Les blocs du terrain : une structure s'arrête à eux (voir `engine_voxel::extract`).
pub fn terrain_blocks(registry: &BlockRegistry) -> std::collections::HashSet<BlockId> {
    [
        "core:stone",
        "core:dirt",
        "core:grass",
        "core:sand",
        "core:gravel",
        "core:deepstone",
        "core:bedrock",
        "core:snow",
        "core:ice",
        "core:clay",
        "core:sandstone",
        "core:volcanic_rock",
        "core:water",
        "core:lava",
        "core:coal_ore",
        "core:iron_ore",
        "core:copper_ore",
        "core:gold_ore",
        "core:cinnabar_ore",
        "core:malachite_ore",
        "core:sapphire_ore",
        "core:jade_ore",
    ]
    .iter()
    .filter_map(|k| registry.id(k))
    .collect()
}

/// À quelle hauteur sous les pieds est le pont, dans le repère de la structure ?
/// `None` s'il n'y a rien de plein dans les deux cases sous les pieds.
fn deck_gap(grid: &SubGrid, local: Vec3) -> Option<f32> {
    let base = (local.y - 0.1).floor() as i32;
    (0..=2).find_map(|d| {
        let cell = IVec3::new(local.x.floor() as i32, base - d, local.z.floor() as i32);
        grid.world()
            .is_solid(cell)
            .then(|| local.y - (cell.y + 1) as f32)
    })
}

/// Le système de mouvement (pas fixe) : avance les structures, porte les passagers, et fait
/// monter ou descendre les entités.
pub fn update(world: &mut Ecs) {
    let Some(shared) = world.get_resource::<SharedSubGrids>().map(|s| s.0.clone()) else {
        return;
    };
    let subs = shared.borrow();
    if subs.is_empty() {
        // Plus de structure : plus de passagers.
        let riding: Vec<Entity> = {
            let mut out = Vec::new();
            world.query::<(&OnGrid,)>().for_each(|e, _| out.push(e));
            out
        };
        for e in riding {
            world.commands().remove::<OnGrid>(e);
        }
        return;
    }
    let dt = world.resource::<mobs::FixedDt>().0;
    let player = world.resource::<mobs::PlayerEntities>().local();
    let control = world
        .get_resource::<ShipControl>()
        .map(|c| *c)
        .unwrap_or_default();
    let shared_world = world.streamer();
    let streamer = shared_world.borrow();

    // Les passagers, avant que la structure ne bouge : leur position dans son repère.
    let mut riders: Vec<(Entity, GridId, Vec3)> = Vec::new();
    world
        .query::<(&OnGrid, &Transform)>()
        .for_each(|e, (g, t)| match subs.get(g.0) {
            Some(cell) => riders.push((e, g.0, cell.borrow().to_local(t.position))),
            None => world.commands().remove::<OnGrid>(e),
        });

    // Chaque structure avance.
    let mut turned: HashMap<GridId, f32> = HashMap::new();
    for id in subs.ids() {
        let mut grid = subs.get(id).expect("identifiant connu").borrow_mut();
        let input = ShipInput {
            steer: if control.grid == Some(id) {
                control.steer
            } else {
                0.0
            },
        };
        let report = step_ship(&mut grid, streamer.world(), input, dt);
        turned.insert(id, report.turned);
    }

    // Les passagers suivent : même position locale, nouvelle position dans le monde.
    let mut player_turn = 0.0;
    for (entity, id, local) in &riders {
        let Some(cell) = subs.get(*id) else { continue };
        let grid = cell.borrow();
        let turn = turned.get(id).copied().unwrap_or(0.0);
        if let Some(mut t) = world.get_mut::<Transform>(*entity) {
            t.position = grid.to_world(*local);
            t.yaw += turn;
        }
        if Some(*entity) == player {
            player_turn += turn;
        }
        // Un passager qui a quitté le pont (sauté, tombé) redevient un habitant du monde.
        if deck_gap(&grid, *local).is_none_or(|gap| gap > 1.8) {
            world.commands().remove::<OnGrid>(*entity);
        }
    }
    if let Some(mut events) = world.get_resource_mut::<ShipEvents>() {
        events.player_turn += player_turn;
    }

    // Une entité qui se pose sur un pont y monte (en étant remise exactement dessus).
    let mut boarding: Vec<(Entity, GridId, Vec3)> = Vec::new();
    world
        .query::<(&Transform, &Collider)>()
        .for_each(|e, (t, _)| {
            if world.has::<OnGrid>(e) || world.has::<components::Flying>(e) {
                return;
            }
            for id in subs.ids() {
                let grid = subs.get(id).expect("identifiant connu").borrow();
                let local = grid.to_local(t.position);
                if let Some(gap) = deck_gap(&grid, local)
                    && (-0.5..0.2).contains(&gap)
                {
                    boarding.push((e, id, local - Vec3::Y * gap));
                    return;
                }
            }
        });
    for (entity, id, local) in boarding {
        let grid = subs.get(id).expect("identifiant connu").borrow();
        if let Some(mut t) = world.get_mut::<Transform>(entity) {
            t.position = grid.to_world(local);
        }
        world.commands().insert(entity, OnGrid(id));
    }
}

/// Les maillages des structures mobiles sur le GPU.
#[derive(Default)]
pub struct GridMeshes {
    built: HashMap<GridId, Built>,
}

struct Built {
    revision: u64,
    /// Les chunks locaux : leur origine locale et leur maillage.
    chunks: Vec<(IVec3, MeshId)>,
}

impl GridMeshes {
    /// Tous les maillages (pour le culling : ils sont toujours dessinés).
    pub fn ids(&self) -> impl Iterator<Item = MeshId> + '_ {
        self.built
            .values()
            .flat_map(|b| b.chunks.iter().map(|c| c.1))
    }

    /// Met les maillages à jour : remaille ce qui a changé, retire ce qui a disparu, et place
    /// chaque maillage selon la transformation actuelle de sa structure.
    pub fn sync(&mut self, renderer: &mut Renderer, grids: &[&SubGrid]) {
        let live: Vec<GridId> = grids.iter().map(|grid| grid.id).collect();
        let gone: Vec<GridId> = self
            .built
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in gone {
            if let Some(built) = self.built.remove(&id) {
                for (_, mesh) in built.chunks {
                    renderer.remove_mesh(mesh);
                }
            }
        }
        for grid in grids {
            let id = grid.id;
            let stale = self
                .built
                .get(&id)
                .is_none_or(|b| b.revision != grid.revision());
            if stale {
                if let Some(old) = self.built.remove(&id) {
                    for (_, mesh) in old.chunks {
                        renderer.remove_mesh(mesh);
                    }
                }
                self.built.insert(id, build(renderer, grid));
            }
            if let Some(built) = self.built.get(&id) {
                for &(origin, mesh) in &built.chunks {
                    let (world_origin, bounds) = place(grid, origin);
                    renderer.set_mesh_transform(mesh, world_origin, grid.yaw, bounds);
                }
            }
        }
    }
}

/// Où est l'origine d'un chunk local dans le monde, et sa boîte englobante dans le monde.
fn place(grid: &SubGrid, chunk_origin: IVec3) -> (Vec3, Aabb) {
    let size = CHUNK_SIZE as f32;
    let base = chunk_origin.as_vec3();
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for corner in 0..8 {
        // Une pièce peut dépasser de son cube pendant sa rotation (au plus
        // sqrt(0.5) - 0.5 bloc). Garder cette marge aussi sur les structures mobiles.
        let offset = Vec3::new(
            (corner & 1) as f32,
            ((corner >> 1) & 1) as f32,
            ((corner >> 2) & 1) as f32,
        ) * (size + 0.5)
            - Vec3::splat(0.25);
        let p = grid.to_world(base + offset);
        min = min.min(p);
        max = max.max(p);
    }
    (grid.to_world(base), Aabb::new(min, max))
}

fn build(renderer: &mut Renderer, grid: &SubGrid) -> Built {
    let mut chunks = Vec::new();
    let mut positions: Vec<IVec3> = grid.world().chunk_positions().collect();
    positions.sort_by_key(|p| p.to_array());
    for pos in positions {
        let mesh = engine_voxel::mesh_binary(grid.world(), pos);
        let Some((min, max)) = mesh.bounds() else {
            continue;
        };
        let _ = (min, max);
        let (origin, bounds) = place(grid, mesh.origin);
        if let Some(id) = renderer.add_mesh_in_frame(
            gpu_vertices(&mesh.vertices),
            &mesh.indices,
            &mesh.translucent_indices,
            origin,
            mesh.origin.as_vec3(),
            bounds,
        ) {
            chunks.push((mesh.origin, id));
        }
    }
    Built {
        revision: grid.revision(),
        chunks,
    }
}

impl VoxelGame {
    /// Détache du monde la structure qui contient `pos`.
    pub(crate) fn assemble(&mut self, pos: IVec3) -> Result<GridId, String> {
        let terrain = terrain_blocks(&self.registry);
        let shared = self.server.mods.subgrids.clone();
        let id = {
            let mut subs = shared.borrow_mut();
            engine_voxel::extract(&mut self.streamer_mut(), &mut subs, pos, &|b| {
                terrain.contains(&b)
            })
        }
        .map_err(|e| e.to_string())?;
        // Les machines de la structure démarrent, comme après un chargement.
        let subs = shared.borrow();
        let grid = subs.get(id).expect("grille créée").borrow();
        let active: Vec<GridPos> = grid
            .cells()
            .filter(|c| {
                self.server
                    .mods
                    .machines
                    .has_behavior(grid.world().block(*c))
            })
            .map(|c| GridPos::new(id, c))
            .collect();
        drop(grid);
        let mut actions = Vec::new();
        let grids = Grids::with_subgrids(&self.server.world, &subs);
        self.server
            .mods
            .machines
            .grid_blocks_loaded(grids, &active, &mut actions);
        drop(subs);
        self.apply_machine_actions(actions);
        self.client.target = None;
        Ok(id)
    }

    /// Repose une structure dans le monde. Renvoie le nombre de blocs posés.
    pub(crate) fn disassemble(&mut self, id: GridId) -> Result<usize, String> {
        let shared = self.server.mods.subgrids.clone();
        let grid = shared
            .borrow_mut()
            .remove(id)
            .ok_or_else(|| "structure inconnue".to_string())?;
        let placed = match engine_voxel::land(&mut self.streamer_mut(), &grid) {
            Ok(placed) => placed,
            Err(err) => {
                shared.borrow_mut().insert(grid);
                return Err(err.to_string());
            }
        };
        if self.server.helm == Some(id) {
            self.server.helm = None;
        }
        // Les machines se réveillent à leur nouvelle place.
        for pos in &placed {
            let block = self.streamer().world().block(*pos);
            if self.server.mods.machines.has_behavior(block) {
                let actions = self.server.mods.block_changed(*pos);
                self.apply_machine_actions(actions);
            }
        }
        self.client.target = None;
        Ok(placed.len())
    }

    /// Le bloc de structure visé, s'il est plus près que celui du monde.
    pub(crate) fn ship_hit(&self) -> Option<(GridId, engine_voxel::RayHit)> {
        let subs = self.server.mods.subgrids.borrow();
        let aim = self.aim();
        let (origin, direction) = (aim.eye, aim.forward);
        let mut best: Option<(GridId, engine_voxel::RayHit)> = None;
        for id in subs.ids() {
            let grid = subs.get(id).expect("identifiant connu").borrow();
            let hit = engine_voxel::raycast(
                grid.world(),
                grid.to_local(origin),
                grid.direction_to_local(direction),
                self.server.players.me().gear.reach(),
            );
            if let Some(hit) = hit
                && best.is_none_or(|(_, b)| hit.distance < b.distance)
            {
                best = Some((id, hit));
            }
        }
        let world_distance = aim.hit.map_or(f32::MAX, |h| h.distance);
        best.filter(|(_, hit)| hit.distance < world_distance)
    }

    /// Un clic droit sur un bloc de structure : l'assembleur la repose, la barre la dirige, un
    /// levier ou un bouton marche ; le reste ne se modifie pas tant qu'elle est assemblée.
    pub(crate) fn use_ship(&mut self) -> bool {
        let Some((id, hit)) = self.ship_hit() else {
            return false;
        };
        let (block, behavior) = {
            let subs = self.server.mods.subgrids.borrow();
            let grid = subs.get(id).expect("identifiant connu").borrow();
            let def = self
                .registry
                .def(self.registry.base(grid.world().block(hit.block_pos)));
            (def.key.clone(), def.behavior.clone())
        };
        match block.as_str() {
            "core:assembler" => match self.disassemble(id) {
                Ok(n) => self.toast(&format!("Structure reposée ({n} blocs)")),
                Err(err) => self.toast(&err),
            },
            "core:helm" => {
                if self.server.helm == Some(id) {
                    self.server.helm = None;
                    self.toast("Barre lâchée");
                } else {
                    self.server.helm = Some(id);
                    self.toast("À la barre : gauche et droite pour tourner");
                }
            }
            _ => {
                let used = behavior.is_some() && {
                    let subs = self.server.mods.subgrids.borrow();
                    let grids = Grids::with_subgrids(&self.server.world, &subs);
                    let mut actions = Vec::new();
                    let used = self.server.mods.machines.use_grid_block(
                        grids,
                        GridPos::new(id, hit.block_pos),
                        &mut actions,
                    );
                    drop(subs);
                    self.apply_machine_actions(actions);
                    used
                };
                if !used {
                    self.toast("Pose la structure (assembleur) pour la modifier");
                }
            }
        }
        true
    }

    /// Un clic droit sur un assembleur du monde : la structure qui y est reliée s'en détache.
    pub(crate) fn use_assembler(&mut self) -> bool {
        let Some(hit) = self.aim().hit else {
            return false;
        };
        let block = self.streamer().world().block(hit.block_pos);
        if self.registry.def(self.registry.base(block)).key != "core:assembler" {
            return false;
        }
        match self.assemble(hit.block_pos) {
            Ok(id) => {
                let size = self
                    .server
                    .mods
                    .subgrids
                    .borrow()
                    .get(id)
                    .map_or(0, |g| g.borrow().len());
                self.toast(&format!("Structure assemblée ({size} blocs)"));
            }
            Err(err) => self.toast(&err),
        }
        true
    }

    /// La barre : gauche et droite, tant que le pilote reste près de sa structure.
    pub(crate) fn update_ships(&mut self, ctx: &Context) {
        let player = player::body(&self.server.ecs, self.server.players.entity()).position;
        let mut steer = 0.0;
        if let Some(id) = self.server.helm {
            let near = self.server.mods.subgrids.borrow().get(id).is_some_and(|g| {
                let g = g.borrow();
                g.to_local(player).distance(g.pivot) < g.len() as f32 + 10.0
            });
            if near {
                let keys = &self.settings.keys;
                steer = f32::from(ctx.input.key_held(keys.key(Action::Right)) as u8)
                    - f32::from(ctx.input.key_held(keys.key(Action::Left)) as u8);
            } else {
                self.server.helm = None;
            }
        }
        if let Some(mut control) = self.server.ecs.get_resource_mut::<ShipControl>() {
            control.grid = control.forced.map(|f| f.0).or(self.server.helm);
            control.steer = control.forced.map_or(steer, |f| f.1);
        }
        // La caméra tourne avec la structure qui porte le joueur.
        let turn = self
            .server
            .ecs
            .get_resource_mut::<ShipEvents>()
            .map(|mut e| std::mem::take(&mut e.player_turn))
            .unwrap_or(0.0);
        self.client.camera.yaw += turn;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use super::*;
    use crate::components::Velocity;
    use crate::mobs::tests::sim_at;
    use crate::mobs::{MobRegistry, spawn_mob};
    use engine_voxel::SubGrids;

    /// Un pont de 6 × 6 planches, avec douze ballons qui le tiennent en l'air (la portance
    /// égale le poids : il flotte sans monter ni tomber), loin au-dessus du terrain.
    fn hovering_deck(world: &Ecs) -> (Rc<RefCell<SubGrids>>, GridId) {
        let registry = world.streamer().borrow().world().registry().clone();
        let mut subs = SubGrids::new(registry.clone());
        let id = subs.create();
        {
            let mut grid = subs.get(id).unwrap().borrow_mut();
            let (planks, balloon) = (
                registry.id("core:oak_planks").unwrap(),
                registry.id("core:balloon").unwrap(),
            );
            use engine_voxel::BlockAccess;
            for x in 0..6 {
                for z in 0..6 {
                    grid.set_block(IVec3::new(x, 0, z), planks);
                }
            }
            for x in 0..6 {
                for z in 0..2 {
                    grid.set_block(IVec3::new(x, 1, z), balloon);
                }
            }
            grid.recenter();
            grid.position = Vec3::new(5.0, 150.0, 5.0);
        }
        (Rc::new(RefCell::new(subs)), id)
    }

    #[test]
    fn passengers_ride_a_moving_and_turning_deck_and_fall_off_the_edge() {
        let mut world = sim_at(Vec3::new(5.0, 40.0, 5.0), 5);
        let (subs, id) = hovering_deck(&world);
        world.insert_resource(SharedSubGrids(subs.clone()));
        world.insert_resource(ShipControl::default());
        world.insert_resource(ShipEvents::default());
        // Un cochon posé sur le pont, côté libre (z = 4) : le dessus est à y local 1.
        let pig_kind = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let start = subs
            .borrow()
            .get(id)
            .unwrap()
            .borrow()
            .to_world(Vec3::new(3.0, 1.0, 4.5));
        let pig = spawn_mob(&mut world, pig_kind, start).unwrap();
        let local_of = |world: &Ecs| {
            let position = world.get::<Transform>(pig).unwrap().position;
            subs.borrow().get(id).unwrap().borrow().to_local(position)
        };
        let before = local_of(&world);

        for _ in 0..180 {
            // Le pont avance et tourne (on le pilote à la main : ce test vérifie les passagers).
            {
                let subs = subs.borrow();
                let mut grid = subs.get(id).unwrap().borrow_mut();
                grid.velocity.x = 3.0;
                grid.yaw_rate = 0.4;
            }
            update(&mut world);
            world.apply_commands();
            crate::mobs::physics(&mut world);
        }
        assert!(world.has::<OnGrid>(pig), "il est monté sur le pont");
        let grid = subs.borrow();
        let grid = grid.get(id).unwrap().borrow();
        assert!(
            grid.position.x > 12.0,
            "le pont a avancé : {}",
            grid.position.x
        );
        assert!(grid.yaw > 0.5, "et tourné : {}", grid.yaw);
        // Le cochon n'a presque pas bougé dans le repère du pont, alors qu'il a voyagé.
        let after = grid.to_local(world.get::<Transform>(pig).unwrap().position);
        assert!((after - before).length() < 0.3, "{before} → {after}");
        let world_move = world.get::<Transform>(pig).unwrap().position.x - start.x;
        assert!(world_move > 5.0, "il a voyagé : {world_move}");
        assert!(
            world
                .get::<crate::components::Collider>(pig)
                .unwrap()
                .on_ground
        );
        // Le passager a tourné avec le pont.
        assert!(world.get::<Transform>(pig).unwrap().yaw > 0.5);
        drop(grid);

        // Poussé hors du pont (à vitesse locale), il en descend : plus de `OnGrid`.
        {
            let subs = subs.borrow();
            let grid = subs.get(id).unwrap().borrow();
            let outside = grid.to_world(Vec3::new(3.0, 1.0, 12.0));
            drop(grid);
            world.get_mut::<Transform>(pig).unwrap().position = outside;
            world.get_mut::<Velocity>(pig).unwrap().0 = Vec3::ZERO;
        }
        update(&mut world);
        world.apply_commands();
        assert!(!world.has::<OnGrid>(pig), "il a quitté le pont");
    }

    #[test]
    fn deck_gap_finds_the_floor_under_the_feet() {
        let (subs, id) = hovering_deck(&sim_at(Vec3::new(5.0, 40.0, 5.0), 5));
        let subs = subs.borrow();
        let grid = subs.get(id).unwrap().borrow();
        // Debout sur le pont (dessus à y local 1) : écart nul.
        assert_eq!(deck_gap(&grid, Vec3::new(3.5, 1.0, 4.5)), Some(0.0));
        // Un saut de 0,8 : écart 0,8. Trop haut ou hors du pont : rien.
        assert!((deck_gap(&grid, Vec3::new(3.5, 1.8, 4.5)).unwrap() - 0.8).abs() < 1e-5);
        assert_eq!(deck_gap(&grid, Vec3::new(3.5, 5.0, 4.5)), None);
        assert_eq!(deck_gap(&grid, Vec3::new(20.5, 1.0, 4.5)), None);
    }

    #[test]
    fn terrain_blocks_are_left_in_the_world() {
        let registry = engine_voxel::BlockRegistry::builtin();
        let terrain = terrain_blocks(&registry);
        for key in ["core:stone", "core:dirt", "core:water", "core:sand"] {
            assert!(terrain.contains(&registry.id(key).unwrap()), "{key}");
        }
        for key in [
            "core:oak_planks",
            "core:glass",
            "core:cobblestone",
            "core:assembler",
        ] {
            assert!(!terrain.contains(&registry.id(key).unwrap()), "{key}");
        }
    }
}
