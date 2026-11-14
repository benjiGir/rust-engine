//! Le bras du joueur, vu à la première personne, et l'objet qu'il tient.
//!
//! Le bras n'existe pas dans le monde : il est décrit en **coordonnées de vue** (la
//! caméra à l'origine, regardant vers -z, x à droite, y en haut) et dessiné dans une
//! passe à part (voir `hand.wgsl`). Il est fait de boîtes, comme les créatures.
//!
//! Toutes les animations sont de petits décalages ajoutés à une pose de repos :
//! - **la marche** : un balancement en 8 qui suit la vitesse (plus ample en sprint) ;
//! - **le coup** : en minant (en boucle), ou une fois en posant un bloc ;
//! - **l'équipement** : le bras descend et remonte quand on change d'objet ;
//! - **l'inertie** : en tournant la tête, le bras traîne un peu derrière.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::path::Path;

use engine_render::{CubeInstance, HandInstance};

#[cfg(test)]
use engine_voxel::ToolKind;
use engine_voxel::{BlockRegistry, Category, Icon, ItemId, ItemRegistry};
use glam::{Mat4, Quat, Vec2, Vec3};
use serde::Deserialize;

/// Les couleurs du bras, lues dans `assets/core/player.ron`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ArmColors {
    pub skin: (f32, f32, f32),
    pub sleeve: (f32, f32, f32),
    pub cuff: (f32, f32, f32),
}

