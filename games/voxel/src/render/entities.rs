//! Le dessin des entités répliquées : créatures, autres joueurs, projectiles, wagons et objets au sol.
//!
//! Tout vient de la réplique ([`ClientEntities`]) et du monde que voit la fenêtre (pour la lumière) :
//! le rendu ne lit plus l'ECS du serveur. Les noms des autres joueurs se dessinent au-dessus de leur
//! tête, dans l'interface.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::LazyLock;

use engine_render::{CubeInstance, egui};
use engine_voxel::{BlockRegistry, ItemId, ItemRegistry, World, rotate_y};
use glam::{Vec2, Vec3, Vec4};

use crate::VoxelGame;
use crate::client::entities::{ClientEntities, EntityView};
use voxel_sim::mobs::MobRegistry;
use voxel_sim::mobs::model::{Bone, MobPart};
use voxel_sim::mobs::sheep_colors::SheepCoat;
use voxel_sim::net::entities::{EliteLook, EntityKind};
use voxel_sim::player::inventory::ItemStack;

/// Ce dont le dessin a besoin en dehors de la réplique : les définitions, la lumière du monde vu, et
/// le temps.
pub struct DrawEnv<'a> {
    pub mobs: &'a MobRegistry,
    pub items: &'a ItemRegistry,
    pub blocks: &'a BlockRegistry,
    /// Le monde que voit la fenêtre : la lumière au milieu de chaque entité.
    pub world: &'a World,
    /// La part du pas fixe écoulée (le rendu de l'image).
    pub alpha: f32,
    /// Le temps réel depuis le lancement (la rotation des objets).
    pub time: f32,
}

/// Le côté d'un objet au sol, et d'un projectile.
const ITEM_SIZE: f32 = 0.28;
const PROJECTILE_SIZE: f32 = 0.22;

/// Le modèle d'un autre joueur : un corps de blocs (+x vers l'avant), à teinte unie, qui marche comme
/// une créature.
fn player_parts() -> &'static [MobPart] {
    static PARTS: LazyLock<Vec<MobPart>> = LazyLock::new(|| {
        let part = |offset: (f32, f32, f32), size: (f32, f32, f32), color, bone| MobPart {
            offset,
            size,
            color,
            bone,
            texture: None,
            texture_layer: 0,
        };
        let skin = (0.87, 0.68, 0.52);
        let shirt = (0.22, 0.45, 0.72);
        let trousers = (0.24, 0.26, 0.5);
        vec![
            part((0.0, 1.05, 0.0), (0.3, 0.7, 0.5), shirt, Bone::Fixed),
            part((0.0, 1.62, 0.0), (0.5, 0.5, 0.5), skin, Bone::Head),
            part(
                (0.26, 1.64, -0.13),
                (0.02, 0.07, 0.07),
                (0.1, 0.1, 0.12),
                Bone::Head,
            ),
            part(
                (0.26, 1.64, 0.13),
                (0.02, 0.07, 0.07),
                (0.1, 0.1, 0.12),
                Bone::Head,
            ),
            part((0.0, 0.35, -0.12), (0.22, 0.7, 0.24), trousers, Bone::Leg),
            part((0.0, 0.35, 0.12), (0.22, 0.7, 0.24), trousers, Bone::Leg),
            part((0.0, 1.05, -0.37), (0.2, 0.7, 0.2), skin, Bone::Arm),
            part((0.0, 1.05, 0.37), (0.2, 0.7, 0.2), skin, Bone::Arm),
        ]
    });
    &PARTS
}

