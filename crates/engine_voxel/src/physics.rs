//! La physique du joueur : une boîte qui tombe, saute et bute contre les blocs.
//!
//! Le joueur est une AABB de 0,6 × 1,8 × 0,6 blocs. À chaque pas de temps fixe :
//! 1. on calcule la vitesse (entrées du joueur, gravité, saut) ;
//! 2. on déplace la boîte **un axe après l'autre** (Y, puis X, puis Z). Après chaque
//!    axe, si la boîte chevauche un bloc plein, on la ramène contre la face de ce bloc.
//!    Traiter les axes séparément permet de glisser le long d'un mur : seul l'axe
//!    bloqué s'arrête, les autres continuent.
//! 3. un grand déplacement est découpé en petits pas (moins d'un bloc et moins que
//!    la largeur du joueur) : même à grande vitesse, on ne peut pas traverser un mur
//!    en « sautant par-dessus » entre deux images.

use glam::{IVec3, Vec3};

use crate::world::World;

pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_HEIGHT: f32 = 1.8;
/// Hauteur des yeux au-dessus des pieds.
pub const PLAYER_EYE_HEIGHT: f32 = 1.62;
/// En blocs par seconde au carré.
pub const GRAVITY: f32 = 32.0;
/// Vitesse verticale au départ d'un saut : environ 1,25 bloc de hauteur (v² / 2g).
pub const JUMP_SPEED: f32 = 9.0;
/// Vitesse de chute maximale.
const MAX_FALL_SPEED: f32 = 60.0;
/// Dans l'eau : vitesses multipliées par ce facteur.
const SWIM_FACTOR: f32 = 0.4;
/// Vitesse de remontée quand on tient Espace dans l'eau.
const SWIM_UP_SPEED: f32 = 4.0;
/// Vitesse de chute maximale dans l'eau.
const SWIM_SINK_SPEED: f32 = 2.5;
/// En dessous de cette part du corps immergée, on marche (une flaque, un ruisseau).
pub const SWIM_DEPTH: f32 = 0.15;
/// La poussée relative d'un corps dans l'eau : un peu plus que son poids une fois
/// immergé aux 9/10 (il flotte, la tête dehors).
const BODY_FLOAT: f32 = 1.1;
/// Plus grand pas de déplacement testé d'un coup.
const MAX_STEP: f32 = 0.4;
/// Petit écart laissé entre le joueur et un bloc, pour qu'il ne soit pas
/// considéré comme « dedans » à cause des arrondis des flottants.
const SKIN: f32 = 1e-3;

/// Le corps physique du joueur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerBody {
    /// Le centre de la base de la boîte (les pieds).
    pub position: Vec3,
    pub velocity: Vec3,
    /// Vrai si le joueur repose sur un bloc : lui seul permet de sauter.
    pub on_ground: bool,
    /// Vrai si le corps est assez enfoncé dans un liquide pour nager (au-delà de
    /// `SWIM_DEPTH` : marcher dans une flaque n'est pas nager).
    pub in_liquid: bool,
    /// La part de la hauteur du corps sous la surface, de 0 à 1.
    pub submersion: f32,
    /// Largeur (x et z) et hauteur de la boîte : celles du joueur par défaut. Un cochon
    /// ou un slime utilisent la même physique avec une autre taille.
    pub width: f32,
    pub height: f32,
}