impl Default for ArmColors {
    fn default() -> Self {
        Self {
            skin: (0.86, 0.66, 0.52),
            sleeve: (0.24, 0.36, 0.33),
            cuff: (0.68, 0.49, 0.31),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlayerFile {
    arm: ArmColors,
}

impl ArmColors {
    pub fn load(assets: &Path) -> Self {
        let path = assets.join("core").join("player.ron");
        std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| Ok(ron::from_str::<PlayerFile>(&text)?.arm))
            .unwrap_or_else(|err| {
                log::warn!("{} : {err:#} (couleurs par défaut)", path.display());
                Self::default()
            })
    }
}

/// Ce que le jeu dit au bras à chaque image.
#[derive(Debug, Clone, Copy, Default)]
pub struct HandInput {
    pub dt: f32,
    /// Vitesse horizontale du joueur, en blocs par seconde.
    pub speed: f32,
    pub sprinting: bool,
    pub on_ground: bool,
    /// Le bouton de minage est tenu sur un bloc.
    pub mining: bool,
    /// La case choisie et son objet : un changement déclenche l'animation d'équipement.
    pub selected: (usize, Option<ItemId>),
    /// L'orientation de la caméra (radians).
    pub yaw: f32,
    pub pitch: f32,
}

/// Un pixel opaque d'une icône : sa position (x, y) et sa couleur (linéaire).
#[cfg(test)]
type Sprite = Vec<(u8, u8, [f32; 3])>;

pub struct Hand {
    colors: ArmColors,
    pub visible: bool,
    /// La phase du pas, en radians.
    pub walk_phase: f32,
    /// L'ampleur du balancement (lissée), de 0 (arrêt) à 1,6 (sprint).
    pub walk_amount: f32,
    /// Le coup en cours, de 0 à 1 (`None` : au repos).
    pub swing: Option<f32>,
    /// 1 : bras baissé (on vient de changer d'objet), 0 : en place.
    pub equip: f32,
    selected: Option<(usize, Option<ItemId>)>,
    /// Le retard du bras quand la tête tourne.
    sway: Vec2,
    last_look: Option<Vec2>,
    /// Modèles extrudés communs au sol, au transport et à la main (en cache).
    sprites: HashMap<ItemId, Vec<CubeInstance>>,
    aspect_ratio: f32,
    /// Figé dans sa pose actuelle (pour les captures).
    pub frozen: bool,
}

/// Durée d'un coup de bras, en secondes.
const SWING_TIME: f32 = 0.3;
const EQUIP_TIME: f32 = 0.25;

impl Hand {
    pub fn new(colors: ArmColors) -> Self {
        Self {
            colors,
            visible: std::env::var("VOXEL_HIDE_ARM").is_err(),
            walk_phase: 0.0,
            walk_amount: 0.0,
            swing: None,
            equip: 0.0,
            selected: None,
            sway: Vec2::ZERO,
            last_look: None,
            sprites: HashMap::new(),
            aspect_ratio: 16.0 / 9.0,
            frozen: false,
        }
    }

    /// Invalide les icônes extrudées et recharge les couleurs sans interrompre la pose.
    pub fn reload_assets(&mut self, colors: ArmColors) {
        self.colors = colors;
        self.sprites.clear();
    }

    /// Ancrage à droite, avec une taille lisible aussi dans une fenêtre étroite.
    pub fn set_aspect_ratio(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 {
            self.aspect_ratio = aspect.clamp(0.5, 4.0);
        }
    }

    /// Profil de référence Java : l'icône reste dans son repère centré, sans prise ni taille par outil.
    fn held_pose(&self) -> Mat4 {
        let t = self.swing.unwrap_or(0.0).clamp(0.0, 1.0);
        let arc = (t.sqrt() * PI).sin();
        let early = (t * t * PI).sin();
        let path = Vec3::new(
            -0.4 * arc,
            0.2 * (2.0 * PI * t.sqrt()).sin(),
            -0.2 * (PI * t).sin(),
        );
        let walk = Vec3::new(self.walk_phase.sin(), -self.walk_phase.cos().abs(), 0.0)
            * self.walk_amount
            * 0.035;
        let inertia = Vec3::new(self.sway.x, -self.sway.y, 0.0) * 0.25;
        let anchor = Vec3::new(0.56, -0.52 - 0.6 * self.equip, -0.72);
        let swing = Mat4::from_rotation_y((45.0 - 20.0 * early).to_radians())
            * Mat4::from_rotation_z((-20.0 * arc).to_radians())
            * Mat4::from_rotation_x((-80.0 * arc).to_radians())
            * Mat4::from_rotation_y(-45.0f32.to_radians());
        Mat4::from_translation(anchor + path + walk + inertia)
            * swing
            * Mat4::from_translation(Vec3::new(1.13, 3.2, 1.13) / 16.0)
            * Mat4::from_rotation_y(-90.0f32.to_radians())
            * Mat4::from_rotation_z(25.0f32.to_radians())
            * Mat4::from_scale(Vec3::splat(0.68))
    }

    /// Un coup de bras (on vient de poser un bloc, de frapper).
    pub fn punch(&mut self) {
        if self.swing.is_none() {
            self.swing = Some(0.0);
        }
    }

    /// Avance les animations d'une image.
    pub fn update(&mut self, input: HandInput) {
        if self.frozen {
            return;
        }
        let dt = input.dt.min(0.1);
        // Lissage exponentiel : on rejoint la cible d'une fraction qui dépend de dt,
        // pour que l'animation ait la même vitesse à 30 et à 120 images par seconde.
        let smooth = |current: f32, target: f32, rate: f32| {
            current + (target - current) * (1.0 - (-rate * dt).exp())
        };

        // La marche : seulement au sol ; l'ampleur suit la vitesse.
        let target = if input.on_ground {
            (input.speed / 4.3).min(1.0) * if input.sprinting { 1.6 } else { 1.0 }
        } else {
            0.0
        };
        self.walk_amount = smooth(self.walk_amount, target, 8.0);
        self.walk_phase = (self.walk_phase + dt * input.speed.max(0.0) * 1.7) % (2.0 * PI);

        // Le coup : il avance, et recommence en boucle tant qu'on mine.
        if input.mining && self.swing.is_none() {
            self.swing = Some(0.0);
        }
        if let Some(t) = self.swing.as_mut() {
            *t += dt / SWING_TIME;
            if *t >= 1.0 {
                self.swing = input.mining.then_some(*t - 1.0);
            }
        }

        // L'équipement : un nouvel objet en main fait redescendre le bras.
        if self.selected.is_some_and(|s| s != input.selected) {
            self.equip = 1.0;
        }
        self.selected = Some(input.selected);
        self.equip = (self.equip - dt / EQUIP_TIME).max(0.0);

        // L'inertie : la rotation de la tête depuis l'image précédente pousse le bras
        // dans l'autre sens, puis il revient.
        let look = Vec2::new(input.yaw, input.pitch);
        let delta = self.last_look.map_or(Vec2::ZERO, |last| look - last);
        self.last_look = Some(look);
        let push = (-delta * 2.0).clamp(Vec2::splat(-0.25), Vec2::splat(0.25));
        self.sway.x = smooth(self.sway.x + push.x, 0.0, 9.0);
        self.sway.y = smooth(self.sway.y + push.y, 0.0, 9.0);
    }

    /// La pose du bras : la matrice du bout de la main (là où l'objet est tenu).
    fn arm_pose(&self) -> Mat4 {
        let bob = self.walk_amount * 0.035;
        let walk = Vec3::new(
            self.walk_phase.sin() * bob,
            -self.walk_phase.cos().abs() * bob,
            0.0,
        );
        let swing = self.swing.map_or(0.0, |t| (t * PI).sin());
        let equip = self.equip * self.equip * (3.0 - 2.0 * self.equip);
        let offset = walk
            + Vec3::new(
                -swing * 0.1 + self.sway.x * 0.25,
                -swing * 0.03 - equip * 0.45 - self.sway.y * 0.25,
                -swing * 0.1,
            );
        let narrow = (1.0 - self.aspect_ratio / (16.0 / 9.0)).clamp(0.0, 1.0);
        let base = Vec3::new(
            0.42 * self.aspect_ratio / (16.0 / 9.0),
            -0.36 - narrow * 0.18,
            -0.6,
        ) + offset;
        // Le bras pointe vers l'avant et un peu vers le centre de l'écran ; le coup le
        // fait plonger (rotation autour de x) et pivoter vers la gauche.
        let rotation =
            Quat::from_rotation_y(0.3 + swing * 0.3) * Quat::from_rotation_x(0.18 - swing * 0.55);
        Mat4::from_rotation_translation(rotation, base)
    }

    /// Les boîtes à dessiner : le bras (manche, liseré, main) et l'objet tenu.
    pub fn instances(
        &mut self,
        held: Option<ItemId>,
        items: &ItemRegistry,
        blocks: &BlockRegistry,
    ) -> Vec<HandInstance> {
        if !self.visible {
            return Vec::new();
        }
        let pose = self.arm_pose();
        let color = |(r, g, b): (f32, f32, f32)| [r, g, b];
        // Les parties du bras, le long de l'axe -z du bras (vers l'avant).
        let part = |center: Vec3, size: Vec3| {
            pose * Mat4::from_translation(center) * Mat4::from_scale(size)
        };
        let mut out = vec![
            HandInstance::new(
                part(Vec3::new(0.0, 0.0, 0.18), Vec3::new(0.15, 0.15, 0.42)),
                color(self.colors.sleeve),
                0,
            ),
            HandInstance::new(
                part(Vec3::new(0.0, 0.0, -0.05), Vec3::new(0.155, 0.155, 0.04)),
                color(self.colors.cuff),
                0,
            ),
            HandInstance::new(
                part(Vec3::new(0.0, 0.0, -0.17), Vec3::new(0.13, 0.13, 0.2)),
                color(self.colors.skin),
                0,
            ),
        ];
        let Some(item) = held else { return out };
        let def = items.get(item);
        // Un outil piloté par script peut n'avoir aucun ToolDef : cela reste une pose, pas une capacité de minage.
        let handheld = def.tool.is_some() || def.category == Category::Tool;
        // Présentation des outils : l'objet remplace le bras, comme dans la vue classique.
        if handheld {
            out.clear();
        }
        // Les blocs et petits objets conservent leur présentation devant les doigts.
        // Le profil des outils est indépendant de ce repère de bras.
        let grip = pose * Mat4::from_translation(Vec3::new(-0.02, 0.07, -0.3));
        match def.icon {
            // Un conduit garde son segment d'inventaire ; les autres blocs sont présentés en cube.
            Icon::Block(block) => {
                let layer = blocks.def(block).face_textures[1];
                let tint = if layer == 0 {
                    blocks.def(block).color
                } else {
                    [1.0; 3]
                };
                let connected = blocks.def(block).connections.is_some();
                let model = grip
                    * Mat4::from_rotation_y(0.7)
                    * Mat4::from_scale(Vec3::splat(if connected { 0.36 } else { 0.2 }));
                if connected {
                    for b in &blocks.def(block).boxes {
                        let part = model
                            * Mat4::from_translation((b.min + b.max) * 0.5 - Vec3::splat(0.5))
                            * Mat4::from_scale(b.max - b.min);
                        out.push(HandInstance::new(part, tint, layer));
                    }
                } else {
                    out.push(HandInstance::new(model, tint, layer));
                }
            }
            // Un outil, un lingot... : son icône extrudée, un petit cube par pixel.
            Icon::Texture(_) => {
                let plane = if handheld {
                    self.held_pose()
                } else {
                    grip * Mat4::from_rotation_y(-0.5)
                        * Mat4::from_rotation_z(0.15)
                        * Mat4::from_translation(Vec3::new(0.0, 0.38 * 0.25, 0.0))
                        * Mat4::from_scale(Vec3::splat(0.38))
                };
                let parts = self
                    .sprites
                    .entry(item)
                    .or_insert_with(|| crate::item_models::build(item, items, blocks));
                for part in parts {
                    let model = plane
                        * Mat4::from_translation(Vec3::from_array(part.center))
                        * Mat4::from_scale(Vec3::from_array(part.size));
                    out.push(HandInstance::new(model, part.color, 0));
                }
            }
        }
        out
    }
}

/// Repère du manche dans les dessins originaux : contrôle d'asset uniquement.
/// Le profil d'affichage de référence utilise le centre du modèle.
#[cfg(test)]
fn handle_pixel(kind: ToolKind) -> Vec2 {
    match kind {
        ToolKind::Sword => Vec2::new(4.5, 12.5),
        ToolKind::Shovel => Vec2::new(5.5, 11.5),
        ToolKind::Hoe => Vec2::new(2.5, 11.5),
        ToolKind::Shears => Vec2::new(9.5, 10.5),
        _ => Vec2::new(3.5, 11.5),
    }
}

/// Les pixels opaques d'une icône d'objet (`items/<nom>.png`), en couleurs linéaires.
#[cfg(test)]
fn load_sprite(key: &str, blocks: &BlockRegistry) -> Sprite {
    let Some(pixels) = blocks
        .asset_path(key, "items", "png")
        .and_then(|path| crate::assets::load_png(&path).ok())
    else {
        return Vec::new();
    };
    // Le PNG est en sRGB ; le shader travaille en couleurs linéaires.
    let linear = |c: u8| (c as f32 / 255.0).powf(2.2);
    let size = engine_render::texture_side(&pixels).unwrap_or(16) as usize;
    (0..size * size)
        .filter(|&i| pixels[i * 4 + 3] > 128)
        .map(|i| {
            let p = &pixels[i * 4..i * 4 + 3];
            (
                (i % size) as u8,
                (i / size) as u8,
                [linear(p[0]), linear(p[1]), linear(p[2])],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_silhouettes_keep_a_visible_handle() {
        let registry = crate::test_registry();
        let items = ItemRegistry::builtin(&registry).unwrap();
        for (name, kind) in [
            ("stone_pickaxe", ToolKind::Pickaxe),
            ("stone_axe", ToolKind::Axe),
            ("copper_shovel", ToolKind::Shovel),
            ("golden_hoe", ToolKind::Hoe),
            ("iron_sword", ToolKind::Sword),
            ("shears", ToolKind::Shears),
        ] {
            let item = items.get(items.id(&format!("core:{name}")).unwrap());
            let Icon::Texture(key) = &item.icon else {
                panic!("icône d'outil attendue")
            };
            let sprite = load_sprite(key, &registry);
            let grip = handle_pixel(kind);
            assert!(
                sprite
                    .iter()
                    .any(|(x, y, _)| *x == grip.x as u8 && *y == grip.y as u8),
                "{name} : le manche doit traverser la main"
            );
        }
    }

    #[test]
    fn reference_display_uses_the_vanilla_center_axes_and_uniform_scale() {
        let hand = Hand::new(ArmColors::default());
        let m = hand.held_pose();
        // Repères indépendants calculés à partir du profil firstperson_righthand :
        // translation 1.13/3.2/1.13 en seizièmes, orientation (0,-90,25), échelle 0.68.
        assert!(
            m.transform_point3(Vec3::ZERO)
                .distance(Vec3::new(0.630625, -0.32, -0.649375))
                < 1e-5
        );
        assert!(
            m.transform_vector3(Vec3::Z)
                .distance(Vec3::new(-0.68, 0.0, 0.0))
                < 1e-5
        );
        assert!(
            m.transform_vector3(Vec3::X)
                .distance(Vec3::new(0.0, 0.2873804, 0.6162893))
                < 1e-5
        );
        assert!(
            m.transform_vector3(Vec3::Y)
                .distance(Vec3::new(0.0, 0.6162893, -0.2873804))
                < 1e-5
        );
        for aspect in [0.75, 1.0, 4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0] {
            let mut resized = Hand::new(hand.colors);
            resized.set_aspect_ratio(aspect);
            assert_eq!(
                m,
                resized.held_pose(),
                "le format change la projection, pas le profil de l'objet"
            );
        }
    }

    #[test]
    fn reference_swing_returns_to_rest_and_equipping_lowers_the_item() {
        let mut hand = Hand::new(ArmColors::default());
        let rest = hand.held_pose();
        hand.swing = Some(1.0);
        assert!(rest.abs_diff_eq(hand.held_pose(), 1e-5));
        hand.swing = Some(0.4);
        let mid = hand.held_pose();
        assert!(!rest.abs_diff_eq(mid, 0.01));
        assert!(mid.transform_point3(Vec3::ZERO).x < rest.transform_point3(Vec3::ZERO).x);
        hand.swing = None;
        hand.equip = 1.0;
        let lowered = hand.held_pose().transform_point3(Vec3::ZERO);
        assert!(
            (lowered - rest.transform_point3(Vec3::ZERO)).distance(Vec3::new(0.0, -0.6, 0.0))
                < 1e-5
        );
    }

    #[test]
    fn axe_blades_face_away_from_the_player_with_both_texture_packs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for packed in [false, true] {
            let mut blocks = crate::test_registry();
            if packed {
                blocks
                    .set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)
                    .unwrap();
            }
            let items = ItemRegistry::load_mods(
                &blocks,
                &root.join("assets"),
                &[("forge".into(), root.join("mods/forge"))],
            )
            .unwrap();
            let mut hand = Hand::new(ArmColors::default());
            hand.visible = true;
            for key in [
                "core:wooden_axe",
                "core:stone_axe",
                "core:copper_axe",
                "core:iron_axe",
                "core:golden_axe",
                "core:sapphire_axe",
                "forge:steel_axe",
            ] {
                let item = items.id(key).unwrap();
                // Centre de l'emmanchement dans chaque dessin ; le manche descend
                // en diagonale vers le bas gauche du sprite. On examine seulement
                // la tête et son col, à partir du vrai modèle extrudé et de la pose.
                let socket = if packed {
                    Vec3::new(20.5 / 32.0 - 0.5, 0.5 - 11.5 / 32.0, 0.0)
                } else {
                    Vec3::new(9.5 / 16.0 - 0.5, 0.5 - 4.5 / 16.0, 0.0)
                };
                let source = crate::item_models::build(item, &items, &blocks);
                let actual = hand.instances(Some(item), &items, &blocks);
                let socket_z = hand.held_pose().transform_point3(socket).z;
                let (mut front, mut back) = (0.0f32, 0.0f32);
                for (raw, held) in source.iter().zip(&actual) {
                    let from_socket = Vec3::from_array(raw.center) - socket;
                    let along_handle = from_socket.dot(Vec3::new(-1.0, -1.0, 0.0).normalize());
                    if !(-0.3..0.16).contains(&along_handle) {
                        continue;
                    }
                    let z = Mat4::from_cols_array_2d(&held.model)
                        .transform_point3(Vec3::ZERO)
                        .z
                        - socket_z;
                    front = front.max(-z);
                    back = back.max(z);
                }
                // La caméra regarde vers -Z : le grand débord de la lame doit
                // se trouver devant l'emmanchement, plutôt que vers le joueur.
                assert!(
                    front > 0.1 && front > back * 1.5,
                    "{key}, pack32={packed} : lame avant={front}, arrière={back}"
                );
            }
        }
    }

    #[test]
    fn tools_and_scripted_tools_use_the_same_reference_profile_and_shared_model() {
        let blocks = crate::test_registry();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let items = ItemRegistry::load_mods(
            &blocks,
            &root.join("assets"),
            &[("forge".into(), root.join("mods/forge"))],
        )
        .unwrap();
        let mut hand = Hand::new(ArmColors::default());
        hand.visible = true;
        for key in [
            "core:iron_pickaxe",
            "core:copper_shovel",
            "core:stone_axe",
            "core:iron_sword",
            "core:golden_hoe",
            "core:shears",
            "forge:bronze_hammer",
            "forge:steel_pickaxe",
            "forge:steel_axe",
            "forge:steel_sword",
        ] {
            let item = items.id(key).unwrap();
            let source = crate::item_models::build(item, &items, &blocks);
            let actual = hand.instances(Some(item), &items, &blocks);
            assert!(!actual.is_empty());
            assert_eq!(actual.len(), source.len());
            for (part, raw) in actual.iter().zip(&source) {
                let center = Mat4::from_cols_array_2d(&part.model).transform_point3(Vec3::ZERO);
                let expected = hand
                    .held_pose()
                    .transform_point3(Vec3::from_array(raw.center));
                assert!(
                    center.distance(expected) < 1e-5,
                    "{key} : origine centrée du modèle"
                );
                assert_eq!(&part.color[..3], &raw.color);
            }
        }
    }

    #[test]
    fn empty_hand_and_asset_reload_keep_working() {
        let blocks = crate::test_registry();
        let items = ItemRegistry::builtin(&blocks).unwrap();
        let mut hand = Hand::new(ArmColors::default());
        hand.visible = true;
        assert_eq!(hand.instances(None, &items, &blocks).len(), 3);
        let item = items.id("core:iron_sword").unwrap();
        let parts = hand.instances(Some(item), &items, &blocks);
        assert!(!hand.sprites.is_empty());
        hand.reload_assets(ArmColors::default());
        assert!(hand.sprites.is_empty());
        assert_eq!(
            hand.instances(Some(item), &items, &blocks).len(),
            parts.len()
        );
    }

    fn input(dt: f32) -> HandInput {
        HandInput {
            dt,
            on_ground: true,
            ..Default::default()
        }
    }

    #[test]
    fn mining_swings_in_a_loop_and_stops() {
        let mut hand = Hand::new(ArmColors::default());
        let mining = HandInput {
            mining: true,
            ..input(0.05)
        };
        for _ in 0..20 {
            hand.update(mining);
            assert!(hand.swing.is_some(), "le coup recommence tant qu'on mine");
        }
        for _ in 0..20 {
            hand.update(input(0.05));
        }
        assert!(hand.swing.is_none(), "au repos quand on lâche");
    }

    #[test]
    fn changing_item_lowers_then_raises_the_arm() {
        let mut hand = Hand::new(ArmColors::default());
        hand.update(HandInput {
            selected: (0, None),
            ..input(0.016)
        });
        hand.update(HandInput {
            selected: (1, None),
            ..input(0.016)
        });
        assert!(hand.equip > 0.9);
        for _ in 0..30 {
            hand.update(HandInput {
                selected: (1, None),
                ..input(0.016)
            });
        }
        assert_eq!(hand.equip, 0.0);
    }

    #[test]
    fn walking_sways_more_when_sprinting() {
        let mut walk = Hand::new(ArmColors::default());
        let mut sprint = Hand::new(ArmColors::default());
        for _ in 0..60 {
            walk.update(HandInput {
                speed: 4.3,
                ..input(0.016)
            });
            sprint.update(HandInput {
                speed: 5.6,
                sprinting: true,
                ..input(0.016)
            });
        }
        assert!(sprint.walk_amount > walk.walk_amount * 1.4);
    }
}