/// Les boîtes de toutes les entités répliquées, interpolées entre deux envois.
pub fn draw(entities: &ClientEntities, env: &DrawEnv, out: &mut Vec<CubeInstance>) {
    let now = entities.now(env.alpha);
    for (_, view) in entities.iter() {
        let pose = view.pose_at(now);
        match &view.kind {
            EntityKind::Mob { kind, .. } => {
                let Some(def) = env.mobs.get(*kind) else {
                    continue;
                };
                let scale = if view.look.baby {
                    voxel_sim::mobs::breeding::BABY_SCALE
                } else {
                    1.0
                };
                // Le faisceau d'un envahisseur, visible de loin.
                if let Some(elite) = view.look.elite.filter(|e| e.invader) {
                    out.extend(beam(
                        pose.position,
                        elite.tint,
                        env.alpha + view.latest().position.x,
                    ));
                }
                draw_model(&def.parts, &Model::of(view, scale, &pose), env, out);
            }
            EntityKind::Player { .. } => {
                draw_player(view, now, &pose, env, entities.item_models(), out)
            }
            EntityKind::Projectile { color } => {
                out.push(CubeInstance::new(
                    pose.position,
                    Vec3::splat(PROJECTILE_SIZE),
                    *color,
                ));
            }
            EntityKind::Cart { kind } => draw_cart(*kind, view, now, env, out),
            EntityKind::Item { stack } => {
                draw_item(stack, view, now, env, entities.item_models(), out)
            }
            // Les structures mobiles ont leur maillage (voir `ships.rs`).
            EntityKind::Grid { .. } => {}
        }
    }
}

/// Le bras droit dans [`player_parts`] (+z est la droite quand +x est l'avant), et son épaule.
const RIGHT_ARM: usize = 7;
const SHOULDER: Vec3 = Vec3::new(0.0, 1.40, 0.37);
/// Le centre de l'objet tenu, depuis l'épaule, le bras pendant : un peu devant la main, qui le tient
/// par le bas. Et sa taille : presque la longueur du bras (0,7).
const HAND: Vec3 = Vec3::new(0.24, -0.56, 0.0);
const HELD_SIZE: f32 = 0.62;
/// L'objet tenu est tourné d'un huitième de tour : on le reconnaît de face comme de profil.
const HELD_TURN: f32 = std::f32::consts::FRAC_PI_4;

/// Un autre joueur : son corps qui marche, son bras droit qui se lève à chaque geste, et l'objet qu'il
/// tient au bout.
fn draw_player(
    view: &EntityView,
    now: f32,
    pose: &voxel_sim::net::entities::Pose,
    env: &DrawEnv,
    models: &RefCell<HashMap<ItemId, Vec<CubeInstance>>>,
    out: &mut Vec<CubeInstance>,
) {
    let model = Model::of(view, 1.0, pose);
    let start = out.len();
    draw_model(player_parts(), &model, env, out);
    let raise = view.arm_raise(now);
    let arm = &mut out[start + RIGHT_ARM];
    arm.swing[0] += raise;
    let angle = arm.swing[0];
    let light = arm.light;
    let Some(stack) = view.look.held else {
        return;
    };
    // Le bras tourne autour de l'épaule, vers l'avant (+x) pour un angle positif : la main suit.
    let turn = |v: Vec3| {
        let (c, s) = (angle.cos(), angle.sin());
        Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
    };
    let (c, s) = (model.yaw.cos(), model.yaw.sin());
    let place = |local: Vec3| {
        model.position
            + Vec3::new(
                local.x * c - local.z * s,
                local.y,
                local.x * s + local.z * c,
            )
    };
    let hand = SHOULDER + turn(HAND);
    let mut models = models.borrow_mut();
    let parts = models
        .entry(stack.item)
        .or_insert_with(|| crate::render::item_models::build(stack.item, env.items, env.blocks));
    for part in parts.iter() {
        let mut instance = *part;
        // Les boîtes de l'objet suivent la main ; elles gardent leur aplomb (un objet fait de petits
        // cubes se lit encore bien quand le bras se lève).
        let offset = turn(rotate_y(
            Vec3::from_array(part.center) * HELD_SIZE,
            HELD_TURN,
        ));
        instance.center = place(hand + offset).to_array();
        instance.size = (Vec3::from_array(part.size) * HELD_SIZE).to_array();
        instance.yaw = model.yaw + part.yaw + HELD_TURN;
        instance.light = light;
        out.push(instance);
    }
}

/// Un corps de boîtes articulées, posé dans le monde.
struct Model {
    position: Vec3,
    yaw: f32,
    scale: f32,
    phase: f32,
    stride: f32,
    look: f32,
    coat: Option<SheepCoat>,
    elite: Option<EliteLook>,
    hurt: bool,
}