impl PlayerBody {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            on_ground: false,
            in_liquid: false,
            submersion: 0.0,
            width: PLAYER_WIDTH,
            height: PLAYER_HEIGHT,
        }
    }

    /// Une boîte d'une autre taille (une créature).
    pub fn with_size(position: Vec3, width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            ..Self::new(position)
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * PLAYER_EYE_HEIGHT
    }

    /// Coins minimum et maximum de la boîte.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let half = self.width / 2.0;
        let min = self.position - Vec3::new(half, 0.0, half);
        let max = self.position + Vec3::new(half, self.height, half);
        (min, max)
    }

    /// Vrai si le bloc `block` occupe une partie de la boîte (pour interdire d'y poser un bloc).
    pub fn overlaps_block(&self, block: IVec3) -> bool {
        let (min, max) = self.bounds();
        let block_min = block.as_vec3();
        let block_max = block_min + Vec3::ONE;
        min.cmplt(block_max).all() && max.cmpgt(block_min).all()
    }

    /// Avance la simulation de `dt` secondes.
    ///
    /// `walk` est la vitesse horizontale voulue (blocs par seconde, `y` ignoré) :
    /// pas d'inertie, le joueur s'arrête dès qu'on lâche la touche, comme dans Minecraft.
    ///
    /// Dans l'eau : on avance moins vite, on coule lentement, et `jump` (Espace tenu)
    /// fait remonter : c'est une nage très simple.
    pub fn step(&mut self, world: &World, walk: Vec3, jump: bool, dt: f32) {
        let (submersion, fluid) = self.submersion(world);
        self.submersion = submersion;
        self.in_liquid = submersion > SWIM_DEPTH;
        if self.in_liquid {
            // La nage dépend du fluide : la viscosité freine (la lave est 6 fois plus
            // visqueuse que l'eau), la densité porte (la poussée d'Archimède).
            let (viscosity, density) = fluid;
            let drag = (5.0 / viscosity.max(1.0)).sqrt().min(1.0);
            self.velocity.x = walk.x * SWIM_FACTOR * drag;
            self.velocity.z = walk.z * SWIM_FACTOR * drag;
            // La poussée : proportionnelle au volume immergé et à la densité du fluide
            // (un corps humain flotte à peine dans l'eau : on s'y enfonce jusqu'aux yeux).
            let buoyancy = GRAVITY * BODY_FLOAT * (density / 1000.0) * submersion;
            self.velocity.y += (buoyancy - GRAVITY) * SWIM_FACTOR * dt;
            if jump {
                self.velocity.y = self.velocity.y.max(SWIM_UP_SPEED * drag);
            }
            // Le frottement du fluide amortit la vitesse verticale.
            self.velocity.y *= 1.0 - (2.0 * dt / drag).min(0.9);
            self.velocity.y = self.velocity.y.clamp(-SWIM_SINK_SPEED, SWIM_UP_SPEED);
        } else {
            self.velocity.x = walk.x;
            self.velocity.z = walk.z;
            if jump && self.on_ground {
                self.velocity.y = JUMP_SPEED;
            }
            self.velocity.y = (self.velocity.y - GRAVITY * dt).max(-MAX_FALL_SPEED);
        }

        let delta = self.velocity * dt;
        self.on_ground = false;
        // Y d'abord : on sait tout de suite si l'on est posé avant de glisser en X et Z.
        for axis in [1, 0, 2] {
            self.move_axis(world, axis, delta[axis]);
        }
    }

    /// La part du corps sous la surface (de 0 à 1), et la viscosité et la densité du
    /// fluide. La surface d'une cellule dépend de sa quantité de fluide : on peut avoir
    /// de l'eau aux chevilles dans une flaque.
    pub fn submersion(&self, world: &World) -> (f32, (f32, f32)) {
        let registry = world.registry();
        let (x, z) = (
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        );
        let (bottom, top) = (self.position.y, self.position.y + self.height);
        let mut under = 0.0;
        let mut fluid = (5.0, 1000.0);
        for y in bottom.floor() as i32..=top.floor() as i32 {
            let pos = IVec3::new(x, y, z);
            let def = registry.def(world.block(pos));
            if !def.liquid {
                continue;
            }
            // La hauteur de la surface dans cette cellule.
            let covered = registry.def(world.block(pos + IVec3::Y)).liquid;
            let level = if covered {
                1.0
            } else {
                world
                    .fluid_amount(pos)
                    .map_or(1.0, |a| (a as f32 / crate::fluid_sim::FULL as f32).min(1.0))
                    * 0.875
            };
            let (low, high) = (y as f32, y as f32 + level);
            under += (high.min(top) - low.max(bottom)).max(0.0);
            if let Some(def) = def
                .fluid
                .as_deref()
                .and_then(|key| world.fluids()?.get(key))
            {
                fluid = (def.viscosity as f32, def.density as f32);
            }
        }
        ((under / self.height).clamp(0.0, 1.0), fluid)
    }

    fn move_axis(&mut self, world: &World, axis: usize, amount: f32) {
        let steps = (amount.abs() / MAX_STEP).ceil().max(1.0);
        let part = amount / steps;
        for _ in 0..steps as u32 {
            if !self.try_move(world, axis, part) {
                break; // Bloqué : inutile de continuer sur cet axe.
            }
        }
    }

    /// Déplace la boîte de `distance` sur `axis`. En cas de collision, la recolle
    /// contre la face du bloc et renvoie `false`.
    fn try_move(&mut self, world: &World, axis: usize, distance: f32) -> bool {
        if distance == 0.0 {
            return true;
        }
        self.position[axis] += distance;
        let (min, max) = self.bounds();

        // Tous les blocs que la boîte touche maintenant.
        let first = min.floor().as_ivec3();
        let last = max.ceil().as_ivec3() - IVec3::ONE;
        // La face de bloc la plus proche dans le sens du mouvement.
        let mut wall: Option<f32> = None;
        for x in first.x..=last.x {
            for y in first.y..=last.y {
                for z in first.z..=last.z {
                    let block = IVec3::new(x, y, z);
                    // Le registre dit si ce bloc arrête le joueur (un bloc de mod peut ne pas le faire).
                    if !world.is_solid(block) {
                        continue;
                    }
                    let face = if distance > 0.0 {
                        block[axis] as f32
                    } else {
                        block[axis] as f32 + 1.0
                    };
                    // `Option::map_or` : la première face trouvée, ou la plus proche des deux.
                    wall = Some(wall.map_or(face, |w| {
                        if distance > 0.0 {
                            w.min(face)
                        } else {
                            w.max(face)
                        }
                    }));
                }
            }
        }

        let Some(face) = wall else {
            return true;
        };
        if distance > 0.0 {
            self.position[axis] -= max[axis] - face + SKIN;
        } else {
            self.position[axis] += face - min[axis] + SKIN;
            if axis == 1 {
                self.on_ground = true;
            }
        }
        self.velocity[axis] = 0.0;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::stone;

    const DT: f32 = 1.0 / 60.0;

    /// Un sol plat en y = 0, de -8 à 8 sur x et z.
    fn floor() -> World {
        let mut world = World::new();
        for x in -8..8 {
            for z in -8..8 {
                world.set_block(IVec3::new(x, 0, z), stone());
            }
        }
        world
    }

    fn run(body: &mut PlayerBody, world: &World, walk: Vec3, jump: bool, ticks: u32) {
        for _ in 0..ticks {
            body.step(world, walk, jump, DT);
        }
    }

    #[test]
    fn falls_and_lands_on_the_ground() {
        let world = floor();
        let mut body = PlayerBody::new(Vec3::new(0.5, 10.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 120);
        assert!(body.on_ground);
        assert!(
            (body.position.y - 1.0).abs() < 0.01,
            "y = {}",
            body.position.y
        );
        assert_eq!(body.velocity.y, 0.0);
    }

    #[test]
    fn fast_movement_does_not_go_through_walls() {
        let mut world = floor();
        // Un mur d'un seul bloc d'épaisseur en x = 3.
        for y in 1..4 {
            for z in -2..3 {
                world.set_block(IVec3::new(3, y, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        // 300 blocs par seconde : 5 blocs par pas de temps.
        run(&mut body, &world, Vec3::new(300.0, 0.0, 0.0), false, 10);
        assert!(
            body.position.x < 3.0 - PLAYER_WIDTH / 2.0,
            "x = {}",
            body.position.x
        );
        assert!(body.position.x > 2.5);
    }

    #[test]
    fn slides_along_a_wall() {
        let mut world = floor();
        for y in 1..4 {
            for z in -8..8 {
                world.set_block(IVec3::new(2, y, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        // En diagonale contre le mur : x bloque, z continue.
        run(&mut body, &world, Vec3::new(4.0, 0.0, 4.0), false, 30);
        assert!(body.position.x < 2.0 - PLAYER_WIDTH / 2.0);
        assert!(body.position.z > 2.0);
    }

    #[test]
    fn jump_climbs_one_block() {
        let world = floor();
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 2);
        let mut highest = body.position.y;
        body.step(&world, Vec3::ZERO, true, DT);
        for _ in 0..60 {
            body.step(&world, Vec3::ZERO, false, DT);
            highest = highest.max(body.position.y);
        }
        assert!(highest > 2.0, "le saut monte à {highest}");
        assert!(body.on_ground, "retombe sur le sol");
    }

    #[test]
    fn ceiling_stops_the_jump() {
        let mut world = floor();
        // Plafond en y = 3 : la tête (à 2,8) n'a que 0,2 bloc de marge.
        for x in -2..3 {
            for z in -2..3 {
                world.set_block(IVec3::new(x, 3, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 2);
        body.step(&world, Vec3::ZERO, true, DT);
        let mut highest_head = 0.0f32;
        for _ in 0..60 {
            body.step(&world, Vec3::ZERO, false, DT);
            highest_head = highest_head.max(body.bounds().1.y);
        }
        assert!(
            highest_head <= 3.0,
            "la tête traverse le plafond : {highest_head}"
        );
        assert!(body.on_ground);
    }

    #[test]
    fn water_slows_the_fall_and_space_swims_up() {
        let water = crate::registry::BlockRegistry::builtin()
            .id("core:water")
            .unwrap();
        let mut world = floor();
        for x in -8..8 {
            for z in -8..8 {
                for y in 1..10 {
                    world.set_block(IVec3::new(x, y, z), water);
                }
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 8.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 30);
        assert!(body.in_liquid);
        assert!(body.velocity.y >= -SWIM_SINK_SPEED, "on coule lentement");
        let low = body.position.y;
        run(&mut body, &world, Vec3::ZERO, true, 30);
        assert!(body.position.y > low + 1.0, "Espace fait remonter");
    }

    #[test]
    fn overlap_test_for_block_placement() {
        let body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        assert!(body.overlaps_block(IVec3::new(0, 1, 0)), "les pieds");
        assert!(body.overlaps_block(IVec3::new(0, 2, 0)), "la tête");
        assert!(
            !body.overlaps_block(IVec3::new(0, 0, 0)),
            "le sol sous les pieds"
        );
        assert!(!body.overlaps_block(IVec3::new(1, 1, 0)), "à côté");
        assert!(
            !body.overlaps_block(IVec3::new(0, 3, 0)),
            "au-dessus de la tête"
        );
    }

    /// Un bassin de `depth` blocs d'un fluide, sur le sol de `floor()`.
    fn pool(key: &str, depth: i32) -> World {
        let block = crate::registry::BlockRegistry::builtin().id(key).unwrap();
        let mut world = floor();
        world.set_fluids(std::sync::Arc::new(crate::fluids::FluidRegistry::builtin()));
        for x in -8..8 {
            for z in -8..8 {
                for y in 1..1 + depth {
                    world.set_block(IVec3::new(x, y, z), block);
                }
            }
        }
        world
    }

    #[test]
    fn a_body_floats_with_its_head_out_of_deep_water() {
        let world = pool("core:water", 8);
        let mut body = PlayerBody::new(Vec3::new(0.5, 2.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 600);
        // La surface est à y = 8 + 0,875 ; les yeux (à 1,62 des pieds) dépassent presque.
        let eyes = body.position.y + PLAYER_EYE_HEIGHT;
        assert!((7.5..10.0).contains(&eyes), "yeux à {eyes}");
        assert!(body.in_liquid);
    }

    #[test]
    fn a_thin_puddle_is_walked_through_and_lava_is_slow() {
        // De l'eau jusqu'aux chevilles : on marche, on ne nage pas.
        let mut world = floor();
        let water = crate::registry::BlockRegistry::builtin()
            .id("core:water")
            .unwrap();
        for x in -8..8 {
            for z in -8..8 {
                let pos = IVec3::new(x, 1, z);
                world.set_block(pos, water);
                let (chunk, [lx, ly, lz]) = World::split(pos);
                world
                    .chunk_mut(chunk)
                    .set_fluid_amount(lx, ly, lz, Some(512));
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 10);
        assert!(!body.in_liquid, "immersion {}", body.submersion);
        assert!(body.submersion > 0.0);
        // Dans la lave, on avance bien moins vite que dans l'eau.
        let speed = |key: &str| {
            let world = pool(key, 4);
            let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
            run(&mut body, &world, Vec3::X * 4.0, false, 60);
            body.position.x - 0.5
        };
        assert!(speed("core:lava") < speed("core:water") * 0.6);
    }
}