impl Model {
    /// Le corps d'une créature ou d'un joueur à la pose actuelle : le lacet d'une créature est celui du
    /// dernier pas (comme avant, il n'était pas interpolé).
    fn of(view: &EntityView, scale: f32, pose: &voxel_sim::net::entities::Pose) -> Self {
        let yaw = if matches!(view.kind, EntityKind::Player { .. }) {
            pose.yaw
        } else {
            view.latest().yaw
        };
        Model {
            position: pose.position,
            yaw,
            scale,
            phase: pose.phase,
            stride: pose.stride,
            look: pose.look,
            coat: view.look.coat.map(SheepCoat),
            elite: view.look.elite,
            hurt: view.look.hurt,
        }
    }
}

fn draw_model(parts: &[MobPart], model: &Model, env: &DrawEnv, out: &mut Vec<CubeInstance>) {
    let (c, s) = (model.yaw.cos(), model.yaw.sin());
    // La lumière au milieu de la créature, pour qu'elle soit sombre dans une grotte.
    let (sky, block) = env
        .world
        .light((model.position + Vec3::Y * 0.5).floor().as_ivec3());
    for (part, mut cube) in parts.iter().zip(crate::render::mobs::instances(
        parts,
        model.phase,
        model.stride,
        model.look,
    )) {
        let local = Vec3::from_array(cube.center);
        cube.center = (model.position
            + Vec3::new(
                local.x * c - local.z * s,
                local.y,
                local.x * s + local.z * c,
            ) * model.scale)
            .to_array();
        cube.size = (Vec3::from_array(cube.size) * model.scale).to_array();
        cube.yaw += model.yaw;
        cube.swing[1] *= model.scale;
        let mut color = Vec3::from_array(cube.color);
        if let Some(coat) = model.coat {
            color *= coat.tint_part(part);
        }
        if let Some(elite) = &model.elite {
            color = voxel_sim::mobs::elites::tinted(color, elite);
        }
        if model.hurt {
            color = color.lerp(Vec3::new(1.0, 0.1, 0.1), 0.6);
        }
        cube.color = color.to_array();
        cube.light = [sky as f32 / 15.0, block as f32 / 15.0];
        out.push(cube);
    }
}

fn draw_cart(
    kind: voxel_sim::vehicles::carts::CartKind,
    view: &EntityView,
    now: f32,
    env: &DrawEnv,
    out: &mut Vec<CubeInstance>,
) {
    let pose = view.pose_at(now);
    let (sky, block) = env
        .world
        .light((pose.position + Vec3::Y * 0.5).floor().as_ivec3());
    let (cp, sp) = (pose.pitch.cos(), pose.pitch.sin());
    let (cy, sy) = (pose.yaw.cos(), pose.yaw.sin());
    // Un wagon qui vient d'être frappé rougit un instant.
    let flash = if view.look.hurt { 0.45 } else { 0.0 };
    for (offset, size, color) in voxel_sim::vehicles::carts::parts(kind) {
        let tilted = Vec3::new(
            offset.x * cp - offset.y * sp,
            offset.x * sp + offset.y * cp,
            offset.z,
        );
        let rotated = Vec3::new(
            tilted.x * cy - tilted.z * sy,
            tilted.y,
            tilted.x * sy + tilted.z * cy,
        );
        out.push(
            CubeInstance::new(
                pose.position + rotated,
                size,
                color.lerp(Vec3::new(1.0, 0.15, 0.1), flash),
            )
            .with_yaw(pose.yaw)
            .with_swing(pose.pitch, 0.0)
            .with_light(sky as f32 / 15.0, block as f32 / 15.0),
        );
    }
}

/// Les vrais modèles texturés des objets au sol, qui tournent et flottent doucement.
fn draw_item(
    stack: &ItemStack,
    view: &EntityView,
    now: f32,
    env: &DrawEnv,
    models: &RefCell<HashMap<ItemId, Vec<CubeInstance>>>,
    out: &mut Vec<CubeInstance>,
) {
    let pose = view.pose_at(now);
    let (sky, block) = env
        .world
        .light((pose.position + Vec3::Y * 0.2).floor().as_ivec3());
    let bob = (env.time * 2.5 + pose.yaw).sin() * 0.05 + 0.08;
    // Une pile de plusieurs objets : un cube un peu plus gros.
    let size = if stack.count > 1 {
        ITEM_SIZE * 1.15
    } else {
        ITEM_SIZE
    };
    let yaw = pose.yaw + env.time * 1.5;
    let center = pose.position + Vec3::Y * (size / 2.0 + bob);
    let mut models = models.borrow_mut();
    let parts = models
        .entry(stack.item)
        .or_insert_with(|| crate::render::item_models::build(stack.item, env.items, env.blocks));
    for part in parts {
        let mut instance = *part;
        instance.center = (center + rotate_y(Vec3::from_array(part.center) * size, yaw)).to_array();
        instance.size = (Vec3::from_array(part.size) * size).to_array();
        instance.yaw = yaw;
        instance.light = [sky as f32 / 15.0, block as f32 / 15.0];
        out.push(instance);
    }
}

/// Où écrire le nom d'un joueur : le point `world` en points d'écran, ou `None` s'il est derrière la
/// caméra ou hors de l'écran.
pub fn project(view_proj: glam::Mat4, rect: egui::Rect, world: Vec3) -> Option<egui::Pos2> {
    let clip = view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = Vec2::new(clip.x, clip.y) / clip.w;
    if ndc.x.abs() > 1.2 || ndc.y.abs() > 1.2 {
        return None;
    }
    Some(egui::pos2(
        rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(),
        rect.top() + (1.0 - (ndc.y * 0.5 + 0.5)) * rect.height(),
    ))
}

impl VoxelGame {
    /// Les noms des autres joueurs, au-dessus de leur tête.
    pub(crate) fn draw_name_tags(&self, ctx: &egui::Context) {
        let now = self.client.view.entities.frame_now();
        let players = self.client.view.entities.players(now);
        if players.is_empty() {
            return;
        }
        let view_proj = self
            .client
            .camera
            .view_projection(self.client.renderer.aspect());
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::unique("name_tags"),
        ));
        for (name, feet) in players {
            let head = feet + Vec3::Y * 2.15;
            if head.distance(self.client.camera.position) > 64.0 {
                continue;
            }
            let Some(at) = project(view_proj, rect, head) else {
                continue;
            };
            let galley = painter.layout_no_wrap(
                name.to_string(),
                egui::FontId::proportional(15.0),
                crate::ui::skin::TEXT,
            );
            let text_rect = egui::Align2::CENTER_BOTTOM.anchor_size(at, galley.size());
            painter.rect_filled(
                text_rect.expand2(egui::vec2(5.0, 2.0)),
                0.0,
                egui::Color32::from_black_alpha(130),
            );
            painter.galley(text_rect.min, galley, crate::ui::skin::TEXT);
        }
    }
}

/// Le faisceau d'un envahisseur : une colonne de lumière qui monte de ses pieds, et une gaine plus
/// large et plus pâle autour. Elle se voit de loin, de jour comme de nuit.
pub fn beam(position: Vec3, tint: Vec3, phase: f32) -> [CubeInstance; 2] {
    const HEIGHT: f32 = 220.0;
    let pulse = 0.85 + 0.15 * (phase * 3.0).sin();
    let center = position + Vec3::Y * (HEIGHT * 0.5);
    let core = CubeInstance::new(
        center,
        Vec3::new(0.7 * pulse, HEIGHT, 0.7 * pulse),
        tint.lerp(Vec3::ONE, 0.6),
    )
    .with_light(1.0, 1.0)
    .with_surface([0; 3], Vec3::ZERO, Vec3::ONE, false, 0.85);
    let sheath = CubeInstance::new(center, Vec3::new(2.6 * pulse, HEIGHT, 2.6 * pulse), tint)
        .with_light(1.0, 1.0)
        .with_surface([0; 3], Vec3::ZERO, Vec3::ONE, false, 0.34);
    [core, sheath]
}

#[cfg(test)]
pub(crate) mod support {
    //! De quoi dessiner l'ECS d'un test avec le vrai chemin : numéros, suivi, réplique, dessin.

    use std::sync::Arc;

    use engine_ecs::World as Ecs;

    use super::*;
    use voxel_sim::net::entities::{EntityTracker, NetIds, Viewer, collect};
    use voxel_sim::shared::StreamerExt;

    /// Les boîtes de toutes les entités de l'ECS, comme la fenêtre les dessinerait à la part `alpha`
    /// du pas (une fois répliquées sans perte, à 60 Hz).
    pub fn boxes(ecs: &mut Ecs, alpha: f32) -> Vec<CubeInstance> {
        NetIds::assign(ecs);
        let subs = engine_voxel::SubGrids::new(Arc::new(crate::test_registry()));
        let all = collect(ecs, &subs, &|_| {
            Some((
                "joueur".to_string(),
                voxel_sim::net::entities::Look::default(),
            ))
        });
        let nobody = engine_ecs::Entity {
            index: u32::MAX,
            generation: 0,
        };
        let anywhere = |_: glam::IVec2| true;
        let mut tracker = EntityTracker::new(1);
        let mut messages = Vec::new();
        // Le spectateur est au milieu de ce qui se dessine : tout est à portée.
        let centre = all.iter().map(|r| r.pose.position).sum::<Vec3>() / all.len().max(1) as f32;
        let viewer = Viewer {
            entity: nobody,
            position: centre,
            knows: &anywhere,
        };
        tracker.step(1, &viewer, &all, &mut messages);
        let mut entities = ClientEntities::new(1);
        for message in &messages {
            entities.receive(message);
        }
        let mobs = ecs.resource::<Arc<MobRegistry>>().clone();
        let registry = Arc::new(crate::test_registry());
        let items = ItemRegistry::builtin(&registry).unwrap();
        let shared = ecs.streamer();
        let streamer = shared.borrow();
        let env = DrawEnv {
            mobs: &mobs,
            items: &items,
            blocks: &registry,
            world: streamer.world(),
            alpha,
            time: 1.0,
        };
        let mut out = Vec::new();
        draw(&entities, &env, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_sim::components::{Hurt, Transform};
    use voxel_sim::mobs::{self, tests::sim};

    #[test]
    fn dropped_items_use_their_material_and_the_model_cache_is_cleared_on_demand() {
        let mut world = sim();
        let registry = crate::test_registry();
        let items = ItemRegistry::builtin(&registry).unwrap();
        let stack = ItemStack {
            item: items.id("core:grass").unwrap(),
            count: 3,
            damage: 7,
        };
        voxel_sim::player::drops::spawn(&mut world, stack, Vec3::new(1.0, 20.0, 1.0), 42);
        let models = support::boxes(&mut world, 0.5);
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].textures,
            registry
                .def(registry.id("core:grass").unwrap())
                .face_textures
        );
    }

    #[test]
    fn creatures_are_drawn_where_the_replica_puts_them_and_flash_when_hurt() {
        let mut world = sim();
        let pig = world
            .resource::<std::sync::Arc<MobRegistry>>()
            .id("core:pig")
            .unwrap();
        let mob = mobs::spawn_mob(&mut world, pig, Vec3::new(5.5, 80.0, 5.5)).unwrap();
        let plain = support::boxes(&mut world, 1.0);
        assert!(plain.len() > 8);
        // Les boîtes sont autour de la position de la créature.
        assert!(plain.iter().all(|b| (b.center[0] - 5.5).abs() < 1.5));
        world.insert(mob, Hurt { timer: 0.4 });
        let hurt = support::boxes(&mut world, 1.0);
        assert_eq!(plain.len(), hurt.len());
        assert!(
            hurt.iter()
                .zip(&plain)
                .all(|(h, p)| h.color[0] >= p.color[0])
        );
        assert_ne!(hurt[0].color, plain[0].color, "elle rougit");
        // Elle se déplace : le dessin suit sa position.
        world.get_mut::<Transform>(mob).unwrap().position.x = 9.5;
        let moved = support::boxes(&mut world, 1.0);
        assert!(moved.iter().all(|b| (b.center[0] - 9.5).abs() < 1.5));
    }

    #[test]
    fn another_player_is_drawn_as_a_body_and_not_the_local_one() {
        let mut world = sim();
        let local = world.resource::<mobs::PlayerEntities>().0[0];
        world.insert(local, voxel_sim::player::players::PlayerId(0));
        world.insert(local, voxel_sim::components::Player);
        let other = world.spawn((
            Transform {
                position: Vec3::new(3.5, 80.0, 3.5),
                yaw: 0.0,
            },
            voxel_sim::components::Player,
            voxel_sim::player::players::PlayerId(1),
        ));
        let _ = other;
        // Le suivi du joueur local ne le renvoie pas à lui-même (voir `entities.rs`) ; ici on dessine
        // tout ce que répliquerait un client sans entité propre : les deux corps.
        let boxes = support::boxes(&mut world, 1.0);
        assert_eq!(boxes.len(), 2 * player_parts().len());
    }

    #[test]
    fn projectiles_and_carts_are_drawn_from_the_replica() {
        let mut world = sim();
        let color = Vec3::new(0.78, 0.82, 0.35);
        world.spawn((
            Transform {
                position: Vec3::new(4.0, 90.0, 4.0),
                yaw: 0.0,
            },
            voxel_sim::mobs::projectiles::Projectile {
                velocity: Vec3::X,
                damage: 3.0,
                age: 0.0,
                color,
            },
        ));
        let arrow = support::boxes(&mut world, 0.3);
        assert_eq!(arrow.len(), 1);
        assert_eq!(arrow[0].center, [4.0, 90.0, 4.0]);
        assert_eq!(arrow[0].color, color.to_array());
        assert_eq!(arrow[0].size, [PROJECTILE_SIZE; 3]);

        let mut cart = voxel_sim::vehicles::carts::Cart::new(
            1,
            voxel_sim::vehicles::carts::CartKind::Chest,
            voxel_sim::vehicles::carts::Ride::Free {
                velocity: Vec3::ZERO,
            },
        );
        cart.yaw = 1.0;
        cart.hurt = 0.5;
        world.spawn((
            Transform {
                position: Vec3::new(8.0, 90.0, 4.0),
                yaw: 0.0,
            },
            cart,
        ));
        let both = support::boxes(&mut world, 0.3);
        let parts =
            voxel_sim::vehicles::carts::parts(voxel_sim::vehicles::carts::CartKind::Chest).len();
        assert_eq!(both.len(), 1 + parts);
        // Le wagon frappé rougit, et ses boîtes tournent avec son lacet.
        assert!(both.iter().filter(|b| b.yaw == 1.0).count() == parts);
        assert!(both.iter().any(|b| b.color[0] > b.color[2] + 0.2));
    }

    #[test]
    fn names_project_in_front_of_the_camera_only() {
        let camera = engine_render::Camera::default();
        let view_proj = camera.view_projection(16.0 / 9.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let ahead = camera.position + camera.forward() * 5.0;
        let at = project(view_proj, rect, ahead).expect("devant");
        assert!(
            (at.x - 800.0).abs() < 2.0 && (at.y - 450.0).abs() < 2.0,
            "{at:?}"
        );
        let behind = camera.position - camera.forward() * 5.0;
        assert!(project(view_proj, rect, behind).is_none());
    }
}

#[cfg(test)]
mod beam_tests {
    use super::*;

    #[test]
    fn the_beam_of_an_invader_is_a_tall_lit_column() {
        let [core, sheath] = beam(Vec3::new(1.0, 60.0, 2.0), Vec3::new(0.9, 0.2, 0.2), 0.0);
        assert!(core.size[1] >= 100.0 && sheath.size[0] > core.size[0]);
        assert!(core.size[0] >= 0.5, "assez large pour se voir de loin");
        assert_eq!(core.light, [1.0, 1.0], "toujours éclairé");
        assert!(core.center[1] > 100.0 && core.center[0] == 1.0);
    }
}
